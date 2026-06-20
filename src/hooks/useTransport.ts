import { usePlayerStore } from "../store/usePlayerStore";

/** Headless transport: play/pause + prev/next + shuffle/repeat state and actions. A theme's
 *  transport control (bar, dock, …) binds this and renders pixels only. */
export function useTransport() {
  const isPlaying = usePlayerStore((s) => s.isPlaying);
  const queueLength = usePlayerStore((s) => s.tracks.length);
  const shuffle = usePlayerStore((s) => s.shuffle);
  const repeat = usePlayerStore((s) => s.repeat);
  return {
    isPlaying,
    hasQueue: queueLength > 0,
    shuffle,
    repeat,
    togglePlay: () => void usePlayerStore.getState().togglePlay(),
    next: () => void usePlayerStore.getState().next(),
    prev: () => void usePlayerStore.getState().prev(),
    toggleShuffle: () => usePlayerStore.getState().toggleShuffle(),
    cycleRepeat: () => usePlayerStore.getState().cycleRepeat(),
  };
}
