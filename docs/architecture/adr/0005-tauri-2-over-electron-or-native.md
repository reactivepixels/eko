# ADR 0005 — Tauri 2 (Rust core + web UI) over native Swift or Electron

**Status:** Accepted

## Context

EKO needs a desktop application shell that can:

1. Host a Rust audio engine with direct access to CoreAudio.
2. Render a custom design-heavy UI at native performance.
3. Be built and maintained by one developer who is a web developer first and a
   systems programmer second.
4. Produce a small, fast binary — a multi-hundred-megabyte download is
   inconsistent with the precision-instrument positioning.

Three approaches were considered:

**Native Swift / SwiftUI (or AppKit)**
The natural choice for macOS audio software (Logic Pro, Roon, Audirvana all use
native stacks). SwiftUI or AppKit give full access to CoreAudio, AVFoundation, and
Metal. The constraint: the maintainer is not a Swift developer. The UI work — the neumorphic
design system, Zustand-like state management, the spectrum canvas, the signal-path
component — would require learning a new UI paradigm in addition to the audio
engineering work. A SwiftUI app also cannot straightforwardly embed a Rust audio
engine; bridging requires FFI via C headers or a framework like UniFFI. Total
estimated ramp: months.

**Electron**
the maintainer already knows web UI development; Electron would make the UI layer
straightforward. The problems:

- Electron ships its own Chromium and Node.js; the installer is typically
  150–300 MB. This is at odds with the "precision instrument" brand.
- Electron's audio story is Web Audio (see ADR 0001) or a native addon via
  `node-gyp`. Writing a `node-gyp` Rust audio addon that calls CoreAudio is
  possible but architecturally awkward — the Rust lives in a Node addon rather
  than in a first-class Rust binary.
- Electron's sandboxing is weaker than Tauri's; memory usage is higher.

**Tauri 2**
- The application core is Rust; the UI is rendered in WKWebView (the system
  WebKit on macOS, not a bundled browser). Binary size is typically 5–20 MB.
- Rust code has direct, idiomatic access to CoreAudio via FFI — the same Rust
  that handles the audio engine also owns the HAL rate-setting in
  `src-tauri/src/coreaudio.rs`.
- The UI layer is React + TypeScript + Vite — exactly the maintainer's existing skill set.
  Hot-reloading works on the frontend; Rust changes trigger a cargo rebuild.
- Tauri 2's security model (capability-based permissions, no Node in the
  renderer) is stricter than Electron.
- The `stream://` custom protocol for cover art, window management, the app menu,
  and system tray are all first-class Tauri features.

## Decision

Use **Tauri 2** with a Rust core and a React 19 + TypeScript + Vite 6 frontend.

The audio engine (`engine.rs`, `coreaudio.rs`) lives entirely in Rust. The
frontend is a web UI communicating with Rust via Tauri's `invoke` / event IPC.
This matches the skills the maintainer has (web development) with the platform requirements
(real Rust audio, CoreAudio HAL access, small binary).

## Consequences

**Positive:**
- the maintainer can work at full speed on the UI without learning Swift or a new UI
  framework.
- The Rust audio engine is a first-class binary, not a native addon; cargo
  manages its dependencies (symphonia, cpal, rustfft, reqwest).
- WKWebView renders the neumorphic design system identically to how it would
  look in a browser; no platform-specific rendering surprises.
- The macOS overlay titlebar (`titleBarStyle: "Overlay"`) is a built-in Tauri
  feature.
- Binary size is consistent with the precision-instrument positioning.

**Negative / trade-offs:**
- WKWebView's JavaScript engine is not V8; some JS API surface differs from
  Chrome. In practice, EKO uses no Chrome-specific APIs.
- The IPC bridge (WKWebView ↔ Rust) has finite throughput. High-frequency calls
  (seek drags, volume drags) must be throttled (~20/sec) to avoid stutter; this
  is a known constraint with documented mitigation in the codebase.
- Tauri 2 is not 1.0-stable on all platforms. The macOS target is well-tested
  and EKO is macOS-only (v1), so this is low risk in practice.
- JavaScript timers in hidden WKWebView windows are throttled by macOS; this
  constrained the mini player design (it reads Rust state directly rather than
  receiving events from the main window — see architecture overview).
- A future iOS port requires adding Tauri's iOS target and a different audio
  session path; the Rust audio engine itself is portable, but the CoreAudio
  HAL module is macOS-only and must be conditionally compiled out.
