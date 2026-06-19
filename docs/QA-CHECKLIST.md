# EKO — manual QA checklist

Automated tests can't hear, can't watch the OS, and can't judge feel. This is the human pass —
run it before every release, and use it to verify features marked "needs ears" in the ROADMAP.
Test on **both** built-in speakers and an **external USB DAC** where it matters.

## Bit-perfect & device rate (the core promise) ⚠️ highest priority
- [ ] Play a **44.1 kHz** track → Audio MIDI Setup shows the device at **44.1 kHz**; seal = **BIT-PERFECT**.
- [ ] Switch to a **48 kHz** / **96 kHz** track → the device rate **follows automatically**; seal stays lit.
- [ ] Pick an output device that *can't* do the file rate → seal honestly drops to **RESAMPLED**.
- [ ] Engage the EQ (non-flat) → seal shows **EQ**; flat again → back to BIT-PERFECT.
- [ ] Pull volume below 100% → seal shows **VOLUME**; unity again → BIT-PERFECT.
- [ ] A/B a track vs Apple Music lossless → EKO is at least as good (ideally better).

## Playback & transport
- [ ] Local file playback starts promptly (no long delay), correct pitch.
- [ ] Navidrome/Subsonic streaming plays bit-perfect through the engine.
- [ ] Seek bar scrubs smoothly (no stutter / no jump-to-cursor).
- [ ] Volume dial turns smoothly; scroll-wheel works; no system-volume hijack.
- [ ] Next / prev / shuffle / repeat behave correctly.

## Output device picker
- [ ] Lists real devices; selecting one re-arms current track on the new device.
- [ ] Choice persists across relaunch.

## Mini player
- [ ] Opens; reflects the actual track/state; play/pause/seek work.
- [ ] Stays live when the main window is hidden/minimised.

## EQ
- [ ] Faders + presets audibly shape the sound (local and server).
- [ ] Flat = truly bypassed (bit-perfect preserved).

## ReplayGain ⚠️ new — built but unverified
- [ ] RG = **Off** → seal reads BIT-PERFECT on a clean track (no change vs before).
- [ ] RG = **Track** on a tagged loud master vs a quiet master → perceived loudness evens out.
- [ ] RG = **Album** → uses album gain; consistent level across one album's tracks.
- [ ] A track with a high peak + positive gain does **not** clip (peak limiting works).
- [ ] When RG is applied, the seal shows **REPLAYGAIN** (honest); Off → back to BIT-PERFECT.
- [ ] An untagged file with RG on → plays normally, stays bit-perfect (no gain applied).

## Gapless ⚠️ new — built but unverified
- [ ] Play a **same-rate album** (e.g. all 44.1/16) from a track; let it run to the end of a
      track **without touching anything** → **no gap/click** at the boundary into the next track.
- [ ] At the boundary the **track title/art advance**, the scrub bar resets to the new track, and
      the seal stays correct (BIT-PERFECT if nothing's processing).
- [ ] Seek within a track stays within that track (doesn't jump into the next/prev).
- [ ] Manual next/prev still works (and is allowed to have the normal tiny gap).
- [ ] A track at a **different sample rate** still plays (small gap as the stream rebuilds — OK).
- [ ] Shuffle / repeat-one → no gapless arming (each track starts fresh, as expected).

## Resume last session ⚠️ new — built but unverified
- [ ] Play a **local** album mid-track, quit, relaunch → same track shown, **paused**, at the
      right position; press play → resumes from there (within ~1s).
- [ ] Relaunch does **not** auto-play.
- [ ] Picking a different track after restore starts that track from 0 (resume only applies once).
- [ ] Server (Navidrome) sessions are not restored (expected, by design for now).
- [ ] Hand-corrupt `localStorage` `eko.state.v1` → app still launches cleanly (fresh session).

## Library & queue
- [ ] Albums sort by Artist / Title / Year.
- [ ] Up-Next: drag-reorder, clear, remove; Add-to-Queue / Play-Next from album detail.
- [ ] Local cover art + sidecar art load; long titles scroll on hover.

## Keyboard
- [ ] Space/K play-pause · ←/→ seek · J/L ±10s · ↑/↓ volume · N/B next/prev · M mute.
- [ ] Shortcuts ignored while typing in the search box.

## Visual / theme
- [ ] Light (Porcelain) and dark (Graphite) both correct — no bright halos, good contrast.
- [ ] Window resize keeps the EQ + deck visible (no clipping).
- [ ] macOS overlay titlebar (traffic lights) intact.

## Notes
Record anything off here → file an issue with source format + output device + seal state.
