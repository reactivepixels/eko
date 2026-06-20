import { useEffect, useMemo, useState } from "react";
import { useUiStore } from "../store/useUiStore";
import { useSubsonic } from "../subsonic/useSubsonic";
import { useLocal } from "../local/useLocal";
import { usePlayerStore } from "../store/usePlayerStore";
import { coverArtUrl } from "../subsonic/client";
import { formatTime, trackLabel } from "../lib/format";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";
import { useContextMenu } from "./ContextMenu";
import type { Track } from "../types";

interface Card {
  id: string;
  name: string;
  artist: string;
  sub: string;
  cover: string | null;
  localPath?: string;
  year?: number;
}
interface Detail {
  name: string;
  artist: string;
  cover: string | null;
  coverPath?: string;
  tracks: Track[];
  from?: string;
}

function playFrom(tracks: Track[], i: number) {
  const p = usePlayerStore.getState();
  p.setQueue(tracks, false);
  void p.playAt(i);
}

export function LibraryView() {
  const source = useUiStore((s) => s.source);
  const libSection = useUiStore((s) => s.libSection);
  const librarySort = useUiStore((s) => s.librarySort);
  const setLibrarySort = useUiStore((s) => s.setLibrarySort);
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
  const curId = curIdx !== null ? queue[curIdx]?.id : undefined;

  const [detail, setDetail] = useState<Detail | null>(null);
  const [artist, setArtist] = useState<string | null>(null);
  const { open: openMenu, menu } = useContextMenu();
  useEffect(() => {
    setDetail(null);
    setArtist(null);
  }, [source, libSection]);

  // Resolve a card's full track list on demand (server fetch / local lookup).
  const tracksForCard = async (c: Card): Promise<Track[]> => {
    if (source === "server") return (await useSubsonic.getState().openAlbum(c.id)).tracks;
    return useLocal.getState().openAlbum(c.id)?.tracks ?? [];
  };
  // Right-click an album card → play / queue the whole album.
  const albumMenu = (c: Card) =>
    openMenu([
      { label: "Play album", onSelect: () => void tracksForCard(c).then((t) => playFrom(t, 0)) },
      {
        label: "Play next",
        onSelect: () => void tracksForCard(c).then((t) => usePlayerStore.getState().playNext(t)),
      },
      {
        label: "Add to queue",
        onSelect: () => void tracksForCard(c).then((t) => usePlayerStore.getState().addToQueue(t)),
      },
    ]);
  // Right-click a track row → play it now / queue just that track.
  const trackMenu = (tracks: Track[], i: number) =>
    openMenu([
      { label: "Play", onSelect: () => playFrom(tracks, i) },
      { label: "Play next", onSelect: () => usePlayerStore.getState().playNext([tracks[i]]) },
      { label: "Add to queue", onSelect: () => usePlayerStore.getState().addToQueue([tracks[i]]) },
    ]);

  const cards: Card[] = useMemo(() => {
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
  const folders = useMemo(() => {
    if (source !== "local") return [] as { path: string; name: string; tracks: Track[] }[];
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

  // ---- empty states ----
  if (source === "server" && !connected)
    return <div className="empty">Connect to your Navidrome server to browse your library.</div>;
  if (source === "local" && !localRoot) {
    return (
      <div className="empty-state">
        <div className="es-icon">
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M3 7l2-2h5l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7z" />
            <path d="M11 18v-4.5l4-1V17" />
            <circle cx="9.6" cy="18" r="1.4" />
            <circle cx="13.6" cy="17" r="1.4" />
          </svg>
        </div>
        <div className="es-title">No local folder yet</div>
        <div className="es-sub">
          Point Eko at a folder of music on your Mac or an external drive — it'll scan the tags and
          build your library.
        </div>
        <div className="btn" onClick={() => void useLocal.getState().pickFolder()}>
          Choose music folder…
        </div>
      </div>
    );
  }
  if (source === "local" && localStatus === "scanning")
    return <div className="empty">Scanning {localRoot}…</div>;

  // ---- album detail ----
  if (detail) {
    return (
      <div className="view detail">
        <div className="back" onClick={() => setDetail(null)}>
          ‹ {detail.from ?? "Albums"}
        </div>
        <div className="detail-head">
          <div className="art">
            {detail.cover ? (
              <img src={detail.cover} alt="" />
            ) : detail.coverPath ? (
              <LocalCover path={detail.coverPath} />
            ) : null}
          </div>
          <div>
            <h2>{detail.name}</h2>
            <div className="meta">
              {detail.artist} · {detail.tracks.length} tracks ·{" "}
              {formatTime(detail.tracks.reduce((a, t) => a + (t.duration || 0), 0))}
            </div>
            <div className="detail-actions">
              <div className="btn" onClick={() => playFrom(detail.tracks, 0)}>
                ▸ Play all
              </div>
              <div
                className="btn ghost"
                onClick={() => usePlayerStore.getState().playNext(detail.tracks)}
                title="Play after the current track"
              >
                Play Next
              </div>
              <div
                className="btn ghost"
                onClick={() => usePlayerStore.getState().addToQueue(detail.tracks)}
                title="Add to the end of the queue"
              >
                + Queue
              </div>
            </div>
          </div>
        </div>
        <div className="tracklist">
          {detail.tracks.map((t, i) => (
            <div
              key={t.id}
              className={`trow${t.id === curId ? " playing" : ""}`}
              onClick={() => playFrom(detail.tracks, i)}
              onContextMenu={trackMenu(detail.tracks, i)}
            >
              <span className="n">{String(i + 1).padStart(2, "0")}</span>
              <span className="tt">{trackLabel(t)}</span>
              <span className="du">{formatTime(t.duration)}</span>
            </div>
          ))}
        </div>
        {menu}
      </div>
    );
  }

  const sortCards = (list: Card[]) => {
    const s = [...list];
    if (librarySort === "name") s.sort((a, b) => a.name.localeCompare(b.name));
    else if (librarySort === "year")
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

  const sortBar = (
    <div className="lib-bar">
      <span className="lib-sort-lbl">SORT</span>
      {(
        [
          ["artist", "Artist"],
          ["name", "Title"],
          ["year", "Year"],
        ] as const
      ).map(([k, label]) => (
        <button
          key={k}
          className={`seg-btn${librarySort === k ? " on" : ""}`}
          onClick={() => setLibrarySort(k)}
        >
          {label}
        </button>
      ))}
    </div>
  );

  const albumGrid = (list: Card[]) =>
    list.length === 0 ? (
      <div className="empty">{query ? "No matches." : "No albums found."}</div>
    ) : (
      <div className="grid">
        {menu}
        {sortCards(list).map((c) => (
          <div
            key={c.id}
            className="card"
            onClick={() => void openAlbum(c.id)}
            onContextMenu={albumMenu(c)}
          >
            <div className="cover">
              {c.cover ? (
                <img src={c.cover} alt="" />
              ) : c.localPath ? (
                <LocalCover path={c.localPath} />
              ) : null}
            </div>
            <Marquee className="ct" text={c.name} hover />
            <div className="ca">{c.artist}</div>
          </div>
        ))}
      </div>
    );

  // ---- ARTISTS ----
  if (libSection === "artists") {
    if (artist) {
      const list = cards.filter((c) => c.artist === artist);
      return (
        <div className="view">
          <div className="back" onClick={() => setArtist(null)}>
            ‹ Artists
          </div>
          <div className="lib-head">
            <h2 className="lib-title">{artist}</h2>
            <span className="lib-count">
              {list.length} {list.length === 1 ? "album" : "albums"}
            </span>
          </div>
          {albumGrid(list)}
        </div>
      );
    }
    const counts = new Map<string, number>();
    for (const c of cards) counts.set(c.artist, (counts.get(c.artist) ?? 0) + 1);
    let artists = [...counts.entries()]
      .map(([name, n]) => ({ name, n }))
      .sort((a, b) => a.name.localeCompare(b.name));
    if (query) artists = artists.filter((a) => a.name.toLowerCase().includes(query));
    return (
      <div className="view">
        {artists.length === 0 ? (
          <div className="empty">{query ? "No matches." : "No artists found."}</div>
        ) : (
          <div className="tracklist">
            {artists.map((a) => (
              <div key={a.name} className="trow" onClick={() => setArtist(a.name)}>
                <span className="tt">{a.name}</span>
                <span className="du">
                  {a.n} {a.n === 1 ? "album" : "albums"}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  // ---- TRACKS ----
  if (libSection === "tracks") {
    if (source === "server")
      return (
        <div className="empty">Browse by album, or use search — a full track index is coming.</div>
      );
    let tracks = localAlbums.flatMap((a) => a.tracks);
    if (query) tracks = tracks.filter((t) => trackLabel(t).toLowerCase().includes(query));
    return (
      <div className="view">
        {tracks.length === 0 ? (
          <div className="empty">{query ? "No matches." : "No tracks."}</div>
        ) : (
          <div className="tracklist">
            {tracks.map((t, i) => (
              <div
                key={t.id}
                className={`trow${t.id === curId ? " playing" : ""}`}
                onClick={() => playFrom(tracks, i)}
                onContextMenu={trackMenu(tracks, i)}
              >
                <span className="tt">{trackLabel(t)}</span>
                <span className="du">{formatTime(t.duration)}</span>
              </div>
            ))}
            {menu}
          </div>
        )}
      </div>
    );
  }

  // ---- FOLDERS ----
  if (libSection === "folders") {
    if (source === "server")
      return (
        <div className="empty">
          Your server organises music by tags — browse via Albums or Artists.
        </div>
      );
    let list = folders;
    if (query) list = list.filter((f) => f.name.toLowerCase().includes(query));
    return (
      <div className="view">
        {list.length === 0 ? (
          <div className="empty">{query ? "No matches." : "No folders."}</div>
        ) : (
          <div className="tracklist">
            {list.map((f) => (
              <div
                key={f.path}
                className="trow"
                onClick={() =>
                  setDetail({
                    name: f.name,
                    artist: "Folder",
                    cover: null,
                    coverPath: f.tracks[0]?.path,
                    tracks: f.tracks,
                    from: "Folders",
                  })
                }
              >
                <span className="tt">{f.name}</span>
                <span className="du">{f.tracks.length} tracks</span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  // ---- PLAYLISTS ----
  if (libSection === "playlists") {
    if (source === "local")
      return <div className="empty">Playlists for local files are coming soon.</div>;
    let list = playlists;
    if (query) list = list.filter((p) => p.name.toLowerCase().includes(query));
    return (
      <div className="view">
        {list.length === 0 ? (
          <div className="empty">{query ? "No matches." : "No playlists on your server."}</div>
        ) : (
          <div className="tracklist">
            {list.map((p) => (
              <div key={p.id} className="trow" onClick={() => void openPlaylist(p.id)}>
                <span className="tt">{p.name}</span>
                <span className="du">
                  {p.songCount ?? ""} {p.songCount ? "tracks" : ""}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  // ---- ALBUMS (default) ----
  const list = query
    ? cards.filter(
        (c) => c.name.toLowerCase().includes(query) || c.artist.toLowerCase().includes(query),
      )
    : cards;
  return (
    <div className="view">
      {cards.length > 1 && sortBar}
      {albumGrid(list)}
    </div>
  );
}
