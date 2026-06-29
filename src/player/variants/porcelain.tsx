/**
 * Porcelain component variants — the free, default per-slot presentations, extracted verbatim
 * from DeckView/TransportBar so a Porcelain-preset render through the generic Shell is identical
 * to today. Each variant consumes its slot's headless hook and renders the existing neu.css
 * control classes (token-skinned by whatever palette is active). Components only — the
 * VariantDefinitions are assembled in src/skin/registerVariants.ts (keeps Fast-Refresh clean).
 */
import { formatTime } from "../../lib/format";
import { Spectrum } from "../Spectrum";
import { useEq } from "../../hooks/useEq";
import { useVolume } from "../../hooks/useVolume";
import { useScrub } from "../../hooks/useScrub";
import { useTransport } from "../../hooks/useTransport";

export function PorcelainVolume() {
  const vol = useVolume();
  return (
    <div
      className="knob-dial"
      title="Volume (drag or scroll)"
      role="slider"
      tabIndex={0}
      aria-label="Volume"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(((vol.deg + 135) / 270) * 100)}
      onPointerDown={vol.onPointerDown}
      onWheel={vol.onWheel}
    >
      <div className="ind" aria-hidden="true" style={{ transform: `rotate(${vol.deg}deg)` }} />
    </div>
  );
}

export function PorcelainSeek() {
  const scrub = useScrub();
  const prog = scrub.progress;
  // Show the buffered fill only when it meaningfully trails the played position — i.e. a
  // server stream still downloading (local / fully-decoded tracks are buffered to 100%, so
  // the bar would just match the track and add no information).
  const showBuffered = scrub.bufferedProgress < 0.999 && scrub.bufferedProgress > prog;
  return (
    <div className="scrub" role="group" aria-label="Seek">
      <span className="t">{formatTime(scrub.currentTime)}</span>
      <div
        className="track"
        role="slider"
        tabIndex={0}
        aria-label="Seek"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(prog * 100)}
        onPointerDown={scrub.onPointerDown}
        onPointerMove={scrub.onPointerMove}
        onPointerUp={scrub.onPointerUp}
        onPointerCancel={scrub.onPointerCancel}
      >
        {showBuffered && (
          <div
            className="buffered"
            aria-hidden="true"
            style={{ width: `${scrub.bufferedProgress * 100}%` }}
          />
        )}
        <div className="fill" style={{ width: `${prog * 100}%` }} />
        <div className="knob" aria-hidden="true" style={{ left: `${prog * 100}%` }} />
      </div>
      <span className="t r">-{formatTime(scrub.remaining)}</span>
    </div>
  );
}

export function PorcelainTransport() {
  const tr = useTransport();
  return (
    <div
      className={`controls${!tr.hasQueue ? " dim" : ""}`}
      role="group"
      aria-label="Playback controls"
    >
      <div
        className={`tbtn sm${tr.shuffle ? " on" : ""}`}
        title="Shuffle"
        onClick={tr.toggleShuffle}
        role="button"
        tabIndex={0}
        aria-label="Shuffle"
        aria-pressed={tr.shuffle}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M3 7h4l9 10h5M16 7h5v5M3 17h4l3-3.4" />
        </svg>
      </div>
      <div
        className="tbtn"
        title="Previous"
        onClick={tr.prev}
        role="button"
        tabIndex={0}
        aria-label="Previous track"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M7 6h2v12H7zM19 6v12l-9-6z" />
        </svg>
      </div>
      <div
        className="tbtn play"
        title="Play / Pause"
        onClick={tr.togglePlay}
        role="button"
        tabIndex={0}
        aria-label={tr.isPlaying ? "Pause" : "Play"}
      >
        {tr.isPlaying ? (
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z" />
          </svg>
        ) : (
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M8 5v14l11-7z" />
          </svg>
        )}
      </div>
      <div
        className="tbtn"
        title="Next"
        onClick={tr.next}
        role="button"
        tabIndex={0}
        aria-label="Next track"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M15 6h2v12h-2zM5 6l9 6-9 6z" />
        </svg>
      </div>
      <div
        className={`tbtn sm${tr.repeat !== "off" ? " on" : ""}`}
        title="Repeat"
        onClick={tr.cycleRepeat}
        role="button"
        tabIndex={0}
        aria-label={`Repeat: ${tr.repeat}`}
        aria-pressed={tr.repeat !== "off"}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M4 9a4 4 0 0 1 4-4h9M17 5l-2-2M17 5l-2 2M20 15a4 4 0 0 1-4 4H7M7 19l2-2M7 19l2 2" />
        </svg>
      </div>
    </div>
  );
}

export function PorcelainSpectrum() {
  return (
    <div className="deck-spec">
      <div className="well">
        <div className="screen">
          <Spectrum bands={36} segs={30} />
        </div>
      </div>
    </div>
  );
}

export function PorcelainEq() {
  const eq = useEq();
  return (
    <div className="eqfaders">
      {eq.bands.map((b, i) => {
        const t = eq.norm(i);
        return (
          <div className="fader" key={b}>
            <div
              className="rail"
              {...eq.railHandlers(i)}
              role="slider"
              tabIndex={0}
              aria-label={`EQ ${b}`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(t * 100)}
            >
              <div
                className="cap"
                aria-hidden="true"
                style={{ bottom: `calc(${t * 100}% - 6.5px)` }}
              />
            </div>
            <div className="fl" aria-hidden="true">
              {b}
            </div>
          </div>
        );
      })}
    </div>
  );
}
