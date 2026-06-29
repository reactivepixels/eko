import { invoke } from "@tauri-apps/api/core";

export interface EngineStatus {
  playing: boolean;
  posMs: number;
  durMs: number;
  bufferedMs: number; // decode-buffered extent within the current track (== durMs once fully decoded)
  rate: number;
  channels: number;
  device: string;
  srcRate: number;
  devRate: number;
  bits: number;
  codec: string;
  seg: number; // playing-track index within the current gapless session (0 = first track)
}

/** Filter type for one parametric EQ band. Mirrors the Rust `ParamBandType` enum. */
export type ParamBandType = "peaking" | "lowShelf" | "highShelf" | "lowPass" | "highPass" | "notch";

/** One parametric EQ band. Mirrors the Rust `ParamBand` struct. */
export interface ParamBand {
  filterType: ParamBandType;
  /** Centre / corner frequency in Hz. */
  freq: number;
  /** Gain in dB (peaking / shelf only; ignored for LP/HP/notch). */
  gainDb: number;
  /** Quality factor. 0.707 = Butterworth; 1.0 = broad bell. */
  q: number;
  /** Whether this band is active. */
  enabled: boolean;
}

/** Which EQ is routed to DSP. Mirrors the Rust `EqMode` enum. */
export type EqMode = "graphic" | "parametric";

/** Parsed AutoEQ result returned by `engine_parse_autoeq`. */
export interface AutoEqResult {
  preamp: number;
  bands: ParamBand[];
}

export interface NowPlaying {
  title: string;
  artist: string;
  coverUrl: string;
  coverPath: string;
  theme: string;
  index: number;
  total: number;
}

let bandData: number[] = [];
let bandTimer: ReturnType<typeof setInterval> | null = null;

/** Native bit-perfect playback (local files) — symphonia → cpal in Rust. */
export const nativeEngine = {
  play: (path: string) => invoke<EngineStatus>("engine_play", { path }),
  playUrl: (url: string) => invoke<EngineStatus>("engine_play_url", { url }),
  pause: () => invoke("engine_pause"),
  resume: () => invoke("engine_resume"),
  seek: (secs: number) => invoke("engine_seek", { secs }),
  setEq: (enabled: boolean, preamp: number, gains: number[]) =>
    invoke("engine_set_eq", { enabled, preamp, gains }),
  /** Switch the active EQ mode (graphic = free 10-band; parametric = Pro N-band). */
  setEqMode: (mode: EqMode) => invoke("engine_set_eq_mode", { mode }),
  /** Push the parametric EQ configuration to the engine. */
  setParamEq: (enabled: boolean, preamp: number, bands: ParamBand[]) =>
    invoke("engine_set_param_eq", { enabled, preamp, bands }),
  /**
   * Compute the parametric EQ frequency-response curve for the on-screen preview.
   * Returns dB values over a log-spaced grid (20 Hz–20 kHz), computed from the SAME
   * biquad coefficients the audio path uses — so the curve can't drift from the sound.
   */
  eqCurve: (bands: ParamBand[], preamp: number) =>
    invoke<number[]>("engine_eq_curve", { bands, preamp }),
  /** Parse an AutoEQ ParametricEQ.txt text. Throws on parse failure. */
  parseAutoEq: (text: string) => invoke<AutoEqResult>("engine_parse_autoeq", { text }),
  /** Read + parse an AutoEQ ParametricEQ.txt file at an absolute path. Throws on I/O or parse failure. */
  importAutoEqFile: (path: string) => invoke<AutoEqResult>("engine_import_autoeq_file", { path }),
  setVolume: (vol: number) => invoke("engine_set_volume", { vol }),
  // ReplayGain (off by default): pass the chosen gain in dB, or null/0 to disable.
  // Any non-zero value takes playback off the bit-perfect path (honestly reflected in the seal).
  setReplayGain: (gainDb: number | null) => invoke("engine_set_replaygain", { gainDb }),
  // Queue the next track for gapless continuation (same-rate → no seam). null/null/null clears it.
  enqueue: (
    path: string | null,
    url: string | null,
    trackId?: string | null,
    plainLen?: number | null,
  ) =>
    invoke("engine_enqueue", { path, url, trackId: trackId ?? null, plainLen: plainLen ?? null }),
  // Play a cached offline track (bit-perfect: EncryptedFileSource → symphonia, same path as local).
  playCached: (trackId: string, plainLen: number) =>
    invoke<EngineStatus>("engine_play_cached", { trackId, plainLen }),
  setNowPlaying: (np: NowPlaying) => invoke("engine_set_now_playing", { np }),
  nowPlaying: () => invoke<NowPlaying>("engine_now_playing"),
  listDevices: () => invoke<string[]>("engine_list_devices"),
  setDevice: (name: string | null) => invoke("engine_set_device", { name }),
  stop: () => invoke("engine_stop"),
  status: () => invoke<EngineStatus | null>("engine_status"),
  // Live spectrum bands (0..1) computed in Rust; polled while native playback is active.
  getBands: () => bandData,
  startBands: () => {
    if (bandTimer) return;
    bandTimer = setInterval(async () => {
      bandData = await invoke<number[]>("engine_bands").catch(() => bandData);
    }, 40);
  },
  stopBands: () => {
    if (bandTimer) {
      clearInterval(bandTimer);
      bandTimer = null;
    }
    bandData = [];
  },
};
