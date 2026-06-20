import { useUiStore } from "../store/useUiStore";
import { Spectrum } from "./Spectrum";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";
import { SignalPath } from "./SignalPath";
import { useNowPlaying } from "../hooks/useNowPlaying";
import { useEq } from "../hooks/useEq";
import { useTransport } from "../hooks/useTransport";
import { useSignalPath } from "../hooks/useSignalPath";
import type { Track } from "../types";

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
  const skin = useUiStore((s) => s.skin);
  const np = useNowPlaying();
  const eq = useEq();
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
                    : " "}
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
        {skin === "studio" ? (
          <div className="eqknobs">
            {eq.bands.map((b, i) => {
              const t = eq.norm(i);
              return (
                <div className="ctl" key={b}>
                  <div className="eqknob" {...eq.knobHandlers(i)}>
                    <svg className="arc" viewBox="0 0 100 100">
                      <circle className="trk" cx="50" cy="50" r="45" pathLength={100} />
                      <circle
                        className="val"
                        cx="50"
                        cy="50"
                        r="45"
                        pathLength={100}
                        style={{ strokeDasharray: `${(t * 75).toFixed(1)} 100` }}
                      />
                    </svg>
                    <div className="body" />
                    <div className="top" />
                    <div
                      className="dial"
                      style={{ transform: `rotate(${(t * 270 - 135).toFixed(1)}deg)` }}
                    >
                      <i className="ind" />
                    </div>
                  </div>
                  <div className="fl">{b}</div>
                </div>
              );
            })}
          </div>
        ) : (
          <div className="eqfaders">
            {eq.bands.map((b, i) => {
              const t = eq.norm(i);
              return (
                <div className="fader" key={b}>
                  <div className="rail" {...eq.railHandlers(i)}>
                    <div className="cap" style={{ bottom: `calc(${t * 100}% - 6.5px)` }} />
                  </div>
                  <div className="fl">{b}</div>
                </div>
              );
            })}
          </div>
        )}
        <div className="eq-side" style={{ position: "relative" }}>
          <div className="pillbtn" onClick={() => setPresetsOpen(!presetsOpen)}>
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
    </div>
  );
}
