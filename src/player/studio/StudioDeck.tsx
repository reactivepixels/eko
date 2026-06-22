import { useState } from "react";
import { useNowPlaying } from "../../hooks/useNowPlaying";
import { useEq } from "../../hooks/useEq";
import { useSignalPath } from "../../hooks/useSignalPath";
import { usePlayerStore } from "../../store/usePlayerStore";
import { Spectrum } from "../Spectrum";
import { LocalCover } from "../LocalCover";
import { Marquee } from "../Marquee";
import { ParametricEqPanel } from "@pro";
import type { Track } from "../../types";
import styles from "./StudioDeck.module.css";

function fmt(t: Track | null) {
  if (!t) return { container: "—", rate: "—", bits: "" };
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
  const bits = t.channels != null ? (t.channels === 1 ? "Mono" : "") : "";
  return { container, rate, bits };
}

const XF_DEFAULT_MS = 6000;

export function StudioDeck() {
  const np = useNowPlaying();
  const eq = useEq();
  const sp = useSignalPath();
  const eqMode = usePlayerStore((s) => s.eqMode);

  const cur = np.track;
  const f = fmt(cur);
  const [presetOpen, setPresetOpen] = useState(false);

  // Crossfade: on = XF_DEFAULT_MS, off = 0. If already on (nonzero), keep the value.
  const xfOn = sp.crossfadeMs > 0;
  const toggleXf = () => sp.setCrossfade(xfOn ? 0 : XF_DEFAULT_MS);

  return (
    <div className={styles.deck}>
      {/* ── ART + META ROW ── */}
      <div className={styles.topRow}>
        <div className={styles.artWrap}>
          {np.coverUrl(220) ? (
            <img src={np.coverUrl(220) ?? ""} alt="" className={styles.artImg} />
          ) : np.coverPath ? (
            <LocalCover path={np.coverPath} className={styles.artImg} />
          ) : (
            <div className={styles.artEmpty} />
          )}
        </div>
        <div className={styles.metaWrap}>
          <div className={styles.metaTitle}>
            <Marquee text={cur?.title ?? "—"} />
          </div>
          <div className={styles.metaArtist}>
            {cur ? `${cur.artist ?? ""}${cur.album ? " · " + cur.album : ""}` : "Nothing playing"}
          </div>
          <div className={styles.fbadge}>
            <span className={styles.fbadgeDot} />
            {f.container}
            {f.rate !== "—" ? ` · ${f.rate}` : ""}
            {f.bits ? ` · ${f.bits}` : ""}
          </div>
        </div>
      </div>

      <div className={styles.rule} />

      {/* ── SPECTRUM SCREEN ── */}
      <div className={styles.specWell}>
        <Spectrum bands={36} segs={30} />
      </div>

      <div className={styles.rule} />

      {/* ── EQ SECTION ── */}
      <div className={styles.eqSection}>
        <div className={styles.eqHead}>
          <span className={styles.eqTitle}>Equaliser</span>
          {/* Preset picker is only shown in graphic mode */}
          {eqMode === "graphic" && (
            <div className={styles.presetWrap}>
              <button
                className={styles.presetBtn}
                onClick={() => setPresetOpen((o) => !o)}
                aria-haspopup="listbox"
                aria-expanded={presetOpen}
              >
                {eq.presetName ?? "Custom"}
                <span className={styles.presetCaret}>▾</span>
              </button>
              {presetOpen && (
                <>
                  <div className={styles.presetBackdrop} onClick={() => setPresetOpen(false)} />
                  <div className={styles.presetMenu} role="listbox">
                    {eq.presets.map((p) => (
                      <button
                        key={p.name}
                        role="option"
                        aria-selected={p.name === eq.presetName}
                        className={`${styles.presetMi}${p.name === eq.presetName ? ` ${styles.presetMiOn}` : ""}`}
                        onClick={() => {
                          eq.applyPreset(p);
                          setPresetOpen(false);
                        }}
                      >
                        {p.name}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          )}
        </div>

        {/* Mode bar + parametric panel (always rendered; mode bar handles graphic ↔ parametric switch) */}
        <ParametricEqPanel />

        {/* 10-band graphic EQ knobs — only shown in graphic mode */}
        {eqMode === "graphic" && (
          <div className={styles.eqKnobs}>
            {eq.bands.map((b, i) => {
              const t = eq.norm(i);
              return (
                <div className={styles.ctl} key={b}>
                  <div className={styles.eqKnob} {...eq.knobHandlers(i)}>
                    <svg className={styles.arc} viewBox="0 0 100 100">
                      <circle className={styles.trk} cx="50" cy="50" r="45" pathLength={100} />
                      <circle
                        className={styles.val}
                        cx="50"
                        cy="50"
                        r="45"
                        pathLength={100}
                        style={{ strokeDasharray: `${(t * 75).toFixed(1)} 100` }}
                      />
                    </svg>
                    <div className={styles.knobBody} />
                    <div className={styles.knobCap} />
                    <div
                      className={styles.knobDial}
                      style={{ transform: `rotate(${(t * 270 - 135).toFixed(1)}deg)` }}
                    >
                      <i className={styles.knobInd} />
                    </div>
                  </div>
                  <div className={styles.kl}>{b}</div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <div className={styles.rule} />

      {/* ── CONSOLE FOOTER ── */}
      <div className={styles.deckFoot}>
        {/* signal controls: crossfade + replaygain */}
        <div className={styles.sigCtl}>
          {/* Crossfade slide-switch */}
          <div className={styles.sg}>
            <span className={styles.sgLab}>Crossfade</span>
            <div
              className={`${styles.slide}${xfOn ? ` ${styles.slideOn}` : ""}`}
              onClick={toggleXf}
              title={xfOn ? `Crossfade ${sp.xfLabel}` : "Crossfade off"}
            >
              <div className={styles.slideFill} />
              <div className={styles.slideNub} />
            </div>
          </div>

          {/* ReplayGain 3-way segmented */}
          <div className={styles.sg}>
            <span className={styles.sgLab}>ReplayGain</span>
            <div className={styles.seg}>
              {(["off", "track", "album"] as const).map((m) => (
                <button
                  key={m}
                  className={`${styles.segBtn}${sp.replayGainMode === m ? ` ${styles.segBtnOn}` : ""}`}
                  onClick={() => sp.setReplayGainMode(m)}
                >
                  {m.charAt(0).toUpperCase() + m.slice(1)}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* bit-perfect status */}
        <div className={styles.sigStatus}>
          <span>
            <span className={styles.sigLabel}>Source</span>
            <span className={styles.sigSrc}>{sp.src || "—"}</span>
          </span>
          {sp.pure && (
            <>
              <span className={styles.sigArrow}>→</span>
              <span className={styles.sigLock}>
                <span className={`${styles.pureDot} ${styles.pureDotOn}`} />
                Bit-Perfect · Locked
              </span>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
