import { ACCENTS, SKINS, useUiStore } from "../store/useUiStore";

export function TopBar() {
  const source = useUiStore((s) => s.source);
  const setSource = useUiStore((s) => s.setSource);
  const query = useUiStore((s) => s.query);
  const setQuery = useUiStore((s) => s.setQuery);
  const theme = useUiStore((s) => s.theme);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const accent = useUiStore((s) => s.accent);
  const setAccent = useUiStore((s) => s.setAccent);
  const skin = useUiStore((s) => s.skin);
  const setSkin = useUiStore((s) => s.setSkin);
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
          <span>PRO</span>
        </div>
      </div>
      <div className="mainzone" data-tauri-drag-region>
        <div className="search">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="11" cy="11" r="7" />
            <path d="M21 21l-4.3-4.3" />
          </svg>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search albums, artists, tracks…"
            spellCheck={false}
          />
        </div>

        <div className="spacer" />

        <div className="srcseg" role="tablist" title="Source">
          <b className={source === "local" ? "on" : ""} onClick={() => setSource("local")}>
            LOCAL
          </b>
          <b className={source === "server" ? "on" : ""} onClick={() => setSource("server")}>
            {source === "server" && <span className="led" />}SERVER
          </b>
        </div>

        <div className="icon-btn" title="Mini player" onClick={toggleCompact}>
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M4 8V5a1 1 0 0 1 1-1h3" />
            <path d="M16 4h3a1 1 0 0 1 1 1v3" />
            <path d="M20 16v3a1 1 0 0 1-1 1h-3" />
            <path d="M8 20H5a1 1 0 0 1-1-1v-3" />
          </svg>
        </div>

        <div className="skinseg" role="tablist" title="Skin">
          {SKINS.map((sk) => (
            <b
              key={sk.id}
              className={skin === sk.id ? "on" : ""}
              onClick={() => setSkin(sk.id)}
            >
              {sk.label}
            </b>
          ))}
        </div>

        <div className="accent-pick" role="group" aria-label="Accent color">
          {ACCENTS.map((a) => (
            <button
              key={a.id}
              type="button"
              className={"acc-sw" + (accent === a.id ? " on" : "")}
              style={{ "--c": a.swatch } as React.CSSProperties}
              title={a.label}
              aria-label={a.label}
              aria-pressed={accent === a.id}
              onClick={() => setAccent(a.id)}
            />
          ))}
        </div>

        <div
          className="icon-btn"
          title={theme === "dark" ? "Light mode" : "Dark mode"}
          onClick={toggleTheme}
        >
          {theme === "dark" ? (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="4" />
              <path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5L19 19M19 5l-1.5 1.5M6.5 17.5L5 19" />
            </svg>
          ) : (
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M12 3a6.4 6.4 0 0 0 9 9 9 9 0 1 1-9-9z" />
            </svg>
          )}
        </div>
      </div>
    </header>
  );
}
