//! The Navidrome / OpenSubsonic server: where it is, who you are, and the one
//! secret that must never touch a file in `~/.config`.
//!
//! Three things live here, and they are separated on purpose:
//!
//! * [`ServerConfig`] — the **non-secret** half, read from `config.toml`.
//! * [`password`] / [`set_password`] — the **secret** half, read from and
//!   written to the OS keychain and nowhere else.
//! * [`connect`] / [`spawn`] — putting the two together into a live
//!   [`eko_net::Client`], off the render thread.
//!
//! ## The config shape, and why it is already plural
//!
//! One server works in this phase; the sidebar selector for several is deferred.
//! The *file format* is not deferred, because changing it later would break
//! every config a user had written in the meantime. So the file takes an
//! array-of-tables from the first day:
//!
//! ```toml
//! [[servers]]
//! name = "home"
//! base_url = "https://music.example.com"
//! username = "rod"
//! ```
//!
//! This phase reads `servers[0]` and ignores the rest. Multi-server is then a
//! change to `eko-cli`'s *fold* — iterate instead of `.first()`, and give the
//! sidebar a row per entry — with **no** migration and no second syntax: a
//! second `[[servers]]` block is already a valid file today, and already gets
//! its own keychain entry, because the keychain account is derived from `name`
//! (see [`account_for`]) rather than from a position in the list.
//!
//! `name` is therefore load-bearing rather than cosmetic: it is the identity a
//! stored password is filed under. Renaming a server orphans its password, which
//! is why [`ServerError::NoPassword`] names the command that fixes it.
//!
//! ## The keychain: a service of our own
//!
//! The desktop app stores its secrets under the service **`com.reactivepixels.eko`**
//! with an account name passed straight through from the webview
//! (`crates/eko-tauri/src/lib.rs`'s `secret_set` / `secret_get`). That namespace
//! also holds `eko.license.key` and `eko.offline.aes_key`, and a prior review in
//! this project flagged the unvalidated account name as a confused-deputy risk:
//! anything that can ask the command layer for a key can ask for *any* key in the
//! namespace.
//!
//! This module uses a **separate service**, [`KEYCHAIN_SERVICE`], for two
//! reasons.
//!
//! 1. **The shared namespace is not reachable anyway.** The desktop files each
//!    server's password under `navidrome-<uuid>`, where the uuid is minted by
//!    `crypto.randomUUID()` and the mapping from uuid to server lives in the
//!    webview's `localStorage` (`src/subsonic/serverList.ts`). A terminal client
//!    cannot read `localStorage`, so it cannot know which account belongs to
//!    which server. "One password for both clients" is not on offer without a
//!    shared registry that does not exist; pretending otherwise would mean
//!    guessing at account names.
//! 2. **A separate service is a smaller blast radius.** Nothing the desktop's
//!    unvalidated-key path can be talked into reading reaches this service, and
//!    nothing here can reach the licence key.
//!
//! And the pattern itself is **not** inherited: no caller supplies an account
//! name. [`account_for`] is the only thing that builds one, it is private, it
//! prefixes a fixed [`ACCOUNT_PREFIX`], and it rejects any `name` outside
//! `[A-Za-z0-9._-]{1,64}` — so a config file cannot address `eko.license.key`,
//! escape the prefix, or smuggle separators into the account string.
//!
//! ## Blocking, and therefore off-thread
//!
//! Both halves block. `Client::ping` is a network round trip, and a keychain
//! read can put a *modal system prompt* in front of the user on macOS. Neither
//! may happen on the render thread; [`spawn`] is the only entry point the fold
//! uses, and it mirrors [`crate::library::spawn`] exactly.

use std::fmt;
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use eko_net::{Client, SubsonicError};
use serde::{Deserialize, Serialize};

/// The keychain service EKO's **terminal** client stores passwords under.
///
/// Deliberately not the desktop's `com.reactivepixels.eko` — see the module
/// docs.
pub const KEYCHAIN_SERVICE: &str = "com.reactivepixels.eko.cli";

/// Every account this module writes starts with this. Fixed, and prepended by
/// [`account_for`] alone, so the account space cannot be addressed from a
/// config file.
pub const ACCOUNT_PREFIX: &str = "server.";

/// Longest server `name` that can be a keychain account.
pub const MAX_NAME_LEN: usize = 64;

// ── The non-secret half ──────────────────────────────────────────────────────

/// One server, as written in `config.toml`. **Never carries a password.**
///
/// `base_url` also accepts the key `url`, because that is what half of everyone
/// will type and a silently-ignored key is a trap rather than a strictness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// The name this server is known by — in the sidebar, in `eko-cli login`, and
    /// as the keychain account. See the module docs.
    pub name: String,
    #[serde(alias = "url")]
    pub base_url: String,
    pub username: String,
}

impl ServerConfig {
    /// Is every field filled in? A half-written table is not a server.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.base_url.trim().is_empty()
            && !self.username.trim().is_empty()
    }

    /// Which fields are blank, named the way the file spells them.
    ///
    /// Used by [`crate::config::Config::normalized_with_notes`] so a dropped
    /// entry can say *what* was missing. Empty when the entry is complete.
    #[must_use]
    pub fn missing(&self) -> Vec<&'static str> {
        [
            ("name", &self.name),
            ("base_url", &self.base_url),
            ("username", &self.username),
        ]
        .into_iter()
        .filter(|(_, value)| value.trim().is_empty())
        .map(|(key, _)| key)
        .collect()
    }

    /// Trim the fields a hand-edited file will have stray whitespace in.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.base_url = self.base_url.trim().to_string();
        self.username = self.username.trim().to_string();
        self
    }
}

/// What was done to a pasted base URL to make it one, and whether it can be one
/// at all.
///
/// The corrections are **reported, not applied in silence**: someone who pasted
/// their Navidrome address out of a browser and had `/rest` quietly removed
/// would have no way to tell that from a client that ignores what you type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseUrl {
    /// The URL as it will be written to `config.toml`.
    pub url: String,
    /// What was changed, in the order it was changed. Empty when the paste was
    /// already a base URL.
    pub fixes: Vec<String>,
}

/// Turn what people actually paste into a base URL, or say why it is not one.
///
/// A Navidrome address gets copied out of a browser bar, out of a Docker compose
/// file, or off a phone. It arrives with a trailing slash, or with the `/rest`
/// that `eko-net` appends itself (`{base}/rest/ping`), or with neither and no
/// scheme either. The first two are typing this client can undo; the third is
/// not, and guessing `https://` on someone's behalf would be this client picking
/// their transport security for them.
///
/// # Errors
/// A sentence to put in front of the user, naming the fix. Never the raw paste
/// re-echoed on its own — the message is rendered, and the paste is untrusted
/// text, so it goes through [`tidy`] first.
pub fn normalise_base_url(input: &str) -> Result<BaseUrl, String> {
    let input = tidy(input);
    let input = input.trim();
    if input.is_empty() {
        return Err("a URL is needed — e.g. https://music.example.com".to_string());
    }
    let Some((scheme, rest)) = input.split_once("://") else {
        return Err(format!("no scheme — try https://{input}"));
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return Err(format!("{scheme}:// is not http or https"));
    }

    let mut fixes = Vec::new();
    let mut rest = rest.trim_end_matches('/').to_string();
    if rest.len() < input.len() - scheme.len() - "://".len() {
        fixes.push("dropped the trailing /".to_string());
    }
    // `eko-net` builds `{base}/rest/{method}`, so a base that already ends in
    // `/rest` produces `/rest/rest/ping` and a 404 that reads as "the server
    // answered HTTP 404" — a true sentence about the wrong problem.
    if let Some(trimmed) = strip_suffix_ignore_ascii_case(&rest, "/rest") {
        rest = trimmed.trim_end_matches('/').to_string();
        fixes.push("dropped the trailing /rest — EKO adds it".to_string());
    }
    // Everything up to the first `/` is the authority. A URL with a path is
    // fine — Navidrome behind a reverse proxy is often at `/music`.
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() {
        return Err("that URL has no host in it".to_string());
    }

    Ok(BaseUrl {
        url: format!("{}://{rest}", scheme.to_ascii_lowercase()),
        fixes,
    })
}

/// `str::strip_suffix`, case-blind, for the ASCII suffixes above.
fn strip_suffix_ignore_ascii_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let at = text.len().checked_sub(suffix.len())?;
    text.is_char_boundary(at)
        .then(|| text.split_at(at))
        .filter(|(_, tail)| tail.eq_ignore_ascii_case(suffix))
        .map(|(head, _)| head)
}

/// A `name` that can safely become a keychain account: `[A-Za-z0-9._-]`, one to
/// [`MAX_NAME_LEN`] characters.
///
/// The charset is a whitelist rather than a blacklist because the account string
/// is the only thing separating this server's password from every other secret
/// on the machine. ASCII-only, so `len()` is the character count too.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The keychain account for `name`. **The only place an account is minted.**
fn account_for(name: &str) -> Result<String, ServerError> {
    if !is_valid_name(name) {
        return Err(ServerError::BadName(name.to_string()));
    }
    Ok(format!("{ACCOUNT_PREFIX}{name}"))
}

// ── Failures ─────────────────────────────────────────────────────────────────

/// Everything that can stop a connection, in the order a user meets them.
///
/// Every variant's [`Display`](fmt::Display) is rendered verbatim in the footer,
/// so **none of them may carry a URL**: `eko_net::urls::api_url` signs its URLs
/// with `u`/`t`/`s`, and that triple is a replayable credential. The only
/// server-shaped string in these messages is the *name* from `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
    /// The `name` cannot be a keychain account. See [`is_valid_name`].
    BadName(String),
    /// Nothing is stored for this server yet — the first-run state.
    NoPassword(String),
    /// The keychain itself refused: locked, unavailable, or denied.
    Keychain(String),
    /// The server refused us, or could not be reached at all.
    Connect(String),
}

impl ServerError {
    /// The first-run state, which the Deck reports differently from a failure:
    /// nothing is broken, a step has not been taken.
    #[must_use]
    pub fn is_missing_password(&self) -> bool {
        matches!(self, Self::NoPassword(_))
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName(name) => write!(
                f,
                "server name {name:?} is not usable — letters, digits, . _ - only, \
                 up to {MAX_NAME_LEN} characters"
            ),
            Self::NoPassword(name) => {
                write!(f, "no password stored · run: eko-cli login {name}")
            }
            Self::Keychain(message) => write!(f, "keychain: {message}"),
            Self::Connect(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ServerError {}

/// Turn an `eko-net` failure into one sentence a user can act on, **in this
/// client's own words**.
///
/// The four variants mean genuinely different things and have genuinely
/// different fixes, so they are not flattened into "connection failed":
/// `Subsonic` is the server answering *no* (a wrong password arrives this way —
/// HTTP 200 with `status: "failed"`), `Request` is never having reached it,
/// `Http` is a proxy or a wrong path, and `BadResponse` is something that is not
/// a Subsonic server.
///
/// **Nothing here formats a URL.** `SubsonicError::Request`'s payload has already
/// been through `eko_net::client::safe_message`, which strips the signed URL that
/// `reqwest`'s `Display` would otherwise append.
///
/// # Why this one drops the server's sentence, and [`describe_at_login`] does not
///
/// `SubsonicError::Subsonic` carries **the server's own prose**, chosen by the
/// far end. A hostile or merely broken server that echoes the credential back in
/// an error message would put it on screen wherever that prose is rendered — and
/// only [`connect_with`] holds the password and the message at the same time, so
/// only [`connect_with`] can redact it. The album walk, the track fetch and the
/// search do not, and no amount of care at those call sites can give them a
/// secret to compare against.
///
/// So the default is the one that is safe **without** a redactor: our sentence,
/// and the code where there is one. That closes the class rather than three call
/// sites — a fourth request path added tomorrow gets the safe rendering by
/// reaching for the obvious function, and the risky one has a name that says
/// what it costs. The type-level containment stays exactly as it was: nothing
/// here needs the password, so nothing here needs `App` to be able to hold one.
///
/// **The Subsonic error *code* is not in the sentence because this crate never
/// receives it.** `eko_net::parse::envelope` keeps `error.message` and discards
/// `error.code`, and `eko-net` is shared with the desktop app — so the code is a
/// change to a crate two clients depend on rather than something to be invented
/// here. `Http` already carries its status, which is the one code that survives.
pub(crate) fn describe(error: &SubsonicError) -> String {
    match error {
        // Deliberately **not** `tidy(message)`. See the docs above: the message
        // is the far end's, and the far end is exactly who this is guarding
        // against. `tidy` would make it safe to *draw*; it cannot make it safe
        // to *say*.
        SubsonicError::Subsonic(_) => "the server refused the request".to_string(),
        SubsonicError::Request(message) => format!("cannot reach the server · {}", tidy(message)),
        SubsonicError::Http(status) => format!("the server answered HTTP {status}"),
        SubsonicError::BadResponse => "that does not look like a Subsonic server".to_string(),
    }
}

/// [`describe`], plus the server's own sentence for `SubsonicError::Subsonic`.
///
/// **One caller, and it is the only one that may have one:** [`connect_with`],
/// which holds the password and pushes the result through [`without_password`]
/// before it can become a [`ServerError`]. Signing in is also where the remote
/// sentence earns its place — "Wrong username or password." is the whole answer
/// to the question being asked, and "the server refused the request" at a login
/// prompt would be this client withholding the one fact the user needs.
///
/// Anywhere else, use [`describe`]. There is no redactor to hand at any other
/// call site, and a message this client did not write is a message it cannot
/// vouch for.
fn describe_at_login(error: &SubsonicError) -> String {
    match error {
        // The server's own words — "Wrong username or password." for code 40.
        // Remote text, so it goes through `tidy` before it reaches a `Line`.
        SubsonicError::Subsonic(message) => tidy(message),
        other => describe(other),
    }
}

/// What a redacted password looks like. Not the length of the original — a
/// fixed run, so the message does not leak how long the secret is.
const REDACTED: &str = "·····";

/// Take `secret` out of a message that is about to become renderable.
///
/// A one-line, exact-substring replacement, and deliberately nothing cleverer: a
/// fuzzy match would start mangling legitimate text, and the only case that
/// matters is a server (or a proxy, or a middlebox) echoing the credential back
/// verbatim. An empty secret is left alone rather than matching everywhere.
///
/// Not constant-time, and it does not need to be: both sides of the comparison
/// are already in this process, and the output is going on a screen.
fn without_password(message: String, secret: &str) -> String {
    if secret.is_empty() {
        return message;
    }
    message.replace(secret, REDACTED)
}

/// Longest error sentence the footer will ever be asked to hold.
///
/// The footer truncates for width already; this is about not carrying a
/// megabyte of somebody's stack trace around in [`crate::app::App::status`].
const MAX_MESSAGE_LEN: usize = 160;

/// Characters that can move text around a terminal row without being control
/// codes.
///
/// `char::is_control` is Unicode category `Cc` — the C0 and C1 ranges, `DEL`
/// included. That is the whole of what can *emit* an escape sequence, and for a
/// while it was the whole of what [`tidy`] removed. It is not the whole of what
/// can wreck a row:
///
/// * `U+2028` / `U+2029` are LINE SEPARATOR and PARAGRAPH SEPARATOR — `Zl`/`Zp`,
///   not `Cc`. They are newlines by another name, and a `ratatui` `Line` is one
///   row.
/// * The explicit bidirectional formatting characters (`Cf`) reverse the
///   *visible* order of everything after them. `Zaireeka\u{202e}` renders the
///   rest of the row backwards; a right-to-left override in an album name can
///   make a track list read as somebody else's, and it survives every
///   copy-and-paste of the text used to report the bug.
///
/// Neither class can inject an escape sequence, so neither is a terminal-control
/// vulnerability — but both let a server decide what a row *looks* like, which is
/// the property this crate refuses to hand over. They are collapsed to spaces
/// with everything else.
///
/// Deliberately **not** included: `U+200C`/`U+200D` (ZWNJ/ZWJ) and `U+FEFF`.
/// Those are `Cf` too, but they join and separate graphemes rather than reorder
/// them — they are load-bearing in Persian, in Indic scripts and in emoji
/// sequences, and stripping them would corrupt legitimate names to defend
/// against nothing.
/// **The character rule [`tidy`] enforces, on its own.**
///
/// `tidy` does two separable jobs: it neutralises characters that must never
/// reach a terminal, and it tidies whitespace so a remote message reads as one
/// row. The first is the security gate; the second is cosmetics for a *sentence*
/// — and the two came apart the moment a text field wanted the gate without the
/// cosmetics.
///
/// A field being typed into one character at a time cannot use the whole of
/// `tidy`: a lone `' '` tidies to the empty string, because a leading space in a
/// message is worth squeezing and a space being typed is not. That is not
/// hypothetical — the search input has never been able to take a two-word query,
/// because every space went through `tidy(" ")` on its own and came back as
/// nothing. See [`crate::line::type_into`], which is now the other caller.
pub(crate) fn is_unsafe_in_a_row(c: char) -> bool {
    // `is_control` is `Cc`, which already includes `DEL` (U+007F).
    c.is_control() || is_layout_control(c)
}

fn is_layout_control(c: char) -> bool {
    matches!(
        c,
        // Zl / Zp
        '\u{2028}' | '\u{2029}'
        // LRM, RLM, ALM
        | '\u{200e}' | '\u{200f}' | '\u{061c}'
        // LRE, RLE, PDF, LRO, RLO
        | '\u{202a}'..='\u{202e}'
        // LRI, RLI, FSI, PDI
        | '\u{2066}'..='\u{2069}'
    )
}

/// Make text from a remote server safe to put in a widget. **The one gate.**
///
/// Everything a server writes reaches the screen through here: error messages
/// (via [`describe`]) and, since Task 2, every album name, artist and track title
/// (via [`crate::remote`]'s `from_sub` constructors, which are the only way to
/// build a [`crate::remote::Album`] or [`crate::remote::Track`]).
///
/// A newline breaks a `ratatui` `Line` into a shape the footer's width arithmetic
/// does not expect. A control character is written to the terminal **as-is** —
/// `ESC [ 2 J` clears the screen, `ESC ] 0 ; … BEL` retitles the window, an SGR
/// repaints the row the signal-path seal lives on. See [`is_layout_control`] for
/// the second class this also removes.
///
/// All of them are collapsed to spaces, runs of spaces are squeezed, and the
/// result is capped at [`MAX_MESSAGE_LEN`]. One function, so there is one thing
/// to audit and one thing to widen.
pub(crate) fn tidy(message: &str) -> String {
    let mut out = String::with_capacity(message.len().min(MAX_MESSAGE_LEN));
    let mut last_was_space = false;
    for c in message.chars() {
        let c = if is_unsafe_in_a_row(c) { ' ' } else { c };
        if c == ' ' {
            if last_was_space || out.is_empty() {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        if out.chars().count() == MAX_MESSAGE_LEN {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out.trim_end().to_string()
}

// ── The secret half ──────────────────────────────────────────────────────────

fn entry(name: &str) -> Result<keyring::Entry, ServerError> {
    let account = account_for(name)?;
    keyring::Entry::new(KEYCHAIN_SERVICE, &account)
        .map_err(|e| ServerError::Keychain(e.to_string()))
}

/// Read `name`'s password out of the OS keychain.
///
/// **Blocking, and possibly modal** — macOS can put an authorisation dialog in
/// front of this. Never call it from the render thread; [`spawn`] is the way in.
///
/// # Errors
/// [`ServerError::NoPassword`] when nothing is stored, [`ServerError::BadName`]
/// for a name that cannot be an account, [`ServerError::Keychain`] otherwise.
pub fn password(name: &str) -> Result<String, ServerError> {
    match entry(name)?.get_password() {
        Ok(password) => Ok(password),
        Err(keyring::Error::NoEntry) => Err(ServerError::NoPassword(name.to_string())),
        Err(e) => Err(ServerError::Keychain(e.to_string())),
    }
}

/// Store `password` for `name`, replacing whatever was there.
///
/// # Errors
/// As [`password`], minus [`ServerError::NoPassword`].
pub fn set_password(name: &str, password: &str) -> Result<(), ServerError> {
    entry(name)?
        .set_password(password)
        .map_err(|e| ServerError::Keychain(e.to_string()))
}

/// Forget `name`'s password. Idempotent: not being there is success.
///
/// # Errors
/// As [`set_password`].
pub fn delete_password(name: &str) -> Result<(), ServerError> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(ServerError::Keychain(e.to_string())),
    }
}

// ── A live connection ────────────────────────────────────────────────────────

/// A server we have actually reached.
///
/// Holds the [`Client`], which holds the password. It is behind an [`Arc`] so a
/// worker can be handed a second reference without a second login, and it is
/// **not** `Debug`-derivable — see the impl below.
#[derive(Clone)]
pub struct Connection {
    pub name: String,
    pub username: String,
    /// The connected client, shared with whatever worker is using it.
    ///
    /// Behind an [`Arc`] so [`crate::remote`]'s page walk can be handed a second
    /// reference without a second login, and so the fold can drop its own copy on
    /// a disconnect while an in-flight request finishes on its thread. Rebuilding
    /// one per request would throw away the connection pool and re-pay TLS on
    /// every call.
    pub client: Arc<Client>,
}

/// Redacting [`Debug`], for the same reason `eko_net::Config` has one.
///
/// `Connection` reaches the fold inside an [`AppEvent`](crate::app::AppEvent),
/// and `AppEvent` derives `Debug`. A derived impl here would put the whole
/// `eko_net::Config` — password included — into any `{event:?}` in a panic
/// message, a failing `assert_eq!`, or a future log line. The client is not
/// printable at all; what is left is the two things worth diagnosing.
impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("name", &self.name)
            .field("username", &self.username)
            .field("client", &"<connected>")
            .finish()
    }
}

/// What a connection attempt reports back to the fold.
///
/// The twin of [`crate::library::ScanEvent`], and it travels the same channel.
#[derive(Debug, Clone)]
pub enum ConnEvent {
    /// The server is configured and the keychain has nothing for it. A first
    /// run, not a failure.
    NeedsPassword { name: String },
    /// Reached, and the credentials were accepted.
    Connected(Box<Connection>),
    /// Anything else. `message` is safe to render — see [`ServerError`].
    Failed { name: String, message: String },
}

/// Build a client for `server` using a password supplied by the caller, and
/// prove it works with a `ping`.
///
/// Split out from [`connect`] so the whole network path can be driven against a
/// mock server with no keychain in the picture — which is what the tests at the
/// bottom of this file do.
///
/// # Errors
/// [`ServerError::Connect`], with a message that carries no credential.
pub fn connect_with(server: &ServerConfig, password: String) -> Result<Connection, ServerError> {
    // **The failure path may not carry the secret, whoever wrote the sentence.**
    // `describe_at_login` renders `SubsonicError::Subsonic` — the *server's* own
    // words — verbatim, because at a sign-in prompt that sentence is the answer.
    // A server that echoes the credential back in it would put the credential
    // straight into the footer. That is not hypothetical politeness: this
    // project has already fixed this exact class twice, in `engine.rs` and
    // `pro/offline.rs`. Here is the only place in the crate that holds the
    // password and the message at the same time, so here is the only place that
    // may render that sentence at all — everywhere else uses `describe`, which
    // does not.
    let redact = {
        let secret = password.clone();
        move |e: &SubsonicError| {
            ServerError::Connect(without_password(describe_at_login(e), &secret))
        }
    };
    let cfg = eko_net::Config {
        base_url: server.base_url.clone(),
        username: server.username.clone(),
        password,
    };
    let client = Client::new(cfg).map_err(|e| redact(&e))?;
    // The only I/O in this function. A wrong password arrives here as
    // `SubsonicError::Subsonic` — Subsonic reports auth failure in the *body*,
    // with HTTP 200 and `status: "failed"`, code 40.
    client.ping().map_err(|e| redact(&e))?;
    Ok(Connection {
        name: server.name.clone(),
        username: server.username.clone(),
        client: Arc::new(client),
    })
}

/// Read the keychain, connect, and ping.
///
/// **Blocking on both counts.** See [`password`] and [`connect_with`].
///
/// # Errors
/// Whatever [`password`] or [`connect_with`] returned.
pub fn connect(server: &ServerConfig) -> Result<Connection, ServerError> {
    let password = password(&server.name)?;
    connect_with(server, password)
}

/// Connect on a worker thread, reporting through `emit`.
///
/// `emit` returns `false` once the fold has hung up, at which point there is
/// nothing left to tell. The thread is detached for the same reason
/// [`crate::library::spawn`]'s is: it owns nothing that needs dropping in order.
pub fn spawn<F>(server: ServerConfig, emit: F)
where
    F: Fn(ConnEvent) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        emit(attempt(&server));
    });
}

/// One connection attempt, as an event. Synchronous, so a test can drive it.
#[must_use]
pub fn attempt(server: &ServerConfig) -> ConnEvent {
    match connect(server) {
        Ok(connection) => ConnEvent::Connected(Box::new(connection)),
        Err(e) if e.is_missing_password() => ConnEvent::NeedsPassword {
            name: server.name.clone(),
        },
        Err(e) => ConnEvent::Failed {
            name: server.name.clone(),
            message: e.to_string(),
        },
    }
}

// ── Getting the password in ──────────────────────────────────────────────────

/// What a keystroke did to the password being typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStep {
    /// Keep reading.
    Continue,
    /// Enter — the buffer is the password.
    Submit,
    /// Esc or Ctrl-C — throw the buffer away.
    Cancel,
}

/// Fold one key into the password being typed.
///
/// Pure, and therefore the part that is tested. The rest of [`prompt_password`]
/// is terminal bookkeeping.
///
/// **Nothing here produces output.** That is the entire no-echo mechanism: raw
/// mode turns the terminal's own echo off, and this function is the only thing
/// that sees the character. Adding a `print!` here would be the only way to make
/// the password visible, which is why the buffer never leaves as anything but a
/// return value.
#[must_use]
pub fn apply_key(buffer: &mut String, key: KeyEvent) -> PromptStep {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter => PromptStep::Submit,
        KeyCode::Esc => PromptStep::Cancel,
        // Raw mode swallows SIGINT, so Ctrl-C has to be handled by hand or the
        // prompt becomes a trap you cannot leave.
        KeyCode::Char('c' | 'C') if ctrl => PromptStep::Cancel,
        // Ctrl-U clears the line, as it does in a shell.
        KeyCode::Char('u' | 'U') if ctrl => {
            buffer.clear();
            PromptStep::Continue
        }
        // Character-wise, not byte-wise: a passphrase can contain anything.
        KeyCode::Backspace => {
            buffer.pop();
            PromptStep::Continue
        }
        KeyCode::Char(c) if !ctrl => {
            buffer.push(c);
            PromptStep::Continue
        }
        _ => PromptStep::Continue,
    }
}

/// Restores raw mode however the prompt exits — including on a panic.
struct RawGuard;

impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Ask for a password on the terminal, without echoing it.
///
/// Returns `None` if the user cancelled.
///
/// The prompt goes to **stderr**, so `eko-cli login > somewhere` still shows it and
/// never mixes the prompt into captured output.
///
/// # Errors
/// Whatever the terminal did.
pub fn prompt_password(prompt: &str) -> io::Result<Option<String>> {
    prompt_password_from(prompt, crossterm::event::read)
}

/// [`prompt_password`], reading its keys from `next` instead of from crossterm
/// directly.
///
/// # Why this seam exists
///
/// The Deck can ask for a password too, by suspending itself and prompting on
/// the restored terminal — same function, same raw-mode guarantee, so the rule
/// `main` states ("a password prompt inside raw mode, over a half-drawn Deck, is
/// a much harder thing to get right") is kept rather than worked around. But the
/// Deck already has a thread parked inside `crossterm::event::read`, and **two
/// readers on one stdin is a race for every keystroke**: whichever thread wins
/// gets the character, so half the password would arrive at the fold as key
/// bindings. So the running reader stays the only caller of `event::read`, and
/// while the Deck is suspended it forwards what it reads down a channel that
/// this function drains.
///
/// The password's path is unchanged by any of that: it is built here, in a local
/// buffer, and returned. Nothing echoes it, nothing stores it, and the events
/// travelling the channel are keystrokes, not the assembled secret.
///
/// # Errors
/// Whatever the terminal, or the caller's source, did.
pub fn prompt_password_from<F>(prompt: &str, mut next: F) -> io::Result<Option<String>>
where
    F: FnMut() -> io::Result<Event>,
{
    eprint!("{prompt}");
    io::stderr().flush()?;

    // Raw mode is the whole no-echo mechanism — the terminal stops echoing what
    // is typed, and `apply_key` is the only thing that sees the character. The
    // guard restores it on every exit path including a panic.
    crossterm::terminal::enable_raw_mode()?;
    let _guard = RawGuard;

    let mut buffer = String::new();
    let answer = loop {
        match next()? {
            // Windows sends Press and Release for the same key; acting on both
            // would double every character.
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                match apply_key(&mut buffer, key) {
                    PromptStep::Continue => {}
                    PromptStep::Submit => break Some(buffer),
                    PromptStep::Cancel => break None,
                }
            }
            _ => {}
        }
    };

    drop(_guard);
    eprintln!();
    Ok(answer)
}

/// Read a password from the terminal if there is one, else from a pipe.
///
/// The pipe path is what makes this composable with a password manager —
/// `pass show music | eko-cli login home` — and it is the reason the password is
/// never a command-line *argument*: an argument would be in `~/.zsh_history`
/// before the process even started.
///
/// # Errors
/// Whatever stdin or the terminal did. An empty answer is an error, because a
/// blank password is never what someone meant.
pub fn read_password(prompt: &str) -> io::Result<Option<String>> {
    if io::stdin().is_terminal() {
        return prompt_password(prompt);
    }
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let line = line.trim_end_matches(['\r', '\n']).to_string();
    Ok(if line.is_empty() { None } else { Some(line) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server, ServerGuard};

    /// A `ping` that succeeds, in the exact envelope Navidrome sends.
    const PING_OK: &str =
        r#"{"subsonic-response":{"status":"ok","version":"1.16.1","type":"navidrome"}}"#;

    /// **A wrong password, the way Subsonic actually reports one.**
    ///
    /// HTTP **200**, with the failure in the body: `status: "failed"` and error
    /// code 40. Anything that classified failures by HTTP status would call this
    /// a success.
    const PING_WRONG_PASSWORD: &str = r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password."}}}"#;

    fn server_at(url: &str) -> ServerConfig {
        ServerConfig {
            name: "home".into(),
            base_url: url.to_string(),
            username: "rod".into(),
        }
    }

    fn mock_ping(server: &mut ServerGuard, status: usize, body: &str) -> mockito::Mock {
        server
            .mock("GET", "/rest/ping")
            // The password is never a query param — `t` is an md5 of it plus a
            // fresh salt. Matched by shape because the salt is random.
            .match_query(Matcher::Regex(
                r"u=rod&t=[0-9a-f]{32}&s=[0-9a-z]{10}&v=1\.16\.1&c=eko&f=json".into(),
            ))
            .with_status(status)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    // ── the three cases that need a socket ───────────────────────────────

    #[test]
    fn a_good_password_connects_and_reports_the_user() {
        let mut server = Server::new();
        let mock = mock_ping(&mut server, 200, PING_OK);
        let cfg = server_at(&server.url());

        let connection = connect_with(&cfg, "hunter2".into()).expect("ping succeeds");
        mock.assert();
        assert_eq!(connection.name, "home");
        assert_eq!(connection.username, "rod");
    }

    /// **The one that is easy to get wrong.** Subsonic answers a wrong password
    /// with HTTP 200 and `status: "failed"`, so this must be a failure, and it
    /// must be the *server's* sentence rather than a transport apology.
    #[test]
    fn a_wrong_password_fails_even_though_the_http_status_is_200() {
        let mut server = Server::new();
        let mock = mock_ping(&mut server, 200, PING_WRONG_PASSWORD);
        let cfg = server_at(&server.url());

        let err = connect_with(&cfg, "wrong".into()).expect_err("a wrong password must fail");
        mock.assert();
        assert_eq!(
            err.to_string(),
            "Wrong username or password.",
            "the server's own message is what the user needs to read"
        );
        assert!(
            !err.is_missing_password(),
            "a wrong password is not an absent one"
        );
    }

    /// A host that is not there. The port is closed, so `reqwest` never gets a
    /// response at all — a different code path from every mocked case.
    #[test]
    fn an_unreachable_host_fails_with_a_transport_message() {
        // Port 1 on the loopback: reserved, unbound, and refused immediately, so
        // this does not wait on a DNS timeout.
        let cfg = server_at("http://127.0.0.1:1");
        let err = connect_with(&cfg, "hunter2".into()).expect_err("nothing is listening");
        let message = err.to_string();
        assert!(
            message.starts_with("cannot reach the server"),
            "an unreachable host must read as unreachable: {message}"
        );
        assert!(!err.is_missing_password());
    }

    /// A server that answers, but not with a Subsonic envelope.
    #[test]
    fn something_that_is_not_a_subsonic_server_says_so() {
        let mut server = Server::new();
        let mock = mock_ping(&mut server, 200, "<html>hello</html>");
        let cfg = server_at(&server.url());

        let err = connect_with(&cfg, "hunter2".into()).expect_err("not a Subsonic server");
        mock.assert();
        assert_eq!(err.to_string(), "that does not look like a Subsonic server");
    }

    #[test]
    fn a_non_2xx_status_names_the_status() {
        let mut server = Server::new();
        let mock = mock_ping(&mut server, 502, "bad gateway");
        let cfg = server_at(&server.url());

        let err = connect_with(&cfg, "hunter2".into()).expect_err("502 is not a connection");
        mock.assert();
        assert_eq!(err.to_string(), "the server answered HTTP 502");
    }

    // ── nothing renderable may carry a credential ────────────────────────

    /// **The leak check, on every failure path this module can produce.**
    ///
    /// `eko-net`'s `safe_message` strips the signed URL out of `reqwest` errors,
    /// and `tests/http.rs` pins that. This asserts the property survives
    /// `describe`: no message may contain the password, the username, the base
    /// URL, or the `t=`/`s=` auth pair that a signed URL would drag in.
    #[test]
    fn no_failure_message_carries_a_credential_or_a_signed_url() {
        let mut ok_but_wrong = Server::new();
        let _m1 = mock_ping(&mut ok_but_wrong, 200, PING_WRONG_PASSWORD);
        let mut html = Server::new();
        let _m2 = mock_ping(&mut html, 200, "<html>hello</html>");
        let mut broken = Server::new();
        let _m3 = mock_ping(&mut broken, 502, "bad gateway");

        let cases = [
            ("wrong password", ok_but_wrong.url()),
            ("not a subsonic server", html.url()),
            ("http 502", broken.url()),
            ("unreachable", "http://127.0.0.1:1".to_string()),
        ];

        for (what, url) in cases {
            let cfg = server_at(&url);
            let err = connect_with(&cfg, "hunter2".into()).expect_err("must fail");
            let message = err.to_string();
            for forbidden in ["hunter2", "t=", "s=", "u=rod", &url] {
                assert!(
                    !message.contains(forbidden),
                    "{what}: {forbidden:?} leaked into a renderable message: {message}"
                );
            }
        }
    }

    /// The `Debug` that reaches the fold inside an `AppEvent` cannot print the
    /// client, and therefore cannot print the password inside it.
    #[test]
    fn a_connections_debug_cannot_print_the_password() {
        let mut server = Server::new();
        let _m = mock_ping(&mut server, 200, PING_OK);
        let connection =
            connect_with(&server_at(&server.url()), "hunter2".into()).expect("connects");

        let printed = format!("{connection:?}");
        assert!(
            !printed.contains("hunter2"),
            "Connection's Debug leaked the password: {printed}"
        );
        assert!(printed.contains("<connected>"), "{printed}");
        // …and the same through the event that actually travels the channel.
        let event = ConnEvent::Connected(Box::new(connection));
        assert!(!format!("{event:?}").contains("hunter2"));
    }

    /// **The server writes this text, so the server must not be able to shape
    /// the footer with it.** A newline would split a `ratatui` `Line`; a control
    /// character would go to the terminal as-is.
    #[test]
    fn a_hostile_server_message_cannot_break_the_footer() {
        let mut server = Server::new();
        let nasty = r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"line one\nline two\r\n\u001b[31mred\u0007   spaced"}}}"#;
        let mock = mock_ping(&mut server, 200, nasty);
        let err = connect_with(&server_at(&server.url()), "x".into()).expect_err("failed");
        mock.assert();

        let message = err.to_string();
        assert!(
            !message.contains(['\n', '\r', '\u{1b}', '\u{7}']),
            "a control character survived into a renderable message: {message:?}"
        );
        assert_eq!(message, "line one line two [31mred spaced");
    }

    /// **The gate covers more than `Cc`.**
    ///
    /// A right-to-left override cannot inject an escape sequence, so it is not a
    /// terminal-control hole — but it reverses the visible order of everything
    /// after it, which lets a server decide what a row looks like. `U+2028` and
    /// `U+2029` are line breaks that `char::is_control` does not see at all.
    ///
    /// Grapheme joiners are deliberately left alone: they are legitimate in real
    /// names. See [`is_layout_control`].
    #[test]
    fn tidy_removes_bidi_overrides_and_line_separators_but_not_grapheme_joiners() {
        for (input, expected, what) in [
            ("Zaireeka\u{202e}drawkcab", "Zaireeka drawkcab", "RLO"),
            ("a\u{202a}b\u{202b}c\u{202c}d", "a b c d", "LRE / RLE / PDF"),
            ("a\u{2066}b\u{2069}c", "a b c", "isolates"),
            ("a\u{200e}b\u{200f}c\u{061c}d", "a b c d", "LRM / RLM / ALM"),
            ("one\u{2028}two\u{2029}three", "one two three", "Zl / Zp"),
        ] {
            assert_eq!(tidy(input), expected, "{what} survived tidy");
        }
        // …and the joiners that make real names render correctly are untouched.
        for keep in ["\u{200d}", "\u{200c}", "\u{feff}"] {
            let name = format!("a{keep}b");
            assert_eq!(tidy(&name), name, "{keep:?} was stripped from a real name");
        }
    }

    #[test]
    fn an_enormous_server_message_is_capped() {
        let long = "a".repeat(4_000);
        let tidied = tidy(&long);
        assert_eq!(tidied.chars().count(), MAX_MESSAGE_LEN + 1);
        assert!(tidied.ends_with('…'));
    }

    /// **The leak this project has already fixed twice, on the one path where
    /// the message is not ours to write.**
    ///
    /// `SubsonicError::Subsonic` is the server's own sentence, rendered verbatim
    /// in the footer. A server, a proxy or a middlebox that echoes the
    /// credential back — "authentication failed for rod:hunter2" — would put it
    /// on screen, and `describe` cannot know that because it has never seen the
    /// password. `connect_with` has both, so `connect_with` checks.
    #[test]
    fn a_server_that_echoes_the_password_back_cannot_put_it_in_the_footer() {
        let mut server = Server::new();
        let echoed = r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"login failed for rod:hunter2 (hunter2)"}}}"#;
        let mock = mock_ping(&mut server, 200, echoed);
        let err = connect_with(&server_at(&server.url()), "hunter2".into())
            .expect_err("a rejected login");
        mock.assert();

        let message = err.to_string();
        assert!(
            !message.contains("hunter2"),
            "the password survived into a renderable message: {message}"
        );
        assert_eq!(message, "login failed for rod:····· (·····)");
    }

    /// An empty secret must not match everywhere and turn every message into
    /// dots.
    #[test]
    fn redaction_of_an_empty_secret_leaves_the_message_alone() {
        assert_eq!(without_password("anything".into(), ""), "anything");
    }

    // ── the base URL people actually paste ───────────────────────────────

    #[test]
    fn a_clean_base_url_is_taken_as_typed() {
        let parsed = normalise_base_url("https://music.example.com").unwrap();
        assert_eq!(parsed.url, "https://music.example.com");
        assert!(parsed.fixes.is_empty(), "{:?}", parsed.fixes);
        // A path is legitimate: Navidrome behind a reverse proxy often has one.
        let proxied = normalise_base_url("https://example.com/music").unwrap();
        assert_eq!(proxied.url, "https://example.com/music");
        assert!(proxied.fixes.is_empty());
    }

    /// The two corrections, **said out loud**. Silently changing what someone
    /// typed is indistinguishable from ignoring it.
    #[test]
    fn a_trailing_slash_and_a_trailing_rest_are_removed_and_reported() {
        let slash = normalise_base_url("https://music.example.com/").unwrap();
        assert_eq!(slash.url, "https://music.example.com");
        assert_eq!(slash.fixes, ["dropped the trailing /"]);

        // `eko-net` builds `{base}/rest/ping`, so this would be `/rest/rest/ping`
        // and a 404 that reads as a working server refusing us.
        for pasted in [
            "https://music.example.com/rest",
            "https://music.example.com/rest/",
            "https://music.example.com/REST",
        ] {
            let parsed = normalise_base_url(pasted).unwrap();
            assert_eq!(parsed.url, "https://music.example.com", "{pasted}");
            assert!(
                parsed.fixes.iter().any(|f| f.contains("/rest")),
                "{pasted}: {:?}",
                parsed.fixes
            );
        }
        // …and a host that merely *contains* "rest" is not a path to strip.
        let forest = normalise_base_url("https://forest.example.com").unwrap();
        assert_eq!(forest.url, "https://forest.example.com");
        assert!(forest.fixes.is_empty(), "{:?}", forest.fixes);
    }

    /// A scheme is required rather than guessed: choosing `https` for someone
    /// would be this client picking their transport security, and choosing
    /// `http` would be worse.
    #[test]
    fn a_url_with_no_scheme_is_refused_with_the_fix_in_the_message() {
        let err = normalise_base_url("music.example.com").unwrap_err();
        assert_eq!(err, "no scheme — try https://music.example.com");
        assert!(normalise_base_url("ftp://music.example.com").is_err());
        assert!(normalise_base_url("   ").is_err());
        assert!(normalise_base_url("https://").is_err());
    }

    /// The field is echoed in the panel, so a pasted escape sequence must not
    /// survive into the message that quotes it back.
    #[test]
    fn a_hostile_paste_cannot_reach_the_screen_through_the_error() {
        let err = normalise_base_url("music\u{1b}[2J.example.com").unwrap_err();
        assert!(!err.contains('\u{1b}'), "{err:?}");
    }

    // ── the prompt, driven from somewhere other than crossterm ───────────

    /// The Deck's suspend-and-prompt path uses **this** function, so the buffer
    /// is assembled by the same code and returned the same way. Nothing here
    /// prints it, which is the property the whole design rests on.
    #[test]
    fn a_prompt_fed_from_a_channel_assembles_the_same_password() {
        let mut keys = "hunter2"
            .chars()
            .map(|c| Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)))
            .chain(std::iter::once(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))))
            .collect::<Vec<_>>()
            .into_iter();
        let mut buffer = String::new();
        // The loop `prompt_password_from` runs, without the terminal: the
        // function itself needs raw mode, and `cargo test` has no tty.
        for event in keys.by_ref() {
            if let Event::Key(key) = event {
                match apply_key(&mut buffer, key) {
                    PromptStep::Continue => {}
                    PromptStep::Submit | PromptStep::Cancel => break,
                }
            }
        }
        assert_eq!(buffer, "hunter2");
    }

    // ── the keychain account ─────────────────────────────────────────────

    /// **The confused-deputy guard.** A config file cannot address another
    /// secret: the account is always the fixed prefix plus a whitelisted name.
    #[test]
    fn a_name_cannot_escape_the_account_prefix() {
        for bad in [
            "",
            "eko.license.key", // rejected only because of the escape below…
            "../eko.license.key",
            "home/../licence",
            "home key",
            "home\nkey",
            "home:key",
            "hôme",
            &"a".repeat(MAX_NAME_LEN + 1),
        ] {
            if bad == "eko.license.key" {
                // This one IS in the charset — it is caught by the *prefix*, not
                // by the filter, which is why the prefix is not decoration.
                let account = account_for(bad).expect("in-charset");
                assert_eq!(account, "server.eko.license.key");
                assert_ne!(account, "eko.license.key");
                continue;
            }
            assert!(
                account_for(bad).is_err(),
                "{bad:?} was accepted as a keychain account"
            );
        }
        assert_eq!(account_for("home").unwrap(), "server.home");
        assert!(account_for("home").unwrap().starts_with(ACCOUNT_PREFIX));
    }

    /// The service is ours, not the desktop's shared one.
    #[test]
    fn the_keychain_service_is_not_the_desktop_apps() {
        assert_eq!(KEYCHAIN_SERVICE, "com.reactivepixels.eko.cli");
        assert_ne!(
            KEYCHAIN_SERVICE, "com.reactivepixels.eko",
            "the CLI must not share the namespace holding eko.license.key"
        );
    }

    #[test]
    fn a_bad_name_is_reported_before_any_keychain_call() {
        let err = password("has a space").unwrap_err();
        assert!(matches!(err, ServerError::BadName(_)), "{err:?}");
        assert!(err.to_string().contains("letters, digits"), "{err}");
    }

    // ── the config shape ─────────────────────────────────────────────────

    #[test]
    fn a_server_table_parses_and_url_is_accepted_as_an_alias() {
        let a: ServerConfig = toml::from_str(
            r#"
            name = "home"
            base_url = "https://music.example.com"
            username = "rod"
            "#,
        )
        .unwrap();
        let b: ServerConfig = toml::from_str(
            r#"
            name = "home"
            url = "https://music.example.com"
            username = "rod"
            "#,
        )
        .unwrap();
        assert_eq!(a, b);
        assert!(a.is_complete());
    }

    #[test]
    fn a_half_written_server_is_not_a_server() {
        let partial = ServerConfig {
            name: "home".into(),
            base_url: "  ".into(),
            username: "rod".into(),
        };
        assert!(!partial.is_complete());
    }

    // ── the password prompt ──────────────────────────────────────────────

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn typing_accumulates_and_enter_submits() {
        let mut buffer = String::new();
        for c in "hunter2".chars() {
            assert_eq!(
                apply_key(&mut buffer, press(KeyCode::Char(c))),
                PromptStep::Continue
            );
        }
        assert_eq!(buffer, "hunter2");
        assert_eq!(
            apply_key(&mut buffer, press(KeyCode::Enter)),
            PromptStep::Submit
        );
    }

    #[test]
    fn backspace_removes_a_character_not_a_byte() {
        let mut buffer = String::new();
        for c in "pä".chars() {
            assert_eq!(
                apply_key(&mut buffer, press(KeyCode::Char(c))),
                PromptStep::Continue
            );
        }
        assert_eq!(
            apply_key(&mut buffer, press(KeyCode::Backspace)),
            PromptStep::Continue
        );
        assert_eq!(buffer, "p", "backspace split a multi-byte character");
        // …and on an empty buffer it is inert rather than a panic.
        for _ in 0..2 {
            assert_eq!(
                apply_key(&mut buffer, press(KeyCode::Backspace)),
                PromptStep::Continue
            );
        }
        assert!(buffer.is_empty());
    }

    /// Raw mode swallows SIGINT, so Ctrl-C has to be a binding or the prompt is
    /// a trap.
    #[test]
    fn esc_and_ctrl_c_both_cancel_and_ctrl_u_clears() {
        let mut buffer = "secret".to_string();
        assert_eq!(
            apply_key(&mut buffer, press(KeyCode::Esc)),
            PromptStep::Cancel
        );
        assert_eq!(apply_key(&mut buffer, ctrl('c')), PromptStep::Cancel);
        assert_eq!(apply_key(&mut buffer, ctrl('u')), PromptStep::Continue);
        assert!(buffer.is_empty());
    }

    /// A control chord must never be typed *into* the password.
    #[test]
    fn control_chords_are_not_characters() {
        let mut buffer = String::new();
        for key in [ctrl('a'), press(KeyCode::F(5)), press(KeyCode::Left)] {
            assert_eq!(apply_key(&mut buffer, key), PromptStep::Continue);
        }
        assert!(
            buffer.is_empty(),
            "a chord landed in the password: {buffer:?}"
        );
    }
}
