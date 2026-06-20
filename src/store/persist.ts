import { usePlayerStore } from "./usePlayerStore";
import { useUiStore } from "./useUiStore";
import type { ReplayGainMode, Track } from "../types";

const KEY = "eko.state.v1";

interface Persisted {
  volume: number;
  outputDevice: string | null;
  eqEnabled: boolean;
  preamp: number;
  gains: number[];
  presetName: string | null;
  repeat: "off" | "all" | "one";
  shuffle: boolean;
  replayGainMode: ReplayGainMode;
  crossfadeMs: number;
  timeDisplay: "elapsed" | "remaining";
  zoom: number;
  eqVisible: boolean;
  plVisible: boolean;
  mainShade: boolean;
  alwaysOnTop: boolean;
  // Resume: the last LOCAL queue + position. Server (Navidrome) sessions are not resumed
  // here because their stream URLs depend on a live connection (see resume-session.md).
  lastSession?: { tracks: Track[]; index: number; positionSec: number } | null;
}

/** Snapshot both stores to localStorage. */
export function saveState() {
  const ps = usePlayerStore.getState();
  const ui = useUiStore.getState();
  // Only persist a resumable session for a LOCAL current track (server URLs need a live login).
  const cur = ps.currentIndex != null ? ps.tracks[ps.currentIndex] : null;
  const lastSession =
    cur && !cur.subsonicId
      ? { tracks: ps.tracks, index: ps.currentIndex as number, positionSec: ps.currentTime }
      : null;
  const data: Persisted = {
    volume: ps.volume,
    outputDevice: ps.outputDevice,
    eqEnabled: ps.eqEnabled,
    preamp: ps.preamp,
    gains: ps.gains,
    presetName: ps.presetName,
    repeat: ps.repeat,
    shuffle: ps.shuffle,
    replayGainMode: ps.replayGainMode,
    crossfadeMs: ps.crossfadeMs,
    timeDisplay: ps.timeDisplay,
    zoom: ui.zoom,
    eqVisible: ui.eqVisible,
    plVisible: ui.plVisible,
    mainShade: ui.mainShade,
    alwaysOnTop: ui.alwaysOnTop,
    lastSession,
  };
  try {
    localStorage.setItem(KEY, JSON.stringify(data));
  } catch {
    /* ignore quota errors */
  }
}

/** Restore persisted settings (and playlist, without auto-playing). */
export async function restoreState() {
  let data: Persisted | null = null;
  try {
    data = JSON.parse(localStorage.getItem(KEY) ?? "null");
  } catch {
    /* malformed state — fall back to defaults */
  }
  if (!data) return;

  const ps = usePlayerStore.getState();
  // Restore the saved software volume (applied to the engine on the next track).
  ps.syncSystemVolume(data.volume ?? 0.8);
  ps.setOutputDevice(data.outputDevice ?? null);
  ps.setEqEnabled(data.eqEnabled ?? true);
  ps.setPreamp(data.preamp ?? 0);
  if (Array.isArray(data.gains)) ps.setAllGains(data.gains);
  // setPreamp/setAllGains mark the EQ "Custom"; restore the real preset name last.
  usePlayerStore.setState({ presetName: data.presetName ?? null });
  usePlayerStore.setState({
    repeat: data.repeat ?? "off",
    shuffle: !!data.shuffle,
    timeDisplay: data.timeDisplay ?? "elapsed",
  });
  useUiStore.setState({
    eqVisible: data.eqVisible ?? true,
    plVisible: data.plVisible ?? true,
    mainShade: !!data.mainShade,
    alwaysOnTop: !!data.alwaysOnTop,
  });
  // Route zoom through setZoom so old/out-of-range values get clamped + snapped.
  useUiStore.getState().setZoom(data.zoom ?? 2);

  usePlayerStore.setState({ replayGainMode: data.replayGainMode ?? "off" });
  usePlayerStore.setState({ crossfadeMs: Math.max(0, Math.min(12000, data.crossfadeMs ?? 0)) });

  // Resume the last LOCAL session — paused, never auto-playing. The first play of the
  // restored track seeks to `pendingResumeSec` (see usePlayerStore.playAt). Server queues
  // are not restored (their stream URLs need a live connection).
  const ls = data.lastSession;
  if (ls && Array.isArray(ls.tracks) && ls.tracks.length > 0) {
    // Validate each track: must have a non-empty path and a finite numeric duration.
    const allValid = ls.tracks.every(
      (t) =>
        typeof t.path === "string" &&
        t.path.length > 0 &&
        typeof t.duration === "number" &&
        Number.isFinite(t.duration),
    );
    if (allValid) {
      const idx = Math.min(Math.max(0, ls.index ?? 0), ls.tracks.length - 1);
      const pos = Math.max(0, ls.positionSec || 0);
      usePlayerStore.setState({
        tracks: ls.tracks,
        currentIndex: idx,
        currentTime: pos,
        duration: ls.tracks[idx]?.duration || 0,
        pendingResumeSec: pos > 1 ? pos : null,
        isPlaying: false,
        engineActive: false,
      });
    }
  }
}

/** Subscribe to both stores and persist (debounced). Returns an unsubscribe fn. */
export function startAutosave(): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const schedule = () => {
    if (timer) clearTimeout(timer);
    // 2500 ms debounce: the poll fires ~8×/sec updating currentTime; serialising the full
    // tracks array on every tick wastes CPU. Structural changes (track add/remove, EQ, etc.)
    // still land within a few seconds rather than the old 400 ms.
    timer = setTimeout(saveState, 2500);
  };
  const u1 = usePlayerStore.subscribe(schedule);
  const u2 = useUiStore.subscribe(schedule);
  return () => {
    u1();
    u2();
    if (timer) clearTimeout(timer);
  };
}
