import { useSubsonic } from "../subsonic/useSubsonic";
import { useUiStore } from "../store/useUiStore";
import type { SubsonicConfig } from "../subsonic/client";

/** Navidrome/OpenSubsonic connection logic for the connect form — keeps `useSubsonic` out of
 *  the presentation component. The form owns only its input state. */
export function useConnect() {
  const addAndConnect = useSubsonic((s) => s.addAndConnect);
  const status = useSubsonic((s) => s.status);
  const error = useSubsonic((s) => s.error);
  return {
    status,
    error,
    /** Connect and add to the server list. */
    connect: (cfg: SubsonicConfig) => addAndConnect(undefined, cfg),
    /** Dismiss the form and fall back to the local library. */
    fallbackToLocal: () => useUiStore.getState().setSource("local"),
  };
}
