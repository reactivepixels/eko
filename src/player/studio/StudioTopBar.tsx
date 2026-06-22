import { useUiStore } from "../../store/useUiStore";
import styles from "./StudioTopBar.module.css";

export function StudioTopBar() {
  const source = useUiStore((s) => s.source);
  const setSource = useUiStore((s) => s.setSource);
  const query = useUiStore((s) => s.query);
  const setQuery = useUiStore((s) => s.setQuery);
  const theme = useUiStore((s) => s.theme);
  const toggleTheme = useUiStore((s) => s.toggleTheme);
  const toggleCompact = useUiStore((s) => s.toggleCompact);

  return (
    <header className={styles.bevel} data-tauri-drag-region>
      {/* brand — drag zone */}
      <div className={styles.brand} data-tauri-drag-region>
        <span className={styles.bars}>
          <i />
          <i className={styles.barsLive} />
          <i />
          <i />
        </span>
        <b className={styles.brandName}>EKO</b>
      </div>

      {/* drag zone: space between brand and controls */}
      <div className={styles.dragFill} data-tauri-drag-region />

      {/* search well */}
      <div className={styles.search}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <circle cx="11" cy="11" r="7" />
          <path d="M21 21l-4.3-4.3" />
        </svg>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search albums, artists, tracks…"
          spellCheck={false}
          className={styles.searchInput}
        />
      </div>

      {/* right cluster — no drag region so controls are clickable */}
      <div className={styles.hdrRight}>
        {/* LOCAL | SERVER source segmented */}
        <div className={styles.source} role="tablist" title="Source">
          <button
            className={
              source === "local" ? `${styles.sourceBtn} ${styles.sourceBtnOn}` : styles.sourceBtn
            }
            onClick={() => setSource("local")}
          >
            LOCAL
          </button>
          <button
            className={
              source === "server" ? `${styles.sourceBtn} ${styles.sourceBtnOn}` : styles.sourceBtn
            }
            onClick={() => setSource("server")}
          >
            {source === "server" && <span className={styles.sourceLed} />}
            SERVER
          </button>
        </div>

        {/* mini-player */}
        <button className={styles.ibtn} title="Mini player" onClick={toggleCompact}>
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
        </button>

        {/* skin + accent now live in the native "Skins" menu (menu bar) */}

        {/* light / dark theme toggle */}
        <button
          className={styles.ibtn}
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
        </button>
      </div>
    </header>
  );
}
