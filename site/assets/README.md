# EKO Web — audio assets

The web player (`../web-player.html`) ships with **`eko-loop.flac`** (lossless, browser-decoded)
in this folder. If the file is absent it generates an ambient bed via Web Audio, so the demo
always works.

## Replace the signature loop
1. Generate a short abstract/ambient piece (see prompts below).
2. Export **FLAC or WAV, 44.1 kHz**, trim to ~30–45 s, ideally a seamless loop.
3. Save it here as **`eko-loop.flac`** (or pass any URL: `web-player.html?src=…`).

## Generate it — fal.ai (the maintainer has a subscription)
- **`fal-ai/stable-audio`** — best for abstract/ambient/hi-fi. Top pick.
- alternates: `fal-ai/ace-step` (open, fast instrumental), `fal-ai/cassetteai/*` (fast variations).
- UI/brand stinger: `fal-ai/elevenlabs/sound-effects`.

### Prompts (EKO aesthetic — warm, minimal, Braun restraint)
1. **Signature loop** — `Minimal Braun-inspired ambient, one restrained warm analog pad slowly morphing, sustained chord, soft tape hiss, refined and elegant, audiophile hi-fi, wide stereo, seamless loop, no drums, no vocals, 60 BPM`
2. **Graphite (dark)** — `Deep dark ambient, low warm sub-bass drone, granular textures, distant metallic resonances, slow and brooding, analog warmth, wide stereo, cinematic, hi-fi, no drums, no vocals`
3. **Porcelain (light)** — `Airy bright ambient, shimmering glassy bells, soft Rhodes chords, light granular shimmer, warm and calm, clean hi-fi production, gentle stereo movement, no drums, no vocals, 70 BPM`
4. **Abstract/textural** — `Abstract sound design, evolving granular textures, filtered noise washes, subtle modular bleeps, organic and tactile, immersive stereo, high fidelity, no melody, no drums, no vocals`
5. **Audiophile showcase** — `Lush hi-fi ambient, deep resonant piano with long natural decay, warm string-pad swells, wide dynamic range, pristine spacious hall reverb, intimate and emotive, no drums, no vocals`
6. **UI stinger** (ElevenLabs SFX) — `Short warm analog chime, single soft bell with gentle reverb tail, refined minimal, hi-fi` (trim ~2–3 s)

## Licensing (important for open source)
Record, per asset, the **model + prompt + date + license terms** of whatever you ship.
fal-hosted Stable Audio output is usable commercially under fal's + Stable Audio's terms;
keep this note current so the OSS repo is clean. For "real hi-res to demo bit-perfect"
use freely-licensed hi-res (2L.no, Linn free downloads) — not AI audio.

| file | model | prompt # | license | date |
|------|-------|----------|---------|------|
| eko-loop.flac | _record before release_ | _tbd_ | _verify fal/Stable Audio terms_ | _tbd_ |
| cover-light.webp | fal.ai | _record_ | _verify_ | _tbd_ |
| cover-dark.webp | fal.ai | _record_ | _verify_ | _tbd_ |
