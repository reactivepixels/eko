//! The whole-track waveform overview — **local files only**.
//!
//! A track's envelope is the peak and the RMS of every slice of the *whole*
//! file — see [`Envelope`] for why it takes both, and why drawing only the
//! first produced a solid slab on real music. Producing one means decoding the
//! whole file. That is a bounded, cheap,
//! off-thread job for something on a local disk and an unbounded one for a
//! Navidrome stream: the client would have to download the entire track before
//! it could draw the first pixel of an overview of it. So a remote track has no
//! envelope, the visualiser shows the plain scrubber instead, and nothing on
//! screen claims otherwise.
//!
//! **Absent is honest. A fabricated envelope is not.** There is no synthesised
//! shape here, no "typical" waveform, and no envelope derived from the 32-band
//! spectrum of the few hundred milliseconds that happen to have played — all
//! three would be a picture of a track, drawn by something that has not read it.
//!
//! ## The shape of this module
//!
//! It is [`crate::art`] again, deliberately, down to the vocabulary: a [`Key`]
//! that says what is wanted, a [`Pane`] that holds what has arrived and the
//! generation counter that drops stale answers, a blocking [`load`] a test can
//! drive with no thread in the way, and a [`spawn`] that is four lines around
//! it. The generation-drop is the same rule [`crate::remote::spawn_albums`]
//! follows and exists for the same reason: a slow decode for track A landing
//! after track B has started must not paint A's shape under B's title.

use std::sync::atomic::{AtomicU64, Ordering};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// How many buckets a track is reduced to.
///
/// Fixed, and **not** a function of the terminal width, so a resize does not
/// throw the decode away — the renderer resamples this down to whatever the
/// visualiser's width happens to be. 512 is finer than any terminal this
/// program will be drawn in (a 4K terminal at a 5px font is ~700 columns) and
/// costs 2 KiB per track.
pub const BUCKETS: usize = 512;

/// The longest track an envelope is computed for.
///
/// A decode is O(track length), and a two-hour DJ set or an unsegmented
/// audiobook is minutes of CPU for a row of eighth-blocks nobody asked for.
/// Past this the answer is [`Outcome::Unavailable`] — the same answer a remote
/// stream gets, and the same plain scrubber on screen.
pub const MAX_SECS: u64 = 30 * 60;

/// What identifies one envelope.
///
/// The **same token the cover is keyed on** — `local:{path}` — so the two
/// caches agree about what "the current track" is by construction rather than
/// by two pieces of arithmetic that have to be kept in step. There is no `cols`
/// or `rows` here, unlike [`crate::art::Key`]: the envelope is resolution-
/// independent and a resize must not restart a decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub token: String,
}

/// A track's envelope: [`BUCKETS`] **peaks** and [`BUCKETS`] **RMS values**,
/// both in `0.0..=1.0` and both on the same scale.
///
/// # Two measurements, because one of them cannot be drawn
///
/// Peak amplitude is the right *measurement* of a slice and the wrong thing to
/// draw on its own. A bucket of this track is about half a second of a mastered
/// mix, and half a second of a mastered mix touches full scale: on
/// *Emergency On Planet Earth* 440 of the 512 buckets peak above seven eighths
/// of full height, and the median bucket peaks at 0.997. Drawn alone that is a
/// solid orange slab with a fade at each end — a true measurement that carries
/// no information, because every column of it says the same thing.
///
/// RMS over the same bucket is the same audio measured for *level* rather than
/// for *extremes*, and it keeps the dynamic range the peak spent: the same
/// track's buckets run 0.00 to 0.38 with a median of 0.26, and not one of them
/// is above seven eighths. So both are kept and both are drawn — the peak as
/// the outline, the RMS as the solid fill inside it, which is what every DAW
/// and SoundCloud draw and for exactly this reason.
///
/// **Both are divided by the same number** — the loudest peak in the track —
/// so the fill is always inside the outline by construction and the gap between
/// them is the slice's real crest factor rather than an artefact of two scales.
#[derive(Clone, PartialEq)]
pub struct Envelope {
    peaks: Vec<f32>,
    rms: Vec<f32>,
}

/// `Debug` prints the lengths, not a thousand floats.
impl std::fmt::Debug for Envelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Envelope")
            .field("peaks", &self.peaks.len())
            .field("rms", &self.rms.len())
            .finish()
    }
}

impl Envelope {
    /// The peak buckets, left to right.
    ///
    /// `#[cfg(test)]`: the renderer reads [`Envelope::peak_column`], because a
    /// terminal column is never one bucket. This exists so the decode can be
    /// asserted about at its own resolution rather than through the reduction.
    #[cfg(test)]
    #[must_use]
    pub fn peaks(&self) -> &[f32] {
        &self.peaks
    }

    /// The RMS buckets, left to right. `#[cfg(test)]`, as [`Envelope::peaks`].
    #[cfg(test)]
    #[must_use]
    pub fn rms(&self) -> &[f32] {
        &self.rms
    }

    /// Build one from bucket values.
    ///
    /// `#[cfg(test)]`, and deliberately the **only** constructor outside
    /// [`load`]: the one way to obtain an envelope in a shipped binary is to
    /// have decoded a file, which is the whole claim this module makes.
    #[cfg(test)]
    #[must_use]
    pub fn from_parts(peaks: Vec<f32>, rms: Vec<f32>) -> Self {
        Self { peaks, rms }
    }

    /// The peak of the slice of the track that maps to column `col` of `width`.
    ///
    /// **A maximum over the slice, never a sample of its first bucket.** At 76
    /// columns each column covers between six and seven buckets, and taking one
    /// of them would drop transients on the floor — a snare that lands in a
    /// skipped bucket simply would not be in the picture. The width is the
    /// terminal's; the resolution is the file's, and the reduction between them
    /// keeps every peak the decode found.
    #[must_use]
    pub fn peak_column(&self, col: u16, width: u16) -> f32 {
        Self::reduce(&self.peaks, col, width, |slice| {
            slice.iter().copied().fold(0.0f32, f32::max)
        })
    }

    /// The RMS of the slice of the track that maps to column `col` of `width`.
    ///
    /// **A quadratic mean, not a maximum and not an arithmetic one.** The
    /// buckets are equal-length by construction, so the root of the mean of
    /// their squares *is* the RMS of the slice they cover — the same
    /// measurement as [`load`] made, at a coarser resolution, rather than a
    /// second statistic derived from the first.
    ///
    /// A maximum here would be the mistake this whole type exists to correct,
    /// one level down: taking the loudest of six buckets pushes every column
    /// towards the loudest thing anywhere near it and flattens the fill again.
    /// An arithmetic mean would under-report a column that holds one loud
    /// bucket and five quiet ones, which is a real thing a column can hold.
    #[must_use]
    pub fn rms_column(&self, col: u16, width: u16) -> f32 {
        Self::reduce(&self.rms, col, width, |slice| {
            let sum: f32 = slice.iter().map(|v| v * v).sum();
            (sum / slice.len() as f32).sqrt()
        })
    }

    /// The buckets column `col` of `width` covers, folded by `f`.
    ///
    /// One range arithmetic for both readings, so the fill and the outline can
    /// never be reduced over different slices of the track.
    fn reduce(buckets: &[f32], col: u16, width: u16, f: impl Fn(&[f32]) -> f32) -> f32 {
        if width == 0 || buckets.is_empty() {
            return 0.0;
        }
        let n = buckets.len();
        let lo = (usize::from(col) * n) / usize::from(width);
        let hi = ((usize::from(col) + 1) * n / usize::from(width)).max(lo + 1);
        let slice = &buckets[lo.min(n - 1)..hi.min(n).max(lo.min(n - 1) + 1)];
        f(slice)
    }
}

/// How one envelope request ended.
///
/// As in [`crate::art::Outcome`] there is no error *message*. A remote track, a
/// file that has been deleted since it was queued, a codec symphonia declines
/// and a track longer than [`MAX_SECS`] are the same fact on a one-row overview
/// — "no shape for this" — and the plain scrubber is what all four show.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Ready(Envelope),
    Unavailable,
}

/// One request's answer, stamped with the generation it was started under.
#[derive(Debug, Clone)]
pub struct Event {
    pub generation: u64,
    pub key: Key,
    pub outcome: Outcome,
}

/// Decode a local file and reduce it to [`BUCKETS`] peaks. **Blocking.**
///
/// Separated from [`spawn`] so a test can drive it against a generated WAV with
/// no thread in the way, exactly as [`crate::art::load`] is.
///
/// `cancelled` is polled once per packet — an atomic load against a few
/// thousand samples of work. A track skipped two seconds in should not go on
/// decoding to its end beside a realtime audio thread.
#[must_use]
pub fn load(path: &str, cancelled: &dyn Fn() -> bool) -> Outcome {
    let Ok(file) = std::fs::File::open(path) else {
        return Outcome::Unavailable;
    };
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_string);
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = &ext {
        hint.with_extension(e);
    }
    let Ok(probed) = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return Outcome::Unavailable;
    };
    let mut format = probed.format;
    let Some(track) = format.default_track() else {
        return Outcome::Unavailable;
    };
    let track_id = track.id;
    let params = track.codec_params.clone();
    let rate = u64::from(params.sample_rate.unwrap_or(0));

    // **The frame count is required, and is not guessed.** The bucket a frame
    // belongs to is `frame * BUCKETS / total`, so without a total there is no
    // mapping — and the alternatives (assume a length, or grow the buckets and
    // rescale at the end) both produce a picture whose horizontal axis does not
    // mean what the scrubber under it means. A container that does not know how
    // long it is gets no overview.
    let Some(total) = params.n_frames.filter(|&n| n > 0) else {
        return Outcome::Unavailable;
    };
    if rate > 0 && total / rate > MAX_SECS {
        return Outcome::Unavailable;
    }

    let Ok(mut decoder) =
        symphonia::default::get_codecs().make(&params, &DecoderOptions::default())
    else {
        return Outcome::Unavailable;
    };

    let mut peaks = vec![0.0f32; BUCKETS];
    // The RMS accumulator, and the frames each bucket actually received.
    //
    // `f64`, because a bucket of a five-minute track at 44.1 kHz is twenty-odd
    // thousand squares and an `f32` running sum loses the quiet ones under the
    // loud ones — the exact error that would flatten the fill this exists to
    // give shape to. The count is per bucket rather than `total / BUCKETS`
    // because a container's `n_frames` is allowed to be a padded estimate.
    let mut sumsq = vec![0.0f64; BUCKETS];
    let mut counts = vec![0u64; BUCKETS];
    let mut frame: u64 = 0;
    let mut sample_buf: Option<(SampleBuffer<f32>, usize)> = None;

    loop {
        if cancelled() {
            return Outcome::Unavailable;
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Any end — clean EOF, a truncated file, a reset — ends the walk
            // with whatever was read. See below for why that is still honest.
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A recoverable glitch: skip the packet and keep going. The frames
            // it held are simply never counted, which the completeness check
            // below is what catches if it happens often enough to matter.
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let want = decoded.capacity();
        // The first packet is not necessarily the largest — a container can end
        // on a short one and a Vorbis stream alternates block sizes — so the
        // scratch buffer is rebuilt whenever a packet needs more room than the
        // one it was sized for. `copy_interleaved_ref` panics rather than
        // truncating if it does not fit.
        let (buf, cap) =
            sample_buf.get_or_insert_with(|| (SampleBuffer::<f32>::new(want as u64, spec), want));
        if *cap < want {
            *buf = SampleBuffer::<f32>::new(want as u64, spec);
            *cap = want;
        }
        buf.copy_interleaved_ref(decoded);
        for chunk in buf.samples().chunks(channels) {
            // Two readings of one frame: the loudest channel, and the mean
            // square across them. The first is what the outline is made of and
            // the second is what the fill is made of — see [`Envelope`].
            let peak = chunk.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
            let mean_square = chunk.iter().map(|s| s * s).sum::<f32>() / channels as f32;
            let bucket = ((frame.min(total.saturating_sub(1))) as u128 * BUCKETS as u128
                / total as u128) as usize;
            let bucket = bucket.min(BUCKETS - 1);
            let slot = &mut peaks[bucket];
            if peak > *slot {
                *slot = peak;
            }
            sumsq[bucket] += f64::from(mean_square);
            counts[bucket] += 1;
            frame += 1;
        }
    }

    // **A decode that stopped early is not an envelope.** A file truncated
    // halfway produces a shape that is half real and half a flat floor, and a
    // flat floor beside a real waveform reads as *silence in the track* rather
    // than as *this program stopped reading*. One of those is a fact about the
    // music. Ten percent of slack for containers whose `n_frames` is a padded
    // estimate, and nothing beyond it.
    if frame * 10 < total * 9 {
        return Outcome::Unavailable;
    }
    if peaks.iter().all(|&p| p <= 0.0) {
        return Outcome::Unavailable;
    }

    // Normalise to the loudest peak, so a quiet master is a visible shape rather
    // than a flat line. This is a *display* normalisation of a picture and
    // touches no sample the engine will ever see — the scale of the drawing, not
    // a gain.
    //
    // **The RMS is divided by the same number**, never by the loudest RMS.
    // Rescaling the fill to its own maximum would make it reach the top of the
    // block and cross the outline it is supposed to sit inside, and the gap
    // between the two would stop being the crest factor of the music and start
    // being the ratio of two unrelated normalisations.
    let loudest = peaks.iter().copied().fold(0.0f32, f32::max);
    let rms = (0..BUCKETS)
        .map(|b| {
            if counts[b] == 0 {
                0.0
            } else {
                (((sumsq[b] / counts[b] as f64).sqrt() as f32) / loudest).clamp(0.0, 1.0)
            }
        })
        .collect();
    for p in &mut peaks {
        *p = (*p / loudest).clamp(0.0, 1.0);
    }
    Outcome::Ready(Envelope { peaks, rms })
}

/// Decode one track's envelope on a worker thread.
///
/// The generation stamp travels out and back exactly as
/// [`crate::art::spawn`]'s does; `emit` returning `false` means the fold has
/// hung up, and is also what stops the decode early.
pub fn spawn<F>(path: String, key: Key, generation: u64, live: &'static AtomicU64, emit: F)
where
    F: Fn(Event) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        let cancelled = || live.load(Ordering::Relaxed) != generation;
        let outcome = load(&path, &cancelled);
        emit(Event {
            generation,
            key,
            outcome,
        });
    });
}

/// The generation the fold currently wants an answer for.
///
/// A `static` rather than an `Arc<AtomicU64>` per request because there is
/// exactly one [`Pane`] in the process — one visualiser, one current track —
/// and a worker needs to be able to ask "am I still wanted?" without holding a
/// reference into [`crate::app::App`] across a thread boundary.
pub static LIVE: AtomicU64 = AtomicU64::new(0);

/// Where an envelope request has got to.
#[derive(Debug, Default, Clone, PartialEq)]
enum State {
    #[default]
    Idle,
    /// A worker is decoding. The visualiser shows the plain scrubber meanwhile;
    /// there is no spinner, because an overview that takes a moment is not an
    /// event.
    Loading,
    Ready(Envelope),
    /// Asked, and there is no shape: remote, unreadable, too long, truncated.
    Unavailable,
}

/// What is wanted, what has arrived, and the counter that drops stale answers.
///
/// [`crate::art::Pane`] with the protocol and the grid taken out. Kept as its
/// own type rather than generic over the payload: two caches that look alike
/// but drop staleness by *different* rules is the bug, and two hundred lines of
/// generics to share forty lines of matching would hide which rule each one
/// follows.
#[derive(Debug, Default)]
pub struct Pane {
    want: Option<Key>,
    state: State,
    generation: u64,
}

impl Pane {
    /// The key currently being asked for, if any.
    #[must_use]
    pub fn wants(&self) -> Option<&Key> {
        self.want.as_ref()
    }

    /// The generation an answer must carry to be believed.
    ///
    /// `#[cfg(test)]`: in a shipped binary the generation never leaves this
    /// type — [`Pane::begin`] hands it to the worker and [`Pane::accept`]
    /// checks it, and there is no third party with a reason to know it. A test
    /// that fakes a worker's answer is the only caller.
    #[cfg(test)]
    #[must_use]
    pub fn generation_for_test(&self) -> u64 {
        self.generation
    }

    /// The envelope, if one has arrived for what is currently wanted.
    #[must_use]
    pub fn envelope(&self) -> Option<&Envelope> {
        match &self.state {
            State::Ready(env) => Some(env),
            _ => None,
        }
    }

    /// Ask for `want`, abandoning whatever was in flight.
    ///
    /// Publishes the new generation to [`LIVE`] as it goes, which is what makes
    /// an abandoned decode stop reading rather than run to the end of a track
    /// nobody is playing any more.
    pub fn begin(&mut self, want: Option<Key>) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.state = if want.is_some() {
            State::Loading
        } else {
            State::Idle
        };
        self.want = want;
        LIVE.store(self.generation, Ordering::Relaxed);
        self.generation
    }

    /// Give up without a request having been made — a remote track, which has
    /// no envelope by construction.
    pub fn give_up(&mut self) {
        if self.want.is_some() {
            self.state = State::Unavailable;
        }
    }

    /// Fold one worker's answer in. `false` when it was stale and dropped.
    pub fn accept(&mut self, event: Event) -> bool {
        if event.generation != self.generation || self.want.as_ref() != Some(&event.key) {
            return false;
        }
        self.state = match event.outcome {
            Outcome::Ready(env) => State::Ready(env),
            Outcome::Unavailable => State::Unavailable,
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-bit mono WAV whose amplitude ramps from silence to full scale, so
    /// the envelope has a shape a test can state rather than eyeball.
    fn ramp_wav(rate: u32, secs: f64) -> Vec<u8> {
        let frames = (f64::from(rate) * secs) as u32;
        let mut data = Vec::with_capacity(frames as usize * 2);
        for i in 0..frames {
            let t = f64::from(i) / f64::from(frames);
            let amp = t * 0.9;
            let s = (amp * (i as f64 * 0.3).sin() * f64::from(i16::MAX)) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
        wav(rate, 1, &data)
    }

    fn wav(rate: u32, channels: u16, data: &[u8]) -> Vec<u8> {
        let block_align: u16 = channels * 2;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    fn write_temp(name: &str, bytes: &[u8]) -> String {
        let path = std::env::temp_dir().join(format!("eko-cli-wave-{name}.wav"));
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().to_string()
    }

    fn never() -> Box<dyn Fn() -> bool> {
        Box::new(|| false)
    }

    #[test]
    fn a_real_file_decodes_to_a_shape_that_matches_it() {
        let path = write_temp("ramp", &ramp_wav(8_000, 2.0));
        let Outcome::Ready(env) = load(&path, &*never()) else {
            panic!("the ramp did not decode");
        };
        assert_eq!(env.peaks().len(), BUCKETS);
        // Every bucket is in range, and the shape rises: this file gets louder,
        // and so does its picture.
        assert!(env.peaks().iter().all(|&p| (0.0..=1.0).contains(&p)));
        let first = env.peaks()[..64].iter().copied().fold(0.0f32, f32::max);
        let last = env.peaks()[BUCKETS - 64..]
            .iter()
            .copied()
            .fold(0.0f32, f32::max);
        assert!(last > first * 4.0, "first {first} last {last}");
        // Normalised: the loudest bucket is exactly full scale.
        let loudest = env.peaks().iter().copied().fold(0.0f32, f32::max);
        assert!((loudest - 1.0).abs() < 1e-6, "{loudest}");
        // The fill is the same length, on the same scale, and inside the
        // outline in every bucket — never rescaled to its own maximum.
        assert_eq!(env.rms().len(), BUCKETS);
        for (i, (&p, &r)) in env.peaks().iter().zip(env.rms()).enumerate() {
            assert!((0.0..=1.0).contains(&r), "bucket {i} rms {r}");
            assert!(r <= p + 1e-6, "bucket {i}: fill {r} outside outline {p}");
        }
        let _ = std::fs::remove_file(&path);
    }

    /// **The fill has the dynamic range the outline spent.**
    ///
    /// This is the whole correction, stated as numbers rather than as taste.
    /// The file is two passages that a peak meter cannot tell apart: four
    /// seconds of a full-scale square wave, then four seconds of full-scale
    /// impulses separated by silence. Every bucket of both peaks at full scale
    /// — which is what a mastered track does, and why the old envelope drew one
    /// flat slab — while the *level* of the two differs by five to one.
    ///
    /// So the assertion is in two halves: the outline is flat to within one
    /// percent across the whole file, and the fill spans most of the block.
    /// The second is the range the drawing gained.
    #[test]
    fn the_fill_keeps_a_range_the_outline_flattens_away() {
        const RATE: u32 = 8_000;
        const SECS: u32 = 4;
        let full = (0.9 * f64::from(i16::MAX)) as i16;
        let mut data = Vec::new();
        // Loud and dense: a full-scale square, so peak and RMS are both 0.9.
        for i in 0..RATE * SECS {
            let s = if (i / 20) % 2 == 0 { full } else { -full };
            data.extend_from_slice(&s.to_le_bytes());
        }
        // Loud and sparse: the same peak, one twenty-fifth of the energy.
        for i in 0..RATE * SECS {
            let s = if i % 25 == 0 { full } else { 0 };
            data.extend_from_slice(&s.to_le_bytes());
        }
        let path = write_temp("crest", &wav(RATE, 1, &data));
        let Outcome::Ready(env) = load(&path, &*never()) else {
            panic!("the crest fixture did not decode");
        };
        let _ = std::fs::remove_file(&path);

        // Away from the two ends and the seam, every bucket of both passages
        // peaks at full scale. A peak-only envelope draws this file as a slab.
        let interior = |lo: usize, hi: usize| &env.peaks()[lo..hi];
        let flat_lo = interior(8, 248)
            .iter()
            .chain(interior(264, 504))
            .copied()
            .fold(1.0f32, f32::min);
        assert!(
            flat_lo > 0.99,
            "the outline was not flat, so this fixture proves nothing: {flat_lo}"
        );

        // The fill, over the same buckets, is not flat at all.
        let dense = env.rms()[8..248].iter().copied().fold(0.0f32, f32::max);
        let sparse = env.rms()[264..504].iter().copied().fold(0.0f32, f32::max);
        assert!(
            dense > 0.9,
            "the dense passage should fill the block: {dense}"
        );
        assert!(sparse < 0.3, "the sparse passage should not: {sparse}");
        assert!(
            dense - sparse > 0.6,
            "the fill has no more range than the outline: {dense} vs {sparse}"
        );
        // And it is still inside the outline everywhere.
        for (i, (&p, &r)) in env.peaks().iter().zip(env.rms()).enumerate() {
            assert!(r <= p + 1e-6, "bucket {i}: fill {r} outside outline {p}");
        }
    }

    /// **Nothing is invented when the file cannot be read.** Four different
    /// failures, one answer, and it is never a shape.
    #[test]
    fn nothing_readable_means_no_envelope_rather_than_a_flat_one() {
        assert_eq!(
            load("/eko-cli-test/does-not-exist.flac", &*never()),
            Outcome::Unavailable
        );
        let garbage = write_temp("garbage", b"this is not audio, it is a sentence");
        assert_eq!(load(&garbage, &*never()), Outcome::Unavailable);
        let _ = std::fs::remove_file(&garbage);
        // Digital silence has no shape to draw, and a row of floor glyphs would
        // read as a shape.
        let silent = write_temp("silence", &wav(8_000, 1, &vec![0u8; 8_000 * 2]));
        assert_eq!(load(&silent, &*never()), Outcome::Unavailable);
        let _ = std::fs::remove_file(&silent);
    }

    /// A decode abandoned partway answers `Unavailable` rather than handing back
    /// the half it managed.
    #[test]
    fn an_abandoned_decode_hands_back_nothing_rather_than_half_a_track() {
        let path = write_temp("cancel", &ramp_wav(8_000, 20.0));
        assert_eq!(load(&path, &|| true), Outcome::Unavailable);
        let _ = std::fs::remove_file(&path);
    }

    /// The reduction to a terminal's width is a **maximum** for the outline, so
    /// a transient in any bucket survives being drawn at 76 columns — and a
    /// **quadratic mean** for the fill, so one loud bucket does not drag five
    /// quiet ones up to meet it.
    #[test]
    fn resampling_to_a_width_keeps_every_peak_and_averages_every_level() {
        let mut peaks = vec![0.1f32; BUCKETS];
        peaks[300] = 1.0;
        let mut rms = vec![0.1f32; BUCKETS];
        rms[300] = 1.0;
        let env = Envelope { peaks, rms };
        // Whichever column bucket 300 lands in must be full height, and no
        // column may exceed the peaks it covers.
        let tallest = (0..76u16)
            .map(|c| env.peak_column(c, 76))
            .fold(0.0f32, f32::max);
        assert!((tallest - 1.0).abs() < 1e-6, "the transient was dropped");
        for c in 0..76u16 {
            assert!(env.peak_column(c, 76) <= 1.0);
        }
        // The same bucket, reduced as a level: the column that holds it is
        // lifted but nowhere near full scale, because the other six buckets in
        // it are quiet and the fill says so. `sqrt((1 + 6×0.01) / 7) ≈ 0.39`.
        let loudest_fill = (0..76u16)
            .map(|c| env.rms_column(c, 76))
            .fold(0.0f32, f32::max);
        assert!(
            (0.3..0.5).contains(&loudest_fill),
            "the fill was reduced by a maximum, not a mean: {loudest_fill}"
        );
        // And every column's fill sits inside its own outline.
        for c in 0..76u16 {
            assert!(env.rms_column(c, 76) <= env.peak_column(c, 76) + 1e-6);
        }
        // Degenerate widths answer rather than panicking.
        assert_eq!(env.peak_column(0, 0), 0.0);
        assert_eq!(env.rms_column(0, 0), 0.0);
        assert!(env.peak_column(1000, 76) <= 1.0);
        assert!(env.rms_column(1000, 76) <= 1.0);
    }

    /// The staleness rule, stated the way [`crate::art::Pane`]'s is: an answer
    /// for the track before last is dropped, not drawn.
    #[test]
    fn an_answer_for_a_track_that_has_been_left_is_dropped() {
        let mut pane = Pane::default();
        let first = Key {
            token: "local:/a.flac".into(),
        };
        let g1 = pane.begin(Some(first.clone()));
        let g2 = pane.begin(Some(Key {
            token: "local:/b.flac".into(),
        }));
        assert_ne!(g1, g2);
        assert!(!pane.accept(Event {
            generation: g1,
            key: first,
            outcome: Outcome::Ready(Envelope {
                peaks: vec![1.0; BUCKETS],
                rms: vec![0.5; BUCKETS],
            }),
        }));
        assert!(pane.envelope().is_none());
        // And the live generation is the one a worker checks itself against.
        assert_eq!(LIVE.load(Ordering::Relaxed), g2);
    }

    #[test]
    fn a_remote_track_is_stated_as_having_no_shape_rather_than_left_loading() {
        let mut pane = Pane::default();
        pane.begin(Some(Key {
            token: "remote:42".into(),
        }));
        pane.give_up();
        assert!(pane.envelope().is_none());
        assert_eq!(pane.state, State::Unavailable);
    }
}
