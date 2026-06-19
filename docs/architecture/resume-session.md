# Resume last session — design

**Status:** spec only. **Build with the maintainer** (startup-regression risk; needs his app running to
verify restore actually works). This is the plan so the build session starts from decisions made.

## Goal

On launch, EKO restores what you were listening to — the track, its queue, and the playback
position — **paused, not auto-playing**. Press play and you're back exactly where you left off.

## Why it's gated on the maintainer

The existing `persist.ts` restores *settings* at startup inside a `try/catch` that silently
falls back to defaults. Resume extends that to *playback state*, which is riskier:
- A bug in restore that throws **outside** the guard could break launch.
- Restoring a **Navidrome** queue depends on the server connection, which isn't available at the
  instant the store hydrates.
So we implement it defensively and the maintainer confirms a real restore on his machine.

## Design

### What to persist (extend `Persisted` in `persist.ts`)

```ts
lastSession?: {
  source: "local" | "navidrome";
  queue: TrackRef[];        // enough to reconstruct rows without a server round-trip
  index: number;            // current track in the queue
  positionSec: number;      // last playback position
  savedAt: number;          // for staleness / debugging
}
```

`TrackRef` = the minimum to rebuild a queue row: for **local**, the file path + cached
title/artist/duration; for **navidrome**, the song id + cached display fields + the stream URL
builder inputs. Persist cached display fields so the UI can render immediately, before any
server call.

- Save (debounced, already wired via `startAutosave`) whenever the queue / current index /
  position changes. Throttle the position write (e.g. every ~5 s + on pause/stop) so we don't
  thrash localStorage.

### Restore (on launch, in `restoreState`)

1. Read `lastSession`; if absent, do nothing (today's behaviour).
2. Rehydrate the **queue + current index into the store** — but **do not call the engine** and
   **do not auto-play**. The deck shows the last track, paused, position at `positionSec`.
3. Defer the position so the *first* play resumes from `positionSec`:
   - Set a `pendingResumeSec` in the store. On the next user-initiated play of the current
     track, after `engine_play`, issue one `engine_seek(pendingResumeSec)` then clear it.
   - (Avoids seeking a stream that hasn't buffered; play starts decode, then we seek.)
4. **Navidrome:** if `source === "navidrome"` and not yet connected, keep the restored queue as
   display-only until the user connects/plays; rebuild stream URLs lazily on play. Never block
   startup on the network.
5. Wrap the whole restore in the existing `try/catch` and **guard each step** — any failure
   leaves a clean default session (never a broken launch).

### Edge cases

- Empty/!valid queue → ignore.
- A local file that no longer exists → on play, skip with a soft notice; don't crash.
- Position beyond the (re-decoded) duration → clamp to 0.
- Don't restore `playing: true` — always resume paused (intentional; avoids surprise audio on
  launch).

## Verification (the maintainer)

- Play a local album mid-track, quit, relaunch → same track shown, paused, at the right spot;
  press play → resumes from there.
- Same for a Navidrome queue after connecting.
- Kill the app uncleanly → next launch still starts cleanly (no corruption).
- Corrupt the persisted blob by hand → app falls back to a fresh session, no error.
