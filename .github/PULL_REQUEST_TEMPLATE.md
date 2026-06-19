<!-- Thanks for contributing to EKO! Keep PRs small and focused. -->

## What & why
<!-- What does this change, and why? Link any related issue (#123). -->

## How I verified it
<!-- Build/typecheck/tests, and for audio-path changes: how you EAR-tested it + on what output. -->

## Checklist
- [ ] `cargo fmt` + `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo test` passes
- [ ] `npx tsc --noEmit` + `npm run lint` clean
- [ ] Bit-perfect path preserved (unity volume + flat EQ + matched rate = untouched samples)
- [ ] Signal-path indicator stays honest
- [ ] UI matches the EKO design language (if applicable)
- [ ] Docs / CHANGELOG updated (if applicable)
