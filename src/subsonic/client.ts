// OpenSubsonic API client for Navidrome. Metadata calls go through Tauri's Rust HTTP
// (no browser CORS). Audio + cover art load via the `stream://` proxy (see src-tauri/
// src/stream.rs): progressive, range-based, and CORS-clean so the EQ + visualiser keep
// working without downloading whole files first.
import { fetch } from "@tauri-apps/plugin-http";
import { md5 } from "js-md5";

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
}

let cfg: SubsonicConfig | null = null;
export function setConfig(c: SubsonicConfig | null) {
  cfg = c;
}
export function getConfig(): SubsonicConfig | null {
  return cfg;
}

function authParams(): URLSearchParams {
  if (!cfg) throw new Error("Subsonic not configured");
  const salt = Math.random().toString(36).slice(2, 12);
  const token = md5(cfg.password + salt);
  return new URLSearchParams({
    u: cfg.username,
    t: token,
    s: salt,
    v: "1.16.1",
    c: "eko",
    f: "json",
  });
}

function apiUrl(method: string, extra: Record<string, string> = {}): string {
  if (!cfg) throw new Error("Subsonic not configured");
  const base = cfg.baseUrl.replace(/\/+$/, "");
  const p = authParams();
  for (const k in extra) p.set(k, extra[k]);
  return `${base}/rest/${method}?${p.toString()}`;
}

async function call(
  method: string,
  extra: Record<string, string> = {},
): Promise<Record<string, unknown>> {
  const res = await fetch(apiUrl(method, extra), { method: "GET" });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const json = (await res.json()) as Record<string, unknown>;
  const sr = json["subsonic-response"] as Record<string, unknown>;
  if (!sr) throw new Error("Bad response");
  if (sr.status !== "ok") {
    const err = sr.error as { message?: string } | undefined;
    throw new Error(err?.message ?? "Subsonic error");
  }
  return sr;
}

/** Verify the connection + credentials. */
export async function ping(): Promise<void> {
  await call("ping");
}

export async function getAlbums(size = 500, offset = 0): Promise<SubAlbum[]> {
  const r = await call("getAlbumList2", {
    type: "alphabeticalByArtist",
    size: String(size),
    offset: String(offset),
  });
  const list = r.albumList2 as { album?: SubAlbum[] } | undefined;
  return list?.album ?? [];
}

export async function getAlbum(id: string): Promise<{ album: SubAlbum; songs: SubSong[] }> {
  const r = await call("getAlbum", { id });
  const album = r.album as SubAlbum & { song?: SubSong[] };
  return { album, songs: album.song ?? [] };
}

export async function search(query: string): Promise<{ albums: SubAlbum[]; songs: SubSong[] }> {
  const r = await call("search3", { query, songCount: "50", albumCount: "30", artistCount: "0" });
  const sr = r.searchResult3 as { album?: SubAlbum[]; song?: SubSong[] } | undefined;
  return { albums: sr?.album ?? [], songs: sr?.song ?? [] };
}

export interface SubPlaylist {
  id: string;
  name: string;
  songCount?: number;
  coverArt?: string;
}

export async function getPlaylists(): Promise<SubPlaylist[]> {
  const r = await call("getPlaylists");
  const p = r.playlists as { playlist?: SubPlaylist[] } | undefined;
  return p?.playlist ?? [];
}

export async function getPlaylist(id: string): Promise<{ name: string; songs: SubSong[] }> {
  const r = await call("getPlaylist", { id });
  const pl = r.playlist as { name?: string; entry?: SubSong[] };
  return { name: pl.name ?? "Playlist", songs: pl.entry ?? [] };
}

export async function getRandomSongs(size = 50): Promise<SubSong[]> {
  const r = await call("getRandomSongs", { size: String(size) });
  const rs = r.randomSongs as { song?: SubSong[] } | undefined;
  return rs?.song ?? [];
}

/**
 * URL the `<audio>` element plays from. It points at the Rust `stream://` proxy, which
 * forwards ranged requests to Navidrome (`format=raw` = original bytes, bit-perfect).
 * Progressive + same-origin, so playback starts instantly and the EQ/spectrum keep working.
 */
export function streamUrl(id: string): string {
  const upstream = apiUrl("stream", { id, format: "raw" });
  return `stream://localhost/?src=${encodeURIComponent(upstream)}`;
}

/** Direct Navidrome stream URL (with auth) — for the native Rust engine to fetch. */
export function streamSrcUrl(id: string): string {
  return apiUrl("stream", { id, format: "raw" });
}

/** Absolute cover-art URL (also routed through the stream proxy so it loads CORS-clean). */
export function coverArtUrl(coverArt: string | undefined, size = 300): string | null {
  if (!coverArt) return null;
  const upstream = apiUrl("getCoverArt", { id: coverArt, size: String(size) });
  return `stream://localhost/?src=${encodeURIComponent(upstream)}`;
}

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
