//! Device proof that a queue plays through on its own: two same-rate files joined
//! gaplessly, then a file at another rate handed to a new session, with nothing but the
//! player's own thread driving it.
//!
//! `#[ignore]`d on purpose: it claims the machine's default output device, so it must
//! not run in `cargo test` or in CI. Run it deliberately, on a Mac:
//!
//! ```text
//! cargo test -p eko-core --test queue_playback -- --ignored --nocapture
//! ```
//!
//! It is silent: the volume is set to zero before the first session exists, and a new
//! session starts from the settings already chosen.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eko_core::engine::{Engine, Source};
use eko_core::player::{ItemMedia, QueueItem, SourceResolver};
use eko_core::signal_path::ReplayGainTags;

/// Write `secs` of a 440 Hz sine as a 16-bit stereo WAV at `rate`.
fn write_wav(path: &Path, rate: u32, secs: f32) {
    let channels: u16 = 2;
    let bits: u16 = 16;
    let frames = (rate as f32 * secs) as u32;
    let block = channels * (bits / 8);
    let data_len = frames * block as u32;
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
    for i in 0..frames {
        let s = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8_000.0) as i16;
        for _ in 0..channels {
            f.write_all(&s.to_le_bytes()).unwrap();
        }
    }
}

struct Files;

impl SourceResolver for Files {
    fn resolve(&self, media: &ItemMedia) -> Option<Source> {
        match media {
            ItemMedia::File { path } => Some(Source::File(path.clone())),
            ItemMedia::Remote { .. } => None,
        }
    }
}

fn item(uid: &str, path: &Path) -> QueueItem {
    QueueItem {
        uid: uid.into(),
        media: ItemMedia::File {
            path: path.to_string_lossy().into_owned(),
        },
        title: uid.into(),
        artist: String::new(),
        album: String::new(),
        duration_ms: 1_500,
        cover_url: String::new(),
        rg: ReplayGainTags::default(),
    }
}

#[test]
#[ignore = "claims the default output device"]
fn a_queue_plays_through_a_gapless_seam_and_a_rate_change_with_nobody_polling() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        dir.path().join("a.wav"),
        dir.path().join("b.wav"),
        dir.path().join("c.wav"),
    );
    write_wav(&a, 44_100, 1.5);
    write_wav(&b, 44_100, 1.5);
    write_wav(&c, 48_000, 1.5);

    let engine = Engine::default();
    engine.set_volume(0.0);
    engine.set_resolver(Arc::new(Files));
    engine.queue_sync(vec![item("a", &a), item("b", &b), item("c", &c)]);
    engine.player_play("a".into());

    // Watch with `status()` and `session_view()` only. Neither decides anything, so any
    // advance seen here was made by the player's own thread.
    let mut seen: Vec<String> = Vec::new();
    let mut c_rate = 0;
    let until = Instant::now() + Duration::from_secs(12);
    while Instant::now() < until {
        if let Some(st) = engine.status() {
            if !st.uid.is_empty() && seen.last() != Some(&st.uid) {
                seen.push(st.uid.clone());
            }
            if st.uid == "c" && st.src_rate > 0 {
                c_rate = st.src_rate;
            }
        }
        if seen.last().map(String::as_str) == Some("c") && !engine.session_view().exists {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(seen, ["a", "b", "c"]);
    assert_eq!(c_rate, 48_000);
    let snap = engine.player_poll();
    assert!(!snap.active, "the queue did not end");
    assert_eq!(snap.error, None);
}
