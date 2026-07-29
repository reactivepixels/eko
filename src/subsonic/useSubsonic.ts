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

/**
 * Album pagination.
 *
 * `getAlbumList2` caps `size` at 500 per the Subsonic spec, so a library larger than that
 * REQUIRES paging — EKO previously fetched one page and stopped, silently showing only the
 * first 500 albums of libraries that routinely run into the thousands.
 *
 * Termination: stop on an EMPTY page, and advance the offset by the page's ACTUAL length
 * rather than the requested size. Some servers silently cap the page size below what you
 * asked for; terminating on `page.length < PAGE_SIZE` would then stop after one short page
 * and reintroduce the same bug. Costs one extra (empty) request; worth it for correctness.
 */
const PAGE_SIZE = 500;
/** Safety stop so a misbehaving server can't spin us forever. ~600 pages. */
const MAX_ALBUMS = 300_000;

/** Bumped on every connect/disconnect so a slow in-flight page load can detect it's stale. */
let loadGen = 0;
/** Bumped on every search so a slow in-flight search can detect it's been superseded. */
let searchGen = 0;

interface SubsonicState {
  connected: boolean;
  status: "idle" | "connecting" | "error";
  error: string | null;
  config: SubsonicConfig | null;
  albums: SubAlbum[];
  playlists: SubPlaylist[];
  /** True while additional album pages are still streaming in behind the first page. */
  albumsLoading: boolean;

  /** Server-side `search3` results. `null` = no active search (show the browse list). */
  searchResults: { albums: SubAlbum[]; tracks: Track[] } | null;
  /** True while a search request is in flight (Navidrome can take 5–20s on huge libraries). */
  searching: boolean;

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

  /** Run a server-side search (`search3`). Supersedes any in-flight search. */
  runSearch: (q: string) => Promise<void>;
  /** Drop search results and return to the browse list. */
  clearSearch: () => void;
}

/**
 * Fetch page 1 of the album list. Returned separately from the rest so `connect` can flip to
 * "connected" and paint the first screen immediately, instead of blocking on a 15k-album library.
 */
async function fetchFirstAlbumPage(): Promise<SubAlbum[]> {
  return getAlbums(PAGE_SIZE, 0);
}

/**
 * Walk every remaining album page. Pure (no Tauri, no store) so it can be unit-tested — the
 * termination rule is the whole point of this fix and it has a subtle failure mode, see below.
 *
 * @param fetchPage  fetch one page at the given offset
 * @param first      page 1, already fetched (used to seed the accumulator + starting offset)
 * @param onPage     called with a fresh array after each page, for progressive rendering
 * @param isStale    checked after every fetch; return true to abandon the walk
 * @param max        hard cap so a misbehaving server can't spin forever
 */
export async function walkAlbumPages(
  fetchPage: (offset: number) => Promise<SubAlbum[]>,
  {
    first = [],
    onPage,
    isStale,
    max = MAX_ALBUMS,
  }: {
    first?: SubAlbum[];
    onPage?: (all: SubAlbum[]) => void;
    isStale?: () => boolean;
    max?: number;
  } = {},
): Promise<SubAlbum[]> {
  const all = [...first];
  // An empty first page means an empty library — nothing more to ask for.
  if (first.length === 0) return all;
  let offset = first.length;
  for (;;) {
    const page = await fetchPage(offset);
    if (isStale?.()) return all;
    // Terminate ONLY on an empty page, and advance by the page's ACTUAL length. Terminating on
    // `page.length < PAGE_SIZE` would break against servers that silently cap the page size
    // below what we asked for — they'd return one short page and we'd stop early, which is
    // exactly the truncation bug this function exists to fix. Costs one extra empty request.
    if (page.length === 0) return all;
    all.push(...page);
    offset += page.length;
    onPage?.([...all]);
    if (all.length >= max) return all;
  }
}

/**
 * Store-facing wrapper: streams remaining pages into state so the grid fills in progressively,
 * and abandons the walk if `loadGen` moved (server switched / disconnected mid-load).
 */
async function loadRemainingAlbums(
  gen: number,
  first: SubAlbum[],
  set: (partial: Partial<SubsonicState>) => void,
): Promise<void> {
  try {
    await walkAlbumPages((offset) => getAlbums(PAGE_SIZE, offset), {
      first,
      onPage: (all) => set({ albums: all }),
      isStale: () => gen !== loadGen,
    });
  } catch {
    // Keep whatever pages already landed — a partial library beats an error screen.
  }
  if (gen === loadGen) set({ albumsLoading: false });
}

export const useSubsonic = create<SubsonicState>((set, get) => ({
  connected: false,
  status: "idle",
  error: null,
  config: null,
  albums: [],
  playlists: [],
  albumsLoading: false,
  searchResults: null,
  searching: false,
  serverList: getServerList(),
  manageOpen: false,

  connect: async (cfg) => {
    set({ status: "connecting", error: null });
    setConfig(cfg);
    setStreamOrigin(cfg.baseUrl); // allow the proxy to fetch this server before any cover art
    const gen = ++loadGen;
    try {
      await ping();
      const albums = await fetchFirstAlbumPage();
      if (gen !== loadGen) return false; // superseded while we were connecting
      // Paint immediately on page 1, then stream the rest in behind it.
      set({
        connected: true,
        status: "idle",
        config: cfg,
        albums,
        error: null,
        searchResults: null,
        searching: false,
        albumsLoading: albums.length >= PAGE_SIZE,
      });
      void loadRemainingAlbums(gen, albums, set);
      getPlaylists()
        .then((playlists) => set({ playlists }))
        .catch(() => {
          /* ignore */
        });
      return true;
    } catch (e) {
      setConfig(null);
      setStreamOrigin(null);
      set({
        connected: false,
        status: "error",
        albumsLoading: false,
        error: friendlyConnectError(e, cfg.baseUrl),
      });
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
    loadGen++; // abandon any in-flight page load
    searchGen++; // and any in-flight search
    set({
      connected: false,
      status: "idle",
      config: null,
      albums: [],
      albumsLoading: false,
      searchResults: null,
      searching: false,
    });
  },

  addAndConnect: async (name, cfg) => {
    set({ status: "connecting", error: null });
    setConfig(cfg);
    setStreamOrigin(cfg.baseUrl);
    const gen = ++loadGen;
    try {
      await ping();
      const albums = await fetchFirstAlbumPage();
      if (gen !== loadGen) return false; // superseded while we were connecting

      // Persist the new server entry.
      const entry = await addServer(
        { name, baseUrl: cfg.baseUrl, username: cfg.username },
        cfg.password,
      );
      setActiveServerId(entry.id);
      const list = getServerList();

      set({
        connected: true,
        status: "idle",
        config: cfg,
        albums,
        error: null,
        serverList: list,
        searchResults: null,
        searching: false,
        albumsLoading: albums.length >= PAGE_SIZE,
      });
      void loadRemainingAlbums(gen, albums, set);
      getPlaylists()
        .then((playlists) => set({ playlists }))
        .catch(() => {
          /* ignore */
        });
      return true;
    } catch (e) {
      setConfig(null);
      setStreamOrigin(null);
      set({
        connected: false,
        status: "error",
        albumsLoading: false,
        error: friendlyConnectError(e, cfg.baseUrl),
      });
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

  /**
   * Server-side search via `search3` — the only way to find anything in a library larger than
   * the albums currently loaded, and the only way to match on SONG TITLE at all (the browse
   * grid only ever knew album names and artists).
   *
   * Guarded by a generation counter rather than AbortController because the Subsonic calls go
   * through Tauri's Rust HTTP client, which we don't hand a signal to. Late responses from a
   * superseded query are discarded instead of clobbering newer results — necessary because
   * Navidrome's own search can take 5–20s on very large libraries, so out-of-order completion
   * is the normal case, not an edge case.
   */
  runSearch: async (q) => {
    const query = q.trim();
    const gen = ++searchGen;
    if (!query) {
      set({ searchResults: null, searching: false });
      return;
    }
    set({ searching: true });
    try {
      const { albums, songs } = await search(query);
      if (gen !== searchGen) return; // superseded by a newer query
      set({ searchResults: { albums, tracks: songs.map(toTrack) }, searching: false });
    } catch {
      if (gen !== searchGen) return;
      // Surface "no matches" rather than an error screen — search failing is not fatal.
      set({ searchResults: { albums: [], tracks: [] }, searching: false });
    }
  },

  clearSearch: () => {
    searchGen++; // discard anything in flight
    set({ searchResults: null, searching: false });
  },

  queueSongs: (songs, autoplay = false) => {
    usePlayerStore.getState().setQueue(songs.map(toTrack), autoplay);
  },
}));

// Re-export ServerEntry type for consumers.
export type { ServerEntry, ServerList };
