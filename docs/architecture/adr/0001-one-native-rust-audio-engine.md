# ADR 0001 — One native Rust audio engine (retire Web Audio)

**Status:** Accepted

## Context

EKO's stated goal is bit-perfect playback: the samples written to the DAC must
be byte-for-byte identical to the samples in the source file, with no resampling
and no implicit gain change introduced by the platform.

The original implementation used the Web Audio API (a browser API surfaced via
Tauri's WKWebView) to decode and play audio. Web Audio is convenient — it is
JavaScript-native and requires no native code — but it is structurally
incompatible with bit-perfect output:

- WKWebView's Web Audio pipeline resamples everything to the OS mixer's current
  rate. That rate is whatever Audio MIDI Setup happens to be set to; EKO cannot
  control it from JavaScript.
- The OS audio mixer applies its own gain staging and potentially its own
  processing (spatial audio, EQ via system preferences) on top.
- There is no JavaScript API to set the CoreAudio device's nominal sample rate.
- Any custom DSP (EQ, spectrum analysis) applied in JavaScript runs after
  decode in a context that has already been resampled.

Additionally, maintaining two code paths — one for local files (Web Audio) and
one intended for server streams — increases complexity without benefit.

## Decision

Retire the Web Audio API entirely. Implement a single native Rust audio engine
(`src-tauri/src/engine.rs`) that handles all sources and all DSP:

- `symphonia` for format detection and packet decode (supports FLAC, ALAC, MP3,
  AAC, WAV, OGG, Opus, and more).
- `cpal` for the output stream, opened at the source file's own sample rate.
- `HttpSource` for server streams (Navidrome/Subsonic), using
  `reqwest::blocking` inside the decode thread.
- Custom 10-band RBJ biquad EQ and square-law volume, with a hard bypass path
  when both are at their neutral values.
- `rustfft` for the 32-band spectrum analyser.

The JavaScript side is reduced to a thin invoke wrapper and a UI state store.
No audio processing runs in the web layer.

## Consequences

**Positive:**
- Bit-perfect output becomes achievable: cpal opens the stream at the file rate,
  and the bypass path copies samples untouched.
- CoreAudio HAL rate-setting (ADR 0002) can be added as a pure Rust module.
- One code path for local and server sources; consistent behaviour across all
  formats.
- DSP quality is fully under EKO's control (filter coefficients, precision,
  bypass logic).
- Binary size stays small — no bundled audio engine beyond what Rust crates
  bring in.

**Negative / trade-offs:**
- Native Rust code means a `cargo rebuild` on every engine change (slower
  iteration than hot-reloading JS).
- The `Mutex<Vec<f32>>` shared buffer is not lock-free; there is a theoretical
  priority-inversion risk in the cpal callback. Observed in practice: none.
- Losing Web Audio also loses its format-fallback for browser-only builds;
  EKO is macOS-only (v1), so this is acceptable.
