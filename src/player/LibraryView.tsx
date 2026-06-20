import { formatTime, trackLabel } from "../lib/format";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";
import { useContextMenu } from "./ContextMenu";
import { useLibrary, type LibraryCard } from "../hooks/useLibrary";

/**
 * Porcelain library surface — pure presentation over the shared `useLibrary()` brain.
 * All source/normalisation/nav/menu logic lives in the hook; this file renders pixels and
 * never touches `useSubsonic` / `useLocal` directly (Phase 1 / Gate 2 of the theming plan).
 */
export function LibraryView() {
  const lib = useLibrary();
  const { open: openMenu, menu } = useContextMenu();

  const albumMenu = (c: LibraryCard) => openMenu(lib.albumMenuItems(c));
  const trackMenu = (tracks: Parameters<typeof lib.trackMenuItems>[0], i: number) =>
    openMenu(lib.trackMenuItems(tracks, i));

  const {
    source,
    section,
    sort,
    setSort,
    query,
    connected,
    localRoot,
    localStatus,
    capabilities,
    currentTrackId: curId,
    cards,
    folders,
    playlists,
    artists,
    tracksIndex,
    detail,
    artist,
  } = lib;

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
        <div className="btn" onClick={lib.pickFolder}>
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
        <div className="back" onClick={lib.closeDetail}>
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
              <div className="btn" onClick={() => lib.playDetail(0)}>
                ▸ Play all
              </div>
              <div
                className="btn ghost"
                onClick={lib.playDetailNext}
                title="Play after the current track"
              >
                Play Next
              </div>
              <div
                className="btn ghost"
                onClick={lib.addDetailToQueue}
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
              onClick={() => lib.playFrom(detail.tracks, i)}
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
        <button key={k} className={`seg-btn${sort === k ? " on" : ""}`} onClick={() => setSort(k)}>
          {label}
        </button>
      ))}
    </div>
  );

  const albumGrid = (list: LibraryCard[]) =>
    list.length === 0 ? (
      <div className="empty">{query ? "No matches." : "No albums found."}</div>
    ) : (
      <div className="grid">
        {menu}
        {lib.sortCards(list).map((c) => (
          <div
            key={c.id}
            className="card"
            onClick={() => void lib.openAlbum(c.id)}
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
  if (section === "artists") {
    if (artist) {
      const list = cards.filter((c) => c.artist === artist);
      return (
        <div className="view">
          <div className="back" onClick={lib.closeArtist}>
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
    const list = query ? artists.filter((a) => a.name.toLowerCase().includes(query)) : artists;
    return (
      <div className="view">
        {list.length === 0 ? (
          <div className="empty">{query ? "No matches." : "No artists found."}</div>
        ) : (
          <div className="tracklist">
            {list.map((a) => (
              <div key={a.name} className="trow" onClick={() => lib.openArtist(a.name)}>
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
  if (section === "tracks") {
    if (!capabilities.tracksIndex)
      return (
        <div className="empty">Browse by album, or use search — a full track index is coming.</div>
      );
    const tracks = query
      ? tracksIndex.filter((t) => trackLabel(t).toLowerCase().includes(query))
      : tracksIndex;
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
                onClick={() => lib.playFrom(tracks, i)}
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
  if (section === "folders") {
    if (!capabilities.folders)
      return (
        <div className="empty">
          Your server organises music by tags — browse via Albums or Artists.
        </div>
      );
    const list = query ? folders.filter((f) => f.name.toLowerCase().includes(query)) : folders;
    return (
      <div className="view">
        {list.length === 0 ? (
          <div className="empty">{query ? "No matches." : "No folders."}</div>
        ) : (
          <div className="tracklist">
            {list.map((f) => (
              <div key={f.path} className="trow" onClick={() => lib.openFolder(f)}>
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
  if (section === "playlists") {
    if (!capabilities.playlists)
      return <div className="empty">Playlists for local files are coming soon.</div>;
    const list = query ? playlists.filter((p) => p.name.toLowerCase().includes(query)) : playlists;
    return (
      <div className="view">
        {list.length === 0 ? (
          <div className="empty">{query ? "No matches." : "No playlists on your server."}</div>
        ) : (
          <div className="tracklist">
            {list.map((p) => (
              <div key={p.id} className="trow" onClick={() => void lib.openPlaylist(p.id)}>
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
  const list = query ? cards.filter(lib.matchesQuery) : cards;
  return (
    <div className="view">
      {cards.length > 1 && sortBar}
      {albumGrid(list)}
    </div>
  );
}
