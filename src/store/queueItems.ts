import type { QueueItem } from "../audio/nativeEngine";
import type { Track } from "../types";

// A counter, not `crypto.randomUUID`: EKO runs on WebKit builds that predate it.
let nextQid = 0;

/**
 * Give each track entering the queue its own slot id. The same song queued twice gets two,
 * which is what lets the engine tell them apart: a server track's `id` is the song's id, so
 * it repeats.
 */
export function withQids(tracks: Track[]): Track[] {
  return tracks.map((t) => ({ ...t, qid: `q${++nextQid}` }));
}

/**
 * The engine's copy of a queued track: how to play it, and what to show while it plays.
 *
 * A server track travels as its id and server, never its signed URL. The engine mints the
 * URL when the track starts (see `eko_core::player::ItemMedia`).
 */
export function toQueueItem(t: Track): QueueItem {
  return {
    uid: t.qid ?? t.id,
    media: t.subsonicId
      ? { kind: "remote", id: t.subsonicId, server: t.server ?? "" }
      : { kind: "file", path: t.path },
    title: t.title ?? "Unknown",
    artist: t.artist ?? "",
    album: t.album ?? "",
    durationMs: Math.max(0, Math.round((t.duration || 0) * 1000)),
    coverUrl: t.coverUrl ?? "",
    rg: {
      trackGain: t.rgTrackGain ?? null,
      trackPeak: t.rgTrackPeak ?? null,
      albumGain: t.rgAlbumGain ?? null,
      albumPeak: t.rgAlbumPeak ?? null,
    },
  };
}
