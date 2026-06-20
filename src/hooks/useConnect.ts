import { useSubsonic } from "../subsonic/useSubsonic";
import { useUiStore } from "../store/useUiStore";
import type { SubsonicConfig } from "../subsonic/client";

/** Navidrome/OpenSubsonic connection logic for the connect form — keeps `useSubsonic` out of
 *  the presentation component. The form owns only its input state. */
export function useConnect() {
  const connect = useSubsonic((s) => s.connect);
  const status = useSubsonic((s) => s.status);
  const error = useSubsonic((s) => s.error);
  return {
    status,
    error,
    connect: (cfg: SubsonicConfig) => connect(cfg),
    /** Dismiss the form and fall back to the local library. */
    useLocalInstead: () => useUiStore.getState().setSource("local"),
  };
}
