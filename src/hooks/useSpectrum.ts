import { usePlayerStore } from "../store/usePlayerStore";
import { nativeEngine } from "../audio/nativeEngine";

/**
 * Headless spectrum source. Returns a non-reactive `read()` for a renderer's requestAnimationFrame
 * loop: the live Rust FFT bands (0..1) while the engine is active, or null when idle (so the
 * visual falls quietly to rest). Keeps `nativeEngine` out of the renderer.
 */
// Stable module-level reader (reads the store + engine singletons at call time), so its identity
// never changes across renders — consumers can safely list it in effect deps without the effect
// (and its rAF loop) tearing down and re-subscribing every render.
const read = (): number[] | null =>
  usePlayerStore.getState().engineActive ? nativeEngine.getBands() : null;

export function useSpectrum() {
  return { read };
}
