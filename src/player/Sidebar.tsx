import { type ReactNode } from "react";
import { useUiStore, type LibSection } from "../store/useUiStore";
import { useMusicSource } from "../hooks/useMusicSource";

const ICON: Record<string, ReactNode> = {
  albums: (
    <>
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </>
  ),
  artists: (
    <>
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21c0-4 4-6 8-6s8 2 8 6" />
    </>
  ),
  tracks: <path d="M4 6h16M4 12h16M4 18h10" />,
  folders: <path d="M3 7l2-2h5l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V7z" />,
  playlists: <path d="M4 6h16M4 12h16M4 18h16" />,
  deck: (
    <>
      <path d="M9 18V5l12-2v13" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="18" cy="16" r="3" />
    </>
  ),
};

function Item({
  id,
  label,
  onClick,
  active,
}: {
  id: string;
  label: string;
  onClick: () => void;
  active: boolean;
}) {
  return (
    <div className={`nav-item${active ? " active" : ""}`} onClick={onClick}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        {ICON[id]}
      </svg>
      {label}
    </div>
  );
}

export function Sidebar() {
  const { playerView, libSection, setPlayerView, setLibSection } = useUiStore();
  const src = useMusicSource();
  const { source, serverConfigured, localRoot } = src;

  const go = (s: LibSection) => {
    setLibSection(s);
    setPlayerView("library");
  };
  const isLib = (s: LibSection) => playerView === "library" && libSection === s;

  const sections: [LibSection, string][] = [
    ["albums", "Albums"],
    ["artists", "Artists"],
    ["tracks", "Tracks"],
    ["folders", "Folders"],
  ];

  const out =
    source === "server"
      ? {
          t: "OUTPUT",
          n: serverConfigured ? "Topping E50 · USB" : "No device",
          s: serverConfigured ? "EXCLUSIVE · 768k" : "—",
        }
      : { t: "OUTPUT", n: "System default", s: localRoot ? `LOCAL · ${localRoot}` : "Bit-perfect" };

  return (
    <aside className="side">
      <div className="nav-grp">
        <div className="h">LIBRARY</div>
        {sections.map(([id, label]) => (
          <Item key={id} id={id} label={label} active={isLib(id)} onClick={() => go(id)} />
        ))}
      </div>
      <div className="nav-grp">
        <div className="h">YOURS</div>
        <Item
          id="playlists"
          label="Playlists"
          active={isLib("playlists")}
          onClick={() => go("playlists")}
        />
        <Item
          id="deck"
          label="Now Playing"
          active={playerView === "deck"}
          onClick={() => setPlayerView("deck")}
        />
      </div>
      <div className="nav-spacer" />
      {source === "local" && localRoot && (
        <div className="dac-card">
          <div className="t">MUSIC FOLDER</div>
          <div className="n">{localRoot}</div>
          <div className="folder-actions">
            <span onClick={src.rescan}>Rescan</span>
            <span onClick={src.changeFolder}>Change</span>
            <span onClick={src.clearFolder}>Clear</span>
          </div>
        </div>
      )}
      <div className="dac-card">
        <div className="t">{out.t}</div>
        <div className="n">{out.n}</div>
        <div className="s">{out.s}</div>
      </div>
    </aside>
  );
}
