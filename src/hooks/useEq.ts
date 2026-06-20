import { useRef } from "react";
import type React from "react";
import { usePlayerStore } from "../store/usePlayerStore";
import { EQ_PRESETS, EQ_GAIN_MIN, EQ_GAIN_MAX, type EqPreset } from "../audio/constants";

const BANDS = ["60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k"];

/**
 * Headless 10-band EQ. Owns the band labels, gain range, preset list, and the pointer
 * interaction math for BOTH control anatomies (vertical fader rail and rotary knob) — so a
 * theme injects whichever visual it wants (Radix `Slot` pattern) and binds the handlers.
 * `norm(i)` is the 0..1 position for either anatomy (the range is symmetric, ±12 dB).
 */
export function useEq() {
  const gains = usePlayerStore((s) => s.gains);
  const preamp = usePlayerStore((s) => s.preamp);
  const presetName = usePlayerStore((s) => s.presetName);

  const setBand = (i: number, g: number) =>
    usePlayerStore.getState().setBandGain(i, Math.max(EQ_GAIN_MIN, Math.min(EQ_GAIN_MAX, g)));
  const applyPreset = (p: EqPreset) => usePlayerStore.getState().applyPreset(p);

  // Fader: absolute position on the rail → gain.
  const setFromRail = (i: number, clientY: number, rail: HTMLElement) => {
    const r = rail.getBoundingClientRect();
    const t = 1 - Math.min(1, Math.max(0, (clientY - r.top) / r.height));
    setBand(i, EQ_GAIN_MIN + t * (EQ_GAIN_MAX - EQ_GAIN_MIN));
  };
  const railHandlers = (i: number) => ({
    onPointerDown: (e: React.PointerEvent) => {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      setFromRail(i, e.clientY, e.currentTarget as HTMLElement);
    },
    onPointerMove: (e: React.PointerEvent) => {
      if (e.buttons & 1) setFromRail(i, e.clientY, e.currentTarget as HTMLElement);
    },
  });

  // Knob: relative vertical drag (~130px = full range).
  const knobDrag = useRef<{ i: number; startY: number; startGain: number } | null>(null);
  const knobHandlers = (i: number) => ({
    onPointerDown: (e: React.PointerEvent) => {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
      knobDrag.current = { i, startY: e.clientY, startGain: gains[i] };
    },
    onPointerMove: (e: React.PointerEvent) => {
      const d = knobDrag.current;
      if (!d || !(e.buttons & 1)) return;
      setBand(d.i, d.startGain + ((d.startY - e.clientY) / 130) * (EQ_GAIN_MAX - EQ_GAIN_MIN));
    },
    onPointerUp: () => {
      knobDrag.current = null;
    },
  });

  return {
    gains,
    preamp,
    presetName,
    bands: BANDS,
    presets: EQ_PRESETS,
    gainMin: EQ_GAIN_MIN,
    gainMax: EQ_GAIN_MAX,
    norm: (i: number) => (gains[i] - EQ_GAIN_MIN) / (EQ_GAIN_MAX - EQ_GAIN_MIN),
    setBand,
    applyPreset,
    railHandlers,
    knobHandlers,
  };
}
