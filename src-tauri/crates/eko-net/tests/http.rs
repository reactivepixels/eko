//! HTTP-layer tests for [`eko_net::Client`] against a mock Subsonic server.
//!
//! The parsing surface is covered offline in `tests/fixtures.rs`; what can only be
//! checked with a socket in the way is what this file asserts:
//!
//! * the exact path and query params each method puts on the wire — every default that
//!   the TypeScript expressed as a default parameter is a silent behaviour change if it
//!   drifts, and nothing else in the suite would notice;
//! * that a non-2xx status becomes [`eko_net::SubsonicError::Http`];
//! * that `scrobble` swallows *every* failure, which is only observable end-to-end;
//! * that the lyrics fallback chain actually issues its second request.

use eko_net::types::AlbumListType;
use eko_net::{Client, Config, SubsonicError};
use mockito::{Matcher, Server, ServerGuard};

const OK_EMPTY: &str = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;

/// The six auth params in the exact order [`eko_net::auth::auth_params`] emits them.
/// `t` and `s` are matched by shape because they derive from a fresh random salt per
/// request; `tests/parse.rs` pins the digest itself.
///
/// Used to build **anchored** whole-query regexes, which is how the tests below prove a
/// param is *absent* — the `regex` crate has no lookahead, so "no `genre=`" has to be
/// expressed as "the query is exactly this and nothing more".
const AUTH_QUERY: &str = r"u=rod&t=[0-9a-f]{32}&s=[0-9a-z]{10}&v=1\.16\.1&c=eko&f=json";

/// Matcher for a query that is the six auth params followed by exactly `tail`
/// (`""` for none). Anchored at both ends, so any extra param fails the match.
fn exact_query(tail: &str) -> Matcher {
    Matcher::Regex(format!("^{AUTH_QUERY}{tail}$"))
}

fn client(server: &ServerGuard) -> Client {
    Client::new(Config {
        base_url: server.url(),
        username: "rod".into(),
        password: "hunter2".into(),
    })
    .expect("http client builds")
}

/// The six auth params every request must carry. `t`/`s` are checked for shape rather
/// than value — they're derived from a random per-request salt, and `tests/parse.rs`
/// already pins the digest itself.
///
/// These prove *presence* only. `Matcher::AllOf` over `UrlEncoded` can never prove a
/// param is **absent**, so this says nothing about the password not being sent — that is
/// established by `tests/parse.rs`'s `token_is_md5_of_password_plus_salt` (which asserts
/// no `p` key) and, on the wire, by the anchored whole-query [`exact_query`] tests, where
/// any extra param fails the match.
fn auth_matchers() -> Vec<Matcher> {
    vec![
        Matcher::UrlEncoded("u".into(), "rod".into()),
        Matcher::UrlEncoded("v".into(), "1.16.1".into()),
        Matcher::UrlEncoded("c".into(), "eko".into()),
        Matcher::UrlEncoded("f".into(), "json".into()),
        Matcher::Regex("t=[0-9a-f]{32}".into()),
        Matcher::Regex("s=[0-9a-z]{10}".into()),
    ]
}

fn query(extra: Vec<Matcher>) -> Matcher {
    let mut all = auth_matchers();
    all.extend(extra);
    Matcher::AllOf(all)
}

#[test]
fn ping_hits_rest_ping_with_the_six_auth_params_and_no_password() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/ping")
        .match_query(exact_query(""))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(include_str!("fixtures/ping.json"))
        .create();

    client(&server).ping().expect("ping succeeds");
    mock.assert();
}

#[test]
fn get_albums_sends_the_typescript_defaults() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/getAlbumList2")
        .match_query(query(vec![
            // These three ARE the behaviour: client.ts:97-102.
            Matcher::UrlEncoded("type".into(), "alphabeticalByArtist".into()),
            Matcher::UrlEncoded("size".into(), "500".into()),
            Matcher::UrlEncoded("offset".into(), "0".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/albumlist2.json"))
        .create();

    let albums = client(&server).get_albums(None, None).unwrap();
    mock.assert();
    assert_eq!(albums.len(), 3);
    assert!(
        albums[0].cover_url.is_some(),
        "the endpoint layer must mint cover URLs; the frontend never sees the password"
    );
}

#[test]
fn get_album_list2_sends_an_explicit_type_and_appends_extra_params() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/getAlbumList2")
        .match_query(query(vec![
            Matcher::UrlEncoded("type".into(), "byGenre".into()),
            Matcher::UrlEncoded("size".into(), "25".into()),
            Matcher::UrlEncoded("offset".into(), "50".into()),
            Matcher::UrlEncoded("genre".into(), "Drum & Bass".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/albumlist2_capped.json"))
        .create();

    let albums = client(&server)
        .get_album_list2(
            AlbumListType::ByGenre,
            Some(25),
            Some(50),
            &[("genre", "Drum & Bass")],
        )
        .unwrap();
    mock.assert();
    assert_eq!(
        albums.len(),
        2,
        "a page shorter than the requested size is returned as-is; it is not a signal \
         that the walk is finished"
    );
}

#[test]
fn get_album_populates_all_four_song_urls_and_the_album_cover() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/getAlbum")
        .match_query(query(vec![Matcher::UrlEncoded(
            "id".into(),
            "al-0002".into(),
        )]))
        .with_status(200)
        .with_body(include_str!("fixtures/album.json"))
        .create();

    let detail = client(&server).get_album("al-0002").unwrap();
    mock.assert();

    assert!(detail.album.cover_url.is_some());
    for song in &detail.songs {
        let download = song.download_url.as_deref().expect("downloadUrl");
        assert!(
            download.contains("/rest/download"),
            "the offline cache reads downloadUrl and must get original bytes, not the \
             (possibly transcoded) stream endpoint"
        );
        assert!(song
            .stream_src_url
            .as_deref()
            .unwrap()
            .contains("/rest/stream"));
        assert!(song
            .stream_url
            .as_deref()
            .unwrap()
            .starts_with("stream://localhost/?src="));
        assert!(song.cover_url.is_some());
    }
}

#[test]
fn search_sends_the_fixed_result_counts() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/search3")
        .match_query(query(vec![
            Matcher::UrlEncoded("query".into(), "kid a".into()),
            // artistCount is deliberately 0 — no artist section in the UI. songCount was
            // the TypeScript's 50 until it proved too low a ceiling for finding one song in
            // a large library; it is pinned here because a silent drift is a behaviour change.
            Matcher::UrlEncoded("songCount".into(), "200".into()),
            Matcher::UrlEncoded("albumCount".into(), "30".into()),
            Matcher::UrlEncoded("artistCount".into(), "0".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/search3.json"))
        .create();

    let result = client(&server).search("kid a").unwrap();
    mock.assert();
    assert_eq!(result.albums.len(), 1);
    assert!(result.songs[0].stream_url.is_some());
}

#[test]
fn get_songs_asks_for_every_song_with_an_empty_query() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/search3")
        .match_query(query(vec![
            // The empty `query` IS the mechanism: Navidrome answers it with the whole song
            // set rather than a match, which is what gives a server a full track index at
            // all (verified against Navidrome 0.63.2). A server that instead treats it as a
            // literal search returns nothing, and the Tracks section falls back to its
            // explanatory copy — no feature probe needed either way.
            Matcher::UrlEncoded("query".into(), "".into()),
            Matcher::UrlEncoded("songCount".into(), "500".into()),
            Matcher::UrlEncoded("songOffset".into(), "0".into()),
            // Albums and artists are dead weight here — this call feeds ONLY the track list.
            Matcher::UrlEncoded("albumCount".into(), "0".into()),
            Matcher::UrlEncoded("artistCount".into(), "0".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/search3.json"))
        .create();

    let songs = client(&server).get_songs(None, None).unwrap();
    mock.assert();
    assert!(
        songs[0].stream_url.is_some(),
        "the endpoint layer must mint stream URLs; the frontend never sees the password"
    );
}

#[test]
fn get_songs_pages_with_an_explicit_size_and_offset() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/search3")
        .match_query(query(vec![
            Matcher::UrlEncoded("query".into(), "".into()),
            Matcher::UrlEncoded("songCount".into(), "250".into()),
            Matcher::UrlEncoded("songOffset".into(), "750".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/search3.json"))
        .create();

    client(&server).get_songs(Some(250), Some(750)).unwrap();
    mock.assert();
}

#[test]
fn get_random_songs_defaults_to_fifty_and_omits_an_absent_genre() {
    let mut server = Server::new();
    // Anchored: `size=50` and nothing after it, which is how "genre is omitted" is
    // asserted. Sending `genre=` would ask for songs whose genre is the empty string.
    let sized = server
        .mock("GET", "/rest/getRandomSongs")
        .match_query(exact_query("&size=50"))
        .with_status(200)
        .with_body(include_str!("fixtures/random_songs.json"))
        .create();

    let songs = client(&server).get_random_songs(None, None).unwrap();
    sized.assert();
    assert_eq!(songs.len(), 2);

    let genred = server
        .mock("GET", "/rest/getRandomSongs")
        .match_query(query(vec![
            Matcher::UrlEncoded("size".into(), "10".into()),
            Matcher::UrlEncoded("genre".into(), "Ambient".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/random_songs.json"))
        .create();

    client(&server)
        .get_random_songs(Some(10), Some("Ambient"))
        .unwrap();
    genred.assert();
}

#[test]
fn get_similar_songs2_defaults_to_a_count_of_fifty() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/getSimilarSongs2")
        .match_query(query(vec![
            Matcher::UrlEncoded("id".into(), "so-00021".into()),
            Matcher::UrlEncoded("count".into(), "50".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/similar_songs2.json"))
        .create();

    let songs = client(&server)
        .get_similar_songs2("so-00021", None)
        .unwrap();
    mock.assert();
    assert_eq!(songs.len(), 1);
    assert!(songs[0].download_url.is_some());
}

#[test]
fn playlist_genres_artist_info_and_starred_reach_their_endpoints() {
    let mut server = Server::new();

    let playlists = server
        .mock("GET", "/rest/getPlaylists")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(include_str!("fixtures/playlists.json"))
        .create();
    assert_eq!(client(&server).get_playlists().unwrap().len(), 2);
    playlists.assert();

    let playlist = server
        .mock("GET", "/rest/getPlaylist")
        .match_query(query(vec![Matcher::UrlEncoded(
            "id".into(),
            "pl-01".into(),
        )]))
        .with_status(200)
        .with_body(include_str!("fixtures/playlist.json"))
        .create();
    let detail = client(&server).get_playlist("pl-01").unwrap();
    playlist.assert();
    assert_eq!(detail.name, "Late Night");
    assert!(detail.songs.iter().all(|s| s.stream_url.is_some()));

    let genres = server
        .mock("GET", "/rest/getGenres")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(include_str!("fixtures/genres.json"))
        .create();
    assert_eq!(client(&server).get_genres().unwrap().len(), 4);
    genres.assert();

    let info = server
        .mock("GET", "/rest/getArtistInfo2")
        .match_query(query(vec![Matcher::UrlEncoded(
            "id".into(),
            "ar-0001".into(),
        )]))
        .with_status(200)
        .with_body(include_str!("fixtures/artist_info2.json"))
        .create();
    assert!(client(&server)
        .get_artist_info2("ar-0001")
        .unwrap()
        .biography
        .is_some());
    info.assert();

    let starred = server
        .mock("GET", "/rest/getStarred2")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(include_str!("fixtures/starred2.json"))
        .create();
    let starred_result = client(&server).get_starred2().unwrap();
    starred.assert();
    assert!(starred_result.song.unwrap()[0].stream_url.is_some());
    assert!(starred_result.album.unwrap()[0].cover_url.is_some());
}

#[test]
fn a_non_2xx_status_becomes_an_http_error() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/getAlbumList2")
        .match_query(Matcher::Any)
        .with_status(500)
        .with_body("upstream exploded")
        .create();

    let err = client(&server).get_albums(None, None).unwrap_err();
    mock.assert();
    assert_eq!(err, SubsonicError::Http(500));
    assert_eq!(err.to_string(), "HTTP 500");
}

#[test]
fn a_failed_envelope_with_http_200_is_still_an_error() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/ping")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(include_str!("fixtures/error_wrong_password.json"))
        .create();

    let err = client(&server).ping().unwrap_err();
    mock.assert();
    assert_eq!(
        err,
        SubsonicError::Subsonic("Wrong username or password".into())
    );
}

#[test]
fn an_unreachable_server_is_a_request_error() {
    // Port 1 on loopback: nothing listens, so the connection is refused outright.
    let client = Client::new(Config {
        base_url: "http://127.0.0.1:1".into(),
        username: "rod".into(),
        password: "hunter2".into(),
    })
    .unwrap();
    assert!(matches!(
        client.ping().unwrap_err(),
        SubsonicError::Request(_)
    ));
}

/// True if `s` contains a run of 32 lowercase hex chars — the shape of the md5 auth
/// token. Written by hand rather than pulling in a regex dep; it has one job.
fn contains_md5_shaped_run(s: &str) -> bool {
    let mut run = 0usize;
    for &b in s.as_bytes() {
        if b.is_ascii_digit() || (b'a'..=b'f').contains(&b) {
            run += 1;
            if run >= 32 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// A transport error's message must not carry the signed URL.
///
/// `reqwest`'s `Display` appends `" for url ({url})"`, and every URL this crate builds is
/// signed: it carries `u`, `t` (md5 of password + salt) and `s` (the salt). Under
/// Subsonic's auth scheme that triple is a **replayable credential** — it authenticates
/// as the user until the password changes. `SubsonicError::Request`'s message crosses the
/// IPC boundary and is rendered verbatim by `ConnectPanel.tsx`, so leaking it there puts
/// a live credential in the DOM, where a screenshot or a pasted support ticket exports it.
///
/// The frontend's `friendlyConnectError` regex is not a backstop: it catches
/// `"sending request"` but not `"error following redirect for url (…)"`, `"request or
/// response body error for url (…)"` or `"error decoding response body for url (…)"`, all
/// of which fall through to `return raw`. The strip has to happen here, in
/// `client::safe_message`.
#[test]
fn a_transport_error_message_never_carries_the_signed_url_or_auth_token() {
    let client = Client::new(Config {
        base_url: "http://127.0.0.1:1".into(),
        username: "rod".into(),
        password: "hunter2".into(),
    })
    .unwrap();

    let SubsonicError::Request(message) = client.ping().unwrap_err() else {
        panic!("expected a transport error from the dead port");
    };

    for needle in [
        "for url",
        "t=",
        "s=",
        "u=rod",
        "rest/ping",
        "127.0.0.1",
        "hunter2",
    ] {
        assert!(
            !message.contains(needle),
            "transport error leaked {needle:?} — the signed URL carries a replayable auth \
             token and this message is rendered in the UI. Message: {message:?}"
        );
    }
    assert!(
        !contains_md5_shaped_run(&message),
        "transport error contains an md5-shaped run — most likely the auth token. \
         Message: {message:?}"
    );

    // Not vacuous: something diagnostic has to survive the strip, or this test would pass
    // just as happily against an empty string.
    assert!(
        !message.is_empty(),
        "the message was stripped to nothing — `without_url` removes the URL, not the error"
    );
}

// ------------------------------------------------------- scrobble swallows everything

#[test]
fn scrobble_sends_submission_and_a_millisecond_timestamp() {
    let mut server = Server::new();
    let now_playing = server
        .mock("GET", "/rest/scrobble")
        .match_query(query(vec![
            Matcher::UrlEncoded("id".into(), "so-00021".into()),
            Matcher::UrlEncoded("submission".into(), "false".into()),
            // 13 digits: milliseconds. A 10-digit (seconds) timestamp would make every
            // scrobble land in 1970 and be silently discarded by the server.
            Matcher::Regex("time=[0-9]{13}".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/scrobble_ok.json"))
        .create();

    client(&server).scrobble("so-00021", false).unwrap();
    now_playing.assert();

    let submission = server
        .mock("GET", "/rest/scrobble")
        .match_query(query(vec![
            Matcher::UrlEncoded("id".into(), "so-00021".into()),
            Matcher::UrlEncoded("submission".into(), "true".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/scrobble_ok.json"))
        .create();

    client(&server).scrobble("so-00021", true).unwrap();
    submission.assert();
}

/// The contract from `client.ts:259-269`: "network/server errors must not affect
/// playback." Every one of these would be an `Err` from any other method.
#[test]
fn scrobble_swallows_a_server_error() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/scrobble")
        .match_query(Matcher::Any)
        .with_status(500)
        .with_body("upstream exploded")
        .create();

    assert_eq!(
        client(&server).scrobble("so-00021", true),
        Ok(()),
        "a 500 from the scrobble endpoint must never surface — it would interrupt playback"
    );
    mock.assert();
}

#[test]
fn scrobble_swallows_a_failed_envelope() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/scrobble")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(include_str!("fixtures/error_wrong_password.json"))
        .create();

    assert_eq!(client(&server).scrobble("so-00021", true), Ok(()));
    mock.assert();
}

#[test]
fn scrobble_swallows_an_unreachable_server() {
    let client = Client::new(Config {
        base_url: "http://127.0.0.1:1".into(),
        username: "rod".into(),
        password: "hunter2".into(),
    })
    .unwrap();
    assert_eq!(
        client.scrobble("so-00021", true),
        Ok(()),
        "an unplugged network cable must not stop the music"
    );
}

// ------------------------------------------------------- lyrics fallback chain

#[test]
fn lyrics_by_song_id_sends_id_not_song_id() {
    let mut server = Server::new();
    // Despite the method name, the param is `id` — client.ts:284 and the OpenSubsonic
    // spec. Sending `songId` would make every server ignore the request.
    let mock = server
        .mock("GET", "/rest/getLyricsBySongId")
        .match_query(query(vec![Matcher::UrlEncoded(
            "id".into(),
            "so-00021".into(),
        )]))
        .with_status(200)
        .with_body(include_str!("fixtures/lyrics_synced.json"))
        .create();

    let result = client(&server).get_lyrics_by_song_id("so-00021").unwrap();
    mock.assert();
    assert_eq!(result.synced.unwrap().len(), 3);
}

#[test]
fn lyrics_by_song_id_falls_back_to_the_legacy_endpoint_on_failure() {
    let mut server = Server::new();
    // A pre-OpenSubsonic server: the endpoint doesn't exist, so it answers "failed".
    let modern = server
        .mock("GET", "/rest/getLyricsBySongId")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(
            r#"{"subsonic-response":{"status":"failed","error":{"code":0,"message":"getLyricsBySongId not implemented"}}}"#,
        )
        .create();
    // The fallback goes out with NO artist and NO title (client.ts:304), which is why it
    // almost always comes back empty — its job is to yield a uniform "no lyrics", not an
    // error the caller has to special-case.
    let legacy = server
        .mock("GET", "/rest/getLyrics")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(OK_EMPTY)
        .create();

    let result = client(&server).get_lyrics_by_song_id("so-00021").unwrap();
    modern.assert();
    legacy.assert();
    assert_eq!(result.synced, None);
    assert_eq!(result.unsynced, None);
}

#[test]
fn legacy_lyrics_send_artist_and_title_and_omit_them_when_absent() {
    let mut server = Server::new();
    let both = server
        .mock("GET", "/rest/getLyrics")
        .match_query(query(vec![
            Matcher::UrlEncoded("artist".into(), "Massive Attack".into()),
            Matcher::UrlEncoded("title".into(), "Teardrop".into()),
        ]))
        .with_status(200)
        .with_body(include_str!("fixtures/lyrics_legacy.json"))
        .create();

    let result = client(&server)
        .get_lyrics_legacy(Some("Massive Attack"), Some("Teardrop"))
        .unwrap();
    both.assert();
    assert!(result.unsynced.unwrap().starts_with("Love, love is a verb"));

    // Empty strings are falsy in `if (artist)` and must not be sent at all — asserted by
    // requiring the query to be exactly the six auth params with no tail.
    let neither = server
        .mock("GET", "/rest/getLyrics")
        .match_query(exact_query(""))
        .with_status(200)
        .with_body(OK_EMPTY)
        .create();
    let result = client(&server).get_lyrics_legacy(Some(""), None).unwrap();
    neither.assert();
    assert_eq!(result.unsynced, None);
}

#[test]
fn legacy_lyrics_swallow_errors_into_an_empty_result() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/rest/getLyrics")
        .match_query(Matcher::Any)
        .with_status(500)
        .create();

    let result = client(&server)
        .get_lyrics_legacy(Some("A"), Some("T"))
        .unwrap();
    mock.assert();
    assert_eq!(result.synced, None);
    assert_eq!(
        result.unsynced, None,
        "client.ts:322-324 swallows the error — a server with no lyrics support must not \
         surface as a failure in the lyrics panel"
    );
}
