# ADR 0002 — CoreAudio HAL nominal sample-rate matching for true bit-perfect on macOS

**Status:** Accepted

## Context

Even with a native Rust audio engine (ADR 0001), one gap remains: cpal opens the
output stream at the file's sample rate, but CoreAudio's HAL will silently
resample the PCM if the output device is currently running at a different rate.
For example, if the device is at 48 kHz and EKO plays a 44.1 kHz file, the OS
mixer interpolates from 44.1 to 48 kHz before the samples reach the DAC. The
samples EKO sends are not the samples the DAC receives.

This is the behaviour of every player that uses cpal or the system audio API
without explicitly managing the device rate — including, historically, Apple Music
itself (which EKO benchmarks against). The OS resample is not inaudible on a
resolving DAC; the maintainer's A/B test (EKO vs Apple Music lossless on the same track,
2026-06-19) confirms the difference is audible.

macOS exposes `kAudioDevicePropertyNominalSampleRate` via the CoreAudio HAL API.
Setting this property changes the rate at which the output device's clock
actually runs. Roon and Audirvana both set this property; it is the mechanism
behind their "exclusive mode" / "integer mode" claims.

Tauri does not expose this property and cpal does not either. It requires a
hand-written FFI layer.

## Decision

Write `src-tauri/src/coreaudio.rs` — a macOS-only module containing:

1. A function to enumerate CoreAudio output devices and find one by name (to
   match the device chosen in the output device picker, ADR 0001).
2. A function to get and set `kAudioDevicePropertyNominalSampleRate` on a device
   using the CoreAudio C API via Rust's `core-foundation` and `core-audio-types`
   crates (or raw `extern "C"` FFI to AudioToolbox).
3. A readback: after setting the rate, read it back and return the actual
   `dev_rate`. If the device rejects the requested rate (e.g. a Bluetooth sink
   that only supports 48 kHz), `dev_rate` will differ from `src_rate` and the
   engine will surface this via `EngineStatus`.

This function is called before cpal opens the output stream. The stream is then
opened at `src_rate`; because the device is already running at `src_rate`, no OS
resample occurs.

`EngineStatus` exposes `dev_rate` alongside `src_rate`. The bit-perfect seal in
`SignalPath.tsx` uses the comparison `dev_rate == src_rate` as one of its three
conditions. If they differ, the seal shows RESAMPLED.

EKO owns the device rate while a stream is active and does not restore it on
close (consistent with Roon/Audirvana behaviour).

This module is compiled only on macOS (`#[cfg(target_os = "macos")]`). The iOS
port (parked) will require a different approach via `AVAudioSession`.

## Consequences

**Positive:**
- True bit-perfect output on macOS: the samples EKO sends are the samples the
  DAC receives.
- The bit-perfect claim becomes falsifiable: if `dev_rate != src_rate` the seal
  honestly says RESAMPLED, never BIT-PERFECT.
- EKO achieves the same result as Roon/Audirvana without requiring the user to
  manually set Audio MIDI Setup before each listening session.

**Negative / trade-offs:**
- Hand-written CoreAudio FFI is macOS-specific and bypasses any future cpal
  abstraction for device-rate control.
- EKO leaves the device at whatever rate the last track used after stopping.
  Applications that open the device next will inherit that rate. Restoring on
  close was considered and rejected: detecting all termination paths
  (normal stop, crash, force-quit) reliably enough to guarantee a restore is
  disproportionate effort.
- This feature needs verification by ear and via Audio MIDI Setup inspection;
  it is tagged [needs ear-verify] in the ROADMAP until the maintainer has confirmed it.
- No equivalent on iOS — the iOS port cannot offer the same bit-perfect
  guarantee without a platform-specific alternative.
