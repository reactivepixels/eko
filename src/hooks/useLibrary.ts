import { useEffect, useMemo, useState } from "react";
import { useUiStore } from "../store/useUiStore";
import { useSubsonic } from "../subsonic/useSubsonic";
import { useLocal } from "../local/useLocal";
import { usePlayerStore } from "../store/usePlayerStore";
import { useSmartPlaylistStore, useIsPro } from "@pro";
import { coverArtUrl } from "../subsonic/client";
import type { Track } from "../types";
import type { MenuItem } from "../player/ContextMenu";

/**
 * Headless library logic — the shared "brain" for any theme's library surface
 * (Phase 1 of docs/skin-architecture.md). ALL `source === "server" | "local"` branching,
 * source-normalisation (server fetch vs local lookup, `coverArtUrl` vs local path), the
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
  tracksIndex: boolean; // a flat all-tracks list (local only)
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
      return subAlbums.map((a) => ({
        id: a.id,
        name: a.name,
        artist: a.artist,
        year: a.year,
        sub: `${a.year ? a.year + " · " : ""}${a.songCount ?? ""} ${a.songCount ? "tracks" : ""}`.trim(),
        cover: coverArtUrl(a.coverArt, 300),
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
  }, [source, subAlbums, localAlbums]);

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

  // Flat all-tracks index (local only).
  const tracksIndex: Track[] = useMemo(
    () => (source === "local" ? localAlbums.flatMap((a) => a.tracks) : []),
    [source, localAlbums],
  );

  // Artists derived from the album cards.
  const artists: LibraryArtist[] = useMemo(() => {
    const counts = new Map<string, number>();
    for (const c of cards) counts.set(c.artist, (counts.get(c.artist) ?? 0) + 1);
    return [...counts.entries()]
      .map(([name, n]) => ({ name, n }))
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [cards]);

  const capabilities: LibraryCapabilities = {
    tracksIndex: source === "local",
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
        cover: coverArtUrl(album.coverArt, 600),
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
  const matchesQuery = (c: LibraryCard) =>
    !query || c.name.toLowerCase().includes(query) || c.artist.toLowerCase().includes(query);

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
    // Instant Mix from album — use first track's similarity (server source only).
    if (source === "server") {
      items.push({ separator: true });
      if (isPro) {
        items.push({
          label: "Instant Mix from album",
          onSelect: () =>
            void tracksForCard(c).then((tracks) => {
              const seed = tracks[0];
              if (seed?.subsonicId) {
                void useSmartPlaylistStore
                  .getState()
                  .instantMixFromTrack(seed.subsonicId, undefined);
              }
            }),
        });
      } else {
        items.push({
          label: "Instant Mix · Pro",
          onSelect: () => undefined,
          disabled: true,
        });
      }
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
    // Instant Mix — only available for server tracks (need subsonicId for getSimilarSongs2).
    if (source === "server" && track.subsonicId) {
      items.push({ separator: true });
      if (isPro) {
        items.push({
          label: "Instant Mix from this track",
          onSelect: () =>
            void useSmartPlaylistStore.getState().instantMixFromTrack(track.subsonicId!, undefined),
        });
      } else {
        items.push({
          label: "Instant Mix · Pro",
          onSelect: () => undefined,
          disabled: true,
        });
      }
    }
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
