import { useUiStore } from "../store/useUiStore";
import { useNativeMenu, VisualizerOverlay } from "@pro";
import { useSleepTimerMenu } from "../hooks/useSleepTimerMenu";
import { ThemeHost } from "./ThemeHost";
import "./neu.css";

/**
 * PlayerApp — the themed shell wrapper.
 *
 * ONE app, themed. `data-theme` (light/dark, free), `data-accent` (free), and `data-skin`
 * (Pro-gated) are stamped on `.app`; the active skin's Shell is rendered by `ThemeHost`, which
 * resolves it from the theme registry (src/skin/registry.ts). Porcelain and Studio are both
 * registered themes — there is no per-skin switch here anymore.
 */
export function PlayerApp() {
  const theme = useUiStore((s) => s.theme);
  const accent = useUiStore((s) => s.accent);
  const skin = useUiStore((s) => s.skin);
  useNativeMenu(); // Pro: bridges the native "Skins" menu ↔ the UI store; no-op in free.
  useSleepTimerMenu(); // Free: bridges the native "Controls ▸ Sleep Timer" menu ↔ the player store.

  return (
    <div className="app" data-theme={theme} data-accent={accent} data-skin={skin}>
      <ThemeHost />
      <VisualizerOverlay />
    </div>
  );
}
