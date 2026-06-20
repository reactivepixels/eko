import { useState } from "react";
import { usePlayerStore } from "../store/usePlayerStore";
import { nativeEngine } from "../audio/nativeEngine";

const khz = (n: number) => (n ? `${(n / 1000).toFixed(n % 1000 ? 1 : 0)} kHz` : "—");

/**
 * Roon-style signal path: SOURCE → ENGINE → OUTPUT, with an honest indicator that's only
 * "pure" when nothing alters the bits (no resample, no EQ, unity volume). Reads the live
 * engine info (real device + rates), not a guess from the file's tags. The OUTPUT node is
 * a picker for choosing the DAC.
 */
export function SignalPath() {
  const info = usePlayerStore((s) => s.engineInfo);
  const engineActive = usePlayerStore((s) => s.engineActive);
  const eqEnabled = usePlayerStore((s) => s.eqEnabled);
  const preamp = usePlayerStore((s) => s.preamp);
  const gains = usePlayerStore((s) => s.gains);
  const volume = usePlayerStore((s) => s.volume);
  const outputDevice = usePlayerStore((s) => s.outputDevice);
  const setOutputDevice = usePlayerStore((s) => s.setOutputDevice);
  const replayGainMode = usePlayerStore((s) => s.replayGainMode);
  const setReplayGainMode = usePlayerStore((s) => s.setReplayGainMode);
  const rgAppliedDb = usePlayerStore((s) => s.rgAppliedDb);
  const crossfadeMs = usePlayerStore((s) => s.crossfadeMs);
  const setCrossfade = usePlayerStore((s) => s.setCrossfade);

  const [open, setOpen] = useState(false);
  const [rgOpen, setRgOpen] = useState(false);
  const [xfOpen, setXfOpen] = useState(false);
  const [devices, setDevices] = useState<string[]>([]);

  if (!engineActive || !info || !info.rate) return null;

  const eqActive = eqEnabled && (preamp !== 0 || gains.some((g) => g !== 0));
  const resampled = info.srcRate > 0 && info.srcRate !== info.rate;
  // The OS device runs at a different rate than EKO's stream → macOS is resampling.
  const osResampled = info.devRate > 0 && info.devRate !== info.rate;
  const attenuated = volume < 1;
  const rgActive = rgAppliedDb != null; // a non-zero ReplayGain adjustment is being applied
  const pure = !eqActive && !resampled && !osResampled && !attenuated && !rgActive;

  const codec = (info.codec || "audio").toUpperCase();
  const src = `${codec} · ${khz(info.srcRate)}${info.bits ? ` · ${info.bits}-bit` : ""}`;
  const engine = pure
    ? "Bit-perfect"
    : [
        resampled && `Resampled → ${khz(info.rate)}`,
        osResampled && `OS resample → ${khz(info.devRate)}`,
        eqActive && "EQ",
        attenuated && "Volume",
        rgActive && `ReplayGain ${rgAppliedDb!.toFixed(1)} dB`,
      ]
        .filter(Boolean)
        .join(" · ");

  const openPicker = async () => {
    setDevices(await nativeEngine.listDevices().catch(() => []));
    setOpen(true);
  };
  const pick = (name: string | null) => {
    setOutputDevice(name);
    setOpen(false);
  };

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
  const pickRg = (mode: "off" | "track" | "album") => {
    setReplayGainMode(mode);
    setRgOpen(false);
  };

  const xfOptions = [0, 2000, 4000, 6000, 8000, 12000];
  const xfLabel = crossfadeMs === 0 ? "Off" : `${crossfadeMs / 1000}s`;
  const pickXf = (ms: number) => {
    setCrossfade(ms);
    setXfOpen(false);
  };

  return (
    <div
      className={`sigpath${pure ? " pure" : ""}`}
      title={pure ? "Untouched signal path — bit-for-bit to your DAC" : `Processing: ${engine}`}
    >
      <div className="sp-node sp-source">
        <span className="sp-k">SOURCE</span>
        <span className="sp-v">{src}</span>
      </div>
      <span className="sp-link" />
      <div className="sp-node sp-out" onClick={openPicker} title="Choose output device">
        <span className="sp-k">
          OUTPUT <span className="sp-caret">▾</span>
        </span>
        <span className="sp-v">
          {info.device || "Output"} · {khz(info.rate)}
        </span>
        {open && (
          <>
            <div
              className="backdrop"
              onClick={(e) => {
                e.stopPropagation();
                setOpen(false);
              }}
            />
            <div className="menu sp-menu" onClick={(e) => e.stopPropagation()}>
              <div className={`mi${outputDevice == null ? " on" : ""}`} onClick={() => pick(null)}>
                System Default
              </div>
              {devices.map((d) => (
                <div
                  key={d}
                  className={`mi${outputDevice === d ? " on" : ""}`}
                  onClick={() => pick(d)}
                >
                  {d}
                </div>
              ))}
            </div>
          </>
        )}
      </div>
      <div
        className="sp-node sp-rg"
        onClick={() => setRgOpen((v) => !v)}
        title="ReplayGain — volume normalisation (off keeps the bit-perfect path)"
      >
        <span className="sp-k">
          RG <span className="sp-caret">▾</span>
        </span>
        <span className="sp-v">{rgLabel}</span>
        {rgOpen && (
          <>
            <div
              className="backdrop"
              onClick={(e) => {
                e.stopPropagation();
                setRgOpen(false);
              }}
            />
            <div className="menu sp-menu" onClick={(e) => e.stopPropagation()}>
              <div
                className={`mi${replayGainMode === "off" ? " on" : ""}`}
                onClick={() => pickRg("off")}
              >
                Off
              </div>
              <div
                className={`mi${replayGainMode === "track" ? " on" : ""}`}
                onClick={() => pickRg("track")}
              >
                Track
              </div>
              <div
                className={`mi${replayGainMode === "album" ? " on" : ""}`}
                onClick={() => pickRg("album")}
              >
                Album
              </div>
            </div>
          </>
        )}
      </div>
      <div
        className="sp-node sp-rg"
        onClick={() => setXfOpen((v) => !v)}
        title="Crossfade between tracks — off keeps the bit-perfect path; only the overlap is mixed"
      >
        <span className="sp-k">
          XFADE <span className="sp-caret">▾</span>
        </span>
        <span className="sp-v">{xfLabel}</span>
        {xfOpen && (
          <>
            <div
              className="backdrop"
              onClick={(e) => {
                e.stopPropagation();
                setXfOpen(false);
              }}
            />
            <div className="menu sp-menu" onClick={(e) => e.stopPropagation()}>
              {xfOptions.map((ms) => (
                <div
                  key={ms}
                  className={`mi${crossfadeMs === ms ? " on" : ""}`}
                  onClick={() => pickXf(ms)}
                >
                  {ms === 0 ? "Off" : `${ms / 1000}s`}
                </div>
              ))}
            </div>
          </>
        )}
      </div>
      <div className="sp-seal">
        <span className="sp-ring">
          {pure ? (
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M5 12.5l4.5 4.5L19 7" />
            </svg>
          ) : (
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="12" cy="12" r="9" />
              <path d="M12 8v5M12 16.3v.4" />
            </svg>
          )}
        </span>
        <span className="sp-seal-lab">{sealLabel}</span>
      </div>
    </div>
  );
}
