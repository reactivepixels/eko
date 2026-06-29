import type React from "react";
import { usePlayerStore } from "../store/usePlayerStore";

/**
 * Headless seek/scrub. Owns the position math and the begin/move/end scrub contract (the
 * store throttles the engine seeks ~80ms and holds an optimistic position until the engine
 * converges — no CSS transition on the dragged element). A theme binds the three pointer
 * handlers to its track element and positions the fill/thumb from `progress`.
 */
export function useScrub() {
  const currentTime = usePlayerStore((s) => s.currentTime);
  const duration = usePlayerStore((s) => s.duration);
  const buffered = usePlayerStore((s) => s.buffered);
  const progress = duration > 0 ? currentTime / duration : 0;
  // Fraction of the track decoded so far (server streams fill progressively; local files
  // are 1 once playing). Drawn as the scrubber's "buffered" fill behind the played fill.
  const bufferedProgress = duration > 0 ? Math.min(1, buffered / duration) : 0;

  const secsAt = (clientX: number, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    const t = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    return t * usePlayerStore.getState().duration;
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (usePlayerStore.getState().duration <= 0) return;
    (e.target as Element).setPointerCapture(e.pointerId);
    const s = usePlayerStore.getState();
    s.beginScrub();
    s.scrubMove(secsAt(e.clientX, e.currentTarget as HTMLElement));
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (e.buttons & 1)
      usePlayerStore.getState().scrubMove(secsAt(e.clientX, e.currentTarget as HTMLElement));
  };
  const onPointerUp = (e: React.PointerEvent) =>
    usePlayerStore.getState().endScrub(secsAt(e.clientX, e.currentTarget as HTMLElement));

  return {
    currentTime,
    duration,
    progress,
    bufferedProgress,
    remaining: Math.max(0, duration - currentTime),
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel: onPointerUp,
  };
}
