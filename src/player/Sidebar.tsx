import { type ReactNode } from "react";
import { useUiStore, type LibSection } from "../store/useUiStore";
import { useMusicSource } from "../hooks/useMusicSource";
import { useIsPro, OfflinePanel } from "@pro";
import { UpdatePanel } from "./UpdatePanel";
import { UPDATER_ENABLED } from "../store/useUpdaterStore";
import { usePlayerStore } from "../store/usePlayerStore";
import { useSignalPath } from "../hooks/useSignalPath";

const khz = (n: number) => (n ? `${(n / 1000).toFixed(n % 1000 ? 1 : 0)} kHz` : "—");

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
  const sp = useSignalPath();

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

  // OUTPUT reflects the REAL CoreAudio device, not a placeholder. While a track is
  // playing the engine reports the device it actually opened (`info.device`) and its
  // live rate; otherwise fall back to the user's selected device (null = system
  // default). The status line uses the honest signal-path seal (BIT-PERFECT / the
  // active modifiers) — never a hardcoded spec. Independent of music source, since the
  // output device is the same whether streaming from a server or playing local files.
  const out = {
    t: "OUTPUT",
    n: sp.active && sp.info?.device ? sp.info.device : (sp.outputDevice ?? "System default"),
    s: sp.active && sp.info ? `${sp.sealLabel} · ${khz(sp.info.rate)}` : "—",
  };

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
        {/* Smart Playlists + Offline are PRO features — gate them strictly on `isPro` so
            they never appear in the free build (where `useIsPro()` is hard-false via the
            @pro stub). Do NOT gate on `source === "server"`: that leaked these Pro entries
            to any free user connected to Navidrome. */}
        {isPro && (
          <Item
            id="smart-playlists"
            label="Smart Playlists"
            active={isLib("smart-playlists")}
            onClick={() => go("smart-playlists")}
          />
        )}
        {isPro && (
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
          <div
            className="pro-feature-row"
            style={{ padding: 0, borderTop: "none" }}
            title="Records each track you play to your server's listening history — and to Last.fm / ListenBrainz if you've connected them in Navidrome."
          >
            <div className="pro-feature-row__label">Scrobble plays</div>
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
