import { useState } from "react";
import { useSignalPath } from "../hooks/useSignalPath";

/**
 * Roon-style signal path: SOURCE → ENGINE → OUTPUT, with an honest seal that's only "pure"
 * when nothing alters the bits. Pure presentation over `useSignalPath()` (which reads the
 * seal Rust derived — see `eko_core::signal_path`); this file owns only the dropdown
 * open-state and formats NOTHING about the signal path itself.
 */
export function SignalPath() {
  const sp = useSignalPath();
  const [open, setOpen] = useState(false);
  const [rgOpen, setRgOpen] = useState(false);

  if (!sp.active) return null;

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

  return (
    <div
      className={`sigpath${sp.pure ? " pure" : ""}`}
      title={
        sp.pure
          ? "Untouched signal path — bit-for-bit to your DAC"
          : // A non-pure seal carrying no long-form label is one whose claim was
            // withdrawn while Rust re-derives it (`unconfirmSeal` in the store). There is
            // no processing to name yet, and "Processing: " with nothing after it would
            // read as a broken string. Say what is actually true instead.
            sp.engineLabel
            ? `Processing: ${sp.engineLabel}`
            : "Checking the signal path — nothing is claimed until it is verified"
      }
    >
      <div className="sp-node sp-source">
        <span className="sp-k">SOURCE</span>
        <span className="sp-v">{sp.src}</span>
      </div>
      <span className="sp-link" />
      <div
        className="sp-node sp-out"
        onClick={openPicker}
        title="Choose output device"
        role="button"
        tabIndex={0}
        aria-label="Choose output device"
        aria-expanded={open}
        aria-haspopup="listbox"
        onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? void openPicker() : undefined)}
      >
        <span className="sp-k">
          OUTPUT{" "}
          <span className="sp-caret" aria-hidden="true">
            ▾
          </span>
        </span>
        <span className="sp-v">{sp.output}</span>
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
        role="button"
        tabIndex={0}
        aria-label={`ReplayGain: ${sp.rgLabel}`}
        aria-expanded={rgOpen}
        aria-haspopup="listbox"
        onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? setRgOpen((v) => !v) : undefined)}
      >
        <span className="sp-k">
          RG{" "}
          <span className="sp-caret" aria-hidden="true">
            ▾
          </span>
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
