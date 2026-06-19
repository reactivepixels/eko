# Architecture Decision Records

This directory contains EKO's Architecture Decision Records (ADRs). Each record
documents a significant technical or product decision: the context that made it
necessary, what was decided, and what the consequences are.

ADRs are written after the decision is made and the outcome is understood. They
are not a planning tool; they are a record of why things are the way they are.

## Index

| ADR | Title | Status |
|---|---|---|
| [0001](0001-one-native-rust-audio-engine.md) | One native Rust audio engine (retire Web Audio) | Accepted |
| [0002](0002-coreaudio-nominal-rate-matching.md) | CoreAudio HAL nominal sample-rate matching for true bit-perfect on macOS | Accepted |
| [0003](0003-mit-license-with-brand-notice.md) | MIT license with a brand NOTICE file | Accepted |
| [0004](0004-bespoke-static-docs.md) | Docs site: bespoke static HTML (Starlight tried, then dropped) | Accepted |
| [0005](0005-tauri-2-over-electron-or-native.md) | Tauri 2 (Rust core + web UI) over native Swift or Electron | Accepted |

## Template

```markdown
# ADR NNNN — Title

**Status:** Proposed | Accepted | Deprecated | Superseded by [NNNN](NNNN-…)

## Context

What situation made this decision necessary?

## Decision

What was decided?

## Consequences

What are the results — positive and negative?
```
