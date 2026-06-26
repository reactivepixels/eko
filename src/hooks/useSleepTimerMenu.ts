import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { usePlayerStore, SLEEP_PRESETS, type SleepPreset } from "../store/usePlayerStore";

/**
 * Bridges the native "Controls ▸ Sleep Timer" menu to the player store.
 *
 * FREE feature, so this is mounted at the app root (PlayerApp) — it must run in every
 * build and skin. It deliberately does NOT live in `useNativeMenu` (Pro-only, a no-op
 * in the free build via the @pro alias) or in `SleepTimerControl` (only mounts in some
 * transports). The native menu items emit `menu-action` events:
 *   `sleep:off | sleep:15 | sleep:30 | sleep:45 | sleep:60 | sleep:eot`
 * The live countdown + cancel affordance is the in-transport pill (SleepTimerControl).
 */
export function useSleepTimerMenu(): void {
  useEffect(() => {
    const un = listen<string>("menu-action", (e) => {
      const [kind, value] = e.payload.split(":");
      if (kind !== "sleep") return;
      const p = usePlayerStore.getState();
      if (value === "off") {
        p.cancelSleepTimer();
      } else if (value === "eot") {
        p.startSleepTimer(-1);
      } else {
        const mins = Number(value) as SleepPreset;
        if ((SLEEP_PRESETS as readonly number[]).includes(mins)) p.startSleepTimer(mins);
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);
}
