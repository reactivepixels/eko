//! Proof that a stopped session frees its audio output, whichever device it played on.
//!
//! Stopping a session drops its cpal stream, and dropping the stream is what frees the
//! CoreAudio audio unit. cpal 0.15 kept any stream opened on a device chosen **by name**
//! alive through a reference cycle in its disconnect listener, so the unit was never freed:
//! the previous track went on playing under the next one, and each one kept its decoded
//! audio in memory. The system default device was unaffected, which is why it hid.
//!
//! `#[ignore]`d on purpose: it opens the machine's default output device (by name), so it
//! must not run in a normal `cargo test` or in CI. Run it deliberately, on a Mac:
//!
//! ```text
//! cargo test -p eko-core --test output_release -- --ignored --nocapture
//! ```
//!
//! It is silent: the file it plays is digital silence.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait};
use eko_core::engine::{live_output_streams, Engine};

/// Write `secs` of digital silence as a 16-bit stereo 44.1 kHz WAV.
fn write_silence(path: &Path, secs: u32) {
    let (rate, channels, bits) = (44_100u32, 2u16, 16u16);
    let block = channels * (bits / 8);
    let data_len = rate * secs * block as u32;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&channels.to_le_bytes()).unwrap();
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&(rate * block as u32).to_le_bytes()).unwrap();
    f.write_all(&block.to_le_bytes()).unwrap();
    f.write_all(&bits.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(&vec![0u8; data_len as usize]).unwrap();
}

/// Wait up to `limit` for `done`, checking every 50 ms.
fn wait_for(limit: Duration, done: impl Fn() -> bool) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

#[test]
#[ignore = "opens the default output device"]
fn stopping_frees_the_output_on_a_device_chosen_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("silence.wav");
    write_silence(&path, 20);
    let path = path.to_string_lossy().into_owned();

    let name = cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .expect("a default output device");
    let engine = Engine::default();
    engine.set_device(Some(name));

    let baseline = live_output_streams();
    let mut most = baseline;
    for _ in 0..4 {
        engine.play(path.clone());
        // Let the session actually open its output before the next one replaces it.
        let opened = wait_for(Duration::from_secs(10), || live_output_streams() > baseline);
        assert!(opened, "the session never opened an output stream");
        std::thread::sleep(Duration::from_millis(300));
        most = most.max(live_output_streams());
    }
    engine.stop();

    let released = wait_for(Duration::from_secs(10), || {
        live_output_streams() == baseline
    });
    assert!(
        released,
        "{} output stream(s) still alive after every session stopped (at most {} at once)",
        live_output_streams() - baseline,
        most - baseline
    );
}
