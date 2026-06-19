# ReplayGain — design & remaining work

**Status:** engine + metadata + tests **done** (off by default); frontend wiring + ear-verify
remaining. This is the spec for finishing it.

## What's done (engine side)

- `metadata.rs` reads `REPLAYGAIN_TRACK_GAIN` / `REPLAYGAIN_ALBUM_GAIN` (dB) and
  `REPLAYGAIN_TRACK_PEAK` / `REPLAYGAIN_ALBUM_PEAK` (linear) into `TrackMetadata`
  (`rgTrackGain`, `rgAlbumGain`, `rgTrackPeak`, `rgAlbumPeak`). Tolerant parser
  `parse_rg_gain_db` handles `"-6.34 dB"`, `"+3 dB"`, `"-6.34"`, any case.
- `engine_set_replaygain(gainDb: Option<f32>)` stores a linear multiplier
  (`10^(dB/20)`, clamped `0..=4`) in `Shared.rg_gain` (lock-free `AtomicU32`, default
  unity). It's folded into the volume multiply in the cpal callback.
- **Bit-perfect is preserved:** the bypass predicate is now `is_bitperfect(eq_active,
  vol, rg_gain)` — bypass only when EQ flat AND volume unity AND `rg_gain == 1.0`.
  Unit-tested. Default `rg_gain == 1.0` ⇒ default runtime behaviour is unchanged.
- `nativeEngine.setReplayGain(gainDb | null)` wrapper exists.

## What remains (frontend — do with the maintainer, ear-verify)

1. **Setting** in `usePlayerStore` (or `useUiStore`): `replayGainMode: "off" | "track" | "album"`,
   default `"off"`, persisted (add to `persist.ts`).
2. **Apply on track load.** Where a track starts (`playAt` / the native play path), compute
   the gain and call `nativeEngine.setReplayGain(...)`:
   - `off` → `setReplayGain(null)`.
   - `track` → `rgTrackGain`; `album` → `rgAlbumGain ?? rgTrackGain`.
   - **Peak limiting (clipping guard):** if a positive gain would push the peak over full
     scale, reduce it. With peak `p` (linear) and gain `g` dB: max safe gain is
     `-20·log10(p)` dB; use `min(chosenGainDb, maxSafeDb)`. If no peak tag, optionally apply
     a small headroom (e.g. cap boosts at 0 dB, i.e. only attenuate) — decide with the maintainer.
   - If the track has no RG tags, `setReplayGain(null)` (leave it bit-perfect).
3. **Navidrome:** the Subsonic API can return `replayGain` on `getSong`/`song` objects
   (`trackGain`, `albumGain`, `trackPeak`, `albumPeak`). Map those into the same fields in
   the subsonic client so server tracks normalise too. (Local already carries them.)
4. **UI.** A 3-way control (Off / Track / Album). Likely homes: the signal-path area or a
   small settings popover. Keep it consistent with the neumorphic control vocabulary. When RG
   is active and non-zero, the **seal already shows non-bit-perfect** (the gain ≠ unity), which
   is correct and honest — consider a dedicated `REPLAYGAIN` seal label (optional; engine could
   expose `rgActive` on `EngineStatus` if we want the seal to name it specifically).

## Ear-verification checklist (the maintainer)

- Off = identical to today; seal still reads BIT-PERFECT on a clean track.
- Track mode on a loud master + a quiet master → perceived loudness matches.
- A track with a high peak + positive album gain does **not** clip (peak limiting works).
- Switching Off↔Track flips the seal between BIT-PERFECT and processed.
