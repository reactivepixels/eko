import { useUiStore } from "../store/useUiStore";
import { usePlayerStore } from "../store/usePlayerStore";
import { coverArtUrl } from "../subsonic/client";
import { EQ_PRESETS, EQ_GAIN_MIN, EQ_GAIN_MAX } from "../audio/constants";
import { Spectrum } from "./Spectrum";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";
import { SignalPath } from "./SignalPath";
import type { Track } from "../types";

const BANDS = ["60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k"];

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

export function DeckView() {
  const presetsOpen = useUiStore((s) => s.presetsOpen);
  const setPresetsOpen = useUiStore((s) => s.setPresetsOpen);
  const { gains, preamp, presetName, currentIndex, tracks, isPlaying, engineInfo } =
    usePlayerStore();
  const setBandGain = usePlayerStore((s) => s.setBandGain);
  const applyPreset = usePlayerStore((s) => s.applyPreset);
  const cur = currentIndex !== null ? (tracks[currentIndex] ?? null) : null;
  const f = fmt(cur);

  const setFromY = (i: number, clientY: number, rail: HTMLElement) => {
    const r = rail.getBoundingClientRect();
    const t = 1 - Math.min(1, Math.max(0, (clientY - r.top) / r.height));
    setBandGain(i, EQ_GAIN_MIN + t * (EQ_GAIN_MAX - EQ_GAIN_MIN));
  };

  return (
    <div className="view deck">
      <div className="deck-disp">
        <div className="art">
          {cur?.coverArt ? (
            <img src={coverArtUrl(cur.coverArt, 200) ?? ""} alt="" />
          ) : cur?.path && !cur.subsonicId ? (
            <LocalCover path={cur.path} />
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
                {cur
                  ? `${cur.artist ?? ""}${cur.album ? " · " + cur.album : ""}`
                  : "Nothing playing"}
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
                    : " "}
              </div>
            </div>
          </div>
        </div>
      </div>

      <SignalPath />

      <div className="deck-spec">
        <div className="well">
          <div className="screen">
            <Spectrum bands={36} segs={30} />
          </div>
        </div>
      </div>

      <div className="deck-eq">
        <div className="eqfaders">
          {BANDS.map((b, i) => {
            const t = (gains[i] + 12) / 24;
            return (
              <div className="fader" key={b}>
                <div
                  className="rail"
                  onPointerDown={(e) => {
                    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
                    setFromY(i, e.clientY, e.currentTarget);
                  }}
                  onPointerMove={(e) => {
                    if (e.buttons & 1) setFromY(i, e.clientY, e.currentTarget);
                  }}
                >
                  <div className="cap" style={{ bottom: `calc(${t * 100}% - 6.5px)` }} />
                </div>
                <div className="fl">{b}</div>
              </div>
            );
          })}
        </div>
        <div className="eq-side" style={{ position: "relative" }}>
          <div className="pillbtn" onClick={() => setPresetsOpen(!presetsOpen)}>
            {(presetName ?? "CUSTOM").toUpperCase()} ▾
          </div>
          <div className="tag">
            PREAMP {preamp > 0 ? "+" : ""}
            {preamp.toFixed(0)} dB
          </div>
          {presetsOpen && (
            <>
              <div className="backdrop" onClick={() => setPresetsOpen(false)} />
              <div className="menu" style={{ right: 0, bottom: 44 }}>
                {EQ_PRESETS.map((p) => (
                  <div
                    key={p.name}
                    className={`mi${p.name === presetName ? " on" : ""}`}
                    onClick={() => {
                      applyPreset(p);
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
    </div>
  );
}
