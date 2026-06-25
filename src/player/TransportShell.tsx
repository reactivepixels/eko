import { useUiStore } from "../store/useUiStore";
import { Slot } from "./Slot";
import { LocalCover } from "./LocalCover";
import { Marquee } from "./Marquee";
import { SleepTimerControl } from "./SleepTimerControl";
import { useNowPlaying } from "../hooks/useNowPlaying";

/**
 * Generic transport footer — same layout/chrome as TransportBar, but the controls, seek, meter
 * and volume are SLOTS resolved to the active preset's variant. Now-playing, lyrics/sleep/queue
 * are shared chrome, token-skinned by palette.
 */
export function TransportShell() {
  const np = useNowPlaying();
  const toggleQueue = useUiStore((s) => s.toggleQueue);
  const lyricsOpen = useUiStore((s) => s.lyricsOpen);
  const toggleLyrics = useUiStore((s) => s.toggleLyrics);

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
        <Slot slot="transport" />
        <Slot slot="seek" />
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
        <Slot slot="meter" />
        <Slot slot="volume" />
      </div>
    </footer>
  );
}
