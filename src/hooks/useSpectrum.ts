import { usePlayerStore } from "../store/usePlayerStore";
import { nativeEngine } from "../audio/nativeEngine";

/**
 * Headless spectrum source. Returns a non-reactive `read()` for a renderer's requestAnimationFrame
 * loop: the live Rust FFT bands (0..1) while the engine is active, or null when idle (so the
 * visual falls quietly to rest). Keeps `nativeEngine` out of the renderer.
 */
export function useSpectrum() {
  return {
    read: (): number[] | null =>
      usePlayerStore.getState().engineActive ? nativeEngine.getBands() : null,
  };
}
