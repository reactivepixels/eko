# Security Policy

EKO is a local macOS application: no servers, no accounts, no telemetry, no analytics. Your
library, your listening, and your Navidrome/Subsonic credentials never leave your machine.

## Reporting a vulnerability
Please **do not open a public issue** for security problems. Use GitHub's
[**private vulnerability reporting**](https://github.com/reactivepixels/eko/security/advisories/new)
(the repo's Security tab → "Report a vulnerability") and include:
- a description of the issue and its impact,
- steps to reproduce (or a proof of concept),
- the EKO version and your macOS version.

You'll get an acknowledgement within a few days. We'll work with you on a fix and credit you
in the release notes unless you'd prefer to stay anonymous.

## Scope (where to look)
The most relevant surfaces are:
- **Local file handling** — metadata/cover parsing of untrusted audio files.
- **Subsonic/Navidrome credentials** — stored locally; how they're persisted and used.
- **The `stream://` proxy** and any network requests to a user-configured server.
- **Tauri capabilities** (`src-tauri/capabilities/`) — the allowlist of what the webview can do.

## Supported versions
Until v1.0, only the latest release is supported. After v1.0 we'll support the current minor
release line.
