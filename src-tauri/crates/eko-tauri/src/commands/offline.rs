//! `#[tauri::command]` wrappers over `eko_core::pro::offline`.
//!
//! Each wrapper does exactly three things and nothing else:
//!
//! 1. Runs the Pro entitlement gate, which needs the `AppHandle` (`compute_status`)
//!    and therefore cannot live in `eko-core`. The gate is verbatim — same position,
//!    same error string — as before the offline cache moved out of `eko-tauri`.
//! 2. Unwraps `tauri::State<OfflineCache>`.
//! 3. Re-attaches Tauri to the progress callback by emitting `offline-progress`
//!    with the `CacheProgress` payload from inside the closure. `eko-core` calls the
//!    callback at exactly the points it previously emitted, so the frontend's
//!    `listen<CacheProgress>("offline-progress", …)` in `src/pro/useOfflineStore.ts`
//!    sees an unchanged stream of events.
//!
//! Command names, parameter names and return types are the IPC contract with the
//! TypeScript frontend (`src/pro/useOfflineStore.ts`). They are unchanged.

use std::sync::Arc;

use eko_core::pro::offline::{CacheEntry, CacheProgress, CacheStats, OfflineCache};
use tauri::Emitter;

/// Download and cache a single track by its Subsonic ID.
///
/// `download_url` must be the track's pre-signed **`downloadUrl`** — the complete,
/// authenticated Subsonic **`download`** endpoint URL, which returns the ORIGINAL file
/// bytes and is never transcoded. The frontend does not build it: signing needs the server
/// password, which no longer leaves Rust. It is minted by
/// `eko_net::urls::download_url`, travels on the payload as `SubSong.downloadUrl`, and
/// reaches the frontend as `Track.downloadUrl` — read it straight off the track.
///
/// **It must never be `streamSrcUrl`.** That is the `stream` endpoint, which the server
/// may transcode. Substituting it would fill the offline cache with transcoded audio while
/// every track still downloaded, played and sounded correct — the whole point of this cache
/// is bit-perfect storage, and no test catches the difference.
///
/// `codec` is a short label like `"flac"` or `"mp3"`.
///
/// Progress events (`offline-progress`) are emitted to the main window.
/// Requires Pro — the frontend also gates the action, but the command verifies
/// independently to prevent IPC bypass.
#[tauri::command]
pub fn cache_track(
    app: tauri::AppHandle,
    track_id: String,
    download_url: String,
    codec: String,
    cache: tauri::State<OfflineCache>,
) -> Result<CacheEntry, String> {
    use eko_core::pro::license::{compute_status, Tier};

    // Pro gate — independently verified server-side.
    let status = compute_status(crate::commands::license::config_dir(&app).as_deref());
    if status.tier == Tier::Free {
        return Err("EKO Pro is required to cache tracks for offline playback.".to_string());
    }

    let emitter = app.clone();
    cache.cache_track(track_id, download_url, codec, &move |p: CacheProgress| {
        let _ = emitter.emit("offline-progress", p);
    })
}

/// Cache every track in an album (sequential, background thread). Returns immediately;
/// progress events arrive via `offline-progress`.
#[tauri::command]
pub fn cache_album(
    app: tauri::AppHandle,
    track_ids: Vec<String>,
    download_urls: Vec<String>,
    codecs: Vec<String>,
    cache: tauri::State<OfflineCache>,
) -> Result<(), String> {
    use eko_core::pro::license::{compute_status, Tier};
    // Pro gate.
    let status = compute_status(crate::commands::license::config_dir(&app).as_deref());
    if status.tier == Tier::Free {
        return Err("EKO Pro required.".to_string());
    }

    let emitter = app.clone();
    cache.cache_album(
        track_ids,
        download_urls,
        codecs,
        Arc::new(move |p: CacheProgress| {
            let _ = emitter.emit("offline-progress", p);
        }),
    )
}

/// Remove a cached track's offline copy.
#[tauri::command]
pub fn remove_offline(track_id: String, cache: tauri::State<OfflineCache>) -> Result<(), String> {
    cache.remove(&track_id)
}

/// List all cached entries.
#[tauri::command]
pub fn offline_list(cache: tauri::State<OfflineCache>) -> Vec<CacheEntry> {
    cache.list()
}

/// Return aggregate cache statistics.
#[tauri::command]
pub fn offline_stats(cache: tauri::State<OfflineCache>) -> CacheStats {
    cache.stats()
}

/// Set the cache size cap in bytes (0 = default 5 GiB).
#[tauri::command]
pub fn set_cache_limit(bytes: u64, cache: tauri::State<OfflineCache>) -> Result<(), String> {
    cache.set_cap(bytes)
}

// There was a `set_cache_bitrate` command here, wrapping
// `OfflineCache::set_transcode_mode`. It is gone, along with the UI toggle that was its
// only caller: nothing in `eko-core` ever acted on the flag (`CacheEntry.transcoded` is
// hardcoded `false` at every construction site, and no `maxBitRate` is sent anywhere), so
// the command persisted a claim the cache never honoured. `set_transcode_mode` itself is
// kept on `OfflineCache` — see the note there. Do not re-register a command for it without
// first building the seal downgrade that a transcoded cached track would require.
