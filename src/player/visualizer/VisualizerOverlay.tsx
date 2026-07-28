/**
 * VisualizerOverlay (free) — full-window fixed GPU visualizer overlay, Galaxy only.
 *
 * Shown when useVisualizerStore().open is true. Manages the WebGL2 canvas
 * lifecycle, runs the rAF loop, drives AudioFeatureTracker from useSpectrum(),
 * handles resize via ResizeObserver, and tears down on close/unmount.
 * Esc closes. Minimal UI: one fading close button.
 *
 * Theme and accent are read from useUiStore each frame so they are always live.
 *
 * Opened from the native "Visualizer ▸ On" menu item (FREE in every build) — this component
 * owns that bridge for the free tier. No license check here: PlayerApp renders this OR the Pro
 * overlay, never both, so reaching this code already means "not licensed Pro". The free registry
 * contains only Galaxy, so there is no preset to pick.
 */

import React, { useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVisualizerStore } from "./useVisualizerStore";
import { useSpectrum } from "../../hooks/useSpectrum";
import { useUiStore, ACCENTS } from "../../store/useUiStore";
import { AudioFeatureTracker } from "../../audio/audioFeatures";
import { visualizers } from "./registry";
import type { Visualizer } from "./types";
import styles from "./overlay.module.css";

// Map from Accent id to linear [r,g,b] (0..1). Derived from the swatch hex values.
// Built lazily on first use (not at module init): useUiStore forms an import cycle
// with some callers, so reading ACCENTS at the top level can hit the temporal dead
// zone depending on module evaluation order. Deferring to first render guarantees
// ACCENTS is initialised by the time we read it.
let ACCENT_RGB: Record<string, [number, number, number]> | null = null;

function toAccentRgb(accent: string): [number, number, number] {
  if (!ACCENT_RGB) {
    ACCENT_RGB = {};
    for (const a of ACCENTS) {
      const hex = a.swatch.replace("#", "");
      ACCENT_RGB[a.id] = [
        parseInt(hex.slice(0, 2), 16) / 255,
        parseInt(hex.slice(2, 4), 16) / 255,
        parseInt(hex.slice(4, 6), 16) / 255,
      ];
    }
  }
  return ACCENT_RGB[accent] ?? [0.16, 0.85, 0.97]; // cyan fallback
}

export function VisualizerOverlay(): React.ReactElement | null {
  const open = useVisualizerStore((s) => s.open);
  const presetId = useVisualizerStore((s) => s.presetId);
  const close = useVisualizerStore((s) => s.close);

  const theme = useUiStore((s) => s.theme);
  const accent = useUiStore((s) => s.accent);

  const spectrum = useSpectrum();

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const vizRef = useRef<Visualizer | null>(null);
  const trackerRef = useRef<AudioFeatureTracker | null>(null);
  const rafRef = useRef<number | null>(null);
  const lastRef = useRef<number>(0);

  const themeRef = useRef(theme);
  const accentRef = useRef(accent);
  useEffect(() => {
    themeRef.current = theme;
  }, [theme]);
  useEffect(() => {
    accentRef.current = accent;
  }, [accent]);

  // ── Native "Visualizer" menu bridge ──────────────────────────────────────
  // Runs regardless of `open` so the menu checkmark stays in sync even when closed.
  useEffect(() => {
    void invoke("sync_visualizer", { open, preset: presetId }).catch(() => {});
  }, [open, presetId]);

  useEffect(() => {
    const unlisten = listen<string>("menu-action", (e) => {
      const id = e.payload;
      if (!id.startsWith("visualizer:")) return;
      if (id === "visualizer:on") {
        useVisualizerStore.getState().toggle();
      } else if (id === "visualizer:galaxy") {
        // Only preset in the free build; the free menu doesn't list presets, but a Pro-built
        // binary running unlicensed can still emit this — treat it as "open Galaxy".
        useVisualizerStore.getState().openWith("galaxy");
      }
      // Pro-only presets (cymatics/murmuration) are absent from the free menu and ignored here.
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []); // stable — store actions are referentially stable

  const handleKey = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    },
    [close],
  );

  useEffect(() => {
    if (!open) return;

    const canvas = canvasRef.current;
    if (!canvas) return;

    const gl = canvas.getContext("webgl2", {
      antialias: false,
      preserveDrawingBuffer: false,
      alpha: false,
    }) as WebGL2RenderingContext | null;

    if (!gl) {
      console.error("VisualizerOverlay: WebGL2 not available");
      return;
    }

    // The free registry only has Galaxy, so any stale/unknown presetId falls back to it.
    const def = visualizers.find((v) => v.id === presetId) ?? visualizers[0];
    if (!def) return;

    const tracker = new AudioFeatureTracker();
    trackerRef.current = tracker;

    const viz = def.create(gl, {
      theme: () => themeRef.current,
      accent: () => toAccentRgb(accentRef.current),
    });
    vizRef.current = viz;

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    viz.resize(w, h, dpr);

    const ro = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const dpr2 = Math.min(window.devicePixelRatio || 1, 2);
      const rect = entry.contentRect;
      canvas.width = Math.round(rect.width * dpr2);
      canvas.height = Math.round(rect.height * dpr2);
      viz.resize(rect.width, rect.height, dpr2);
    });
    ro.observe(canvas);

    function loop(now: number) {
      const dtMs = lastRef.current === 0 ? 16 : Math.min(now - lastRef.current, 50);
      lastRef.current = now;

      tracker.update(spectrum.read(), dtMs);
      viz.frame(tracker.features, dtMs);

      rafRef.current = requestAnimationFrame(loop);
    }
    rafRef.current = requestAnimationFrame(loop);

    window.addEventListener("keydown", handleKey);

    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      ro.disconnect();
      viz.dispose();
      vizRef.current = null;
      trackerRef.current = null;
      window.removeEventListener("keydown", handleKey);
    };
  }, [open, presetId, handleKey]); // eslint-disable-line react-hooks/exhaustive-deps
  // Note: spectrum intentionally excluded from deps (stable read() reference).

  if (!open) return null;

  const isLight = theme === "light";

  return (
    <div
      className={`${styles.overlay} ${isLight ? styles.overlayLight : ""}`}
      role="dialog"
      aria-modal="true"
      aria-label="Visualizer"
    >
      <canvas ref={canvasRef} className={styles.canvas} />
      <button className={styles.close} onClick={close} aria-label="Close visualizer">
        Close
      </button>
    </div>
  );
}
