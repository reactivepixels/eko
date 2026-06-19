# Gapless playback — design (build with the maintainer, ear-test required)

## The tension
EKO's identity is **bit-perfect** = output at the file's own sample rate, untouched bits.
True gapless needs ONE continuous cpal stream across tracks. But a stream runs at ONE
device rate. So:
- Tracks that share a sample rate (a normal album: all 44.1/16 or all 96/24) → can be fed
  through one stream **gaplessly AND bit-perfectly**.
- A rate change between tracks → the stream must be torn down + rebuilt at the new rate
  (a tiny gap), OR the next track gets resampled (not bit-perfect). EKO should rebuild
  (keep bit-perfect) — a gap only between differing-rate tracks, which is rare mid-album.

This is exactly how Roon behaves. It's the honest design.

## Approach: append into the running stream when rates match
The decode loop already resamples every chunk to the session's `out_rate`/`out_ch` and
appends to `shared.samples`; the cpal callback just reads that growing buffer. So:

1. **Enqueue** — `engine_enqueue(path|url)` stores the next source on the session
   (Mutex<Option<Source>>), set by the frontend when the current track is ~10s from end.
2. **On EOF of the current decoder**, instead of `done=true`:
   - If a next source is queued, probe it. If its `file_rate == out_rate` (and channels
     fit) → build its decoder and **continue the same loop**, appending its resampled
     samples to `shared.samples`. Record the boundary sample offset; bump
     `track_seq: AtomicUsize`.
   - Else → set `done=true` as today; the frontend does a normal `next()` (stream rebuilds
     at the new rate — small gap, bit-perfect preserved).
3. **Frontend sync** — poll `track_seq`. When it increments, advance `currentIndex` + 1 and
   `pushNowPlaying()` WITHOUT restarting playback. Suppress the old poll-detected-end →
   `next()` path while a gapless continuation is armed.

## Edge cases to handle
- `total`/duration + the scrub bar must reflect the CURRENT track, not the concatenated
  buffer → track per-segment [start,len] and report position relative to the current one.
- Seek must clamp within the current track's segment.
- Stop / manual next/prev must cancel the armed next source and reset segments.
- URL (server) next-source needs its downloader thread started at enqueue time so the
  bytes are ready at the boundary (pre-buffer).
- Pre-buffer the next track early enough that decode keeps ahead of playback at the seam.

## Why not tonight
Touches the core decode loop, position/seek math, and the track-advance path — high
regression risk, and the payoff (no seam) can only be confirmed by ear. Build it with the maintainer
listening; keep the non-gapless path byte-for-byte unchanged as the fallback.
