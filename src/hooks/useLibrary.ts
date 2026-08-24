import { useEffect, useMemo, useState } from "react";
import { useUiStore } from "../store/useUiStore";
import { useSubsonic } from "../subsonic/useSubsonic";
import { useLocal } from "../local/useLocal";
import { usePlayerStore } from "../store/usePlayerStore";
import {
  useSmartPlaylistStore,
  useIsPro,
  offlineTrackMenuItems,
  offlineAlbumMenuItems,
} from "@pro";
import { coverAt } from "../subsonic/nativeSubsonic";
import type { Track } from "../types";
import type { MenuItem } from "../player/ContextMenu";

/**
 * Headless library logic — the shared "brain" for any theme's library surface
 * (Phase 1 of docs/skin-architecture.md). ALL `source === "server" | "local"` branching,
 * source-normalisation (server fetch vs local lookup, the server's `coverUrl` vs a local path), the
 * per-source capability flags, master/detail navigation state, and the play/queue menu
 * actions live HERE, once. A theme component consumes this and renders pixels only — it
 * never imports `useSubsonic` / `useLocal` (Gate 2).
 */

export interface LibraryCard {
  id: string;
  name: string;
  artist: string;
  sub: string;
  cover: string | null;
  localPath?: string;
  year?: number;
}
export interface LibraryDetail {
  name: string;
  artist: string;
  cover: string | null;
  coverPath?: string;
  tracks: Track[];
  from?: string;
}
export interface LibraryFolder {
  path: string;
  name: string;
  tracks: Track[];
}
export interface LibraryArtist {
  name: string;
  n: number;
}
/** What the current source can actually show — a theme can't silently lose a feature. */
export interface LibraryCapabilities {
  tracksIndex: boolean; // a flat all-tracks list (local always; server once indexed)
  folders: boolean; // browse by containing folder (local only)
  playlists: boolean; // server playlists (server only)
}

/** Replace the queue with `tracks` and start at `i` (shared by rows + menus). */
function playFrom(tracks: Track[], i: number) {
  const p = usePlayerStore.getState();
  p.setQueue(tracks, false);
  void p.playAt(i);
}

export function useLibrary() {
  const source = useUiStore((s) => s.source);
  const section = useUiStore((s) => s.libSection);
  const isPro = useIsPro();
  const sort = useUiStore((s) => s.librarySort);
  const setSort = useUiStore((s) => s.setLibrarySort);
  const query = useUiStore((s) => s.query)
    .trim()
    .toLowerCase();

  const subAlbums = useSubsonic((s) => s.albums);
  const playlists = useSubsonic((s) => s.playlists);
  const connected = useSubsonic((s) => s.connected);
  const albumsLoading = useSubsonic((s) => s.albumsLoading);
  const searchResults = useSubsonic((s) => s.searchResults);
  const searching = useSubsonic((s) => s.searching);
  const serverSongs = useSubsonic((s) => s.songs);
  const songsLoading = useSubsonic((s) => s.songsLoading);
  const songsLoaded = useSubsonic((s) => s.songsLoaded);

  /**
   * Debounced server-side search. Local libraries filter in memory (instant, no request), but a
   * server library can be far larger than what's loaded, and only `search3` can match song
   * titles at all.
   *
   * 350ms debounce so we don't fire a request per keystroke: Navidrome's search is genuinely
   * slow on large libraries, and un-debounced typing would queue a dozen expensive queries and
   * hammer the server. The store discards superseded responses, so late arrivals can't clobber
   * newer results.
   */
  useEffect(() => {
    if (source !== "server" || !connected) return;
    const { runSearch, clearSearch } = useSubsonic.getState();
    if (!query) {
      clearSearch();
      return;
    }
    const t = setTimeout(() => void runSearch(query), 350);
    return () => clearTimeout(t);
  }, [query, source, connected]);

  /**
   * Build the server's track index the first time Tracks is opened — not on connect.
   *
   * The walk is one request for a small library but ~200 for a very large one, and most
   * sessions never open this section, so paying for it up front would tax everyone for a
   * feature few use. `loadSongs` is itself idempotent, so re-entering the section (or a
   * re-render) costs nothing.
   */
  useEffect(() => {
    if (source !== "server" || !connected || section !== "tracks") return;
    void useSubsonic.getState().loadSongs();
  }, [source, connected, section]);

  /** True when the visible server list came from `search3` (already filtered by the server). */
  const serverSearchActive = source === "server" && !!query && searchResults !== null;
  const localAlbums = useLocal((s) => s.albums);
  const localStatus = useLocal((s) => s.status);
  const localRoot = useLocal((s) => s.rootName);

  const curIdx = usePlayerStore((s) => s.currentIndex);
  const queue = usePlayerStore((s) => s.tracks);
  const currentTrackId = curIdx !== null ? queue[curIdx]?.id : undefined;

  // Master/detail navigation lifts out of the renderer into this layer.
  const [detail, setDetail] = useState<LibraryDetail | null>(null);
  const [artist, setArtist] = useState<string | null>(null);
  useEffect(() => {
    setDetail(null);
    setArtist(null);
  }, [source, section]);

  const cards: LibraryCard[] = useMemo(() => {
    if (source === "server") {
      // While searching, show the server's matches instead of the (possibly partial) browse list.
      const list = serverSearchActive ? searchResults!.albums : subAlbums;
      return list.map((a) => ({
        id: a.id,
        name: a.name,
        artist: a.artist,
        year: a.year,
        sub: `${a.year ? a.year + " · " : ""}${a.songCount ?? ""} ${a.songCount ? "tracks" : ""}`.trim(),
        cover: coverAt(a.coverUrl, 300),
      }));
    }
    return localAlbums.map((a) => ({
      id: a.id,
      name: a.name,
      artist: a.artist,
      sub: `${a.songCount} tracks`,
      cover: null,
      localPath: a.tracks[0]?.path,
    }));
  }, [source, subAlbums, localAlbums, serverSearchActive, searchResults]);

  // Local tracks grouped by their containing folder.
  const folders: LibraryFolder[] = useMemo(() => {
    if (source !== "local") return [];
    const m = new Map<string, Track[]>();
    for (const a of localAlbums)
      for (const t of a.tracks) {
        const dir = t.path.slice(0, t.path.lastIndexOf("/"));
        const arr = m.get(dir) ?? [];
        arr.push(t);
        m.set(dir, arr);
      }
    return [...m.entries()]
      .map(([path, tracks]) => ({
        path,
        name: path.slice(path.lastIndexOf("/") + 1) || path,
        tracks,
      }))
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [source, localAlbums]);

  /**
   * Flat track index. Local = every scanned track. Server = the full index walked from
   * `search3` with an empty query (see `useSubsonic.loadSongs`), or the search hits while a
   * search is active — search wins because the server has already filtered, and it can match
   * titles beyond whatever the index holds.
   */
  const tracksIndex: Track[] = useMemo(() => {
    if (source === "local") return localAlbums.flatMap((a) => a.tracks);
    if (serverSearchActive) return searchResults!.tracks;
    return serverSongs;
  }, [source, localAlbums, serverSearchActive, searchResults, serverSongs]);

  // Artists derived from the album cards.
  const artists: LibraryArtist[] = useMemo(() => {
    const counts = new Map<string, number>();
    for (const c of cards) counts.set(c.artist, (counts.get(c.artist) ?? 0) + 1);
    return [...counts.entries()]
      .map(([name, n]) => ({ name, n }))
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [cards]);

  const capabilities: LibraryCapabilities = {
    // Local always has one. A server has one once the walk finds anything, and always while a
    // search is active. It stays FALSE for a server whose walk came back empty — that server
    // doesn't support the empty-query trick, and Tracks explains itself instead of pretending.
    tracksIndex: source === "local" || serverSearchActive || songsLoading || serverSongs.length > 0,
    folders: source === "local",
    playlists: source === "server",
  };

  // ---- navigation ----
  const openAlbum = async (id: string) => {
    if (source === "server") {
      const { album, tracks } = await useSubsonic.getState().openAlbum(id);
      setDetail({
        name: album.name,
        artist: album.artist,
        cover: coverAt(album.coverUrl, 600),
        tracks,
        from: artist ?? "Albums",
      });
    } else {
      const a = useLocal.getState().openAlbum(id);
      if (a)
        setDetail({
          name: a.name,
          artist: a.artist,
          cover: null,
          coverPath: a.tracks[0]?.path,
          tracks: a.tracks,
          from: artist ?? "Albums",
        });
    }
  };
  const openPlaylist = async (id: string) => {
    const { name, tracks } = await useSubsonic.getState().openPlaylist(id);
    setDetail({ name, artist: "Playlist", cover: null, tracks, from: "Playlists" });
  };
  const openFolder = (folder: LibraryFolder) => {
    setDetail({
      name: folder.name,
      artist: "Folder",
      cover: null,
      coverPath: folder.tracks[0]?.path,
      tracks: folder.tracks,
      from: "Folders",
    });
  };
  const openArtist = (name: string) => setArtist(name);
  const closeDetail = () => setDetail(null);
  const closeArtist = () => setArtist(null);
  /** Choose a local music folder (empty-state action). */
  const pickFolder = () => void useLocal.getState().pickFolder();

  // ---- selectors (pure; presentation calls these instead of re-deriving) ----
  const sortCards = (list: LibraryCard[]) => {
    const s = [...list];
    if (sort === "name") s.sort((a, b) => a.name.localeCompare(b.name));
    else if (sort === "year")
      s.sort((a, b) => (b.year ?? 0) - (a.year ?? 0) || a.artist.localeCompare(b.artist));
    else
      s.sort(
        (a, b) =>
          a.artist.localeCompare(b.artist) ||
          (a.year ?? 0) - (b.year ?? 0) ||
          a.name.localeCompare(b.name),
      );
    return s;
  };
  /**
   * Local filtering only. When the server did the filtering (`search3`), everything it returned
   * is a match by definition — re-filtering here would wrongly drop hits the server matched on
   * something we can't see (song title, genre, artist alias).
   */
  const matchesQuery = (c: LibraryCard) =>
    serverSearchActive ||
    !query ||
    c.name.toLowerCase().includes(query) ||
    c.artist.toLowerCase().includes(query);

  // ---- play / queue actions + context-menu item builders ----
  const tracksForCard = async (c: LibraryCard): Promise<Track[]> => {
    if (source === "server") return (await useSubsonic.getState().openAlbum(c.id)).tracks;
    return useLocal.getState().openAlbum(c.id)?.tracks ?? [];
  };
  const albumMenuItems = (c: LibraryCard): MenuItem[] => {
    const items: MenuItem[] = [
      { label: "Play album", onSelect: () => void tracksForCard(c).then((t) => playFrom(t, 0)) },
      {
        label: "Play next",
        onSelect: () => void tracksForCard(c).then((t) => usePlayerStore.getState().playNext(t)),
      },
      {
        label: "Add to queue",
        onSelect: () => void tracksForCard(c).then((t) => usePlayerStore.getState().addToQueue(t)),
      },
    ];
    // Instant Mix from album — Pro feature, server source only. No "· Pro" teaser in the
    // free build; the item simply isn't offered when unlicensed.
    if (source === "server" && isPro) {
      items.push({ separator: true });
      items.push({
        label: "Instant Mix from album",
        onSelect: () =>
          void tracksForCard(c).then((tracks) => {
            const seed = tracks[0];
            if (seed?.subsonicId) {
              void useSmartPlaylistStore.getState().instantMixFromTrack(seed.subsonicId, undefined);
            }
          }),
      });
    }
    // Offline caching, from `@pro` — `[]` in the free build, separator included.
    //
    // Server albums only, and that gate belongs here rather than in the helper: a local
    // album's card id addresses the local scan, so `getAlbum` (which the helper uses to
    // find the album's tracks and their download URLs) has nothing to look up. Same
    // free-side distinction Instant Mix draws just above.
    if (source === "server") {
      items.push(...offlineAlbumMenuItems(c.id));
    }
    return items;
  };
  const trackMenuItems = (tracks: Track[], i: number): MenuItem[] => {
    const track = tracks[i];
    const items: MenuItem[] = [
      { label: "Play", onSelect: () => playFrom(tracks, i) },
      { label: "Play next", onSelect: () => usePlayerStore.getState().playNext([tracks[i]]) },
      { label: "Add to queue", onSelect: () => usePlayerStore.getState().addToQueue([tracks[i]]) },
    ];
    // Instant Mix — Pro feature, server tracks only (needs subsonicId). No "· Pro" teaser
    // in the free build; the item simply isn't offered when unlicensed.
    if (source === "server" && track.subsonicId && isPro) {
      items.push({ separator: true });
      items.push({
        label: "Instant Mix from this track",
        onSelect: () =>
          void useSmartPlaylistStore.getState().instantMixFromTrack(track.subsonicId!, undefined),
      });
    }
    // Offline caching, from `@pro` — `[]` in the free build, separator included. No source
    // check needed: the helper omits the item for anything without a `subsonicId`, which is
    // exactly what a local file is.
    items.push(...offlineTrackMenuItems(track));
    return items;
  };

  const playDetail = (i: number) => detail && playFrom(detail.tracks, i);
  const playDetailNext = () => detail && usePlayerStore.getState().playNext(detail.tracks);
  const addDetailToQueue = () => detail && usePlayerStore.getState().addToQueue(detail.tracks);

  return {
    // state
    source,
    section,
    sort,
    setSort,
    query,
    connected,
    localRoot,
    localStatus,
    capabilities,
    currentTrackId,
    isPro,
    /** More album pages are still streaming in from the server. */
    albumsLoading,
    /** A server-side search request is in flight. */
    searching,
    /** The visible server list came from `search3`, not the browse list. */
    serverSearchActive,
    /** True while the server's track index is still being walked. */
    songsLoading,
    /** True once the index walk has settled — an empty index is a final answer. */
    songsLoaded,
    // data
    cards,
    folders,
    playlists,
    artists,
    tracksIndex,
    // nav
    detail,
    artist,
    openAlbum,
    openArtist,
    openPlaylist,
    openFolder,
    closeDetail,
    closeArtist,
    pickFolder,
    // selectors
    sortCards,
    matchesQuery,
    // actions / menus
    playFrom,
    albumMenuItems,
    trackMenuItems,
    playDetail,
    playDetailNext,
    addDetailToQueue,
  };
}
