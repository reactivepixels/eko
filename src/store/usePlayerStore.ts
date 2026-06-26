import { create } from "zustand";
import { toTrack } from "../audio/loader";
import { nativeEngine } from "../audio/nativeEngine";
import type { ParamBand, EqMode } from "../audio/nativeEngine";
import { mediaMetadata, mediaPlayback, mediaStopped } from "../audio/media";
import { streamSrcUrl, coverArtUrl, scrobble } from "../subsonic/client";
import { EQ_BAND_COUNT, EQ_PRESETS, FLAT_GAINS, type EqPreset } from "../audio/constants";
import { offlineEntry, useOfflineStore } from "@pro";
import type { ReplayGainMode, RepeatMode, Track } from "../types";

// ── Scrobble threshold (Last.fm convention) ────────────────────────────────
// A track is scrobbled after 50% of its duration has played, OR 4 minutes —
// whichever comes first — but only once per track load.
export const SCROBBLE_MIN_SECS = 30; // don't scrobble very short clips
export const SCROBBLE_MAX_SECS = 4 * 60; // 4 min cap

/** Return the elapsed-seconds threshold at which to fire the "submission" scrobble. */
export function scrobbleThreshold(durationSec: number): number {
  if (durationSec <= 0) return Infinity;
  return Math.min(durationSec * 0.5, SCROBBLE_MAX_SECS);
}

// ── Sleep-timer state (module-level, not persisted) ────────────────────────
/** Preset durations offered in the UI (minutes), plus the sentinel -1 = end-of-track. */
export const SLEEP_PRESETS = [15, 30, 45, 60] as const;
export type SleepPreset = (typeof SLEEP_PRESETS)[number] | -1; // -1 = end of track

interface SleepTimer {
  /** Wall-clock ms when the timer was started. */
  startedAt: number;
  /** Total duration in ms (Infinity for end-of-track mode). */
  durationMs: number;
  /** Whether this is "end of track" mode (pause when the current track finishes). */
  endOfTrack: boolean;
}

// Active sleep timer and the interval that drives the countdown display.
let _sleepTimer: SleepTimer | null = null;
let _sleepInterval: ReturnType<typeof setInterval> | null = null;

function clearSleepTimer() {
  if (_sleepInterval) {
    clearInterval(_sleepInterval);
    _sleepInterval = null;
  }
  _sleepTimer = null;
  usePlayerStore.setState({ sleepTimer: null });
}

/** Remaining ms on the sleep timer, or null if inactive. */
export function sleepTimerRemaining(): number | null {
  if (!_sleepTimer || _sleepTimer.endOfTrack) return null;
  const remaining = _sleepTimer.durationMs - (Date.now() - _sleepTimer.startedAt);
  return Math.max(0, remaining);
}

// ── Scrobble tracking (module-level, reset on each track) ─────────────────
let _scrobbleId: string | null = null; // id of the track being tracked
let _submissionSent = false;

export type { ParamBand, EqMode };

export interface EngineInfo {
  device: string;
  rate: number; // EKO's output stream rate
  srcRate: number; // file's own sample rate
  devRate: number; // the OS device's actual rate (≠ rate ⇒ macOS is resampling)
  bits: number; // file bit depth (0 = unknown)
  codec: string; // short codec name
  channels: number;
}

let posTimer: ReturnType<typeof setInterval> | null = null;
// Poll generation: incremented on stop so an in-flight await after clearInterval can
// detect it's stale and bail out without touching store state.
let pollGen = 0;
// While the user drags the seek bar we drive `currentTime` ourselves and throttle the
// engine seeks — so the poll must not fight the drag, and we don't flood IPC.
let scrubbing = false;
let lastSeekSent = 0;
// Gapless: which queue index we've already armed the next track for (avoid re-enqueuing
// every poll). Reset on a fresh session and after each gapless advance.
let enqueuedFor: number | null = null;
// Gapless session integrity: set to true when the queue is mutated mid-session (reorder,
// removeTrack, playNext, setQueue without immediate playAt). While dirty the poll skips
// the seg-advance and enqueue-next blocks and clears any armed next-source. Cleared by
// playAt (a fresh session) and clearPlaylist.
let sessionDirty = false;
// Seek convergence: after a seek/click, hold the optimistic position until the engine's
// reported time catches up — otherwise a stale status poll snaps the thumb back to the old
// spot for a frame. Cleared on convergence or when the guard window lapses.
let seekTarget: number | null = null;
let seekGuardUntil = 0;
function markSeek(sec: number) {
  seekTarget = sec;
  seekGuardUntil = Date.now() + 1000;
}
function stopNativePoll() {
  if (posTimer) {
    clearInterval(posTimer);
    posTimer = null;
  }
  // Bump the generation so any in-flight await in the last callback knows to bail.
  pollGen++;
}
function startNativePoll() {
  stopNativePoll();
  const gen = ++pollGen;
  posTimer = setInterval(async () => {
    if (scrubbing) return;
    const st = await nativeEngine.status().catch(() => null);
    // Bail if this interval was cancelled while we were awaiting.
    if (gen !== pollGen) return;
    if (!st) return;

    // Gapless advance: the engine reports the playing-track index within the session
    // (st.seg). When it moves past 0, advance the UI to that track WITHOUT restarting —
    // and the boundary is therefore never seen as "ended".
    // Skip when sessionDirty (queue mutated mid-session — wrong track would be shown).
    const s0 = usePlayerStore.getState();
    if (!sessionDirty) {
      const wantIndex = s0.sessionStartIndex + (st.seg ?? 0);
      if (st.seg > 0 && wantIndex !== s0.currentIndex && wantIndex < s0.tracks.length) {
        usePlayerStore.setState({ currentIndex: wantIndex });
        enqueuedFor = null; // can arm the following track now
        pushNowPlaying();
        pushPlayback();
        applyReplayGain();
      }
    }

    const ended = st.durMs > 0 && st.posMs >= st.durMs - 350 && !st.playing;
    // Hold the clicked/seeked position until the engine's reported time converges to it, so a
    // stale poll never snaps the thumb back. The guard lapses after ~1s as a safety net.
    const engTime = st.posMs / 1000;
    let acceptTime = true;
    if (seekTarget != null && Date.now() < seekGuardUntil) {
      if (Math.abs(engTime - seekTarget) < 0.4) seekTarget = null;
      else acceptTime = false;
    }
    usePlayerStore.setState({
      ...(acceptTime ? { currentTime: engTime } : {}),
      duration: st.durMs / 1000,
      isPlaying: st.playing,
    });

    // ── Scrobble: submission at the play threshold ──────────────────────────
    // Fire once per track when elapsed time crosses the threshold (50% or 4 min).
    const scrobbleState = usePlayerStore.getState();
    if (
      scrobbleState.scrobbleEnabled &&
      _scrobbleId !== null &&
      !_submissionSent &&
      st.playing &&
      st.posMs > 0 &&
      st.durMs > 0
    ) {
      const threshold = scrobbleThreshold(st.durMs / 1000);
      if (engTime >= threshold && st.durMs / 1000 >= SCROBBLE_MIN_SECS) {
        _submissionSent = true;
        void scrobble(_scrobbleId, true);
      }
    }

    // ── Sleep timer: wall-clock countdown ──────────────────────────────────
    if (_sleepTimer && !_sleepTimer.endOfTrack) {
      const remaining = Math.max(0, _sleepTimer.durationMs - (Date.now() - _sleepTimer.startedAt));
      usePlayerStore.setState({
        sleepTimer: {
          endOfTrack: false,
          remainingSec: Math.ceil(remaining / 1000),
          totalSec: Math.round(_sleepTimer.durationMs / 1000),
        },
      });
      if (remaining <= 0 && st.playing) {
        clearSleepTimer();
        void nativeEngine.pause();
        usePlayerStore.setState({ isPlaying: false });
        pushPlayback();
      }
    }

    // Arm the next sequential track for gapless continuation ~12s before the current ends
    // (once per track; only when playing straight through — not shuffle / repeat-one).
    // Skip when sessionDirty and clear any previously armed next-source.
    const cur = usePlayerStore.getState();
    if (sessionDirty) {
      void nativeEngine.enqueue(null, null);
    } else {
      // Arm the next sequential track EARLY — as soon as this track is playing — not near
      // its end. The native decoder races ahead of playback and reaches end-of-decode within
      // a second or two of a local track; the gapless continuation only fires if the next
      // source is already queued at that moment. (Server streams decode at ~1× as
      // they download, so this is also harmless there.) Once consumed the engine clears it;
      // we re-arm when the displayed track advances (enqueuedFor !== currentIndex).
      const sequential = !cur.shuffle && cur.repeat !== "one";
      const nextIdx = cur.currentIndex != null ? cur.currentIndex + 1 : -1;
      if (
        sequential &&
        nextIdx > 0 &&
        nextIdx < cur.tracks.length &&
        enqueuedFor !== cur.currentIndex &&
        st.durMs > 0
      ) {
        const nt = cur.tracks[nextIdx];
        enqueuedFor = cur.currentIndex;
        if (nt.subsonicId) {
          const cached = offlineEntry(useOfflineStore.getState().entries, nt.subsonicId);
          if (cached) {
            void nativeEngine.enqueue(null, null, cached.trackId, cached.bytes);
          } else {
            void nativeEngine.enqueue(null, streamSrcUrl(nt.subsonicId));
          }
        } else {
          void nativeEngine.enqueue(nt.path, null);
        }
      }
    }
    // Signal-path info changes only per track — only write (and re-render) when it does.
    const prev = usePlayerStore.getState().engineInfo;
    if (
      !prev ||
      prev.rate !== st.rate ||
      prev.srcRate !== st.srcRate ||
      prev.devRate !== st.devRate ||
      prev.bits !== st.bits ||
      prev.codec !== st.codec ||
      prev.device !== st.device ||
      prev.channels !== st.channels
    ) {
      usePlayerStore.setState({
        engineInfo: {
          device: st.device,
          rate: st.rate,
          srcRate: st.srcRate,
          devRate: st.devRate,
          bits: st.bits,
          codec: st.codec,
          channels: st.channels,
        },
      });
    }
    if (ended) {
      stopNativePoll();
      // Sleep timer "end of track": pause instead of advancing.
      if (_sleepTimer?.endOfTrack) {
        clearSleepTimer();
        usePlayerStore.setState({ isPlaying: false });
        pushPlayback();
        return;
      }
      void usePlayerStore.getState().next();
    }
  }, 120);
}

interface PlayerState {
  // Playlist
  tracks: Track[];
  currentIndex: number | null;
  // Queue index the current engine session began on; gapless continuations advance the
  // displayed track as `sessionStartIndex + engineSeg` without restarting the session.
  sessionStartIndex: number;

  // Transport
  isPlaying: boolean;
  currentTime: number;
  duration: number;
  timeDisplay: "elapsed" | "remaining";
  engineActive: boolean; // true while the native (local) engine is the audio source
  engineInfo: EngineInfo | null; // live signal-path info from the engine (per track)
  outputDevice: string | null; // preferred DAC name (null = system default)

  // Output
  volume: number; // 0..1

  // EQ
  eqEnabled: boolean;
  preamp: number; // dB
  gains: number[]; // dB, length EQ_BAND_COUNT
  presetName: string | null; // name of the applied preset, or null once edited ("Custom")
  // Parametric EQ (Pro feature)
  eqMode: EqMode; // "graphic" (free/default) | "parametric" (Pro)
  paramEqEnabled: boolean;
  paramEqPreamp: number; // dB
  paramEqBands: ParamBand[];

  // Modes
  repeat: RepeatMode;
  shuffle: boolean;
  replayGainMode: ReplayGainMode; // volume normalisation (off by default)
  rgAppliedDb: number | null; // dB currently applied to the engine (null = none); for the seal

  // Resume
  pendingResumeSec: number | null; // restored position to seek to on the next play

  // Scrobble
  scrobbleEnabled: boolean; // user toggle (default true)

  // Sleep timer (reactive display state — the raw timer lives at module level)
  sleepTimer: {
    endOfTrack: boolean;
    remainingSec: number | null; // null in end-of-track mode (no countdown)
    totalSec: number | null;
  } | null;

  // --- actions ---
  init: () => void;
  addPaths: (paths: string[], autoplay?: boolean) => Promise<void>;
  removeTrack: (id: string) => void;
  clearPlaylist: () => void;
  reorder: (from: number, to: number) => void;
  playAt: (index: number) => Promise<void>;
  setQueue: (tracks: Track[], autoplay?: boolean) => void;
  addToQueue: (tracks: Track[]) => void;
  playNext: (tracks: Track[]) => void;
  togglePlay: () => Promise<void>;
  stop: () => void;
  next: () => Promise<void>;
  prev: () => Promise<void>;
  seek: (seconds: number) => void;
  toggleTimeDisplay: () => void;
  beginScrub: () => void;
  scrubMove: (seconds: number) => void;
  endScrub: (seconds: number) => void;
  setVolume: (v: number) => void;
  syncSystemVolume: (v: number) => void;
  setOutputDevice: (name: string | null) => void;
  setEqEnabled: (on: boolean) => void;
  setPreamp: (db: number) => void;
  setBandGain: (index: number, db: number) => void;
  setAllGains: (gains: number[]) => void;
  applyPreset: (preset: EqPreset) => void;
  // Parametric EQ (Pro)
  setEqMode: (mode: EqMode) => void;
  setParamEqEnabled: (on: boolean) => void;
  setParamEqPreamp: (db: number) => void;
  setParamEqBands: (bands: ParamBand[]) => void;
  cycleRepeat: () => void;
  toggleShuffle: () => void;
  setReplayGainMode: (mode: ReplayGainMode) => void;
  setScrobbleEnabled: (on: boolean) => void;
  startSleepTimer: (preset: SleepPreset) => void;
  cancelSleepTimer: () => void;
}

// Guards against attaching audio element listeners twice (e.g. StrictMode in dev).
let storeInitialized = false;

/** Push the current EQ (enabled + preamp + gains) into the native engine. */
function syncEq() {
  const s = usePlayerStore.getState();
  void nativeEngine.setEq(s.eqEnabled, s.preamp, s.gains);
}

/** Push the current parametric EQ config into the native engine. */
function syncParamEq() {
  const s = usePlayerStore.getState();
  void nativeEngine.setParamEq(s.paramEqEnabled, s.paramEqPreamp, s.paramEqBands);
}

/** Push the EQ mode (graphic/parametric) into the native engine. */
function syncEqMode() {
  const s = usePlayerStore.getState();
  void nativeEngine.setEqMode(s.eqMode);
}

/** Compute the ReplayGain adjustment (dB) for a track under the current mode, peak-limited
 *  so a positive gain can't push the file's peak past full scale (clipping). Returns null
 *  when RG is off or the track has no usable tags (→ engine stays bit-perfect). */
function rgGainDbFor(track: Track | undefined, mode: ReplayGainMode): number | null {
  if (!track || mode === "off") return null;
  const gain = mode === "album" ? (track.rgAlbumGain ?? track.rgTrackGain) : track.rgTrackGain;
  if (gain == null) return null;
  const peak = mode === "album" ? (track.rgAlbumPeak ?? track.rgTrackPeak) : track.rgTrackPeak;
  let g = gain;
  if (peak != null && peak > 0) {
    const maxSafeDb = -20 * Math.log10(peak); // headroom (dB) before the peak clips
    g = Math.min(g, maxSafeDb);
  }
  return g;
}

/** Apply ReplayGain for the current track to the engine and record the applied dB (so the
 *  signal-path seal can show it honestly). A 0 dB adjustment is still bit-perfect. */
function applyReplayGain() {
  const s = usePlayerStore.getState();
  const track = s.currentIndex != null ? s.tracks[s.currentIndex] : undefined;
  const db = rgGainDbFor(track, s.replayGainMode);
  void nativeEngine.setReplayGain(db);
  usePlayerStore.setState({ rgAppliedDb: db != null && Math.abs(db) > 0.01 ? db : null });
}

/** Push current-track metadata into the engine so the mini player can read it directly
 *  from Rust (live even when the main window is hidden + throttled). */
function pushNowPlaying() {
  const s = usePlayerStore.getState();
  const t = s.currentIndex != null ? s.tracks[s.currentIndex] : null;
  let theme = "light";
  try {
    theme = localStorage.getItem("eko.theme") === "dark" ? "dark" : "light";
  } catch {
    /* default */
  }
  void nativeEngine.setNowPlaying({
    title: t?.title ?? "EKO",
    artist: t ? (t.artist ?? "") : "Pick an album",
    coverUrl: t?.coverArt ? (coverArtUrl(t.coverArt, 160) ?? "") : "",
    coverPath: t?.path && !t.subsonicId ? t.path : "",
    theme,
    index: s.currentIndex ?? -1,
    total: s.tracks.length,
  });
  // Mirror to the OS now-playing card (lock screen / Control Center). Server cover art is a
  // URL the OS can fetch; local embedded art has no URL, so it's omitted.
  if (t) {
    mediaMetadata({
      title: t.title ?? "Unknown",
      artist: t.artist ?? "",
      album: t.album ?? "",
      coverUrl: t.coverArt ? (coverArtUrl(t.coverArt, 512) ?? undefined) : undefined,
      duration: t.duration,
    });
  }
}

/** Push the current play/pause state + elapsed position to the OS now-playing card.
 *  Only needs calling on transitions — macOS extrapolates the running clock itself. */
function pushPlayback() {
  const s = usePlayerStore.getState();
  mediaPlayback(s.isPlaying, s.currentTime);
}

/** Push the current volume (dial 0..1) into the native engine, throttled to ~20/sec so a
 *  drag can't flood the IPC bridge. Sends immediately, then trails the final value. */
let volTimer: ReturnType<typeof setTimeout> | null = null;
let volPending = false;
function syncVol() {
  if (volTimer) {
    volPending = true;
    return;
  }
  void nativeEngine.setVolume(usePlayerStore.getState().volume);
  volTimer = setTimeout(() => {
    volTimer = null;
    if (volPending) {
      volPending = false;
      syncVol();
    }
  }, 50);
}

export const usePlayerStore = create<PlayerState>((set, get) => ({
  tracks: [],
  currentIndex: null,
  sessionStartIndex: 0,
  isPlaying: false,
  currentTime: 0,
  duration: 0,
  timeDisplay: "elapsed",
  engineActive: false,
  engineInfo: null,
  outputDevice: null,
  volume: 0.8,
  eqEnabled: true,
  preamp: 0,
  gains: FLAT_GAINS(),
  presetName: "Flat",
  eqMode: "graphic" as EqMode,
  paramEqEnabled: false,
  paramEqPreamp: 0,
  paramEqBands: [],
  repeat: "off",
  shuffle: false,
  replayGainMode: "off",
  rgAppliedDb: null,
  pendingResumeSec: null,
  scrobbleEnabled: true,
  sleepTimer: null,

  init: () => {
    if (storeInitialized) return;
    storeInitialized = true;
    // Playback state is driven by the native engine poll; nothing to wire here but the
    // initial EQ push (a no-op until a track starts a session).
    syncEq();
    syncEqMode();
    syncParamEq();
  },

  addPaths: async (paths, autoplay = true) => {
    const newTracks = await Promise.all(paths.map((p) => toTrack(p)));
    const wasEmpty = get().tracks.length === 0;
    set((s) => ({ tracks: [...s.tracks, ...newTracks] }));
    // Auto-select the first added track if nothing is loaded yet.
    if (autoplay && wasEmpty && newTracks.length > 0) {
      await get().playAt(0);
    }
  },

  removeTrack: (id) => {
    set((s) => {
      const idx = s.tracks.findIndex((t) => t.id === id);
      if (idx === -1) return s;
      const tracks = s.tracks.filter((t) => t.id !== id);
      let currentIndex = s.currentIndex;
      if (currentIndex !== null) {
        if (idx < currentIndex) currentIndex -= 1;
        else if (idx === currentIndex) currentIndex = null;
      }
      return { tracks, currentIndex };
    });
    sessionDirty = true;
  },

  clearPlaylist: () => {
    void nativeEngine.stop();
    nativeEngine.stopBands();
    stopNativePoll();
    enqueuedFor = null;
    sessionDirty = false;
    _scrobbleId = null;
    _submissionSent = false;
    clearSleepTimer();
    set({
      tracks: [],
      currentIndex: null,
      sessionStartIndex: 0,
      isPlaying: false,
      currentTime: 0,
      duration: 0,
      engineActive: false,
    });
    mediaStopped();
  },

  reorder: (from, to) => {
    set((s) => {
      const tracks = [...s.tracks];
      const [moved] = tracks.splice(from, 1);
      tracks.splice(to, 0, moved);
      let currentIndex = s.currentIndex;
      if (currentIndex === from) currentIndex = to;
      else if (currentIndex !== null && from < currentIndex && to >= currentIndex)
        currentIndex -= 1;
      else if (currentIndex !== null && from > currentIndex && to <= currentIndex)
        currentIndex += 1;
      return { tracks, currentIndex };
    });
    sessionDirty = true;
  },

  playAt: async (index) => {
    const { tracks, currentIndex: prevIndex, pendingResumeSec } = get();
    const track = tracks[index];
    if (!track) return;
    // Resume only applies to the exact track that was restored; any other play clears it.
    const resumeSec = pendingResumeSec != null && index === prevIndex ? pendingResumeSec : null;
    // A manual play starts a fresh engine session at this index; re-arm gapless from here.
    enqueuedFor = null;
    sessionDirty = false;
    set({
      currentIndex: index,
      sessionStartIndex: index,
      duration: track.duration || 0,
      currentTime: resumeSec ?? 0,
      isPlaying: true,
      engineActive: true,
      pendingResumeSec: null,
    });
    // ── Scrobble: reset per-track state and fire "now playing" ────────────
    _scrobbleId = track.subsonicId ?? null;
    _submissionSent = false;
    if (track.subsonicId && get().scrobbleEnabled) {
      void scrobble(track.subsonicId, false);
    }

    // ── Sleep timer: "end of track" mode — cancel on new track (pause already fired) ──
    // Only cancel if it's the end-of-track variant; fixed-duration timers survive track changes.
    if (_sleepTimer?.endOfTrack) {
      clearSleepTimer();
    }

    // Route: cached offline (EncryptedFileSource, bit-perfect) → local file → server stream.
    if (track.subsonicId) {
      const cached = offlineEntry(useOfflineStore.getState().entries, track.subsonicId);
      if (cached) {
        // Play from the encrypted local cache — no network, fully bit-perfect.
        void nativeEngine.playCached(cached.trackId, cached.bytes);
      } else {
        void nativeEngine.playUrl(streamSrcUrl(track.subsonicId));
      }
    } else {
      void nativeEngine.play(track.path);
    }
    startNativePoll();
    nativeEngine.startBands();
    syncEq();
    syncEqMode();
    syncParamEq();
    syncVol();
    applyReplayGain();
    pushNowPlaying();
    pushPlayback();
    // Restored session: once decode has buffered, seek to where we left off.
    if (resumeSec != null && resumeSec > 0) {
      setTimeout(() => void nativeEngine.seek(resumeSec), 500);
    }
  },

  setQueue: (tracks, autoplay = true) => {
    set({ tracks, currentIndex: null });
    if (autoplay && tracks.length > 0) {
      void get().playAt(0);
    } else {
      // Queue replaced without an immediate playAt — the current gapless session's
      // sessionStartIndex and enqueuedFor are now stale.
      sessionDirty = true;
    }
  },

  // Append to the end of the queue (starts playback if nothing is loaded).
  addToQueue: (add) => {
    if (add.length === 0) return;
    const empty = get().tracks.length === 0;
    set((s) => ({ tracks: [...s.tracks, ...add] }));
    if (empty) void get().playAt(0);
  },

  // Insert right after the current track (or at the front if nothing is playing).
  playNext: (add) => {
    if (add.length === 0) return;
    const { currentIndex, tracks } = get();
    if (tracks.length === 0) {
      void get().setQueue(add, true);
      return;
    }
    const at = currentIndex != null ? currentIndex + 1 : 0;
    set((s) => ({ tracks: [...s.tracks.slice(0, at), ...add, ...s.tracks.slice(at)] }));
    sessionDirty = true;
  },

  togglePlay: async () => {
    const { isPlaying, currentIndex, tracks, engineActive } = get();
    if (currentIndex === null) {
      // Nothing loaded yet — start the queue if there is one, otherwise no-op.
      if (tracks.length > 0) await get().playAt(0);
      return;
    }
    if (isPlaying) {
      void nativeEngine.pause();
      set({ isPlaying: false });
      pushPlayback();
    } else if (!engineActive) {
      // A restored (resumed) session has a selected track but no live engine session yet —
      // start it fresh (playAt consumes pendingResumeSec to seek to the saved position).
      await get().playAt(currentIndex);
    } else {
      void nativeEngine.resume();
      set({ isPlaying: true });
      pushPlayback();
    }
  },

  stop: () => {
    void nativeEngine.stop();
    nativeEngine.stopBands();
    stopNativePoll();
    enqueuedFor = null;
    set({ isPlaying: false, currentTime: 0, engineActive: false });
    mediaStopped();
  },

  next: async () => {
    const { tracks, currentIndex, repeat, shuffle } = get();
    if (tracks.length === 0) return;
    if (repeat === "one" && currentIndex !== null) {
      await get().playAt(currentIndex);
      return;
    }
    let nextIndex: number;
    if (shuffle) {
      if (tracks.length > 1 && currentIndex !== null) {
        // Exclude the current track so shuffle never repeats back-to-back.
        let candidate: number;
        do {
          candidate = Math.floor(Math.random() * tracks.length);
        } while (candidate === currentIndex);
        nextIndex = candidate;
      } else {
        nextIndex = Math.floor(Math.random() * tracks.length);
      }
    } else if (currentIndex === null) {
      nextIndex = 0;
    } else if (currentIndex + 1 < tracks.length) {
      nextIndex = currentIndex + 1;
    } else if (repeat === "all") {
      nextIndex = 0;
    } else {
      get().stop();
      return;
    }
    await get().playAt(nextIndex);
  },

  prev: async () => {
    const { tracks, currentIndex, currentTime } = get();
    if (tracks.length === 0 || currentIndex === null) return;
    // Restart current track if more than 3s in, else go to previous.
    if (currentTime > 3) {
      get().seek(0);
      return;
    }
    const prevIndex = currentIndex - 1 >= 0 ? currentIndex - 1 : 0;
    await get().playAt(prevIndex);
  },

  seek: (seconds) => {
    markSeek(seconds);
    void nativeEngine.seek(seconds);
    set({ currentTime: seconds });
    pushPlayback();
  },

  toggleTimeDisplay: () =>
    set((s) => ({ timeDisplay: s.timeDisplay === "elapsed" ? "remaining" : "elapsed" })),

  // --- Seek-bar scrubbing: move the thumb optimistically, throttle the real seek ---
  beginScrub: () => {
    scrubbing = true;
    lastSeekSent = 0; // the first move (a click) seeks immediately, not throttled
  },
  scrubMove: (seconds) => {
    set({ currentTime: seconds });
    const now = Date.now();
    if (now - lastSeekSent > 80) {
      lastSeekSent = now;
      markSeek(seconds);
      void nativeEngine.seek(seconds);
    }
  },
  endScrub: (seconds) => {
    scrubbing = false;
    markSeek(seconds);
    set({ currentTime: seconds });
    void nativeEngine.seek(seconds);
    pushPlayback();
  },

  // User moved the dial → EKO's own software volume in the engine (instant, EKO-only).
  setVolume: (v) => {
    const vol = Math.min(1, Math.max(0, v));
    set({ volume: vol });
    syncVol();
  },

  // Restore a persisted volume (no engine session may exist yet — playAt re-syncs).
  syncSystemVolume: (v) => {
    const vol = Math.min(1, Math.max(0, v));
    set({ volume: vol });
    syncVol();
  },

  // Choose the output DAC. Takes effect on the next track; if something is playing we
  // re-arm the current track on the new device so the switch is immediate.
  setOutputDevice: (name) => {
    set({ outputDevice: name });
    void nativeEngine.setDevice(name);
    const s = get();
    if (s.currentIndex != null && s.isPlaying) void s.playAt(s.currentIndex);
  },

  setEqEnabled: (on) => {
    set({ eqEnabled: on });
    syncEq();
  },

  setPreamp: (db) => {
    set({ preamp: db, presetName: null });
    syncEq();
  },

  setBandGain: (index, db) => {
    set((s) => {
      const gains = [...s.gains];
      gains[index] = db;
      return { gains, presetName: null };
    });
    syncEq();
  },

  setAllGains: (gains) => {
    const padded = gains.slice(0, EQ_BAND_COUNT);
    set({ gains: padded, presetName: null });
    syncEq();
  },

  applyPreset: (preset) => {
    set({ preamp: preset.preamp, gains: [...preset.gains], presetName: preset.name });
    syncEq();
  },

  setEqMode: (mode) => {
    set({ eqMode: mode });
    syncEqMode();
  },

  setParamEqEnabled: (on) => {
    set({ paramEqEnabled: on });
    syncParamEq();
  },

  setParamEqPreamp: (db) => {
    set({ paramEqPreamp: db });
    syncParamEq();
  },

  setParamEqBands: (bands) => {
    set({ paramEqBands: bands });
    syncParamEq();
  },

  cycleRepeat: () =>
    set((s) => ({
      repeat: s.repeat === "off" ? "all" : s.repeat === "all" ? "one" : "off",
    })),

  toggleShuffle: () => set((s) => ({ shuffle: !s.shuffle })),

  // Volume normalisation. Off keeps the bit-perfect path; track/album apply the file's
  // ReplayGain tag (peak-limited) to the current and subsequent tracks.
  setReplayGainMode: (mode) => {
    set({ replayGainMode: mode });
    applyReplayGain();
  },

  setScrobbleEnabled: (on) => {
    set({ scrobbleEnabled: on });
  },

  startSleepTimer: (preset) => {
    // Clear any existing timer first.
    if (_sleepInterval) clearInterval(_sleepInterval);
    _sleepInterval = null;

    if (preset === -1) {
      // End-of-track mode: no countdown, just set the flag.
      _sleepTimer = { startedAt: Date.now(), durationMs: Infinity, endOfTrack: true };
      set({ sleepTimer: { endOfTrack: true, remainingSec: null, totalSec: null } });
    } else {
      const durationMs = preset * 60 * 1000;
      _sleepTimer = { startedAt: Date.now(), durationMs, endOfTrack: false };
      set({
        sleepTimer: {
          endOfTrack: false,
          remainingSec: preset * 60,
          totalSec: preset * 60,
        },
      });
      // The poll loop handles countdown ticks; no separate interval needed.
    }
  },

  cancelSleepTimer: () => {
    clearSleepTimer();
  },
}));

export { EQ_PRESETS };

/** Pause the main status poll while the mini window is active (compact mode).
 *  Call `resumeMainPoll` when returning to the full player and a track is playing. */
export function pauseMainPoll() {
  stopNativePoll();
  nativeEngine.stopBands();
}

/** Restart the main status poll when leaving compact mode with a track active. */
export function resumeMainPoll() {
  startNativePoll();
  nativeEngine.startBands();
}
