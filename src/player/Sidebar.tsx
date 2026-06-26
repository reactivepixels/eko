import { type ReactNode } from "react";
import { useUiStore, type LibSection } from "../store/useUiStore";
import { useMusicSource } from "../hooks/useMusicSource";
import { useIsPro, OfflinePanel } from "@pro";
import { UpdatePanel } from "./UpdatePanel";
import { UPDATER_ENABLED } from "../store/useUpdaterStore";
import { usePlayerStore } from "../store/usePlayerStore";

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
  "smart-playlists": (
    <>
      <path d="M4 6h16M4 12h10M4 18h6" />
      <circle cx="19" cy="17" r="3" />
      <path d="M19 15v2l1 1" />
    </>
  ),
  offline: (
    <>
      <path d="M5 12.5l4.5 4.5L19 7" />
    </>
  ),
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
    <div
      className={`nav-item${active ? " active" : ""}`}
      onClick={onClick}
      role="button"
      tabIndex={0}
      aria-current={active ? "page" : undefined}
      onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? onClick() : undefined)}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
        {ICON[id]}
      </svg>
      {label}
    </div>
  );
}

export function Sidebar() {
  const { playerView, libSection, setPlayerView, setLibSection } = useUiStore();
  const src = useMusicSource();
  const isPro = useIsPro();
  const { source, serverConfigured, localRoot, serverList, switchServer, openManageServers } = src;
  const scrobbleEnabled = usePlayerStore((s) => s.scrobbleEnabled);
  const setScrobbleEnabled = usePlayerStore((s) => s.setScrobbleEnabled);

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
        {/* Smart Playlists — Pro feature (creation gated); viewing stays accessible always
            when source is server. Show the nav item whenever server is connected. */}
        {source === "server" && (
          <Item
            id="smart-playlists"
            label="Smart Playlists"
            active={isLib("smart-playlists")}
            onClick={() => go("smart-playlists")}
          />
        )}
        {/* Offline is a Pro feature; show the item when Pro OR when already connected to Navidrome
            (allows browsing cached tracks even if license just lapsed). */}
        {(isPro || source === "server") && (
          <Item
            id="offline"
            label="Offline"
            active={isLib("offline")}
            onClick={() => go("offline")}
          />
        )}
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
            <span
              onClick={src.rescan}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? src.rescan() : undefined)}
            >
              Rescan
            </span>
            <span
              onClick={src.changeFolder}
              role="button"
              tabIndex={0}
              onKeyDown={(e) =>
                e.key === "Enter" || e.key === " " ? src.changeFolder() : undefined
              }
            >
              Change
            </span>
            <span
              onClick={src.clearFolder}
              role="button"
              tabIndex={0}
              onKeyDown={(e) =>
                e.key === "Enter" || e.key === " " ? src.clearFolder() : undefined
              }
            >
              Clear
            </span>
          </div>
        </div>
      )}
      <div className="dac-card">
        <div className="t">{out.t}</div>
        <div className="n">{out.n}</div>
        <div className="s">{out.s}</div>
      </div>
      {/* Server switcher — only shown in server mode when there is more than one server */}
      {source === "server" && serverList.servers.length > 0 && (
        <div className="dac-card">
          <div className="t">SERVER</div>
          {serverList.servers.length > 1 ? (
            <select
              style={{
                marginTop: 7,
                width: "100%",
                fontSize: 12,
                fontWeight: 600,
                fontFamily: "inherit",
                background: "var(--bg2)",
                color: "var(--ink)",
                border: "1px solid var(--line)",
                borderRadius: 6,
                padding: "5px 8px",
                cursor: "pointer",
              }}
              value={serverList.activeId ?? ""}
              onChange={(e) => {
                const id = e.target.value;
                if (id) void switchServer(id);
              }}
              aria-label="Switch server"
            >
              {serverList.servers.map((srv) => (
                <option key={srv.id} value={srv.id}>
                  {srv.name}
                </option>
              ))}
            </select>
          ) : (
            <div className="n">{serverList.servers[0]?.name ?? "—"}</div>
          )}
          <div className="folder-actions" style={{ marginTop: 8 }}>
            <span
              onClick={openManageServers}
              role="button"
              tabIndex={0}
              onKeyDown={(e) =>
                e.key === "Enter" || e.key === " " ? openManageServers() : undefined
              }
            >
              Manage
            </span>
          </div>
        </div>
      )}
      {/* Scrobble toggle — only meaningful when connected to a Navidrome server */}
      {source === "server" && serverConfigured && (
        <div className="dac-card">
          <div className="pro-feature-row" style={{ paddingTop: 0, borderTop: "none" }}>
            <div className="pro-feature-row__label">
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                aria-hidden="true"
                style={{ width: 13, height: 13 }}
              >
                <path d="M9 18V5l12-2v13" />
                <circle cx="6" cy="18" r="3" />
                <circle cx="18" cy="16" r="3" />
              </svg>
              Scrobble plays
            </div>
            <button
              className={`pro-toggle${scrobbleEnabled ? " pro-toggle--on" : ""}`}
              onClick={() => setScrobbleEnabled(!scrobbleEnabled)}
              aria-pressed={scrobbleEnabled}
              aria-label={scrobbleEnabled ? "Disable scrobbling" : "Enable scrobbling"}
              title={scrobbleEnabled ? "Scrobbling on" : "Scrobbling off"}
            >
              <span className="pro-toggle__thumb" />
            </button>
          </div>
        </div>
      )}
      <OfflinePanel />
      {UPDATER_ENABLED && <UpdatePanel />}
    </aside>
  );
}
