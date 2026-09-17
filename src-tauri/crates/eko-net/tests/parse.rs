//! Integration tests for `eko-net`'s public auth + envelope surface, exercised the
//! way a downstream caller (eko-tauri, or a future terminal client) would use them.

use std::collections::BTreeSet;

use eko_net::types::{ReplayGain, SubSong};
use eko_net::{auth, urls, Config};

fn cfg() -> Config {
    Config {
        base_url: "https://music.example.com/".into(),
        username: "rod".into(),
        password: "hunter2".into(),
    }
}

#[test]
fn token_is_md5_of_password_plus_salt() {
    // Reference value: md5("hunter2" + "abcdefghij"), computed independently via
    // `printf '%s' 'hunter2abcdefghij' | md5sum` — not derived from the
    // implementation under test.
    let p = auth::auth_params(&cfg(), "abcdefghij");
    let get = |k: &str| {
        p.iter()
            .find(|(a, _)| a == k)
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    assert_eq!(get("u"), "rod");
    assert_eq!(get("s"), "abcdefghij");
    assert_eq!(get("v"), "1.16.1");
    assert_eq!(get("c"), "eko");
    assert_eq!(get("f"), "json");
    assert_eq!(get("t"), "7a57aa734040ee9e0904f3329aa0c3e1");
    assert!(
        !p.iter().any(|(k, _)| k == "p"),
        "password must never be sent in the clear"
    );
}

#[test]
fn envelope_extracts_payload_and_maps_errors() {
    let ok =
        r#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{"album":[]}}}"#;
    assert!(eko_net::parse::envelope(ok)
        .unwrap()
        .get("albumList2")
        .is_some());

    let err = r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong username or password"}}}"#;
    let e = eko_net::parse::envelope(err).unwrap_err();
    assert!(e.to_string().contains("Wrong username or password"));

    let junk = r#"{"nope":true}"#;
    assert!(
        eko_net::parse::envelope(junk).is_err(),
        "missing envelope must error"
    );
}

/// Guards against silently renaming, dropping, or changing the optionality of any
/// field the existing React frontend already consumes as JSON. A fully-populated
/// `SubSong` must produce exactly this key set — no more, no less.
#[test]
fn fully_populated_sub_song_round_trips_the_exact_key_set() {
    let song = SubSong {
        id: "song-1".into(),
        title: "Title".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        duration: Some(245),
        bit_rate: Some(320),
        sampling_rate: Some(44100),
        channel_count: Some(2),
        suffix: Some("flac".into()),
        content_type: Some("audio/flac".into()),
        track: Some(3),
        cover_art: Some("al-1".into()),
        replay_gain: Some(ReplayGain {
            track_gain: Some(-6.5),
            album_gain: Some(-7.2),
            track_peak: Some(0.98),
            album_peak: Some(0.99),
        }),
        stream_url: Some("stream://localhost/?src=...".into()),
        stream_src_url: Some("https://music.example.com/rest/stream?...".into()),
        download_url: Some("https://music.example.com/rest/download?...".into()),
        cover_url: Some("stream://localhost/?src=...".into()),
        server: Some("https://music.example.com".into()),
    };

    let value = serde_json::to_value(&song).unwrap();
    let obj = value.as_object().unwrap();
    let keys: BTreeSet<&str> = obj.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = [
        "id",
        "title",
        "artist",
        "album",
        "duration",
        "bitRate",
        "samplingRate",
        "channelCount",
        "suffix",
        "contentType",
        "track",
        "coverArt",
        "replayGain",
        "streamUrl",
        "streamSrcUrl",
        "downloadUrl",
        "coverUrl",
        "server",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, expected);

    let rg = obj["replayGain"].as_object().unwrap();
    let rg_keys: BTreeSet<&str> = rg.keys().map(String::as_str).collect();
    let rg_expected: BTreeSet<&str> = ["trackGain", "albumGain", "trackPeak", "albumPeak"]
        .into_iter()
        .collect();
    assert_eq!(rg_keys, rg_expected);
}

/// The mirror image: absent optionals must disappear from the JSON entirely
/// rather than becoming explicit `null`s, matching how the TypeScript client's
/// optional (`foo?:`) fields behave.
#[test]
fn minimal_sub_song_omits_every_absent_optional() {
    let song = SubSong {
        id: "song-1".into(),
        title: "Title".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        duration: None,
        bit_rate: None,
        sampling_rate: None,
        channel_count: None,
        suffix: None,
        content_type: None,
        track: None,
        cover_art: None,
        replay_gain: None,
        stream_url: None,
        stream_src_url: None,
        download_url: None,
        cover_url: None,
        server: None,
    };

    let value = serde_json::to_value(&song).unwrap();
    let obj = value.as_object().unwrap();
    let keys: BTreeSet<&str> = obj.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = ["id", "title", "artist", "album"].into_iter().collect();
    assert_eq!(keys, expected);
}

#[test]
fn stream_url_is_wrapped_and_cover_url_is_wrapped_but_src_and_download_are_direct() {
    let c = cfg();
    let stream = urls::stream_url(&c, "song-1", "saltsaltxx");
    assert!(
        stream.starts_with("stream://localhost/?src="),
        "webview URL must be proxied"
    );
    assert!(
        stream.contains("format%3Draw"),
        "format=raw must survive urlencoding"
    );

    let src = urls::stream_src_url(&c, "song-1", "saltsaltxx");
    assert!(
        src.starts_with("https://music.example.com/rest/stream?"),
        "engine URL must be direct"
    );
    assert!(!src.contains("stream://"));

    let dl = urls::download_url(&c, "song-1", "saltsaltxx");
    assert!(
        dl.starts_with("https://music.example.com/rest/download?"),
        "must be direct"
    );

    let cover = urls::cover_art_url(&c, Some("al-1"), None, "saltsaltxx").unwrap();
    assert!(cover.starts_with("stream://localhost/?src="));
    assert!(
        urls::cover_art_url(&c, None, None, "saltsaltxx").is_none(),
        "no coverArt -> None"
    );

    // Trailing slashes on base_url must be stripped exactly once, as client.ts:70 does.
    assert!(!src.contains("com//rest"));
}
