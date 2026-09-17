import { create } from "zustand";
import { toTrack } from "../audio/loader";
import { nativeEngine, player } from "../audio/nativeEngine";
import type {
  ParamBand,
  EqMode,
  PlayerSnapshot,
  Poll,
  SealInput,
  SignalPathReport,
} from "../audio/nativeEngine";
import { coverAt } from "../subsonic/nativeSubsonic";
import { EQ_BAND_COUNT, EQ_PRESETS, FLAT_GAINS, type EqPreset } from "../audio/constants";
import type { ReplayGainMode, RepeatMode, Track } from "../types";
import { toQueueItem, withQids } from "./queueItems";

// ── Sleep timer ─────────────────────────────────────────────────────────────
// The engine keeps the timer (so it fires with this window asleep); the store only shows it.
/** Preset durations offered in the UI (minutes), plus the sentinel -1 = end-of-track. */
export const SLEEP_PRESETS = [15, 30, 45, 60] as const;
export type SleepPreset = (typeof SLEEP_PRESETS)[number] | -1; // -1 = end of track

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
// The queue slot the poll last saw playing. A different one means the engine changed track
// by itself (a gapless seam, auto-advance, a media key, the mini player), so the seal's
// inputs are stale.
let lastUid: string | null = null;
let lastError: string | null = null;
// Bumped whenever the store sends the engine something. A poll already in flight describes
// the engine from before that, so its answer is dropped rather than folded in over the
// store's own, newer state.
let intentGen = 0;
function intent() {
  intentGen++;
}
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
    const at = intentGen;
    const poll = await player.poll().catch(() => null);
    // Bail if this interval was cancelled, or the store told the engine something since.
    if (gen !== pollGen || at !== intentGen || !poll) return;
    applyPoll(poll);
  }, 120);
}

/**
 * Fold one engine poll into the store. The engine decides what is playing; this only
 * reports it. Exported for `usePlayerStore.test.ts`.
 */
export function applyPoll({ status: st, player: pl }: Poll) {
  const s0 = usePlayerStore.getState();

  if (pl.uid !== lastUid) {
    lastUid = pl.uid;
    const idx = pl.uid == null ? -1 : s0.tracks.findIndex((t) => t.qid === pl.uid);
    usePlayerStore.setState({
      currentIndex: idx >= 0 ? idx : pl.uid == null ? null : s0.currentIndex,
      // The outgoing track's stream info now describes the wrong track. Drop it, exactly as
      // `playAt` does, so no seal can be derived from it.
      engineInfo: null,
      rgAppliedDb: pl.rgSealDb,
    });
    clearSignalPath();
    pushNowPlaying();
  } else if (pl.rgSealDb !== s0.rgAppliedDb) {
    usePlayerStore.setState({ rgAppliedDb: pl.rgSealDb });
    refreshSignalPath();
  }

  if (st && pl.active) {
    const engTime = st.posMs / 1000;
    // Hold the clicked/seeked position until the engine's reported time converges to it, so a
    // stale poll never snaps the thumb back. The guard lapses after ~1s as a safety net.
    let acceptTime = true;
    if (seekTarget != null && Date.now() < seekGuardUntil) {
      if (Math.abs(engTime - seekTarget) < 0.4) seekTarget = null;
      else acceptTime = false;
    }
    usePlayerStore.setState({
      ...(acceptTime ? { currentTime: engTime } : {}),
      duration: st.durMs / 1000,
      buffered: st.bufferedMs / 1000,
    });

    // Signal-path info changes only per track. Only write (and re-render) when it does.
    const prev = usePlayerStore.getState().engineInfo;
    const infoChanged =
      !prev ||
      prev.rate !== st.rate ||
      prev.srcRate !== st.srcRate ||
      prev.devRate !== st.devRate ||
      prev.bits !== st.bits ||
      prev.codec !== st.codec ||
      prev.device !== st.device ||
      prev.channels !== st.channels;
    if (infoChanged) {
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
    // Re-derive the seal (in Rust) on the per-track edge, and whenever it is missing while
    // the engine is reporting. Self-terminating: it stops once one lands.
    if (infoChanged || usePlayerStore.getState().signalPath == null) refreshSignalPath();
  }

  usePlayerStore.setState({
    isPlaying: pl.playing,
    engineActive: pl.active,
    sleepTimer: sleepDisplay(pl, s0.sleepTimer),
  });
  if (pl.active && !s0.engineActive) nativeEngine.startBands();
  if (!pl.active && s0.engineActive) {
    nativeEngine.stopBands();
    clearSignalPath();
  }
  if (pl.error !== lastError) {
    lastError = pl.error;
    // The store has no error surface yet. Say it where it can be found.
    if (pl.error) console.error(`EKO: ${pl.error}`);
  }
}

/** The sleep timer as the transport shows it, from what the engine is keeping. */
function sleepDisplay(
  pl: PlayerSnapshot,
  prev: PlayerState["sleepTimer"],
): PlayerState["sleepTimer"] {
  if (pl.stopAfterCurrent) return { endOfTrack: true, remainingSec: null, totalSec: null };
  if (pl.sleepRemainingMs == null) return null;
  const remainingSec = Math.ceil(pl.sleepRemainingMs / 1000);
  return { endOfTrack: false, remainingSec, totalSec: prev?.totalSec ?? remainingSec };
}

interface PlayerState {
  // Playlist
  tracks: Track[];
  currentIndex: number | null;

  // Transport
  isPlaying: boolean;
  currentTime: number;
  duration: number;
  /** Decode-buffered position (seconds) within the current track. Equals `duration` for a
   *  local / fully-decoded track; lags it while a server stream downloads. Drives the
   *  scrubber's "buffered" fill and shows when an armed forward-seek is still buffering. */
  buffered: number;
  timeDisplay: "elapsed" | "remaining";
  engineActive: boolean; // true while the native (local) engine is the audio source
  engineInfo: EngineInfo | null; // live signal-path info from the engine (per track)
  /** The bit-perfect seal, as derived by Rust. Null = not derived yet; consumers must
   *  render nothing rather than assume anything. Never write this from the frontend —
   *  `refreshSignalPath()` owns it. */
  signalPath: SignalPathReport | null;
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
  /** The dB the seal should report (null = the seal treats ReplayGain as inactive).
   *  Decided by Rust and dead-banded there — NOT the raw dB sent to the engine, which can
   *  be a negligible non-zero value that must still read as bit-perfect. */
  rgAppliedDb: number | null;

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
  /** Push everything the engine keeps for itself (queue, modes, gain mode, scrobbling,
   *  DSP), once the last run's state has been restored. */
  resyncEngine: () => void;
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

// ── The bit-perfect seal ───────────────────────────────────────────────────
//
// The seal is derived in Rust (`eko_core::signal_path::derive`) so the desktop app and
// the terminal client cannot report different things about the same playback. That
// derivation is only reachable over async IPC, so it is refreshed HERE — once per
// change to one of its inputs — and cached in the store. `useSignalPath` is then a
// synchronous store read, exactly as it was before the move:
//
//   * every consumer of the seal reads the same object, so two seals on screen can
//     never disagree with each other;
//   * nothing is derived per render, so there is no render loop and no per-frame IPC;
//   * `signalPath` is null until the first real derivation lands, and consumers render
//     NOTHING while it is null. The seal never shows a default or a guess — a seal
//     that flickered to BIT-PERFECT while resampling would be a lying seal.
//
// Every input funnels through the `sync*` helpers below, `applyReplayGain`, or the
// native poll's per-track `engineInfo` write, so those are the only call sites needed.
//
// Moving the derivation into Rust made it ASYNCHRONOUS, and that opened a window the
// synchronous TypeScript version never had: between an input changing and the reply
// landing, the cached verdict describes the *previous* settings. Drag the volume off
// unity on a bit-perfect track and, for that window, the seal was still rendering a
// green BIT-PERFECT over samples the engine had already begun attenuating.
//
// `unconfirmSeal()` below closes it. The asymmetry that makes this safe:
//
//   * the frontend can always withdraw a claim without asking Rust — withdrawing
//     asserts nothing;
//   * the frontend can NEVER make one. Resampling and OS-resampling are engine facts,
//     so volume back at unity and a flat EQ still do not add up to BIT-PERFECT.
//
// So: downgrade eagerly, upgrade lazily. Nothing here derives a seal, and nothing here
// invents a modifier — the verdict is still 100% Rust's.
let sealGen = 0;

/**
 * The seal label shown while a derivation is in flight over settings the cached verdict
 * no longer covers. Deliberately NOT a verdict: it names no modifier and makes no claim,
 * because at this point the frontend genuinely does not know one. Rust replaces it a
 * round trip later with the real thing.
 */
const CHECKING_SEAL_LABEL = "CHECKING…";

/**
 * Withdraw the cached seal's bit-perfect claim, synchronously, because one of its inputs
 * just changed and the reply that would confirm it has not landed yet.
 *
 * Only ever fires on the one transition that can lie — a cached `pure` seal. A cached
 * NON-pure seal is left completely alone: it is not overclaiming, and rewriting it would
 * make its label churn on every tick of a volume drag.
 *
 * It also never blanks. `clearSignalPath()` is the honest response to "nothing is
 * playing", but using it here would unmount the whole signal-path row mid-drag (see
 * `SignalPath.tsx`'s `if (!sp.active) return null`) and make the seal blink. Everything
 * that does not depend on the changed input — SOURCE, OUTPUT, RG, `active` — is kept
 * exactly as it was, so only the claim itself changes.
 *
 * Does NOT bump `sealGen`: the pending reply is the authority and must still win.
 */
function unconfirmSeal() {
  const cached = usePlayerStore.getState().signalPath;
  if (!cached?.pure) return;
  usePlayerStore.setState({
    signalPath: {
      ...cached,
      pure: false,
      // Non-committal on purpose. The five modifier flags come through the spread above
      // untouched — all false, because they came off a pure seal — and are deliberately
      // NOT guessed at: this state means "no modifier is known yet", never "no modifier
      // exists". Naming one here (`"VOLUME"`) would be deriving the seal in TypeScript,
      // which is the one thing the front end must never do.
      sealLabel: CHECKING_SEAL_LABEL,
      // Empty, so the long-form tooltip cannot read "Processing: Bit-perfect". Consumers
      // treat an empty `engineLabel` on a non-pure seal as "no claim yet".
      engineLabel: "",
    },
  });
}

/** The exact snapshot Rust derives from. Built in one place so a reply can be checked
 *  against the settings that are current when it lands, not just when it was sent. */
function sealInputs(): SealInput {
  const s = usePlayerStore.getState();
  return {
    engineActive: s.engineActive,
    info: s.engineInfo
      ? {
          rate: s.engineInfo.rate,
          srcRate: s.engineInfo.srcRate,
          devRate: s.engineInfo.devRate,
          bits: s.engineInfo.bits,
          codec: s.engineInfo.codec,
          device: s.engineInfo.device,
        }
      : null,
    eq: {
      mode: s.eqMode,
      enabled: s.eqEnabled,
      preamp: s.preamp,
      gains: s.gains,
      paramEnabled: s.paramEqEnabled,
      paramPreamp: s.paramEqPreamp,
      paramBands: s.paramEqBands,
    },
    volume: s.volume,
    replaygainDb: s.rgAppliedDb,
    replaygainMode: s.replayGainMode,
  };
}

/** Whether two snapshots describe the same playback. Both are plain data built by the
 *  single literal above, so their key order is identical and a serialised compare is
 *  exact — no field can be added to the payload and silently skipped here. */
function sameSealInputs(a: SealInput, b: SealInput): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Snapshot the seal's inputs and hand them to Rust. Never derives anything locally. */
function refreshSignalPath() {
  // The inputs have moved; the cached verdict no longer covers them.
  unconfirmSeal();
  const input = sealInputs();
  const gen = ++sealGen;
  void nativeEngine
    .signalPath(input)
    .then((sp) => {
      // Last request wins: a volume drag fires these faster than they can resolve, and
      // an out-of-order reply would show a seal for settings that no longer apply.
      if (gen !== sealGen) return;
      usePlayerStore.setState({ signalPath: sp });
      // A verdict is only valid for the inputs it was derived from. `sealGen` catches a
      // reply that a NEWER request has superseded, but `syncVol`'s throttle can move the
      // volume without sending one — so dragging down through unity could land a reply
      // derived at 1.0, repainting a green BIT-PERFECT over an attenuated stream. The
      // throttle's trailing call re-requests within 50 ms; until it answers, claim nothing.
      if (!sameSealInputs(input, sealInputs())) unconfirmSeal();
    })
    .catch(() => {
      // The seal cannot be guessed. Drop it rather than show a stale or invented one.
      if (gen === sealGen) usePlayerStore.setState({ signalPath: null });
    });
}

/** Drop the seal — nothing is going through the engine, so "no seal" is the honest
 *  state. Bumps the generation so an in-flight derivation can't land afterwards. */
function clearSignalPath() {
  sealGen++;
  usePlayerStore.setState({ signalPath: null });
}

/** Push the current EQ (enabled + preamp + gains) into the native engine. */
function syncEq() {
  const s = usePlayerStore.getState();
  void nativeEngine.setEq(s.eqEnabled, s.preamp, s.gains);
  refreshSignalPath();
}

/** Push the current parametric EQ config into the native engine. */
function syncParamEq() {
  const s = usePlayerStore.getState();
  void nativeEngine.setParamEq(s.paramEqEnabled, s.paramEqPreamp, s.paramEqBands);
  refreshSignalPath();
}

/** Push the EQ mode (graphic/parametric) into the native engine. */
function syncEqMode() {
  const s = usePlayerStore.getState();
  void nativeEngine.setEqMode(s.eqMode);
  refreshSignalPath();
}

/**
 * Set the ReplayGain mode in the engine, and record the dB the seal should report.
 *
 * BOTH numbers come from Rust (`eko_core::signal_path::replaygain_decision`), decided in the
 * engine from the playing track's tags, which it already holds from the queue. Neither is
 * computed here, and neither may be: the dead-band is the boundary that decides whether EKO
 * claims bit-perfect. The engine applies the gain itself, on every track, including those it
 * reaches while this window sleeps.
 *
 * Async, so `rgGen` guards against an out-of-order reply.
 */
let rgGen = 0;
function applyReplayGain() {
  // This one refreshes the seal only in its `.then()`, so without this the cached verdict
  // would survive the WHOLE round trip after the ReplayGain picker moved.
  unconfirmSeal();
  intent();
  const gen = ++rgGen;
  void player
    .setReplayGain(usePlayerStore.getState().replayGainMode)
    .then(({ sealDb }) => {
      if (gen !== rgGen) return;
      usePlayerStore.setState({ rgAppliedDb: sealDb });
      refreshSignalPath();
    })
    .catch(() => {
      if (gen !== rgGen) return;
      // Could not decide the gain. "No ReplayGain" is NOT a safe fallback: it is itself an
      // assertion, and the strongest one this product makes, and the engine may already be
      // applying a real gain.
      //
      // So do both: clear the gain in the engine so it matches what we can honestly claim,
      // and DROP the seal rather than derive one. `clearSignalPath` also bumps `sealGen`, so
      // no in-flight derivation can land after this.
      void nativeEngine.setReplayGain(null);
      usePlayerStore.setState({ rgAppliedDb: null });
      clearSignalPath();
    });
}

/** Push current-track metadata into the engine for the mini player. When the engine has a
 *  queue item it overrides title, artist, cover and position with its own, so this mainly
 *  carries the theme. The lock-screen card is updated from Rust on every track change. */
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
    coverUrl: coverAt(t?.coverUrl, 160) ?? "",
    coverPath: t?.path && !t.subsonicId ? t.path : "",
    theme,
    index: s.currentIndex ?? -1,
    total: s.tracks.length,
  });
}

/** Push the current volume (dial 0..1) into the native engine, throttled to ~20/sec so a
 *  drag can't flood the IPC bridge. Sends immediately, then trails the final value. */
let volTimer: ReturnType<typeof setTimeout> | null = null;
let volPending = false;
function syncVol() {
  // BEFORE the throttle gate, not after: a drag's 2nd..Nth ticks return early below and
  // never reach `refreshSignalPath`, so a claim withdrawn only there would leave the
  // in-flight reply for the PREVIOUS volume landing as a fresh green BIT-PERFECT over an
  // already-attenuated stream — the same lie, just 50 ms later.
  unconfirmSeal();
  if (volTimer) {
    volPending = true;
    return;
  }
  void nativeEngine.setVolume(usePlayerStore.getState().volume);
  // Inherits this throttle, so a drag refreshes the seal ~20/sec rather than per frame.
  refreshSignalPath();
  volTimer = setTimeout(() => {
    volTimer = null;
    if (volPending) {
      volPending = false;
      syncVol();
    }
  }, 50);
}

/** Hand the engine the whole queue. It owns what plays next, and this is how it hears about
 *  every change to the list. */
function syncQueue() {
  intent();
  void player.sync(usePlayerStore.getState().tracks.map(toQueueItem));
}

export const usePlayerStore = create<PlayerState>((set, get) => ({
  tracks: [],
  currentIndex: null,
  isPlaying: false,
  currentTime: 0,
  duration: 0,
  buffered: 0,
  timeDisplay: "elapsed",
  engineActive: false,
  engineInfo: null,
  signalPath: null,
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
    // The engine keeps DSP settings across sessions, so they can be pushed before any play.
    syncEq();
    syncEqMode();
    syncParamEq();
    // Always polling: the engine can start, move on or stop by itself (a media key, the
    // mini player, the end of the queue), and the transport has to follow.
    startNativePoll();
  },

  resyncEngine: () => {
    const s = get();
    intent();
    void player.setModes(s.repeat, s.shuffle);
    void player.setScrobble(s.scrobbleEnabled);
    syncEq();
    syncEqMode();
    syncParamEq();
    syncVol();
    applyReplayGain();
    const items = s.tracks.map(toQueueItem);
    if (s.currentIndex != null && items.length > 0) {
      void player.restore(items, s.currentIndex, (s.pendingResumeSec ?? 0) * 1000);
    } else {
      void player.sync(items);
    }
  },

  addPaths: async (paths, autoplay = true) => {
    const newTracks = withQids(await Promise.all(paths.map((p) => toTrack(p))));
    const wasEmpty = get().tracks.length === 0;
    set((s) => ({ tracks: [...s.tracks, ...newTracks] }));
    syncQueue();
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
    syncQueue();
  },

  clearPlaylist: () => {
    intent();
    void player.clear();
    nativeEngine.stopBands();
    lastUid = null;
    set({
      tracks: [],
      currentIndex: null,
      isPlaying: false,
      currentTime: 0,
      duration: 0,
      buffered: 0,
      engineActive: false,
      sleepTimer: null,
    });
    clearSignalPath();
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
    syncQueue();
  },

  playAt: async (index) => {
    const { tracks, currentIndex: prevIndex, pendingResumeSec } = get();
    const track = tracks[index];
    if (!track?.qid) return;
    // The engine resumes a restored track itself; this only shows where it will start.
    const resumeSec = pendingResumeSec != null && index === prevIndex ? pendingResumeSec : null;
    intent();
    lastUid = track.qid;
    set({
      currentIndex: index,
      duration: track.duration || 0,
      currentTime: resumeSec ?? 0,
      isPlaying: true,
      engineActive: true,
      pendingResumeSec: null,
      // Drop the OUTGOING track's stream info. It is the seal's dominant input, and until
      // the poll reports the incoming stream it describes the wrong track: a 44.1 kHz
      // source followed by a 96 kHz one the device will resample would go on claiming
      // BIT-PERFECT. Cleared, `derive` returns `active: false` and every consumer renders
      // nothing until the truth arrives.
      engineInfo: null,
    });
    clearSignalPath();
    void player.play(track.qid);
    nativeEngine.startBands();
    pushNowPlaying();
  },

  setQueue: (tracks, autoplay = true) => {
    set({ tracks: withQids(tracks), currentIndex: null });
    syncQueue();
    if (autoplay && tracks.length > 0) {
      void get().playAt(0);
    }
  },

  // Append to the end of the queue (starts playback if nothing is loaded).
  addToQueue: (add) => {
    if (add.length === 0) return;
    const empty = get().tracks.length === 0;
    set((s) => ({ tracks: [...s.tracks, ...withQids(add)] }));
    syncQueue();
    if (empty) void get().playAt(0);
  },

  // Insert right after the current track (or at the front if nothing is playing).
  playNext: (add) => {
    if (add.length === 0) return;
    const { currentIndex, tracks } = get();
    if (tracks.length === 0) {
      get().setQueue(add, true);
      return;
    }
    const at = currentIndex != null ? currentIndex + 1 : 0;
    const inserted = withQids(add);
    set((s) => ({ tracks: [...s.tracks.slice(0, at), ...inserted, ...s.tracks.slice(at)] }));
    syncQueue();
  },

  togglePlay: async () => {
    const { tracks, isPlaying } = get();
    if (tracks.length === 0) return;
    intent();
    set({ isPlaying: !isPlaying });
    void player.toggle();
  },

  stop: () => {
    intent();
    void player.stop();
    nativeEngine.stopBands();
    set({ isPlaying: false, currentTime: 0, engineActive: false });
    clearSignalPath();
  },

  next: async () => {
    intent();
    void player.next();
  },

  prev: async () => {
    intent();
    void player.prev();
  },

  seek: (seconds) => {
    intent();
    markSeek(seconds);
    void player.seek(seconds * 1000);
    set({ currentTime: seconds });
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
    intent();
    markSeek(seconds);
    set({ currentTime: seconds });
    void player.seek(seconds * 1000);
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

  // Choose the output DAC. If something is playing, the engine restarts it on the new device.
  setOutputDevice: (name) => {
    set({ outputDevice: name });
    void nativeEngine.setDevice(name);
    intent();
    void player.restart();
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

  cycleRepeat: () => {
    const cur = get().repeat;
    const repeat: RepeatMode = cur === "off" ? "all" : cur === "all" ? "one" : "off";
    set({ repeat });
    intent();
    void player.setModes(repeat, get().shuffle);
  },

  toggleShuffle: () => {
    const shuffle = !get().shuffle;
    set({ shuffle });
    intent();
    void player.setModes(get().repeat, shuffle);
  },

  // Volume normalisation. Off keeps the bit-perfect path; track/album apply the file's
  // ReplayGain tag (peak-limited) to the current and subsequent tracks.
  setReplayGainMode: (mode) => {
    set({ replayGainMode: mode });
    applyReplayGain();
  },

  setScrobbleEnabled: (on) => {
    set({ scrobbleEnabled: on });
    void player.setScrobble(on);
  },

  startSleepTimer: (preset) => {
    intent();
    if (preset === -1) {
      void player.sleepAfterTrack();
      set({ sleepTimer: { endOfTrack: true, remainingSec: null, totalSec: null } });
    } else {
      void player.sleepIn(preset * 60 * 1000);
      set({
        sleepTimer: { endOfTrack: false, remainingSec: preset * 60, totalSec: preset * 60 },
      });
    }
  },

  cancelSleepTimer: () => {
    intent();
    void player.cancelSleep();
    set({ sleepTimer: null });
  },
}));

export { EQ_PRESETS };

/** Pause the main status poll while the mini window is active (compact mode).
 *  Call `resumeMainPoll` when returning to the full player and a track is playing. */
export function pauseMainPoll() {
  stopNativePoll();
  nativeEngine.stopBands();
}

/** Restart the main status poll when leaving compact mode. The spectrum feed restarts only
 *  if a session is live; after that the poll starts and stops it as the engine does. */
export function resumeMainPoll() {
  startNativePoll();
  if (usePlayerStore.getState().engineActive) nativeEngine.startBands();
}
