import { usePlayerStore } from "../store/usePlayerStore";
import { useUiStore } from "../store/useUiStore";
import { coverArtUrl } from "../subsonic/client";
import { formatTime } from "../lib/format";
import { Spectrum } from "./Spectrum";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";

export function TransportBar() {
  const {
    isPlaying,
    currentTime,
    duration,
    currentIndex,
    tracks,
    volume,
    shuffle,
    repeat,
    eqEnabled,
    preamp,
    gains,
  } = usePlayerStore();
  const setPlayerView = useUiStore((s) => s.setPlayerView);
  const cur = currentIndex !== null ? (tracks[currentIndex] ?? null) : null;
  const prog = duration > 0 ? currentTime / duration : 0;
  const volDeg = -135 + volume * 270;
  // Bit-perfect only when nothing touches the samples: unity volume + flat/off EQ.
  const eqActive = eqEnabled && (preamp !== 0 || gains.some((g) => g !== 0));
  const bitPerfect = volume >= 1 && !eqActive;

  const scrubSecs = (clientX: number, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    const t = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    return t * usePlayerStore.getState().duration;
  };
  const volDrag = (e: React.PointerEvent) => {
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    let v = usePlayerStore.getState().volume;
    // Pointer lock: hides the cursor and feeds relative motion (movementY) that keeps
    // flowing even at the screen edge — so this bottom-docked knob never runs out of
    // travel (the cause of the position-dependent stutter). Falls back gracefully.
    try {
      el.requestPointerLock();
    } catch {
      /* unsupported → plain window drag below */
    }
    const mv = (ev: PointerEvent) => {
      v = Math.min(1, Math.max(0, v - ev.movementY / 130));
      usePlayerStore.getState().setVolume(v);
    };
    const up = () => {
      try {
        document.exitPointerLock();
      } catch {
        /* noop */
      }
      window.removeEventListener("pointermove", mv);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
  };
  const volWheel = (e: React.WheelEvent) => {
    const s = usePlayerStore.getState();
    s.setVolume(Math.min(1, Math.max(0, s.volume - Math.sign(e.deltaY) * 0.03)));
  };

  return (
    <footer className="bar">
      <div className="np" onClick={() => cur && setPlayerView("deck")} title="Now Playing">
        <div className="art">
          {cur?.coverArt ? (
            <img src={coverArtUrl(cur.coverArt, 120) ?? ""} alt="" />
          ) : cur?.path && !cur.subsonicId ? (
            <LocalCover path={cur.path} />
          ) : null}
        </div>
        <div style={{ minWidth: 0 }}>
          <div className="tt">
            <Marquee text={cur?.title ?? "EKO"} />
          </div>
          <div className="ar">{cur ? (cur.artist ?? "") : "Pick an album to play"}</div>
        </div>
      </div>

      <div className="center">
        <div className={`controls${tracks.length === 0 ? " dim" : ""}`}>
          <div
            className={`tbtn sm${shuffle ? " on" : ""}`}
            title="Shuffle"
            onClick={() => usePlayerStore.getState().toggleShuffle()}
          >
            <svg viewBox="0 0 24 24">
              <path d="M3 7h4l9 10h5M16 7h5v5M3 17h4l3-3.4" />
            </svg>
          </div>
          <div className="tbtn" title="Previous" onClick={() => usePlayerStore.getState().prev()}>
            <svg viewBox="0 0 24 24">
              <path d="M7 6h2v12H7zM19 6v12l-9-6z" />
            </svg>
          </div>
          <div
            className="tbtn play"
            title="Play / Pause"
            onClick={() => usePlayerStore.getState().togglePlay()}
          >
            {isPlaying ? (
              <svg viewBox="0 0 24 24">
                <path d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24">
                <path d="M8 5v14l11-7z" />
              </svg>
            )}
          </div>
          <div className="tbtn" title="Next" onClick={() => usePlayerStore.getState().next()}>
            <svg viewBox="0 0 24 24">
              <path d="M15 6h2v12h-2zM5 6l9 6-9 6z" />
            </svg>
          </div>
          <div
            className={`tbtn sm${repeat !== "off" ? " on" : ""}`}
            title="Repeat"
            onClick={() => usePlayerStore.getState().cycleRepeat()}
          >
            <svg viewBox="0 0 24 24">
              <path d="M4 9a4 4 0 0 1 4-4h9M17 5l-2-2M17 5l-2 2M20 15a4 4 0 0 1-4 4H7M7 19l2-2M7 19l2 2" />
            </svg>
          </div>
        </div>
        <div className="scrub">
          <span className="t">{formatTime(currentTime)}</span>
          <div
            className="track"
            onPointerDown={(e) => {
              if (usePlayerStore.getState().duration <= 0) return;
              (e.target as Element).setPointerCapture(e.pointerId);
              const s = usePlayerStore.getState();
              s.beginScrub();
              s.scrubMove(scrubSecs(e.clientX, e.currentTarget));
            }}
            onPointerMove={(e) => {
              if (e.buttons & 1)
                usePlayerStore.getState().scrubMove(scrubSecs(e.clientX, e.currentTarget));
            }}
            onPointerUp={(e) => {
              usePlayerStore.getState().endScrub(scrubSecs(e.clientX, e.currentTarget));
            }}
            onPointerCancel={(e) => {
              usePlayerStore.getState().endScrub(scrubSecs(e.clientX, e.currentTarget));
            }}
          >
            <div className="fill" style={{ width: `${prog * 100}%` }} />
            <div className="knob" style={{ left: `${prog * 100}%` }} />
          </div>
          <span className="t r">-{formatTime(Math.max(0, duration - currentTime))}</span>
        </div>
      </div>

      <div className="right">
        <div
          className="icon-btn"
          title="Up Next"
          onClick={() => useUiStore.getState().toggleQueue()}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M4 7h11M4 12h11M4 17h7" />
            <path d="M16 13.5v7l5-3.5z" fill="currentColor" stroke="none" />
          </svg>
        </div>
        <div className="minispec">
          <div className="well">
            <div className="screen">
              <Spectrum bands={13} segs={8} bargap={2} />
            </div>
          </div>
        </div>
        <div className="sig">
          <div className="rate">
            <span
              className={`pure-dot${bitPerfect ? " on" : ""}`}
              title={
                bitPerfect
                  ? "Bit-perfect — untouched signal path"
                  : "Volume/EQ is shaping the signal"
              }
            />
            {cur?.sampleRate
              ? `${(cur.sampleRate / 1000).toFixed(cur.sampleRate % 1000 ? 1 : 0)}k`
              : "—"}
          </div>
        </div>
        <div
          className="knob-dial"
          title="Volume (drag or scroll)"
          onPointerDown={volDrag}
          onWheel={volWheel}
        >
          <div className="hub" />
          <div className="ind" style={{ transform: `rotate(${volDeg}deg)` }} />
        </div>
      </div>
    </footer>
  );
}
