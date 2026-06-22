import { useEffect, useRef, useState } from "react";
import { usePlayerStore } from "../store/usePlayerStore";
import { getLyricsBySongId, getLyricsLegacy, getConfig } from "../subsonic/client";
import type { LyricsResult } from "../subsonic/client";
import { activeLyricLine } from "../lib/lyrics";

/** Lyrics panel — fetches and displays synced or plain lyrics for the current track.
 *  Synced lyrics auto-scroll to the active line and highlight it. */
export function LyricsPanel({ onClose }: { onClose: () => void }) {
  const track = usePlayerStore((s) =>
    s.currentIndex !== null ? (s.tracks[s.currentIndex] ?? null) : null,
  );
  const currentTimeSec = usePlayerStore((s) => s.currentTime);
  const posMs = currentTimeSec * 1000;

  const [lyrics, setLyrics] = useState<LyricsResult | null>(null);
  const [loading, setLoading] = useState(false);

  // Track which song we loaded lyrics for so we reload on track change.
  const loadedForId = useRef<string | null>(null);

  useEffect(() => {
    const trackKey = track?.subsonicId ?? track?.path ?? null;
    if (!track || trackKey === loadedForId.current) return;

    // Only fetch lyrics when a Subsonic server is configured.
    if (!getConfig()) {
      setLyrics({ synced: null, unsynced: null });
      loadedForId.current = trackKey;
      return;
    }

    loadedForId.current = trackKey;
    setLoading(true);
    setLyrics(null);

    void (async () => {
      let result: LyricsResult;
      if (track.subsonicId) {
        result = await getLyricsBySongId(track.subsonicId);
        // If OpenSubsonic returned nothing, fall back to legacy endpoint.
        if (!result.synced && !result.unsynced) {
          result = await getLyricsLegacy(track.artist ?? null, track.title ?? null);
        }
      } else {
        // Local track — no Subsonic id, try legacy by artist+title.
        result = await getLyricsLegacy(track.artist ?? null, track.title ?? null);
      }
      setLyrics(result);
      setLoading(false);
    })();
  }, [track]);

  // Auto-scroll the active line into view.
  const activeIndex = lyrics?.synced ? activeLyricLine(lyrics.synced, posMs) : -1;
  const activeRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (activeRef.current && scrollRef.current) {
      const container = scrollRef.current;
      const el = activeRef.current;
      const top = el.offsetTop - container.clientHeight / 2 + el.clientHeight / 2;
      container.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
    }
  }, [activeIndex]);

  const hasContent = lyrics && (lyrics.synced || lyrics.unsynced);

  return (
    <>
      <div className="q-backdrop" onClick={onClose} />
      <aside className="queue lyrics-panel">
        <div className="queue-head">
          <b>Lyrics</b>
          <div className="spacer" />
          <div
            className="q-close icon-btn"
            onClick={onClose}
            title="Close"
            role="button"
            tabIndex={0}
            aria-label="Close lyrics panel"
            onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? onClose() : undefined)}
          >
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              aria-hidden="true"
            >
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </div>
        </div>

        <div className="lyrics-scroll" ref={scrollRef}>
          {!track ? (
            <div className="lyrics-empty">Nothing playing.</div>
          ) : loading ? (
            <div className="lyrics-empty lyrics-loading">
              <span className="update-spinner" aria-label="Loading lyrics" />
            </div>
          ) : !hasContent ? (
            <div className="lyrics-empty">No lyrics available.</div>
          ) : lyrics.synced ? (
            <div className="lyrics-lines" role="list" aria-label="Synced lyrics">
              {lyrics.synced.map((line, i) => {
                const isActive = i === activeIndex;
                return (
                  <div
                    key={`${line.start}-${i}`}
                    className={`lyrics-line${isActive ? " active" : ""}`}
                    ref={isActive ? activeRef : null}
                    role="listitem"
                    aria-current={isActive ? "true" : undefined}
                  >
                    {line.value || " " /* non-breaking space for empty instrumental lines */}
                  </div>
                );
              })}
            </div>
          ) : (
            <pre className="lyrics-plain">{lyrics.unsynced}</pre>
          )}
        </div>
      </aside>
    </>
  );
}
