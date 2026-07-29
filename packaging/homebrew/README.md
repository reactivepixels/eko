# Homebrew cask

The live cask is **not** in this repo. It lives in the tap:

> **[github.com/reactivepixels/homebrew-eko](https://github.com/reactivepixels/homebrew-eko)** → `Casks/eko.rb`

Users install with:

```sh
brew tap reactivepixels/eko
brew install --cask eko
```

## It is bumped automatically — don't hand-edit it

`.github/workflows/release.yml` has a **Bump the Homebrew cask** step that runs on every release
tag: it computes the shipped DMG's sha256, rewrites `version` + `sha256` in the tap, and pushes.

It **fails the workflow loudly** if it can't push, because a silently-skipped bump is exactly the
drift it exists to prevent — the cask had fallen 4 releases behind (0.4.31 vs 0.4.35) while
`README.md` was telling users to `brew install`, so anyone taking that path installed a stale
build.

Requires `PUBLIC_REPO_TOKEN` to carry `Contents:write` on **both** `reactivepixels/eko` and
`reactivepixels/homebrew-eko`.

> A hand-maintained template `eko.rb` used to sit here. It was removed: it documented the old
> manual process, still referenced the per-arch `EKO_x.y.z_aarch64.dmg` / `_x64.dmg` filenames
> (we ship one `_universal.dmg`), and carried a stale `desc`. Two sources of truth for one cask
> is how it drifted in the first place.

## ⚠️ Open question: minimum macOS version

Three sources disagree and it has never been resolved:

| Source | Claims |
|---|---|
| Shipped app (`LSMinimumSystemVersion`, verified on v0.4.35) | **10.13** High Sierra |
| Live cask (`depends_on macos:`) | **:big_sur** (11) |
| `src-tauri/tauri.conf.json` → `bundle.macOS` | nothing set (Tauri default) |

Consequences today: a user on 10.13–10.15 can download and launch the DMG (the app claims to
support them) but `brew install --cask eko` refuses them. And nobody has verified the app actually
*works* on 10.13 — that's the pre-Safari-16.4 WebKit era, which is why `src/lib/roundRectPolyfill.ts`
exists.

**To resolve:** decide the version you actually support and test, then set it explicitly in
`tauri.conf.json` (`bundle.macOS.minimumSystemVersion`) and make the cask's `depends_on` match.
Until then, treat the cask's Big Sur floor as the de-facto answer.
