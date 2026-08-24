//! Subsonic token auth. The password is hashed with a per-request salt and never
//! sent in the clear (Subsonic's plaintext `p=` param is intentionally unsupported).
//!
//! Mirrors `client.ts`'s `authParams()` (repo root `src/subsonic/client.ts:54-66`).

use md5::{Digest, Md5};
use rand::Rng;

use crate::Config;

/// OpenSubsonic protocol version reported to the server, matching `client.ts`.
pub const API_VERSION: &str = "1.16.1";
/// Client identifier reported to the server, matching `client.ts`.
pub const CLIENT_NAME: &str = "eko";

/// Lowercase-hex-encode raw digest bytes, matching the string shape `js-md5`
/// produces in the TypeScript client.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// MD5 hex digest of `input`, matching the TypeScript client's `md5(password + salt)`.
fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    to_hex(&hasher.finalize())
}

/// Build the six auth/query params Subsonic's token auth requires: `u`, `t`, `s`,
/// `v`, `c`, `f`. `salt` is injected so this stays pure and testable; production
/// callers pass [`random_salt`]. The password itself never appears in the output.
pub fn auth_params(cfg: &Config, salt: &str) -> Vec<(String, String)> {
    let token = md5_hex(&format!("{}{}", cfg.password, salt));
    vec![
        ("u".to_string(), cfg.username.clone()),
        ("t".to_string(), token),
        ("s".to_string(), salt.to_string()),
        ("v".to_string(), API_VERSION.to_string()),
        ("c".to_string(), CLIENT_NAME.to_string()),
        ("f".to_string(), "json".to_string()),
    ]
}

/// 10 lowercase base-36 characters, matching the TypeScript client's salt shape
/// (`Math.random().toString(36).slice(2, 12)`).
pub fn random_salt() -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut rng = rand::thread_rng();
    (0..10)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            base_url: "https://music.example.com/".into(),
            username: "rod".into(),
            password: "hunter2".into(),
        }
    }

    #[test]
    fn random_salt_is_ten_base36_chars() {
        let salt = random_salt();
        assert_eq!(salt.len(), 10);
        assert!(salt
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()));
    }

    #[test]
    fn random_salt_is_not_constant() {
        assert_ne!(random_salt(), random_salt());
    }

    #[test]
    fn token_hashes_password_plus_salt() {
        let params = auth_params(&cfg(), "abcdefghij");
        let get = |k: &str| {
            params
                .iter()
                .find(|(a, _)| a == k)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("t"), md5_hex("hunter2abcdefghij"));
    }
}
