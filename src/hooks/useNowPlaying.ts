import { usePlayerStore } from "../store/usePlayerStore";
import { useUiStore } from "../store/useUiStore";
import { coverAt } from "../subsonic/nativeSubsonic";
import type { Track } from "../types";

/** Current-track metadata + cover resolution for any theme's now-playing chrome. The cover
 *  source (server URL vs local embedded art path) is normalised here so renderers never
 *  size a cover URL themselves. */
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
    coverUrl: (size: number) => coverAt(track?.coverUrl, size),
    /** Local file path whose embedded art a `<LocalCover>` can render, or null. */
    coverPath: track?.path && !track.subsonicId ? track.path : null,
    openDeck: () => {
      if (track) setPlayerView("deck");
    },
  };
}
