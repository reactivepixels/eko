import { useState } from "react";
import { usePlayerStore } from "../store/usePlayerStore";
import { nativeEngine } from "../audio/nativeEngine";
import type { ReplayGainMode } from "../types";

const khz = (n: number) => (n ? `${(n / 1000).toFixed(n % 1000 ? 1 : 0)} kHz` : "—");
const XF_OPTIONS = [0, 2000, 4000, 6000, 8000, 12000];

/**
 * The single source of bit-perfect truth (docs/skin-architecture.md §5 point 1) — formerly
 * derived in BOTH `TransportBar` and `SignalPath`, slightly differently. Reads live engine
 * info (real device + rates), derives `pure` honestly (no EQ, unity volume, no ReplayGain,
 * no resample at either the engine or the OS device), and exposes the output-device /
 * ReplayGain / crossfade pickers' state + actions. A `StatusLamp`/seal binds `pure`; it is a
 * lit status, NEVER a toggle.
 */
export function useSignalPath() {
  const info = usePlayerStore((s) => s.engineInfo);
  const engineActive = usePlayerStore((s) => s.engineActive);
  const eqEnabled = usePlayerStore((s) => s.eqEnabled);
  const preamp = usePlayerStore((s) => s.preamp);
  const gains = usePlayerStore((s) => s.gains);
  const volume = usePlayerStore((s) => s.volume);
  const outputDevice = usePlayerStore((s) => s.outputDevice);
  const replayGainMode = usePlayerStore((s) => s.replayGainMode);
  const rgAppliedDb = usePlayerStore((s) => s.rgAppliedDb);
  const crossfadeMs = usePlayerStore((s) => s.crossfadeMs);

  const [devices, setDevices] = useState<string[]>([]);

  // Whether a full signal-path display has live data to show.
  const active = !!(engineActive && info && info.rate);

  const eqActive = eqEnabled && (preamp !== 0 || gains.some((g) => g !== 0));
  const resampled = !!info && info.srcRate > 0 && info.srcRate !== info.rate;
  // The OS device runs at a different rate than EKO's stream → macOS is resampling.
  const osResampled = !!info && info.devRate > 0 && info.devRate !== info.rate;
  const attenuated = volume < 1;
  const rgActive = rgAppliedDb != null;
  const pure = !eqActive && !attenuated && !rgActive && !resampled && !osResampled;

  const codec = (info?.codec || "audio").toUpperCase();
  const src = info ? `${codec} · ${khz(info.srcRate)}${info.bits ? ` · ${info.bits}-bit` : ""}` : "";
  const engineLabel = pure
    ? "Bit-perfect"
    : [
        resampled && info && `Resampled → ${khz(info.rate)}`,
        osResampled && info && `OS resample → ${khz(info.devRate)}`,
        eqActive && "EQ",
        attenuated && "Volume",
        rgActive && `ReplayGain ${rgAppliedDb!.toFixed(1)} dB`,
      ]
        .filter(Boolean)
        .join(" · ");
  const sealLabel = pure
    ? "BIT-PERFECT"
    : [
        (resampled || osResampled) && "RESAMPLED",
        eqActive && "EQ",
        attenuated && "VOLUME",
        rgActive && "REPLAYGAIN",
      ]
        .filter(Boolean)
        .join(" · ") || "PROCESSED";
  const rgLabel =
    replayGainMode === "off"
      ? "Off"
      : `${replayGainMode === "album" ? "Album" : "Track"}${rgActive ? ` · ${rgAppliedDb!.toFixed(1)} dB` : ""}`;
  const xfLabel = crossfadeMs === 0 ? "Off" : `${crossfadeMs / 1000}s`;

  return {
    active,
    info,
    // bit-perfect truth + breakdown
    pure,
    eqActive,
    resampled,
    osResampled,
    attenuated,
    rgActive,
    // display strings
    codec,
    src,
    engineLabel,
    sealLabel,
    // ReplayGain
    replayGainMode,
    rgAppliedDb,
    rgLabel,
    setReplayGainMode: (m: ReplayGainMode) => usePlayerStore.getState().setReplayGainMode(m),
    // crossfade
    crossfadeMs,
    xfLabel,
    xfOptions: XF_OPTIONS,
    setCrossfade: (ms: number) => usePlayerStore.getState().setCrossfade(ms),
    // output device picker
    outputDevice,
    setOutputDevice: (name: string | null) => usePlayerStore.getState().setOutputDevice(name),
    devices,
    loadDevices: async () => setDevices(await nativeEngine.listDevices().catch(() => [])),
  };
}
