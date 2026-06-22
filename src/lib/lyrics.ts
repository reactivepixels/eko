import type { SyncedLyricLine } from "../subsonic/client";

/** Return the index of the currently-active synced lyric line given the current position (ms).
 *  Returns -1 for an empty line list.
 *  The active line is the last one whose `start` timestamp is ≤ posMs. */
export function activeLyricLine(lines: SyncedLyricLine[], posMs: number): number {
  if (lines.length === 0) return -1;
  let idx = 0;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].start <= posMs) idx = i;
    else break;
  }
  return idx;
}
