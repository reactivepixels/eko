/**
 * Renderer contract for all EKO GPU visualizers.
 *
 * Every visualizer (Galaxy, Cymatics, Murmuration, …) implements Visualizer and
 * is described by a VisualizerDef.  The overlay creates/destroys them; it drives
 * each frame with the shared AudioFeatures from AudioFeatureTracker.
 */

import type { AudioFeatures } from "../../audio/audioFeatures";

/** Lifecycle API for a live GPU renderer. */
export interface Visualizer {
  /** Canvas was resized (or DPR changed). Called before the first frame too. */
  resize(w: number, h: number, dpr: number): void;
  /**
   * Called once per animation frame while the overlay is open.
   * @param features  Current audio features (shared reference, do not cache).
   * @param dtMs      Frame delta in milliseconds.
   */
  frame(features: AudioFeatures, dtMs: number): void;
  /** Tear down all WebGL objects (called on unmount / preset change). */
  dispose(): void;
}

/** Descriptor for a registered visualizer. */
export interface VisualizerDef {
  /** Unique identifier (used in the store + URL param). */
  id: string;
  /** Display label shown in a future preset picker. */
  label: string;
  /**
   * Factory: create and return a live Visualizer bound to the given GL context.
   * Must NOT start a rAF loop — the overlay drives frames.
   */
  create(
    gl: WebGL2RenderingContext,
    opts: {
      /** Returns the current theme; reactive via closure, read each frame. */
      theme: () => "dark" | "light";
      /** Returns the current accent colour as linear [r, g, b] each in 0..1. */
      accent: () => [number, number, number];
    },
  ): Visualizer;
}
