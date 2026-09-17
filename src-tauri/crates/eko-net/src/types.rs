//! Response payload types for the OpenSubsonic JSON API.
//!
//! Ported field-for-field from the TypeScript interfaces at `src/subsonic/client.ts`
//! (repo root) so JSON crossing the Tauri IPC boundary is byte-for-byte what the
//! existing React frontend already expects: same camelCase keys, same optionality
//! (an absent TS `foo?:` field stays absent here rather than becoming `null`).
//!
//! Each type also gains pre-signed URL fields that don't exist in the TypeScript —
//! `SubSong::stream_url` / `stream_src_url` / `download_url` / `cover_url` and
//! `SubAlbum::cover_url`. Task 3's endpoint layer populates them from [`crate::urls`]
//! so the frontend can use them directly with no async URL-minting step of its own.
//! This module only defines the shape; it does not populate them.

use serde::{Deserialize, Serialize};

/// A single album, as returned by `getAlbumList2`, `getAlbum`, `search3`, etc.
/// Mirrors `client.ts:14-22`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAlbum {
    /// Load-bearing: without it there is nothing to fetch art for or drill into, so
    /// unlike the display fields below this stays **required** and a record whose `id` is
    /// missing *or* empty is rejected. [`crate::lenient::id`] accepts a JSON number as
    /// well as a string — some servers use integer ids — but nothing else.
    #[serde(deserialize_with = "crate::lenient::id")]
    pub id: String,
    /// Display-only, and `#[serde(default)]` for the reason given on [`SubSong::title`]:
    /// an untagged album must render blank, not fail the whole page.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
    /// Number-or-numeric-string; see [`crate::lenient`] for why the numeric fields are
    /// permissive on the way in and why a value that makes no sense reads as absent.
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub song_count: Option<u32>,
    /// Number-or-numeric-string. `"year": "1998"` and `"year": -1` both used to fail the
    /// entire page; see [`crate::lenient`].
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_art: Option<String>,

    /// Pre-signed, `stream://`-wrapped cover URL. Not part of the OpenSubsonic
    /// payload — minted by Task 3's endpoint layer via [`crate::urls::cover_art_url`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
}

/// OpenSubsonic ReplayGain info attached to a song: gains are dB, peaks are linear
/// (see the comment at `client.ts:37`). Present only on servers that support it.
/// Mirrors `client.ts:38-43`.
///
/// All four are number-or-numeric-string on the way in: `{"trackGain": "-7.2"}` is a real
/// shape in the wild and used to fail the whole page. See [`crate::lenient`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGain {
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub track_gain: Option<f64>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub album_gain: Option<f64>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub track_peak: Option<f64>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub album_peak: Option<f64>,
}

/// A single song/track, as returned by `getAlbum`, `getPlaylist`, `search3`,
/// `getRandomSongs`, `getSimilarSongs2`, `getStarred2`. Mirrors `client.ts:24-44`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubSong {
    /// The one genuinely required field. Streaming, downloading, scrobbling and art all
    /// key off it, and [`crate::urls::attach_song_urls`] would mint four garbage URLs
    /// from an unusable one — so a song whose `id` is absent, **empty**, or of any type
    /// other than a string or a number is rejected outright, and dropped from its page
    /// rather than failing it. A *numeric* id is accepted and stringified. All of this
    /// lives in [`crate::lenient::id`], the only place any `id` in this module is read;
    /// `""` used to slip through the plain `String` derive and was the one case that
    /// really did mint four URLs ending `&id=`.
    #[serde(deserialize_with = "crate::lenient::id")]
    pub id: String,

    /// Display-only, and deliberately **not** required.
    ///
    /// Sparse tagging is normal — compilations routinely ship tracks with no `artist`,
    /// rips with no `title`. Making these required would turn that into a fatal: `serde`
    /// rejects the *whole* `Vec<SubSong>` if one element is missing a field, so a single
    /// untagged track would fail the entire album page, search, shuffle or playlist.
    /// `#[serde(default)]` is what keeps one bad tag from taking down a page, and that
    /// part of the decision stands.
    ///
    /// **Correction to an earlier version of this comment.** It claimed `""` was
    /// "observationally identical to `null` at every call site — both falsy, both hit
    /// the `??`". That is wrong, and it mattered: `??` is *nullish* coalescing, so
    /// `"" ?? "EKO"` is `""`, not `"EKO"`. Falsiness is what `||` tests; `??` tests only
    /// `null`/`undefined`. Defaulting to `""` therefore does **not** reproduce the old
    /// TypeScript on its own — it silently defeats seven downstream placeholder
    /// fallbacks (`?? "EKO"`, `?? "Unknown"` on the macOS lock-screen card, `?? "—"`,
    /// `?? "track"`), each of which renders blank instead.
    ///
    /// What makes the defaulting safe is the *frontend* boundary, not this type:
    /// `useSubsonic.ts`'s `toTrack` normalises `""` back to `null` with `||` before a
    /// `Track` is ever built, and `src/subsonic/toTrack.test.ts` pins that. Nothing in
    /// the frontend compares these fields against `null`, so the two are interchangeable
    /// to every *reader* — which is the real reason this is sound. Do not remove that
    /// `||` on the strength of this comment; it is load-bearing.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    /// Seconds. Number-or-numeric-string, and a float truncates (`251.5` → `251`) rather
    /// than failing the page — see [`crate::lenient`]. The frontend has always treated
    /// this as an integer count of seconds; widening the field to `f64` would change the
    /// IPC wire type for a precision nobody reads.
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub duration: Option<u32>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub bit_rate: Option<u32>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub sampling_rate: Option<u32>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub track: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_art: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_gain: Option<ReplayGain>,

    /// Pre-signed, `stream://`-wrapped URL for webview `<audio>` playback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
    /// Pre-signed DIRECT (unwrapped) upstream URL, for the native Rust engine
    /// to fetch. Never routed through the `stream://` proxy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_src_url: Option<String>,
    /// Pre-signed DIRECT `download` URL — original bytes, no transcode. The
    /// Pro offline cache stores exactly these bytes so cached tracks stay
    /// bit-perfect; it must never substitute `stream_src_url` here, since
    /// that endpoint may transcode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    /// Pre-signed, `stream://`-wrapped cover URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// [`crate::urls::server_key`] of the server that listed this song. Carries no
    /// credential. Lets a play queue hold an id instead of a signed URL and still refuse
    /// to play it against a different server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
}

/// Mirrors `client.ts:119-125`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubPlaylist {
    #[serde(deserialize_with = "crate::lenient::id")]
    pub id: String,
    /// Display-only; defaulted for the reason given on [`SubSong::title`]. Note this is
    /// the *listing*'s name — `getPlaylist` substitutes the literal `"Playlist"` for a
    /// missing name instead (`client.ts:135`), which [`crate::parse::playlist`] does.
    #[serde(default)]
    pub name: String,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_u32",
        skip_serializing_if = "Option::is_none"
    )]
    pub song_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_art: Option<String>,
}

/// Genre list entry. Note the OpenSubsonic XML `name` field arrives as JSON
/// `value` — see the comment at `client.ts:148`. Mirrors `client.ts:147-151`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubGenre {
    pub value: String,
    pub song_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_count: Option<u32>,
}

/// `getAlbumList2` type filter, used for smart playlist rules too. A Rust
/// mirror of the TS string union at `client.ts:165-174`; `rename_all =
/// "camelCase"` turns each PascalCase variant into the identical wire string
/// (e.g. `AlphabeticalByArtist` -> `"alphabeticalByArtist"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlbumListType {
    Newest,
    Recent,
    Frequent,
    Random,
    Starred,
    AlphabeticalByArtist,
    AlphabeticalByName,
    ByGenre,
    ByYear,
}

impl AlbumListType {
    /// The exact wire string sent as the `type` query param, matching what
    /// `serde_json` would produce — exposed directly since callers need this
    /// as a plain query value, not JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            AlbumListType::Newest => "newest",
            AlbumListType::Recent => "recent",
            AlbumListType::Frequent => "frequent",
            AlbumListType::Random => "random",
            AlbumListType::Starred => "starred",
            AlbumListType::AlphabeticalByArtist => "alphabeticalByArtist",
            AlbumListType::AlphabeticalByName => "alphabeticalByName",
            AlbumListType::ByGenre => "byGenre",
            AlbumListType::ByYear => "byYear",
        }
    }
}

/// Mirrors `client.ts:200-203`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubSimilarArtist {
    #[serde(deserialize_with = "crate::lenient::id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// `getArtistInfo2` result: bio + similar artists. Mirrors `client.ts:204-208`.
///
/// [`Default`] is the `?? {}` of `client.ts:212` — a server with no `artistInfo2` in the
/// envelope yields all-absent fields, not an error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubArtistInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub biography: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fm_url: Option<String>,
    /// Element-by-element, dropping records that fail — this payload reaches `serde`
    /// through a whole-object `from_value`, so it cannot go through
    /// [`crate::parse`]'s collection reader and needs the same treatment here.
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_vec_skipping_bad",
        skip_serializing_if = "Option::is_none"
    )]
    pub similar_artist: Option<Vec<SubSimilarArtist>>,
}

/// `getStarred2` result. Mirrors `client.ts:216-219`.
///
/// [`Default`] is the `?? {}` of `client.ts:222`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubStarred {
    /// Element-by-element; see [`SubArtistInfo::similar_artist`].
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_vec_skipping_bad",
        skip_serializing_if = "Option::is_none"
    )]
    pub song: Option<Vec<SubSong>>,
    #[serde(
        default,
        deserialize_with = "crate::lenient::opt_vec_skipping_bad",
        skip_serializing_if = "Option::is_none"
    )]
    pub album: Option<Vec<SubAlbum>>,
}

/// One synced-lyric line: `start` is an offset in ms from track start, `value`
/// is the line text. Mirrors `client.ts:273-276`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncedLyricLine {
    pub start: u32,
    #[serde(default)]
    pub value: String,
}

/// `getLyricsBySongId` / `getLyricsLegacy` result. Both fields are `T | null`
/// in TypeScript (always present, value nullable) rather than optional
/// (`foo?:`), so — unlike every other field in this module — they are NOT
/// `skip_serializing_if`: `None` must serialise as JSON `null` to match
/// `client.ts:277-280` exactly, not disappear from the payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsResult {
    pub synced: Option<Vec<SyncedLyricLine>>,
    pub unsynced: Option<String>,
}

impl LyricsResult {
    /// "This track has no lyrics" — what `client.ts:301`, `304` and `323` all return.
    /// The lyrics endpoints never surface an error to the caller; they degrade to this.
    pub const EMPTY: Self = Self {
        synced: None,
        unsynced: None,
    };
}

/// `getAlbum`'s result: the album, plus its tracks lifted out of `album.song`. Mirrors
/// the anonymous `{ album, songs }` returned at `client.ts:110`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDetail {
    pub album: SubAlbum,
    pub songs: Vec<SubSong>,
}

/// `search3`'s result. Mirrors the anonymous `{ albums, songs }` at `client.ts:116`.
/// Note the plural key names: the frontend already destructures `{ albums, songs }`,
/// not the server's singular `album`/`song`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub albums: Vec<SubAlbum>,
    pub songs: Vec<SubSong>,
}

/// `getPlaylist`'s result. Mirrors the anonymous `{ name, songs }` at `client.ts:135`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDetail {
    pub name: String,
    pub songs: Vec<SubSong>,
}

/// MIME type to hand the decoder for a song. Mirrors `client.ts:327-339`.
///
/// The server's own `contentType` always wins — unless it is the empty string, which is
/// falsy in the TypeScript's `if (s.contentType)` and so must fall through here too.
/// Otherwise the file suffix is mapped case-insensitively, and anything unrecognised —
/// including a song with no suffix at all — falls back to `audio/mpeg`. That fallback is
/// a guess by design: it keeps playback attempts alive on servers with sparse metadata
/// rather than failing early.
pub fn mime_for_song(song: &SubSong) -> String {
    if let Some(content_type) = song.content_type.as_deref().filter(|c| !c.is_empty()) {
        return content_type.to_string();
    }
    let suffix = song.suffix.as_deref().unwrap_or("").to_ascii_lowercase();
    match suffix.as_str() {
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        _ => "audio/mpeg",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_list_type_serialises_to_the_exact_wire_strings() {
        let cases = [
            (AlbumListType::Newest, "newest"),
            (AlbumListType::Recent, "recent"),
            (AlbumListType::Frequent, "frequent"),
            (AlbumListType::Random, "random"),
            (AlbumListType::Starred, "starred"),
            (AlbumListType::AlphabeticalByArtist, "alphabeticalByArtist"),
            (AlbumListType::AlphabeticalByName, "alphabeticalByName"),
            (AlbumListType::ByGenre, "byGenre"),
            (AlbumListType::ByYear, "byYear"),
        ];
        for (variant, wire) in cases {
            assert_eq!(
                serde_json::to_string(&variant).unwrap(),
                format!("\"{wire}\"")
            );
            assert_eq!(variant.as_str(), wire);
        }
    }

    #[test]
    fn lyrics_result_nulls_survive_serialisation() {
        let both_null = LyricsResult {
            synced: None,
            unsynced: None,
        };
        let v = serde_json::to_value(&both_null).unwrap();
        assert_eq!(v["synced"], serde_json::Value::Null);
        assert_eq!(v["unsynced"], serde_json::Value::Null);
    }
}
