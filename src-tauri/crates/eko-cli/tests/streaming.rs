//! Numeric proof that a **streamed** track actually plays, and that the seal
//! stays honest about it.
//!
//! The twin of `tests/playback.rs`, one layer out. That test proves the local
//! file path reaches a real output device; this one proves the same for
//! `Engine::play_url` — the path `eko-cli` takes for a Navidrome track — and then
//! proves the thing the product is actually for: that the signal-path seal
//! derived from a *stream* says what the rates say and nothing more.
//!
//! There is no Navidrome server here or in CI, so one is constructed: `mockito`
//! serves a generated WAV from `127.0.0.1`, and the engine downloads and decodes
//! it exactly as it would from a real server. Everything upstream of the socket —
//! URL signing, paging, parsing — is covered elsewhere and is not what this is
//! about.
//!
//! What is measured, in order:
//!
//! 1. `Engine::status().playing` is `true` and `pos_ms` **advances** between two
//!    polls about a second apart — which only happens if bytes arrived over HTTP,
//!    decoded, and were consumed by the output callback in real time.
//! 2. What `derive` returns at unity with a flat EQ — and, as the negative
//!    control, what it returns for an attenuated one. **`eko-cli` cannot produce
//!    an attenuated path**: software volume was removed, `App::volume` is pinned
//!    at unity and there is no key or config key behind it. The case is kept
//!    because it is what makes the unity assertion mean anything — a `derive`
//!    that returned `pure` for everything would pass the first check alone.
//! 3. **What the seal reads when the stream's rate differs from the device's.**
//!    It must be `RESAMPLED`. A streamed track the device resamples showing
//!    `BIT-PERFECT` is the single worst outcome available in this codebase.
//!
//! `#[ignore]`d on purpose: these claim the machine's default output device, so
//! they must not run in a normal `cargo test` or in CI. Run them deliberately:
//!
//! ```text
//! cargo test -p eko-cli --test streaming -- --ignored --nocapture
//! ```
//!
//! They are **silent** — engine volume goes to zero the instant the session
//! exists, and `pos_ms` is read from the output callback's own counter, so
//! nothing has to be audible for the measurement to mean something.
//!
//! ## `stream_info`, restated here
//!
//! `eko-cli` is a binary, so an integration test cannot call into it. The gate in
//! `app::stream_info` — *no `StreamInfo` until `rate`, `src_rate` and `dev_rate`
//! are all populated* — is therefore restated in [`gated_stream_info`] below,
//! with the same reasoning. If one moves, move the other: handed a half-filled
//! status raw, `derive` reads `active` (because `rate` is floored at 1) with both
//! resample checks skipped (they need `> 0`), i.e. **`BIT-PERFECT`**.

use std::time::{Duration, Instant};

use eko_core::engine::{Engine, EngineStatus};
use eko_core::signal_path::{self, EqState, RgMode, SealInput, SignalPath, StreamInfo};

const CHANNELS: u16 = 2;
const BITS: u16 = 16;

/// `secs` of a `freq` Hz sine as a 16-bit stereo WAV at `rate`.
///
/// Hand-rolled for the same reason `tests/playback.rs` hand-rolls its own: a
/// 44-byte header is less dependency than a dependency. `rate` is a parameter
/// here because the resample case turns on it.
fn sine_wav(rate: u32, secs: f64, freq: f64, amplitude: f64) -> Vec<u8> {
    let frames = (f64::from(rate) * secs) as u32;
    let block_align = CHANNELS * BITS / 8;
    let byte_rate = rate * u32::from(block_align);
    let data_len = frames * u32::from(block_align);

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format: PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());

    for i in 0..frames {
        let t = f64::from(i) / f64::from(rate);
        let s = (amplitude * (2.0 * std::f64::consts::PI * freq * t).sin() * f64::from(i16::MAX))
            as i16;
        for _ in 0..CHANNELS {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

/// A mock Navidrome answering `getAlbumList2`-shaped nonsense at every path and
/// the generated WAV at `/rest/stream`, plus the URL to play.
///
/// The guard has to outlive the playback, so it is returned rather than dropped.
fn serve(rate: u32) -> (mockito::ServerGuard, String) {
    let body = sine_wav(rate, 8.0, 440.0, 0.5);
    let mut server = mockito::Server::new();
    let url = format!("{}/rest/stream?id=tr-1&format=raw", server.url());
    server
        .mock("GET", "/rest/stream")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "audio/wav")
        .with_body(body)
        .expect_at_least(1)
        .create();
    (server, url)
}

/// The gate from `app::stream_info`, restated — see the module docs.
fn gated_stream_info(status: &EngineStatus) -> Option<StreamInfo> {
    if status.rate == 0 || status.src_rate == 0 || status.dev_rate == 0 {
        return None;
    }
    Some(StreamInfo::from(status))
}

/// Exactly what `App::seal` derives, for a client with no EQ and no ReplayGain.
fn seal(info: Option<StreamInfo>, playing: bool, volume: f64) -> SignalPath {
    signal_path::derive(&SealInput {
        engine_active: playing,
        info,
        eq: EqState::default(),
        volume,
        replaygain_db: signal_path::applied_replaygain_db(None),
        replaygain_mode: RgMode::Off,
    })
}

fn describe(status: &EngineStatus) -> String {
    format!(
        "playing={} codec={} src_rate={} rate={} dev_rate={} bits={} device={:?} dur_ms={}",
        status.playing,
        status.codec,
        status.src_rate,
        status.rate,
        status.dev_rate,
        status.bits,
        status.device,
        status.dur_ms
    )
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

/// Start a stream and wait until the engine has described it whole.
///
/// Returns the engine (so the caller can keep polling it) and the gated
/// `StreamInfo`, or `None` if the engine never got that far.
fn stream_until_described(url: &str) -> (Engine, Option<StreamInfo>) {
    let engine = Engine::default();
    engine.play_url(url.to_string());
    // Silent. The measurements below read the output callback's own counters.
    engine.set_volume(0.0);
    wait_for(Duration::from_secs(15), || {
        engine
            .status()
            .is_some_and(|s| gated_stream_info(&s).is_some())
    });
    let info = engine.status().as_ref().and_then(gated_stream_info);
    (engine, info)
}

/// **1 and 2: a streamed track plays, and the seal reads what the rates say.**
#[test]
#[ignore = "claims the default audio output device; run it deliberately"]
fn a_streamed_sine_plays_and_seals_honestly() {
    let (_server, url) = serve(44_100);
    println!("serving 8s of 44.1 kHz 16-bit stereo PCM over HTTP");

    let (engine, info) = stream_until_described(&url);
    let status = engine.status().expect("a session");
    println!("1. {}", describe(&status));
    let info = info.expect("the engine never described the stream");

    // ── 1. it is playing, and the playhead advances ──────────────────────
    assert!(status.playing, "status().playing was false for a stream");
    let before = engine.status().unwrap().pos_ms;
    std::thread::sleep(Duration::from_millis(1_200));
    let after = engine.status().unwrap().pos_ms;
    println!(
        "   pos_ms {before} -> {after} over ~1200ms (delta {})",
        after - before
    );
    assert!(
        after > before,
        "pos_ms did not advance: {before} -> {after} — nothing is being consumed"
    );

    // ── 2. the seal at unity, against an attenuated control ──────────────
    let unity = seal(Some(info.clone()), true, 1.0);
    println!(
        "2. UNITY  active={} pure={} seal_label={:?} engine_label={:?}\n   flags={:?}\n   src={:?} output={:?}",
        unity.active, unity.pure, unity.seal_label, unity.engine_label, unity.flags, unity.src, unity.output
    );
    assert!(unity.active, "a fully described stream derived no seal");
    assert_eq!(
        unity.pure,
        info.src_rate == info.rate && info.dev_rate == info.rate,
        "`pure` and the rates disagree: {info:?}"
    );
    assert_eq!(unity.pure, unity.seal_label == "BIT-PERFECT");

    // Not a state this client can reach — see the module docs. It is the
    // control that proves `pure` above is a finding rather than a constant.
    let quiet = seal(Some(info.clone()), true, 0.95);
    println!(
        "   CONTROL (unreachable from eko-cli) volume 0.95  pure={} seal_label={:?} attenuated={}",
        quiet.pure, quiet.seal_label, quiet.flags.attenuated
    );
    assert!(!quiet.pure, "attenuation did not break a streamed seal");
    assert!(quiet.flags.attenuated);

    engine.stop();
}

/// **3: a stream the device cannot run at its own rate must read `RESAMPLED`.**
///
/// On macOS EKO *sets* the device's nominal rate to the file's, so the seal is
/// only honest to test against a rate the hardware will refuse. The candidates
/// below descend past what any DAC offers; the first that produces a genuine rate
/// difference is the case, and the test says which one it used.
///
/// If none of them produced a difference the test **fails** rather than passing
/// vacuously — an untested resample claim is the one thing this file exists to
/// prevent.
#[test]
#[ignore = "claims the default audio output device; run it deliberately"]
fn a_stream_the_device_resamples_never_reads_bit_perfect() {
    let mut constructed = None;

    for rate in [22_050u32, 11_025, 8_000] {
        let (_server, url) = serve(rate);
        let (engine, info) = stream_until_described(&url);
        let status = engine.status().expect("a session");
        println!("{rate} Hz source: {}", describe(&status));
        let Some(info) = info else {
            engine.stop();
            continue;
        };
        let differs = info.src_rate != info.rate || info.dev_rate != info.rate;
        let s = seal(Some(info.clone()), true, 1.0);
        println!(
            "   active={} pure={} seal_label={:?} resampled={} os_resampled={}\n   src={:?} output={:?}",
            s.active, s.pure, s.seal_label, s.flags.resampled, s.flags.os_resampled, s.src, s.output
        );

        // Whatever the rates turned out to be, the claim has to match them.
        assert_eq!(
            s.pure, !differs,
            "{rate} Hz: `pure` and the rates disagree: {info:?}"
        );
        if differs {
            assert!(
                !s.pure,
                "{rate} Hz: a resampled stream claimed to be bit-perfect"
            );
            assert_eq!(s.seal_label, "RESAMPLED", "{rate} Hz");
            assert!(s.flags.resampled || s.flags.os_resampled);
            constructed = Some((rate, info, s));
            engine.stop();
            break;
        }
        engine.stop();
    }

    let (rate, info, s) = constructed.expect(
        "no candidate source rate produced a device-rate mismatch on this machine — \
         the resample case was NOT tested; find a rate this output device refuses, \
         or run against one that does, before believing the seal here",
    );
    println!(
        "\nCONSTRUCTED at {rate} Hz: src_rate={} rate={} dev_rate={} -> {:?}",
        info.src_rate, info.rate, info.dev_rate, s.seal_label
    );
}

/// **A stream that 404s claims nothing.**
///
/// `Engine::play_url` returns a stub status before a single byte is requested and
/// reports nothing at all when the request fails, so the only signal is what it
/// does *not* write: the rates are never stored. That is what the `stream_info`
/// gate reads, and it is why the Deck shows a hollow lamp rather than a green
/// one.
///
/// Note the second assertion. `derive` returns **`pure: true`** for a status it
/// has no `StreamInfo` for — every flag is false because there is no stream to
/// raise one — so `pure` on its own is not a guard. `active` is. A front end that
/// branched on `pure` would paint the green lamp over a 404, which is the desktop
/// app's bug on this exact path.
///
/// No output device is claimed: the open fails long before one is asked for. This
/// one therefore runs in a normal `cargo test`.
#[test]
fn a_stream_that_answers_404_never_reaches_the_seal() {
    let mut server = mockito::Server::new();
    let url = format!("{}/rest/stream?id=nope", server.url());
    let _m = server
        .mock("GET", "/rest/stream")
        .match_query(mockito::Matcher::Any)
        .with_status(404)
        .with_body("Not Found")
        .create();

    let engine = Engine::default();
    engine.play_url(url);
    assert!(
        wait_for(Duration::from_secs(15), || engine
            .status()
            .is_some_and(|s| !s.playing)),
        "the engine never gave up on a 404"
    );

    let status = engine.status().expect("a session");
    println!("404: {}", describe(&status));
    assert!(!status.playing);
    assert_eq!(status.pos_ms, 0, "a failed open moved the playhead");
    assert!(
        gated_stream_info(&status).is_none(),
        "a failed open described a stream: {}",
        describe(&status)
    );

    // What the Deck would derive: `playback` is back to `Stopped`, so the engine
    // is not active and there is no stream either.
    let s = seal(gated_stream_info(&status), false, 1.0);
    assert!(!s.active, "a 404 derived an active seal");
    assert!(
        s.pure,
        "the premise of the footer's branch order changed — re-read it"
    );

    // And the trap, stated: handed the raw status the gate refused, `derive`
    // seals it.
    let ungated = seal(Some(StreamInfo::from(&status)), true, 1.0);
    println!(
        "   gated: active={} · ungated: active={} pure={} label={:?}",
        s.active, ungated.active, ungated.pure, ungated.seal_label
    );
    assert!(
        ungated.active && ungated.pure,
        "the gate is no longer guarding anything — if this changed, `stream_info` \
         may no longer be load-bearing"
    );

    engine.stop();
}

/// A reverse proxy answering with an HTML login page is a 200 full of bytes that
/// are not audio. It has to fail the same way a 404 does.
#[test]
fn a_proxy_that_answers_with_html_never_reaches_the_seal() {
    let mut server = mockito::Server::new();
    let url = format!("{}/rest/stream?id=tr-1", server.url());
    let _m = server
        .mock("GET", "/rest/stream")
        .match_query(mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<!doctype html><html><body>Sign in to continue</body></html>")
        .create();

    let engine = Engine::default();
    engine.play_url(url);
    assert!(
        wait_for(Duration::from_secs(15), || engine
            .status()
            .is_some_and(|s| !s.playing)),
        "the engine never gave up on an HTML body"
    );
    let status = engine.status().expect("a session");
    println!("html: {}", describe(&status));
    assert!(gated_stream_info(&status).is_none());
    assert!(!seal(gated_stream_info(&status), false, 1.0).active);

    engine.stop();
}
