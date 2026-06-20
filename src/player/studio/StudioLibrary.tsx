import { formatTime, trackLabel } from "../../lib/format";
import { LocalCover } from "../LocalCover";
import { Marquee } from "../Marquee";
import { useContextMenu } from "../ContextMenu";
import { useLibrary, type LibraryCard } from "../../hooks/useLibrary";
import "./studio.css";

/**
 * Studio's OWN library renderer (theme-owned pixels) over the shared `useLibrary()` brain —
 * NOT Porcelain restyled. Album wall as framed prints with mono catalogue labels, matte
 * device lists, in the approved Studio concept. Same data + actions as Porcelain's
 * `LibraryView`; entirely different surface. This is the payoff of the Phase-1 hook extraction.
 */
export function StudioLibrary() {
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

  const coverArt = (c: LibraryCard) =>
    c.cover ? <img src={c.cover} alt="" /> : c.localPath ? <LocalCover path={c.localPath} /> : null;

  // ---- empty states ----
  if (source === "server" && !connected)
    return (
      <div className="sl">
        <div className="sl-empty">
          Connect to your Navidrome server to browse your library.
          <div className="mono" style={{ marginTop: 8 }}>
            No source · standby
          </div>
        </div>
      </div>
    );
  if (source === "local" && !localRoot)
    return (
      <div className="sl">
        <div className="sl-empty">
          No local folder yet.
          <div className="mono" style={{ marginTop: 10 }}>
            <span className="sl-btn ghost" onClick={lib.pickFolder}>
              Choose music folder
            </span>
          </div>
        </div>
      </div>
    );
  if (source === "local" && localStatus === "scanning")
    return (
      <div className="sl">
        <div className="sl-empty">
          <span className="mono">Scanning · {localRoot}</span>
        </div>
      </div>
    );

  // ---- album detail ----
  if (detail) {
    return (
      <div className="sl">
        <div className="sl-scroll">
          <div className="sl-back" onClick={lib.closeDetail}>
            ‹ {detail.from ?? "Albums"}
          </div>
          <div className="sl-detail-head">
            <div className="sl-art">
              {detail.cover ? (
                <img src={detail.cover} alt="" />
              ) : detail.coverPath ? (
                <LocalCover path={detail.coverPath} />
              ) : null}
            </div>
            <div className="sl-detail-meta">
              <div className="t">{detail.name}</div>
              <div className="a">{detail.artist}</div>
              <div className="sub">
                {detail.tracks.length} tracks ·{" "}
                {formatTime(detail.tracks.reduce((a, t) => a + (t.duration || 0), 0))}
              </div>
              <div className="sl-actions">
                <button className="sl-btn" onClick={() => lib.playDetail(0)}>
                  ▸ Play all
                </button>
                <button className="sl-btn ghost" onClick={lib.playDetailNext}>
                  Play next
                </button>
                <button className="sl-btn ghost" onClick={lib.addDetailToQueue}>
                  + Queue
                </button>
              </div>
            </div>
          </div>
          <div className="sl-list">
            {detail.tracks.map((t, i) => (
              <div
                key={t.id}
                className={`sl-row${t.id === curId ? " playing" : ""}`}
                onClick={() => lib.playFrom(detail.tracks, i)}
                onContextMenu={trackMenu(detail.tracks, i)}
              >
                <span className="n">{String(i + 1).padStart(2, "0")}</span>
                <span className="tt">{trackLabel(t)}</span>
                <span className="du">{formatTime(t.duration)}</span>
              </div>
            ))}
          </div>
        </div>
        {menu}
      </div>
    );
  }

  const sortBar = (
    <div className="sl-bar">
      <span className="lbl">Sort</span>
      {(
        [
          ["artist", "Artist"],
          ["name", "Title"],
          ["year", "Year"],
        ] as const
      ).map(([k, label]) => (
        <button key={k} className={`sl-pill${sort === k ? " on" : ""}`} onClick={() => setSort(k)}>
          {label}
        </button>
      ))}
    </div>
  );

  const albumWall = (list: LibraryCard[]) =>
    list.length === 0 ? (
      <div className="sl-empty">
        <span className="mono">{query ? "No matches" : "No albums"}</span>
      </div>
    ) : (
      <div className="sl-grid">
        {menu}
        {lib.sortCards(list).map((c) => (
          <div
            key={c.id}
            className="sl-album"
            onClick={() => void lib.openAlbum(c.id)}
            onContextMenu={albumMenu(c)}
          >
            <div className="sl-cover">
              {coverArt(c)}
              {c.year ? <span className="sl-cat">{c.year}</span> : null}
              <span className="sl-nowtag">
                <i />
                <i />
                <i />
              </span>
            </div>
            <div className="sl-meta">
              <Marquee className="sl-at" text={c.name} hover />
              <div className="sl-aa">{c.artist}</div>
            </div>
          </div>
        ))}
      </div>
    );

  // ---- ARTISTS ----
  if (section === "artists") {
    if (artist) {
      const list = cards.filter((c) => c.artist === artist);
      return (
        <div className="sl">
          <div className="sl-scroll">
            <div className="sl-back" onClick={lib.closeArtist}>
              ‹ Artists
            </div>
            <div className="sl-head">
              <span className="sl-title">{artist}</span>
              <span className="sl-count">
                {list.length} {list.length === 1 ? "album" : "albums"}
              </span>
            </div>
            {albumWall(list)}
          </div>
        </div>
      );
    }
    const list = query ? artists.filter((a) => a.name.toLowerCase().includes(query)) : artists;
    return (
      <div className="sl">
        <div className="sl-scroll">
          {list.length === 0 ? (
            <div className="sl-empty">
              <span className="mono">{query ? "No matches" : "No artists"}</span>
            </div>
          ) : (
            <div className="sl-list">
              {list.map((a) => (
                <div key={a.name} className="sl-row" onClick={() => lib.openArtist(a.name)}>
                  <span className="tt">{a.name}</span>
                  <span className="du">
                    {a.n} {a.n === 1 ? "album" : "albums"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    );
  }

  // ---- TRACKS ----
  if (section === "tracks") {
    if (!capabilities.tracksIndex)
      return (
        <div className="sl">
          <div className="sl-empty">
            Browse by album, or use search — a full track index is coming.
          </div>
        </div>
      );
    const tracks = query
      ? tracksIndex.filter((t) => trackLabel(t).toLowerCase().includes(query))
      : tracksIndex;
    return (
      <div className="sl">
        <div className="sl-scroll">
          {tracks.length === 0 ? (
            <div className="sl-empty">
              <span className="mono">{query ? "No matches" : "No tracks"}</span>
            </div>
          ) : (
            <div className="sl-list">
              {tracks.map((t, i) => (
                <div
                  key={t.id}
                  className={`sl-row${t.id === curId ? " playing" : ""}`}
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
      </div>
    );
  }

  // ---- FOLDERS ----
  if (section === "folders") {
    if (!capabilities.folders)
      return (
        <div className="sl">
          <div className="sl-empty">
            Your server organises music by tags — browse via Albums or Artists.
          </div>
        </div>
      );
    const list = query ? folders.filter((f) => f.name.toLowerCase().includes(query)) : folders;
    return (
      <div className="sl">
        <div className="sl-scroll">
          {list.length === 0 ? (
            <div className="sl-empty">
              <span className="mono">{query ? "No matches" : "No folders"}</span>
            </div>
          ) : (
            <div className="sl-list">
              {list.map((f) => (
                <div key={f.path} className="sl-row" onClick={() => lib.openFolder(f)}>
                  <span className="tt">{f.name}</span>
                  <span className="du">{f.tracks.length} tracks</span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    );
  }

  // ---- PLAYLISTS ----
  if (section === "playlists") {
    if (!capabilities.playlists)
      return (
        <div className="sl">
          <div className="sl-empty">Playlists for local files are coming soon.</div>
        </div>
      );
    const list = query ? playlists.filter((p) => p.name.toLowerCase().includes(query)) : playlists;
    return (
      <div className="sl">
        <div className="sl-scroll">
          {list.length === 0 ? (
            <div className="sl-empty">
              <span className="mono">{query ? "No matches" : "No playlists"}</span>
            </div>
          ) : (
            <div className="sl-list">
              {list.map((p) => (
                <div key={p.id} className="sl-row" onClick={() => void lib.openPlaylist(p.id)}>
                  <span className="tt">{p.name}</span>
                  <span className="du">{p.songCount ?? ""}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    );
  }

  // ---- ALBUMS (default) ----
  const list = query ? cards.filter(lib.matchesQuery) : cards;
  return (
    <div className="sl">
      <div className="sl-scroll">
        {cards.length > 1 && sortBar}
        {albumWall(list)}
      </div>
    </div>
  );
}
