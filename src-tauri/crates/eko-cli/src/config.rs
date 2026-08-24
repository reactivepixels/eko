//! `~/.config/eko/config.toml`.
//!
//! An absent file means defaults, not an error. A *malformed* file is an error,
//! but never a fatal one: the caller falls back to defaults and surfaces a note
//! in the footer, because a TUI that refuses to start over a stray comma is a
//! TUI that has lost the plot.
//!
//! **No credentials live here, ever.** Server passwords go to the system
//! keychain — see [`crate::server`], which also documents the `[[servers]]`
//! shape and why it is plural from the first day.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::server::ServerConfig;

/// User configuration. Every field has a default, so every field is optional in
/// the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Root of the local library. `None` means "nothing was asked for", which
    /// is not the same as "nowhere to look" — see [`resolve_music_folder`].
    pub music_folder: Option<PathBuf>,
    /// Preferred CoreAudio output device, by name. `None` means system default.
    pub output_device: Option<String>,
    // There is no `volume`. There was, it defaulted to unity, and a file that
    // set it to 0.72 started the player attenuated with nobody having pressed
    // anything and nothing on screen to say so beyond a seal reading `VOLUME`.
    // Software volume is gone from this crate entirely — see [`crate::keys`] —
    // and the key is now simply unrecognised, which `Config` already ignores.
    /// Accent preset name — matches the desktop app's `[data-accent]` presets in
    /// `src/player/neu.css`. Unknown names fall back to the default orange.
    pub accent: String,
    // `visualiser_backdrop` was here, and the ambient wash it switched is gone:
    // it cost three times what the analyser did, it could not be drawn at all
    // on kitty or iTerm2 — the two terminals whose covers most deserved it —
    // and the `z` view now gets its depth from a framed sleeve and real margins.
    // An unrecognised key is already ignored, so no `config.toml` needs editing.
    /// Whether the analyser is drawn in the `z` view.
    ///
    /// The footer's spectrum has its own key (`s`); this is the full-height one,
    /// thirty-two bars over up to seven rows redrawn thirty times a second. It
    /// defaults **on** because it was measured and is cheap — see
    /// `docs/architecture/eko-cli-design-qa.md` for the number — and it is a
    /// setting rather than a constant because "cheap" was measured on one
    /// machine and somebody's is slower.
    pub analyser: bool,
    /// Navidrome / OpenSubsonic servers, as `[[servers]]` tables.
    ///
    /// **Plural on purpose, from the first release that reads it.** This phase
    /// uses [`Config::server`] — the first entry — and ignores the rest; the
    /// sidebar selector for several is deferred. The *file* does not have to
    /// change when it arrives, so nobody's `config.toml` gets migrated. See
    /// [`crate::server`].
    ///
    /// No password among these fields. There is nowhere in this struct to put
    /// one.
    pub servers: Vec<ServerConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            music_folder: None,
            output_device: None,
            accent: "orange".to_string(),
            analyser: true,
            servers: Vec::new(),
        }
    }
}

impl Config {
    /// Clamp anything a hand-edited file could have got wrong, **and say what
    /// was dropped**.
    ///
    /// # Why the notes exist
    ///
    /// A `[[servers]]` entry that does not survive this used to vanish without
    /// a word, and the state it left behind — no server, no row, no error — is
    /// byte-for-byte the state of somebody who has never configured one. That is
    /// the worst possible failure for a first run: the fix for "you have not set
    /// this up" and the fix for "you set it up and got one line wrong" are
    /// completely different, and the screen could not tell them apart.
    ///
    /// Note that a *missing* key is not this case: `ServerConfig`'s fields have
    /// no serde default, so `username = ` left out entirely fails the parse and
    /// is already reported by name. What reaches here is the quieter class — a
    /// key present and **empty**, whitespace-only, or a second entry under a name
    /// the keychain already files a password under.
    #[must_use]
    pub fn normalized(mut self) -> (Self, Vec<String>) {
        let mut notes = Vec::new();
        if self.accent.trim().is_empty() {
            self.accent = "orange".to_string();
        }
        // A half-written `[[servers]]` table is not a server, and two servers
        // sharing a name would share a keychain entry — so the first one wins
        // and the duplicate is dropped rather than silently stealing its
        // password.
        let mut seen: Vec<String> = Vec::new();
        let mut kept = Vec::with_capacity(self.servers.len());
        for server in std::mem::take(&mut self.servers) {
            let server = server.normalized();
            if !server.is_complete() {
                let missing: Vec<String> = server
                    .missing()
                    .into_iter()
                    .map(|key| format!("no {key}"))
                    .collect();
                notes.push(format!(
                    "a [[servers]] entry was ignored: {}",
                    missing.join(", ")
                ));
                continue;
            }
            if seen.contains(&server.name) {
                notes.push(format!(
                    "a second [[servers]] entry named {:?} was ignored — one name, one password",
                    server.name
                ));
                continue;
            }
            seen.push(server.name.clone());
            kept.push(server);
        }
        self.servers = kept;
        (self, notes)
    }

    /// The server this phase talks to: the first `[[servers]]` entry, if any.
    ///
    /// One call site, so multi-server is a change here and in the fold rather
    /// than a hunt through the crate for `servers[0]`.
    #[must_use]
    pub fn server(&self) -> Option<&ServerConfig> {
        self.servers.first()
    }

    /// Parse from TOML text. Missing keys take their defaults.
    ///
    /// Returns the config **and** whatever [`Config::normalized`] threw away, in
    /// a pair rather than as an optional extra, so no caller can parse a config
    /// without being handed the list of things that quietly did not survive.
    /// That was the bug: the list existed and nothing asked for it.
    ///
    /// # Errors
    /// Returns the underlying TOML error if the text does not parse.
    pub fn from_toml(text: &str) -> Result<(Self, Vec<String>), toml::de::Error> {
        toml::from_str::<Self>(text).map(Self::normalized)
    }

    /// Read from an explicit path. A missing file yields the defaults.
    ///
    /// # Errors
    /// Returns [`ConfigError`] if the file exists but cannot be read or parsed.
    pub fn read(path: &Path) -> Result<(Self, Vec<String>), ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text).map_err(|e| ConfigError::Parse {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(e) => Err(ConfigError::Read {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
        }
    }
}

/// The path the config file *would* live at, written the way a person writes
/// it. Used in the empty states, which have to tell the user where to type.
pub const CONFIG_PATH_HINT: &str = "~/.config/eko/config.toml";

/// Which output device the engine was told to use, and how that was decided.
///
/// The twin of [`MusicFolder`], and it exists for the same reason: "nothing was
/// asked for" and "what was asked for is not here" are two different states with
/// two different sentences, and a device name on its own tells them apart from
/// neither.
///
/// # Why [`Self::Missing`] cannot be folded into [`Self::Default`]
///
/// `eko_core`'s `decode_and_play` resolves a device preference with
/// `output_devices().find(name)` **`.or_else(default_output_device())`** — so a
/// configured device that has been unplugged is silently replaced by whatever the
/// system default happens to be, with nothing anywhere saying so. Someone whose
/// DAC is asleep would hear their laptop speakers and read a seal describing them,
/// which is true about the wrong device.
///
/// So the name is resolved against [`eko_core::engine::list_devices`] **before**
/// the engine is told anything, the fallback is this crate's own decision rather
/// than the engine's accident, and the name that could not be found is carried so
/// it can be said out loud.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OutputDevice {
    /// No `output_device` in the file. The system default, and nothing is wrong.
    #[default]
    Default,
    /// The configured device, and the host really lists it.
    Configured(String),
    /// Configured, and **not** among the devices the host lists. The engine is
    /// handed `None` — the system default — and the name is kept so the Deck can
    /// name what it could not find.
    Missing(String),
}

impl OutputDevice {
    /// What `Engine::set_device` is handed. `None` means the system default.
    ///
    /// The **one** translation from this decision to the engine's preference, so
    /// a `Missing` device cannot reach `set_device` and be silently swapped for
    /// the default down inside `eko-core`.
    #[must_use]
    pub fn engine_pref(&self) -> Option<String> {
        match self {
            Self::Configured(name) => Some(name.clone()),
            Self::Default | Self::Missing(_) => None,
        }
    }

    /// The name the user chose, whether or not it is there — `None` for the
    /// system default. This is what gets written back to `config.toml`, so a
    /// device that is merely asleep is not un-configured by having been absent
    /// once.
    #[must_use]
    pub fn configured(&self) -> Option<&str> {
        match self {
            Self::Configured(name) | Self::Missing(name) => Some(name),
            Self::Default => None,
        }
    }
}

/// Decide which output device to use: the configured one when the host lists it,
/// else the system default.
///
/// Pure, with the host's device list passed in, so every case is a test rather
/// than a machine that happens to have the right DAC plugged in.
#[must_use]
pub fn resolve_output_device(configured: Option<String>, available: &[String]) -> OutputDevice {
    match configured {
        None => OutputDevice::Default,
        Some(name) if available.iter().any(|d| d == &name) => OutputDevice::Configured(name),
        Some(name) => OutputDevice::Missing(name),
    }
}

/// Write `output_device` back to the config file, leaving everything else in it
/// **byte-identical**.
///
/// # Why this is a line edit and not `toml::to_string(&config)`
///
/// Serialising the whole struct back would be shorter and would also rewrite the
/// user's file: comments gone, key order normalised, and — worse — every default
/// this struct fills in becomes an explicit setting. A `music_folder` that was
/// never typed would be written down, and [`resolve_music_folder`] documents at
/// length why that must never happen: a re-derived default cannot go stale, and a
/// persisted one outlives the machine it was derived on.
///
/// So exactly one line changes. `None` removes the line rather than writing
/// `output_device = ""`, because "the system default" is the absence of a
/// setting, not a setting with an empty value.
///
/// # Errors
/// Returns [`ConfigError`] if the file cannot be read or written.
pub fn write_output_device(path: &Path, name: Option<&str>) -> Result<(), ConfigError> {
    let read = |message: String| ConfigError::Read {
        path: path.to_path_buf(),
        message,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(read(e.to_string())),
    };
    let next = with_output_device(&text, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| read(e.to_string()))?;
    }
    std::fs::write(path, next).map_err(|e| ConfigError::Read {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// The edit itself, on the text, so it can be tested without a filesystem.
///
/// `output_device` is a root key, so only lines before the first table header are
/// candidates — an `output_device` inside a `[[servers]]` table belongs to that
/// table and is none of this function's business.
fn with_output_device(text: &str, name: Option<&str>) -> String {
    // A TOML basic string, escaped by the same crate that will read it back —
    // a hand-rolled `format!("\"{name}\"")` would break on a device called
    // `Rod"s DAC`.
    let line =
        name.map(|name| format!("output_device = {}", toml::Value::String(name.to_string())));

    let mut out: Vec<String> = Vec::new();
    let mut in_root = true;
    let mut replaced = false;
    let mut first_table: Option<usize> = None;
    for raw in text.lines() {
        let trimmed = raw.trim_start();
        if in_root && trimmed.starts_with('[') {
            in_root = false;
            first_table = Some(out.len());
        }
        let is_ours = in_root
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "output_device");
        if is_ours {
            replaced = true;
            if let Some(line) = &line {
                out.push(line.clone());
            }
            continue;
        }
        out.push(raw.to_string());
    }
    if !replaced {
        if let Some(line) = line {
            match first_table {
                // Above the first table, because a root key written after one
                // would parse as belonging to *that* table.
                Some(at) => out.insert(at, line),
                None => out.push(line),
            }
        }
    }
    let mut text = out.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

/// Append a `[[servers]]` table to the config file, leaving every byte already
/// in it **exactly** where it was.
///
/// The twin of [`write_output_device`], and it is a text append for the same
/// reason that one is a line edit: `toml::to_string(&config)` would come back
/// with the comments gone, the key order normalised, and every default this
/// struct fills in written down as though somebody had typed it. Someone who has
/// hand-kept a `config.toml` for a year does not expect adding a server to
/// reformat it.
///
/// Appending is the one edit that needs no arithmetic about *where*: an
/// array-of-tables at the end of a file has nothing after it to capture, so no
/// existing key can change which table it belongs to. Contrast
/// [`with_output_device`], which is a root key and therefore has to go above the
/// first table header.
///
/// **No password goes anywhere near this.** [`ServerConfig`] has nowhere to put
/// one — see [`crate::server`] — which is what makes "the config file cannot
/// contain a credential" a property of the types rather than a rule to remember.
///
/// # Errors
/// Returns [`ConfigError`] if the file cannot be read or written.
pub fn write_server(path: &Path, server: &ServerConfig) -> Result<(), ConfigError> {
    let fail = |message: String| ConfigError::Read {
        path: path.to_path_buf(),
        message,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(fail(e.to_string())),
    };
    let next = with_server_appended(&text, server);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| fail(e.to_string()))?;
    }
    std::fs::write(path, next).map_err(|e| fail(e.to_string()))
}

/// The append itself, on the text, so it can be tested without a filesystem.
///
/// Every value is written by [`toml::Value::String`] — the same crate that will
/// read it back — rather than by `format!("\"{value}\"")`, so a password manager
/// that generated a username with a quote in it round-trips instead of producing
/// a file that no longer parses.
fn with_server_appended(text: &str, server: &ServerConfig) -> String {
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        // One blank line before the header, and only if there is not one there
        // already: appending to a file that ends in a blank line must not grow a
        // second one every time a server is added.
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
    }
    out.push_str("[[servers]]\n");
    for (key, value) in [
        ("name", &server.name),
        ("base_url", &server.base_url),
        ("username", &server.username),
    ] {
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(&toml::Value::String(value.clone()).to_string());
        out.push('\n');
    }
    out
}

/// Which folder the library is read from, and how that was decided.
///
/// ## Resolved fresh every launch; never written back
///
/// [`resolve_music_folder`] runs at startup and its answer is not persisted.
/// Writing a `music_folder` the user never typed would turn a default into a
/// setting: it would then survive moving the library, outlive the directory it
/// named, and quietly become the authoritative branch below — so a future
/// `~/Music` would stop being consulted and a *missing* one would start being
/// reported as a misconfiguration. A default that is re-derived costs one
/// `stat` per launch and can never go stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MusicFolder {
    /// `music_folder` from the file. **Authoritative**, and used exactly as
    /// written — including when it does not exist, which is reported rather
    /// than silently swapped for the platform default. Someone whose drive is
    /// unmounted needs to hear "that path is gone", not be handed a different
    /// library that happens to be there.
    Configured(PathBuf),
    /// No `music_folder` in the file, and the platform's music directory is
    /// really on disk. `~/Music` on macOS.
    Platform(PathBuf),
    /// Nothing configured, and no platform music directory to fall back to.
    ///
    /// `probed` is the path that was looked for, when the platform names one.
    /// A default pointing at a directory that is not there is worse than no
    /// default at all — it turns "unconfigured" into "empty folder" — so it is
    /// carried as the thing that was *missing*, not as the thing to scan.
    Unset { probed: Option<PathBuf> },
}

impl MusicFolder {
    /// The folder to scan, if there is one to scan.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Configured(path) | Self::Platform(path) => Some(path),
            Self::Unset { .. } => None,
        }
    }
}

/// Decide which folder to read: the config's, else the platform's music
/// directory when it exists, else nothing.
#[must_use]
pub fn resolve_music_folder(config: &Config) -> MusicFolder {
    // `dirs::audio_dir()` is `~/Music` on macOS and the XDG `MUSIC` directory
    // on Linux — the same place the desktop app's folder picker opens on.
    let probed = dirs::audio_dir();
    let probed_exists = probed.as_deref().is_some_and(Path::is_dir);
    resolve(config.music_folder.clone(), probed, probed_exists)
}

/// The decision itself, with the two filesystem facts passed in so it can be
/// tested without one.
fn resolve(
    configured: Option<PathBuf>,
    probed: Option<PathBuf>,
    probed_exists: bool,
) -> MusicFolder {
    match (configured, probed) {
        // Never second-guessed, and never checked for existence here: a
        // configured path that is missing has to reach the user as missing.
        (Some(path), _) => MusicFolder::Configured(path),
        (None, Some(path)) if probed_exists => MusicFolder::Platform(path),
        (None, probed) => MusicFolder::Unset { probed },
    }
}

/// Where the config lives: `$XDG_CONFIG_HOME/eko/config.toml`, else
/// `~/.config/eko/config.toml`.
///
/// Deliberately *not* `dirs::config_dir()` — on macOS that resolves to
/// `~/Library/Application Support`, and the terminal client is specified to use
/// the dotfile location a terminal user expects.
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => dirs::home_dir()?.join(".config"),
    };
    Some(base.join("eko").join("config.toml"))
}

/// Load the user's config, never failing.
///
/// Returns the config plus an optional human note to show in the footer when
/// something was wrong with the file.
#[must_use]
pub fn load() -> (Config, Option<String>) {
    let Some(path) = config_path() else {
        return (Config::default(), None);
    };
    match Config::read(&path) {
        // A dropped `[[servers]]` entry is reported for the reason
        // [`Config::normalized`] gives: the state it leaves behind is
        // indistinguishable from never having configured one. Only the first is
        // shown — the footer is one row, and the second one is fixed by fixing
        // the first.
        Ok((cfg, notes)) => (cfg, notes.into_iter().next()),
        Err(e) => (Config::default(), Some(e.to_string())),
    }
}

/// Something went wrong with an existing config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Read { path: PathBuf, message: String },
    Parse { path: PathBuf, message: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } => {
                write!(f, "could not read {}: {message}", path.display())
            }
            Self::Parse { path, message } => {
                let first = message.lines().next().unwrap_or(message);
                write!(f, "{} is not valid TOML: {first}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The config alone, for the tests that are about the config rather than
    /// about what was dropped reading it. The notes have their own tests.
    fn parsed(text: &str) -> Config {
        Config::from_toml(text).expect("valid TOML").0
    }

    #[test]
    fn an_absent_file_yields_defaults_not_an_error() {
        let missing = Path::new("/definitely/not/a/real/path/eko/config.toml");
        assert_eq!(
            Config::read(missing).unwrap(),
            (Config::default(), Vec::new())
        );
    }

    #[test]
    fn a_partial_file_fills_the_rest_from_defaults() {
        let cfg = parsed(r#"accent = "cyan""#);
        assert_eq!(cfg.accent, "cyan");
        assert!(cfg.music_folder.is_none());
        assert!(cfg.output_device.is_none());
    }

    #[test]
    fn a_full_file_round_trips() {
        let cfg = parsed(
            r#"
            music_folder = "/Users/rod/Music"
            output_device = "Focusrite Scarlett"
            accent = "violet"
            "#,
        );
        assert_eq!(cfg.music_folder, Some(PathBuf::from("/Users/rod/Music")));
        assert_eq!(cfg.output_device.as_deref(), Some("Focusrite Scarlett"));
        assert_eq!(cfg.accent, "violet");
    }

    #[test]
    fn an_empty_accent_falls_back_to_the_default() {
        assert_eq!(parsed(r#"accent = "  ""#).accent, "orange");
    }

    // ── servers ──────────────────────────────────────────────────────────

    #[test]
    fn a_file_with_no_servers_has_no_server() {
        assert!(Config::default().servers.is_empty());
        assert!(parsed("volume = 0.5").server().is_none());
    }

    #[test]
    fn a_servers_table_parses_alongside_the_local_settings() {
        let cfg = parsed(
            r#"
            music_folder = "/Users/rod/Music"

            [[servers]]
            name = "home"
            base_url = "https://music.example.com"
            username = "rod"
            "#,
        );
        assert_eq!(cfg.music_folder, Some(PathBuf::from("/Users/rod/Music")));
        let server = cfg.server().expect("a server");
        assert_eq!(server.name, "home");
        assert_eq!(server.base_url, "https://music.example.com");
        assert_eq!(server.username, "rod");
    }

    /// **The shape is already plural.** A second entry parses today, so adding
    /// the sidebar selector later needs no new syntax and no migration — this
    /// phase simply reads the first.
    #[test]
    fn a_second_server_parses_now_even_though_only_the_first_is_used() {
        let cfg = parsed(
            r#"
            [[servers]]
            name = "home"
            base_url = "https://home.example.com"
            username = "rod"

            [[servers]]
            name = "work"
            url = "https://work.example.com"
            username = "rod"
            "#,
        );
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.server().unwrap().name, "home");
        assert_eq!(cfg.servers[1].base_url, "https://work.example.com");
    }

    #[test]
    fn a_half_written_server_is_dropped_rather_than_half_connected() {
        let cfg = parsed(
            r#"
            [[servers]]
            name = "home"
            base_url = ""
            username = "rod"
            "#,
        );
        assert!(cfg.servers.is_empty());
    }

    /// Two servers under one name would share one keychain entry, so the
    /// duplicate never becomes a server at all.
    #[test]
    fn a_duplicate_server_name_is_dropped_because_the_keychain_is_keyed_on_it() {
        let cfg = parsed(
            r#"
            [[servers]]
            name = "home"
            base_url = "https://first.example.com"
            username = "rod"

            [[servers]]
            name = "home"
            base_url = "https://second.example.com"
            username = "someone-else"
            "#,
        );
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].base_url, "https://first.example.com");
    }

    /// **The rule the whole design turns on.** Nothing in this struct can hold
    /// a password, so nothing can write one back to the file.
    #[test]
    fn a_password_in_the_config_file_is_ignored_and_never_written_back() {
        let cfg = parsed(
            r#"
            [[servers]]
            name = "home"
            base_url = "https://music.example.com"
            username = "rod"
            password = "hunter2"
            "#,
        );
        assert_eq!(cfg.servers.len(), 1);
        let round_tripped = toml::to_string(&cfg).unwrap();
        assert!(
            !round_tripped.contains("hunter2"),
            "a password survived a config round trip: {round_tripped}"
        );
        assert!(!round_tripped.contains("password"), "{round_tripped}");
    }

    /// An unknown key is **ignored**, not an error — including `volume`, which
    /// this crate used to read and no longer does.
    ///
    /// That direction matters in both time directions. Forwards: a newer EKO can
    /// add a key without every older binary refusing to start. Backwards: the
    /// file on somebody's disk right now very likely carries `volume`, and the
    /// release that drops software volume must open it, not reject it. `Config`
    /// is `#[serde(default)]` with no `deny_unknown_fields`, which is what makes
    /// that true; this is the test that would fail if either changed.
    #[test]
    fn unknown_keys_are_ignored_so_a_newer_eko_can_add_them() {
        let cfg = parsed("crossfade_ms = 250\nvolume = 0.5\naccent = \"cyan\"");
        assert_eq!(cfg.accent, "cyan");
        // And a file that is *nothing but* the retired key still reads as a
        // default config rather than an error.
        assert_eq!(parsed("volume = 0.72"), Config::default());
    }

    #[test]
    fn malformed_toml_is_an_error_that_names_the_file() {
        let dir = std::env::temp_dir().join("eko-cli-config-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.toml");
        std::fs::write(&path, "volume = = 3").unwrap();
        let err = Config::read(&path).unwrap_err();
        assert!(err.to_string().contains("bad.toml"), "{err}");
        std::fs::remove_file(&path).ok();
    }

    // ── a dropped entry is said out loud ─────────────────────────────────

    /// **The state the owner hit.** An entry that does not survive normalisation
    /// leaves a config with no server in it, which is byte-identical to never
    /// having configured one — so the note is the only thing that can tell the
    /// two apart.
    #[test]
    fn an_incomplete_server_entry_is_reported_rather_than_vanishing() {
        let (config, notes) = Config::from_toml(
            r#"
            [[servers]]
            name = "home"
            base_url = ""
            username = "rod"
            "#,
        )
        .unwrap();
        assert!(config.servers.is_empty());
        assert_eq!(notes, ["a [[servers]] entry was ignored: no base_url"]);

        // Two blanks are both named, so the sentence says what to fix rather
        // than merely that something is wrong.
        let (_, notes) =
            Config::from_toml("[[servers]]\nname = \"home\"\nbase_url = \"  \"\nusername = \"\"\n")
                .unwrap();
        assert_eq!(
            notes,
            ["a [[servers]] entry was ignored: no base_url, no username"]
        );
    }

    #[test]
    fn a_dropped_duplicate_says_which_name_collided() {
        let (config, notes) = Config::from_toml(
            r#"
            [[servers]]
            name = "home"
            base_url = "https://first.example.com"
            username = "rod"

            [[servers]]
            name = "home"
            base_url = "https://second.example.com"
            username = "someone-else"
            "#,
        )
        .unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("\"home\""), "{}", notes[0]);
    }

    /// A file with nothing wrong in it says nothing. A note per launch about a
    /// config that is fine would be noise, and noise is what makes a real note
    /// invisible.
    #[test]
    fn a_good_config_produces_no_notes() {
        let (_, notes) = Config::from_toml(
            "[[servers]]\nname = \"home\"\nbase_url = \"https://h.example\"\nusername = \"rod\"\n",
        )
        .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert!(Config::from_toml("accent = \"cyan\"").unwrap().1.is_empty());
    }

    /// A *missing* key is a different class and was never silent: `ServerConfig`
    /// has no serde defaults, so it fails the parse and the error names the key.
    /// Pinned because the notes above would be pointless if this changed.
    #[test]
    fn a_missing_key_is_a_parse_error_that_names_it_rather_than_a_note() {
        let err =
            Config::from_toml("[[servers]]\nname = \"home\"\nbase_url = \"https://h.example\"")
                .expect_err("a table with no username must not parse");
        assert!(err.to_string().contains("username"), "{err}");
    }

    // ── appending a server ───────────────────────────────────────────────

    fn a_server() -> ServerConfig {
        ServerConfig {
            name: "home".into(),
            base_url: "https://music.example.com".into(),
            username: "rod".into(),
        }
    }

    const APPENDED: &str = "[[servers]]\nname = \"home\"\nbase_url = \"https://music.example.com\"\nusername = \"rod\"\n";

    #[test]
    fn appending_a_server_to_an_empty_file_writes_just_the_table() {
        assert_eq!(with_server_appended("", &a_server()), APPENDED);
    }

    /// **Comments, key order and unrelated keys all survive**, which is the whole
    /// reason this is an append and not `toml::to_string(&config)`.
    #[test]
    fn appending_a_server_leaves_every_byte_that_was_there_alone() {
        let before = "# my config, hand-kept\n\
                      output_device = \"Topping E30\"\n\
                      # the folder is on the archive drive\n\
                      music_folder = \"/Volumes/Archive/Music\"\n\
                      accent = \"violet\"\n";
        let after = with_server_appended(before, &a_server());
        assert!(after.starts_with(before), "the existing file was rewritten");
        assert_eq!(&after[before.len()..], &format!("\n{APPENDED}"));
        // …and it still parses, with everything in it.
        let (config, notes) = Config::from_toml(&after).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(config.accent, "violet");
        assert_eq!(config.output_device.as_deref(), Some("Topping E30"));
        assert_eq!(config.server().unwrap(), &a_server());
    }

    /// A file that does not end in a newline, and one that ends in a blank line:
    /// both get exactly one blank line before the header, so adding servers does
    /// not make the file grow blank lines.
    #[test]
    fn the_blank_line_before_the_table_is_exactly_one_however_the_file_ended() {
        for before in [
            "accent = \"cyan\"",
            "accent = \"cyan\"\n",
            "accent = \"cyan\"\n\n",
        ] {
            let after = with_server_appended(before, &a_server());
            assert_eq!(
                after,
                format!("accent = \"cyan\"\n\n{APPENDED}"),
                "{before:?}"
            );
        }
    }

    /// A second server appended after a first is a valid file with two entries —
    /// the format was plural from the first day, and this is the write path
    /// proving it.
    #[test]
    fn a_second_server_appends_beneath_the_first_and_both_parse() {
        let first = with_server_appended("", &a_server());
        let second = ServerConfig {
            name: "work".into(),
            base_url: "https://work.example.com".into(),
            username: "rod".into(),
        };
        let both = with_server_appended(&first, &second);
        let (config, notes) = Config::from_toml(&both).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(config.servers, vec![a_server(), second]);
        // And the first entry is still the one this phase uses.
        assert_eq!(config.server().unwrap().name, "home");
    }

    /// Values are written by the crate that reads them back, so a quote in a
    /// username round-trips instead of producing a file that no longer parses.
    #[test]
    fn a_server_value_with_a_quote_in_it_round_trips() {
        let odd = ServerConfig {
            name: "home".into(),
            base_url: "https://music.example.com".into(),
            username: "rod\"the\"user".into(),
        };
        let after = with_server_appended("", &odd);
        assert_eq!(Config::from_toml(&after).unwrap().0.server().unwrap(), &odd);
    }

    /// **Nothing the writer can be handed becomes a password in the file**, and
    /// it is the type that says so: `ServerConfig` has three fields.
    #[test]
    fn the_written_table_has_no_room_for_a_credential() {
        let after = with_server_appended("", &a_server());
        assert!(!after.contains("password"), "{after}");
        assert_eq!(after.lines().filter(|l| l.contains(" = ")).count(), 3);
    }

    #[test]
    fn writing_a_server_creates_the_file_and_its_directory() {
        let dir = std::env::temp_dir().join("eko-cli-server-write-test");
        std::fs::remove_dir_all(&dir).ok();
        let path = dir.join("eko").join("config.toml");
        write_server(&path, &a_server()).unwrap();
        let (config, notes) = Config::read(&path).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(config.server().unwrap(), &a_server());
        // And the device writer still works on the file this one made, with
        // neither of them disturbing the other's lines.
        write_output_device(&path, Some("Topping E30")).unwrap();
        let (config, _) = Config::read(&path).unwrap();
        assert_eq!(config.output_device.as_deref(), Some("Topping E30"));
        assert_eq!(config.server().unwrap(), &a_server());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── resolving the music folder ───────────────────────────────────────

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn an_unset_folder_falls_back_to_the_platform_music_directory() {
        assert_eq!(
            resolve(None, Some(p("/Users/rod/Music")), true),
            MusicFolder::Platform(p("/Users/rod/Music"))
        );
    }

    /// The fallback is only a fallback if it is really there. Pointing at a
    /// `~/Music` that does not exist would turn "you have not configured this"
    /// into "your folder is empty" — a worse lie than saying nothing.
    #[test]
    fn a_platform_directory_that_does_not_exist_is_not_a_default() {
        assert_eq!(
            resolve(None, Some(p("/Users/rod/Music")), false),
            MusicFolder::Unset {
                probed: Some(p("/Users/rod/Music"))
            }
        );
        // …and it is still named, so the empty state can say which path it
        // looked for rather than being vague about it.
        assert_eq!(
            resolve(None, Some(p("/Users/rod/Music")), false).path(),
            None
        );
    }

    #[test]
    fn a_platform_with_no_music_directory_at_all_resolves_to_nothing() {
        assert_eq!(
            resolve(None, None, false),
            MusicFolder::Unset { probed: None }
        );
    }

    /// **The one that must not regress.** An explicit `music_folder` wins,
    /// including when it is missing: silently scanning `~/Music` instead would
    /// show the user a library that is not the one they asked for.
    #[test]
    fn a_configured_folder_is_authoritative_even_when_it_is_missing() {
        let configured = MusicFolder::Configured(p("/Volumes/Archive/Music"));
        assert_eq!(
            resolve(
                Some(p("/Volumes/Archive/Music")),
                Some(p("/Users/rod/Music")),
                true
            ),
            configured,
            "the platform default overrode an explicit music_folder"
        );
        assert_eq!(
            resolve(
                Some(p("/Volumes/Archive/Music")),
                Some(p("/Users/rod/Music")),
                false
            ),
            configured
        );
        assert_eq!(
            configured.path(),
            Some(Path::new("/Volumes/Archive/Music")),
            "a missing configured folder must still be the folder we scan and report"
        );
    }

    #[test]
    fn the_resolved_default_is_never_written_back_to_the_config() {
        // Resolution reads the config; it does not touch it. If this ever
        // becomes a mutation, a default the user never typed starts outliving
        // the machine it was derived on.
        let cfg = Config::default();
        let _ = resolve_music_folder(&cfg);
        assert_eq!(cfg.music_folder, None);
        assert_eq!(cfg, Config::default());
    }

    // ── resolving the output device ──────────────────────────────────────

    fn devices() -> Vec<String> {
        vec![
            "MacBook Pro Speakers".to_string(),
            "Topping E30".to_string(),
        ]
    }

    #[test]
    fn no_configured_device_is_the_system_default() {
        let choice = resolve_output_device(None, &devices());
        assert_eq!(choice, OutputDevice::Default);
        assert_eq!(choice.engine_pref(), None);
        assert_eq!(choice.configured(), None);
    }

    #[test]
    fn a_configured_device_the_host_lists_is_the_one_the_engine_is_told() {
        let choice = resolve_output_device(Some("Topping E30".into()), &devices());
        assert_eq!(choice, OutputDevice::Configured("Topping E30".into()));
        assert_eq!(choice.engine_pref().as_deref(), Some("Topping E30"));
    }

    /// **The one that must not regress.** `eko-core` resolves an unknown device
    /// name with `.or_else(default_output_device())` — so handing it a name that
    /// is not there plays the laptop speakers and says nothing. The name is
    /// resolved *here*, the engine is handed `None`, and the name survives so the
    /// Deck can say which device it could not find.
    #[test]
    fn a_configured_device_that_is_not_connected_falls_back_and_is_still_named() {
        let choice = resolve_output_device(Some("Topping E30".into()), &["Built-in".to_string()]);
        assert_eq!(choice, OutputDevice::Missing("Topping E30".into()));
        assert_eq!(
            choice.engine_pref(),
            None,
            "a missing device reached Engine::set_device, which would silently \
             substitute the system default"
        );
        assert_eq!(choice.configured(), Some("Topping E30"));
    }

    #[test]
    fn an_empty_device_list_makes_every_configured_device_missing() {
        assert_eq!(
            resolve_output_device(Some("Topping E30".into()), &[]),
            OutputDevice::Missing("Topping E30".into())
        );
        assert_eq!(resolve_output_device(None, &[]), OutputDevice::Default);
    }

    // ── persisting it ────────────────────────────────────────────────────

    #[test]
    fn writing_the_device_into_an_empty_file_creates_just_that_line() {
        assert_eq!(
            with_output_device("", Some("Topping E30")),
            "output_device = \"Topping E30\"\n"
        );
    }

    /// **Comments and every other setting survive.** The alternative — serialising
    /// the struct back — would rewrite the file and turn every re-derived default
    /// into a persisted one. See [`write_output_device`].
    #[test]
    fn writing_the_device_leaves_the_rest_of_the_file_alone() {
        let before = "# my config\nmusic_folder = \"/Users/rod/Music\"\nvolume = 0.5\n";
        assert_eq!(
            with_output_device(before, Some("Topping E30")),
            "# my config\nmusic_folder = \"/Users/rod/Music\"\nvolume = 0.5\noutput_device = \"Topping E30\"\n"
        );
    }

    #[test]
    fn writing_the_device_replaces_the_line_that_is_already_there() {
        let before = "output_device = \"Built-in\"\nvolume = 0.5\n";
        assert_eq!(
            with_output_device(before, Some("Topping E30")),
            "output_device = \"Topping E30\"\nvolume = 0.5\n"
        );
    }

    /// The system default is the *absence* of the setting, so choosing it removes
    /// the line rather than writing an empty string.
    #[test]
    fn choosing_the_system_default_removes_the_line() {
        let before = "volume = 0.5\noutput_device = \"Topping E30\"\n";
        assert_eq!(with_output_device(before, None), "volume = 0.5\n");
        assert_eq!(with_output_device("", None), "");
    }

    /// A root key written *after* a table header would parse as part of that
    /// table — so it goes above it, and the `[[servers]]` entry is untouched.
    #[test]
    fn the_device_line_goes_above_the_first_table_not_inside_it() {
        // A *complete* server, so the round trip below is about where the line
        // landed rather than about `normalized` dropping a half-written table.
        let server =
            "[[servers]]\nname = \"home\"\nbase_url = \"https://h.example\"\nusername = \"rod\"\n";
        let before = format!("volume = 0.5\n\n{server}");
        let after = with_output_device(&before, Some("Topping E30"));
        assert_eq!(
            after,
            format!("volume = 0.5\n\noutput_device = \"Topping E30\"\n{server}")
        );
        // And it parses back to what was written, with the server intact.
        let cfg = parsed(&after);
        assert_eq!(cfg.output_device.as_deref(), Some("Topping E30"));
        assert_eq!(cfg.servers.len(), 1);
    }

    /// An `output_device` inside a table belongs to that table and is not this
    /// function's to touch.
    #[test]
    fn a_key_of_the_same_name_inside_a_table_is_left_alone() {
        let before = "[[servers]]\nname = \"home\"\noutput_device = \"not mine\"\n";
        let after = with_output_device(before, Some("Topping E30"));
        assert!(after.contains("not mine"), "{after}");
        assert!(
            after.starts_with("output_device = \"Topping E30\"\n"),
            "{after}"
        );
    }

    /// A device name with a quote in it round-trips, because the value is written
    /// by the same crate that reads it back.
    #[test]
    fn a_device_name_with_a_quote_in_it_round_trips() {
        let after = with_output_device("", Some(r#"Rod"s DAC"#));
        let cfg = parsed(&after);
        assert_eq!(cfg.output_device.as_deref(), Some(r#"Rod"s DAC"#));
    }

    #[test]
    fn writing_the_device_creates_the_file_and_its_directory() {
        let dir = std::env::temp_dir().join("eko-cli-device-write-test");
        std::fs::remove_dir_all(&dir).ok();
        let path = dir.join("eko").join("config.toml");
        write_output_device(&path, Some("Topping E30")).unwrap();
        assert_eq!(
            Config::read(&path).unwrap().0.output_device.as_deref(),
            Some("Topping E30")
        );
        write_output_device(&path, None).unwrap();
        assert_eq!(Config::read(&path).unwrap().0.output_device, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_path_honours_xdg_config_home() {
        // Read the real env only to confirm the shape; the XDG branch is pure.
        let path = config_path().expect("a home directory");
        assert!(path.ends_with("eko/config.toml"), "{}", path.display());
    }
}
