//! Fixture-driven tests for the pure parsing layer.
//!
//! Every fixture under `tests/fixtures/` is a real-shaped Navidrome response, so these
//! tests cover the full parsing surface with no server, no runtime and no clock. That
//! split is the point: [`eko_net::Client`] adds only "build URL, GET, check status", and
//! `tests/http.rs` covers that separately.
//!
//! Several fixtures exist specifically to pin down behaviour that is easy to "improve"
//! into a bug — a missing collection key, a short page, an absent `replayGain`, an
//! HTTP-200 auth failure. Each of those assertions carries the reason in its message.

use eko_net::parse;
use eko_net::types::{mime_for_song, SubSong};
use eko_net::urls;
use eko_net::{Config, SubsonicError};

fn cfg() -> Config {
    Config {
        base_url: "https://music.example.com/".into(),
        username: "rod".into(),
        password: "hunter2".into(),
    }
}

/// Unwrap a fixture's envelope, failing loudly if the fixture itself is malformed.
fn payload(body: &str) -> serde_json::Value {
    parse::envelope(body).expect("fixture must contain an ok subsonic-response envelope")
}

// ---------------------------------------------------------------- ping

#[test]
fn ping_envelope_unwraps() {
    let sr = payload(include_str!("fixtures/ping.json"));
    assert_eq!(sr["status"], "ok");
    assert_eq!(sr["type"], "navidrome");
}

// ---------------------------------------------------------------- getAlbumList2

#[test]
fn album_list_parses_every_album_and_its_optionals() {
    let albums = parse::album_list(&payload(include_str!("fixtures/albumlist2.json"))).unwrap();
    assert_eq!(albums.len(), 3);

    assert_eq!(albums[0].id, "al-0001");
    assert_eq!(albums[0].name, "Selected Ambient Works 85-92");
    assert_eq!(albums[0].artist, "Aphex Twin");
    assert_eq!(albums[0].artist_id.as_deref(), Some("ar-0001"));
    assert_eq!(albums[0].year, Some(1992));
    assert_eq!(albums[0].song_count, Some(13));
    assert_eq!(albums[0].cover_art.as_deref(), Some("al-0001"));

    // Unknown server fields (bpm, sortName, musicBrainzId, explicitStatus, …) must be
    // ignored rather than rejected — Navidrome adds fields between releases.
    assert_eq!(albums[1].name, "Kid A");

    // Sparsely-tagged album: no year, no artistId, no coverArt. All absent, not an error.
    assert_eq!(albums[2].id, "al-0003");
    assert_eq!(albums[2].year, None);
    assert_eq!(albums[2].artist_id, None);
    assert_eq!(albums[2].cover_art, None);

    // Nothing populates the URL fields at the parse layer.
    assert!(albums.iter().all(|a| a.cover_url.is_none()));
}

#[test]
fn album_list_with_no_album_key_parses_as_empty() {
    let albums =
        parse::album_list(&payload(include_str!("fixtures/albumlist2_empty.json"))).unwrap();
    assert!(
        albums.is_empty(),
        "a missing `album` key is an empty library, not an error — client.ts:104 leans on `?? []`"
    );
}

#[test]
fn album_list_with_a_null_album_key_parses_as_empty() {
    let body = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":null}}}"#;
    assert!(
        parse::album_list(&payload(body)).unwrap().is_empty(),
        "explicit null must behave like absent, matching `?? []`"
    );
}

#[test]
fn a_page_shorter_than_requested_is_reported_verbatim_and_implies_nothing() {
    let albums =
        parse::album_list(&payload(include_str!("fixtures/albumlist2_capped.json"))).unwrap();
    // The request asked for size=500; the server sent 2. That is a *cap*, not the end of
    // the library. useSubsonic.ts:185 documents the truncation bug caused by terminating
    // a page-walk on `page.length < PAGE_SIZE`. The parser's contract is therefore to
    // report exactly what arrived and expose no "is this the last page?" signal at all —
    // only an empty page may end a walk.
    assert_eq!(albums.len(), 2);
    assert_eq!(albums[0].name, "Zaireeka");
    assert_eq!(albums[1].name, "Zen Arcade");
}

#[test]
fn a_missing_album_list_container_is_empty_not_an_error() {
    let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    assert!(parse::album_list(&payload(body)).unwrap().is_empty());
}

/// Sparse tagging must degrade to blank text, never fail a page.
///
/// `useSubsonic.ts:76-78` coalesces `title`/`artist`/`album` to `null` and every render
/// site reads them through `?? ""` or a truthiness guard — nothing in the codebase does
/// a `=== null` comparison on them. `""` is therefore observationally identical to the
/// TypeScript's `null` at every call site.
#[test]
fn albums_missing_display_fields_still_arrive_with_their_siblings() {
    let albums = parse::album_list(&payload(include_str!("fixtures/album_no_name.json"))).unwrap();
    assert_eq!(
        albums.len(),
        3,
        "serde rejects the whole Vec if one element fails, so one untagged folder must \
         not blank out the entire library grid"
    );
    assert_eq!(albums[0].name, "Properly Tagged Album");
    assert_eq!(albums[1].name, "", "a missing `name` degrades to empty");
    assert_eq!(albums[1].artist, "Unknown Artist");
    assert_eq!(albums[2].artist, "", "a missing `artist` degrades to empty");
    assert_eq!(albums[2].name, "Orphaned Folder");
    // The load-bearing field is untouched by any of this.
    assert_eq!(albums[1].id, "al-0081");
}

#[test]
fn songs_missing_display_fields_still_arrive_with_their_siblings() {
    let detail = parse::album(&payload(include_str!("fixtures/song_no_artist.json"))).unwrap();
    assert_eq!(
        detail.songs.len(),
        3,
        "one untagged track must not fail the entire album page"
    );
    assert_eq!(detail.songs[0].artist, "The Tagged");
    assert_eq!(
        detail.songs[1].artist, "",
        "a missing `artist` degrades to empty"
    );
    assert_eq!(detail.songs[1].title, "Nobody Knows Who Made This");
    assert_eq!(
        detail.songs[2].title, "",
        "a missing `title` degrades to empty"
    );
    assert_eq!(detail.songs[2].artist, "The Tagged");
    assert!(detail.songs.iter().all(|s| !s.id.is_empty()));
}

#[test]
fn a_playlist_listing_without_a_name_degrades_to_empty() {
    let body = r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"pl-1"},{"id":"pl-2","name":"Kept"}]}}}"#;
    let lists = parse::playlists(&payload(body)).unwrap();
    assert_eq!(lists.len(), 2, "the named sibling must survive");
    assert_eq!(lists[0].name, "");
    assert_eq!(lists[1].name, "Kept");
}

/// The counterpart to the three tests above: `id` is the one field that is *not*
/// relaxed, in any of the three types.
///
/// **What "rejected" means changed, and the change is deliberate.** It used to mean the
/// whole array failed with [`SubsonicError::BadResponse`]. It now means the record is
/// **dropped** and its siblings survive — `parse`'s collection reader deserializes
/// element-by-element. The invariant this test exists to pin is unchanged and is asserted
/// harder below: *a record with no usable `id` never reaches the frontend*. What is gone
/// is the collateral damage, which was the real defect: one id-less record on album page 1
/// used to fail `connect()` itself, because `useSubsonic.ts` fetches that page inside its
/// try block and `friendlyConnectError` has no pattern for "Bad response".
#[test]
fn records_missing_an_id_are_still_rejected() {
    // id is load-bearing: streaming, downloading, scrobbling and art all key off it, and
    // attach_song_urls would mint four garbage URLs from an empty one.
    let no_id_album = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"name":"X","artist":"Y"}]}}}"#;
    assert!(
        parse::album_list(&payload(no_id_album)).unwrap().is_empty(),
        "an album with no id must not reach the frontend"
    );

    let no_id_song = r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[{"title":"X","artist":"Y"}]}}}"#;
    assert!(
        parse::random_songs(&payload(no_id_song))
            .unwrap()
            .is_empty(),
        "a song with no id must not reach the frontend"
    );

    let no_id_playlist =
        r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"name":"X"}]}}}"#;
    assert!(
        parse::playlists(&payload(no_id_playlist))
            .unwrap()
            .is_empty(),
        "a playlist with no id must not reach the frontend"
    );
}

/// The other half of the same rule, and the reason the rule was relaxed from "fail the
/// array" to "drop the record": the siblings of a broken record must still arrive.
#[test]
fn one_unusable_record_does_not_take_down_its_siblings() {
    // Three albums; the middle one has no `id` at all, the third has an `id` of a type
    // that can never be one. Both are dropped; the good ones are untouched.
    let body = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[
        {"id":"al-1","name":"Kept","artist":"A"},
        {"name":"No Id At All","artist":"B"},
        {"id":{"nope":true},"name":"Unusable Id","artist":"C"},
        {"id":"al-4","name":"Also Kept","artist":"D"}
    ]}}}"#;
    let albums = parse::album_list(&payload(body)).unwrap();
    assert_eq!(
        albums.len(),
        2,
        "the two well-formed albums must survive their broken neighbours"
    );
    assert_eq!(albums[0].name, "Kept");
    assert_eq!(albums[1].name, "Also Kept");

    // Same rule on the song path, which is what an album page and a playlist walk use.
    let songs_body = r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[
        {"id":"so-1","title":"First"},
        {"title":"Headless"},
        {"id":"so-3","title":"Third"}
    ]}}}"#;
    let songs = parse::random_songs(&payload(songs_body)).unwrap();
    assert_eq!(songs.len(), 2, "the id-less track must not blank the page");
    assert_eq!(songs[0].title, "First");
    assert_eq!(songs[1].title, "Third");
}

#[test]
fn a_structurally_broken_album_is_dropped_not_fatal() {
    // No `id`. The TypeScript's `as SubAlbum[]` cast cannot detect this and would hand the
    // UI an album whose id is `undefined`; `eko-net` refuses to surface it — but refusing
    // it is now a per-record decision, not a per-page one. See
    // `records_missing_an_id_are_still_rejected` for why that distinction mattered.
    let body = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"name":"X","artist":"Y"}]}}}"#;
    assert!(parse::album_list(&payload(body)).unwrap().is_empty());
}

/// An **empty-string** `id` is as unusable as an absent one, and is rejected the same way.
///
/// This is the case three doc comments used to warn about while not actually covering: the
/// plain `String` derive accepted `""` happily, and `attach_song_urls` would then mint four
/// URLs ending `&id=` — a track the UI renders but that can never stream, download or
/// scrobble. `lenient::id` being the single place any `id` is read is what lets this be
/// closed at all.
#[test]
fn an_empty_string_id_is_rejected_like_a_missing_one() {
    let body = r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[
        {"id":"","title":"Unplayable"},
        {"id":"   ","title":"Also Unplayable"},
        {"id":"so-3","title":"Fine"}
    ]}}}"#;
    let songs = parse::random_songs(&payload(body)).unwrap();
    assert_eq!(songs.len(), 1, "both unusable ids must be dropped");
    assert_eq!(songs[0].id, "so-3");

    // Same on albums and playlists — one helper, one rule.
    let al =
        r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"","name":"N"}]}}}"#;
    assert!(parse::album_list(&payload(al)).unwrap().is_empty());
    let pl =
        r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":"","name":"N"}]}}}"#;
    assert!(parse::playlists(&payload(pl)).unwrap().is_empty());
}

// ---------------------------------------- single-element lists as bare objects
//
// Several XML-derived Subsonic JSON encoders collapse a one-element list into the object
// itself. This is the highest-ranked remaining non-Navidrome risk, and it briefly read as
// an **empty** vec — a library that connected cleanly and showed nothing, which is strictly
// worse than the `BadResponse` it replaced, because there was nothing left to diagnose.

#[test]
fn a_single_element_list_encoded_as_a_bare_object_yields_one_record() {
    // `album` is the object itself, not an array containing it.
    let body = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":
        {"id":"al-1","name":"Lonely Album","artist":"A","year":"1998"}
    }}}"#;
    let albums = parse::album_list(&payload(body)).unwrap();
    assert_eq!(
        albums.len(),
        1,
        "a bare object must read as a one-element list, not as an empty library"
    );
    assert_eq!(albums[0].id, "al-1");
    assert_eq!(albums[0].name, "Lonely Album");
    assert_eq!(albums[0].year, Some(1998), "field leniency still applies");

    // The song path too — a one-track album, and a one-track playlist under `entry`.
    let song_body = r#"{"subsonic-response":{"status":"ok","album":
        {"id":"al-2","name":"One Track","song":{"id":"so-1","title":"Only"}}
    }}"#;
    let detail = parse::album(&payload(song_body)).unwrap();
    assert_eq!(detail.songs.len(), 1);
    assert_eq!(detail.songs[0].title, "Only");

    let pl_body = r#"{"subsonic-response":{"status":"ok","playlist":
        {"id":"pl-1","name":"Solo","entry":{"id":"so-9","title":"Single Entry"}}
    }}"#;
    let pl = parse::playlist(&payload(pl_body)).unwrap();
    assert_eq!(pl.songs.len(), 1);
    assert_eq!(pl.songs[0].id, "so-9");

    // And `starred2`, which reaches serde through a whole-object `from_value`.
    let st_body = r#"{"subsonic-response":{"status":"ok","starred2":
        {"song":{"id":"so-5","title":"Starred"},"album":{"id":"al-5","name":"Starred Album"}}
    }}"#;
    let starred = parse::starred(&payload(st_body)).unwrap();
    assert_eq!(starred.song.as_ref().map(Vec::len), Some(1));
    assert_eq!(starred.album.as_ref().map(Vec::len), Some(1));
}

#[test]
fn a_bare_object_that_is_itself_unusable_is_empty_not_an_error() {
    // The object is the whole "list" and it has no `id`. It is dropped like any other
    // unreadable record — an empty page, NOT `Err(BadResponse)`. One unusable record has
    // never been worth failing a request over, and this arm cannot make things worse than
    // the empty vec it replaced.
    let body = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":{"name":"No Id","artist":"A"}}}}"#;
    assert!(
        parse::album_list(&payload(body)).unwrap().is_empty(),
        "an unusable bare object is an empty page, not an error"
    );

    // A list key that is neither an array nor an object stays an empty page too.
    for junk in [r#""nope""#, "42", "true"] {
        let b =
            format!(r#"{{"subsonic-response":{{"status":"ok","albumList2":{{"album":{junk}}}}}}}"#);
        assert!(
            parse::album_list(&payload(&b)).unwrap().is_empty(),
            "a non-list `album` key of {junk} must be an empty page"
        );
    }
}

// ------------------------------------------------- loose numeric/id encodings
//
// The regression suite for the publish blocker. Every payload below used to fail the
// **whole page** with `BadResponse` (the last one silently read as `0`), and because
// `useSubsonic.ts`'s `connect()` fetches the first album page inside its try block, on
// page 1 that surfaced as a connect panel reading literally "Bad response" against a
// server the previous release handled fine. Navidrome is well-behaved; Gonic, Ampache,
// Astiga and LMS are the exposure, and this crate ships to a public MIT repo.
//
// One test per row of that table, in the same order.

#[test]
fn row_1_a_string_encoded_year_parses_instead_of_failing_the_page() {
    let body = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[
        {"id":"al-1","name":"OK Computer","artist":"Radiohead","year":"1997"}
    ]}}}"#;
    let albums = parse::album_list(&payload(body)).unwrap();
    assert_eq!(albums.len(), 1, "a string year must not fail the page");
    assert_eq!(albums[0].year, Some(1997));
}

#[test]
fn row_2_a_float_duration_truncates_instead_of_failing_the_page() {
    let body = r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[
        {"id":"so-1","title":"Angel","duration":251.5}
    ]}}}"#;
    let songs = parse::random_songs(&payload(body)).unwrap();
    assert_eq!(songs.len(), 1, "a float duration must not fail the page");
    assert_eq!(
        songs[0].duration,
        Some(251),
        "seconds truncate; the frontend has always read this as an integer"
    );
}

#[test]
fn row_3_a_numeric_id_is_stringified_instead_of_failing_the_page() {
    let body = r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[
        {"id":123,"title":"Integer Ids Are Real"}
    ]}}}"#;
    let songs = parse::random_songs(&payload(body)).unwrap();
    assert_eq!(songs.len(), 1, "a numeric id must not fail the page");
    assert_eq!(songs[0].id, "123");

    // Same on albums and playlists — every id in the crate goes through the same helper.
    let al =
        r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":4471,"name":"N"}]}}}"#;
    assert_eq!(parse::album_list(&payload(al)).unwrap()[0].id, "4471");
    let pl =
        r#"{"subsonic-response":{"status":"ok","playlists":{"playlist":[{"id":7,"name":"N"}]}}}"#;
    assert_eq!(parse::playlists(&payload(pl)).unwrap()[0].id, "7");
}

#[test]
fn row_4_a_nonsensical_year_reads_as_absent_instead_of_failing_the_page() {
    // -1 is how several servers spell "unknown". It must be `None` — never `u32::MAX`,
    // which would render as the year 4294967295, and never a failed page.
    let body = r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[
        {"id":"al-1","name":"Untagged Rip","artist":"A","year":-1},
        {"id":"al-2","name":"Also Untagged","artist":"B","year":"unknown"},
        {"id":"al-3","name":"Empty String","artist":"C","year":""}
    ]}}}"#;
    let albums = parse::album_list(&payload(body)).unwrap();
    assert_eq!(albums.len(), 3, "none of these may fail the page");
    assert!(
        albums.iter().all(|a| a.year.is_none()),
        "a year that cannot be one reads as absent, exactly like a server that omitted it"
    );
}

#[test]
fn row_5_string_encoded_replaygain_parses_instead_of_failing_the_page() {
    let body = r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[
        {"id":"so-1","title":"T","replayGain":{"trackGain":"-7.2","albumGain":"-6.85","trackPeak":"0.977","albumPeak":"1.0"}}
    ]}}}"#;
    let songs = parse::random_songs(&payload(body)).unwrap();
    assert_eq!(songs.len(), 1, "string gains must not fail the page");
    let rg = songs[0].replay_gain.as_ref().expect("replayGain present");
    assert_eq!(rg.track_gain, Some(-7.2));
    assert_eq!(rg.album_gain, Some(-6.85));
    assert_eq!(rg.track_peak, Some(0.977));
    assert_eq!(rg.album_peak, Some(1.0));
}

#[test]
fn row_6_a_string_encoded_genre_song_count_is_read_not_silently_zeroed() {
    // This row did not fail — it was worse. `Value::as_u64` returned `None` for a string,
    // `unwrap_or(0)` made it zero, and the genre rendered as if it held no songs with
    // nothing anywhere to say otherwise.
    let body = r#"{"subsonic-response":{"status":"ok","genres":{"genre":[
        {"value":"Trip Hop","songCount":"42"},
        {"value":"Ambient","songCount":17},
        {"value":"Broken","songCount":"lots"}
    ]}}}"#;
    let genres = parse::genres(&payload(body)).unwrap();
    assert_eq!(genres.len(), 3);
    assert_eq!(genres[0].song_count, 42, "was silently 0 before the fix");
    assert_eq!(genres[1].song_count, 17);
    assert_eq!(
        genres[2].song_count, 0,
        "an unreadable count still degrades to 0 — but nothing else about the genre is lost"
    );
}

/// The six rows above as one realistic page, from a server that encodes loosely
/// throughout — the shape that actually broke `connect()`.
#[test]
fn a_loosely_encoded_album_page_parses_whole() {
    let detail = parse::album(&payload(include_str!("fixtures/loose_encodings.json"))).unwrap();

    // Album: numeric id, string songCount, string year.
    assert_eq!(detail.album.id, "4471");
    assert_eq!(detail.album.song_count, Some(3));
    assert_eq!(detail.album.year, Some(1998));

    // Two usable tracks; the third has no id at all and is dropped without collateral.
    assert_eq!(
        detail.songs.len(),
        2,
        "the id-less third track is dropped; its siblings survive"
    );

    let first = &detail.songs[0];
    assert_eq!(first.id, "44711", "numeric id, stringified");
    assert_eq!(first.duration, Some(379), "float duration, truncated");
    assert_eq!(first.bit_rate, Some(1005));
    assert_eq!(first.sampling_rate, Some(44100));
    assert_eq!(first.channel_count, Some(2));
    assert_eq!(first.track, Some(1));
    let rg = first.replay_gain.as_ref().expect("replayGain present");
    assert_eq!(rg.track_gain, Some(-7.2));
    assert_eq!(rg.album_peak, Some(1.0));

    let second = &detail.songs[1];
    assert_eq!(second.id, "so-44712");
    assert_eq!(second.duration, Some(297));
    assert_eq!(second.bit_rate, None, "-1 is 'unknown', not u32::MAX");
    assert_eq!(second.sampling_rate, None, "\"unknown\" reads as absent");
    assert_eq!(second.channel_count, None, "null reads as absent");
    assert_eq!(second.track, Some(2));
}

// ---------------------------------------------------------------- getAlbum

#[test]
fn album_lifts_songs_out_of_the_album_object() {
    let detail = parse::album(&payload(include_str!("fixtures/album.json"))).unwrap();
    assert_eq!(detail.album.id, "al-0002");
    assert_eq!(detail.album.name, "Kid A");
    assert_eq!(detail.songs.len(), 2);

    let first = &detail.songs[0];
    assert_eq!(first.id, "so-00021");
    assert_eq!(first.title, "Everything In Its Right Place");
    assert_eq!(first.artist, "Radiohead");
    assert_eq!(first.album, "Kid A");
    assert_eq!(first.duration, Some(251));
    assert_eq!(first.bit_rate, Some(1411));
    assert_eq!(first.sampling_rate, Some(44100));
    assert_eq!(first.channel_count, Some(2));
    assert_eq!(first.suffix.as_deref(), Some("flac"));
    assert_eq!(first.content_type.as_deref(), Some("audio/flac"));
    assert_eq!(first.track, Some(1));
    assert_eq!(first.cover_art.as_deref(), Some("mf-00021"));

    let rg = first.replay_gain.as_ref().expect("replayGain present");
    assert_eq!(rg.track_gain, Some(-6.48));
    assert_eq!(rg.album_gain, Some(-7.21));
    assert_eq!(rg.track_peak, Some(0.988_556));
    assert_eq!(rg.album_peak, Some(0.999_969));
}

#[test]
fn album_with_no_song_key_yields_no_songs() {
    let body =
        r#"{"subsonic-response":{"status":"ok","album":{"id":"al-1","name":"N","artist":"A"}}}"#;
    let detail = parse::album(&payload(body)).unwrap();
    assert!(
        detail.songs.is_empty(),
        "`album.song ?? []` — an album with no tracks listed is not an error"
    );
}

#[test]
fn album_with_no_album_key_is_an_error() {
    // client.ts:109-110 dereferences `album.song` on an unchecked cast, so a response
    // with no `album` throws there too. This is the one place a missing key is fatal.
    let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    assert_eq!(
        parse::album(&payload(body)).unwrap_err(),
        SubsonicError::BadResponse
    );
}

#[test]
fn songs_without_replaygain_parse_with_none() {
    let detail = parse::album(&payload(include_str!("fixtures/song_no_replaygain.json"))).unwrap();
    assert_eq!(detail.songs.len(), 2);
    assert!(
        detail.songs.iter().all(|s| s.replay_gain.is_none()),
        "a server with no ReplayGain support must parse cleanly, not fail"
    );
    // Other absences on the same songs, for good measure.
    assert_eq!(detail.album.cover_art, None);
    assert_eq!(detail.songs[0].content_type, None);
    assert_eq!(detail.songs[0].bit_rate, None);
    assert_eq!(detail.songs[0].track, None);
}

// ---------------------------------------------------------------- search3

#[test]
fn search_result_splits_albums_and_songs() {
    let result = parse::search_result(&payload(include_str!("fixtures/search3.json"))).unwrap();
    assert_eq!(result.albums.len(), 1);
    assert_eq!(result.albums[0].id, "al-0002");
    assert_eq!(result.songs.len(), 1);
    assert_eq!(result.songs[0].id, "so-00022");
}

#[test]
fn search_result_with_no_container_is_two_empty_vecs() {
    let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    let result = parse::search_result(&payload(body)).unwrap();
    assert!(result.albums.is_empty());
    assert!(
        result.songs.is_empty(),
        "a search with no hits omits searchResult3 entirely — `sr?.song ?? []`"
    );
}

// ---------------------------------------------------------------- getPlaylists / getPlaylist

#[test]
fn playlists_parse_with_optional_cover_art() {
    let lists = parse::playlists(&payload(include_str!("fixtures/playlists.json"))).unwrap();
    assert_eq!(lists.len(), 2);
    assert_eq!(lists[0].id, "pl-01");
    assert_eq!(lists[0].name, "Late Night");
    assert_eq!(lists[0].song_count, Some(42));
    assert_eq!(lists[0].cover_art.as_deref(), Some("pl-pl-01"));
    assert_eq!(lists[1].song_count, Some(0));
    assert_eq!(lists[1].cover_art, None);
}

#[test]
fn playlist_reads_tracks_from_entry_not_song() {
    let detail = parse::playlist(&payload(include_str!("fixtures/playlist.json"))).unwrap();
    assert_eq!(detail.name, "Late Night");
    assert_eq!(
        detail.songs.len(),
        2,
        "getPlaylist nests tracks under `entry`; reading `song` would silently yield none"
    );
    assert_eq!(detail.songs[0].id, "so-00021");
    assert_eq!(detail.songs[1].title, "Teardrop");
}

#[test]
fn playlist_without_a_name_falls_back_to_the_literal_playlist() {
    let body = r#"{"subsonic-response":{"status":"ok","playlist":{"id":"pl-9","entry":[]}}}"#;
    let detail = parse::playlist(&payload(body)).unwrap();
    assert_eq!(
        detail.name, "Playlist",
        "client.ts:135 substitutes the literal \"Playlist\" for a missing name"
    );
    assert!(detail.songs.is_empty());
}

// ---------------------------------------------------------------- getRandomSongs

#[test]
fn random_songs_parse() {
    let songs = parse::random_songs(&payload(include_str!("fixtures/random_songs.json"))).unwrap();
    assert_eq!(songs.len(), 2);
    assert_eq!(songs[0].title, "Windowlicker");
    // Partial replayGain: only the track values are present, album values absent.
    let rg = songs[0].replay_gain.as_ref().unwrap();
    assert_eq!(rg.track_gain, Some(-8.11));
    assert_eq!(rg.album_gain, None);
    assert_eq!(rg.track_peak, Some(1.0));
    assert_eq!(rg.album_peak, None);
}

// ---------------------------------------------------------------- getGenres

#[test]
fn genres_drop_album_count_and_default_missing_fields() {
    let genres = parse::genres(&payload(include_str!("fixtures/genres.json"))).unwrap();
    assert_eq!(genres.len(), 4);
    assert_eq!(genres[0].value, "Ambient");
    assert_eq!(genres[0].song_count, 214);
    assert!(
        genres.iter().all(|g| g.album_count.is_none()),
        "client.ts:158-161 rebuilds each genre as {{value, songCount}}, dropping albumCount — \
         passing it through would change what the frontend receives"
    );
    assert_eq!(
        genres[3].value, "",
        "a missing `value` defaults to the empty string"
    );
    assert_eq!(
        genres[3].song_count, 0,
        "a missing `songCount` defaults to 0"
    );
}

// ---------------------------------------------------------------- getSimilarSongs2

#[test]
fn similar_songs_parse() {
    let songs =
        parse::similar_songs(&payload(include_str!("fixtures/similar_songs2.json"))).unwrap();
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].title, "Xtal");
}

#[test]
fn similar_songs_with_no_matches_is_empty() {
    let body = r#"{"subsonic-response":{"status":"ok","similarSongs2":{}}}"#;
    assert!(parse::similar_songs(&payload(body)).unwrap().is_empty());
}

// ---------------------------------------------------------------- getArtistInfo2

#[test]
fn artist_info_parses_bio_and_similar_artists() {
    let info = parse::artist_info(&payload(include_str!("fixtures/artist_info2.json"))).unwrap();
    assert!(info.biography.unwrap().starts_with("Richard David James"));
    assert_eq!(
        info.last_fm_url.as_deref(),
        Some("https://www.last.fm/music/Aphex+Twin")
    );
    let similar = info.similar_artist.unwrap();
    assert_eq!(similar.len(), 2);
    assert_eq!(similar[0].name, "Squarepusher");
}

#[test]
fn artist_info_absent_is_all_empty_not_an_error() {
    let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    let info = parse::artist_info(&payload(body)).unwrap();
    assert_eq!(info.biography, None);
    assert_eq!(info.last_fm_url, None);
    assert_eq!(
        info.similar_artist, None,
        "client.ts:212 substitutes `{{}}` — servers without a Last.fm agent send nothing"
    );
}

// ---------------------------------------------------------------- getStarred2

#[test]
fn starred_parses_songs_and_albums() {
    let starred = parse::starred(&payload(include_str!("fixtures/starred2.json"))).unwrap();
    assert_eq!(starred.song.as_ref().unwrap().len(), 1);
    assert_eq!(starred.album.as_ref().unwrap().len(), 1);
    assert_eq!(starred.song.unwrap()[0].id, "so-00021");
}

#[test]
fn starred_absent_is_all_empty_not_an_error() {
    let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
    let starred = parse::starred(&payload(body)).unwrap();
    assert_eq!(starred.song, None);
    assert_eq!(starred.album, None);
}

// ---------------------------------------------------------------- lyrics

#[test]
fn synced_lyrics_win_over_an_unsynced_entry_listed_first() {
    let result =
        parse::lyrics_by_song_id(&payload(include_str!("fixtures/lyrics_synced.json"))).unwrap();
    let lines = result
        .synced
        .expect("the synced entry must win regardless of array order");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].start, 12_400);
    assert_eq!(lines[0].value, "Everything in its right place");
    assert_eq!(lines[2].start, 25_130);
    assert_eq!(
        result.unsynced, None,
        "the synced branch must not also populate unsynced"
    );
}

#[test]
fn unsynced_lyrics_join_with_newlines_and_tolerate_missing_start() {
    let result =
        parse::lyrics_by_song_id(&payload(include_str!("fixtures/lyrics_unsynced.json"))).unwrap();
    assert_eq!(
        result.synced, None,
        "a synced entry with zero lines must not be chosen"
    );
    assert_eq!(
        result.unsynced.as_deref(),
        Some("Love, love is a verb\nLove is a doing word\nFeathers on my breath"),
        "Navidrome omits `start` on unsynced lines; requiring it would send this response \
         down the legacy fallback and lose lyrics the server actually had"
    );
}

#[test]
fn lyrics_list_absent_or_empty_is_no_lyrics() {
    for body in [
        r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#,
        r#"{"subsonic-response":{"status":"ok","lyricsList":{}}}"#,
        r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[]}}}"#,
    ] {
        let result = parse::lyrics_by_song_id(&payload(body)).unwrap();
        assert_eq!(result.synced, None);
        assert_eq!(result.unsynced, None);
    }
}

#[test]
fn legacy_lyrics_trim_the_plain_text_blob() {
    let result =
        parse::lyrics_legacy(&payload(include_str!("fixtures/lyrics_legacy.json"))).unwrap();
    assert_eq!(result.synced, None);
    assert_eq!(
        result.unsynced.as_deref(),
        Some("Love, love is a verb\nLove is a doing word\nFeathers on my breath"),
        "client.ts:320 trims the outer whitespace but keeps the inner newlines"
    );
}

#[test]
fn legacy_lyrics_that_are_blank_or_absent_become_none() {
    for body in [
        r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#,
        r#"{"subsonic-response":{"status":"ok","lyrics":{}}}"#,
        r#"{"subsonic-response":{"status":"ok","lyrics":{"value":"   \n  "}}}"#,
    ] {
        assert_eq!(
            parse::lyrics_legacy(&payload(body)).unwrap().unsynced,
            None,
            "whitespace-only lyrics must collapse to None, not an empty string"
        );
    }
}

// ---------------------------------------------------------------- errors

#[test]
fn wrong_password_is_an_error_despite_http_200() {
    let body = include_str!("fixtures/error_wrong_password.json");
    let err = parse::envelope(body).unwrap_err();
    assert_eq!(
        err,
        SubsonicError::Subsonic("Wrong username or password".into()),
        "Subsonic reports auth failure in the body with HTTP 200 — checking only the \
         status code would read a wrong password as an empty library"
    );
    assert_eq!(err.to_string(), "Wrong username or password");
}

// ---------------------------------------------------------------- URL attachment

#[test]
fn attaching_song_urls_fills_all_four_fields_and_keeps_download_distinct() {
    let mut song = parse::album(&payload(include_str!("fixtures/album.json")))
        .unwrap()
        .songs
        .remove(0);
    assert!(song.stream_url.is_none(), "parse must not mint URLs");

    urls::attach_song_urls(&cfg(), &mut song);

    let stream = song.stream_url.unwrap();
    assert!(
        stream.starts_with("stream://localhost/?src="),
        "the webview URL must go through the proxy"
    );

    let src = song.stream_src_url.unwrap();
    assert!(
        src.starts_with("https://music.example.com/rest/stream?"),
        "the native engine's URL must be direct and unwrapped"
    );

    let download = song.download_url.unwrap();
    assert!(
        download.starts_with("https://music.example.com/rest/download?"),
        "the offline cache must be pointed at `download` (original bytes), never `stream` \
         (which the server may transcode) — swapping these breaks bit-perfect caching silently"
    );
    assert!(!download.contains("/rest/stream"));

    let cover = song.cover_url.unwrap();
    assert!(cover.starts_with("stream://localhost/?src="));
    // `size%3D` rather than `size`: the salt is base36, so it could in principle contain
    // the bare substring "size" and flake the test. The encoded `=` cannot.
    assert!(
        !cover.contains("size%3D"),
        "the minted cover URL must be SIZE-AGNOSTIC. The app renders art at seven edge \
         sizes (80/120/160/200/300/512/600), so one baked size cannot serve them all; the \
         size rides on the outer stream:// URL and stream.rs applies it. Baking one in \
         here would silently pin every slot to that size. Got: {cover}"
    );
}

/// The size that *is* baked in — not into the URL, but as the fallback `stream.rs`
/// applies when the outer URL carries none. It is still the TypeScript's `size = 300`
/// (`client.ts:241`); only the place it is applied has moved.
#[test]
fn the_default_cover_size_is_still_the_typescripts_300() {
    assert_eq!(urls::DEFAULT_COVER_SIZE, 300);
}

/// The parameter is retained because the endpoint genuinely supports it — this crate
/// just always passes `None`. If a non-proxied consumer ever needs a sized URL, this is
/// the behaviour it gets.
#[test]
fn an_explicit_size_is_still_honoured_when_asked_for() {
    let sized = urls::cover_art_url(&cfg(), Some("al-1"), Some(600), "saltsaltxx").unwrap();
    assert!(sized.contains("size%3D600"));

    let agnostic = urls::cover_art_url(&cfg(), Some("al-1"), None, "saltsaltxx").unwrap();
    assert!(!agnostic.contains("size%3D"));

    assert!(
        urls::cover_art_url(&cfg(), None, Some(600), "saltsaltxx").is_none(),
        "no coverArt id still means None, size or no size"
    );
}

/// [`urls::cover_art_src_url`] is the direct sibling of [`urls::cover_art_url`], for
/// consumers (e.g. `eko-cli`) with no `stream://` protocol handler to resolve a proxied
/// URL. It must never look like the proxied builder: no `stream://`, direct upstream,
/// carries the real auth params, and — unlike the proxied builder — bakes its size in
/// because there is no proxy downstream to add one later.
#[test]
fn cover_art_src_url_is_direct_authenticated_and_sized() {
    let c = cfg();

    let cover = urls::cover_art_src_url(&c, Some("al-1"), Some(600), "saltsaltxx").unwrap();
    assert!(
        cover.starts_with("https://music.example.com/rest/getCoverArt?"),
        "must be direct, hitting Navidrome's base_url, not the stream:// proxy"
    );
    assert!(!cover.contains("stream://"));
    // Trailing slash on base_url must be stripped exactly once, as the other direct
    // builders do (base_url_trimmed).
    assert!(!cover.contains("com//rest"));

    // Auth params travel with it: username and salt appear verbatim, and a token is
    // present. Critically, the plaintext password must never appear — Subsonic's `p=`
    // param is intentionally unsupported everywhere in this crate.
    assert!(cover.contains("u=rod"));
    assert!(cover.contains("s=saltsaltxx"));
    assert!(cover.contains("t="), "must carry the hashed token");
    assert!(
        !cover.contains("hunter2"),
        "the plaintext password must never appear in a minted URL"
    );
    assert!(
        !cover.split(['?', '&']).any(|p| p.starts_with("p=")),
        "must never carry Subsonic's plaintext p= param"
    );

    // A requested size is honoured verbatim.
    assert!(cover.contains("size=600"), "got: {cover}");

    // None falls back to something sensible rather than omitting size entirely — there
    // is no stream:// proxy downstream to apply a default the way stream.rs does for
    // the proxied builder.
    let default_sized = urls::cover_art_src_url(&c, Some("al-1"), None, "saltsaltxx").unwrap();
    assert!(
        default_sized.contains(&format!("size={}", urls::DEFAULT_COVER_SIZE)),
        "None must fall back to DEFAULT_COVER_SIZE, not omit size: {default_sized}"
    );

    // No coverArt id means no art to fetch, matching cover_art_url's None behaviour.
    assert!(
        urls::cover_art_src_url(&c, None, Some(600), "saltsaltxx").is_none(),
        "no coverArt id -> None, matching cover_art_url"
    );
}

#[test]
fn attaching_urls_to_art_less_items_leaves_cover_url_none() {
    let mut albums = parse::album_list(&payload(include_str!("fixtures/albumlist2.json"))).unwrap();
    for album in &mut albums {
        urls::attach_album_urls(&cfg(), album);
    }
    assert!(albums[0].cover_url.is_some());
    assert!(
        albums[2].cover_url.is_none(),
        "no coverArt id means there is no art to fetch — must stay None, not an empty string"
    );

    let mut song = SubSong {
        id: "so-1".into(),
        title: "T".into(),
        artist: "A".into(),
        album: "Al".into(),
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
    urls::attach_song_urls(&cfg(), &mut song);
    assert!(song.cover_url.is_none());
    assert!(song.stream_url.is_some(), "streamable regardless of art");
}

#[test]
fn each_attached_url_carries_its_own_fresh_salt() {
    let mut song = parse::album(&payload(include_str!("fixtures/album.json")))
        .unwrap()
        .songs
        .remove(0);
    urls::attach_song_urls(&cfg(), &mut song);

    // Only the two direct URLs are inspected: the proxied ones percent-encode their
    // whole upstream into a single `src` value, so `s=` isn't a top-level param there.
    fn salt_of(url: &str) -> String {
        url.split(['?', '&'])
            .find_map(|p| p.strip_prefix("s=").map(str::to_string))
            .expect("every minted URL must carry a salt")
    }

    let direct = salt_of(song.stream_src_url.as_deref().unwrap());
    let download = salt_of(song.download_url.as_deref().unwrap());
    assert_eq!(direct.len(), 10);
    assert_ne!(
        direct, download,
        "salts are per-request nonces; reusing one across URLs is pointless coupling"
    );
}

// ---------------------------------------------------------------- mime_for_song

#[test]
fn mime_prefers_the_servers_content_type_over_the_suffix() {
    let songs = parse::random_songs(&payload(include_str!("fixtures/random_songs.json"))).unwrap();
    // suffix "opus" but contentType "audio/ogg" — the server wins.
    assert_eq!(songs[1].suffix.as_deref(), Some("opus"));
    assert_eq!(mime_for_song(&songs[1]), "audio/ogg");
}

#[test]
fn mime_maps_every_suffix_in_the_table_case_insensitively() {
    let with = |content_type: Option<&str>, suffix: Option<&str>| SubSong {
        id: "s".into(),
        title: "t".into(),
        artist: "a".into(),
        album: "al".into(),
        duration: None,
        bit_rate: None,
        sampling_rate: None,
        channel_count: None,
        suffix: suffix.map(str::to_string),
        content_type: content_type.map(str::to_string),
        track: None,
        cover_art: None,
        replay_gain: None,
        stream_url: None,
        stream_src_url: None,
        download_url: None,
        cover_url: None,
        server: None,
    };

    for (suffix, expected) in [
        ("flac", "audio/flac"),
        ("FLAC", "audio/flac"),
        ("mp3", "audio/mpeg"),
        ("m4a", "audio/mp4"),
        ("aac", "audio/aac"),
        ("ogg", "audio/ogg"),
        ("Opus", "audio/opus"),
        ("wav", "audio/wav"),
    ] {
        assert_eq!(mime_for_song(&with(None, Some(suffix))), expected);
    }

    // Unknown suffix, no suffix at all, and an empty contentType all fall back.
    assert_eq!(mime_for_song(&with(None, Some("aiff"))), "audio/mpeg");
    assert_eq!(mime_for_song(&with(None, None)), "audio/mpeg");
    assert_eq!(
        mime_for_song(&with(Some(""), Some("flac"))),
        "audio/flac",
        "an empty contentType is falsy in `if (s.contentType)` and must fall through"
    );
}

#[test]
fn an_attached_song_names_its_server_and_the_name_carries_no_credential() {
    let mut song = SubSong {
        id: "so-1".into(),
        title: "T".into(),
        artist: "A".into(),
        album: "Al".into(),
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
    urls::attach_song_urls(&cfg(), &mut song);
    let server = song.server.expect("attach_song_urls stamps the server");
    assert_eq!(server, urls::server_key(&cfg()));
    assert_eq!(server, "https://music.example.com");
    let config = cfg();
    for secret in [
        "u=",
        "t=",
        "s=",
        config.username.as_str(),
        config.password.as_str(),
    ] {
        assert!(!server.contains(secret), "{secret:?} in {server}");
    }
}
