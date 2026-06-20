import { invoke } from "@tauri-apps/api/core";

export interface EngineStatus {
  playing: boolean;
  posMs: number;
  durMs: number;
  rate: number;
  channels: number;
  device: string;
  srcRate: number;
  devRate: number;
  bits: number;
  codec: string;
  seg: number; // playing-track index within the current gapless session (0 = first track)
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
  setVolume: (vol: number) => invoke("engine_set_volume", { vol }),
  // ReplayGain (off by default): pass the chosen gain in dB, or null/0 to disable.
  // Any non-zero value takes playback off the bit-perfect path (honestly reflected in the seal).
  setReplayGain: (gainDb: number | null) => invoke("engine_set_replaygain", { gainDb }),
  // Queue the next track for gapless continuation (same-rate → no seam). null/null clears it.
  enqueue: (path: string | null, url: string | null) => invoke("engine_enqueue", { path, url }),
  // Crossfade duration (ms; 0 = off). Re-pushed per track. Off keeps the bit-perfect path.
  setCrossfade: (ms: number) => invoke("engine_set_crossfade", { ms }),
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
