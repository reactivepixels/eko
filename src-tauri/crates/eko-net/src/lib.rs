//! EKO's OpenSubsonic / Navidrome client.
//!
//! Ported from the TypeScript client at `src/subsonic/client.ts` (repo root) so the
//! desktop app and a future terminal client can share one implementation, and so the
//! server password never has to reach the frontend. This crate is FREE — no Pro
//! logic belongs here.
//!
//! **On the `client.ts:NNN` citations throughout this crate:** that file no longer exists
//! at HEAD — this crate replaced it, and it was deleted once the last consumer migrated.
//! The line numbers refer to the pre-deletion file, readable at
//! `git show 1393cf3:src/subsonic/client.ts`. They are kept rather than stripped because
//! they document *why* each behaviour is shaped the way it is, including the quirks
//! preserved deliberately for byte-exact parity.

//! The layering is deliberate and worth keeping: [`urls`] mints URLs, [`parse`] turns
//! response bodies into typed payloads, and both are pure — no I/O, no clock, no
//! randomness they don't receive as an argument. [`client`] is the only module that
//! touches the network, and it is thin by design so the fixture suite can cover the
//! interesting logic without a server.
//!
//! [`lenient`] sits under [`types`] and [`parse`]: it is what keeps a server with loose
//! JSON encoding habits (a string-encoded `year`, a float `duration`, an integer `id`)
//! from failing a whole page, and it carries the reasoning for why that leniency is the
//! correct default here rather than a shortcut.

pub mod auth;
pub mod client;
pub mod lenient;
pub mod parse;
pub mod types;
pub mod urls;

use serde::{Deserialize, Serialize};

pub use client::Client;
pub use parse::SubsonicError;
pub use types::mime_for_song;

/// Connection details for a Subsonic/Navidrome server.
///
/// `Serialize`/`Deserialize` exist so this can cross the Tauri IPC boundary as the
/// argument of `subsonic_set_config`. The wire shape is the TypeScript
/// `SubsonicConfig` interface (`src/subsonic/client.ts:8-12`) unchanged, which is why
/// `rename_all = "camelCase"` is load-bearing: `base_url` **must** serialise as
/// `baseUrl`. `username` and `password` are single words and unaffected.
///
/// Note this type carries the server password in the clear. It is deserialised from the
/// frontend exactly once, at connect; nothing serialises it back out, and no
/// [`Client`]-returning API ever hands it to the webview.
///
/// [`Debug`] is written by hand rather than derived, and **redacts the password** — see
/// the impl below.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub base_url: String,
    pub username: String,
    pub password: String,
}

/// Redacting [`Debug`]. Deriving it would let a `tracing::debug!("{cfg:?}")`, a panic
/// message, an `.unwrap()` on a `Result<_, Config>` or a failing `assert_eq!` print the
/// user's server password into a log file, a crash report or CI output — with nothing in
/// the type system objecting.
///
/// Keeping the password out of every place that does not need it is the whole point of
/// this phase, so the guarantee is structural rather than a convention to remember.
/// [`Serialize`] stays derived: the command layer needs it to accept a config over IPC,
/// and that direction is inbound only.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("base_url", &self.base_url)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl Config {
    /// `base_url` with any trailing slashes stripped, matching `client.ts`'s
    /// `apiUrl()`: `cfg.baseUrl.replace(/\/+$/, "")`.
    pub fn base_url_trimmed(&self) -> &str {
        self.base_url.trim_end_matches('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The IPC wire shape is the TypeScript `SubsonicConfig`, so `base_url` has to
    /// travel as `baseUrl`. Dropping the `rename_all` would leave the frontend's config
    /// silently un-deserialisable — a runtime-only failure at connect.
    #[test]
    fn config_round_trips_through_the_typescript_camel_case_shape() {
        let json =
            r#"{"baseUrl":"https://music.example.com","username":"rod","password":"hunter2"}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.base_url, "https://music.example.com");
        assert_eq!(cfg.username, "rod");
        assert_eq!(cfg.password, "hunter2");
        assert_eq!(serde_json::to_string(&cfg).unwrap(), json);
    }

    /// `{cfg:?}` must never be a credential leak. Serialisation is checked here too, so
    /// the test cannot be read as "the password is gone" — it is very much still there
    /// on the wire, which is exactly why `Debug` has to be the one that hides it.
    #[test]
    fn debug_redacts_the_password_but_serialisation_still_carries_it() {
        let cfg = Config {
            base_url: "https://music.example.com".into(),
            username: "rod".into(),
            password: "hunter2".into(),
        };

        let debug = format!("{cfg:?}");
        assert!(
            !debug.contains("hunter2"),
            "Config's Debug leaked the password: {debug}"
        );
        assert!(
            debug.contains("<redacted>"),
            "unexpected Debug shape: {debug}"
        );
        // The non-secret fields still have to be useful for diagnosis.
        assert!(debug.contains("https://music.example.com"));
        assert!(debug.contains("rod"));

        // Serialize is deliberately NOT redacted — it is the IPC wire format.
        assert!(serde_json::to_string(&cfg).unwrap().contains("hunter2"));
    }

    #[test]
    fn base_url_trimmed_strips_trailing_slashes() {
        let cfg = Config {
            base_url: "https://music.example.com///".into(),
            username: "rod".into(),
            password: "hunter2".into(),
        };
        assert_eq!(cfg.base_url_trimmed(), "https://music.example.com");
    }
}
