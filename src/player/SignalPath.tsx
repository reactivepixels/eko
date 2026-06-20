import { useState } from "react";
import { useSignalPath } from "../hooks/useSignalPath";

const khz = (n: number) => (n ? `${(n / 1000).toFixed(n % 1000 ? 1 : 0)} kHz` : "—");

/**
 * Roon-style signal path: SOURCE → ENGINE → OUTPUT, with an honest seal that's only "pure"
 * when nothing alters the bits. Pure presentation over `useSignalPath()` (the single
 * bit-perfect truth + device/ReplayGain/crossfade actions); this file owns only the
 * dropdown open-state.
 */
export function SignalPath() {
  const sp = useSignalPath();
  const [open, setOpen] = useState(false);
  const [rgOpen, setRgOpen] = useState(false);
  const [xfOpen, setXfOpen] = useState(false);

  if (!sp.active || !sp.info) return null;
  const info = sp.info;

  const openPicker = async () => {
    await sp.loadDevices();
    setOpen(true);
  };
  const pick = (name: string | null) => {
    sp.setOutputDevice(name);
    setOpen(false);
  };
  const pickRg = (mode: "off" | "track" | "album") => {
    sp.setReplayGainMode(mode);
    setRgOpen(false);
  };
  const pickXf = (ms: number) => {
    sp.setCrossfade(ms);
    setXfOpen(false);
  };

  return (
    <div
      className={`sigpath${sp.pure ? " pure" : ""}`}
      title={
        sp.pure ? "Untouched signal path — bit-for-bit to your DAC" : `Processing: ${sp.engineLabel}`
      }
    >
      <div className="sp-node sp-source">
        <span className="sp-k">SOURCE</span>
        <span className="sp-v">{sp.src}</span>
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
              <div
                className={`mi${sp.outputDevice == null ? " on" : ""}`}
                onClick={() => pick(null)}
              >
                System Default
              </div>
              {sp.devices.map((d) => (
                <div
                  key={d}
                  className={`mi${sp.outputDevice === d ? " on" : ""}`}
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
        <span className="sp-v">{sp.rgLabel}</span>
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
                className={`mi${sp.replayGainMode === "off" ? " on" : ""}`}
                onClick={() => pickRg("off")}
              >
                Off
              </div>
              <div
                className={`mi${sp.replayGainMode === "track" ? " on" : ""}`}
                onClick={() => pickRg("track")}
              >
                Track
              </div>
              <div
                className={`mi${sp.replayGainMode === "album" ? " on" : ""}`}
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
        <span className="sp-v">{sp.xfLabel}</span>
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
              {sp.xfOptions.map((ms) => (
                <div
                  key={ms}
                  className={`mi${sp.crossfadeMs === ms ? " on" : ""}`}
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
          {sp.pure ? (
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
        <span className="sp-seal-lab">{sp.sealLabel}</span>
      </div>
    </div>
  );
}
