import { create } from "zustand";
import {
  setConfig,
  ping,
  getAlbums,
  getSongs,
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
} from "./nativeSubsonic";
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
 * Turn a raw connect failure into a short, user-facing message.
 *
 * Every error reaching here now comes from `eko-net` through the IPC boundary, in one of
 * four shapes: a Subsonic API message (`"Wrong username or password."`), `"HTTP <code>"`,
 * `"Bad response"`, or a `reqwest` transport message. This function's job is purely
 * **legibility** — turn the transport shapes into something a user can act on, and pass
 * the already-clean ones through.
 *
 * **This is no longer a token filter, and must not be relied on as one.** An earlier
 * version of this comment said transport failures arrive as
 * `error sending request for url (…&t=<token>)` and that the regex existed to stop that
 * reaching the user. Both premises are dead: `client.ts` and `@tauri-apps/plugin-http`
 * are gone, and `eko-net`'s `safe_message` calls `reqwest::Error::without_url()` at every
 * error site in the crate, so no URL and no auth token is present in `raw` by the time it
 * gets here. The credential guarantee lives in Rust, where it is enforced by
 * `a_transport_error_message_never_carries_the_signed_url_or_auth_token` — not in this
 * regex, which never covered `"error following redirect"`, `"request or response body
 * error"` or `"error decoding response body"` anyway. Deleting a pattern from this list
 * makes a message uglier; it does not leak anything.
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

/**
 * `SubSong` (wire) → `Track` (app). Exported for `toTrack.test.ts`, which pins the
 * empty-string normalisation below — the one place that difference can be caught.
 */
export function toTrack(s: SubSong): Track {
  return {
    id: s.id,
    subsonicId: s.id,
    path: "",
    // Carried through, not rebuilt: signing lives in Rust now, so these are the only
    // handles the frontend will ever have on this track's audio and art.
    streamSrcUrl: s.streamSrcUrl,
    downloadUrl: s.downloadUrl,
    coverUrl: s.coverUrl,
    server: s.server,
    // `||`, NOT `??` — and this is the whole boundary for it.
    //
    // `eko-net` types these as `String` with `#[serde(default)]`, so an untagged track
    // arrives as `""` where the old TypeScript client left the field `undefined`. `??` is
    // *nullish* coalescing: `"" ?? null` is `""`, so the empty string would flow straight
    // into `Track` — and every downstream fallback (`?? "EKO"`, `?? "Unknown"`, `?? "—"`,
    // `?? "track"`, in useNowPlaying, usePlayerStore, DeckShell and QueuePanel) is itself
    // `??`, so all seven would silently render blank instead of their placeholder. The OS
    // now-playing card is the worst of them: a blank title on the macOS lock screen.
    //
    // Normalising `""` to `null` here restores byte-exact parity at all seven at once.
    // Nothing anywhere compares these against `null` (no `=== null` / `!= null` on
    // title/artist/album in `src/`), so `""` and `null` are interchangeable to every
    // reader — which is exactly what makes fixing it at the boundary safe.
    title: s.title || null,
    artist: s.artist || null,
    album: s.album || null,
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
/**
 * The same safety stop for the track index, and also a real memory ceiling: unlike albums,
 * every entry here becomes a `Track` object held for the session. 100k tracks (~200 pages)
 * is far beyond any real library; a server that never returns an empty page stops here.
 */
export const MAX_SONGS = 100_000;

/**
 * The library state a freshly-connected server starts from.
 *
 * Spread into EVERY block that flips `connected: true`, and into `disconnect`. This exists as
 * one function because it used to be an inline list duplicated across `connect` and
 * `addAndConnect`, and adding the track index to only one of them meant a second server
 * silently inherited the first server's tracks — `songsLoaded` was already true, so no walk
 * ran to correct it. A new object each call: these are state values, never shared.
 */
function freshLibrary() {
  return {
    songs: [] as Track[],
    songsLoading: false,
    songsLoaded: false,
    searchResults: null,
    searching: false,
  };
}

/** Bumped on every connect/disconnect so a slow in-flight page load can detect it's stale. */
let loadGen = 0;
/** Bumped on every search so a slow in-flight search can detect it's been superseded. */
let searchGen = 0;

/**
 * What the store remembers about the connected server — deliberately **not**
 * `SubsonicConfig`.
 *
 * The password used to live here for the lifetime of the session, in a zustand store any
 * component could read and any devtools snapshot would capture. It now goes straight from
 * the Keychain (or the connect form) into `setConfig`, which hands it to Rust; nothing
 * keeps a copy on this side. These two fields are all any consumer ever read.
 */
export interface ServerIdentity {
  baseUrl: string;
  username: string;
}

interface SubsonicState {
  connected: boolean;
  status: "idle" | "connecting" | "error";
  error: string | null;
  /** The active server's non-secret identity, or `null` when disconnected. */
  config: ServerIdentity | null;
  albums: SubAlbum[];
  playlists: SubPlaylist[];
  /** True while additional album pages are still streaming in behind the first page. */
  albumsLoading: boolean;

  /**
   * The full server track index, built lazily by `loadSongs`. Empty until the Tracks
   * section is first opened — see `loadSongs` for why it isn't fetched on connect.
   */
  songs: Track[];
  /** True while the track-index walk is still streaming pages in. */
  songsLoading: boolean;
  /**
   * True once a walk has finished for this server. Distinct from `songs.length > 0`
   * because an EMPTY index is a real, final answer: it's what a server that doesn't
   * support the empty-query trick returns, and it must not re-trigger the walk forever.
   */
  songsLoaded: boolean;

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
  /** Async because clearing the config is now an IPC round-trip: callers that
   *  immediately reconnect MUST await it, or the clear can land after the connect. */
  disconnect: () => Promise<void>;

  // ── Server list management ─────────────────────────────────────────────────
  /** Add a new server (after a successful connection via ConnectPanel). */
  addAndConnect: (name: string | undefined, cfg: SubsonicConfig) => Promise<boolean>;
  removeServer: (id: string) => Promise<void>;
  renameServer: (id: string, name: string) => void;
  switchServer: (id: string) => Promise<void>;
  refreshServerList: () => void;
  setManageOpen: (open: boolean) => void;

  /**
   * Build the full track index. Idempotent — a no-op once loaded or in flight.
   */
  loadSongs: () => Promise<void>;

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
export async function walkPages<T>(
  fetchPage: (offset: number) => Promise<T[]>,
  {
    first = [],
    onPage,
    isStale,
    max = MAX_ALBUMS,
  }: {
    first?: T[];
    onPage?: (all: T[]) => void;
    isStale?: () => boolean;
    max?: number;
  } = {},
): Promise<T[]> {
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
 * The album-typed walk: [`walkPages`] with the album cap bound. Kept as its own name because
 * the two callers want different caps (albums are cheap rows, songs become `Track` objects),
 * and because that is what `albumPaging.test.ts` pins.
 */
export const walkAlbumPages = (
  fetchPage: (offset: number) => Promise<SubAlbum[]>,
  opts: {
    first?: SubAlbum[];
    onPage?: (all: SubAlbum[]) => void;
    isStale?: () => boolean;
    max?: number;
  } = {},
): Promise<SubAlbum[]> => walkPages(fetchPage, { max: MAX_ALBUMS, ...opts });

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
  songs: [],
  songsLoading: false,
  songsLoaded: false,
  playlists: [],
  albumsLoading: false,
  searchResults: null,
  searching: false,
  serverList: getServerList(),
  manageOpen: false,

  connect: async (cfg) => {
    set({ status: "connecting", error: null });
    const gen = ++loadGen;
    try {
      // Awaited, and inside the try: every subsequent call reaches the same Rust client,
      // so a `subsonic_ping` issued before the config landed would fail as "not
      // configured". `Client::new` does NOT parse the base URL — an earlier version of
      // this comment claimed it could fail on a malformed one; it cannot. The only way
      // `build()` fails is TLS-backend initialisation, and a malformed base URL surfaces
      // at `ping()` below. The await placement stands on the ordering reason alone.
      // This is also the last time `cfg.password` is touched on this side.
      await setConfig(cfg);
      setStreamOrigin(cfg.baseUrl); // allow the proxy to fetch this server before any cover art
      await ping();
      const albums = await fetchFirstAlbumPage();
      if (gen !== loadGen) return false; // superseded while we were connecting
      // Paint immediately on page 1, then stream the rest in behind it.
      set({
        connected: true,
        status: "idle",
        config: { baseUrl: cfg.baseUrl, username: cfg.username },
        albums,
        error: null,
        ...freshLibrary(),
        // Any non-empty first page means "there might be more" — we can't know until we probe.
        // Deliberately NOT `>= PAGE_SIZE`: a server that caps pages below what we asked for
        // would leave this false while the walk silently pulled thousands more in the
        // background, which is the same wrong assumption walkAlbumPages exists to avoid.
        albumsLoading: albums.length > 0,
      });
      void loadRemainingAlbums(gen, albums, set);
      getPlaylists()
        .then((playlists) => set({ playlists }))
        .catch(() => {
          /* ignore */
        });
      return true;
    } catch (e) {
      // Superseded while we were failing: a newer connect owns the Rust config and the
      // store now, so touch neither. Clearing here would unconfigure the *winner* —
      // this became reachable the moment the clear grew an `await`, because a loser's
      // teardown can now interleave with a winner's setup rather than running to
      // completion synchronously. The success path has always had this guard.
      if (gen !== loadGen) return false;
      await setConfig(null).catch(() => {
        /* clearing must not mask the original failure */
      });
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
    // The one place a password still passes through TypeScript: Keychain → `connect` →
    // `setConfig` → Rust, as a local that is never stored, logged or put in the store.
    // Removing even this transit needs a Rust-side "configure from Keychain key" command,
    // which is a backend change and out of this task's scope.
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

  disconnect: async () => {
    // Awaited by callers that reconnect straight afterwards (`switchServer`,
    // `removeServer`): both this and the following `connect` write the same Rust slot,
    // and an un-awaited clear could land last and unconfigure the new server.
    await setConfig(null).catch(() => {
      /* already unconfigured is not a failure */
    });
    setStreamOrigin(null);
    loadGen++; // abandon any in-flight page load
    searchGen++; // and any in-flight search
    set({
      connected: false,
      status: "idle",
      config: null,
      albums: [],
      albumsLoading: false,
      ...freshLibrary(),
    });
  },

  addAndConnect: async (name, cfg) => {
    set({ status: "connecting", error: null });
    const gen = ++loadGen;
    try {
      // See `connect` — awaited, and inside the try, for the same two reasons.
      await setConfig(cfg);
      setStreamOrigin(cfg.baseUrl);
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
        config: { baseUrl: cfg.baseUrl, username: cfg.username },
        albums,
        error: null,
        serverList: list,
        ...freshLibrary(),
        // Any non-empty first page means "there might be more" — we can't know until we probe.
        // Deliberately NOT `>= PAGE_SIZE`: a server that caps pages below what we asked for
        // would leave this false while the walk silently pulled thousands more in the
        // background, which is the same wrong assumption walkAlbumPages exists to avoid.
        albumsLoading: albums.length > 0,
      });
      void loadRemainingAlbums(gen, albums, set);
      getPlaylists()
        .then((playlists) => set({ playlists }))
        .catch(() => {
          /* ignore */
        });
      return true;
    } catch (e) {
      // Same guard, same reason, as `connect`'s catch — this function is its twin and
      // carries the identical interleaving hazard.
      if (gen !== loadGen) return false;
      await setConfig(null).catch(() => {
        /* clearing must not mask the original failure */
      });
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
      await get().disconnect();
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
    await get().disconnect();
    setActiveServerId(id);
    set({ serverList: getServerList() });
    await get().connectById(id);
  },

  refreshServerList: () => {
    set({ serverList: getServerList() });
  },

  setManageOpen: (open) => set({ manageOpen: open }),

  loadSongs: async () => {
    // Lazy, not eager: the walk is one request for a small library but ~200 for a huge one,
    // and most sessions never open Tracks. `songsLoaded` (not `songs.length`) is the guard,
    // so a legitimately empty index isn't retried on every visit.
    if (get().songsLoading || get().songsLoaded) return;
    const gen = loadGen;
    set({ songsLoading: true });
    // Converted incrementally: `walkPages` hands back the cumulative array each page, so
    // re-mapping all of it per page would be quadratic on a large library.
    const tracks: Track[] = [];
    const absorb = (all: SubSong[]) => {
      for (let i = tracks.length; i < all.length; i++) tracks.push(toTrack(all[i]));
      set({ songs: [...tracks] });
    };
    try {
      const first = await getSongs(PAGE_SIZE, 0);
      if (gen !== loadGen) return; // server switched while page 1 was in flight
      absorb(first);
      await walkPages((offset) => getSongs(PAGE_SIZE, offset), {
        first,
        max: MAX_SONGS,
        isStale: () => gen !== loadGen,
        onPage: absorb,
      });
    } catch {
      // Keep whatever pages landed — a partial index beats an error screen, exactly as the
      // album walk does. `songsLoaded` still flips below so a failed walk shows the empty
      // state rather than a spinner that never resolves.
    }
    if (gen !== loadGen) return;
    set({ songsLoading: false, songsLoaded: true });
  },

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
