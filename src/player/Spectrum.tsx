import { useEffect, useRef } from "react";
import { usePlayerStore } from "../store/usePlayerStore";
import { nativeEngine } from "../audio/nativeEngine";

interface Props {
  bands?: number;
  segs?: number;
  bargap?: number;
  className?: string;
}

/**
 * Segmented LED spectrum, drawn on canvas and driven by the live AnalyserNode.
 * Short, wide, flat segments (no glow) with white peak-hold caps that fall away —
 * the classic digital-amp display. Falls quietly to rest when nothing's playing.
 */
export function Spectrum({ bands = 36, bargap = 2, className }: Props) {
  const cvs = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = cvs.current!;
    const ctx = canvas.getContext("2d")!;
    // Translucent monochrome (theme-aware via --spec-rgb): ink on light, white on dark.
    const specRgb = () =>
      getComputedStyle(canvas).getPropertyValue("--spec-rgb").trim() || "64,59,52";

    const lvl = new Float32Array(bands);
    const peak = new Float32Array(bands);
    let raf = 0;

    const resize = () => {
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      const w = canvas.clientWidth,
        h = canvas.clientHeight;
      canvas.width = Math.max(1, Math.round(w * dpr));
      canvas.height = Math.max(1, Math.round(h * dpr));
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(canvas);

    const draw = () => {
      const w = canvas.clientWidth,
        h = canvas.clientHeight;
      ctx.clearRect(0, 0, w, h);

      // Bands come from the Rust FFT (engine_bands); fall quietly to rest when idle.
      const eng = usePlayerStore.getState().engineActive ? nativeEngine.getBands() : null;
      if (eng && eng.length) {
        for (let b = 0; b < bands; b++) {
          const v = eng[Math.min(eng.length - 1, Math.floor((b / bands) * eng.length))] ?? 0;
          lvl[b] = v > lvl[b] ? v : lvl[b] * 0.82 + v * 0.18; // fast attack, smooth release
        }
      } else {
        for (let b = 0; b < bands; b++) lvl[b] *= 0.9;
      }

      // Padding scales with size so the same component works for the big deck
      // display and the tiny transport mini-spectrum.
      const padX = Math.max(3, Math.min(12, w * 0.04));
      const padY = Math.max(2, Math.min(11, h * 0.1));
      const innerW = w - padX * 2,
        innerH = h - padY * 2;
      const bandW = (innerW - bargap * (bands - 1)) / bands;
      // Derive the segment count from the height at a fixed ~5px pitch, so each
      // segment stays thin/delicate no matter how tall the display is.
      const segs = Math.max(4, Math.round(innerH / 5));
      const vgap = innerH < 60 ? 0.7 : 1;
      const segH = (innerH - vgap * (segs - 1)) / segs;
      const rad = Math.min(1.6, segH / 2);
      const rgb = specRgb();

      for (let b = 0; b < bands; b++) {
        const x = padX + b * (bandW + bargap);
        const lit = Math.round(lvl[b] * segs);
        // peak-hold
        if (lvl[b] >= peak[b]) peak[b] = lvl[b];
        else peak[b] = Math.max(lvl[b], peak[b] - 0.012);
        const pkRow = Math.min(segs - 1, Math.round(peak[b] * segs));

        for (let s = 0; s < segs; s++) {
          const y = padY + innerH - (s + 1) * segH - s * vgap;
          const f = (s + 1) / segs;
          if (s === pkRow && peak[b] > 0.02) ctx.fillStyle = `rgba(${rgb},0.9)`;
          else if (s < lit) ctx.fillStyle = `rgba(${rgb},${(0.24 + 0.5 * f).toFixed(3)})`;
          else ctx.fillStyle = `rgba(${rgb},0.07)`;
          ctx.beginPath();
          ctx.roundRect(x, y, bandW, segH, rad);
          ctx.fill();
        }
      }
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [bands, bargap]);

  return <canvas ref={cvs} className={className} />;
}
