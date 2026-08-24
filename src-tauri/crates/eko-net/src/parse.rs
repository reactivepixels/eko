//! Response envelope + payload parsing for the OpenSubsonic JSON API.
//!
//! [`envelope`] mirrors `client.ts`'s `call()` (repo root
//! `src/subsonic/client.ts:76-90`): unwrap the `subsonic-response` envelope, and when
//! `status != "ok"` raise `error.message`, falling back to `"Subsonic error"`.
//!
//! Everything below it is a **pure** function from an already-unwrapped envelope
//! ([`serde_json::Value`]) to a typed payload. Nothing here touches the network, which
//! is what lets the fixture suite in `tests/fixtures.rs` cover the whole parsing
//! surface offline; [`crate::Client`] only adds "build URL, GET, check status".
//!
//! Two habits from the TypeScript are load-bearing and reproduced deliberately:
//!
//! * **`?? []` everywhere.** A missing *or* `null` collection key is an empty vec, never
//!   an error — `{"albumList2":{}}` with no `album` key at all is a normal, healthy
//!   response from a server with an empty library.
//! * **Counts are never inferred.** These parsers report exactly what the server sent.
//!   A page shorter than the requested `size` is not a signal that the walk is over —
//!   some servers silently cap page size, and treating a short page as the last page
//!   caused a real library-truncation bug (see the termination-rule comment at
//!   `src/subsonic/useSubsonic.ts:185`). Page-walk termination is the caller's business.

use std::fmt;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use crate::types::{
    AlbumDetail, LyricsResult, PlaylistDetail, SearchResult, SubAlbum, SubArtistInfo, SubGenre,
    SubPlaylist, SubSong, SubStarred, SyncedLyricLine,
};

/// Errors surfaced while talking to a Subsonic/Navidrome server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubsonicError {
    /// The HTTP request itself failed with a non-2xx status, matching `client.ts`'s
    /// `if (!res.ok) throw new Error(\`HTTP ${res.status}\`)`.
    Http(u16),
    /// The request never completed — DNS failure, refused connection, TLS error. The
    /// TypeScript has no distinct case for this (a rejected `fetch` and a thrown
    /// `HTTP 500` are both just exceptions); Rust makes it explicit.
    Request(String),
    /// The body didn't contain a `subsonic-response` envelope at all, matching
    /// `client.ts`'s `if (!sr) throw new Error("Bad response")`. Also raised when a
    /// payload is present but structurally unusable — e.g. a song missing `id`. The
    /// TypeScript can't detect that case at all (its `as` casts are unchecked, so the
    /// field silently becomes `undefined` downstream); Rust must reject it.
    BadResponse,
    /// The envelope's `status` was not `"ok"`, matching `client.ts`'s
    /// `err?.message ?? "Subsonic error"`.
    Subsonic(String),
}

impl fmt::Display for SubsonicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubsonicError::Http(status) => write!(f, "HTTP {status}"),
            SubsonicError::Request(message) => write!(f, "{message}"),
            SubsonicError::BadResponse => write!(f, "Bad response"),
            SubsonicError::Subsonic(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SubsonicError {}

/// Extract the `subsonic-response` payload from a JSON response body.
///
/// Mirrors `client.ts:82-89` exactly: a body that doesn't parse as JSON, or that
/// parses but has no `subsonic-response` key, is [`SubsonicError::BadResponse`]. A
/// `subsonic-response` whose `status` isn't `"ok"` is [`SubsonicError::Subsonic`],
/// carrying `error.message` when present or else the literal `"Subsonic error"`.
pub fn envelope(body: &str) -> Result<serde_json::Value, SubsonicError> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|_| SubsonicError::BadResponse)?;

    let sr = json
        .get("subsonic-response")
        .ok_or(SubsonicError::BadResponse)?;

    let status = sr
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if status != "ok" {
        let message = sr
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Subsonic error")
            .to_string();
        return Err(SubsonicError::Subsonic(message));
    }

    Ok(sr.clone())
}

/// Deserialize `sr[container][key]` as a list, treating an absent *or* `null` key as an
/// empty list. This is the `?? []` of `client.ts:104`, `116`, `129`, `135`, `143`, `189`
/// and `196` — the single most repeated idiom in the file, and the reason a server with
/// nothing to report is not an error.
///
/// **Element-by-element, and a failing element is dropped rather than fatal.** This is the
/// array-level half of the leniency [`crate::lenient`] argues for at the field level, and
/// the same philosophy [`crate::types::SubSong::title`] already states: one bad record
/// must not take down the page around it. Deserializing the whole `Value` in one go —
/// which this used to do — meant a single unreadable record blanked every sibling on the
/// page with [`SubsonicError::BadResponse`], and because `useSubsonic.ts`'s `connect()`
/// fetches the first album page *inside* its try block, on page 1 it blanked the
/// connection itself: the user got a connect panel reading "Bad response".
///
/// A **bare object** where an array belongs is read as a single-element list, because
/// several XML-derived Subsonic JSON encoders collapse one-element lists that way — see
/// [`crate::lenient::vec_skipping_bad`]. Dropped records are reported on stderr, so a short
/// page is diagnosable rather than silent.
///
/// The return type stays a `Result` because callers read it as one; it is now `Ok` for
/// every input.
fn collection<T: DeserializeOwned>(
    sr: &Value,
    container: &str,
    key: &str,
) -> Result<Vec<T>, SubsonicError> {
    match sr.get(container).and_then(|c| c.get(key)) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(list) => Ok(crate::lenient::vec_skipping_bad(list)),
    }
}

/// Deserialize `sr[key]` as `T`, or return `T::default()` when the key is absent or
/// `null`. Mirrors the `(r.x as T | undefined) ?? {}` of `client.ts:212` and `222`.
fn object_or_default<T: DeserializeOwned + Default>(
    sr: &Value,
    key: &str,
) -> Result<T, SubsonicError> {
    match sr.get(key) {
        None | Some(Value::Null) => Ok(T::default()),
        Some(obj) => serde_json::from_value(obj.clone()).map_err(|_| SubsonicError::BadResponse),
    }
}

/// `getAlbumList2` -> `albumList2.album`. Mirrors `client.ts:103-104` and `188-189`.
///
/// The returned length is whatever the server sent and carries **no** meaning about
/// whether more pages exist; see the module docs.
pub fn album_list(sr: &Value) -> Result<Vec<SubAlbum>, SubsonicError> {
    collection(sr, "albumList2", "album")
}

/// `getAlbum` -> the album plus its tracks (`album.song ?? []`). Mirrors
/// `client.ts:108-110`.
///
/// Unlike the collection parsers, a missing `album` key *is* an error: the TypeScript
/// would dereference `undefined.song` and throw here too.
pub fn album(sr: &Value) -> Result<AlbumDetail, SubsonicError> {
    let raw = sr.get("album").ok_or(SubsonicError::BadResponse)?;
    let album: SubAlbum =
        serde_json::from_value(raw.clone()).map_err(|_| SubsonicError::BadResponse)?;
    // Element-by-element, for the reason given on `collection`: one unreadable track must
    // not blank the album page around it.
    let songs = match raw.get("song") {
        None | Some(Value::Null) => Vec::new(),
        Some(list) => crate::lenient::vec_skipping_bad(list),
    };
    Ok(AlbumDetail { album, songs })
}

/// `search3` -> `searchResult3.{album,song}`. Mirrors `client.ts:115-116`. A response
/// with no `searchResult3` at all yields two empty vecs, exactly like `sr?.album ?? []`.
pub fn search_result(sr: &Value) -> Result<SearchResult, SubsonicError> {
    Ok(SearchResult {
        albums: collection(sr, "searchResult3", "album")?,
        songs: collection(sr, "searchResult3", "song")?,
    })
}

/// `getPlaylists` -> `playlists.playlist`. Mirrors `client.ts:128-129`.
pub fn playlists(sr: &Value) -> Result<Vec<SubPlaylist>, SubsonicError> {
    collection(sr, "playlists", "playlist")
}

/// `getPlaylist` -> `{name, songs}`, where tracks live under `entry` (not `song`) and an
/// absent name falls back to the literal `"Playlist"`. Mirrors `client.ts:134-135`.
pub fn playlist(sr: &Value) -> Result<PlaylistDetail, SubsonicError> {
    let raw = sr.get("playlist").ok_or(SubsonicError::BadResponse)?;
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Playlist")
        .to_string();
    // Element-by-element; see `collection`.
    let songs = match raw.get("entry") {
        None | Some(Value::Null) => Vec::new(),
        Some(list) => crate::lenient::vec_skipping_bad(list),
    };
    Ok(PlaylistDetail { name, songs })
}

/// `getRandomSongs` -> `randomSongs.song`. Mirrors `client.ts:142-143`.
pub fn random_songs(sr: &Value) -> Result<Vec<SubSong>, SubsonicError> {
    collection(sr, "randomSongs", "song")
}

/// `getGenres` -> `genres.genre`, narrowed to `{value, songCount}`.
///
/// Deliberately **drops** `albumCount` and defaults a missing `value` to `""` / a missing
/// `songCount` to `0`, because `client.ts:158-161` explicitly rebuilds each entry that
/// way rather than passing the server's object through. Widening this to include
/// `albumCount` would change what the frontend receives.
///
/// `songCount` goes through [`crate::lenient::u32_from_value`], not `Value::as_u64`: the
/// latter silently read `"songCount": "42"` — a string, which several non-Navidrome
/// servers send — as `0`, so the genre rendered with a count of nothing and nobody could
/// tell it apart from an empty genre.
pub fn genres(sr: &Value) -> Result<Vec<SubGenre>, SubsonicError> {
    let raw: Vec<Value> = collection(sr, "genres", "genre")?;
    Ok(raw
        .into_iter()
        .map(|g| SubGenre {
            value: g
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            song_count: g
                .get("songCount")
                .and_then(crate::lenient::u32_from_value)
                .unwrap_or(0),
            album_count: None,
        })
        .collect())
}

/// `getSimilarSongs2` -> `similarSongs2.song`. Mirrors `client.ts:195-196`.
pub fn similar_songs(sr: &Value) -> Result<Vec<SubSong>, SubsonicError> {
    collection(sr, "similarSongs2", "song")
}

/// `getArtistInfo2` -> `artistInfo2`, or an all-empty [`SubArtistInfo`] when absent.
/// Mirrors `client.ts:212`.
pub fn artist_info(sr: &Value) -> Result<SubArtistInfo, SubsonicError> {
    object_or_default(sr, "artistInfo2")
}

/// `getStarred2` -> `starred2`, or an all-empty [`SubStarred`] when absent. Mirrors
/// `client.ts:222`.
pub fn starred(sr: &Value) -> Result<SubStarred, SubsonicError> {
    object_or_default(sr, "starred2")
}

/// One entry of the OpenSubsonic `lyricsList.structuredLyrics` array.
///
/// `start` is `Option` here even though [`SyncedLyricLine::start`] is not: Navidrome
/// omits it on *unsynced* lines, and rejecting those would send every unsynced-lyrics
/// response down the legacy fallback path. Unsynced lines only ever have their `value`
/// read, so filling `start` with `0` is unobservable.
#[derive(Debug, Deserialize)]
struct RawStructuredLyrics {
    #[serde(default)]
    synced: bool,
    #[serde(default)]
    line: Vec<RawLyricLine>,
}

#[derive(Debug, Deserialize)]
struct RawLyricLine {
    /// Number-or-numeric-string, like every other numeric field — see [`crate::lenient`].
    #[serde(default, deserialize_with = "crate::lenient::opt_u32")]
    start: Option<u32>,
    #[serde(default)]
    value: String,
}

impl RawStructuredLyrics {
    fn lines(&self) -> Vec<SyncedLyricLine> {
        self.line
            .iter()
            .map(|l| SyncedLyricLine {
                start: l.start.unwrap_or(0),
                value: l.value.clone(),
            })
            .collect()
    }
}

/// `getLyricsBySongId` -> `lyricsList.structuredLyrics`, resolved to one
/// [`LyricsResult`]. Mirrors `client.ts:286-301`.
///
/// Preference order, exactly as the TypeScript has it: the first entry with
/// `synced == true` and at least one line wins and is returned as `synced`; otherwise
/// the first non-synced non-empty entry is joined with `\n` into `unsynced`; otherwise
/// both fields are `None`. Note an entry with no `synced` key counts as *un*synced,
/// because `!undefined` is `true`.
pub fn lyrics_by_song_id(sr: &Value) -> Result<LyricsResult, SubsonicError> {
    let list: Vec<RawStructuredLyrics> = collection(sr, "lyricsList", "structuredLyrics")?;

    if let Some(entry) = list.iter().find(|e| e.synced && !e.line.is_empty()) {
        return Ok(LyricsResult {
            synced: Some(entry.lines()),
            unsynced: None,
        });
    }
    if let Some(entry) = list.iter().find(|e| !e.synced && !e.line.is_empty()) {
        let text = entry
            .lines()
            .into_iter()
            .map(|l| l.value)
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(LyricsResult {
            synced: None,
            unsynced: Some(text),
        });
    }
    Ok(LyricsResult::EMPTY)
}

/// Legacy `getLyrics` -> `lyrics.value`, trimmed. Mirrors `client.ts:319-321`: whitespace
/// only (or absent) becomes `None` rather than an empty string, so callers can treat
/// "no lyrics" as one case.
pub fn lyrics_legacy(sr: &Value) -> Result<LyricsResult, SubsonicError> {
    let unsynced = sr
        .get("lyrics")
        .and_then(|l| l.get("value"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    Ok(LyricsResult {
        synced: None,
        unsynced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_status_returns_payload() {
        let ok =
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{"album":[]}}}"#;
        let payload = envelope(ok).unwrap();
        assert!(payload.get("albumList2").is_some());
    }

    #[test]
    fn failed_status_carries_error_message() {
        let err = r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong username or password"}}}"#;
        let e = envelope(err).unwrap_err();
        assert_eq!(
            e,
            SubsonicError::Subsonic("Wrong username or password".into())
        );
        assert_eq!(e.to_string(), "Wrong username or password");
    }

    #[test]
    fn failed_status_without_message_falls_back() {
        let err = r#"{"subsonic-response":{"status":"failed"}}"#;
        let e = envelope(err).unwrap_err();
        assert_eq!(e.to_string(), "Subsonic error");
    }

    #[test]
    fn missing_envelope_is_bad_response() {
        let junk = r#"{"nope":true}"#;
        let e = envelope(junk).unwrap_err();
        assert_eq!(e, SubsonicError::BadResponse);
        assert_eq!(e.to_string(), "Bad response");
    }

    #[test]
    fn invalid_json_is_bad_response() {
        let e = envelope("not json").unwrap_err();
        assert_eq!(e, SubsonicError::BadResponse);
    }

    #[test]
    fn http_error_formats_with_status_code() {
        assert_eq!(SubsonicError::Http(500).to_string(), "HTTP 500");
    }
}
