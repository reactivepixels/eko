import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useUiStore, type Accent, type Skin } from "../store/useUiStore";

/**
 * Bridges the native macOS "Skins" menu to the UI store, both directions:
 *  - menu click → Rust emits `menu-action` (e.g. "skin:studio") → we update the store
 *  - store changes (skin/accent/theme) → we call `sync_menu` so the checkmarks stay correct
 * The store stays the single source of truth (persisted to localStorage).
 */
export function useNativeMenu() {
  const skin = useUiStore((s) => s.skin);
  const accent = useUiStore((s) => s.accent);
  const theme = useUiStore((s) => s.theme);

  // Push current state to the native menu checkmarks (on mount + whenever it changes).
  useEffect(() => {
    void invoke("sync_menu", { skin, accent, theme }).catch(() => {});
  }, [skin, accent, theme]);

  // Apply menu clicks to the store.
  useEffect(() => {
    const un = listen<string>("menu-action", (e) => {
      const [kind, value] = e.payload.split(":");
      const st = useUiStore.getState();
      if (kind === "skin") st.setSkin(value as Skin);
      else if (kind === "accent") st.setAccent(value as Accent);
      else if (kind === "theme") st.toggleTheme();
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);
}
