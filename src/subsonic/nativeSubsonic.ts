/**
 * The OpenSubsonic surface, served by Rust (`eko-net`) over Tauri IPC.
 *
 * This is a shape-compatible replacement for `client.ts`: the same exported names and
 * signatures, so a call site migrates by changing its import path and nothing else.
 * Two things genuinely differ, and both are deliberate:
 *
 *  1. **`setConfig` is `async`.** It invokes a command instead of assigning a
 *     module-level variable, so every caller has to `await` it. There is no
 *     `getConfig()` counterpart — the config (and with it the server password) lives in
 *     Rust now, and handing it back to the webview would undo the entire point. A call
 *     site that only wanted to know "is a server configured?" reads `useSubsonic`'s
 *     `config` field, which carries the base URL and username and no credential.
 *
 *  2. **The four URL builders are gone.** `streamUrl` / `streamSrcUrl` / `downloadUrl` /
 *     `coverArtUrl` needed the password to sign a URL. Rust pre-signs them into the
 *     payloads instead — `SubSong.streamUrl` / `.streamSrcUrl` / `.downloadUrl` /
 *     `.coverUrl` and `SubAlbum.coverUrl` — so call sites read a field. The one thing a
 *     field read cannot express is cover-art *size*, because a single pre-signed URL
 *     cannot serve the seven edge sizes the app renders at; [`coverAt`] below is the
 *     sole way to add one, and it is deliberately the only implementation in the tree.
 *
 * `invoke` is called with a **literal** command name at every site on purpose:
 * `crates/eko-tauri/tests/ipc_contract.rs` greps for exactly that and asserts a
 * registered handler exists, which is what turns a typo here into a failing test rather
 * than a dead button in the user's hands. Do not route these through a helper that takes
 * the name as a parameter.
 */

import { invoke } from "@tauri-apps/api/core";
import type { DownloadUrl, StreamSrcUrl } from "../types";

// ── Error transport ───────────────────────────────────────────────────────────

/**
 * Tauri rejects with the command's plain error *string*; `client.ts` threw an `Error`.
 * Callers all over the app branch on `e instanceof Error ? e.message : String(e)`
 * (`useSubsonic.ts`'s `friendlyConnectError` among them), so restore the `Error` wrapper
 * here rather than making every catch block handle both shapes.
 */
function asError(e: unknown): Error {
  if (e instanceof Error) return e;
  return new Error(typeof e === "string" ? e : String(e));
}

/** Re-throw an IPC rejection as an `Error`, preserving the message verbatim. */
function lift<T>(p: Promise<T>): Promise<T> {
  return p.catch((e: unknown) => {
    throw asError(e);
  });
}

// ── Types (mirror `eko_net::types`, which mirrors the old `client.ts`) ─────────

export interface SubsonicConfig {
  baseUrl: string;
  username: string;
  password: string;
}

export interface SubAlbum {
  id: string;
  name: string;
  artist: string;
  artistId?: string;
  songCount?: number;
  year?: number;
  coverArt?: string;
  /** Pre-signed, proxied, and **size-agnostic** — add a size with [`coverAt`]. */
  coverUrl?: string;
}

export interface SubSong {
  id: string;
  title: string;
  artist: string;
  album: string;
  duration?: number;
  bitRate?: number;
  samplingRate?: number;
  channelCount?: number;
  suffix?: string;
  contentType?: string;
  track?: number;
  coverArt?: string;
  // OpenSubsonic ReplayGain (gains in dB, peaks linear) — present on servers that support it.
  replayGain?: {
    trackGain?: number;
    albumGain?: number;
    trackPeak?: number;
    albumPeak?: number;
  };
  /** Pre-signed `stream://` URL for webview `<audio>` playback. */
  streamUrl?: string;
  /**
   * Pre-signed DIRECT upstream stream URL — what the native Rust engine fetches. This is
   * the `stream` endpoint, so the server may transcode it. Branded (`StreamSrcUrl`) so it
   * cannot be passed where a `DownloadUrl` is required; see the note in `types.ts`.
   */
  streamSrcUrl?: StreamSrcUrl;
  /**
   * Pre-signed DIRECT `download` URL — original bytes, no transcode (offline cache).
   * Branded (`DownloadUrl`) so `streamSrcUrl` cannot be substituted for it; see `types.ts`.
   */
  downloadUrl?: DownloadUrl;
  /** Pre-signed, proxied, and **size-agnostic** — add a size with [`coverAt`]. */
  coverUrl?: string;
  /** The server that listed this song (its base URL). Carries no credential. */
  server?: string;
}

export interface SubPlaylist {
  id: string;
  name: string;
  songCount?: number;
  coverArt?: string;
}

/** Genre list from the server (name + songCount). */
export interface SubGenre {
  value: string; // genre name (the "name" field in Subsonic XML is returned as "value" in JSON)
  songCount: number;
  albumCount?: number;
}

/** getAlbumList2 type filter — used for smart playlist rules. */
export type AlbumListType =
  | "newest"
  | "recent"
  | "frequent"
  | "random"
  | "starred"
  | "alphabeticalByArtist"
  | "alphabeticalByName"
  | "byGenre"
  | "byYear";

export interface SubSimilarArtist {
  id: string;
  name: string;
}

export interface SubArtistInfo {
  biography?: string;
  lastFmUrl?: string;
  similarArtist?: SubSimilarArtist[];
}

export interface SubStarred {
  song?: SubSong[];
  album?: SubAlbum[];
}

/** One synced-lyric line: `start` is an ms offset from track start. */
export interface SyncedLyricLine {
  start: number;
  value: string;
}

export interface LyricsResult {
  synced: SyncedLyricLine[] | null;
  unsynced: string | null; // plain text block (newline-separated)
}

// ── Cover art sizing ──────────────────────────────────────────────────────────

/**
 * Request a pre-signed cover URL at a given pixel size.
 *
 * The minted `coverUrl` carries no size: the app renders art at seven different edge
 * sizes (80 → 600) and one signed URL cannot serve them all. `stream.rs` reads `size`
 * off the **outer** `stream://` URL and folds it into the upstream `getCoverArt`
 * request, defaulting to 300 when absent — so appending it here is all that's needed.
 *
 * **This is the only implementation of that append in the tree, and must stay that way.**
 * A second `.replace()` or template literal somewhere else is exactly how the proxy's
 * contract and the frontend's drift apart silently.
 */
export function coverAt(coverUrl: string | null | undefined, size: number): string | null {
  return coverUrl ? `${coverUrl}&size=${size}` : null;
}

// ── Connection ────────────────────────────────────────────────────────────────

/**
 * Set (or, with `null`, clear) the connection. **Async** — unlike `client.ts`'s
 * synchronous assignment, this hands the config to Rust, which builds the HTTP client
 * and keeps the password. Callers must `await` it before any other call in this module.
 */
export async function setConfig(c: SubsonicConfig | null): Promise<void> {
  await lift(invoke<void>("subsonic_set_config", { config: c }));
}

/** Verify the connection + credentials. */
export async function ping(): Promise<void> {
  await lift(invoke<void>("subsonic_ping"));
}

// ── Library ───────────────────────────────────────────────────────────────────
//
// **On the restated default arguments below (500 / 0 / 50 / 50).**
//
// `eko-net` already carries these: every one of these commands takes an `Option<T>` and
// applies the same fallback in Rust, so **`eko-net` is the authority** and these
// TypeScript defaults mean the Rust ones never fire in production — the frontend always
// sends a concrete value. The duplication is deliberate, not an oversight: the old
// `client.ts` signatures had these defaults, ~20 call sites rely on calling
// `getAlbums()` / `getRandomSongs()` with no arguments, and keeping them here is what
// makes the port behaviour-preserving rather than a signature change smuggled into a
// refactor.
//
// What is *not* enforced anywhere is that the two copies keep agreeing. They agree today
// (checked field for field). If you change a default, change it in `eko-net`'s endpoint
// layer too — or delete the one here and let the `Option<T>` do its job.

export function getAlbums(size = 500, offset = 0): Promise<SubAlbum[]> {
  return lift(invoke<SubAlbum[]>("subsonic_get_albums", { size, offset }));
}

/**
 * Every song on the server, one page at a time — the full track index.
 *
 * Backed by `search3` with an empty query (there is no list-all-songs endpoint); Navidrome
 * answers that with the whole song set. A server that reads it literally returns none, which
 * the caller treats as "no index available" rather than an error. Defaults mirror
 * `eko-net`'s `DEFAULT_SONG_LIST_SIZE` / `DEFAULT_SONG_LIST_OFFSET`; see the note above.
 */
export function getSongs(size = 500, offset = 0): Promise<SubSong[]> {
  return lift(invoke<SubSong[]>("subsonic_get_songs", { size, offset }));
}

export function getAlbum(id: string): Promise<{ album: SubAlbum; songs: SubSong[] }> {
  return lift(invoke<{ album: SubAlbum; songs: SubSong[] }>("subsonic_get_album", { id }));
}

export function search(query: string): Promise<{ albums: SubAlbum[]; songs: SubSong[] }> {
  return lift(invoke<{ albums: SubAlbum[]; songs: SubSong[] }>("subsonic_search", { query }));
}

export function getPlaylists(): Promise<SubPlaylist[]> {
  return lift(invoke<SubPlaylist[]>("subsonic_get_playlists"));
}

export function getPlaylist(id: string): Promise<{ name: string; songs: SubSong[] }> {
  return lift(invoke<{ name: string; songs: SubSong[] }>("subsonic_get_playlist", { id }));
}

/** `size` default duplicates `eko-net`'s; see the note at the top of this section. */
export function getRandomSongs(size = 50, genre?: string): Promise<SubSong[]> {
  return lift(invoke<SubSong[]>("subsonic_get_random_songs", { size, genre: genre ?? null }));
}

export function getGenres(): Promise<SubGenre[]> {
  return lift(invoke<SubGenre[]>("subsonic_get_genres"));
}

/**
 * `getAlbumList2` with an explicit type filter.
 *
 * The command's first parameter is `listType`, not `type` — `type` is a Rust keyword and
 * cannot name a command argument. The TypeScript parameter keeps its old name so call
 * sites are unaffected.
 *
 * `size` / `offset` defaults duplicate `eko-net`'s; see the note at the top of this
 * section.
 */
export function getAlbumList2(
  type: AlbumListType,
  size = 500,
  offset = 0,
  extra: Record<string, string> = {},
): Promise<SubAlbum[]> {
  return lift(
    invoke<SubAlbum[]>("subsonic_get_album_list2", { listType: type, size, offset, extra }),
  );
}

/**
 * Songs similar to the given song (by id).
 *
 * `count` default duplicates `eko-net`'s; see the note at the top of this section.
 */
export function getSimilarSongs2(id: string, count = 50): Promise<SubSong[]> {
  return lift(invoke<SubSong[]>("subsonic_get_similar_songs2", { id, count }));
}

/** Biography + list of similar artists. */
export function getArtistInfo2(id: string): Promise<SubArtistInfo> {
  return lift(invoke<SubArtistInfo>("subsonic_get_artist_info2", { id }));
}

/** Starred songs/albums from the server. */
export function getStarred2(): Promise<SubStarred> {
  return lift(invoke<SubStarred>("subsonic_get_starred2"));
}

// ── Scrobble ──────────────────────────────────────────────────────────────────

/** Send a scrobble to the server.
 *  `submission=false` → "now playing" (called at track start).
 *  `submission=true`  → permanent scrobble (called at the play threshold).
 *  Failures are swallowed — scrobble must never interrupt playback. `eko-net` already
 *  swallows server-side failures; this also swallows "not configured" and IPC errors,
 *  exactly as `client.ts`'s try/catch did. */
export async function scrobble(id: string, submission: boolean): Promise<void> {
  try {
    await invoke<void>("subsonic_scrobble", { id, submission });
  } catch {
    // Best-effort: network/server errors must not affect playback.
  }
}

// ── Lyrics ────────────────────────────────────────────────────────────────────

/** OpenSubsonic `getLyricsBySongId` — synced lyrics with per-line ms offsets. `eko-net`
 *  falls back to the legacy endpoint internally when the server doesn't support it; the
 *  catch here covers the remaining failure (no server configured), which `client.ts`
 *  also degraded to an empty result. */
export async function getLyricsBySongId(songId: string): Promise<LyricsResult> {
  try {
    return await invoke<LyricsResult>("subsonic_get_lyrics_by_song_id", { songId });
  } catch {
    return { synced: null, unsynced: null };
  }
}

/** Legacy `getLyrics` — plain-text, unsynced, keyed by artist + title. Never throws. */
export async function getLyricsLegacy(
  artist: string | null,
  title: string | null,
): Promise<LyricsResult> {
  try {
    return await invoke<LyricsResult>("subsonic_get_lyrics_legacy", { artist, title });
  } catch {
    return { synced: null, unsynced: null };
  }
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/** MIME type to hand the decoder for a song. Pure — no server round-trip needed, so it
 *  stays in TypeScript (`eko_net::types::mime_for_song` is the Rust-side twin, used when
 *  the payload is built there). */
export function mimeForSong(s: { contentType?: string; suffix?: string }): string {
  if (s.contentType) return s.contentType;
  const map: Record<string, string> = {
    flac: "audio/flac",
    mp3: "audio/mpeg",
    m4a: "audio/mp4",
    aac: "audio/aac",
    ogg: "audio/ogg",
    opus: "audio/opus",
    wav: "audio/wav",
  };
  return map[(s.suffix ?? "").toLowerCase()] ?? "audio/mpeg";
}
