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
struct HttpSource {
    /// Shared byte buffer filled by the download thread.
    buf: Arc<Mutex<Vec<u8>>>,
    /// Set to `true` by the download thread when the response body is exhausted.
    done: Arc<AtomicBool>,
    /// Current read position within `buf` (in bytes).
    pos: usize,
}
impl Read for HttpSource {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            {
                let buf = self.buf.lock().unwrap();
                let avail = buf.len().saturating_sub(self.pos);
                if avail > 0 {
                    let n = avail.min(out.len());
                    out[..n].copy_from_slice(&buf[self.pos..self.pos + n]);
                    self.pos += n;
                    return Ok(n);
                }
                if self.done.load(Ordering::Relaxed) {
                    return Ok(0);
                }
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

/// Direct-form II transposed biquad filter — the standard second-order IIR section
/// used for all EQ bands. Coefficients follow the RBJ Audio EQ Cookbook notation
/// (`b0/b1/b2` = feed-forward, `a1/a2` = feedback; `a0` is normalised out).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}
impl Biquad {
    /// Compute RBJ peaking EQ coefficients.
    ///
    /// A peaking filter boosts or cuts by `gain_db` dB at `freq` Hz with bandwidth
    /// controlled by `q`. At `gain_db = 0.0` the filter is an identity (H(z) = 1).
    ///
    /// Formula: RBJ Audio EQ Cookbook, §"Peaking EQ filter".
    ///
    /// - `freq`    — centre frequency in Hz
    /// - `q`       — quality factor (narrowness of the bell); 1.0 is a gentle broad shape
    /// - `gain_db` — boost (+) or cut (−) in dB
    /// - `fs`      — sample rate in Hz
    fn peaking(freq: f32, q: f32, gain_db: f32, fs: f32) -> Biquad {
        // A = 10^(gain_dB/40) so that A² = linear amplitude ratio at the peak.
        let a = 10f32.powf(gain_db / 40.0);
        // ω₀ = 2π·freq/fs — the normalised angular frequency.
        let w0 = 2.0 * PI * freq / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        // α = sin(ω₀)/(2Q) — controls the bandwidth of the peak.
        let alpha = sw / (2.0 * q);
        // a0 is the normalisation denominator (divided out of all other coefficients).
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cw) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    /// Process one sample through the filter using direct-form II transposed state.
    ///
    /// `z` is the two-element delay state `(w[n-1], w[n-2])` which must be
    /// maintained between calls (one state pair per channel per band). Returns the
    /// filtered sample `y[n]`.
    #[inline]
    fn process(&self, z: &mut (f32, f32), x: f32) -> f32 {
        let y = self.b0 * x + z.0;
        z.0 = self.b1 * x - self.a1 * y + z.1;
        z.1 = self.b2 * x - self.a2 * y;
        y
    }
}
/// Build the 10 biquad coefficient sets for the current EQ gains at sample rate `fs`.
///
/// Called once at the start of each cpal callback invocation when the EQ is active.
/// Computing coefficients per-callback (rather than caching them) keeps the callback
/// code simple and the cost is negligible — 10 `powf` + trig calls takes < 1 µs.
fn eq_coeffs(gains: &[f32; 10], fs: u32) -> [Biquad; 10] {
    let mut c = [Biquad::default(); 10];
    for i in 0..10 {
        c[i] = Biquad::peaking(EQ_FREQS[i], 1.0, gains[i], fs as f32);
    }
    c
}

/// Decide whether the cpal callback may take the untouched-samples bypass.
///
/// Bit-perfect playback requires that *nothing* alters the decoded PCM: the EQ must be
/// inactive, software volume must be exactly unity, and any ReplayGain adjustment must be
/// exactly unity (off / 0 dB). Any deviation forces the per-sample DSP path. (On macOS the
/// device-rate match is enforced separately by [`crate::coreaudio`] before playback starts,
/// and surfaced via [`EngineStatus::dev_rate`].)
///
/// Extracted as a pure function so the invariant is unit-tested rather than living only
/// inline in the realtime callback.
#[inline]
fn is_bitperfect(eq_active: bool, vol: f32, rg_gain: f32) -> bool {
    !eq_active && vol == 1.0 && rg_gain == 1.0
}

/// Convert an optional ReplayGain value in dB to a clamped linear multiplier.
///
/// `None` or `Some(0.0)` → `1.0` (off / 0 dB → no change → the bit-perfect bypass stays
/// available). Otherwise `10^(dB/20)`, clamped to `0.0..=4.0` to guard against malformed
/// tags producing extreme or invalid gains.
fn rg_db_to_linear(gain_db: Option<f32>) -> f32 {
    match gain_db {
        Some(db) if db != 0.0 => 10f32.powf(db / 20.0).clamp(0.0, 4.0),
        _ => 1.0,
    }
}

/// A queued next source for gapless continuation, set via [`engine_enqueue`] and consumed
/// by the decode loop at end-of-track when its sample rate matches the open stream.
enum Source {
    File(String),
    Url(String),
}

/// Index of the segment (track) that contains interleaved sample position `pos`.
///
/// `starts` is the ascending list of per-track start offsets in the concatenated buffer
/// (always begins with `0`). Returns the index of the last start that is `<= pos`. Used to
/// report position/duration relative to the current track during gapless playback. For a
/// single track (`starts == [0]`) this is always `0`, so non-gapless behaviour is unchanged.
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

/// Commands sent from the Tauri command handlers to the decode/playback thread via an
/// `mpsc` channel. The thread drains all pending commands each loop iteration so that
/// a burst of seek events during a scrub collapses to the final position.
enum Cmd {
    Pause,
    Resume,
    /// Seek to the given output-rate frame index (not seconds — the sender converts).
    Seek(usize),
    Stop,
}

/// State shared between the Tauri command handlers, the decode thread, and the cpal
/// audio callback. All fields are individually synchronised (atomics or `Mutex`) so
/// that the three concurrent actors can read and write without a single coarse lock.
struct Shared {
    /// Current read position in `samples`, as an interleaved sample index.
    /// Written exclusively by the cpal callback; read by status queries and seek.
    pos: AtomicUsize,
    /// Estimated (during decode) or exact (after decode) total interleaved sample count.
    /// Used to compute `dur_ms` in [`EngineStatus`].
    total: AtomicUsize,
    /// Output sample rate in Hz — the rate cpal was opened at (may differ from
    /// `src_rate` if the hardware can't match the file's rate).
    rate: AtomicU32,
    /// Number of interleaved channels in `samples` and in the cpal output stream.
    channels: AtomicU32,
    /// The file's own native sample rate in Hz. When `src_rate == rate` AND
    /// `dev_rate == rate`, the full bit-perfect signal path is confirmed.
    src_rate: AtomicU32,
    /// The OS device's actual nominal rate after [`crate::coreaudio::match_device_rate`]
    /// runs. If this differs from `rate` macOS is resampling despite our best effort.
    dev_rate: AtomicU32,
    /// Bit depth of the source file (e.g. 16, 24, 32). `0` for lossy/compressed
    /// formats where the concept doesn't apply.
    bits: AtomicU32,
    /// Short codec identifier string (e.g. `"flac"`, `"mp3"`, `"aac"`, `"pcm"`).
    codec: Mutex<String>,
    /// `true` while the stream is paused (callback outputs silence).
    paused: AtomicBool,
    /// `false` once the track finishes or `engine_stop` is called.
    playing: AtomicBool,
    /// `true` once the decode thread has pushed the last packet into `samples`.
    done: AtomicBool,
    /// Display name of the cpal output device chosen for this track.
    device: Mutex<String>,
    /// Latest 32-band spectrum magnitudes (0.0–1.0), updated at ~30 fps by the
    /// decode thread's FFT loop and read by `engine_bands`.
    bands: Mutex<Vec<f32>>,
    /// Growing interleaved PCM buffer (f32, output rate/channels). Appended to by
    /// the decode thread; read (never mutated) by the cpal callback and the FFT.
    /// See the module-level docs for the realtime-safety discussion of this lock.
    samples: Mutex<Vec<f32>>,
    /// Current EQ parameters, written by `engine_set_eq` and snapshotted at the
    /// start of each cpal callback. Protected by a `Mutex` (not atomics) because
    /// `EqParams` is 44 bytes and needs to be read atomically as a unit.
    eq: Mutex<EqParams>,
    /// Playback gain stored as raw `f32` bits in an `AtomicU32` for lock-free access
    /// from the cpal callback. `1.0f32.to_bits()` = unity gain = bit-perfect bypass.
    vol: AtomicU32,
    /// ReplayGain adjustment as a linear multiplier, stored as raw `f32` bits in an
    /// `AtomicU32` for lock-free access from the cpal callback. `1.0` = off / 0 dB = no
    /// change, so the bit-perfect bypass remains available. Set via
    /// [`engine_set_replaygain`]; **off (unity) by default**.
    rg_gain: AtomicU32,
    /// Queued next track for gapless continuation, set by the frontend ~10 s before the
    /// current track ends. Consumed at decode-EOF only when its sample rate matches the open
    /// stream (else the frontend does a normal track change and the stream rebuilds).
    next_src: Mutex<Option<Source>>,
    /// Interleaved sample offsets where each track begins in `samples` (gapless segments).
    /// Always starts `[0]`; a boundary is pushed on each gapless continuation. Lets
    /// [`engine_status`] report position/duration — and the playing-track index — relative to
    /// the *current* track. A single track keeps this `[0]`, so non-gapless math is unchanged.
    seg_starts: Mutex<Vec<usize>>,
}

/// The Tauri-managed engine state, registered once at startup via `tauri::Builder::manage`.
///
/// A single `Engine` instance lives for the lifetime of the app. Each call to
/// `engine_play` / `engine_play_url` tears down the previous session and starts a fresh
/// one, replacing `cmd` and `shared` atomically under their respective `Mutex` guards.
#[derive(Default)]
pub struct Engine {
    /// Channel sender to the active decode thread. `None` when nothing is playing.
    cmd: Mutex<Option<Sender<Cmd>>>,
    /// Live shared state for the current track. Wrapped in an `Arc<Mutex<Option<…>>>`
    /// so that command handlers can safely check whether a session is active.
    shared: Arc<SharedHolder>,
    /// Current track metadata (title, artist, cover, theme, index). Written by the
    /// main window and read by the mini-player window directly from Rust, so it stays
    /// live even when the main window is hidden and its JS timers are throttled.
    now: Mutex<NowPlaying>,
    /// Preferred output device name (`None` = system default). Stored here so that
    /// `engine_set_device` can be called at any time and takes effect on the next track.
    device_pref: Mutex<Option<String>>,
}

/// Now-playing metadata, set by the main window and read by any window (e.g. the mini
/// player) directly from Rust — so it stays live even when the main window is hidden
/// and its JS timers are throttled by macOS.
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    /// Track title (from the library, not the file tags).
    pub title: String,
    /// Artist name.
    pub artist: String,
    /// Remote cover art URL (Navidrome cover endpoint), if available.
    pub cover_url: String,
    /// Local file path to the cover image, if available (used when offline).
    pub cover_path: String,
    /// UI theme token for this track's accent colour (e.g. `"amber"`, `"violet"`).
    pub theme: String,
    /// Zero-based index of this track in the active playlist.
    pub index: i64,
    /// Total number of tracks in the active playlist.
    pub total: i64,
}

#[derive(Default)]
struct SharedHolder(Mutex<Option<Arc<Shared>>>);

/// Snapshot of engine state returned by [`engine_status`] and polled by the UI at
/// ~4 Hz to update the progress bar, signal-path badge, and device display.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// `true` while audio is actively playing (i.e. not paused and not finished).
    pub playing: bool,
    /// Playback position in milliseconds.
    pub pos_ms: u64,
    /// Track duration in milliseconds (estimated during decode, exact afterwards).
    pub dur_ms: u64,
    /// Output sample rate in Hz — the rate cpal and the DAC are running at.
    pub rate: u32,
    /// Number of output channels.
    pub channels: u32,
    /// Display name of the active cpal output device.
    pub device: String,
    /// Native sample rate of the source file in Hz.
    ///
    /// When `src_rate == rate` the engine is not resampling internally.
    /// The UI uses `src_rate == rate == dev_rate` as the "bit-perfect confirmed" signal.
    pub src_rate: u32,
    /// Actual nominal rate of the OS output device in Hz, read back after
    /// [`crate::coreaudio::match_device_rate`] runs.
    ///
    /// When `dev_rate == rate` macOS is not resampling at the HAL layer.
    /// A mismatch (e.g. `dev_rate = 44100` while `rate = 96000`) means the OS
    /// couldn't switch — typically because the device doesn't support that rate.
    pub dev_rate: u32,
    /// Bit depth of the source file (16, 24, 32). `0` for compressed/lossy formats.
    pub bits: u32,
    /// Short codec name (e.g. `"flac"`, `"alac"`, `"mp3"`, `"aac"`, `"pcm"`).
    pub codec: String,
    /// Index of the track currently *playing* within this gapless session (0 = the track the
    /// session started on, +1 for each gaplessly-continued track). The frontend adds this to
    /// the queue index it started playback from to keep the UI in sync without a restart.
    /// Always `0` for normal (non-gapless) playback.
    pub seg: u32,
}

/// Re-channel `data` from `from`-channel interleaved PCM to `to`-channel interleaved PCM.
///
/// Strategy: when `to > from`, new channels are filled with the mono mix of the
/// source. When `to < from` or `from == to`, the data is returned as-is (no remix).
/// This keeps stereo content stereo on stereo devices without summation artefacts.
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
///
/// This is a fallback, not the normal path — when [`crate::coreaudio::match_device_rate`]
/// succeeds there is nothing to resample. Linear interpolation introduces audible
/// high-frequency roll-off on wideband content; a polyphase FIR would be cleaner but
/// this codepath should rarely activate in practice.
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
///
/// Iterates the device's supported configs looking for an F32 config that covers
/// `rate`. Falls back to the device's default config if no exact match exists —
/// in that case the `resample` pass will compensate, but OS-level resampling may
/// still occur and the signal-path seal will flag a mismatch.
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
// Index-based loops are clearer than iterator chains for this windowed-FFT + log-banding math.
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

/// Everything the decode loop needs for one source, produced by [`open_source`].
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

/// Open a local file or server URL: start any download, probe the container, and build a
/// decoder. Returns `None` on any failure. Used for both the first track and each gapless
/// continuation, so the two share one code path.
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
        Source::Url(url) => {
            // Stream the body into a shared buffer the decoder reads from (see `HttpSource`).
            let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
            let done = Arc::new(AtomicBool::new(false));
            {
                let (buf, done) = (buf.clone(), done.clone());
                std::thread::spawn(move || {
                    match reqwest::blocking::get(&url) {
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
            let s = HttpSource { buf, done, pos: 0 };
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

fn decode_and_play(
    first: Source,
    shared: Arc<Shared>,
    rx: mpsc::Receiver<Cmd>,
    device_name: Option<String>,
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

    // Bit-perfect: switch the OS device to the file's own rate so macOS doesn't resample.
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
    // The device's actual rate after the switch (macOS); if it can't match, the seal flags it.
    #[cfg(target_os = "macos")]
    let dev_rate = matched_rate.unwrap_or(out_rate);
    #[cfg(not(target_os = "macos"))]
    let dev_rate = out_rate;

    // Estimated total (for the duration readout) — corrected to the real length when done.
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
    let mut eq_state = [[(0.0f32, 0.0f32); MAX_CH]; 10];
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            if cb.paused.load(Ordering::Relaxed) {
                data.iter_mut().for_each(|s| *s = 0.0);
                return;
            }
            // Resolve EQ once per callback (bypassed entirely when flat → bit-perfect).
            let (active, pre, coeffs) = {
                let e = cb.eq.lock().unwrap();
                if e.active() {
                    (
                        true,
                        10f32.powf(e.preamp / 20.0),
                        eq_coeffs(&e.gains, out_rate),
                    )
                } else {
                    (false, 1.0, [Biquad::default(); 10])
                }
            };
            let vol = f32::from_bits(cb.vol.load(Ordering::Relaxed));
            let rg = f32::from_bits(cb.rg_gain.load(Ordering::Relaxed));
            // Bit-perfect path only when nothing touches the samples: EQ flat AND unity
            // volume AND ReplayGain off (see `is_bitperfect`).
            let bit_perfect = is_bitperfect(active, vol, rg);
            let buf = cb.samples.lock().unwrap();
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
                            if active {
                                x *= pre;
                                for b in 0..10 {
                                    x = coeffs[b].process(&mut eq_state[b][cc], x);
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
        |e| eprintln!("cpal stream: {e}"),
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
                // Gapless: if a same-rate next track is queued, keep decoding into the SAME
                // stream/buffer (the cpal callback reads one contiguous buffer, so there's no
                // seam and the bit-perfect path is untouched). Otherwise finalize normally.
                let mut continued = false;
                if let Some(src) = shared.next_src.lock().unwrap().take() {
                    if let Some(nd) = open_source(src) {
                        if nd.file_rate == out_rate {
                            let boundary = shared.samples.lock().unwrap().len();
                            shared.seg_starts.lock().unwrap().push(boundary);
                            // total stays the GLOBAL concatenated estimate; status derives the
                            // current track's duration from it minus the segment start.
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
                        // rate mismatch → drop nd; the frontend starts it as a normal track.
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

        // While decoding, don't block (keep filling); when done, idle on the channel.
        let timeout = if decoding {
            Duration::from_millis(0)
        } else {
            Duration::from_millis(33)
        };
        let mut stop = false;
        let mut first = rx.recv_timeout(timeout);
        // Drain the whole backlog this pass — a burst of seeks from a drag collapses to
        // the last one instead of being applied one-per-loop (which lags behind the user).
        loop {
            match first {
                Ok(Cmd::Pause) => shared.paused.store(true, Ordering::SeqCst),
                Ok(Cmd::Resume) => shared.paused.store(false, Ordering::SeqCst),
                Ok(Cmd::Seek(frame)) => {
                    // Seek is relative to the CURRENT track: map it into the concatenated
                    // buffer via the segment boundaries and clamp to this track's range.
                    let starts = shared.seg_starts.lock().unwrap().clone();
                    let cur = shared.pos.load(Ordering::SeqCst);
                    let i = segment_at(&starts, cur);
                    let track_start = starts[i];
                    let len = shared.samples.lock().unwrap().len();
                    let track_end = starts.get(i + 1).copied().unwrap_or(len);
                    let target = (track_start + frame * out_ch).min(track_end);
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
    // stream dropped here
}

fn stop_current(engine: &Engine) {
    if let Some(tx) = engine.cmd.lock().unwrap().take() {
        let _ = tx.send(Cmd::Stop);
    }
}

fn new_shared() -> Arc<Shared> {
    Arc::new(Shared {
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
        vol: AtomicU32::new(1.0f32.to_bits()),
        rg_gain: AtomicU32::new(1.0f32.to_bits()),
        next_src: Mutex::new(None),
        seg_starts: Mutex::new(vec![0]),
    })
}

fn start_session(engine: &Engine) -> (Arc<Shared>, mpsc::Receiver<Cmd>) {
    stop_current(engine);
    let shared = new_shared();
    *engine.shared.0.lock().unwrap() = Some(shared.clone());
    let (tx, rx) = mpsc::channel();
    *engine.cmd.lock().unwrap() = Some(tx);
    (shared, rx)
}

fn stub() -> EngineStatus {
    EngineStatus {
        playing: true,
        pos_ms: 0,
        dur_ms: 0,
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
    let (shared, rx) = start_session(&engine);
    let dev = engine.device_pref.lock().unwrap().clone();
    std::thread::spawn(move || decode_and_play(Source::File(path), shared, rx, dev));
    stub()
}

/// Play a remote URL (Navidrome stream) — downloaded + decoded natively (bit-perfect).
#[tauri::command]
pub fn engine_play_url(url: String, engine: tauri::State<Engine>) -> EngineStatus {
    let (shared, rx) = start_session(&engine);
    let dev = engine.device_pref.lock().unwrap().clone();
    std::thread::spawn(move || decode_and_play(Source::Url(url), shared, rx, dev));
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

/// Choose the output device by name (None / empty = system default). Applies to the next
/// track that starts (the frontend re-arms the current track for an immediate switch).
#[tauri::command]
pub fn engine_set_device(name: Option<String>, engine: tauri::State<Engine>) {
    *engine.device_pref.lock().unwrap() = name.filter(|s| !s.is_empty());
}

fn send(engine: &Engine, cmd: Cmd) {
    if let Some(tx) = engine.cmd.lock().unwrap().as_ref() {
        let _ = tx.send(cmd);
    }
}

/// Pause playback (the cpal callback outputs silence while paused).
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
///
/// Converts the timestamp to an output-rate frame index and enqueues a [`Cmd::Seek`].
/// The decode thread applies seeks in bulk (draining the command backlog each loop)
/// so scrubbing fast doesn't queue up stale positions.
#[tauri::command]
pub fn engine_seek(secs: f64, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        let rate = sh.rate.load(Ordering::SeqCst);
        let frame = (secs.max(0.0) * rate as f64) as usize;
        send(&engine, Cmd::Seek(frame));
    }
}
/// Stop playback and tear down the current session.
///
/// Sends [`Cmd::Stop`] to the decode thread (which exits its loop and drops the cpal
/// stream) and marks the shared state as not-playing so the UI clears immediately.
#[tauri::command]
pub fn engine_stop(engine: tauri::State<Engine>) {
    stop_current(&engine);
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        sh.playing.store(false, Ordering::SeqCst);
    }
}

/// Update the 10-band EQ parameters.
///
/// - `enabled` — master EQ on/off switch; when `false` the callback bypasses DSP entirely.
/// - `preamp`  — pre-amplifier gain in dB (typically −12..+12).
/// - `gains`   — per-band peak gain in dB, one value per [`EQ_FREQS`] centre frequency.
///   Missing values default to 0 dB (flat).
///
/// The new parameters take effect on the very next cpal callback invocation.
#[tauri::command]
pub fn engine_set_eq(enabled: bool, preamp: f64, gains: Vec<f32>, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        let mut e = sh.eq.lock().unwrap();
        e.enabled = enabled;
        e.preamp = preamp as f32;
        for i in 0..10 {
            e.gains[i] = gains.get(i).copied().unwrap_or(0.0);
        }
    }
}

/// Set playback volume from the dial position (0..1). Square-law taper for a natural
/// feel; exactly 1.0 at the top → unity gain → the callback stays bit-perfect.
#[tauri::command]
pub fn engine_set_volume(vol: f64, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        let v = vol.clamp(0.0, 1.0) as f32;
        let gain = v * v;
        sh.vol.store(gain.to_bits(), Ordering::Relaxed);
    }
}

/// Apply a ReplayGain adjustment, in dB, as an output gain (**off by default**).
///
/// `None` or `Some(0.0)` disables it (unity / 0 dB) — the bit-perfect bypass stays
/// available. A non-zero value applies `10^(dB/20)` (clamped to a sane range) as a linear
/// multiplier folded into the volume stage. Like any non-unity gain this takes playback off
/// the bit-perfect path, and the signal-path seal reflects that honestly. The frontend
/// chooses the value (track vs album gain, peak-limited) and calls this per track.
#[tauri::command]
pub fn engine_set_replaygain(gain_db: Option<f32>, engine: tauri::State<Engine>) {
    if let Some(sh) = engine.shared.0.lock().unwrap().as_ref() {
        sh.rg_gain
            .store(rg_db_to_linear(gain_db).to_bits(), Ordering::Relaxed);
    }
}

/// Queue the next track for gapless continuation. Pass either a local `path` or a server
/// `url`; the decode loop continues into it at end-of-track **only if its sample rate matches
/// the open stream** (otherwise it's left for the frontend to start as a normal track change,
/// rebuilding the stream — a tiny gap, but bit-perfect preserved). Passing neither clears it.
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
///
/// The values are updated by the decode thread at ~30 fps via the FFT loop in
/// `decode_and_play`. The UI polls this at the same cadence to animate the visualiser.
/// Returns an empty `Vec` when nothing is playing.
#[tauri::command]
pub fn engine_bands(engine: tauri::State<Engine>) -> Vec<f32> {
    let guard = engine.shared.0.lock().unwrap();
    guard
        .as_ref()
        .map(|sh| sh.bands.lock().unwrap().clone())
        .unwrap_or_default()
}

/// Return an [`EngineStatus`] snapshot for the currently-active session, or `None`
/// when no session exists.
///
/// The UI polls this at ~4 Hz to update the progress bar and signal-path badge.
/// All reads are from atomics or short `Mutex` critical sections and complete in
/// microseconds.
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

    // Report position/duration relative to the CURRENT gapless segment (track). For a single
    // track `seg_starts == [0]`, so `track_start == 0` and `track_total == total` — identical
    // to the pre-gapless math.
    let starts = sh.seg_starts.lock().unwrap().clone();
    let seg = segment_at(&starts, pos);
    let track_start = starts[seg];
    let track_total = match starts.get(seg + 1) {
        Some(&next_start) => next_start.saturating_sub(track_start),
        None => total.saturating_sub(track_start),
    };
    let pos_in = pos.saturating_sub(track_start) as u64;
    let denom = rate as u64 * ch as u64;

    Some(EngineStatus {
        playing,
        pos_ms: pos_in * 1000 / denom,
        dur_ms: track_total as u64 * 1000 / denom,
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
        assert!(up[0].abs() < 1e-3); // first sample preserved
    }

    #[test]
    fn biquad_0db_is_unity() {
        // A 0 dB peaking filter has H(z) = 1, so steady DC passes unchanged.
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
        // The sacred invariant: bypass ONLY with flat EQ, unity volume, and RG off.
        assert!(is_bitperfect(false, 1.0, 1.0));
        assert!(
            !is_bitperfect(true, 1.0, 1.0),
            "active EQ must not be bit-perfect"
        );
        assert!(
            !is_bitperfect(false, 0.5, 1.0),
            "non-unity volume must not be bit-perfect"
        );
        assert!(
            !is_bitperfect(false, 1.0, 0.5),
            "ReplayGain cut must not be bit-perfect"
        );
        assert!(
            !is_bitperfect(false, 1.0, 1.9),
            "ReplayGain boost must not be bit-perfect"
        );
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
        // single track → always segment 0 (non-gapless math unchanged)
        assert_eq!(segment_at(&[0], 0), 0);
        assert_eq!(segment_at(&[0], 999), 0);
        // gapless: boundaries at 0, 100, 250
        let starts = [0usize, 100, 250];
        assert_eq!(segment_at(&starts, 0), 0);
        assert_eq!(segment_at(&starts, 99), 0);
        assert_eq!(segment_at(&starts, 100), 1); // exactly on a boundary = the new track
        assert_eq!(segment_at(&starts, 249), 1);
        assert_eq!(segment_at(&starts, 250), 2);
        assert_eq!(segment_at(&starts, 10_000), 2); // past the end clamps to the last track
    }

    #[test]
    fn replaygain_clamps_extremes() {
        assert_eq!(
            rg_db_to_linear(Some(100.0)),
            4.0,
            "extreme boost clamps to 4.0"
        );
        let low = rg_db_to_linear(Some(-300.0));
        assert!(
            (0.0..=4.0).contains(&low) && low.is_finite(),
            "extreme cut stays finite & in range"
        );
    }

    #[test]
    fn httpsource_reads_then_eof() {
        let src_buf = Arc::new(Mutex::new(vec![1u8, 2, 3, 4]));
        let done = Arc::new(AtomicBool::new(true));
        let mut src = HttpSource {
            buf: src_buf,
            done,
            pos: 0,
        };
        let mut out = [0u8; 3];
        assert_eq!(src.read(&mut out).unwrap(), 3);
        assert_eq!(&out, &[1, 2, 3]);
        let mut out2 = [0u8; 8];
        assert_eq!(src.read(&mut out2).unwrap(), 1);
        assert_eq!(out2[0], 4);
        assert_eq!(src.read(&mut out2).unwrap(), 0); // EOF (download done)
    }

    #[test]
    fn httpsource_seek_clamps() {
        let src_buf = Arc::new(Mutex::new(vec![0u8; 10]));
        let done = Arc::new(AtomicBool::new(true));
        let mut src = HttpSource {
            buf: src_buf,
            done,
            pos: 0,
        };
        assert_eq!(src.seek(SeekFrom::Start(4)).unwrap(), 4);
        assert_eq!(src.seek(SeekFrom::End(0)).unwrap(), 10);
        assert_eq!(src.seek(SeekFrom::Start(999)).unwrap(), 10); // clamped to len
        assert_eq!(src.seek(SeekFrom::Current(-3)).unwrap(), 7);
    }
}
