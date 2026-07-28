import { useUiStore } from "../store/useUiStore";
import { AccentPicker } from "./AccentPicker";
import { ThemeSwitcher } from "./ThemeSwitcher";
import { useVisualizerStore } from "./visualizer/useVisualizerStore";
// @pro → src/pro-stub in the free build (tier always "free" → no badge); the real
// license store in the Pro build drives the PRO badge.
import { useLicenseStore } from "@pro";

export function TopBar() {
  const source = useUiStore((s) => s.source);
  const setSource = useUiStore((s) => s.setSource);
  const query = useUiStore((s) => s.query);
  const setQuery = useUiStore((s) => s.setQuery);
  const toggleCompact = useUiStore((s) => s.toggleCompact);
  const proTier = useLicenseStore((s) => s.tier);
  const vizOpen = useVisualizerStore((s) => s.open);
  const toggleVisualizer = useVisualizerStore((s) => s.toggle);

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
          {proTier === "pro" && (
            <span className="pro-badge" title="EKO Pro">
              PRO
            </span>
          )}
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

        {/* GPU visualizer — Galaxy is FREE; Cymatics/Murmuration are Pro-only presets
            reachable via the native "Visualizer" menu. This button just opens/closes
            whichever overlay the current license tier renders (PlayerApp.tsx). */}
        <div
          className="icon-btn"
          title="Visualizer"
          onClick={toggleVisualizer}
          role="button"
          tabIndex={0}
          aria-label="Toggle visualizer"
          aria-pressed={vizOpen}
          onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? toggleVisualizer() : undefined)}
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
            <path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1" />
            <circle cx="12" cy="12" r="3" />
          </svg>
        </div>

        {/* alternate skins remain Pro-only, in the native "Skins" menu (menu bar) */}

        {/* accent color — FREE feature */}
        <AccentPicker />

        {/* dark-mode toggle — FREE feature; Porcelain ↔ Graphite */}
        <ThemeSwitcher />
      </div>
    </header>
  );
}
