# Contributing to EKO

Thanks for being here. EKO is a bit-perfect, beautiful macOS music player, and the bar is
high on purpose — its whole reason to exist is **audio fidelity + design taste**. Contributions
that protect and raise that bar are very welcome.

## The non-negotiables (read these first)
1. **Never break the bit-perfect path.** Playback must stay bit-for-bit when volume is at
   unity, the EQ is flat/off, and the device rate matches the file. The cpal callback has a
   dedicated bypass for this — don't add processing or allocation that defeats it.
2. **Be honest about fidelity.** The signal-path seal must reflect the *actual* state. Never
   show "bit-perfect" when the signal is being resampled, EQ'd, or volume-scaled.
3. **Match the design language.** EKO is a neumorphic, Braun-inspired system (light *Porcelain*
   / dark *Graphite*, one orange accent). New UI must feel native to it — see the design tokens
   in `src/player/neu.css`. For anything visual, **start with a static HTML concept** (see
   `concepts/`) before building.

## Quick start
```bash
# prerequisites: Rust (stable), Node 18+, macOS, Xcode command-line tools
git clone https://github.com/<you>/eko.git && cd eko
npm install
npm run tauri dev      # run with hot reload
npm run tauri build    # produce a .app
npx tsc --noEmit       # typecheck
```
A new contributor should be running EKO in under 10 minutes. If you weren't, that's a docs
bug — please tell us.

## Project layout
The full map (architecture, the audio engine, hard-won gotchas) is the
**[architecture deep-dive](docs/architecture/overview.md)** and the
**[decision records](docs/architecture/adr/)** — read them before deep changes. In short:
`src-tauri/src/engine.rs` is the native audio engine, `src-tauri/src/coreaudio.rs` is macOS
device rate-matching, `src/player/` is the UI, and `src/store/` is the state. Roadmap +
status: **`docs/ROADMAP.md`**.

## Code style & checks
Everything below runs in CI; run it locally before opening a PR:
```bash
cargo fmt --all && cargo clippy --all-targets -- -D warnings
cargo test
npx tsc --noEmit && npm run lint
```
- **Rust:** idiomatic, `rustfmt`-clean, `clippy`-clean. Public items in the engine get doc
  comments. No `unwrap()` on fallible I/O in production paths.
- **TypeScript:** strict, no unused exports, typed props. No leftover dead code.
- **Commits:** clear, present-tense summaries. Conventional Commits (`feat:`, `fix:`, `docs:`)
  appreciated but not required.

## Pull requests
- Keep PRs **small and focused** — one concern each.
- Fill in the PR template; describe what you changed and how you verified it.
- For audio-path changes, say how you **ear-tested** it (and on what output) — automated tests
  can't hear.
- CI (fmt, clippy, tests, typecheck, lint, build) must be green.

## Reporting bugs / proposing features
Use the issue templates. For anything audio-fidelity-related, include your source format,
output device, and what the signal-path seal showed.

## Conduct & security
By participating you agree to the [Code of Conduct](./CODE_OF_CONDUCT.md). For security issues,
**do not** open a public issue — see [SECURITY.md](./SECURITY.md).

Questions and ideas are welcome in **GitHub Discussions**. Thank you for helping make EKO the
best-sounding, best-looking open player there is.
