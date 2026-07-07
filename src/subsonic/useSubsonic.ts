import { create } from "zustand";
import {
  setConfig,
  ping,
  getAlbums,
  getAlbum,
  getRandomSongs,
  search,
  mimeForSong,
  getPlaylists,
  getPlaylist,
  type SubsonicConfig,
  type SubAlbum,
  type SubSong,
  type SubPlaylist,
} from "./client";
import { invoke } from "@tauri-apps/api/core";
import { usePlayerStore } from "../store/usePlayerStore";
import type { Track } from "../types";
import {
  getServerList,
  addServer,
  removeServer,
  renameServer,
  setActiveServerId,
  getServerPassword,
  migrateLegacyServer,
  type ServerEntry,
  type ServerList,
} from "./serverList";

/** The origin (scheme://host:port) the `stream://` proxy is allowed to fetch — the SSRF
 *  allowlist. Registered on connect, cleared on disconnect. */
function setStreamOrigin(baseUrl: string | null) {
  let origin: string | null = null;
  if (baseUrl) {
    try {
      origin = new URL(baseUrl).origin;
    } catch {
      origin = null;
    }
  }
  void invoke("set_stream_origin", { origin });
}

/**
 * Turn a raw connect failure into a short, user-facing message. Subsonic API errors from
 * `client.ts` (e.g. "Wrong username or password.") are already clean and pass through.
 * Transport failures from the Tauri HTTP plugin look like
 * `error sending request for url (http://…/rest/ping?u=…&t=<token>)` — never surface that:
 * it's noise and it leaks the auth token. Show a friendly, actionable message instead.
 */
function friendlyConnectError(e: unknown, baseUrl: string): string {
  const raw = e instanceof Error ? e.message : String(e);
  const isTransport =
    /sending request|failed to fetch|load failed|trying to connect|dns error|timed out|timeout|connection (refused|reset|closed)|network|unreachable|not permitted/i.test(
      raw,
    );
  if (isTransport) {
    let host = baseUrl;
    try {
      host = new URL(baseUrl).host;
    } catch {
      /* keep baseUrl as-is */
    }
    return `Couldn't reach ${host}. Check the address and that the server is running. If it's on your local network, allow EKO under System Settings → Privacy & Security → Local Network.`;
  }
  if (/^HTTP 401$|unauthor|wrong (username|password)/i.test(raw)) {
    return "Wrong username or password.";
  }
  return raw;
}

function toTrack(s: SubSong): Track {
  return {
    id: s.id,
    subsonicId: s.id,
    path: "",
    title: s.title ?? null,
    artist: s.artist ?? null,
    album: s.album ?? null,
    duration: s.duration ?? 0,
    bitrate: s.bitRate ?? null,
    sampleRate: s.samplingRate ?? null,
    channels: s.channelCount ?? 2,
    mime: mimeForSong(s),
    coverArt: s.coverArt,
    // OpenSubsonic ReplayGain → same fields local files carry, so server tracks normalise too.
    rgTrackGain: s.replayGain?.trackGain ?? null,
    rgAlbumGain: s.replayGain?.albumGain ?? null,
    rgTrackPeak: s.replayGain?.trackPeak ?? null,
    rgAlbumPeak: s.replayGain?.albumPeak ?? null,
  };
}

interface SubsonicState {
  connected: boolean;
  status: "idle" | "connecting" | "error";
  error: string | null;
  config: SubsonicConfig | null;
  albums: SubAlbum[];
  playlists: SubPlaylist[];

  // ── Multi-server ───────────────────────────────────────────────────────────
  /** The server list metadata (no passwords). */
  serverList: ServerList;
  /** Whether the manage-servers panel is open. */
  manageOpen: boolean;

  connect: (cfg: SubsonicConfig) => Promise<boolean>;
  /** Connect to the given server entry using its stored Keychain password. */
  connectById: (id: string) => Promise<boolean>;
  autoConnect: () => Promise<void>;
  disconnect: () => void;

  // ── Server list management ─────────────────────────────────────────────────
  /** Add a new server (after a successful connection via ConnectPanel). */
  addAndConnect: (name: string | undefined, cfg: SubsonicConfig) => Promise<boolean>;
  removeServer: (id: string) => Promise<void>;
  renameServer: (id: string, name: string) => void;
  switchServer: (id: string) => Promise<void>;
  refreshServerList: () => void;
  setManageOpen: (open: boolean) => void;

  playAlbum: (id: string) => Promise<void>;
  openAlbum: (id: string) => Promise<{ album: SubAlbum; tracks: Track[] }>;
  openPlaylist: (id: string) => Promise<{ name: string; tracks: Track[] }>;
  playTracks: (tracks: Track[], index: number) => void;
  loadRandom: () => Promise<void>;
  doSearch: (q: string) => Promise<{ albums: SubAlbum[]; songs: SubSong[] }>;
  queueSongs: (songs: SubSong[], autoplay?: boolean) => void;
}

export const useSubsonic = create<SubsonicState>((set, get) => ({
  connected: false,
  status: "idle",
  error: null,
  config: null,
  albums: [],
  playlists: [],
  serverList: getServerList(),
  manageOpen: false,

  connect: async (cfg) => {
    set({ status: "connecting", error: null });
    setConfig(cfg);
    setStreamOrigin(cfg.baseUrl); // allow the proxy to fetch this server before any cover art
    try {
      await ping();
      const albums = await getAlbums(500);
      set({ connected: true, status: "idle", config: cfg, albums, error: null });
      getPlaylists()
        .then((playlists) => set({ playlists }))
        .catch(() => {
          /* ignore */
        });
      return true;
    } catch (e) {
      setConfig(null);
      setStreamOrigin(null);
      set({ connected: false, status: "error", error: friendlyConnectError(e, cfg.baseUrl) });
      return false;
    }
  },

  connectById: async (id) => {
    const list = getServerList();
    const entry = list.servers.find((s) => s.id === id);
    if (!entry) {
      set({ status: "error", error: "Server not found" });
      return false;
    }
    const password = await getServerPassword(id);
    if (!password) {
      set({ status: "error", error: "No password stored for this server" });
      return false;
    }
    return get().connect({ baseUrl: entry.baseUrl, username: entry.username, password });
  },

  autoConnect: async () => {
    // Step 1: migrate the legacy single-server entry if present.
    const migrated = await migrateLegacyServer();

    // Refresh the server list after potential migration.
    const list = getServerList();
    set({ serverList: list });

    if (migrated) {
      // We just migrated — connect using the migrated password directly.
      if (migrated.password) {
        await get().connect({
          baseUrl: migrated.baseUrl,
          username: migrated.username,
          password: migrated.password,
        });
        setActiveServerId(migrated.id);
      }
      return;
    }

    // Step 2: connect to the active server (or first in list).
    if (!list.activeId) return;
    await get().connectById(list.activeId);
  },

  disconnect: () => {
    setConfig(null);
    setStreamOrigin(null);
    set({ connected: false, status: "idle", config: null, albums: [] });
  },

  addAndConnect: async (name, cfg) => {
    set({ status: "connecting", error: null });
    setConfig(cfg);
    setStreamOrigin(cfg.baseUrl);
    try {
      await ping();
      const albums = await getAlbums(500);

      // Persist the new server entry.
      const entry = await addServer(
        { name, baseUrl: cfg.baseUrl, username: cfg.username },
        cfg.password,
      );
      setActiveServerId(entry.id);
      const list = getServerList();

      set({ connected: true, status: "idle", config: cfg, albums, error: null, serverList: list });
      getPlaylists()
        .then((playlists) => set({ playlists }))
        .catch(() => {
          /* ignore */
        });
      return true;
    } catch (e) {
      setConfig(null);
      setStreamOrigin(null);
      set({ connected: false, status: "error", error: friendlyConnectError(e, cfg.baseUrl) });
      return false;
    }
  },

  removeServer: async (id) => {
    const wasActive = getServerList().activeId === id;
    await removeServer(id);
    const list = getServerList();
    set({ serverList: list });
    if (wasActive) {
      // Disconnect and try the next server (if any).
      get().disconnect();
      if (list.activeId) {
        await get().connectById(list.activeId);
      }
    }
  },

  renameServer: (id, name) => {
    renameServer(id, name);
    set({ serverList: getServerList() });
  },

  switchServer: async (id) => {
    if (id === getServerList().activeId && get().connected) return;
    get().disconnect();
    setActiveServerId(id);
    set({ serverList: getServerList() });
    await get().connectById(id);
  },

  refreshServerList: () => {
    set({ serverList: getServerList() });
  },

  setManageOpen: (open) => set({ manageOpen: open }),

  playAlbum: async (id) => {
    const { songs } = await getAlbum(id);
    usePlayerStore.getState().setQueue(songs.map(toTrack), true);
  },

  openAlbum: async (id) => {
    const { album, songs } = await getAlbum(id);
    return { album, tracks: songs.map(toTrack) };
  },

  openPlaylist: async (id) => {
    const { name, songs } = await getPlaylist(id);
    return { name, tracks: songs.map(toTrack) };
  },

  playTracks: (tracks, index) => {
    const p = usePlayerStore.getState();
    p.setQueue(tracks, false);
    void p.playAt(index);
  },

  loadRandom: async () => {
    const songs = await getRandomSongs(50);
    usePlayerStore.getState().setQueue(songs.map(toTrack), false);
  },

  doSearch: async (q) => search(q),

  queueSongs: (songs, autoplay = false) => {
    usePlayerStore.getState().setQueue(songs.map(toTrack), autoplay);
  },
}));

// Re-export ServerEntry type for consumers.
export type { ServerEntry, ServerList };
