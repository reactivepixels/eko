# EKO — Roadmap to best-in-class

Benchmark: **Roon / Audirvana** for audiophile depth, **Apple Music** for polish.
Principle: go hard on high-value things EKO can win at (audio fidelity + design +
the things commodity players skip). Don't sink time into low-value-hard work.

## Foundations (DONE)
- Native Rust audio engine: symphonia decode → cpal at the file's own sample rate.
- Bit-perfect bypass path + honest BIT-PERFECT badge (lit only at unity gain + flat EQ).
- Local + server (Navidrome) both routed through the one engine.
- Real 10-band biquad EQ in the engine (bypassed when flat).
- Rust FFT spectrum. Software volume (square-law, unity = bit-perfect). Smooth seek/dial.

## Tier 1 — high value, in EKO's wheelhouse (DOING TONIGHT)
1. **Mini player (S4)** — compact always-on-top window reading Rust engine state directly.
2. **Signal path display** — Roon's signature feature. Show FILE → engine → DAC with
   sample rate / bit depth / "bit-perfect" or "EQ active". Low effort, high audiophile wow.
3. **Output device selection** — pick the DAC explicitly (cpal device enumeration).
   Core audiophile feature; Apple Music can't do it.
4. **Gapless playback** — decode the next track ahead and feed the stream with no gap.
   High value for album listening.
5. **Library depth** — sort (name/artist/year/recently added), richer browse, better search.
6. **Queue management** — reorder, play-next, add-to-queue, clear.
7. **Keyboard shortcuts + macOS media keys / Now Playing.**

## Tier 2 — valuable, do if time
8. **ReplayGain / volume normalization** — read RG tags (lofty), apply as engine gain.
9. **Crossfade** (overlap two decode streams).
10. **Resume last session** (track + position) on launch.
11. **Album/track context menus.**

## Tier 3 — noted, lower priority / hard
- DSD / MQA, cue sheets, upsampling/room-correction DSP, macOS hog (exclusive) mode.
  (Hog mode is the one genuinely-hard audiophile gap; cpal doesn't expose it cleanly.)

## Status log
- (start) Tier 1 kicked off.
- ✅ **S4 Mini player** — rebuilt to read engine state directly from Rust (engine now
  holds NowPlaying meta: title/artist/cover/theme/index/total via engine_set_now_playing
  / engine_now_playing). Mini polls Rust @300ms, drives pause/resume/seek direct, sends
  only next/prev/expand to main. Removed the throttled main→mini event push. Opens via a
  TopBar button (toggleCompact). Builds clean, running. NEEDS the maintainer's eyes in the morning.
- ✅ **Signal-path display** (Roon-signature) — engine exposes src rate / bit depth / codec
  / device + resample detection. DeckView shows SOURCE → ENGINE → OUTPUT chain w/ honest
  pure-path dot; FORMAT badge now honest (BIT-PERFECT vs PROCESSED). `SignalPath.tsx`.
- ✅ **Output device selection** — engine enumerates DACs (engine_list_devices) + targets a
  chosen one (engine_set_device, device_pref). Picker on the OUTPUT node; persisted;
  re-arms current track on switch. Applies per-track.
- ✅ **Keyboard shortcuts** — space/k play-pause, ←/→ seek 5s, j/l seek 10s, ↑/↓ volume,
  n/b next/prev, shift+←/→ prev/next, m mute. Ignores the search box. `App.tsx`.
- ✅ **Queue management** — drag-to-reorder rows + Clear button in the Up-Next panel;
  Add-to-Queue / Play-Next buttons on album detail (store: addToQueue, playNext).
- ✅ **Library sort** — Albums sortable by Artist / Title / Year (useUiStore.librarySort).
- ✅ **Signal path = Concept G** (chosen from an A–G option set, since removed) —
  SOURCE → OUTPUT chain + a dedicated **bit-perfect "seal"** (ring that lights orange w/
  checkmark when untouched, warning glyph + RESAMPLED/EQ/VOLUME when not). The ENGINE stage
  collapses into the seal. Connector = an **engraved channel** carved into the recessed panel,
  ends mask-faded so it reads as a conduit, not a slider (iterated: dots/slider/arrow all
  rejected). `SignalPath.tsx` + `.sigpath` CSS in `neu.css`.
- ✅ **TRUE bit-perfect on macOS (CoreAudio)** — the big one. EKO now sets the OS output
  device's nominal sample rate to match the file (`src-tauri/src/coreaudio.rs`, macOS-only,
  hand-written CoreAudio HAL FFI: enumerate devices, find by name, get/set
  kAudioDevicePropertyNominalSampleRate). So a 44.1 file flips the device to 44.1, 96k → 96k
  — no silent macOS resample (what Roon/Audirvana do). Engine reads the device's ACTUAL rate
  back (`dev_rate`) so the seal is honest: shows RESAMPLED if the device can't match. Closes
  the one real hole in the bit-perfect claim. EKO now owns the device rate while playing.
  ⚠️ NEEDS THE MAINTAINER TO VERIFY: play different-rate tracks, watch Audio MIDI Setup follow + seal stay lit.
- ✅ **EQ-clipped-on-resize fix** — the signal path added height; bumped window minHeight
  640→700, deck shrinks spectrum first (min 80px) + scrolls as a safety net.
- ✅ **Icons** — mini-player button = four-corners SVG (centered); moon = stroked crescent
  R6.4 (matches the sun's weight).

### Done in the build sessions, verified by build + logic. NEEDS THE MAINTAINER'S EYES/EARS:
mini player, signal path (Concept G) + device picker, **CoreAudio device-rate switching
(ear/Audio-MIDI verify)**, keyboard shortcuts, queue reorder, library sort.

### The site (plain static HTML, ships as-is — this IS the docs/marketing site):
`site/index.html` (landing) · `site/docs.html` · `site/web-player.html` ("EKO Web Lite") ·
`site/mobile.html` (iOS concept, parked) · `site/assets/` (covers, demo loop). Bespoke,
built directly on the app's neumorphic design system — Starlight was tried and dropped (a
framework couldn't match the design without compromise). `concepts/` now holds only design
scratch (icon explorations). Deploy target: **eko.reactivepixels.com** (Vercel).

### 🅿️ PARKED — iOS / iPad app (banked future milestone)
Decided 2026-06-19: do NOT build now. It's a separate project (AVAudioSession audio path —
the CoreAudio HAL rate-switching + DAC picker are macOS-only; sandboxed files → Navidrome
streaming is the iOS story; full touch redesign). Premature before the macOS player has
traction. **Toolchain confirmed ready** (Xcode 26.2, Tauri CLI 2.11 — just needs
`rustup target add` iOS targets + `tauri ios init`). **Design mocked + cleaned** at
`site/mobile.html` (bottom tab bar · full-screen Now Playing w/ inline signal-path row ·
EQ as a bottom sheet). **Revisit triggers:** (1) macOS EKO gets real traction, or (2) the maintainer
wants it on his own iPad/iPhone. First build when resumed = UI + Subsonic streaming in the
simulator, audio through the default session, macOS-only engine bits cfg'd out.

### Deferred — high value but need the maintainer awake to ear-test / carry startup risk:
- **Gapless** (capstone): design in `architecture/gapless.md`. Same-rate append into the running
  cpal stream = truly gapless AND bit-perfect for same-rate albums (the common case);
  fall back to per-track restart when the next track's rate differs (preserves
  bit-perfect). Real engine refactor + can't ear-test alone → build it together.
- **ReplayGain** — read RG tags (lofty) → apply as engine gain (off = bit-perfect).
- **Resume last session** — persist track id + position + queue; restore on launch.
  Carries startup-regression risk; do it with the maintainer watching.
- **macOS media keys / Now Playing** (MPNowPlayingInfoCenter) — needs native/plugin work.
</content>
