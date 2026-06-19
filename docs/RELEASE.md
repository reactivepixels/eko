# EKO — Release Process

This document covers the end-to-end process for cutting a signed, notarized EKO release
and publishing it to GitHub Releases. It is written for the project maintainer (the maintainer) and
assumes macOS as the local machine.

---

## Prerequisites

### Apple Developer Program
A paid **Apple Developer Program** membership ($99 USD/yr) is required for code signing
and notarization. Without it, builds are **unsigned** and macOS Gatekeeper will block
users from opening EKO with a warning ("cannot be opened because it is from an
unidentified developer"). You can still ship unsigned releases for testing — just omit
the `APPLE_*` secrets and tauri-action will skip signing.

### Tools
- Node 22 + npm (see `.nvmrc`)
- Rust stable (`rustup update stable`)
- Tauri CLI v2 (`npm run tauri` via the local devDependency)
- A GitHub account with push access and the ability to set repository secrets

---

## Required GitHub Secrets

Set these in **GitHub → repo → Settings → Secrets and variables → Actions → New repository secret**.
Never commit these values to the repository.

| Secret name | What it is | How to get it |
|---|---|---|
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12` certificate | See below |
| `APPLE_CERTIFICATE_PASSWORD` | Password you set when exporting the `.p12` | Same export step |
| `APPLE_SIGNING_IDENTITY` | Common Name of the cert, e.g. `Developer ID Application: Reactive Pixels (XXXXXXXXXX)` | Run `security find-identity -v -p codesigning` after importing |
| `APPLE_ID` | Your Apple ID email (the one registered with the Developer Program) | Your Apple account |
| `APPLE_PASSWORD` | An **app-specific password** for that Apple ID | [appleid.apple.com](https://appleid.apple.com) → Sign-in and Security → App-Specific Passwords |
| `APPLE_TEAM_ID` | 10-character team ID | [developer.apple.com/account](https://developer.apple.com/account) → Membership Details |
| `TAURI_SIGNING_PRIVATE_KEY` | PEM private key for Tauri updater signing | See below |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the Tauri key (can be empty) | Same step as above |

`GITHUB_TOKEN` is provided automatically by GitHub Actions — no secret entry needed.

---

### Obtaining the Apple Developer ID certificate

1. Open **Xcode → Settings → Accounts**, select your Apple ID, then click **Manage Certificates**.
   Click **+** and choose **Developer ID Application**. Xcode will request and install the cert.
   Alternatively, create and download a `.cer` from the [Certificates portal](https://developer.apple.com/account/resources/certificates/list)
   and double-click to import into Keychain.

2. In **Keychain Access**, find the certificate (under "My Certificates" — it will have a private
   key nested under it). Right-click → **Export** → choose `.p12` format → set a strong password.
   This password becomes `APPLE_CERTIFICATE_PASSWORD`.

3. Convert the `.p12` to base64 and copy to clipboard:
   ```sh
   base64 -i ~/Downloads/DeveloperID.p12 | pbcopy
   ```
   Paste this as the `APPLE_CERTIFICATE` secret.

4. Find the exact signing identity string:
   ```sh
   security find-identity -v -p codesigning
   ```
   Copy the full string in quotes (e.g. `Developer ID Application: Reactive Pixels (ABCD123456)`).
   This becomes `APPLE_SIGNING_IDENTITY`.

---

### Generating the Tauri updater keypair

The Tauri auto-updater uses a keypair to verify that update artifacts haven't been tampered with.
The **public key** goes in `tauri.conf.json` (committed to the repo). The **private key** is
a secret used only in CI to sign artifacts.

1. Generate the keypair:
   ```sh
   npm run tauri signer generate
   ```
   This prints a public key and a private key. Optionally set a password when prompted.

2. Add the public key to `src-tauri/tauri.conf.json` — see the
   [Updater plugin config block](#updater-plugin-configuration) section below.

3. Add the private key (the full PEM block including `-----BEGIN...` headers) as
   `TAURI_SIGNING_PRIVATE_KEY`. Add the password (or an empty string) as
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

> The private key signs `.app.tar.gz` update artifacts. Anyone with it can ship a
> malicious update. Treat it like a deploy key — never commit it.

---

## Updater Plugin Configuration

The Tauri auto-updater requires two things:
1. The **`tauri-plugin-updater`** Rust crate added to `src-tauri/Cargo.toml` and registered
   in `src-tauri/src/lib.rs`.
2. A **`plugins.updater`** config block in `src-tauri/tauri.conf.json`.

Because installing the plugin requires runtime code changes (not just config), the block is
documented here rather than committed prematurely. Add it when you are ready to wire up
the plugin in Rust.

**Step 1 — Add the Cargo dependency** (`src-tauri/Cargo.toml`):
```toml
tauri-plugin-updater = "2"
```

**Step 2 — Register the plugin** (`src-tauri/src/lib.rs`, inside `tauri::Builder`):
```rust
.plugin(tauri_plugin_updater::Builder::new().build())
```

**Step 3 — Add the config block** (`src-tauri/tauri.conf.json`, at the top level):
```json
"plugins": {
  "updater": {
    "pubkey": "<paste the public key from npm run tauri signer generate>",
    "endpoints": [
      "https://github.com/reactivepixels/eko/releases/latest/download/latest.json"
    ]
  }
}
```

Replace `reactivepixels/eko` with the actual `owner/repo` slug once the repository is public.

The `latest.json` file is produced automatically by `tauri-action` when it builds the
`.app.tar.gz` update artifact — you do not need to create it manually.

---

## How Signing and Notarization Work

`tauri-action` (used in `.github/workflows/release.yml`) handles the entire Apple flow:

1. **Import certificate** — the action imports `APPLE_CERTIFICATE` into a temporary macOS keychain
   on the CI runner. The private key is unlocked using `APPLE_CERTIFICATE_PASSWORD`.

2. **Code sign** — `tauri build` calls `codesign` with `--deep --force --verify` using the
   identity from `APPLE_SIGNING_IDENTITY`. All binaries and frameworks inside the `.app` bundle
   are signed.

3. **Notarize** — the action submits the signed DMG to Apple's notary service using `notarytool`
   (the successor to `altool`) via `APPLE_ID`, `APPLE_PASSWORD`, and `APPLE_TEAM_ID`.
   Apple scans the binary for malware and returns a ticket (typically within 1–5 minutes).

4. **Staple** — `xcrun stapler staple` attaches the notarization ticket to the DMG so it can be
   verified offline. Users can open EKO without an internet connection and without seeing a
   Gatekeeper warning.

5. **GitHub Release** — the action calls the GitHub Releases API with `GITHUB_TOKEN` to create
   a draft release tagged with the pushed tag, attaching the DMG and the `.app.tar.gz` updater
   artifact.

---

## Cutting a Release (step-by-step)

### 1. Bump the version in two places

`src-tauri/tauri.conf.json`:
```json
"version": "0.2.0"
```

`package.json`:
```json
"version": "0.2.0"
```

Both must match. The tauri-action reads the version from `tauri.conf.json`.

### 2. Update CHANGELOG.md

Follow [Keep a Changelog](https://keepachangelog.com) format. Move items from `[Unreleased]`
to a new `[0.2.0] - YYYY-MM-DD` section. Commit the changelog alongside the version bump.

### 3. Commit and tag

```sh
git add src-tauri/tauri.conf.json package.json CHANGELOG.md
git commit -m "chore: release v0.2.0"
git tag v0.2.0
git push origin main --follow-tags
```

Pushing the tag triggers the release workflow. Watch the Actions tab for progress.

### 4. Review and publish the draft release

The workflow creates the release as a **draft**. Once the artifacts are attached:

1. Open **GitHub → Releases → Drafts**.
2. Verify the DMG is attached and the changelog body looks correct.
3. Click **Publish release**.

### 5. Verify notarization locally (optional)

After downloading the published DMG:
```sh
spctl --assess --verbose=4 /Volumes/EKO/EKO.app
# Expected: /Volumes/EKO/EKO.app: accepted
# source=Notarized Developer ID
```

---

## Universal Binary (arm64 + x86_64)

The current workflow builds for the runner's native architecture (`arm64` on `macos-latest`).
To produce a single DMG that runs on both Apple Silicon and Intel Macs:

1. Add the cross-compilation target in the workflow before the tauri-action step:
   ```yaml
   - name: Add universal targets
     run: |
       rustup target add aarch64-apple-darwin
       rustup target add x86_64-apple-darwin
   ```

2. Pass `--target universal-apple-darwin` to the action:
   ```yaml
   with:
     args: --target universal-apple-darwin
   ```

Note: universal builds roughly double the Rust compile time. Confirm the native build works
and signing is set up correctly before enabling universal.

---

## Unsigned / Pre-signing Releases

Before Apple Developer signing is configured, you can still use the workflow — just leave
the `APPLE_*` secrets unset. tauri-action will build and attach an unsigned DMG. Users
will need to right-click → Open the first time to bypass Gatekeeper, or run:
```sh
xattr -dr com.apple.quarantine /Applications/EKO.app
```

Document this clearly in the release notes for any pre-signing releases.

## Deploying the website (`site/`)

The docs + marketing site is **plain static HTML** in `site/` (no build step). It ships to
**`eko.reactivepixels.com`** on Vercel.

**One-time setup:**
1. Vercel → **Add New → Project** → import `reactivepixels/eko`.
2. **Root Directory:** `site` · **Framework Preset:** Other · **Build Command:** none (leave
   empty) · **Output Directory:** leave default (serves `site/` as-is).
3. **Domains** → add `eko.reactivepixels.com`; add the CNAME Vercel shows to your DNS.

`site/vercel.json` sets clean URLs (`/docs`, `/web-player`) and basic security headers. It does
**not** set `X-Frame-Options` — the web player (`/web-player`) is meant to be embeddable on other
sites, which is a feature.

**Every push to `main`** then redeploys automatically; PRs get preview URLs.

**CLI alternative:** `cd site && vercel --prod` (after `vercel login` + `vercel link`).

> Future: the Tauri auto-updater's `latest.json` endpoint and the `@rpxl/eko` web-player package
> can live on the same Vercel project — another reason the site is here rather than GitHub Pages.
