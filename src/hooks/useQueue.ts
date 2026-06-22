import { usePlayerStore } from "../store/usePlayerStore";
import { useUiStore } from "../store/useUiStore";
import { coverArtUrl } from "../subsonic/client";
import type { Track } from "../types";
import type { MenuItem } from "../player/ContextMenu";

/** Headless queue: the up-next list, current position, reorder/remove/clear ops, panel
 *  open-state, and the row context-menu items. Cover resolution is normalised here. */
export function useQueue() {
  const tracks = usePlayerStore((s) => s.tracks);
  const currentIndex = usePlayerStore((s) => s.currentIndex);
  const isOpen = useUiStore((s) => s.queueOpen);
  const close = useUiStore((s) => s.toggleQueue);

  return {
    tracks,
    currentIndex,
    isOpen,
    close,
    playAt: (i: number) => void usePlayerStore.getState().playAt(i),
    remove: (id: string) => usePlayerStore.getState().removeTrack(id),
    clear: () => usePlayerStore.getState().clearPlaylist(),
    reorder: (from: number, to: number) => usePlayerStore.getState().reorder(from, to),
    /** Server cover thumbnail URL, or null (local art uses `<LocalCover>` via the track path). */
    coverUrl: (t: Track, size: number) =>
      t.coverArt ? (coverArtUrl(t.coverArt, size) ?? null) : null,
    rowMenuItems: (id: string, i: number): MenuItem[] => [
      { label: "Play now", onSelect: () => void usePlayerStore.getState().playAt(i) },
      { separator: true },
      {
        label: "Remove from queue",
        danger: true,
        onSelect: () => usePlayerStore.getState().removeTrack(id),
      },
    ],
  };
}
