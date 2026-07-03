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

/** Player-state string a companion app expects — mirrors Spotify's own notification. */
export type BroadcastPlayerState = "Playing" | "Paused" | "Stopped";

// Skip redundant IPC calls when the poll loop / scrub handlers re-push the same state.
let lastBroadcast: { state: BroadcastPlayerState; name: string; artist: string } | null = null;

/** Post a macOS distributed notification ("com.reactivepixels.eko.playbackState") so a
 *  companion app can react to play/pause/stop/track-change, mirroring the shape of
 *  Spotify's own notification. Best-effort — failures are swallowed like the rest of this
 *  module. No-op (throws away silently) on non-macOS builds via the Rust side. */
export function broadcastPlayback(state: BroadcastPlayerState, name: string, artist: string) {
  if (
    lastBroadcast &&
    lastBroadcast.state === state &&
    lastBroadcast.name === name &&
    lastBroadcast.artist === artist
  ) {
    return;
  }
  lastBroadcast = { state, name, artist };
  void invoke("broadcast_playback", { state, name, artist }).catch(() => {});
}
