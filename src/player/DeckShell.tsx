import { useUiStore } from "../store/useUiStore";
import { Slot } from "./Slot";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";
import { SignalPath } from "./SignalPath";
import { ParametricEqPanel } from "@pro";
import { useNowPlaying } from "../hooks/useNowPlaying";
import { useEq } from "../hooks/useEq";
import { useTransport } from "../hooks/useTransport";
import { useSignalPath } from "../hooks/useSignalPath";
import { usePlayerStore } from "../store/usePlayerStore";
import type { Track } from "../types";

/**
 * Generic Now-Playing deck — the same layout/chrome as DeckView, but the spectrum and EQ are
 * SLOTS resolved to the active preset's variant (Porcelain faders / Studio knobs / …). The
 * display, signal path and EQ-side (preset + preamp) are shared chrome, token-skinned by palette.
 */
function fmt(t: Track | null) {
  if (!t) return { container: "—", rate: "—" };
  const m = t.mime ?? "";
  const container = m.includes("flac")
    ? "FLAC"
    : m.includes("mpeg")
      ? "MP3"
      : m.includes("mp4") || m.includes("aac")
        ? "AAC/ALAC"
        : m.includes("wav")
          ? "WAV"
          : m.includes("ogg") || m.includes("opus")
            ? "OGG"
            : "AUDIO";
  const rate = t.sampleRate
    ? `${(t.sampleRate / 1000).toFixed(t.sampleRate % 1000 ? 1 : 0)}k`
    : "—";
  return { container, rate };
}

export function DeckShell() {
  const presetsOpen = useUiStore((s) => s.presetsOpen);
  const setPresetsOpen = useUiStore((s) => s.setPresetsOpen);
  const np = useNowPlaying();
  const eq = useEq();
  const eqMode = usePlayerStore((s) => s.eqMode);
  const { isPlaying } = useTransport();
  const { info: engineInfo } = useSignalPath();
  const cur = np.track;
  const f = fmt(cur);

  return (
    <div className="view deck">
      <div className="deck-disp">
        <div className="art">
          {np.coverUrl(200) ? (
            <img src={np.coverUrl(200) ?? ""} alt="" />
          ) : np.coverPath ? (
            <LocalCover path={np.coverPath} />
          ) : null}
        </div>
        <div className="well">
          <div className="screen">
            <div className="vfd" style={{ flex: 1, minWidth: 0 }}>
              <div className="lab">
                NOW PLAYING
                <span className={`eqglyph${isPlaying ? "" : " paused"}`}>
                  <i />
                  <i />
                  <i />
                  <i />
                </span>
              </div>
              <div style={{ fontSize: 19, fontWeight: 600, color: "var(--ink)", marginTop: 6 }}>
                <Marquee text={cur?.title ?? "—"} />
              </div>
              <div
                style={{
                  fontSize: 12,
                  color: "var(--ink-3)",
                  marginTop: 5,
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                }}
              >
                {cur ? `${cur.artist ?? ""}${cur.album ? " · " + cur.album : ""}` : "Nothing playing"}
              </div>
            </div>
            <div className="vfd" style={{ textAlign: "right", flex: "0 0 auto", paddingLeft: 16 }}>
              <div className="lab">FORMAT</div>
              <div style={{ fontSize: 16, fontWeight: 600, color: "var(--ink)", marginTop: 6 }}>
                {f.container} {f.rate}
              </div>
              <div style={{ fontSize: 10, color: "var(--ink-3)", marginTop: 5, letterSpacing: 1 }}>
                {engineInfo?.bits
                  ? `${engineInfo.bits}-BIT`
                  : engineInfo
                    ? engineInfo.channels === 1
                      ? "MONO"
                      : "STEREO"
                    : " "}
              </div>
            </div>
          </div>
        </div>
      </div>

      <SignalPath />

      {/* Spectrum slot — each theme's variant owns its well/screen */}
      <Slot slot="spectrum" />

      {/* EQ — graphic/parametric mode bar (Pro) sits above the graphic faders. ParametricEqPanel
          renders null in the free build, and eqMode stays "graphic", so the free deck is unchanged;
          in Pro every theme gets the same graphic↔parametric EQ as Studio (feature parity). */}
      <ParametricEqPanel />
      {eqMode === "graphic" && (
      <div className="deck-eq">
        {/* EQ slot — Porcelain faders / Studio knobs */}
        <Slot slot="eq" />
        <div className="eq-side" style={{ position: "relative" }}>
          <div
            className="pillbtn"
            onClick={() => setPresetsOpen(!presetsOpen)}
            role="button"
            tabIndex={0}
            aria-label={`EQ preset: ${eq.presetName ?? "Custom"}`}
            aria-expanded={presetsOpen}
            aria-haspopup="listbox"
            onKeyDown={(e) =>
              e.key === "Enter" || e.key === " " ? setPresetsOpen(!presetsOpen) : undefined
            }
          >
            {(eq.presetName ?? "CUSTOM").toUpperCase()} ▾
          </div>
          <div className="tag">
            PREAMP {eq.preamp > 0 ? "+" : ""}
            {eq.preamp.toFixed(0)} dB
          </div>
          {presetsOpen && (
            <>
              <div className="backdrop" onClick={() => setPresetsOpen(false)} />
              <div className="menu" style={{ right: 0, bottom: 44 }}>
                {eq.presets.map((p) => (
                  <div
                    key={p.name}
                    className={`mi${p.name === eq.presetName ? " on" : ""}`}
                    onClick={() => {
                      eq.applyPreset(p);
                      setPresetsOpen(false);
                    }}
                  >
                    {p.name}
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
      )}
    </div>
  );
}
