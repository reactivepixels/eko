//! The 15 OpenSubsonic endpoints EKO uses, ported from `src/subsonic/client.ts`.
//!
//! This module is deliberately thin. Every method does the same four things — build a
//! URL via [`crate::urls`], issue a blocking GET, map a non-2xx status to
//! [`SubsonicError::Http`], hand the body to a pure [`crate::parse`] function — and then
//! decorates the result with pre-signed URLs. All the shape-handling logic lives in
//! `parse`, where it is testable against fixtures with no server involved.
//!
//! ## Defaults are behaviour
//!
//! TypeScript's default parameters vanish in a Rust port, so each one is a named
//! constant below with the `client.ts` line it came from. `Option<u32>` arguments exist
//! precisely so a caller can say "whatever the TypeScript would have used" instead of
//! restating a magic number. Changing any of these constants changes what users see
//! without changing a single call site.
//!
//! ## Blocking, on purpose
//!
//! [`reqwest::blocking`] keeps this crate usable from a plain `fn` — a future terminal
//! client, a test — with no runtime to thread through. Tauri callers must therefore not
//! invoke these methods directly on an async executor thread; wrap them in
//! `tauri::async_runtime::spawn_blocking` (Task 4's job).

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::parse::{self, SubsonicError};
use crate::types::{
    AlbumDetail, AlbumListType, LyricsResult, PlaylistDetail, SearchResult, SubAlbum,
    SubArtistInfo, SubGenre, SubPlaylist, SubSong, SubStarred,
};
use crate::{auth, urls, Config};

/// Albums per `getAlbumList2` page. From `client.ts:97` and `client.ts:178`.
///
/// 500 is the Subsonic spec's maximum, which is why the frontend has to page-walk at all
/// for libraries larger than that (`src/subsonic/useSubsonic.ts:99`).
pub const DEFAULT_ALBUM_LIST_SIZE: u32 = 500;
/// Starting offset for `getAlbumList2`. From `client.ts:97` and `client.ts:179`.
pub const DEFAULT_ALBUM_LIST_OFFSET: u32 = 0;
/// Songs per [`Client::get_songs`] page. Mirrors [`DEFAULT_ALBUM_LIST_SIZE`]: 500 is the
/// Subsonic spec's per-page maximum, so a full track index is always a page walk.
pub const DEFAULT_SONG_LIST_SIZE: u32 = 500;
/// Starting offset for [`Client::get_songs`].
pub const DEFAULT_SONG_LIST_OFFSET: u32 = 0;
/// `search3`'s `songCount`.
///
/// Raised from the TypeScript's 50, which was a low ceiling for the one thing server search
/// is uniquely good at: finding a specific song in a library too big to browse. 50 hits is
/// under one screenful of scrolling on a large library, and the payload is small either way.
pub const SEARCH_SONG_COUNT: u32 = 200;
/// `search3`'s `albumCount`. From `client.ts:114`.
pub const SEARCH_ALBUM_COUNT: u32 = 30;
/// `search3`'s `artistCount`. From `client.ts:114`.
///
/// Zero is not an oversight: EKO's search UI has no artist section, so asking for
/// artists would cost the server work for results nothing renders.
pub const SEARCH_ARTIST_COUNT: u32 = 0;
/// `getRandomSongs`'s `size`. From `client.ts:138`.
pub const DEFAULT_RANDOM_SONGS_SIZE: u32 = 50;
/// `getSimilarSongs2`'s `count`. From `client.ts:193`.
pub const DEFAULT_SIMILAR_SONGS_COUNT: u32 = 50;

/// A configured OpenSubsonic client bound to one server.
///
/// Holds the credentials and a reusable [`reqwest::blocking::Client`] (connection pool
/// included). Cheap to clone conceptually but not [`Clone`] — construct one per server
/// and share it behind a lock or `OnceCell`.
pub struct Client {
    cfg: Config,
    http: reqwest::blocking::Client,
}

/// Turn a `reqwest` error into a message that is safe to render.
///
/// **`without_url` is the load-bearing call, not a tidy-up.** `reqwest`'s `Display`
/// appends `" for url ({url})"`, and for this crate that URL is one
/// [`urls::api_url`] has already signed — it carries `u` (username), `t` (the md5 of
/// password + salt) and `s` (the salt). Under Subsonic's auth scheme that triple is a
/// **replayable credential**: anyone holding it can issue requests as the user until the
/// password changes. `SubsonicError::Request`'s message reaches the webview through the
/// command layer and is rendered verbatim by `ConnectPanel.tsx`, so leaking it there puts
/// a live credential in the DOM.
///
/// The frontend's `friendlyConnectError` regex is *not* a substitute. It matches
/// `"sending request"` and a handful of transport phrases, but `reqwest` also produces
/// `"error following redirect for url (…)"`, `"request or response body error for url
/// (…)"` and `"error decoding response body for url (…)"` — none of which it catches, all
/// of which fall through to `return raw`.
///
/// Stripping the URL costs nothing diagnostically: the failing server is already known to
/// every caller (it is the one they just connected to), and the base URL travels
/// separately in `friendlyConnectError`'s own message.
fn safe_message(e: reqwest::Error) -> String {
    e.without_url().to_string()
}

impl Client {
    /// Build a client for `cfg`, with a fresh HTTP connection pool.
    ///
    /// No timeout is configured, matching the TypeScript's bare `fetch`. Adding one
    /// would be a behaviour change: large `getAlbumList2` pages on a cold Navidrome
    /// scan can legitimately take a long time.
    pub fn new(cfg: Config) -> Result<Self, SubsonicError> {
        let http = reqwest::blocking::Client::builder()
            .build()
            .map_err(|e| SubsonicError::Request(safe_message(e)))?;
        Ok(Self { cfg, http })
    }

    /// Build a client over a caller-supplied HTTP client — for tests, or to share one
    /// pool across several servers.
    pub fn with_http(cfg: Config, http: reqwest::blocking::Client) -> Self {
        Self { cfg, http }
    }

    /// The server this client is bound to.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// GET `{base}/rest/{method}` and unwrap the `subsonic-response` envelope.
    ///
    /// Mirrors `client.ts:76-90`, including the order of failure checks: transport
    /// error, then HTTP status, then envelope. A fresh salt is minted per call, so no
    /// two requests reuse an auth token.
    ///
    /// `url` is signed and must never reach an error message — every `reqwest` error here
    /// goes through [`safe_message`], which strips it. See that function for why.
    fn call(&self, method: &str, extra: &[(&str, &str)]) -> Result<Value, SubsonicError> {
        let url = urls::api_url(&self.cfg, method, extra, &auth::random_salt());
        let response = self
            .http
            .get(&url)
            .send()
            .map_err(|e| SubsonicError::Request(safe_message(e)))?;
        let status = response.status();
        if !status.is_success() {
            return Err(SubsonicError::Http(status.as_u16()));
        }
        let body = response
            .text()
            .map_err(|e| SubsonicError::Request(safe_message(e)))?;
        parse::envelope(&body)
    }

    fn decorate_song(&self, song: &mut SubSong) {
        urls::attach_song_urls(&self.cfg, song);
    }

    fn decorate_songs(&self, songs: &mut [SubSong]) {
        for song in songs {
            self.decorate_song(song);
        }
    }

    fn decorate_albums(&self, albums: &mut [SubAlbum]) {
        for album in albums {
            urls::attach_album_urls(&self.cfg, album);
        }
    }

    /// `ping` — verify the connection and credentials. Mirrors `client.ts:93-95`.
    ///
    /// The payload is discarded; a wrong password surfaces as
    /// [`SubsonicError::Subsonic`] because the envelope carries `status: "failed"` with
    /// HTTP 200.
    pub fn ping(&self) -> Result<(), SubsonicError> {
        self.call("ping", &[])?;
        Ok(())
    }

    /// `getAlbumList2` with `type=alphabeticalByArtist` — the main library listing.
    /// Mirrors `client.ts:97-105`. `None` for either argument uses
    /// [`DEFAULT_ALBUM_LIST_SIZE`] / [`DEFAULT_ALBUM_LIST_OFFSET`].
    ///
    /// A page shorter than `size` does **not** mean the library ends there; see
    /// [`crate::parse`]'s module docs.
    pub fn get_albums(
        &self,
        size: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<SubAlbum>, SubsonicError> {
        self.get_album_list2(AlbumListType::AlphabeticalByArtist, size, offset, &[])
    }

    /// `getAlbum` — one album plus its tracks. Mirrors `client.ts:107-111`.
    pub fn get_album(&self, id: &str) -> Result<AlbumDetail, SubsonicError> {
        let payload = self.call("getAlbum", &[("id", id)])?;
        let mut detail = parse::album(&payload)?;
        urls::attach_album_urls(&self.cfg, &mut detail.album);
        self.decorate_songs(&mut detail.songs);
        Ok(detail)
    }

    /// `search3` — albums and songs matching `query`. Mirrors `client.ts:113-117`.
    ///
    /// Counts are fixed, not caller-tunable, exactly as in the TypeScript: see
    /// [`SEARCH_SONG_COUNT`], [`SEARCH_ALBUM_COUNT`], [`SEARCH_ARTIST_COUNT`].
    pub fn search(&self, query: &str) -> Result<SearchResult, SubsonicError> {
        let (song_count, album_count, artist_count) = (
            SEARCH_SONG_COUNT.to_string(),
            SEARCH_ALBUM_COUNT.to_string(),
            SEARCH_ARTIST_COUNT.to_string(),
        );
        let payload = self.call(
            "search3",
            &[
                ("query", query),
                ("songCount", &song_count),
                ("albumCount", &album_count),
                ("artistCount", &artist_count),
            ],
        )?;
        let mut result = parse::search_result(&payload)?;
        self.decorate_albums(&mut result.albums);
        self.decorate_songs(&mut result.songs);
        Ok(result)
    }

    /// Every song on the server, one page at a time — the full track index.
    ///
    /// There is no "list all songs" endpoint in the Subsonic API, so this leans on the one
    /// documented quirk that gets there: `search3` with an **empty** `query`. Navidrome
    /// answers that with the whole song set (verified against 0.63.2), paged by
    /// `songOffset`/`songCount`. `None` for either argument uses
    /// [`DEFAULT_SONG_LIST_SIZE`] / [`DEFAULT_SONG_LIST_OFFSET`].
    ///
    /// Deliberately NOT a call to [`Self::search`]: that method's counts are fixed and it
    /// also fetches albums, which this caller throws away.
    ///
    /// Two properties the caller depends on:
    ///
    /// * A server that reads the empty query literally returns **no** songs rather than an
    ///   error, so the caller needs no capability probe — an empty walk is the answer.
    /// * A page shorter than `size` does **not** mean the library ends there; see
    ///   [`crate::parse`]'s module docs. Termination is the caller's business.
    pub fn get_songs(
        &self,
        size: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<SubSong>, SubsonicError> {
        let (song_count, song_offset) = (
            size.unwrap_or(DEFAULT_SONG_LIST_SIZE).to_string(),
            offset.unwrap_or(DEFAULT_SONG_LIST_OFFSET).to_string(),
        );
        let payload = self.call(
            "search3",
            &[
                ("query", ""),
                ("songCount", &song_count),
                ("songOffset", &song_offset),
                // This call feeds the track list only; album/artist hits would be waste.
                ("albumCount", "0"),
                ("artistCount", "0"),
            ],
        )?;
        let mut result = parse::search_result(&payload)?;
        self.decorate_songs(&mut result.songs);
        Ok(result.songs)
    }

    /// `getPlaylists` — the server's playlists. Mirrors `client.ts:126-130`.
    ///
    /// Playlists carry no pre-signed URLs: nothing in the app renders playlist cover art
    /// from this listing, and minting art URLs for every playlist would be waste.
    pub fn get_playlists(&self) -> Result<Vec<SubPlaylist>, SubsonicError> {
        let payload = self.call("getPlaylists", &[])?;
        parse::playlists(&payload)
    }

    /// `getPlaylist` — one playlist's name and tracks. Mirrors `client.ts:132-136`.
    pub fn get_playlist(&self, id: &str) -> Result<PlaylistDetail, SubsonicError> {
        let payload = self.call("getPlaylist", &[("id", id)])?;
        let mut detail = parse::playlist(&payload)?;
        self.decorate_songs(&mut detail.songs);
        Ok(detail)
    }

    /// `getRandomSongs` — shuffle fodder, optionally restricted to one genre. Mirrors
    /// `client.ts:138-144`. `None` for `size` uses [`DEFAULT_RANDOM_SONGS_SIZE`].
    ///
    /// `genre` is omitted from the query entirely when `None` *or* empty — the
    /// TypeScript's `if (genre)` treats `""` as absent, and sending `genre=` would ask
    /// the server for songs whose genre is the empty string.
    pub fn get_random_songs(
        &self,
        size: Option<u32>,
        genre: Option<&str>,
    ) -> Result<Vec<SubSong>, SubsonicError> {
        let size = size.unwrap_or(DEFAULT_RANDOM_SONGS_SIZE).to_string();
        let mut extra = vec![("size", size.as_str())];
        if let Some(genre) = genre.filter(|g| !g.is_empty()) {
            extra.push(("genre", genre));
        }
        let payload = self.call("getRandomSongs", &extra)?;
        let mut songs = parse::random_songs(&payload)?;
        self.decorate_songs(&mut songs);
        Ok(songs)
    }

    /// `getGenres` — genre names and song counts. Mirrors `client.ts:153-162`.
    pub fn get_genres(&self) -> Result<Vec<SubGenre>, SubsonicError> {
        let payload = self.call("getGenres", &[])?;
        parse::genres(&payload)
    }

    /// `getAlbumList2` with an explicit `type` — the general form, used by smart
    /// playlist rules. Mirrors `client.ts:176-190`.
    ///
    /// `extra` is applied last and overrides by key, matching the TypeScript's spread;
    /// `byGenre` and `byYear` need it to pass `genre` / `fromYear` + `toYear`.
    pub fn get_album_list2(
        &self,
        list_type: AlbumListType,
        size: Option<u32>,
        offset: Option<u32>,
        extra: &[(&str, &str)],
    ) -> Result<Vec<SubAlbum>, SubsonicError> {
        let size = size.unwrap_or(DEFAULT_ALBUM_LIST_SIZE).to_string();
        let offset = offset.unwrap_or(DEFAULT_ALBUM_LIST_OFFSET).to_string();
        let mut params = vec![
            ("type", list_type.as_str()),
            ("size", size.as_str()),
            ("offset", offset.as_str()),
        ];
        params.extend_from_slice(extra);
        let payload = self.call("getAlbumList2", &params)?;
        let mut albums = parse::album_list(&payload)?;
        self.decorate_albums(&mut albums);
        Ok(albums)
    }

    /// `getSimilarSongs2` — songs similar to `id`, for radio-style continuation.
    /// Mirrors `client.ts:193-197`. `None` for `count` uses
    /// [`DEFAULT_SIMILAR_SONGS_COUNT`].
    pub fn get_similar_songs2(
        &self,
        id: &str,
        count: Option<u32>,
    ) -> Result<Vec<SubSong>, SubsonicError> {
        let count = count.unwrap_or(DEFAULT_SIMILAR_SONGS_COUNT).to_string();
        let payload = self.call("getSimilarSongs2", &[("id", id), ("count", &count)])?;
        let mut songs = parse::similar_songs(&payload)?;
        self.decorate_songs(&mut songs);
        Ok(songs)
    }

    /// `getArtistInfo2` — biography and similar artists. Mirrors `client.ts:210-213`.
    pub fn get_artist_info2(&self, id: &str) -> Result<SubArtistInfo, SubsonicError> {
        let payload = self.call("getArtistInfo2", &[("id", id)])?;
        parse::artist_info(&payload)
    }

    /// `getStarred2` — starred songs and albums. Mirrors `client.ts:220-223`.
    pub fn get_starred2(&self) -> Result<SubStarred, SubsonicError> {
        let payload = self.call("getStarred2", &[])?;
        let mut starred = parse::starred(&payload)?;
        if let Some(songs) = starred.song.as_mut() {
            self.decorate_songs(songs);
        }
        if let Some(albums) = starred.album.as_mut() {
            self.decorate_albums(albums);
        }
        Ok(starred)
    }

    /// `scrobble` — `submission = false` is "now playing" (sent at track start),
    /// `true` is a permanent scrobble (sent at the play threshold). `time` is the
    /// current Unix time in **milliseconds**, as `Date.now()` produces.
    ///
    /// **Every failure is swallowed and this returns `Ok(())` regardless** — a 500, a
    /// wrong password, an unplugged network cable. That is the whole point of
    /// `client.ts:259-269`: scrobbling is best-effort telemetry, and letting it
    /// propagate would let a flaky server interrupt playback. If you ever want the
    /// error, add a separate method; do not change this one's contract.
    pub fn scrobble(&self, id: &str, submission: bool) -> Result<(), SubsonicError> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
            .to_string();
        let _ignored = self.call(
            "scrobble",
            &[
                ("id", id),
                ("submission", if submission { "true" } else { "false" }),
                ("time", &time),
            ],
        );
        Ok(())
    }

    /// OpenSubsonic `getLyricsBySongId` — synced lyrics with per-line millisecond
    /// offsets, available on Navidrome >= 0.52. Mirrors `client.ts:282-306`.
    ///
    /// Note the query param is `id`, not `songId`, despite the method name — that is
    /// what `client.ts:284` sends and what the OpenSubsonic spec defines.
    ///
    /// On **any** failure — including servers that don't implement the endpoint at all
    /// and answer with `status: "failed"` — this falls back to
    /// [`Client::get_lyrics_legacy`] with no artist or title, exactly as the TypeScript
    /// does. That fallback almost always yields nothing; it exists so the caller gets
    /// one uniform "no lyrics" answer instead of an error to special-case.
    pub fn get_lyrics_by_song_id(&self, song_id: &str) -> Result<LyricsResult, SubsonicError> {
        match self
            .call("getLyricsBySongId", &[("id", song_id)])
            .and_then(|payload| parse::lyrics_by_song_id(&payload))
        {
            Ok(result) => Ok(result),
            Err(_) => self.get_lyrics_legacy(None, None),
        }
    }

    /// Legacy `getLyrics` — plain, unsynced text keyed by artist + title. Mirrors
    /// `client.ts:310-325`.
    ///
    /// `artist` and `title` are each omitted from the query when `None` or empty
    /// (the TypeScript's `if (artist)`). Like its TypeScript original this never fails:
    /// errors become [`LyricsResult::EMPTY`]. The `Result` is kept only so the signature
    /// matches its siblings.
    pub fn get_lyrics_legacy(
        &self,
        artist: Option<&str>,
        title: Option<&str>,
    ) -> Result<LyricsResult, SubsonicError> {
        let mut extra: Vec<(&str, &str)> = Vec::new();
        if let Some(artist) = artist.filter(|a| !a.is_empty()) {
            extra.push(("artist", artist));
        }
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            extra.push(("title", title));
        }
        match self
            .call("getLyrics", &extra)
            .and_then(|payload| parse::lyrics_legacy(&payload))
        {
            Ok(result) => Ok(result),
            Err(_) => Ok(LyricsResult::EMPTY),
        }
    }
}
