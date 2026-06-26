import { useEffect, useRef } from "react";
import { useSpectrum } from "../hooks/useSpectrum";

/**
 * Meter — the ONE segmented LED level meter shared by every theme (the unified design from the
 * standardized kits). Lit-segment count is driven by the RMS of the live spectrum, with fast attack
 * / slow release smoothing. Material (off-colour, recess, accent) is 100% CSS tokens (.meter/.mbar
 * in neu.css) so each skin re-tints it — no per-theme meter component. The last two segments are the
 * peak band (accent-2). Free + shared: registered once as the "meter" slot variant for all presets.
 */
const METER_SEGS = 12;
// Makeup gain + perceptual curve mapping the RMS level → lit segments. The raw bands
// are sqrt(power) amplitudes across 32 log-spaced bins, so a music spectrum's many
// quiet HF bands keep the RMS low (~0.1–0.2) — without makeup the meter only ever lit
// 2–3 segments. GAIN lifts normal playback into the upper half; CURVE (<1) expands the
// low/mid range so the meter is responsive without pinning. Tunable — calibrate by ear
// on live audio (the harness feeds no live spectrum, so this can't be screenshot-verified).
const METER_GAIN = 2.4;
const METER_CURVE = 0.62;

export function Meter() {
  const { read } = useSpectrum();
  const refs = useRef<(HTMLSpanElement | null)[]>([]);
  useEffect(() => {
    let raf = 0;
    let smoothed = 0;
    const tick = () => {
      const bands = read();
      let target = 0;
      if (bands && bands.length > 0) {
        // Loudness proxy: RMS of the band magnitudes (perceived level tracks energy,
        // not the arithmetic mean of a spectrum — which the quiet HF bands drag down).
        let energy = 0;
        for (let i = 0; i < bands.length; i++) energy += bands[i] * bands[i];
        const rms = Math.sqrt(energy / bands.length);
        target = Math.pow(rms, METER_CURVE) * METER_GAIN;
        if (target > 1) target = 1;
      }
      smoothed =
        target > smoothed ? smoothed * 0.5 + target * 0.5 : smoothed * 0.88 + target * 0.12;
      if (smoothed < 0) smoothed = 0;
      if (smoothed > 1) smoothed = 1;
      const lit = Math.round(smoothed * METER_SEGS);
      for (let i = 0; i < METER_SEGS; i++) {
        const el = refs.current[i];
        if (el) el.dataset.lit = i < lit ? "1" : "0";
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [read]);
  return (
    <div className="meter" aria-hidden="true">
      {Array.from({ length: METER_SEGS }, (_, i) => (
        <span
          key={i}
          className={`mbar${i >= METER_SEGS - 2 ? " pk" : ""}`}
          ref={(el) => {
            refs.current[i] = el;
          }}
          data-lit="0"
        />
      ))}
    </div>
  );
}
