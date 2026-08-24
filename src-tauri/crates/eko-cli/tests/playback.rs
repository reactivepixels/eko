//! Numeric proof that playback actually plays.
//!
//! Nothing in `cargo test` covers the CoreAudio path, and "it sounds right" is
//! not a thing an automated run can assert. This is the next best thing: it
//! generates a WAV, hands it to a real [`Engine`], opens the real default
//! output device, and measures three independent facts that are all false when
//! the audio path is broken.
//!
//! 1. `Engine::status().playing` is `true`
//! 2. `status().pos_ms` **advances** between two polls a second apart — which
//!    only happens if the output callback is consuming samples in real time
//! 3. `Engine::bands()` reports at least one non-zero magnitude — which only
//!    happens if the decoder produced PCM the FFT could read
//!
//! `#[ignore]`d on purpose: it claims the machine's default output device, so
//! it must not run in a normal `cargo test` or in CI. Run it deliberately:
//!
//! ```text
//! cargo test -p eko-cli --test playback -- --ignored --nocapture
//! ```
//!
//! It is **silent**. The engine volume is set to zero the instant the session
//! exists; the spectrum is computed from the decoded sample buffer rather than
//! from the output, so band 2 is still measurable with nothing coming out of
//! the speakers. Removing that line is the only way to hear it.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eko_core::engine::Engine;

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
const BITS: u16 = 16;

/// Write `secs` of a `freq` Hz sine as a 16-bit stereo WAV.
///
/// Hand-rolled rather than pulled from a crate: `symphonia` decodes plain PCM
/// WAV, and a 44-byte header is less dependency than a dependency.
fn write_sine_wav(path: &PathBuf, secs: f64, freq: f64, amplitude: f64) -> std::io::Result<()> {
    let frames = (SAMPLE_RATE as f64 * secs) as u32;
    let block_align = CHANNELS * BITS / 8;
    let byte_rate = SAMPLE_RATE * u32::from(block_align);
    let data_len = frames * u32::from(block_align);

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format: PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());

    for i in 0..frames {
        let t = f64::from(i) / f64::from(SAMPLE_RATE);
        let s = (amplitude * (2.0 * std::f64::consts::PI * freq * t).sin() * f64::from(i16::MAX))
            as i16;
        for _ in 0..CHANNELS {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }

    let mut file = std::fs::File::create(path)?;
    file.write_all(&out)?;
    Ok(())
}

/// A fresh 6-second 440 Hz WAV. Named per test, because the two tests run in
/// parallel and would otherwise truncate each other's fixture mid-read.
fn fixture(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("eko-cli-verify-440-{name}.wav"));
    write_sine_wav(&path, 6.0, 440.0, 0.5).expect("write the fixture WAV");
    path
}

/// Wait until `f` holds, or give up. Returns whether it held.
fn wait_for(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
#[ignore = "claims the default audio output device; run it deliberately"]
fn a_generated_sine_actually_plays_through_the_engine() {
    let path = fixture("sine");
    println!(
        "fixture: {} ({} bytes)",
        path.display(),
        std::fs::metadata(&path).unwrap().len()
    );

    let engine = Engine::default();
    engine.play(path.to_string_lossy().to_string());
    // Silent. See the module docs — the FFT reads decoded samples, not output.
    engine.set_volume(0.0);

    // ── 1. the engine reports it is playing ──────────────────────────────
    let up = wait_for(Duration::from_secs(5), || {
        engine.status().is_some_and(|s| s.playing && s.dur_ms > 0)
    });
    let status = engine.status().expect("a session");
    println!(
        "1. playing={} codec={} rate={} src_rate={} dev_rate={} bits={} device={:?} dur_ms={}",
        status.playing,
        status.codec,
        status.rate,
        status.src_rate,
        status.dev_rate,
        status.bits,
        status.device,
        status.dur_ms
    );
    assert!(up, "the engine never reported a playing session");
    assert!(status.playing, "status().playing was false");

    // ── 2. the position advances in real time ────────────────────────────
    let before = engine.status().unwrap().pos_ms;
    std::thread::sleep(Duration::from_millis(1_200));
    let after = engine.status().unwrap().pos_ms;
    println!(
        "2. pos_ms {before} -> {after} over ~1200ms (delta {})",
        after - before
    );
    assert!(
        after > before,
        "pos_ms did not advance: {before} -> {after} — the output callback is not consuming samples"
    );

    // ── 3. the spectrum has energy in it ─────────────────────────────────
    let bands = engine.bands();
    let peak = bands.iter().copied().fold(0.0f32, f32::max);
    let loudest = bands
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, v)| (i, *v));
    println!(
        "3. bands: {} values, peak {peak:.4}, loudest {loudest:?}",
        bands.len()
    );
    println!("   {bands:?}");
    assert!(!bands.is_empty(), "bands() returned nothing");
    assert!(peak > 0.0, "every band was zero — nothing was decoded");

    engine.stop();
    std::fs::remove_file(&path).ok();
}

#[test]
#[ignore = "claims the default audio output device; run it deliberately"]
fn pause_resume_and_seek_move_the_playhead() {
    let path = fixture("transport");
    let engine = Engine::default();
    engine.play(path.to_string_lossy().to_string());
    engine.set_volume(0.0);
    assert!(
        wait_for(Duration::from_secs(5), || engine
            .status()
            .is_some_and(|s| s.playing && s.pos_ms > 0)),
        "playback never started"
    );

    engine.pause();
    std::thread::sleep(Duration::from_millis(300));
    let paused_at = engine.status().unwrap().pos_ms;
    std::thread::sleep(Duration::from_millis(700));
    let still = engine.status().unwrap().pos_ms;
    println!("pause: {paused_at} -> {still} (should not move)");
    assert_eq!(paused_at, still, "the playhead moved while paused");
    assert!(
        !engine.status().unwrap().playing,
        "status().playing stayed true while paused"
    );

    engine.resume();
    assert!(
        wait_for(Duration::from_secs(3), || engine.status().unwrap().pos_ms
            > still),
        "the playhead did not move again after resume"
    );

    engine.seek(4.0);
    let sought = wait_for(Duration::from_secs(3), || {
        engine.status().unwrap().pos_ms >= 3_900
    });
    let pos = engine.status().unwrap().pos_ms;
    println!("seek to 4.0s: pos_ms {pos}");
    assert!(sought, "seek did not land: pos_ms {pos}");

    engine.stop();
    std::fs::remove_file(&path).ok();
}
