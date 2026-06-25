/**
 * ThemeHost — resolves the active skin to a registered theme and renders its Shell.
 *
 * Replaces the old `StudioApp` switch. `resolveTheme` falls back to Porcelain for any
 * unregistered id (e.g. a stale persisted `studio` pref in a free build), so the host always
 * renders a valid theme. Registration happens once at boot (App.tsx imports registerThemes).
 */
import { useUiStore } from "../store/useUiStore";
import { resolveTheme } from "../skin/registry";

export function ThemeHost() {
  const skin = useUiStore((s) => s.skin);
  const { Shell } = resolveTheme(skin);
  return <Shell />;
}
