//! `#[tauri::command]` wrappers over [`eko_net::Client`] — the OpenSubsonic surface.
//!
//! **On the `client.ts:NNN` citations below:** `src/subsonic/client.ts` no longer exists at
//! HEAD — `eko-net` replaced it, and it was deleted once the last consumer migrated. The
//! line numbers refer to the pre-deletion file, readable at
//! `git show 1393cf3:src/subsonic/client.ts`. They are kept because they record which
//! behaviours here are deliberate parity choices rather than free ones.
//!
//! Every command here is a **pure forward**. No defaults, no branching on argument
//! values, no reshaping of payloads: `eko-net` owns all of that (its `Option<u32>`
//! arguments exist precisely so a `None` arriving from the frontend can mean "whatever
//! the TypeScript's default parameter would have used"). If you find yourself wanting
//! an `unwrap_or` in this file, the default belongs in `eko-net` instead. There is one
//! surviving exception, flagged at [`subsonic_get_album_list2`].
//!
//! Two things this layer *does* own, because `eko-net` cannot see them:
//!
//! 1. **The runtime handover.** [`eko_net::Client`] wraps a
//!    [`reqwest::blocking::Client`], which drives its own tokio runtime on a dedicated
//!    thread. Both *constructing* and *calling* one from a thread already inside a tokio
//!    runtime — which is exactly what a `#[tauri::command]` is — panics (`Cannot drop a
//!    runtime in a context where blocking is not allowed`). Nothing in the type system
//!    objects, so both paths are routed through
//!    [`tauri::async_runtime::spawn_blocking`]: construction in [`subsonic_set_config`],
//!    every request via [`run`]. **No command in this file may touch the client outside
//!    one of those closures.**
//!
//!    Both halves are pinned by tests, because both regress silently otherwise:
//!    `run_never_touches_the_client_on_the_async_executor` (below) fails if [`run`]
//!    loses its `spawn_blocking`, and `tests/blocking_handover.rs` carries the two
//!    negative controls proving the hazard is real rather than assumed.
//!
//! 2. **Error transport.** Tauri requires a `Serialize` error type, and the frontend's
//!    `catch` blocks already expect a bare string (`client.ts` throws `Error(message)`).
//!    `SubsonicError`'s `Display` is that message verbatim — this is an encoding, not a
//!    remapping; no wrapper here invents, reworded or swallows an error.
//!
//! Command names and parameter names are the IPC contract with the TypeScript frontend.
//! Renaming either silently breaks the app at runtime.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use eko_net::types::{
    AlbumDetail, AlbumListType, LyricsResult, PlaylistDetail, SearchResult, SubAlbum,
    SubArtistInfo, SubGenre, SubPlaylist, SubSong, SubStarred,
};
use eko_net::{Client, Config};

/// The configured OpenSubsonic client, or `None` before connect / after disconnect.
///
/// This is the Rust-side counterpart of `client.ts`'s module-level `let cfg` — the
/// single place the server password lives now. It is `.manage(…)`d at startup
/// alongside `StreamOrigin` and the Pro `OfflineCache`.
///
/// The client sits behind an [`Arc`] rather than being stored inline so a command can
/// clone a handle out, **drop the lock**, and only then make the (blocking, possibly
/// slow) network call. Holding the mutex across the request would serialise every
/// Subsonic call in the app behind whichever one is currently in flight — a cold
/// `getAlbumList2` page walk would stall cover art, search and scrobbles.
#[derive(Default)]
pub struct SubsonicClient(Mutex<Option<Arc<Client>>>);

impl SubsonicClient {
    /// A handle to the configured client, or the "not configured" error — the Rust
    /// equivalent of `client.ts:53`'s `throw new Error("Subsonic not configured")`.
    fn handle(&self) -> Result<Arc<Client>, String> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "Subsonic not configured".to_string())
    }
}

/// Run one `eko-net` call on a blocking thread and encode its error as a string.
///
/// This is the single choke point for requirement (1) in the module docs: the closure
/// receives the client, and it only ever executes inside `spawn_blocking`, off the
/// async executor. The `Send + 'static` bounds are what make that sound — they are the
/// compiler enforcing that nothing borrowed from the command's async context (least of
/// all a `tauri::State` or a `MutexGuard`) can leak into the blocking thread.
///
/// A panic inside the closure surfaces as the join error's message rather than
/// unwinding the command; `eko-net` has no panicking paths, so this is belt-and-braces.
async fn run<T, F>(state: &SubsonicClient, call: F) -> Result<T, String>
where
    F: FnOnce(&Client) -> Result<T, eko_net::SubsonicError> + Send + 'static,
    T: Send + 'static,
{
    // The lock is taken and released here, before the await — never held across it.
    let client = state.handle()?;
    tauri::async_runtime::spawn_blocking(move || call(&client))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Set (or, with `config: null`, clear) the OpenSubsonic connection.
///
/// Mirrors `setConfig(c)` / `setConfig(null)` at `client.ts:46-48`. Clearing drops the
/// client, and with it the only in-memory copy of the password.
///
/// As belt-and-braces this also keeps the `stream://` proxy's SSRF allowlist in step
/// with the configured server: connecting registers the base URL's origin, clearing
/// revokes it. The separate `set_stream_origin` command is untouched and still
/// authoritative for the frontend — both write the same value, so calling either, both,
/// or neither in any order converges. Nothing here *widens* the allowlist: a `None`
/// config always clears it.
///
/// [`Client::new`] is itself run on a blocking thread: building a
/// `reqwest::blocking::Client` stands up a runtime, which is not safe to do from the
/// async executor.
#[tauri::command]
pub async fn subsonic_set_config(
    config: Option<Config>,
    state: tauri::State<'_, SubsonicClient>,
    origin: tauri::State<'_, crate::StreamOrigin>,
) -> Result<(), String> {
    let base_url = config.as_ref().map(|c| c.base_url.clone());

    let client = match config {
        Some(cfg) => Some(Arc::new(
            tauri::async_runtime::spawn_blocking(move || Client::new(cfg))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?,
        )),
        None => None,
    };

    *state.0.lock().unwrap() = client;
    *origin.0.lock().unwrap() = base_url.filter(|u| !u.is_empty());
    Ok(())
}

/// `ping` — verify the connection and credentials.
#[tauri::command]
pub async fn subsonic_ping(state: tauri::State<'_, SubsonicClient>) -> Result<(), String> {
    run(&state, |c| c.ping()).await
}

/// `getAlbumList2` with `type=alphabeticalByArtist` — the main library listing.
/// `null` for either argument takes `eko-net`'s default (500 / 0).
#[tauri::command]
pub async fn subsonic_get_albums(
    size: Option<u32>,
    offset: Option<u32>,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<Vec<SubAlbum>, String> {
    run(&state, move |c| c.get_albums(size, offset)).await
}

/// Every song on the server, one page at a time — the full track index, via `search3`
/// with an empty query. `null` for either argument takes `eko-net`'s default (500 / 0).
#[tauri::command]
pub async fn subsonic_get_songs(
    size: Option<u32>,
    offset: Option<u32>,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<Vec<SubSong>, String> {
    run(&state, move |c| c.get_songs(size, offset)).await
}

/// `getAlbum` — one album plus its tracks.
#[tauri::command]
pub async fn subsonic_get_album(
    id: String,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<AlbumDetail, String> {
    run(&state, move |c| c.get_album(&id)).await
}

/// `search3` — albums and songs matching `query`. The result counts are fixed in
/// `eko-net`, exactly as they are in the TypeScript.
#[tauri::command]
pub async fn subsonic_search(
    query: String,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<SearchResult, String> {
    run(&state, move |c| c.search(&query)).await
}

/// `getPlaylists` — the server's playlists.
#[tauri::command]
pub async fn subsonic_get_playlists(
    state: tauri::State<'_, SubsonicClient>,
) -> Result<Vec<SubPlaylist>, String> {
    run(&state, |c| c.get_playlists()).await
}

/// `getPlaylist` — one playlist's name and tracks.
#[tauri::command]
pub async fn subsonic_get_playlist(
    id: String,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<PlaylistDetail, String> {
    run(&state, move |c| c.get_playlist(&id)).await
}

/// `getRandomSongs` — shuffle fodder, optionally restricted to one genre. `null` for
/// `size` takes `eko-net`'s default (50); a `null` or empty `genre` is omitted from the
/// query entirely, which is `eko-net`'s call, not this wrapper's.
#[tauri::command]
pub async fn subsonic_get_random_songs(
    size: Option<u32>,
    genre: Option<String>,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<Vec<SubSong>, String> {
    run(&state, move |c| c.get_random_songs(size, genre.as_deref())).await
}

/// `getGenres` — genre names and song counts.
#[tauri::command]
pub async fn subsonic_get_genres(
    state: tauri::State<'_, SubsonicClient>,
) -> Result<Vec<SubGenre>, String> {
    run(&state, |c| c.get_genres()).await
}

/// `getAlbumList2` with an explicit type — the general form behind smart playlist rules.
///
/// The parameter is `list_type`, not `type`: `type` is a Rust keyword and cannot name a
/// command argument. `extra` is the TypeScript's `extra: Record<string, string>`
/// (`client.ts:180`) — a map, so its keys are unique; `eko-net` applies it last so it
/// overrides the base params by key, matching the TypeScript's object spread.
///
/// **Known shim-rule violation, deliberately deferred.** The `extra.unwrap_or_default()`
/// below is a default living in a wrapper, which this module's own rules forbid. It is
/// semantically inert — `eko-net` takes `&[(&str, &str)]`, so a `None` and an empty map
/// are indistinguishable by the time they reach it, and there is no behaviour here to
/// drift from. Removing it properly means widening `eko_net::Client::get_album_list2` to
/// accept an `Option`, which is an `eko-net` change, not a shim change.
#[tauri::command]
pub async fn subsonic_get_album_list2(
    list_type: AlbumListType,
    size: Option<u32>,
    offset: Option<u32>,
    extra: Option<BTreeMap<String, String>>,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<Vec<SubAlbum>, String> {
    run(&state, move |c| {
        let extra = extra.unwrap_or_default();
        let extra: Vec<(&str, &str)> = extra
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        c.get_album_list2(list_type, size, offset, &extra)
    })
    .await
}

/// `getSimilarSongs2` — songs similar to `id`, for radio-style continuation. `null` for
/// `count` takes `eko-net`'s default (50).
#[tauri::command]
pub async fn subsonic_get_similar_songs2(
    id: String,
    count: Option<u32>,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<Vec<SubSong>, String> {
    run(&state, move |c| c.get_similar_songs2(&id, count)).await
}

/// `getArtistInfo2` — biography and similar artists.
#[tauri::command]
pub async fn subsonic_get_artist_info2(
    id: String,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<SubArtistInfo, String> {
    run(&state, move |c| c.get_artist_info2(&id)).await
}

/// `getStarred2` — starred songs and albums.
#[tauri::command]
pub async fn subsonic_get_starred2(
    state: tauri::State<'_, SubsonicClient>,
) -> Result<SubStarred, String> {
    run(&state, |c| c.get_starred2()).await
}

/// `scrobble` — `submission: false` is "now playing", `true` is a permanent scrobble.
///
/// This resolves `Ok` even when the server rejects it: swallowing the failure is
/// `eko-net`'s documented contract (and `client.ts:259-269`'s), so that a flaky server
/// can never interrupt playback. Do not "improve" that here.
#[tauri::command]
pub async fn subsonic_scrobble(
    id: String,
    submission: bool,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<(), String> {
    run(&state, move |c| c.scrobble(&id, submission)).await
}

/// OpenSubsonic `getLyricsBySongId` — synced lyrics with per-line ms offsets, falling
/// back to the legacy endpoint (and then to an empty result) inside `eko-net`.
#[tauri::command]
pub async fn subsonic_get_lyrics_by_song_id(
    song_id: String,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<LyricsResult, String> {
    run(&state, move |c| c.get_lyrics_by_song_id(&song_id)).await
}

/// Legacy `getLyrics` — plain, unsynced text keyed by artist + title. Never fails:
/// an error becomes an empty result, in `eko-net`.
#[tauri::command]
pub async fn subsonic_get_lyrics_legacy(
    artist: Option<String>,
    title: Option<String>,
    state: tauri::State<'_, SubsonicClient>,
) -> Result<LyricsResult, String> {
    run(&state, move |c| {
        c.get_lyrics_legacy(artist.as_deref(), title.as_deref())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dead port: a working call fails fast with a transport error, so the only thing
    /// under test is whether the call happened in the right runtime context.
    fn cfg() -> Config {
        Config {
            base_url: "http://127.0.0.1:1".into(),
            username: "rod".into(),
            password: "hunter2".into(),
        }
    }

    /// The guard on [`run`] itself — the function every one of the fifteen endpoint
    /// wrappers goes through.
    ///
    /// `tests/blocking_handover.rs` proves the *hazard* exists (a blocking call on the
    /// async executor panics). This proves this crate's production code **avoids** it,
    /// which is a different claim and the one that actually regresses: delete the
    /// `spawn_blocking` from `run` and the `block_on` below panics, failing this test.
    /// Without it, that deletion left the whole suite green while the app died on the
    /// first `ping`.
    #[test]
    fn run_never_touches_the_client_on_the_async_executor() {
        let state = SubsonicClient::default();
        // Built here, on a plain test thread with no runtime — the same way
        // `subsonic_set_config` builds it on a blocking thread.
        *state.0.lock().unwrap() = Some(Arc::new(Client::new(cfg()).unwrap()));

        // A panic inside `block_on` propagates and fails the test; that is the assertion.
        let outcome = tauri::async_runtime::block_on(run(&state, |c| c.ping()));

        assert!(
            outcome.is_err(),
            "expected a transport error from the dead port, got {outcome:?}"
        );
    }

    /// The "not configured" path resolves cleanly rather than panicking or hanging —
    /// and never reaches `spawn_blocking` at all.
    #[test]
    fn run_reports_an_unconfigured_client() {
        let state = SubsonicClient::default();
        let outcome = tauri::async_runtime::block_on(run(&state, |c| c.ping()));
        assert_eq!(outcome, Err("Subsonic not configured".to_string()));
    }
}
