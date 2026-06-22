import { useUiStore } from "../store/useUiStore";
import { formatTime } from "../lib/format";
import { Spectrum } from "./Spectrum";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";
import { useNowPlaying } from "../hooks/useNowPlaying";
import { useTransport } from "../hooks/useTransport";
import { useVolume } from "../hooks/useVolume";
import { useScrub } from "../hooks/useScrub";
import { SleepTimerControl } from "./SleepTimerControl";

/**
 * Porcelain transport bar — pure presentation over the shared control hooks. Owns no audio
 * logic, no data-source access, and no bit-perfect derivation (that's `useSignalPath`, the
 * single source of truth shared with the deck's seal).
 */
export function TransportBar() {
  const np = useNowPlaying();
  const tr = useTransport();
  const vol = useVolume();
  const scrub = useScrub();
  const toggleQueue = useUiStore((s) => s.toggleQueue);
  const lyricsOpen = useUiStore((s) => s.lyricsOpen);
  const toggleLyrics = useUiStore((s) => s.toggleLyrics);

  const prog = scrub.progress;

  return (
    <footer className="bar">
      <div
        className="np"
        onClick={np.openDeck}
        title="Now Playing"
        role="button"
        tabIndex={0}
        aria-label="Now Playing — open deck view"
        onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? np.openDeck() : undefined)}
      >
        <div className="art" aria-hidden="true">
          {np.coverUrl(120) ? (
            <img src={np.coverUrl(120) ?? ""} alt="" />
          ) : np.coverPath ? (
            <LocalCover path={np.coverPath} />
          ) : null}
        </div>
        <div style={{ minWidth: 0 }}>
          <div className="tt">
            <Marquee text={np.title} />
          </div>
          <div className="ar">{np.hasTrack ? (np.artist ?? "") : "Pick an album to play"}</div>
        </div>
      </div>

      <div className="center">
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
            onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? tr.toggleShuffle() : undefined)}
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
            onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? tr.prev() : undefined)}
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
            onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? tr.togglePlay() : undefined)}
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
            onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? tr.next() : undefined)}
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
            onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? tr.cycleRepeat() : undefined)}
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M4 9a4 4 0 0 1 4-4h9M17 5l-2-2M17 5l-2 2M20 15a4 4 0 0 1-4 4H7M7 19l2-2M7 19l2 2" />
            </svg>
          </div>
        </div>
        <div className="scrub" role="group" aria-label="Seek">
          <span className="t" aria-label={`Current time: ${formatTime(scrub.currentTime)}`}>
            {formatTime(scrub.currentTime)}
          </span>
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
            <div className="fill" style={{ width: `${prog * 100}%` }} />
            <div className="knob" aria-hidden="true" style={{ left: `${prog * 100}%` }} />
          </div>
          <span className="t r" aria-label={`Remaining: ${formatTime(scrub.remaining)}`}>
            -{formatTime(scrub.remaining)}
          </span>
        </div>
      </div>

      <div className="right">
        <div
          className={`icon-btn${lyricsOpen ? " on" : ""}`}
          title="Lyrics"
          onClick={toggleLyrics}
          role="button"
          tabIndex={0}
          aria-label="Lyrics"
          aria-pressed={lyricsOpen}
          onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? toggleLyrics() : undefined)}
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden="true"
          >
            <path d="M9 18V5l12-2v13" />
            <circle cx="6" cy="18" r="3" />
            <circle cx="18" cy="16" r="3" />
          </svg>
        </div>
        <SleepTimerControl />
        <div
          className="icon-btn"
          title="Up Next"
          onClick={toggleQueue}
          role="button"
          tabIndex={0}
          aria-label="Up Next queue"
          onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? toggleQueue() : undefined)}
        >
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden="true"
          >
            <path d="M4 7h11M4 12h11M4 17h7" />
            <path d="M16 13.5v7l5-3.5z" fill="currentColor" stroke="none" />
          </svg>
        </div>
        <div className="minispec" aria-hidden="true" role="presentation">
          <div className="well">
            <div className="screen">
              <Spectrum bands={13} segs={8} bargap={2} />
            </div>
          </div>
        </div>
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
          <div className="hub" aria-hidden="true" />
          <div className="ind" aria-hidden="true" style={{ transform: `rotate(${vol.deg}deg)` }} />
        </div>
      </div>
    </footer>
  );
}
