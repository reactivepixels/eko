//! Native bit-perfect audio engine.
//!
//! # Role in the one-engine architecture
//!
//! This module is EKO's single source of audio truth. The frontend (TypeScript/React)
//! calls the Tauri commands defined here; everything else — hardware negotiation,
//! decoding, DSP, and spectrum analysis — lives in Rust. No audio data crosses the
//! IPC boundary.
//!
//! # Bit-perfect philosophy
//!
//! "Bit-perfect" means the PCM values the decoder produces reach the DAC unchanged.
//! Two things can break that guarantee:
//!
//! 1. **OS resampling.** macOS will silently resample any stream whose rate differs
//!    from the device's current nominal rate. We prevent this via [`crate::coreaudio`],
//!    which switches the device to the file's own sample rate before playback starts —
//!    exactly what Roon and Audirvana do.
//!
//! 2. **In-process DSP.** Every sample that passes through the EQ biquad chain or a
//!    volume multiplier is no longer bit-perfect. The callback detects when EQ is flat
//!    *and* volume is unity and takes a dedicated bypass path (`copy_from_slice`) that
//!    copies decoded samples verbatim. See [`EqParams::active`] and the bypass condition
//!    in `decode_and_play`'s cpal callback.
//!
//! When both conditions hold the signal path is: decoder → `Mutex<Vec<f32>>` →
//! `copy_from_slice` → cpal → CoreAudio HAL → DAC. No arithmetic is applied.
//!
//! # Streaming decode
//!
//! Decoding is streamed via [`symphonia`]: playback starts as soon as the first batch
//! of packets has been decoded, and a background decode loop appends to a growing
//! `Mutex<Vec<f32>>` while the cpal callback consumes from the front. The full decoded
//! buffer is retained in memory for the duration of the track so that seeking and the
//! FFT spectrum analyser both have O(1) random access.
//!
//! # Realtime-safety of the cpal audio callback
//!
//! An audio callback runs on a high-priority thread with a hard deadline. The OS
//! expects the callback to fill its output buffer and return before the next hardware
//! interrupt fires (typically every 5–15 ms on macOS). Violating that deadline causes
//! a buffer underrun — an audible glitch. The practical rules for safe audio callbacks
//! are:
//!
//! - **No allocation.** `malloc`/`free` can block on the global heap lock.
//! - **No unbounded blocking.** Any syscall, lock, or I/O that can block arbitrarily
//!   long can starve the callback past its deadline.
//! - **No priority inversion.** If the callback holds a lock that a lower-priority
//!   thread also tries to acquire, the high-priority callback can be preempted while
//!   it waits for the lower-priority thread to release — the classic real-time
//!   priority-inversion scenario.
//!
//! **What the current code actually does:**
//!
//! The callback acquires two `Mutex` locks on every invocation:
//!
//! 1. `cb.eq.lock()` — to read EQ parameters.
//! 2. `cb.samples.lock()` — to read decoded PCM samples.
//!
//! Both are held for a very short critical section (a parameter copy and a
//! `copy_from_slice` / per-sample arithmetic pass), and the only other thread that
//! contends for `samples` is the decode loop, which holds it briefly to `extend_from_slice`
//! a local chunk. There is no allocation inside either locked region.
//!
//! **Why this is acceptable in practice:**
//!
//! On macOS with a typical 512-sample buffer at 44.1 kHz the callback deadline is
//! ~11 ms. The decode thread holds `samples` for at most a few microseconds per
//! `extend_from_slice` call, and because `Vec::extend_from_slice` only reallocates
//! when the capacity is exhausted — and the Vec is pre-growing by 16-packet batches —
//! reallocation is rare and brief. In a single-producer / single-consumer pattern like
//! this the probability of the callback catching the decode thread mid-lock is low, and
//! the contention window is far shorter than any realistic deadline.
//!
//! **The known caveat:**
//!
//! This is still *not* formally realtime-safe. A sufficiently slow system, a large
//! reallocation, or OS scheduling jitter could in principle cause a contended lock to
//! exceed the callback deadline. In practice, on a modern Mac, glitches from this source
//! have not been observed.
//!
//! **The principled improvement (documented future option):**
//!
//! The textbook fix is a lock-free SPSC (single-producer, single-consumer) ring buffer:
//! the decode thread writes decoded frames into one end; the callback reads from the
//! other with no lock at all, using only atomic index updates. A triple-buffer scheme
//! achieves the same for the EQ parameters. Neither `std` nor the current dependency
//! set includes a production-quality SPSC ring, but crates such as `ringbuf` (Apache
//! 2.0) provide one. This is the right long-term direction if glitches are ever
//! observed or if EKO is ported to a stricter real-time host environment.

use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::f32::consts::PI;
use std::sync::Arc as StdArc;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use triple_buffer::triple_buffer;

/// A [`symphonia::core::io::MediaSource`] backed by an in-progress HTTP download.
///
/// `engine_play_url` spawns a dedicated thread that pulls the response body into
/// `buf` in 64 KiB chunks. `HttpSource` implements [`std::io::Read`] on top of that
/// same buffer: if the decoder asks for bytes that haven't arrived yet the read blocks
/// in an 8 ms poll loop until they do (or the download thread sets `done`).
///
/// This lets symphonia start probing and decoding the file before it's fully
/// downloaded — the same streaming-decode latency benefit as local files, but over the
/// network. The design is forward-only: [`Seek`] is implemented (symphonia requires it)
/// but `is_seekable` returns `false` so symphonia never actually issues backward seeks.
///
/// # Stall guard
///
/// If the download thread stalls (no new bytes and `done` not set), consecutive empty
/// polls are counted. After ~15 s (1875 × 8 ms polls) with no progress, `read` returns
/// `Ok(0)` so symphonia aborts cleanly rather than looping forever.
struct HttpSource {
    /// Shared byte buffer filled by the download thread.
    buf: Arc<Mutex<Vec<u8>>>,
    /// Set to `true` by the download thread when the response body is exhausted.
    done: Arc<AtomicBool>,
    /// Current read position within `buf` (in bytes).
    pos: usize,
    /// Consecutive empty-poll counter for the stall guard (~15 s at 8 ms/poll).
    stall_count: u32,
}
impl Read for HttpSource {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        // ~15 s stall budget: 1875 × 8 ms = 15 000 ms.
        const STALL_LIMIT: u32 = 1875;
        loop {
            {
                let buf = self.buf.lock().unwrap();
                let avail = buf.len().saturating_sub(self.pos);
                if avail > 0 {
                    let n = avail.min(out.len());
                    out[..n].copy_from_slice(&buf[self.pos..self.pos + n]);
                    self.pos += n;
                    self.stall_count = 0; // progress made — reset the stall counter
                    return Ok(n);
                }
                if self.done.load(Ordering::Relaxed) {
                    return Ok(0);
                }
            }
            self.stall_count += 1;
            if self.stall_count >= STALL_LIMIT {
                // Network has been stalled for ~15 s with no new bytes and download
                // thread still running. Return EOF so symphonia aborts cleanly.
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "HttpSource: network stall timeout",
                ));
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}
impl Seek for HttpSource {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let len = self.buf.lock().unwrap().len();
        let np = match from {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(n) => self.pos as i64 + n,
            SeekFrom::End(n) => len as i64 + n,
        };
        self.pos = np.max(0).min(len as i64) as usize;
        Ok(self.pos as u64)
    }
}
impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// FFT window length in samples. 1024 gives ~43 Hz bin resolution at 44.1 kHz,
/// which is more than enough for a 32-band log display.
const FFT_N: usize = 1024;

/// Number of log-spaced frequency bands in the spectrum display.
const N_BANDS: usize = 32;

/// Centre frequencies (Hz) of the 10 EQ bands. Matches classic hardware parametric EQ
/// positions: 60 Hz sub, 170 Hz bass, 310 Hz low-mid, 600 Hz mid, 1 kHz presence,
/// 3/6/12/14/16 kHz air.
const EQ_FREQS: [f32; 10] = [
    60., 170., 310., 600., 1000., 3000., 6000., 12000., 14000., 16000.,
];

/// Maximum channel count the EQ state arrays are allocated for. Streams with more
/// channels than this fall back to copying channel `MAX_CH - 1` — intentionally
/// conservative; real-world content is ≤ 8 channels.
const MAX_CH: usize = 8;

/// Maximum number of biquad stages in the unified band_state array. When Pro is
/// disabled this is 10 (graphic EQ). When Pro is enabled it grows to MAX_PARAM_BANDS
/// to also serve the parametric EQ cascade — the same array serves both paths.
#[cfg(not(feature = "pro"))]
const MAX_BAND_STATE: usize = 10;
#[cfg(feature = "pro")]
const MAX_BAND_STATE: usize = crate::pro::param_eq::MAX_PARAM_BANDS;

/// Snapshot of the EQ parameters written by `engine_set_eq` and read by the cpal
/// callback. Cloned in full each time the callback enters the DSP path.
#[derive(Clone)]
struct EqParams {
    /// Global EQ on/off switch. When `false`, [`EqParams::active`] is always `false`
    /// regardless of the band gains, and the callback bypasses all DSP.
    enabled: bool,
    /// Pre-amplifier gain applied before the biquad chain, in dB. Compensates for the
    /// headroom reduction that boosting EQ bands can cause.
    preamp: f32,
    /// Per-band peak gain in dB, one value per [`EQ_FREQS`] entry. `0.0` = flat.
    gains: [f32; 10],
}
impl Default for EqParams {
    fn default() -> Self {
        EqParams {
            enabled: true,
            preamp: 0.0,
            gains: [0.0; 10],
        }
    }
}
impl EqParams {
    /// EQ only colours the signal when it's actually doing something — otherwise the
    /// callback bypasses it entirely, keeping playback bit-perfect.
    fn active(&self) -> bool {
        self.enabled && (self.preamp != 0.0 || self.gains.iter().any(|&g| g != 0.0))
    }
}

// The biquad filter lives in the shared `crate::biquad` module (free DSP) so the
// graphic EQ here and the Pro parametric EQ build coefficients from one source.
use crate::biquad::Biquad;

// ── EQ mode (free: Graphic only; pro: Graphic | Parametric) ──────────────────

/// Which EQ is routed to the DSP path. Only one can be active at a time so the
/// bypass (bit-perfect) condition remains unambiguous.
#[derive(Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EqMode {
    /// 10-band graphic EQ (the free / default mode).
    #[default]
    Graphic,
    /// N-band parametric EQ (Pro feature).
    #[cfg(feature = "pro")]
    Parametric,
}

// ── Lock-free DSP parameter handoff ───────────────────────────────────────────
//
// The cpal audio callback runs on a high-priority thread and must never block.
// All DSP parameters that the callback reads (EQ mode, graphic params, parametric
// params) are published through a `triple_buffer::Input` / `Output` pair:
//
//   Control side (Tauri command handlers):
//     Build a fresh `DspSnapshot` → `dsp_input.write(snapshot)` — wait-free.
//
//   Audio side (cpal callback):
//     `dsp_output.read()` — always returns the latest snapshot, wait-free.

/// A snapshot of all DSP parameters consumed by the cpal audio callback.
///
/// Written atomically by Tauri command handlers via a triple-buffer `Input`.
/// Read on the audio thread via `Output::read()` — wait-free, no allocation, no lock.
#[derive(Clone, Default)]
struct DspSnapshot {
    /// Which EQ is routed to DSP.
    mode: EqMode,
    /// Graphic EQ parameters (10-band).
    graphic: EqParams,
    /// Parametric EQ parameters (Pro). Only populated when `feature = "pro"`.
    #[cfg(feature = "pro")]
    parametric: crate::pro::param_eq::ParamEqParams,
}

impl DspSnapshot {
    /// True when the currently-routed EQ is doing anything to the signal.
    /// Mirrors the per-type `active()` predicates; drives the bit-perfect bypass.
    fn eq_active(&self) -> bool {
        match self.mode {
            EqMode::Graphic => self.graphic.active(),
            #[cfg(feature = "pro")]
            EqMode::Parametric => self.parametric.active(),
        }
    }
}

/// A queued next source for gapless continuation, set via [`engine_enqueue`] and consumed
/// by the decode loop at end-of-track when its sample rate matches the open stream.
///
/// `Source::Cached` is a Pro feature — it plays from the offline encrypted cache.
enum Source {
    File(String),
    Url(String),
    /// Play from the offline encrypted cache (Pro only). `plain_len` is the original
    /// plaintext file size stored in the cache index (required for seek).
    #[cfg(feature = "pro")]
    Cached {
        track_id: String,
        plain_len: u64,
    },
}

/// Index of the segment (track) that contains interleaved sample position `pos`.
fn segment_at(starts: &[usize], pos: usize) -> usize {
    let mut i = 0;
    for (k, &s) in starts.iter().enumerate() {
        if pos >= s {
            i = k;
        } else {
            break;
        }
    }
    i
}

/// Commands sent from the Tauri command handlers to the decode/playback thread.
enum Cmd {
    Pause,
    Resume,
    Seek(usize),
    Stop,
}

/// State shared between the Tauri command handlers, the decode thread, and the cpal
/// audio callback.
struct Shared {
    pos: AtomicUsize,
    total: AtomicUsize,
    rate: AtomicU32,
    channels: AtomicU32,
    src_rate: AtomicU32,
    dev_rate: AtomicU32,
    bits: AtomicU32,
    codec: Mutex<String>,
    paused: AtomicBool,
    playing: AtomicBool,
    done: AtomicBool,
    device: Mutex<String>,
    bands: Mutex<Vec<f32>>,
    samples: Mutex<Vec<f32>>,
    /// Current graphic EQ parameters, kept for command-handler reads.
    eq: Mutex<EqParams>,
    /// Parametric EQ parameters (Pro feature). Kept for command-handler reads.
    #[cfg(feature = "pro")]
    param_eq: Mutex<crate::pro::param_eq::ParamEqParams>,
    /// Which EQ is routed to DSP.
    eq_mode: Mutex<EqMode>,
    /// Lock-free DSP parameter handoff.
    dsp_input: Mutex<triple_buffer::Input<DspSnapshot>>,
    vol: AtomicU32,
    rg_gain: AtomicU32,
    next_src: Mutex<Option<Source>>,
    seg_starts: Mutex<Vec<usize>>,
}

/// The Tauri-managed engine state, registered once at startup.
#[derive(Default)]
pub struct Engine {
    cmd: Mutex<Option<Sender<Cmd>>>,
    shared: Arc<SharedHolder>,
    now: Mutex<NowPlaying>,
    device_pref: Mutex<Option<String>>,
}

/// Now-playing metadata, set by the main window and read by any window.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub cover_url: String,
    pub cover_path: String,
    pub theme: String,
    pub index: i64,
    pub total: i64,
}

#[derive(Default)]
struct SharedHolder(Mutex<Option<Arc<Shared>>>);

/// Snapshot of engine state returned by [`engine_status`].
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub playing: bool,
    pub pos_ms: u64,
    pub dur_ms: u64,
    /// How far the decode buffer has filled, in ms within the current track. Equals
    /// `dur_ms` for a local / fully-decoded track; lags it while a server stream is still
    /// downloading. The frontend draws this as the "buffered" region on the scrubber and
    /// uses it to show when an armed forward-seek is still waiting for the download.
    pub buffered_ms: u64,
    pub rate: u32,
    pub channels: u32,
    pub device: String,
    pub src_rate: u32,
    pub dev_rate: u32,
    pub bits: u32,
    pub codec: String,
    pub seg: u32,
}

/// Re-channel `data` from `from`-channel interleaved PCM to `to`-channel interleaved PCM.
fn remix(data: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || from == 0 {
        return data.to_vec();
    }
    let frames = data.len() / from;
    let mut out = vec![0.0f32; frames * to];
    for f in 0..frames {
        let mut mono = 0.0;
        for c in 0..from {
            mono += data[f * from + c];
        }
        mono /= from as f32;
        for c in 0..to {
            out[f * to + c] = if c < from { data[f * from + c] } else { mono };
        }
    }
    out
}

/// Linear-interpolation resampler for the uncommon case where the hardware can't run
/// at the file's native rate.
fn resample(data: &[f32], ch: usize, from: u32, to: u32) -> Vec<f32> {
    if from == to || ch == 0 {
        return data.to_vec();
    }
    let frames_in = data.len() / ch;
    if frames_in < 2 {
        return data.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let frames_out = (frames_in as f64 * ratio) as usize;
    let mut out = vec![0.0f32; frames_out * ch];
    for f in 0..frames_out {
        let src = f as f64 / ratio;
        let i0 = src.floor() as usize;
        let frac = (src - i0 as f64) as f32;
        let i1 = (i0 + 1).min(frames_in - 1);
        for c in 0..ch {
            let a = data[i0 * ch + c];
            let b = data[i1 * ch + c];
            out[f * ch + c] = a + (b - a) * frac;
        }
    }
    out
}

/// Select a cpal `StreamConfig` that matches the file's native rate and channel count.
fn pick_config(
    device: &cpal::Device,
    rate: u32,
    channels: usize,
) -> Result<cpal::StreamConfig, String> {
    if let Ok(configs) = device.supported_output_configs() {
        for c in configs {
            if c.channels() as usize == channels
                && c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate().0 <= rate
                && rate <= c.max_sample_rate().0
            {
                return Ok(c.with_sample_rate(cpal::SampleRate(rate)).config());
            }
        }
    }
    device
        .default_output_config()
        .map(|c| c.config())
        .map_err(|e| e.to_string())
}

/// FFT a window of the currently-playing audio into 32 log-spaced band magnitudes.
#[allow(clippy::needless_range_loop)]
fn analyze(shared: &Shared, fft: &StdArc<dyn Fft<f32>>, fbuf: &mut [Complex<f32>]) {
    if !shared.playing.load(Ordering::Relaxed) || shared.paused.load(Ordering::Relaxed) {
        let mut g = shared.bands.lock().unwrap();
        for x in g.iter_mut() {
            *x *= 0.85;
        }
        return;
    }
    let ch = (shared.channels.load(Ordering::Relaxed) as usize).max(1);
    {
        let buf = shared.samples.lock().unwrap();
        let frames = buf.len() / ch;
        if frames < FFT_N {
            return;
        }
        let start = (shared.pos.load(Ordering::Relaxed) / ch).min(frames - FFT_N);
        for i in 0..FFT_N {
            let fi = start + i;
            let mut s = 0.0;
            for c in 0..ch {
                s += buf[fi * ch + c];
            }
            s /= ch as f32;
            let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_N as f32 - 1.0)).cos();
            fbuf[i] = Complex::new(s * w, 0.0);
        }
    }
    fft.process(fbuf);
    let bins = FFT_N / 2;
    let mut bands = vec![0f32; N_BANDS];
    for b in 0..N_BANDS {
        let lo = (bins as f32).powf(b as f32 / N_BANDS as f32) as usize;
        let hi = ((bins as f32).powf((b as f32 + 1.0) / N_BANDS as f32) as usize)
            .max(lo + 1)
            .min(bins);
        let mut m = 0.0;
        let mut n = 0usize;
        for k in lo..hi {
            m += fbuf[k].norm();
            n += 1;
        }
        if n > 0 {
            m /= n as f32;
        }
        bands[b] = (m / (FFT_N as f32 * 0.25)).sqrt().min(1.0);
    }
    *shared.bands.lock().unwrap() = bands;
}

/// Everything the decode loop needs for one source.
struct OpenSource {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    file_rate: u32,
    file_ch: usize,
    src_bits: u32,
    codec: String,
    n_frames: Option<u64>,
}

/// Open a local file, server URL, or cached encrypted file: start any download, probe the
/// container, and build a decoder. Returns `None` on any failure.
fn open_source(src: Source) -> Option<OpenSource> {
    let (mss, ext): (MediaSourceStream, Option<String>) = match src {
        Source::File(path) => {
            let file = std::fs::File::open(&path).ok()?;
            let ext = std::path::Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string());
            (
                MediaSourceStream::new(Box::new(file), Default::default()),
                ext,
            )
        }
        #[cfg(feature = "pro")]
        Source::Cached {
            track_id,
            plain_len,
        } => {
            use crate::pro::offline::EncryptedFileSource;
            let cache_dir = {
                let base = std::env::var("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| std::env::temp_dir());
                base.join("Library/Caches/com.reactivepixels.eko/offline")
            };
            let sanitized: String = track_id
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let enc_path = cache_dir.join(format!("{sanitized}.enc"));
            let codec_hint = {
                let idx_path = cache_dir.join("index.json");
                std::fs::read_to_string(&idx_path)
                    .ok()
                    .and_then(|s| {
                        let v: serde_json::Value = serde_json::from_str(&s).ok()?;
                        v["entries"].as_array()?.iter().find_map(|e| {
                            if e["trackId"].as_str()? == track_id {
                                e["codec"].as_str().map(|c| c.to_string())
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or_default()
            };
            let src = EncryptedFileSource::open(&enc_path, plain_len).ok()?;
            let ext = if codec_hint.is_empty() {
                None
            } else {
                Some(codec_hint)
            };
            (
                MediaSourceStream::new(Box::new(src), Default::default()),
                ext,
            )
        }
        Source::Url(url) => {
            let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
            let done = Arc::new(AtomicBool::new(false));
            {
                let (buf, done) = (buf.clone(), done.clone());
                std::thread::spawn(move || {
                    let client = reqwest::blocking::Client::builder()
                        .connect_timeout(Duration::from_secs(10))
                        .timeout(Duration::from_secs(60))
                        .build()
                        .unwrap_or_else(|_| reqwest::blocking::Client::new());
                    match client.get(&url).send() {
                        Ok(mut resp) => {
                            let mut chunk = vec![0u8; 65536];
                            loop {
                                match resp.read(&mut chunk) {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => buf.lock().unwrap().extend_from_slice(&chunk[..n]),
                                }
                            }
                        }
                        Err(e) => eprintln!("engine fetch: {e}"),
                    }
                    done.store(true, Ordering::SeqCst);
                });
            }
            let s = HttpSource {
                buf,
                done,
                pos: 0,
                stall_count: 0,
            };
            (
                MediaSourceStream::new(Box::new(s), Default::default()),
                None,
            )
        }
    };

    let mut hint = Hint::new();
    if let Some(e) = &ext {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let format = probed.format;
    let (track_id, file_rate, file_ch, n_frames, src_bits, codec) = {
        let t = format.default_track()?;
        let cp = &t.codec_params;
        let codec = symphonia::default::get_codecs()
            .get_codec(cp.codec)
            .map(|d| d.short_name.to_string())
            .unwrap_or_default();
        (
            t.id,
            cp.sample_rate.unwrap_or(44100),
            cp.channels.map(|c| c.count()).unwrap_or(2),
            cp.n_frames,
            cp.bits_per_sample.unwrap_or(0),
            codec,
        )
    };
    let decoder = symphonia::default::get_codecs()
        .make(
            &format.default_track()?.codec_params,
            &DecoderOptions::default(),
        )
        .ok()?;
    Some(OpenSource {
        format,
        decoder,
        track_id,
        file_rate,
        file_ch,
        src_bits,
        codec,
        n_frames,
    })
}

/// Build the 10 biquad coefficient sets for the current EQ gains at sample rate `fs`.
fn eq_coeffs(gains: &[f32; 10], fs: u32) -> [Biquad; 10] {
    let mut c = [Biquad::default(); 10];
    for i in 0..10 {
        c[i] = Biquad::peaking(EQ_FREQS[i], 1.0, gains[i], fs as f32);
    }
    c
}

/// Decide whether the cpal callback may take the untouched-samples bypass.
#[inline]
fn is_bitperfect(eq_active: bool, vol: f32, rg_gain: f32) -> bool {
    !eq_active && vol == 1.0 && rg_gain == 1.0
}

/// Convert an optional ReplayGain value in dB to a clamped linear multiplier.
fn rg_db_to_linear(gain_db: Option<f32>) -> f32 {
    match gain_db {
        Some(db) if db != 0.0 => 10f32.powf(db / 20.0).clamp(0.0, 4.0),
        _ => 1.0,
    }
}

fn decode_and_play(
    first: Source,
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Cmd>,
    device_name: Option<String>,
    mut dsp_output: triple_buffer::Output<DspSnapshot>,
) {
    let fail = |shared: &Shared| shared.playing.store(false, Ordering::SeqCst);

    let opened = match open_source(first) {
        Some(o) => o,
        None => {
            fail(&shared);
            return;
        }
    };
    let mut format = opened.format;
    let mut decoder = opened.decoder;
    let mut track_id = opened.track_id;
    let mut file_rate = opened.file_rate;
    let mut file_ch = opened.file_ch;
    let n_frames = opened.n_frames;
    let src_bits = opened.src_bits;
    let codec_name = opened.codec;

    let host = cpal::default_host();
    let device = device_name
        .as_ref()
        .and_then(|name| {
            host.output_devices()
                .ok()
                .and_then(|mut ds| ds.find(|d| d.name().map(|n| n == *name).unwrap_or(false)))
        })
        .or_else(|| host.default_output_device());
    let device = match device {
        Some(d) => d,
        None => {
            fail(&shared);
            return;
        }
    };
    let dev_name = device.name().unwrap_or_else(|_| "Output".into());
    *shared.device.lock().unwrap() = dev_name.clone();

    #[cfg(target_os = "macos")]
    let matched_rate = crate::coreaudio::match_device_rate(Some(&dev_name), file_rate);

    let config = match pick_config(&device, file_rate, file_ch) {
        Ok(c) => c,
        Err(_) => {
            fail(&shared);
            return;
        }
    };
    let out_rate = config.sample_rate.0;
    let out_ch = config.channels as usize;
    #[cfg(target_os = "macos")]
    let dev_rate = matched_rate.unwrap_or(out_rate);
    #[cfg(not(target_os = "macos"))]
    let dev_rate = out_rate;

    let est_total = n_frames
        .map(|n| ((n as f64 * out_rate as f64 / file_rate as f64) as usize) * out_ch)
        .unwrap_or(0);
    shared.total.store(est_total, Ordering::SeqCst);
    shared.rate.store(out_rate, Ordering::SeqCst);
    shared.channels.store(out_ch as u32, Ordering::SeqCst);
    shared.src_rate.store(file_rate, Ordering::SeqCst);
    shared.dev_rate.store(dev_rate, Ordering::SeqCst);
    shared.bits.store(src_bits, Ordering::SeqCst);
    *shared.codec.lock().unwrap() = codec_name;

    let cb = shared.clone();
    let err_shared = shared.clone();
    // Unified per-band, per-channel biquad delay state. Sized to MAX_BAND_STATE so the
    // same array serves the graphic EQ (free) and the parametric EQ (pro).
    // Owned entirely by the audio thread — never shared, never locked.
    let mut band_state = [[(0.0f32, 0.0f32); MAX_CH]; MAX_BAND_STATE];
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            if cb.paused.load(Ordering::Relaxed) {
                data.iter_mut().for_each(|s| *s = 0.0);
                return;
            }

            // ── Lock-free DSP parameter read ──────────────────────────────────
            // `dsp_output.read()` is wait-free (atomic index swap + pointer read).
            let dsp = dsp_output.read();

            let eq_active = dsp.eq_active();
            let (pre, coeffs, n_coeffs): (f32, [Biquad; MAX_BAND_STATE], usize) = if eq_active {
                match dsp.mode {
                    EqMode::Graphic => {
                        let c = eq_coeffs(&dsp.graphic.gains, out_rate);
                        let mut packed = [Biquad::default(); MAX_BAND_STATE];
                        packed[..10].copy_from_slice(&c);
                        (10f32.powf(dsp.graphic.preamp / 20.0), packed, 10usize)
                    }
                    #[cfg(feature = "pro")]
                    EqMode::Parametric => {
                        // `param_eq` builds the shared `Biquad` directly — no type bridge.
                        let (pc, n) = dsp.parametric.build_coeffs(out_rate as f32);
                        let mut packed = [Biquad::default(); MAX_BAND_STATE];
                        packed[..n].copy_from_slice(&pc[..n]);
                        (10f32.powf(dsp.parametric.preamp / 20.0), packed, n)
                    }
                }
            } else {
                (1.0, [Biquad::default(); MAX_BAND_STATE], 0usize)
            };

            let vol = f32::from_bits(cb.vol.load(Ordering::Relaxed));
            let rg = f32::from_bits(cb.rg_gain.load(Ordering::Relaxed));
            let bit_perfect = is_bitperfect(eq_active, vol, rg);
            let buf = cb.samples.lock().unwrap_or_else(|e| e.into_inner());
            let len = buf.len();
            let mut p = cb.pos.load(Ordering::Relaxed);
            for frame in data.chunks_mut(out_ch) {
                if p + out_ch <= len {
                    if bit_perfect {
                        frame.copy_from_slice(&buf[p..p + out_ch]);
                    } else {
                        for c in 0..out_ch {
                            let cc = c.min(MAX_CH - 1);
                            let mut x = buf[p + c];
                            if eq_active {
                                x *= pre;
                                for b in 0..n_coeffs {
                                    x = coeffs[b].process(&mut band_state[b][cc], x);
                                }
                            }
                            frame[c] = x * vol * rg;
                        }
                    }
                    p += out_ch;
                } else {
                    frame.iter_mut().for_each(|s| *s = 0.0);
                }
            }
            cb.pos.store(p, Ordering::Relaxed);
            drop(buf);
            if p >= len && cb.done.load(Ordering::Relaxed) {
                cb.playing.store(false, Ordering::Relaxed);
            }
        },
        move |e| {
            eprintln!("cpal stream error: {e}");
            err_shared.playing.store(false, Ordering::SeqCst);
            err_shared.done.store(true, Ordering::SeqCst);
        },
        None,
    );
    let stream = match stream {
        Ok(s) => s,
        Err(_) => {
            fail(&shared);
            return;
        }
    };
    if stream.play().is_err() {
        fail(&shared);
        return;
    }

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_N);
    let mut fbuf = vec![Complex::<f32>::new(0.0, 0.0); FFT_N];
    let mut last_analyze = Instant::now();
    let mut decoding = true;

    loop {
        if decoding {
            let mut local: Vec<f32> = Vec::new();
            for _ in 0..16 {
                match format.next_packet() {
                    Ok(packet) => {
                        if packet.track_id() != track_id {
                            continue;
                        }
                        match decoder.decode(&packet) {
                            Ok(decoded) => {
                                let spec = *decoded.spec();
                                let mut sb =
                                    SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                                sb.copy_interleaved_ref(decoded);
                                let mut chunk = sb.samples().to_vec();
                                if file_ch != out_ch {
                                    chunk = remix(&chunk, file_ch, out_ch);
                                }
                                if file_rate != out_rate {
                                    chunk = resample(&chunk, out_ch, file_rate, out_rate);
                                }
                                local.extend_from_slice(&chunk);
                            }
                            Err(SymError::DecodeError(_)) => continue,
                            Err(_) => {
                                decoding = false;
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        decoding = false;
                        break;
                    }
                }
            }
            if !local.is_empty() {
                shared.samples.lock().unwrap().extend_from_slice(&local);
            }
            if !decoding {
                let mut continued = false;
                if let Some(src) = shared.next_src.lock().unwrap().take() {
                    if let Some(nd) = open_source(src) {
                        // Gapless continuation only when the next track matches the output rate
                        // (so there's no resampling seam). A rate change ends the session and the
                        // frontend restarts at the new track's native rate — keeping playback
                        // bit-perfect.
                        if nd.file_rate == out_rate {
                            let boundary = shared.samples.lock().unwrap().len();
                            shared.seg_starts.lock().unwrap().push(boundary);
                            let est = nd.n_frames.map(|n| n as usize * out_ch).unwrap_or(0);
                            shared.total.store(boundary + est, Ordering::SeqCst);
                            shared.src_rate.store(nd.file_rate, Ordering::SeqCst);
                            shared.bits.store(nd.src_bits, Ordering::SeqCst);
                            *shared.codec.lock().unwrap() = nd.codec;
                            format = nd.format;
                            decoder = nd.decoder;
                            track_id = nd.track_id;
                            file_rate = nd.file_rate;
                            file_ch = nd.file_ch;
                            decoding = true;
                            continued = true;
                        }
                    }
                }
                if !continued {
                    let actual = shared.samples.lock().unwrap().len();
                    shared.total.store(actual, Ordering::SeqCst);
                    shared.done.store(true, Ordering::SeqCst);
                }
            }
        }

        if last_analyze.elapsed() >= Duration::from_millis(33) {
            analyze(&shared, &fft, &mut fbuf);
            last_analyze = Instant::now();
        }

        let timeout = if decoding {
            Duration::from_millis(0)
        } else {
            Duration::from_millis(33)
        };
        let mut stop = false;
        let mut first = rx.recv_timeout(timeout);
        loop {
            match first {
                Ok(Cmd::Pause) => shared.paused.store(true, Ordering::SeqCst),
                Ok(Cmd::Resume) => shared.paused.store(false, Ordering::SeqCst),
                Ok(Cmd::Seek(frame)) => {
                    let starts = shared.seg_starts.lock().unwrap().clone();
                    let cur = shared.pos.load(Ordering::SeqCst);
                    let i = segment_at(&starts, cur);
                    let track_start = starts[i];
                    let len = shared.samples.lock().unwrap().len();
                    // Seek upper bound:
                    //  - a non-final segment ends at the next boundary (fixed);
                    //  - the final segment, once fully decoded, ends at the real decoded `len`;
                    //  - the final segment while still STREAMING ends at the ESTIMATED track
                    //    length (`shared.total`), so a forward seek may land PAST the download
                    //    edge. The callback outputs silence and holds `pos` while it's beyond
                    //    the decoded buffer (it only advances when `p + out_ch <= len`), so
                    //    playback auto-resumes the instant the decoder reaches the target —
                    //    "arm-and-snap". This needs no realtime-callback change and stays
                    //    bit-perfect (we still decode the original file, never a transcode).
                    //    The frontend shows the buffered extent (`buffered_ms`) so the wait is
                    //    visible. See docs/architecture/overview.md + ROADMAP.
                    let done = shared.done.load(Ordering::SeqCst);
                    let end = match starts.get(i + 1).copied() {
                        Some(next) => next,
                        None if done => len,
                        None => (shared.total.load(Ordering::SeqCst)).max(len),
                    };
                    let target = track_start
                        .saturating_add(frame.saturating_mul(out_ch))
                        .min(end);
                    shared.pos.store(target, Ordering::SeqCst);
                }
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    stop = true;
                    break;
                }
                Err(RecvTimeoutError::Timeout) => break,
            }
            match rx.try_recv() {
                Ok(c) => first = Ok(c),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    stop = true;
                    break;
                }
            }
        }
        if stop {
            break;
        }
    }
}

fn stop_current(engine: &Engine) {
    if let Some(tx) = engine.cmd.lock().unwrap().take() {
        let _ = tx.send(Cmd::Stop);
    }
}

fn new_shared() -> (Arc<Shared>, triple_buffer::Output<DspSnapshot>) {
    let (dsp_input, dsp_output) = triple_buffer(&DspSnapshot::default());
    let shared = Arc::new(Shared {
        pos: AtomicUsize::new(0),
        total: AtomicUsize::new(0),
        rate: AtomicU32::new(0),
        channels: AtomicU32::new(2),
        src_rate: AtomicU32::new(0),
        dev_rate: AtomicU32::new(0),
        bits: AtomicU32::new(0),
        codec: Mutex::new(String::new()),
        paused: AtomicBool::new(false),
        playing: AtomicBool::new(true),
        done: AtomicBool::new(false),
        device: Mutex::new(String::new()),
        bands: Mutex::new(vec![0.0; N_BANDS]),
        samples: Mutex::new(Vec::new()),
        eq: Mutex::new(EqParams::default()),
        #[cfg(feature = "pro")]
        param_eq: Mutex::new(crate::pro::param_eq::ParamEqParams::default()),
        eq_mode: Mutex::new(EqMode::default()),
        dsp_input: Mutex::new(dsp_input),
        vol: AtomicU32::new(1.0f32.to_bits()),
        rg_gain: AtomicU32::new(1.0f32.to_bits()),
        next_src: Mutex::new(None),
        seg_starts: Mutex::new(vec![0]),
    });
    (shared, dsp_output)
}

fn start_session(
    engine: &Engine,
) -> (
    Arc<Shared>,
    mpsc::Receiver<Cmd>,
    triple_buffer::Output<DspSnapshot>,
) {
    stop_current(engine);
    let (shared, dsp_output) = new_shared();
    *engine.shared.0.lock().unwrap() = Some(shared.clone());
    let (tx, rx) = mpsc::channel();
    *engine.cmd.lock().unwrap() = Some(tx);
    (shared, rx, dsp_output)
}

fn stub() -> EngineStatus {
    EngineStatus {
        playing: true,
        pos_ms: 0,
        dur_ms: 0,
        buffered_ms: 0,
        rate: 0,
        channels: 0,
        device: String::new(),
        src_rate: 0,
        dev_rate: 0,
        bits: 0,
        codec: String::new(),
        seg: 0,
    }
}

/// Play a local file.
#[tauri::command]
pub fn engine_play(path: String, engine: tauri::State<Engine>) -> EngineStatus {
    let (shared, rx, dsp_out) = start_session(&engine);
    let dev = engine.device_pref.lock().unwrap().clone();
    std::thread::spawn(move || decode_and_play(Source::File(path), shared, rx, dev, dsp_out));
    stub()
}

/// Play a remote URL (Navidrome stream) — downloaded + decoded natively (bit-perfect).
#[tauri::command]
pub fn engine_play_url(url: String, engine: tauri::State<Engine>) -> EngineStatus {
    let (shared, rx, dsp_out) = start_session(&engine);
    let dev = engine.device_pref.lock().unwrap().clone();
    std::thread::spawn(move || decode_and_play(Source::Url(url), shared, rx, dev, dsp_out));
    stub()
}

/// Play a cached offline track by Subsonic track ID (Pro only).
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_play_cached(
    track_id: String,
    plain_len: u64,
    engine: tauri::State<Engine>,
) -> EngineStatus {
    let (shared, rx, dsp_out) = start_session(&engine);
    let dev = engine.device_pref.lock().unwrap().clone();
    std::thread::spawn(move || {
        decode_and_play(
            Source::Cached {
                track_id,
                plain_len,
            },
            shared,
            rx,
            dev,
            dsp_out,
        )
    });
    stub()
}

/// List available output devices (DACs) by name.
#[tauri::command]
pub fn engine_list_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|ds| ds.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// Choose the output device by name (None / empty = system default).
#[tauri::command]
pub fn engine_set_device(name: Option<String>, engine: tauri::State<Engine>) {
    *engine.device_pref.lock().unwrap() = name.filter(|s| !s.is_empty());
}

fn send(engine: &Engine, cmd: Cmd) {
    if let Some(tx) = engine.cmd.lock().unwrap().as_ref() {
        let _ = tx.send(cmd);
    }
}

/// Pause playback.
#[tauri::command]
pub fn engine_pause(engine: tauri::State<Engine>) {
    send(&engine, Cmd::Pause);
}

/// Resume playback from the current position.
#[tauri::command]
pub fn engine_resume(engine: tauri::State<Engine>) {
    send(&engine, Cmd::Resume);
}

/// Seek to `secs` seconds from the beginning of the track.
#[tauri::command]
pub fn engine_seek(secs: f64, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        let rate = sh.rate.load(Ordering::SeqCst);
        let clamped = if secs.is_nan() {
            0.0
        } else {
            secs.clamp(0.0, 1.0e7)
        };
        let frame = (clamped * rate as f64) as usize;
        send(&engine, Cmd::Seek(frame));
    }
}

/// Stop playback and tear down the current session.
#[tauri::command]
pub fn engine_stop(engine: tauri::State<Engine>) {
    stop_current(&engine);
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        sh.playing.store(false, Ordering::SeqCst);
    }
}

/// Update the 10-band graphic EQ parameters.
#[tauri::command]
pub fn engine_set_eq(enabled: bool, preamp: f64, gains: Vec<f32>, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        {
            let mut e = sh.eq.lock().unwrap();
            e.enabled = enabled;
            e.preamp = preamp as f32;
            for i in 0..10 {
                e.gains[i] = gains.get(i).copied().unwrap_or(0.0);
            }
        }
        let snapshot = DspSnapshot {
            mode: *sh.eq_mode.lock().unwrap(),
            graphic: sh.eq.lock().unwrap().clone(),
            #[cfg(feature = "pro")]
            parametric: sh.param_eq.lock().unwrap().clone(),
        };
        sh.dsp_input.lock().unwrap().write(snapshot);
    }
}

/// Switch which EQ mode is routed to the DSP path.
#[tauri::command]
pub fn engine_set_eq_mode(mode: EqMode, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        *sh.eq_mode.lock().unwrap() = mode;
        let snapshot = DspSnapshot {
            mode,
            graphic: sh.eq.lock().unwrap().clone(),
            #[cfg(feature = "pro")]
            parametric: sh.param_eq.lock().unwrap().clone(),
        };
        sh.dsp_input.lock().unwrap().write(snapshot);
    }
}

/// Set the parametric EQ configuration (Pro only).
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_set_param_eq(
    app: tauri::AppHandle,
    enabled: bool,
    preamp: f64,
    bands: Vec<crate::pro::param_eq::ParamBand>,
    engine: tauri::State<Engine>,
) -> Result<(), String> {
    // Pro gate (defense-in-depth): the parametric EQ must not apply without a license,
    // so a frontend-only patch (forcing useIsPro) can't enable it. Mirrors offline.rs.
    if crate::pro::license::compute_status(&app).tier == crate::pro::license::Tier::Free {
        return Err("EKO Pro is required for the parametric EQ.".to_string());
    }
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        {
            let mut p = sh.param_eq.lock().unwrap();
            p.enabled = enabled;
            p.preamp = preamp as f32;
            p.bands = [None; crate::pro::param_eq::MAX_PARAM_BANDS];
            let count = bands.len().min(crate::pro::param_eq::MAX_PARAM_BANDS);
            for (i, b) in bands
                .into_iter()
                .take(crate::pro::param_eq::MAX_PARAM_BANDS)
                .enumerate()
            {
                p.bands[i] = Some(b);
            }
            p.count = count;
        }
        let snapshot = DspSnapshot {
            mode: *sh.eq_mode.lock().unwrap(),
            graphic: sh.eq.lock().unwrap().clone(),
            parametric: sh.param_eq.lock().unwrap().clone(),
        };
        sh.dsp_input.lock().unwrap().write(snapshot);
    }
    Ok(())
}

/// Compute the parametric EQ frequency-response curve for the on-screen preview
/// (Pro only). Returns `CURVE_POINTS + 1` summed-magnitude values (dB, clamped to
/// ±20) over a log-spaced grid from 20 Hz to 20 kHz at 48 kHz. Built from the SAME
/// `Biquad` coefficients the audio path uses, so the curve can't drift from what's
/// audible — there is no separate response math anywhere else.
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_eq_curve(bands: Vec<crate::pro::param_eq::ParamBand>, preamp: f64) -> Vec<f32> {
    const CURVE_POINTS: usize = 200;
    const F_MIN: f32 = 20.0;
    const F_MAX: f32 = 20_000.0;
    const FS: f32 = 48_000.0;
    const DB_RANGE: f32 = 20.0;

    let preamp = preamp as f32;
    // Precompute coefficients once per active band (disabled / zero-gain bands
    // contribute nothing, exactly as the audio cascade skips them).
    let coeffs: Vec<Biquad> = bands
        .iter()
        .filter(|b| b.is_active())
        .map(|b| b.coeffs(FS))
        .collect();

    (0..=CURVE_POINTS)
        .map(|i| {
            let frac = i as f32 / CURVE_POINTS as f32;
            let f = F_MIN * (F_MAX / F_MIN).powf(frac);
            let db = coeffs
                .iter()
                .fold(preamp, |acc, c| acc + c.magnitude_db(f, FS));
            db.clamp(-DB_RANGE, DB_RANGE)
        })
        .collect()
}

/// Parse an AutoEQ `ParametricEQ.txt` text and return the bands + preamp (Pro only).
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_parse_autoeq(
    app: tauri::AppHandle,
    text: String,
) -> Result<serde_json::Value, String> {
    if crate::pro::license::compute_status(&app).tier == crate::pro::license::Tier::Free {
        return Err("EKO Pro is required for AutoEQ import.".to_string());
    }
    let (preamp, bands) = crate::pro::param_eq::parse_autoeq(&text)?;
    let v = serde_json::json!({ "preamp": preamp, "bands": bands });
    Ok(v)
}

/// Read an AutoEQ `ParametricEQ.txt` file at the given absolute path and parse it (Pro only).
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_import_autoeq_file(
    app: tauri::AppHandle,
    path: String,
) -> Result<serde_json::Value, String> {
    if crate::pro::license::compute_status(&app).tier == crate::pro::license::Tier::Free {
        return Err("EKO Pro is required for AutoEQ import.".to_string());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file '{}': {}", path, e))?;
    let (preamp, bands) = crate::pro::param_eq::parse_autoeq(&text)?;
    let v = serde_json::json!({ "preamp": preamp, "bands": bands });
    Ok(v)
}

/// Set playback volume from the dial position (0..1).
#[tauri::command]
pub fn engine_set_volume(vol: f64, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        let v = vol.clamp(0.0, 1.0) as f32;
        let gain = v * v;
        sh.vol.store(gain.to_bits(), Ordering::Relaxed);
    }
}

/// Apply a ReplayGain adjustment, in dB, as an output gain (off by default).
#[tauri::command]
pub fn engine_set_replaygain(gain_db: Option<f32>, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        sh.rg_gain
            .store(rg_db_to_linear(gain_db).to_bits(), Ordering::Relaxed);
    }
}

/// Queue the next track for gapless continuation (free build: file + URL only).
#[cfg(not(feature = "pro"))]
#[tauri::command]
pub fn engine_enqueue(path: Option<String>, url: Option<String>, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        let src = match (path, url) {
            (Some(p), _) if !p.is_empty() => Some(Source::File(p)),
            (_, Some(u)) if !u.is_empty() => Some(Source::Url(u)),
            _ => None,
        };
        *sh.next_src.lock().unwrap() = src;
    }
}

/// Queue the next track for gapless continuation (Pro build: file + URL + cached).
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_enqueue(
    path: Option<String>,
    url: Option<String>,
    track_id: Option<String>,
    plain_len: Option<u64>,
    engine: tauri::State<Engine>,
) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        let src = match (path, url, track_id, plain_len) {
            (Some(p), _, _, _) if !p.is_empty() => Some(Source::File(p)),
            (_, Some(u), _, _) if !u.is_empty() => Some(Source::Url(u)),
            (_, _, Some(id), Some(pl)) if !id.is_empty() => Some(Source::Cached {
                track_id: id,
                plain_len: pl,
            }),
            _ => None,
        };
        *sh.next_src.lock().unwrap() = src;
    }
}

/// Store the current track metadata for the mini player to read.
#[tauri::command]
pub fn engine_set_now_playing(np: NowPlaying, engine: tauri::State<Engine>) {
    *engine.now.lock().unwrap() = np;
}

/// Read the current track metadata (used by the mini-player window).
#[tauri::command]
pub fn engine_now_playing(engine: tauri::State<Engine>) -> NowPlaying {
    engine.now.lock().unwrap().clone()
}

/// Return the latest 32-band spectrum magnitudes (0.0–1.0, log-spaced).
#[tauri::command]
pub fn engine_bands(engine: tauri::State<Engine>) -> Vec<f32> {
    let guard = engine.shared.0.lock().unwrap();
    guard
        .as_ref()
        .map(|sh| sh.bands.lock().unwrap().clone())
        .unwrap_or_default()
}

/// Return an [`EngineStatus`] snapshot for the currently-active session.
#[tauri::command]
pub fn engine_status(engine: tauri::State<Engine>) -> Option<EngineStatus> {
    let guard = engine.shared.0.lock().unwrap();
    let sh = guard.as_ref()?;
    let rate = sh.rate.load(Ordering::SeqCst).max(1);
    let ch = sh.channels.load(Ordering::SeqCst).max(1);
    let pos = sh.pos.load(Ordering::SeqCst);
    let total = sh.total.load(Ordering::SeqCst);
    let playing = sh.playing.load(Ordering::SeqCst) && !sh.paused.load(Ordering::SeqCst);
    let device = sh.device.lock().unwrap().clone();
    let codec = sh.codec.lock().unwrap().clone();
    let src_rate = sh.src_rate.load(Ordering::SeqCst);
    let dev_rate = sh.dev_rate.load(Ordering::SeqCst);
    let bits = sh.bits.load(Ordering::SeqCst);

    let starts = sh.seg_starts.lock().unwrap().clone();
    let seg = segment_at(&starts, pos);
    let track_start = starts[seg];
    let track_total = match starts.get(seg + 1) {
        Some(&next_start) => next_start.saturating_sub(track_start),
        None => total.saturating_sub(track_start),
    };
    let pos_in = pos.saturating_sub(track_start) as u64;
    let denom = rate as u64 * ch as u64;
    // Decoded-so-far point within the current track (the "buffered" extent on the scrubber).
    let buffered = sh.samples.lock().unwrap().len();
    let buffered_in = buffered.saturating_sub(track_start).min(track_total) as u64;

    Some(EngineStatus {
        playing,
        pos_ms: pos_in * 1000 / denom,
        dur_ms: track_total as u64 * 1000 / denom,
        buffered_ms: buffered_in * 1000 / denom,
        rate,
        channels: ch,
        device,
        src_rate,
        dev_rate,
        bits,
        codec,
        seg: seg as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remix_passthrough_when_equal() {
        let d = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(remix(&d, 2, 2), d);
    }

    #[test]
    fn remix_mono_to_stereo_duplicates() {
        let mono = vec![0.5, -0.5];
        assert_eq!(remix(&mono, 1, 2), vec![0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn resample_identity_when_equal() {
        let d = vec![0.0, 1.0, 0.0, -1.0];
        assert_eq!(resample(&d, 1, 48_000, 48_000), d);
    }

    #[test]
    fn resample_doubles_length_on_2x() {
        let d: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let up = resample(&d, 1, 1000, 2000);
        assert!((up.len() as i32 - 200).abs() <= 2, "len={}", up.len());
        assert!(up[0].abs() < 1e-3);
    }

    #[test]
    fn biquad_0db_is_unity() {
        let bq = Biquad::peaking(1000.0, 1.0, 0.0, 48_000.0);
        let mut z = (0.0f32, 0.0f32);
        let mut y = 0.0;
        for _ in 0..2000 {
            y = bq.process(&mut z, 1.0);
        }
        assert!((y - 1.0).abs() < 1e-3, "y={y}");
    }

    #[test]
    fn biquad_boost_differs_from_flat() {
        let flat = Biquad::peaking(1000.0, 1.0, 0.0, 48_000.0);
        let boost = Biquad::peaking(1000.0, 1.0, 12.0, 48_000.0);
        assert!((boost.b0 - flat.b0).abs() > 1e-3);
    }

    #[test]
    fn eq_active_only_when_shaping() {
        let mut e = EqParams::default();
        assert!(!e.active(), "flat eq must bypass (bit-perfect)");
        e.gains[3] = 3.0;
        assert!(e.active());
        e.gains[3] = 0.0;
        e.preamp = -2.0;
        assert!(e.active());
        e.preamp = 0.0;
        e.enabled = false;
        e.gains[0] = 5.0;
        assert!(!e.active(), "disabled eq must bypass");
    }

    #[test]
    fn bitperfect_only_when_nothing_touches_samples() {
        assert!(is_bitperfect(false, 1.0, 1.0));
        assert!(!is_bitperfect(true, 1.0, 1.0));
        assert!(!is_bitperfect(false, 0.5, 1.0));
        assert!(!is_bitperfect(false, 1.0, 0.5));
        assert!(!is_bitperfect(false, 1.0, 1.9));
    }

    #[test]
    fn replaygain_off_is_unity() {
        assert_eq!(rg_db_to_linear(None), 1.0);
        assert_eq!(rg_db_to_linear(Some(0.0)), 1.0);
    }

    #[test]
    fn replaygain_db_to_linear_matches_formula() {
        assert!(
            (rg_db_to_linear(Some(-6.0206)) - 0.5).abs() < 1e-3,
            "−6 dB ≈ 0.5"
        );
        assert!(
            (rg_db_to_linear(Some(6.0206)) - 2.0).abs() < 1e-3,
            "+6 dB ≈ 2.0"
        );
    }

    #[test]
    fn segment_at_maps_position_to_track() {
        assert_eq!(segment_at(&[0], 0), 0);
        assert_eq!(segment_at(&[0], 999), 0);
        let starts = [0usize, 100, 250];
        assert_eq!(segment_at(&starts, 0), 0);
        assert_eq!(segment_at(&starts, 99), 0);
        assert_eq!(segment_at(&starts, 100), 1);
        assert_eq!(segment_at(&starts, 249), 1);
        assert_eq!(segment_at(&starts, 250), 2);
        assert_eq!(segment_at(&starts, 10_000), 2);
    }

    #[test]
    fn replaygain_clamps_extremes() {
        assert_eq!(rg_db_to_linear(Some(100.0)), 4.0);
        let low = rg_db_to_linear(Some(-300.0));
        assert!((0.0..=4.0).contains(&low) && low.is_finite());
    }

    #[test]
    fn httpsource_reads_then_eof() {
        let src_buf = Arc::new(Mutex::new(vec![1u8, 2, 3, 4]));
        let done = Arc::new(AtomicBool::new(true));
        let mut src = HttpSource {
            buf: src_buf,
            done,
            pos: 0,
            stall_count: 0,
        };
        let mut out = [0u8; 3];
        assert_eq!(src.read(&mut out).unwrap(), 3);
        assert_eq!(&out, &[1, 2, 3]);
        let mut out2 = [0u8; 8];
        assert_eq!(src.read(&mut out2).unwrap(), 1);
        assert_eq!(out2[0], 4);
        assert_eq!(src.read(&mut out2).unwrap(), 0);
    }

    #[test]
    fn httpsource_seek_clamps() {
        let src_buf = Arc::new(Mutex::new(vec![0u8; 10]));
        let done = Arc::new(AtomicBool::new(true));
        let mut src = HttpSource {
            buf: src_buf,
            done,
            pos: 0,
            stall_count: 0,
        };
        assert_eq!(src.seek(SeekFrom::Start(4)).unwrap(), 4);
        assert_eq!(src.seek(SeekFrom::End(0)).unwrap(), 10);
        assert_eq!(src.seek(SeekFrom::Start(999)).unwrap(), 10);
        assert_eq!(src.seek(SeekFrom::Current(-3)).unwrap(), 7);
    }

    // ── Lock-free DSP snapshot tests (free mode) ─────────────────────────────

    #[test]
    fn dsp_snapshot_default_is_bitperfect() {
        let snap = DspSnapshot::default();
        assert!(!snap.eq_active());
        assert!(is_bitperfect(snap.eq_active(), 1.0, 1.0));
    }

    #[test]
    fn dsp_snapshot_triple_buffer_roundtrip() {
        use triple_buffer::triple_buffer;

        let (mut input, mut output) = triple_buffer(&DspSnapshot::default());
        {
            let snap = output.read();
            assert!(!snap.eq_active());
            assert!(is_bitperfect(snap.eq_active(), 1.0, 1.0));
        }

        let mut gains = [0.0f32; 10];
        gains[3] = 6.0;
        let active_snap = DspSnapshot {
            mode: EqMode::Graphic,
            graphic: EqParams {
                enabled: true,
                gains,
                ..Default::default()
            },
            #[cfg(feature = "pro")]
            parametric: crate::pro::param_eq::ParamEqParams::default(),
        };
        input.write(active_snap);

        {
            let snap = output.read();
            assert!(snap.eq_active());
            assert!(!is_bitperfect(snap.eq_active(), 1.0, 1.0));
        }

        input.write(DspSnapshot::default());
        {
            let snap = output.read();
            assert!(!snap.eq_active());
            assert!(is_bitperfect(snap.eq_active(), 1.0, 1.0));
        }
    }

    #[cfg(feature = "pro")]
    #[test]
    fn eq_curve_contract() {
        use crate::pro::param_eq::{ParamBand, ParamBandType};
        // Contract the frontend (`buildResponsePath`) relies on: a 201-point curve over
        // the log grid 20 Hz–20 kHz. If the point count or range drifts, the preview's
        // x-axis silently misaligns from the band markers.
        let flat = engine_eq_curve(vec![], 3.0);
        assert_eq!(flat.len(), 201, "curve must be CURVE_POINTS + 1 points");
        assert!(
            flat.iter().all(|&db| (db - 3.0).abs() < 1e-3),
            "a flat config should equal the preamp at every point"
        );

        // A +6 dB peak at 1 kHz lifts the curve to ~+6 at its centre and ~0 at the edges.
        let band = ParamBand {
            filter_type: ParamBandType::Peaking,
            freq: 1000.0,
            gain_db: 6.0,
            q: 1.0,
            enabled: true,
        };
        let curve = engine_eq_curve(vec![band], 0.0);
        let peak = curve.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            (peak - 6.0).abs() < 0.5,
            "peak gain {peak:.2} dB should be ≈ +6"
        );
        assert!(
            curve[0].abs() < 0.5 && curve[200].abs() < 0.5,
            "curve edges should be ≈ 0 dB (got {} / {})",
            curve[0],
            curve[200]
        );
    }
}
