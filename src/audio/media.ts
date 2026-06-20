import { invoke } from "@tauri-apps/api/core";

/** Bridge to the OS "Now Playing" card + hardware media keys (macOS). All best-effort —
 *  failures are swallowed so a platform without support never breaks playback. */

export function mediaMetadata(meta: {
  title: string;
  artist: string;
  album: string;
  coverUrl?: string;
  duration?: number;
}) {
  void invoke("media_metadata", {
    title: meta.title,
    artist: meta.artist,
    album: meta.album,
    coverUrl: meta.coverUrl ?? null,
    duration: meta.duration && meta.duration > 0 ? meta.duration : null,
  }).catch(() => {});
}

export function mediaPlayback(playing: boolean, elapsed: number) {
  void invoke("media_playback", { playing, elapsed: Math.max(0, elapsed) }).catch(() => {});
}

export function mediaStopped() {
  void invoke("media_stopped").catch(() => {});
}
