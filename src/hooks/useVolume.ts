import type React from "react";
import { usePlayerStore } from "../store/usePlayerStore";

/**
 * Headless volume control. Owns the pointer-lock drag (relative `movementY` so a bottom-docked
 * knob never runs out of travel at the screen edge) and wheel interaction; the store already
 * throttles the IPC. A theme binds `onPointerDown`/`onWheel` to whatever anatomy it draws
 * (dial, fader, slider) and positions from `value`/`deg`.
 */
export function useVolume() {
  const value = usePlayerStore((s) => s.volume);

  const onPointerDown = (e: React.PointerEvent) => {
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    let v = usePlayerStore.getState().volume;
    try {
      el.requestPointerLock();
    } catch {
      /* unsupported → plain window drag below */
    }
    const mv = (ev: PointerEvent) => {
      v = Math.min(1, Math.max(0, v - ev.movementY / 130));
      usePlayerStore.getState().setVolume(v);
    };
    const up = () => {
      try {
        document.exitPointerLock();
      } catch {
        /* noop */
      }
      window.removeEventListener("pointermove", mv);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
  };

  const onWheel = (e: React.WheelEvent) => {
    const s = usePlayerStore.getState();
    s.setVolume(Math.min(1, Math.max(0, s.volume - Math.sign(e.deltaY) * 0.03)));
  };

  return {
    value,
    /** Needle angle for a rotary dial (−135°…+135°). */
    deg: -135 + value * 270,
    onPointerDown,
    onWheel,
    set: (v: number) => usePlayerStore.getState().setVolume(Math.min(1, Math.max(0, v))),
  };
}
