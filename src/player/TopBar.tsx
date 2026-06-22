import { useUiStore } from "../store/useUiStore";
import { ThemeSwitcher } from "./ThemeSwitcher";

export function TopBar() {
  const source = useUiStore((s) => s.source);
  const setSource = useUiStore((s) => s.setSource);
  const query = useUiStore((s) => s.query);
  const setQuery = useUiStore((s) => s.setQuery);
  const toggleCompact = useUiStore((s) => s.toggleCompact);

  return (
    <header className="topbar" data-tauri-drag-region>
      <div className="brandzone" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <span className="bars">
            <i />
            <i />
            <i />
            <i />
          </span>
          <b>EKO</b>
        </div>
      </div>
      <div className="mainzone" data-tauri-drag-region>
        <div className="search">
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden="true"
          >
            <circle cx="11" cy="11" r="7" />
            <path d="M21 21l-4.3-4.3" />
          </svg>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search albums, artists, tracks…"
            spellCheck={false}
            aria-label="Search albums, artists, tracks"
          />
        </div>

        <div className="spacer" />

        <div className="srcseg" role="group" aria-label="Music source">
          <b
            className={source === "local" ? "on" : ""}
            onClick={() => setSource("local")}
            role="button"
            tabIndex={0}
            aria-pressed={source === "local"}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? setSource("local") : undefined)}
          >
            LOCAL
          </b>
          <b
            className={source === "server" ? "on" : ""}
            onClick={() => setSource("server")}
            role="button"
            tabIndex={0}
            aria-pressed={source === "server"}
            onKeyDown={(e) =>
              e.key === "Enter" || e.key === " " ? setSource("server") : undefined
            }
          >
            {source === "server" && <span className="led" aria-hidden="true" />}SERVER
          </b>
        </div>

        <div
          className="icon-btn"
          title="Mini player"
          onClick={toggleCompact}
          role="button"
          tabIndex={0}
          aria-label="Switch to mini player"
          onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? toggleCompact() : undefined)}
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M4 8V5a1 1 0 0 1 1-1h3" />
            <path d="M16 4h3a1 1 0 0 1 1 1v3" />
            <path d="M20 16v3a1 1 0 0 1-1 1h-3" />
            <path d="M8 20H5a1 1 0 0 1-1-1v-3" />
          </svg>
        </div>

        {/* skin + accent now live in the native "Skins" menu (menu bar) */}

        {/* dark-mode toggle — FREE feature; Porcelain ↔ Graphite */}
        <ThemeSwitcher />
      </div>
    </header>
  );
}
