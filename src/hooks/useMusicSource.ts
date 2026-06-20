import { useUiStore } from "../store/useUiStore";
import { useSubsonic } from "../subsonic/useSubsonic";
import { useLocal } from "../local/useLocal";

/**
 * Source/connection info + local-folder management for the nav shell. Keeps `useSubsonic` /
 * `useLocal` out of the Sidebar (and any theme's nav), exposing only what the chrome shows
 * (which source is active, whether the server is configured, the local root) and the folder
 * actions.
 */
export function useMusicSource() {
  const source = useUiStore((s) => s.source);
  const serverConfigured = useSubsonic((s) => !!s.config);
  const localRoot = useLocal((s) => s.rootName);
  const subCount = useSubsonic((s) => s.albums.length);
  const localCount = useLocal((s) => s.albums.length);
  return {
    source,
    serverConfigured,
    localRoot,
    /** Album count for the active source (shell header). */
    albumCount: source === "server" ? subCount : localCount,
    rescan: () => useLocal.getState().rescan(),
    changeFolder: () => void useLocal.getState().pickFolder(),
    clearFolder: () => useLocal.getState().reset(),
  };
}
