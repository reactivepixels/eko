//! URL minting for the OpenSubsonic API.
//!
//! Ported from the TypeScript client's `apiUrl` / `streamUrl` / `streamSrcUrl` /
//! `coverArtUrl` / `downloadUrl`. That file is gone; read the original at
//! `git show 1393cf3:src/subsonic/client.ts` (lines 68-74 and 230-253).
//!
//! Two of the four builders are **proxied** through the app's `stream://` scheme
//! (see `src-tauri/crates/eko-tauri/src/stream.rs`) so the webview loads audio/art CORS-clean without
//! downloading whole files first; two are **direct**, authenticated upstream URLs for
//! Rust-side consumers. Getting this backwards is the highest-stakes mistake in this
//! module — in particular `download_url` (original bytes, for the Pro offline cache)
//! must never be confused with `stream_src_url` (the native engine's stream, which may
//! be transcoded by the server), or the bit-perfect claim for cached tracks breaks
//! silently.
//!
//! | builder              | shape                                   | consumer              |
//! |-----------------------|------------------------------------------|-----------------------|
//! | `stream_url`          | proxied: `stream://localhost/?src=<enc>` | the webview           |
//! | `cover_art_url`       | proxied: `stream://localhost/?src=<enc>` | `<img src>`           |
//! | `stream_src_url`      | direct, authenticated upstream           | native Rust engine    |
//! | `cover_art_src_url`   | direct, authenticated upstream, sized    | non-webview consumers (e.g. `eko-cli`) |
//! | `download_url`        | direct, authenticated upstream, `download` (no transcode) | Pro offline cache |
//!
//! `cover_art_src_url` is the odd one out in that table: it is the *direct* sibling of
//! `cover_art_url`, added for consumers — like the terminal client — that have no
//! `stream://` protocol handler to resolve a proxied URL and so must hit Navidrome
//! directly. Pick by environment, not by habit: inside the Tauri webview, always
//! `cover_art_url`; anywhere else (a native process fetching bytes itself),
//! `cover_art_src_url`. See the doc comment on each for why the size handling differs.
//!
//! `salt` is an explicit parameter on every builder so they stay pure and testable;
//! production callers pass [`crate::auth::random_salt`].

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

use crate::types::{SubAlbum, SubSong};
use crate::{auth, Config};

/// Cover-art edge size in pixels to use when a proxied cover URL carries none.
///
/// This is the TypeScript's `size = 300` (`client.ts:241`), but it is no longer applied
/// when a URL is minted — see [`cover_art_url`] for why. The `stream://` proxy
/// (`src-tauri/src/stream.rs`) is what consumes it: when the outer URL carries no
/// `size`, the proxy sets this on the upstream before fetching. It lives here rather
/// than in `stream.rs` so the value stays next to the builder whose default it is.
pub const DEFAULT_COVER_SIZE: u32 = 300;

/// Characters JS's `encodeURIComponent` leaves unescaped (unreserved chars plus
/// `! * ' ( )`); everything else — including `: / ? & =` — gets percent-encoded.
/// This is what turns the literal `format=raw` inside an upstream URL into
/// `format%3Draw` once that whole URL is wrapped as the `stream://` proxy's `src`
/// query value.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

fn encode_uri_component(s: &str) -> String {
    utf8_percent_encode(s, COMPONENT).to_string()
}

/// Direct, authenticated upstream URL: `{base}/rest/{method}?{query}`. Mirrors
/// `client.ts:68-74`. `base_url`'s trailing slashes are stripped exactly once (via
/// [`Config::base_url_trimmed`]) before `/rest/` is appended, so a `base_url` that
/// already ends in `/` never produces a doubled slash. `extra` params are applied
/// after the six auth params, overriding any of them by key (mirrors `p.set(k,
/// extra[k])` in `client.ts:72`).
pub fn api_url(cfg: &Config, method: &str, extra: &[(&str, &str)], salt: &str) -> String {
    let base = cfg.base_url_trimmed();
    let mut params = auth::auth_params(cfg, salt);
    for (k, v) in extra {
        match params.iter_mut().find(|(pk, _)| pk == k) {
            Some(existing) => existing.1 = (*v).to_string(),
            None => params.push((k.to_string(), (*v).to_string())),
        }
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish();
    format!("{base}/rest/{method}?{query}")
}

/// URL the webview's `<audio>` element plays from: the direct `stream` upstream
/// URL, wrapped in the `stream://` proxy so ranged requests forward CORS-clean.
/// `format=raw` requests original, un-transcoded bytes. Mirrors `client.ts:230-233`.
pub fn stream_url(cfg: &Config, id: &str, salt: &str) -> String {
    let upstream = api_url(cfg, "stream", &[("id", id), ("format", "raw")], salt);
    format!(
        "stream://localhost/?src={}",
        encode_uri_component(&upstream)
    )
}

/// DIRECT authenticated `stream` URL (no proxy wrapping) for the native Rust audio
/// engine to fetch. `format=raw` requests original, un-transcoded bytes. Mirrors
/// `client.ts:236-238`.
///
/// This is distinct from [`download_url`]: both may return the same bytes today,
/// but `stream` is Navidrome's playback endpoint (free to transcode per-request in
/// principle) while `download` is guaranteed original bytes. The offline cache must
/// use `download_url`, not this function.
pub fn stream_src_url(cfg: &Config, id: &str, salt: &str) -> String {
    api_url(cfg, "stream", &[("id", id), ("format", "raw")], salt)
}

/// Proxied cover-art URL for `<img src>`, wrapped like [`stream_url`] so it loads
/// CORS-clean. Returns `None` when `cover_art` is absent — there is no art to fetch
/// — matching `coverArtUrl(undefined) -> null` at `client.ts:241-245`.
///
/// # Why `size` is optional, and why this crate always passes `None`
///
/// The app renders cover art at seven different edge sizes — 80, 120, 160, 200, 300,
/// 512 and 600 (`QueuePanel.tsx:69`, `TransportShell.tsx:31`, `usePlayerStore.ts:413`,
/// `DeckShell.tsx:54`, `useLibrary.ts:126`, `usePlayerStore.ts:426`,
/// `useLibrary.ts:193`). A single pre-signed URL baked at one size cannot serve them
/// all, so [`attach_song_urls`] and [`attach_album_urls`] mint **size-agnostic** URLs:
/// `size` is left off the upstream entirely.
///
/// The size then rides on the *outer* `stream://` URL instead — the frontend appends
/// `&size=600` to the proxy URL and `stream.rs` applies it to the upstream before
/// fetching, falling back to [`DEFAULT_COVER_SIZE`] when it is absent.
///
/// That split exists to keep the pre-signed URL **opaque and immutable**. Subsonic's
/// token signs only `password + salt`, so mutating upstream params would in fact be
/// safe — but it would mean the frontend performing string surgery on a
/// credential-bearing URL, and keeping credentials out of the frontend's hands is the
/// property this whole phase exists to establish. The outer URL is ours to edit; the
/// inner one is a sealed envelope.
///
/// The parameter is kept rather than deleted because the endpoint genuinely supports it
/// and a non-proxied consumer may want it; `None` at every call site in this crate is a
/// deliberate choice, visible as such.
pub fn cover_art_url(
    cfg: &Config,
    cover_art: Option<&str>,
    size: Option<u32>,
    salt: &str,
) -> Option<String> {
    let cover_art = cover_art?;
    let size = size.map(|s| s.to_string());
    let mut extra = vec![("id", cover_art)];
    if let Some(size) = size.as_deref() {
        extra.push(("size", size));
    }
    let upstream = api_url(cfg, "getCoverArt", &extra, salt);
    Some(format!(
        "stream://localhost/?src={}",
        encode_uri_component(&upstream)
    ))
}

/// DIRECT, authenticated `getCoverArt` URL (no `stream://` wrapping) for consumers with
/// no protocol handler to resolve a proxied URL — namely `eko-cli`, which runs as a
/// bare terminal process and cannot depend on the Tauri webview. Returns `None` when
/// `cover_art` is absent, matching [`cover_art_url`].
///
/// # Why this one takes a size and always applies it
///
/// [`cover_art_url`] mints size-agnostic upstream URLs on purpose: the `stream://` proxy
/// sits between the frontend and Navidrome, so the size can ride on the *outer* URL and
/// get applied when the proxy fetches (falling back to [`DEFAULT_COVER_SIZE`] if the
/// caller left it off). That is the whole reason the size parameter could be deferred.
///
/// A direct URL has no proxy in the path to defer to — this URL that goes out over the
/// wire is the only chance to say what size is wanted, so `size` is applied to the
/// upstream request itself, not left for something downstream to add. A `None` here
/// falls back to [`DEFAULT_COVER_SIZE`] for the same reason `stream.rs` does: some size
/// must go on the wire, and 300 is the existing default worth staying consistent with.
///
/// **Do not use this from inside the Tauri app.** It is a direct, credential-bearing
/// URL — handing it to `<img src>` would leak the signed token into the webview/DOM,
/// which is exactly what the `stream://` proxy in [`cover_art_url`] exists to prevent.
pub fn cover_art_src_url(
    cfg: &Config,
    cover_art: Option<&str>,
    size: Option<u32>,
    salt: &str,
) -> Option<String> {
    let cover_art = cover_art?;
    let size = size.unwrap_or(DEFAULT_COVER_SIZE).to_string();
    let extra = [("id", cover_art), ("size", size.as_str())];
    Some(api_url(cfg, "getCoverArt", &extra, salt))
}

/// DIRECT, authenticated `download` endpoint URL — returns the ORIGINAL file bytes,
/// no transcode, ever. Mirrors `client.ts:251-253`.
///
/// **This is what the Pro offline cache must store.** Substituting
/// [`stream_src_url`] here would let the cache silently persist transcoded audio,
/// breaking the bit-perfect guarantee for every cached track.
pub fn download_url(cfg: &Config, id: &str, salt: &str) -> String {
    api_url(cfg, "download", &[("id", id)], salt)
}

/// Which server a song belongs to, as a play queue stores it: the configured base URL
/// without a trailing slash. Carries no credential, so it is safe to hold and log.
pub fn server_key(cfg: &Config) -> String {
    cfg.base_url.trim_end_matches('/').to_string()
}

/// Fill in a song's four pre-signed URL fields in place.
///
/// The TypeScript minted these lazily at each call site; here every song leaving
/// [`crate::Client`] carries them, so the frontend never needs the password. Each URL
/// gets its own fresh salt from [`auth::random_salt`] — salts are per-request nonces,
/// not per-session, and reusing one across URLs would be pointless coupling.
///
/// `cover_url` is `None` exactly when the song has no `coverArt` id, matching
/// `coverArtUrl(undefined) -> null`. It is minted **without** a `size` — the caller
/// picks that on the outer `stream://` URL; see [`cover_art_url`].
///
/// Note which endpoint each field points at, because two of them look interchangeable
/// and are not: `stream_src_url` is the *stream* endpoint (playback; the server may
/// transcode) while `download_url` is the *download* endpoint (original bytes,
/// guaranteed). The Pro offline cache reads `download_url`; swapping them would make it
/// persist transcoded audio and break bit-perfect playback for cached tracks silently.
pub fn attach_song_urls(cfg: &Config, song: &mut SubSong) {
    song.stream_url = Some(stream_url(cfg, &song.id, &auth::random_salt()));
    song.stream_src_url = Some(stream_src_url(cfg, &song.id, &auth::random_salt()));
    song.download_url = Some(download_url(cfg, &song.id, &auth::random_salt()));
    song.cover_url = cover_art_url(cfg, song.cover_art.as_deref(), None, &auth::random_salt());
    song.server = Some(server_key(cfg));
}

/// Fill in an album's `cover_url` in place — `None` when it has no `coverArt` id, and
/// size-agnostic like [`attach_song_urls`]. Albums have no streamable bytes of their
/// own, so this is the only URL they carry.
pub fn attach_album_urls(cfg: &Config, album: &mut SubAlbum) {
    album.cover_url = cover_art_url(cfg, album.cover_art.as_deref(), None, &auth::random_salt());
}
