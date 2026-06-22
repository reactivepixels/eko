/**
 * ThemeSwitcher — free dark-mode toggle button (Porcelain ↔ Graphite).
 *
 * Light/dark mode is a free feature. This component lives in src/player/
 * (not behind @pro) and is rendered in TopBar in both free and Pro builds.
 *
 * The Studio skin + skin switcher remain Pro-only (see StudioApp / useUiStore).
 */
import { useUiStore } from "../store/useUiStore";

export function ThemeSwitcher() {
  const theme = useUiStore((s) => s.theme);
  const toggleTheme = useUiStore((s) => s.toggleTheme);

  return (
    <div
      className="icon-btn"
      title={theme === "dark" ? "Light mode" : "Dark mode"}
      onClick={toggleTheme}
      role="button"
      tabIndex={0}
      aria-label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
      onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? toggleTheme() : undefined)}
    >
      {theme === "dark" ? (
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          aria-hidden="true"
        >
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
          aria-hidden="true"
        >
          <path d="M12 3a6.4 6.4 0 0 0 9 9 9 9 0 1 1-9-9z" />
        </svg>
      )}
    </div>
  );
}
