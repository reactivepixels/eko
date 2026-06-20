# Changelog

All notable changes to EKO will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
EKO uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The project is pre-1.0 — the API and feature set are still settling.

## [Unreleased]

Everything below reflects what has been built and verified to compile and run.
Items marked **[needs ear-verify]** are complete in code but have not been
confirmed by the maintainer's listening tests or Audio MIDI Setup inspection. Do not treat
those as shipping-quality until they pass the [QA checklist](docs/QA-CHECKLIST.md).

## [0.3.1] — 2026-06-20

Patch release: ship a working, downloadable macOS build.

### Fixed
- **macOS release pipeline** — releases now produce a working DMG **without** a paid Apple
  Developer ID: an ad-hoc (unsigned) build that opens via right-click → **Open** (or
  `xattr -dr com.apple.quarantine /Applications/EKO.app`). The workflow auto-upgrades to a
  signed + notarized build the moment the Apple signing secrets are configured. Previously every
  release run died at the codesign step ("failed to import keychain certificate").

> The **skinnable UI** — light/dark theme × user-selectable accent × the matte **Studio** skin
> (see [docs/skinning.md](docs/skinning.md)) — landed in the 0.3.0 commit; 0.3.1 is the first
> release you can actually download and run cleanly.

## [0.3.0] — 2026-06-20

### Added
- **System "Now Playing" + hardware media keys (macOS)** — the lock-screen / Control-Center
  now-playing card and the F7/F8/F9 keys (and headphone controls) now drive EKO, via
  `MPNowPlayingInfoCenter` / `MPRemoteCommandCenter` (the `souvlaki` crate, `src-tauri/src/media.rs`).
  Remote commands are routed into the existing `eko:cmd` event path, so one command channel
  serves the mini player and the OS alike. Metadata, play/pause and elapsed position push on
  every transition; server cover art appears on the card. **[needs the maintainer's eyes — media
  keys bind for the bundled `.app`, not `tauri dev`.]**
- **Right-click context menus** — albums (Play album / Play next / Add to queue), track rows
  (Play / Play next / Add to queue) and queue rows (Play now / Remove). One reusable
  `ContextMenu` component; viewport-clamped, theme-aware, closes on outside-click / Esc / scroll.
- **Crossfade between tracks** (`off` by default) — a same-rate transition can overlap with an
  equal-power fade (2–12 s, picker in the signal-path strip). The fade is baked into the shared
  PCM buffer by the decode thread, so **the realtime cpal callback is unchanged and steady-state
  playback (plus the bit-perfect bypass) is untouched** — only the short overlap region holds
  mixed samples. Falls back to the hard gapless join when the overlap can't be staged safely
  ahead of the play head (e.g. download-paced server streams). The next track is now armed as
  soon as it's playing (not 12 s before the end) so the decode-ahead continuation reliably fires.
  **[needs ear-verify.]**

## [0.2.0] — 2026-06-20

Security + robustness hardening over the v0.1.0 preview (no feature regressions):

- **Security:** the `stream://` proxy is now restricted to the configured Navidrome
  origin (was an open SSRF); the broad `**` filesystem capability was removed; a
  Content-Security-Policy was added; the Navidrome password moved from plaintext
  `localStorage` to the macOS Keychain.
- **Engine:** the realtime audio callback recovers from a poisoned lock instead of
  crashing; seek arithmetic is clamped; network streams get timeouts + a stall guard;
  a device error clears playback state; **the DAC's sample rate is restored on quit.**
- **Frontend:** gapless session state resets on queue edits; a poll generation guard
  fixes an after-stop race; the file-open path carries all track metadata; faster,
  stickier seek bar; ReplayGain now also works for Navidrome.

### Added

#### Audio engine
- Native Rust audio engine (`src-tauri/src/engine.rs`) — replaces all use of the
  Web Audio API. `symphonia` decodes local files; `cpal` opens the output stream
  at the file's own sample rate. Both local files and Navidrome/Subsonic HTTP
  streams travel the same code path.
- Bit-perfect bypass: when EQ is flat AND software volume is at unity (1.0), the
  cpal callback uses `copy_from_slice` to write samples untouched rather than
  passing them through the DSP chain.
- 10-band RBJ biquad EQ in the engine (`engine_set_eq`); each band is a
  second-order parametric filter computed per-sample. The entire chain is skipped
  when all bands are at 0 dB.
- Software volume with square-law scaling (`engine_set_volume`). Unity (100%)
  preserves the bit-perfect bypass; any other value routes through the multiply
  path.
- 32-band Rust FFT spectrum analyser (`rustfft`); exposed via `engine_bands`
  command and polled by the frontend at animation-frame rate.
- Streaming decode into a growing `Mutex<Vec<f32>>` buffer — playback of server
  tracks starts as soon as enough samples are decoded; seek and FFT have random
  access into the full buffer without waiting for a complete download.
- `HttpSource` for Navidrome/Subsonic: `reqwest::blocking` streams server URLs
  directly into the symphonia decoder.
- `engine_list_devices` / `engine_set_device`: cpal device enumeration and
  explicit DAC selection, persisted across relaunches.
- `EngineStatus` struct exposed to the frontend: `rate` (stream rate), `src_rate`
  (file rate), `dev_rate` (actual device rate read back from CoreAudio), `bits`,
  `codec`, `device`.
- ReplayGain volume normalisation (**off by default**) — **ear-verified 2026-06-19** (Album
  mode on a loud master applied −9.2 dB, audibly quieter; seal lit `REPLAYGAIN`). Full
  stack: reads RG tags (`metadata.rs`: track/album gain + peak, tolerant `"-6.34 dB"`
  parser); `engine_set_replaygain` applies the chosen gain in dB as a linear multiplier
  folded into the volume stage; a store mode (`off`/`track`/`album`, persisted) selects
  track vs album gain and **peak-limits** positive gains to prevent clipping; an
  Off/Track/Album picker in the signal path drives it, and the seal honestly shows
  `REPLAYGAIN` when active. Unity (off / 0 dB) preserves the bit-perfect bypass — an
  invariant now pinned by a unit test (`is_bitperfect`, refactored out of the realtime
  callback). Works for **local files and Navidrome/Subsonic** (OpenSubsonic `replayGain` tags
  mapped through the same path).
- Gapless playback **[needs ear-verify]** — same-rate tracks play through one continuous cpal
  stream with no seam (and bit-perfect); a rate change between tracks rebuilds the stream (tiny
  gap, bit-perfect preserved), exactly like Roon. Implemented by continuing the decode loop into
  a queued next source at end-of-track (`engine_enqueue` + `Source` + `open_source`), with
  per-track segment offsets (`seg_starts`) so position/seek/`seg` are reported relative to the
  current track. **The cpal callback is byte-identical** — it reads one contiguous buffer — so
  the bit-perfect path is untouched, and a single track collapses to the previous math. The
  frontend arms the next track ~12s before the end and advances the displayed track on the
  engine's `seg` without restarting. Unit-tested boundary math (`segment_at`).
- Resume last session (local queues) **[needs verify]** — the last local queue, current
  track, and position persist; on launch they're restored **paused** (never auto-playing),
  and the first play seeks to where you left off. Defensive: wrapped in the existing
  restore guard, server queues excluded (their stream URLs need a live login). See
  `docs/architecture/resume-session.md`.

#### CoreAudio device-rate matching — macOS **[needs ear-verify]**
- Hand-written CoreAudio HAL FFI (`src-tauri/src/coreaudio.rs`, macOS-only).
  Sets `kAudioDevicePropertyNominalSampleRate` on the selected output device to
  match the playing file's sample rate before the cpal stream opens.
- On a 44.1 kHz file the device switches to 44.1 kHz; on a 96 kHz file to 96 kHz.
  The OS mixer resample that Apple Music and most players silently apply is
  bypassed entirely — the same technique Roon and Audirvana use.
- `dev_rate` is read back from the device after the switch so the seal reflects the
  device's actual operating rate, not the intended one. If the device cannot match
  (e.g. a Bluetooth sink locked to 48 kHz) the seal honestly shows RESAMPLED.

#### Signal path display **[needs ear-verify with CoreAudio]**
- `SignalPath.tsx` (Concept G): SOURCE → ENGINE → OUTPUT chain rendered in the
  neumorphic deck. The ENGINE stage collapses into the bit-perfect seal — a ring
  that lights orange with a checkmark when the signal is untouched, and shows a
  warning glyph plus a label (RESAMPLED / EQ / VOLUME) for any deviation.
- Connector between SOURCE and OUTPUT is an engraved channel carved into the
  recessed panel, mask-faded at both ends.
- FORMAT badge in `DeckView.tsx` reflects the same logic: BIT-PERFECT when the
  bypass is active; PROCESSED otherwise.

#### Mini player **[needs the maintainer's eyes]**
- `MiniWindow.tsx`: a compact always-on-top window that polls Rust state directly
  (`engine_status` / `engine_now_playing`) at 300 ms. Avoids the JS-timer-throttle
  macOS applies to hidden windows by bypassing the main-window event bridge.
- Play/pause and seek control Rust directly from the mini window; next/prev/expand
  are forwarded to the main window.
- Opened and closed via a TopBar button (`toggleCompact`).

#### Library and queue
- Queue panel (`QueuePanel.tsx`): drag-to-reorder rows, Clear button, Remove
  per-row.
- Play-Next and Add-to-Queue actions on the album detail view; backed by
  `addToQueue` / `playNext` in `usePlayerStore`.
- Library sort: Albums sortable by Artist, Title, or Year (`useUiStore.librarySort`).

#### Keyboard shortcuts
- `App.tsx` global key handler: Space/K play-pause · ← / → seek ±5 s · J / L
  seek ±10 s · ↑ / ↓ volume · N / B next / prev · Shift+← / → prev / next · M mute.
- Shortcuts are suppressed when focus is inside the search input.

#### UI and design system
- Neumorphic Braun-inspired design system in `src/player/neu.css`: dual soft
  shadows, `--out*` / `--in*` / `--bevel` tokens, `--ink*` text scale, accent
  orange `#ef6a1e`.
- Light theme (Porcelain) and dark theme (Graphite) via `[data-theme="dark"]`.
- macOS overlay titlebar (`titleBarStyle: "Overlay"`) with custom traffic-light
  header; `setDecorations` is never toggled at runtime (destroys the overlay).
- Window `minHeight` raised from 640 to 700 px to accommodate the signal-path row;
  the spectrum panel shrinks first (min 80 px) and the deck scrolls as a safety net.
- Mini-player button: four-corners SVG icon. Theme toggle: stroked crescent moon
  matching the sun glyph weight.
- IPC throttling on seek and volume drags (~20 invokes/sec) to prevent
  WKWebView bridge saturation.

#### Metadata and cover art
- `src-tauri/src/metadata.rs`: lofty-based tag reader (title, artist, album, year,
  track number). Embedded cover art extracted and served via a `stream://` custom
  protocol proxy; sidecar art (`cover.jpg` / `folder.jpg`) used as fallback.

#### Source support
- Local file playback (all formats symphonia decodes: FLAC, ALAC, MP3, AAC, WAV,
  AIFF, OGG, Opus).
- Navidrome / Subsonic streaming: album browse, track list, cover art proxy, server
  URL playback through the native engine.

### Changed
- Web Audio API removed entirely; the Rust engine is the sole audio path.

### Fixed
- ReplayGain (and any future per-track metadata) was dropped on the local-library path: the
  local scanner (`useLocal.ts`) had its own `ScannedTrack`/`toTrack` that didn't copy the new
  RG fields, so `engine_set_replaygain` always received no gain. Both track builders
  (`loader.ts` and `useLocal.ts`) now carry the ReplayGain fields through. (Caught by ear.)

### Deferred (not built yet)
- **In-app update detection + auto-update** — check GitHub Releases for a newer
  version on launch and surface an "Update available" prompt; ideally a one-click
  background update via the Tauri updater (`tauri-plugin-updater` + signed release
  artifacts). Scaffolding notes already in `docs/RELEASE.md`. Not built yet.
- **macOS exclusive (hog) mode** — cpal does not expose this cleanly; deferred to
  Tier 3.
- **iOS / iPad** — toolchain confirmed ready; parked until macOS build has traction
  (see ROADMAP for resume triggers).

[Unreleased]: https://github.com/reactivepixels/eko/compare/HEAD
