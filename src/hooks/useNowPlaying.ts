import { usePlayerStore } from "../store/usePlayerStore";
import { useUiStore } from "../store/useUiStore";
import { coverArtUrl } from "../subsonic/client";
import type { Track } from "../types";

/** Current-track metadata + cover resolution for any theme's now-playing chrome. The cover
 *  source (server URL vs local embedded art path) is normalised here so renderers never call
 *  `coverArtUrl` directly. */
export function useNowPlaying() {
  const currentIndex = usePlayerStore((s) => s.currentIndex);
  const tracks = usePlayerStore((s) => s.tracks);
  const setPlayerView = useUiStore((s) => s.setPlayerView);
  const track: Track | null = currentIndex !== null ? (tracks[currentIndex] ?? null) : null;
  return {
    track,
    hasTrack: !!track,
    title: track?.title ?? "EKO",
    artist: track ? (track.artist ?? "") : null,
    sampleRate: track?.sampleRate ?? null,
    /** Server cover art URL at the given size, or null (local art uses `coverPath`). */
    coverUrl: (size: number) =>
      track?.coverArt ? (coverArtUrl(track.coverArt, size) ?? null) : null,
    /** Local file path whose embedded art a `<LocalCover>` can render, or null. */
    coverPath: track?.path && !track.subsonicId ? track.path : null,
    openDeck: () => {
      if (track) setPlayerView("deck");
    },
  };
}
