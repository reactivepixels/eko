# EKO — Architecture Overview

EKO is a macOS music player built for audio fidelity first. The central
architectural premise is simple: **one native Rust audio engine handles
everything** — local files and server streams, decode and output, DSP and
bypass — and the React/TypeScript UI is purely a control surface that reads
state from that engine. Nothing audio-related runs in JavaScript.

This document covers the end-to-end signal path, how the bit-perfect bypass
works and how it stays honest, the streaming buffer design, the frontend/backend
split, and the macOS-specific decisions.

---

## 1. The one-engine philosophy

Early in EKO's development the player used the Web Audio API for decoding and
output. Web Audio is convenient but it is incompatible with the goal: the browser
audio pipeline resamples everything to the OS mixer rate, applies its own gain
staging, and gives you no access to device-rate control. Bit-perfect output — the
kind that beats Apple Music on a resolving DAC — is structurally impossible through
Web Audio.

EKO replaced it entirely. Today the engine lives in `src-tauri/src/engine.rs` and
is the only audio path. Every source — a local FLAC, a Navidrome stream, a
Subsonic HTTP URL — is decoded in Rust via `symphonia`, rate-matched to the
device via the CoreAudio HAL, and output via `cpal`. The JavaScript side never
touches a sample.

This means one code path to maintain, one bit-perfect guarantee to uphold, and
one place to look when something sounds wrong.

---

## 2. Signal path end-to-end

```
┌─────────────────────────────────────────────────────────────────────────┐
│  SOURCE                                                                 │
│  local file  ──► FileSource (std::fs)                                  │
│  server URL  ──► HttpSource (reqwest::blocking, streaming)              │
└──────────────────────────┬──────────────────────────────────────────────┘
                           │ bytes
                           ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  DECODE  (symphonia)                                                     │
│  Probe format → select decoder (FLAC/ALAC/MP3/AAC/WAV/OGG/Opus/…)      │
│  Decode packets → interleaved f32 samples at src_rate / src_channels    │
│  Convert to f32 / interleave / channel-map                              │
│  Append to Mutex<Vec<f32>> shared buffer (growing, random-access)       │
└──────────────────────────┬───────────────────────────────────────────────┘
                           │ shared sample buffer
                           ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  CPAL OUTPUT STREAM  (opened at src_rate / src_channels)                │
│                                                                          │
│  Per-sample in the callback:                                             │
│                                                                          │
│  if EQ flat AND volume == 1.0 AND dev_rate == src_rate                  │
│  └─► BYPASS: copy_from_slice (samples touch nothing)          ◄── ●     │
│                                                                          │
│  else                                                                    │
│  └─► 10-band RBJ biquad EQ  ──►  square-law volume scale               │
│                                                                          │
└──────────────────────────┬───────────────────────────────────────────────┘
                           │ PCM at src_rate
                           ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  COREAUDIO HAL  (src-tauri/src/coreaudio.rs, macOS-only)                │
│  kAudioDevicePropertyNominalSampleRate set to src_rate before stream     │
│  opens.  dev_rate read back after set → seal reflects reality.           │
│  EKO owns the device rate while a stream is active.                     │
└──────────────────────────┬───────────────────────────────────────────────┘
                           │ PCM at dev_rate (== src_rate when bit-perfect)
                           ▼
                        DAC / speakers
```

The key insight is the ordering: EKO tells the CoreAudio HAL what rate the
device should run at *before* cpal opens the stream. If the device accepts the
rate, the samples it receives are at the file's native rate — the OS never
resamples them. If the device refuses (e.g. a Bluetooth sink locked to 48 kHz),
`dev_rate` comes back different from `src_rate` and the seal says RESAMPLED. EKO
never silently lies about what's happening.

---

## 3. The bit-perfect bypass and the honest seal

"Bit-perfect" is a specific claim, not a marketing slogan. EKO defines it as:

> The samples written to the hardware are identical to the samples decoded from
> the source file: no resampling, no gain change, no filtering.

That claim holds only when **all three** conditions are met simultaneously:

| Condition | Why it matters |
|---|---|
| Software volume == 1.0 (unity) | Any other value applies a per-sample multiply |
| EQ flat (all bands 0 dB) | Any active band runs the biquad filter chain |
| Device rate == file rate | Mismatch means the OS or cpal is resampling |

The bypass is a single `if` in the cpal callback. When the condition is true the
callback calls `copy_from_slice` directly from the shared buffer into the output
buffer — no DSP, no multiply, no branch on each sample. When false, the full chain
runs.

`EngineStatus` reports `rate` (stream rate), `src_rate` (decoded file rate), and
`dev_rate` (actual device rate read back from CoreAudio). The frontend seal
(`SignalPath.tsx`) reads all three plus the EQ and volume state and renders:

- **BIT-PERFECT (orange ring, checkmark)** — bypass active, all conditions met.
- **RESAMPLED** — `dev_rate != src_rate`.
- **EQ** — any band non-zero.
- **VOLUME** — software volume not unity.

The seal reflects the engine's actual state on each status poll. It cannot be
forced; it turns on when the physics are right.

---

## 4. Streaming decode into a growing buffer

Server tracks (Navidrome/Subsonic) present a challenge: you cannot seek into a
stream that hasn't been downloaded yet, and you cannot run FFT on samples you
don't have. EKO's solution is the shared `Mutex<Vec<f32>>` buffer.

`HttpSource` uses `reqwest::blocking` in the decode thread. As the network
delivers bytes, symphonia decodes packets and appends the resulting f32 samples to
the shared buffer. Meanwhile the cpal callback reads ahead from wherever playback
is positioned in that buffer. If the callback catches up to the decode frontier
(buffer starvation), it inserts silence and retries — audio dropouts are
preferable to a crash.

Because the buffer grows monotonically (no circular ring — the whole decoded file
accumulates), seek is a simple index write and FFT has random access. The
trade-off is memory: a 50-minute 96/24 stereo album is roughly 100 MB in f32.
That is acceptable for a desktop audiophile player. Local files take the same
path, which keeps the cpal callback identical regardless of source.

---

## 5. Frontend / backend split

```
┌───────────────────────────────────────────────────────────────────────┐
│  RUST (Tauri core)                                                    │
│                                                                       │
│  engine.rs         audio engine (decode / DSP / output)              │
│  coreaudio.rs      CoreAudio HAL FFI (rate-set, rate-read)           │
│  metadata.rs       lofty tag reader + cover art extraction           │
│  stream.rs         stream:// custom protocol (cover art proxy)       │
│  lib.rs            Tauri builder, command registry, Engine managed   │
└──────────────────────────────────┬────────────────────────────────────┘
                                   │  Tauri invoke / events (IPC)
┌──────────────────────────────────▼────────────────────────────────────┐
│  TYPESCRIPT / REACT (Vite 6, WKWebView)                               │
│                                                                       │
│  src/audio/nativeEngine.ts   thin invoke wrappers + bands poller     │
│  src/store/usePlayerStore.ts Zustand: queue, transport, EQ,          │
│                              volume, outputDevice, engineInfo         │
│  src/store/useUiStore.ts     view / source / theme / sort / layout   │
│  src/player/*.tsx            all UI components                        │
└───────────────────────────────────────────────────────────────────────┘
```

The IPC surface is narrow and deliberate. The frontend invokes named commands
(`engine_play`, `engine_pause`, `engine_seek`, `engine_set_eq`,
`engine_set_volume`, `engine_set_device`, etc.) and polls status
(`engine_status`, `engine_bands`) on a timer. The engine emits no Tauri events
toward the frontend — the frontend is the poller. This makes the data flow easy
to reason about: if something looks wrong in the UI, start by checking what
`engine_status` is returning.

### IPC throttling

The WKWebView IPC bridge has finite bandwidth. During drag interactions (seek
scrub, volume dial) the frontend generates many events per second. EKO throttles
`invoke` calls to approximately 20 per second during active drags — enough for
smooth visual feedback without flooding the bridge. Volume and seek values are
always applied at pointer-up regardless of throttle state.

### Zustand state stores

`usePlayerStore` owns everything that changes during playback: the queue array,
`currentIndex`, transport state (playing/paused/stopped), the software volume
value, the selected output device name, and `engineInfo` (the decoded
`EngineStatus` from the last status poll). Components subscribe to only the
slices they read; unrelated re-renders are not triggered.

`useUiStore` owns view state (deck vs library), active source (local vs
Navidrome), theme, library sort order, queue panel open/closed, and compact
(mini-player) mode.

---

## 6. macOS-specific decisions

### Overlay titlebar

The main window uses `titleBarStyle: "Overlay"` so the macOS traffic-light
buttons float over the custom header rather than occupying a separate system bar.
`setDecorations` must never be toggled at runtime — doing so destroys the overlay
and cannot be recovered without a window rebuild. This is a hard constraint in
the codebase.

### Hidden-window timer throttling

macOS throttles JavaScript timers in hidden or minimised windows. The mini player
window is separate (always-on-top) and its JavaScript timers would stall if it
relied on the main window forwarding events. EKO sidesteps this by having the
mini player call `engine_status` and `engine_now_playing` directly from its own
Tauri context. The Rust engine is the single source of truth; both windows read
it independently. No main→mini event bridge exists; don't reintroduce one.

### CoreAudio ownership

EKO sets the device's nominal sample rate when it opens a stream and does not
restore it when the stream closes. This mirrors Roon's and Audirvana's behaviour:
the player owns the device rate while it is active. Other applications that open
the same device after EKO will inherit whatever rate EKO left it at. This is a
deliberate trade-off — restoring the rate on close would require detecting all
playback-end conditions, including abnormal termination.

---

## 7. Known limits and honest caveats

**macOS only (v1).** CoreAudio HAL rate-setting has no equivalent on iOS
(`AVAudioSession` controls the session rate, not the hardware), so the bit-perfect
bypass cannot be offered on iOS in the same form. The iOS port is parked until
the macOS app has traction.

**Mutex in the callback.** The cpal output callback acquires a `Mutex` to read
from the shared sample buffer. In real-time audio this is technically incorrect —
a lock in the audio thread can cause a priority inversion that produces a dropout.
In practice, contention is very low (the decode thread appends quickly and
releases; the callback holds for a bounded read), and no dropouts have been
observed. A lock-free ring buffer would be the correct fix if this becomes a
problem.

**No gapless yet.** Tracks are played as discrete streams; the cpal stream is torn
down and rebuilt between tracks. A full gapless design is documented in
[`docs/architecture/gapless.md`](gapless.md). It is deferred because it requires
ear-testing with the maintainer present and carries regression risk to the decode loop.

**No MQA or DSD.** symphonia does not decode either format. These are Tier 3
items.

**No exclusive (hog) mode.** cpal does not expose CoreAudio exclusive mode
cleanly. EKO sets the device rate via the HAL which achieves the primary goal
(no OS resample), but other applications can still open the same device
concurrently. True exclusive mode is a Tier 3 item.
