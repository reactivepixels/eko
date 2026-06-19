# EKO Homebrew Cask — TEMPLATE
#
# This file is a per-release template, NOT a live tap formula.
# It lives in the EKO source repo for reference; the actual published
# formula goes in a separate tap repository:
#
#   github.com/reactivepixels/homebrew-eko   (create this repo)
#   └── Casks/
#       └── eko.rb                       (the live, up-to-date formula)
#
# Users install via:
#   brew tap reactivepixels/eko
#   brew install --cask eko
#
# ── Per-release update process ─────────────────────────────────────────────
#
# After each GitHub Release is published:
#
# 1. Download the DMG from the release:
#    curl -L -o /tmp/eko.dmg \
#      https://github.com/reactivepixels/eko/releases/download/vX.Y.Z/EKO_X.Y.Z_x64.dmg
#
# 2. Compute the SHA-256:
#    shasum -a 256 /tmp/eko.dmg
#
# 3. In the tap repo (homebrew-eko), update the three placeholders below:
#    - version  → X.Y.Z  (without the "v" prefix)
#    - sha256   → the hash from step 2
#    - url      → the exact release asset URL from step 1
#
# 4. Commit and push to the tap repo. Homebrew picks up the change
#    the next time a user runs `brew update`.
#
# ── DMG filename note ──────────────────────────────────────────────────────
#
# tauri-action produces the DMG with a filename like:
#   EKO_0.1.0_aarch64.dmg   (Apple Silicon runner)
#   EKO_0.1.0_x64.dmg       (Intel runner)
# If you add a universal build (--target universal-apple-darwin) the filename
# will be EKO_0.1.0_universal.dmg. Update the url and sha256 accordingly.
#
# ── Tap repo bootstrap ─────────────────────────────────────────────────────
#
# Create github.com/reactivepixels/homebrew-eko with:
#   mkdir -p Casks && cp /path/to/this/eko.rb Casks/eko.rb
#   git add . && git commit -m "chore: add eko cask" && git push
#
# Test locally before publishing:
#   brew install --cask --build-from-source ./Casks/eko.rb
# ---------------------------------------------------------------------------

cask "eko" do
  version "PLACEHOLDER"  # e.g. 0.1.0
  sha256 "PLACEHOLDER"   # SHA-256 of the DMG (shasum -a 256 EKO_*.dmg)

  url "https://github.com/reactivepixels/eko/releases/download/v#{version}/EKO_#{version}_aarch64.dmg"
  name "EKO"
  desc "Bit-perfect audiophile music player for macOS"
  homepage "https://github.com/reactivepixels/eko"

  # Require macOS 12 Monterey or later (Tauri 2 minimum).
  depends_on macos: ">= :monterey"

  app "EKO.app"

  # Zap removes user data on `brew uninstall --zap`.
  # Adjust paths once the app's actual data directories are known.
  zap trash: [
    "~/Library/Application Support/com.reactivepixels.eko",
    "~/Library/Caches/com.reactivepixels.eko",
    "~/Library/Preferences/com.reactivepixels.eko.plist",
    "~/Library/Saved Application State/com.reactivepixels.eko.savedState",
    "~/Library/WebKit/com.reactivepixels.eko",
  ]
end
