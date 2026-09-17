//! Application state and the event fold.
//!
//! The main thread does two things and nothing else: fold events into [`App`],
//! and draw. Input arrives on its own channel; the library scan arrives on
//! another as [`AppEvent::Scan`]. In later tasks network work and keychain
//! calls arrive the same way.
//!
//! ## The engine is not async, and must not become async
//!
//! [`App`] owns one [`Engine`] for the life of the process. `eko-core`'s engine
//! is **synchronous**: it spawns its own decode/output thread per session and
//! is driven by plain blocking calls. It must never be driven from inside a
//! tokio runtime — `eko-core` reaches `reqwest::blocking` on the streaming path,
//! and `reqwest::blocking` panics when constructed on a runtime worker. Earlier
//! work in this project hit exactly that.
//!
//! `eko-cli` has no async runtime at all, which makes the constraint free
//! today. If one is ever added — for the OpenSubsonic client, say — the engine
//! calls have to stay on a plain thread, `spawn_blocking` or otherwise off the
//! reactor.

use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::Rect;

use eko_core::engine::{Engine, EngineStatus};
use eko_core::signal_path::{self, RgMode, SealInput, SignalPath, StreamInfo};

use crate::art;
use crate::config::{self, Config, MusicFolder, OutputDevice};
use crate::eq;
use crate::keys::{self, Action, SEEK_STEP_SECS};
use crate::library::{self, Library, ScanEvent};
use crate::queue::{Entry, Media, Queue, NO_ROW};
use crate::remote::{self, RemoteEvent, RemoteKind, SearchEvent, SearchKind};
use crate::server::{self, ConnEvent, Connection, ServerConfig};
use crate::ui::theme::Theme;
use crate::wave;

/// ~30fps. The ceiling, not the norm — see [`tick_interval`].
pub const FRAME_INTERVAL: Duration = Duration::from_millis(33);

/// 1Hz. Enough to move an elapsed clock, cheap enough to ignore.
pub const CLOCK_INTERVAL: Duration = Duration::from_secs(1);

/// Everything that can move the application forward.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A key, a resize, a paste — straight from crossterm.
    Input(Event),
    /// The adaptive tick fired. Nothing else generates these.
    Tick,
    /// Progress or a result from the library scanner thread.
    Scan(ScanEvent),
    /// The result of a connection attempt, from the connector thread.
    ///
    /// `Debug` on this reaches every `{event:?}`; [`Connection`]'s own `Debug`
    /// is hand-written so a derived one here cannot print the password. See
    /// [`crate::server::Connection`].
    Conn(ConnEvent),
    /// A page of albums, or one album's tracks, from a remote worker.
    ///
    /// Carries no client and therefore no credential — see [`crate::remote`].
    Remote(RemoteEvent),
    /// What a search matched, from a search worker.
    ///
    /// Its own variant rather than another [`RemoteKind`], because it is stamped
    /// against its own counter — see [`crate::remote::SearchEvent`].
    Search(SearchEvent),
    /// One cover, fetched, decoded and encoded on a worker.
    ///
    /// Stamped against its own counter for the same reason [`SearchEvent`] is,
    /// and carrying no URL: the signed one it was fetched with lived on the
    /// worker thread and was dropped there. Its `Debug` prints a byte count
    /// rather than the payload — see [`crate::art::Escape`].
    Art(art::Event),
    /// One track's waveform envelope, decoded on a worker.
    ///
    /// Stamped against its own counter, exactly as [`AppEvent::Art`] is, and for
    /// exactly the same reason: a decode that takes half a second for a track
    /// that has since been skipped must be dropped rather than drawn. See
    /// [`crate::wave`].
    Wave(wave::Event),
}

/// Transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Playback {
    #[default]
    Stopped,
    Paused,
    Playing,
}

/// Which pane has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    Sidebar,
    /// The library. The default, because that is what you came here to use.
    #[default]
    Main,
}

/// What the main pane is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Albums,
    /// One album's tracks, by index into [`Library::albums`].
    Album(usize),
}

/// Where the library scan has got to.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ScanState {
    /// Nowhere to look: no `music_folder` in the config **and** no platform
    /// music directory to fall back to. Not an error — but not a state to sit
    /// in silently either, because nothing will ever arrive on its own.
    #[default]
    Unconfigured,
    /// Walking the tree.
    Discovering {
        found: usize,
    },
    /// Reading tags.
    Reading {
        done: usize,
        total: usize,
    },
    /// Finished. The library may still be empty; that is a normal outcome.
    Ready,
    /// The folder we were told to read is not there. Carries the path, because
    /// the path is the fix.
    Missing(std::path::PathBuf),
    Failed(String),
}

impl ScanState {
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Discovering { .. } | Self::Reading { .. })
    }
}

/// Where the connection to the configured server has got to.
///
/// Five states, because they have five different fixes and a user has to be able
/// to tell them apart **at a glance**. The previous phase learned this the hard
/// way: an unconfigured Deck that looked identical to a working one was read as
/// a broken one. So the sidebar carries a one-word state in the instrument-lamp
/// colours ([`crate::ui::sidebar`]) and the footer carries the sentence.
///
/// Pure data on purpose — no [`Connection`] in here. The live client lives in
/// [`App::connection`], written in the same place this is, so the two cannot
/// drift; keeping it out means this stays `PartialEq` and every state can be
/// stated in a test rather than connected to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnState {
    /// No `[[servers]]` in `config.toml`. Nothing is wrong; nothing was asked
    /// for.
    #[default]
    NotConfigured,
    /// A server is configured and the keychain has no password for it. The
    /// first-run state, and the only one with a command as its fix.
    NeedsPassword { name: String },
    /// A worker is talking to it right now.
    Connecting { name: String },
    /// Reached, and the credentials were accepted.
    Connected { name: String, username: String },
    /// Everything else, carrying the real message — see
    /// [`crate::server::ServerError`], none of whose variants can contain a
    /// credential or a signed URL.
    Failed { name: String, message: String },
}

impl ConnState {
    /// A worker is in flight. Guards against stacking connectors on `r`, the
    /// same way [`ScanState::is_running`] guards the scanner.
    #[must_use]
    pub fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting { .. })
    }
}

/// How far the server's album list has got.
///
/// The twin of [`ScanState`], and it exists for the same reason: "empty",
/// "loading" and "broken" are three different screens with three different fixes,
/// and a list that is merely blank cannot tell them apart.
///
/// Note that [`RemoteState::Failed`] does **not** discard the pages that already
/// arrived. A walk can deliver four pages and then lose the network; showing what
/// there is *and* saying it is incomplete is honest, and throwing it away is not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RemoteState {
    /// Nothing has been asked for — no live connection to ask through.
    #[default]
    Idle,
    /// A page walk is in flight. Albums appear as pages land.
    Loading,
    /// The walk finished. The library may legitimately be empty.
    Ready,
    /// The walk stopped early, carrying the reason. Whatever arrived stands.
    Failed(String),
}

/// How the open remote album's track fetch is going.
///
/// Only one album is open at a time, so this needs no key. Loaded tracks live on
/// the album itself ([`crate::remote::Album::tracks`]); this is only the state of
/// the request that is fetching them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DetailState {
    /// Nothing outstanding: either no album is open, or its tracks are in hand.
    #[default]
    Idle,
    Loading,
    Failed(String),
}

/// What the footer is playing.
///
/// # `source` is the field that stops the wrong track playing
///
/// `album` and `track` are **indices**, and there are now two libraries to index.
/// Index 3 of the server's album list and index 3 of the scanned folder are
/// unrelated records; a transport that held only the pair would resolve either
/// one against whichever list it happened to reach for. That is not hypothetical
/// — until this task `App::play` always indexed [`App::library`], and the only
/// thing standing between a remote album list and a completely unrelated local
/// track playing was a pair of "not wired yet" guards.
///
/// So the source travels **with** the indices, in the one struct that records
/// what is playing, and every consumer — `play`, `step_track`, the `▶` marker in
/// both lists — reads it from here rather than from wherever the cursor happens
/// to be. The cursor can move to the other source mid-track; what is playing
/// cannot change because of that.
#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    /// Which library `album` and `track` index. See the type docs.
    pub source: Source,
    pub album: usize,
    pub track: usize,
    pub title: String,
    pub artist: String,
    pub album_name: String,
    /// Duration from the file's tags, in ms. The engine overwrites this with
    /// the decoded length once a session reports one.
    pub dur_ms: u64,
}

/// How long the event loop should wait for input before synthesising a
/// [`AppEvent::Tick`].
///
/// `None` means "block until something happens" — the loop is idle and redraws
/// on change only.
///
/// This is the whole adaptive-tick decision, extracted so it is a pure function
/// with a table of cases rather than a sleep buried in a loop. A TUI that
/// redraws unconditionally spins a core, and spending a core beside a realtime
/// audio thread is precisely the wrong trade for this application: the audio
/// callback has a hard deadline and the spectrum does not.
///
/// | playback | bars on screen | interval |
/// |---|---|---|
/// | playing | yes | ~30fps — the bars have to move |
/// | playing | no  | 1Hz — only the elapsed clock changes |
/// | paused / stopped | either | none — block on input |
///
/// `bars` is *anything animated by the FFT*: the footer's spectrum, or the
/// visualiser's analyser. It used to be `spectrum_visible` alone, and it is now
/// [`App::animating`] — because a `z` view with the footer spectrum turned off
/// is still thirty-two bars that have to move, and a `z` view with the analyser
/// configured off is a big cover and a clock that do not. Reading one of the two
/// flags would have made the tick rate wrong in both directions.
///
/// The scanner is not in this table on purpose: it pushes its own progress
/// events into the same channel, so a scanning-but-idle Deck still blocks on
/// `recv` and still wakes exactly when there is something new to draw.
#[must_use]
pub fn tick_interval(playback: Playback, bars: bool) -> Option<Duration> {
    match playback {
        Playback::Playing if bars => Some(FRAME_INTERVAL),
        Playback::Playing => Some(CLOCK_INTERVAL),
        Playback::Paused | Playback::Stopped => None,
    }
}

/// The seal's view of a live [`EngineStatus`], or `None` when the engine has not
/// yet reported enough to derive one from.
///
/// # Why this gate has to exist
///
/// `Engine::play` creates the session *before* the decode thread has opened
/// either the file or the output device, and `Engine::status` reports
/// `rate: sh.rate.load(..).max(1)` — so a session that has only just started, or
/// one whose file could not be opened at all, still answers with a `Some` whose
/// `rate` is `1` and whose `src_rate`, `dev_rate` and `bits` are all `0`.
///
/// Handed straight to [`signal_path::derive`] that snapshot reads as
/// **`BIT-PERFECT`**: `active` is true because `rate != 0`, and both resample
/// checks are guarded by `src_rate > 0` / `dev_rate > 0` and so silently pass.
/// That is exactly the shape of the three false-`BIT-PERFECT` bugs already found
/// in the desktop app — a missing field defaulting to "nothing touched the
/// samples" rather than to "I do not know".
///
/// So the seal only sees a status once **every rate it reads** is populated. The
/// engine stores `rate`, `src_rate`, `dev_rate`, `bits` and `codec` in one place
/// (`decode_and_play`, after the device is open), so this is one edge, not three.
/// `bits`, `codec` and `device` are display-only and are not gated on — they
/// cannot move the seal.
#[must_use]
pub fn stream_info(status: &EngineStatus) -> Option<StreamInfo> {
    if status.rate == 0 || status.src_rate == 0 || status.dev_rate == 0 {
        return None;
    }
    Some(StreamInfo::from(status))
}

/// Which library a source row browses.
///
/// The **only** thing that decides what the main pane is showing: it is read off
/// the selected row rather than stored beside it, so the sidebar cursor and the
/// list can never disagree about which source is open. Moving the sidebar cursor
/// *is* switching source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// The scanned music folder. See [`crate::library`].
    #[default]
    Local,
    /// The configured Navidrome / OpenSubsonic server. See [`crate::remote`].
    Remote,
    /// The play queue. See [`crate::queue`].
    ///
    /// Not a library — it is a *view of the transport*, and it is the only source
    /// whose rows can hold tracks from both of the others at once. It is here
    /// rather than in a pane of its own because it is browsed with exactly the
    /// same keys, and because a queue nobody can look at is a queue nobody can
    /// trust.
    ///
    /// [`NowPlaying::source`] can never be this: a queue entry's source is read
    /// off its own [`crate::queue::Media`], so what is playing is always named as
    /// `Local` or `Remote`.
    Queue,
    /// What the server matched for a query. See [`Search`].
    ///
    /// Also not a library: it is a *question about* one, and its rows are the
    /// server's answer rather than anything this client walked. Like [`Self::Queue`]
    /// it is here rather than in a mode of its own because it is browsed with
    /// exactly the same keys — the results list has a cursor, `Enter`, `a` and
    /// `esc`, and none of them are spelled twice.
    ///
    /// [`NowPlaying::source`] can never be this either: a search result is
    /// streamed from the server, so an entry made here reports `Remote`. What it
    /// cannot report is a *position*, because a result is at no index in either
    /// library — see [`crate::queue::NO_ROW`].
    Search,
}

/// One row of the source sidebar.
///
/// **Every row in [`App::sources`] works.** A row here is selectable, focusable
/// and openable, so putting one in the list is a claim that it does something.
/// A view that is designed but not wired is not a row and is not named
/// anywhere — see the note on the `NOT YET` section in [`crate::ui::sidebar`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRow {
    pub label: String,
    /// Which library this row browses.
    pub source: Source,
    /// 0 = a root source, 1 = one of its views.
    pub depth: u8,
    /// Whether this row discloses children. See [`SourceRow::root`].
    pub expandable: bool,
    pub expanded: bool,
    /// Right-aligned count, when the row has real data behind it.
    pub badge: Option<String>,
    /// Whether this row is an **invitation** rather than a library.
    ///
    /// Exactly one row is ever this: `+ Add server`, which stands in the server's
    /// place while there is no server. It is drawn with a `+` instead of a lamp
    /// — a lamp answers "is there something behind this row", and the honest
    /// answer for this one is "not yet, and that is what it is for" — and `enter`
    /// on it opens the setup panel instead of disclosing children.
    ///
    /// It carries [`Source::Remote`] so that nothing else in the crate has to
    /// learn a new source: every `match` on [`Source`] keeps its arms, the main
    /// pane already has a not-configured state to draw, and the day a server is
    /// added the row simply stops being an invitation.
    pub action: bool,
}

impl SourceRow {
    /// A root source with nothing under it — the whole source *is* the row.
    fn source(label: &str, source: Source) -> Self {
        Self {
            label: label.to_string(),
            source,
            depth: 0,
            expandable: false,
            expanded: false,
            badge: None,
            action: false,
        }
    }

    /// The row that offers to configure a source that is not there. See
    /// [`SourceRow::action`].
    fn invitation(label: &str, source: Source) -> Self {
        Self {
            action: true,
            ..Self::source(label, source)
        }
    }

    /// A root source that discloses children.
    ///
    /// No shipped source has any: `Local` is a leaf, and a caret that opens onto
    /// nothing is a control that does nothing. The disclosure machinery —
    /// [`App::visible_sources`] and `App::toggle_expand` — is general and stays
    /// under test through this constructor, because Phase 1b-ii's Subsonic
    /// source has Albums / Artists / Lists beneath it.
    #[cfg(test)]
    fn root(label: &str, expanded: bool) -> Self {
        Self {
            label: label.to_string(),
            source: Source::Local,
            depth: 0,
            expandable: true,
            expanded,
            badge: None,
            action: false,
        }
    }

    /// One view beneath a disclosing root. See [`SourceRow::root`].
    #[cfg(test)]
    fn child(label: &str) -> Self {
        Self {
            label: label.to_string(),
            source: Source::Local,
            depth: 1,
            expandable: false,
            expanded: false,
            badge: None,
            action: false,
        }
    }
}

/// How far a query has got.
///
/// The twin of [`RemoteState`], and it exists for the same reason: "nobody has
/// asked", "asking", "asked and matched nothing" and "the ask failed" are four
/// different screens with four different next moves, and a blank list tells them
/// apart from none of the others.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SearchState {
    /// No query has been run. Not an error — nothing was asked for.
    #[default]
    Idle,
    /// A request is in flight. The previous results, if any, still stand until
    /// it answers.
    Searching,
    /// The server answered. It may legitimately have matched nothing.
    Ready,
    /// The request failed, carrying the reason.
    Failed(String),
}

/// The longest query this client will send.
///
/// Not a security boundary — the server would refuse or ignore a silly one — but
/// the query is echoed in the pane's summary row and in its empty-state
/// sentences, and an unbounded string there is an unbounded line to lay out.
pub const MAX_QUERY: usize = 96;

/// The search pane: what was asked, what came back, and what is being typed.
///
/// # Why it is a struct and not six fields on [`App`]
///
/// Every one of these has to move **together**. A new query has to bump the
/// generation, park the state at `Searching`, and put the cursor back at the top
/// in the same breath; leaving them loose beside `remote_*` invites exactly the
/// drift [`App::view`] and [`App::source`] are written to avoid.
///
/// # The input is `Option<String>`, and that is the whole "mode"
///
/// `Some` means the line editor has the keyboard, `None` means the keymap does.
/// There is no `mode: Mode` enum, no stack, and nothing else in the application
/// branches on it: [`App::on_key`] offers the key to the editor and falls
/// through. `query` is what was last *run*; `input` is what is being *typed*, so
/// cancelling with `esc` restores the results already on screen rather than
/// clearing them.
#[derive(Debug, Clone, Default)]
pub struct Search {
    /// The query the results below came from. Empty until one has been run.
    pub query: String,
    /// The line being typed, or `None` when the input is closed.
    pub input: Option<String>,
    pub state: SearchState,
    /// What the server matched. Sanitised at the boundary — see
    /// [`crate::remote::Results`].
    pub results: remote::Results,
    /// The pane's own view and cursors, for the same reason the remote source has
    /// its own: an index into a result list is not an index into anything else.
    pub view: View,
    pub cursor: usize,
    pub track_cursor: usize,
    /// How a result album's track fetch is going.
    pub detail: DetailState,
    /// Which query the fold is currently listening to. See
    /// [`crate::remote::SearchEvent`].
    generation: u64,
}

impl Search {
    /// The result album the pane has open, if any.
    #[must_use]
    pub fn open_album(&self) -> Option<&remote::Album> {
        match self.view {
            View::Album(i) => self.results.albums.get(i),
            View::Albums => None,
        }
    }

    /// The song at flat row `row` of the results list, if that row is a song.
    ///
    /// The list is albums first, then songs, so a row past the albums is a song
    /// at `row - albums.len()`. One function, because three call sites doing that
    /// subtraction by hand is three chances to do it differently.
    #[must_use]
    pub fn song_at(&self, row: usize) -> Option<(usize, &remote::Track)> {
        let index = row.checked_sub(self.results.albums.len())?;
        self.results.songs.get(index).map(|song| (index, song))
    }
}

/// The three things the add-server panel asks for, in the order it asks.
///
/// **There is no fourth.** The password is not a field here, it is not a field
/// anywhere in this crate's state, and [`AddServer`] documents why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Field {
    #[default]
    Name,
    BaseUrl,
    Username,
}

impl Field {
    /// Every field, in order — the panel draws them from this, so a field cannot
    /// be added to the type and forgotten by the renderer.
    pub const ALL: [Self; 3] = [Self::Name, Self::BaseUrl, Self::Username];

    /// The label the panel writes.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::BaseUrl => "url",
            Self::Username => "user",
        }
    }

    /// What this field is for, in one short line under the panel.
    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::Name => "what to call it here · letters, digits, . _ -",
            Self::BaseUrl => "https://music.example.com",
            Self::Username => "your Navidrome user",
        }
    }

    /// The widest value this field will take, in **terminal columns** — the
    /// same unit [`crate::ui::server_panel`] sizes itself in and
    /// [`crate::line::columns`] measures.
    ///
    /// `name` is bounded by the keychain account rule; the other two are bounded
    /// so the panel cannot be widened past the pane by a paste.
    ///
    /// **Columns and not characters**, which is the whole of that last sentence
    /// actually being true. These were characters while the panel's width was
    /// columns, so sixty pasted CJK characters made a 120-column value inside a
    /// 38-column field — and the panel, unable to fit, drew nothing while
    /// [`crate::app::App::add`] stayed `Some` and swallowed every key. Note that
    /// `name` is *also* checked against [`crate::server::is_valid_name`], whose
    /// charset admits no wide character at all; the cap is the layout's
    /// guarantee, not the validator's.
    #[must_use]
    pub fn max(self) -> usize {
        match self {
            Self::Name => server::MAX_NAME_LEN,
            Self::BaseUrl => 120,
            Self::Username => 64,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Name => Self::BaseUrl,
            Self::BaseUrl => Self::Username,
            Self::Username => Self::Name,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Name => Self::Username,
            Self::BaseUrl => Self::Name,
            Self::Username => Self::BaseUrl,
        }
    }
}

/// The add-server panel: three lines of text and what is wrong with them.
///
/// # There is no password in here, and there cannot be
///
/// This struct is the whole of what the panel holds, it lives on [`App`], and
/// `App` is what every frame is drawn from. A password field here would be a
/// password in the ratatui buffer the moment anybody typed into it — and in every
/// `{app:?}`, every snapshot, and every future crash report. So the panel
/// collects the three things that go in `config.toml` and *stops*; the password
/// is asked for afterwards, on the restored terminal, by `main` — see
/// [`Prompt`] — and goes from that prompt to the keychain without passing
/// through the fold at all.
///
/// `tests::a_password_can_never_reach_a_rendered_frame` is the assertion.
#[derive(Debug, Clone, Default)]
pub struct AddServer {
    pub name: String,
    pub base_url: String,
    pub username: String,
    /// Which line the keyboard is on.
    pub field: Field,
    /// What is wrong, shown against the field it is wrong about. Validation
    /// happens here, in front of the user, rather than at save time where the
    /// only thing left to do about it is a footer note.
    pub error: Option<(Field, String)>,
    /// Whether a server is already configured — in which case this panel is
    /// adding a *second* one, which the file will hold and this phase will not
    /// use. Said in the panel rather than discovered afterwards.
    pub second: bool,
}

impl AddServer {
    /// The value of one field.
    #[must_use]
    pub fn value(&self, field: Field) -> &str {
        match field {
            Field::Name => &self.name,
            Field::BaseUrl => &self.base_url,
            Field::Username => &self.username,
        }
    }

    fn value_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Name => &mut self.name,
            Field::BaseUrl => &mut self.base_url,
            Field::Username => &mut self.username,
        }
    }

    /// The message to draw under `field`, if it is the one that is wrong.
    #[must_use]
    pub fn error_for(&self, field: Field) -> Option<&str> {
        match &self.error {
            Some((at, message)) if *at == field => Some(message.as_str()),
            _ => None,
        }
    }

    /// Check the three fields and build the server, or say which one is wrong.
    ///
    /// Pure, so every rule is a case in a test rather than something found by
    /// typing into a panel. The `name` rule is [`server::is_valid_name`] — the
    /// same predicate the keychain account is minted through — rather than a
    /// second copy of the charset that could drift from it.
    ///
    /// # `taken` is not a nicety, it is the guard on a working credential
    ///
    /// A name is a **keychain account**, and the keychain has one entry per
    /// account. Adding a second server under a name the config already holds
    /// therefore ran `set_password(name, new)` over the *working* password of
    /// the existing one; the duplicate `[[servers]]` table that followed was
    /// then dropped by [`crate::config::Config::normalized`] on the next launch
    /// — "one name, one password", the rule that already exists for a
    /// hand-edited file. Net: the server that used to sign in stopped, and the
    /// one that was typed in was never kept.
    ///
    /// So the same rule is enforced **here**, in front of the person typing,
    /// before anything reaches the keychain. Exact comparison rather than
    /// case-insensitive, so this refuses precisely what `normalized` drops and
    /// not one name more.
    pub fn validate(
        &self,
        taken: &[String],
    ) -> Result<(ServerConfig, Vec<String>), (Field, String)> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err((Field::Name, "a name is needed".to_string()));
        }
        if !server::is_valid_name(&name) {
            return Err((
                Field::Name,
                format!(
                    "letters, digits, . _ - only, up to {} characters",
                    server::MAX_NAME_LEN
                ),
            ));
        }
        // Thirty-eight columns exactly, which is `server_panel::VALUE_W` — the
        // note row is the widest thing this rule can put on screen, and a
        // message the panel has to ellipsise is a message with the fix cut off
        // the end of it. The name itself is left out because it is on the line
        // directly above.
        if taken.iter().any(|existing| existing == &name) {
            return Err((
                Field::Name,
                "already configured · pick another name".to_string(),
            ));
        }
        let base = server::normalise_base_url(&self.base_url)
            .map_err(|message| (Field::BaseUrl, message))?;
        let username = self.username.trim().to_string();
        if username.is_empty() {
            return Err((Field::Username, "a username is needed".to_string()));
        }
        Ok((
            ServerConfig {
                name,
                base_url: base.url,
                username,
            },
            base.fixes,
        ))
    }
}

/// What the fold is asking `main` to do **outside** the terminal takeover.
///
/// One job, in two shapes, and neither of them can carry a secret: the fields
/// are a [`ServerConfig`], which has nowhere to put a password. `main` suspends
/// the Deck, prompts on the restored terminal with
/// [`server::prompt_password_from`], hands the answer straight to the keychain,
/// and gives the fold back a [`PromptOutcome`] — which also has nowhere to put
/// one.
///
/// That asymmetry is the design. The password crosses no boundary this module
/// can see, so no amount of getting the UI wrong can render it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// A server that is already in `config.toml` and has no stored password —
    /// or whose stored password the server refused. Nothing is written to the
    /// file; only the keychain changes.
    SignIn(ServerConfig),
    /// A server that has just been typed into the panel. The keychain is
    /// written first and `config.toml` second, so a cancelled prompt leaves the
    /// file exactly as it was rather than adding an entry nobody can sign in to.
    Add(ServerConfig),
}

impl Prompt {
    /// The server the prompt is about.
    #[must_use]
    pub fn server(&self) -> &ServerConfig {
        match self {
            Self::SignIn(server) | Self::Add(server) => server,
        }
    }
}

/// How a [`Prompt`] ended. **Carries no password**, by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptOutcome {
    /// It worked. The server is the one to connect to now, and `notes` is
    /// whatever had to be said about it — a corrected URL, a file that could not
    /// be written, the fact that this is a second server.
    Stored {
        server: ServerConfig,
        notes: Vec<String>,
    },
    /// Esc or Ctrl-C at the prompt. Nothing was stored and nothing was written.
    Cancelled,
    /// The keychain or the config file refused. The message is safe to render.
    Failed(String),
}

/// The sleep timer's steps, in minutes. `t` walks them and then switches off.
///
/// Four, and no free-form entry: the gesture is "keep going a bit longer", not
/// "compute a deadline". A picker for this would be a second modal for a value
/// with four options.
pub const SLEEP_STEPS: &[u64] = &[15, 30, 45, 60];

/// A running sleep timer.
///
/// # Why the remaining time is folded rather than derived at render time
///
/// The obvious shape is an `Instant` deadline and a `deadline - now()` in the
/// footer. That puts a clock read inside the render, which makes the drawn frame
/// a function of the wall clock rather than of [`App`] — every snapshot test of a
/// Deck with a timer up would then depend on how long the test took to reach the
/// assertion. So the countdown is state: [`App::tick_sleep`] brings it down by
/// the time that has really passed, and the footer prints what it finds.
///
/// It counts down **only while something is playing**, which is also why the
/// elapsed time is measured rather than counted in ticks: the tick rate is 30fps
/// with the spectrum up and 1Hz without it, so a tick is not a unit of time. See
/// [`tick_interval`].
#[derive(Debug, Clone)]
pub struct Sleep {
    /// What the footer shows, and what expiry is decided on.
    pub remaining: Duration,
    /// How long the timer was set for, so the note can name it.
    pub minutes: u64,
    /// When [`Sleep::remaining`] was last brought up to date.
    last: Instant,
}

impl Sleep {
    fn new(minutes: u64) -> Self {
        Self {
            remaining: Duration::from_secs(minutes * 60),
            minutes,
            last: Instant::now(),
        }
    }
}

/// Index of the `Local` row in [`App::sources`]. Always present, always first.
const LOCAL_ROW: usize = 0;

// There is no `NOT_YET_SOURCES`. The sidebar used to list `Local`, `Navidrome`,
// `Albums`, `Artists`, `Lists` and `Queue`, and only `Local` did anything; the
// first fix was to move the five dead names under a dim `NOT YET` heading so
// their absence read as *unbuilt* rather than as *broken*. Since then the
// server, its search and the queue all became real rows with real data behind
// them, and the heading was left carrying two names — `Artists` and `Lists` —
// that were nothing but an advertisement for features this build does not have.
// Naming an absent feature is a smaller version of the claim the signal-path
// seal exists to refuse, so it is gone rather than relabelled. See
// [`crate::ui::sidebar`].

/// The whole application.
///
/// Not `Debug` or `Clone`: it owns the [`Engine`], which is neither, and
/// deliberately so — there is exactly one engine per process and cloning the
/// app would imply otherwise.
pub struct App {
    /// The folder the library is read from, and how that was decided. Resolved
    /// once at startup — see [`crate::config::resolve_music_folder`].
    ///
    /// This is the **only** copy of that decision the fold keeps. The rest of
    /// [`Config`] is applied at construction and not held: a second, staler
    /// `music_folder` sitting beside this one is exactly the kind of split that
    /// let the scan and the screen disagree in the first place.
    pub folder: MusicFolder,
    pub theme: Theme,
    /// The flattened source tree shown in the sidebar.
    pub sources: Vec<SourceRow>,
    pub selected: usize,
    pub focus: Focus,
    pub playback: Playback,
    /// Whether the footer's spectrum is on screen. Drives the tick, so it is
    /// state rather than a render-time decision.
    pub spectrum_visible: bool,
    /// Whether the full-screen Now Playing view is up. See
    /// [`crate::ui::visualiser`].
    ///
    /// A **view**, not an overlay: it replaces the body, and the footer does not
    /// move. It is mutually exclusive with the three overlays — see
    /// [`App::act`] — so exactly one thing owns the body at any moment and there
    /// is no layering rule to get wrong.
    pub visualiser: bool,
    /// Whether the visualiser draws the analyser at all. From `config.toml`.
    pub analyser: bool,
    // There is no `backdrop` here any more. The `z` view used to wash the whole
    // body with the cover at eleven percent; it cost three times what the
    // analyser did, it could not be drawn at all on kitty or iTerm2 — the two
    // terminals where the big cover is a real photograph and therefore the two
    // this is most likely to be running on — and the framed sleeve and the
    // margins do what it was reaching for. See [`crate::ui::visualiser`].
    /// Right-hand title on the top border.
    pub context: String,
    /// Transient footer note — errors and hints, never a modal.
    pub status: Option<String>,

    // ── library ──────────────────────────────────────────────────────────
    pub library: Library,
    pub scan: ScanState,
    /// What the main pane is showing **for the local source**.
    pub view: View,
    /// Cursor in the local album list.
    pub album_cursor: usize,
    /// Cursor in the open local album's track list.
    pub track_cursor: usize,

    // ── the remote library ───────────────────────────────────────────────
    /// The server's albums. Filled in page by page — see [`crate::remote`].
    pub remote: remote::Library,
    pub remote_state: RemoteState,
    /// How the open remote album's track fetch is going.
    pub remote_detail: DetailState,
    /// The remote source's own view and cursors.
    ///
    /// Separate from the local ones on purpose: `tab`bing between two sources
    /// that shared a cursor would drop you somewhere arbitrary in the other
    /// list, and an index valid in a 4,000-album server is not valid in a
    /// two-album folder. Each source remembers where you were.
    pub remote_view: View,
    pub remote_album_cursor: usize,
    pub remote_track_cursor: usize,
    /// Which fetch the fold is currently listening to.
    ///
    /// `r` can start a second walk while the first still has pages in flight;
    /// without this they would both append and every album would appear twice.
    /// See [`crate::remote::RemoteEvent`].
    remote_generation: u64,

    // ── the server ───────────────────────────────────────────────────────
    /// The configured server, if there is one. Taken from
    /// [`Config::server`] once, at construction — multi-server replaces this
    /// field with a list and this comment with a selector.
    pub server: Option<ServerConfig>,
    /// **Every name the config already files a server under** — not just
    /// [`App::server`]'s.
    ///
    /// The fold talks to one server, so `server` is the first `[[servers]]`
    /// entry and the rest are ignored. A *name*, though, is a keychain account,
    /// and the keychain has exactly one entry per account: a second server typed
    /// in under an existing name overwrote the working password of the one that
    /// already had it, and then lost its own table to
    /// [`crate::config::Config::normalized`]'s "one name, one password" rule on
    /// the next launch. Two servers, neither of them signing in.
    ///
    /// So the panel is given the whole list rather than the one server the fold
    /// uses — see [`AddServer::validate`]. It is the names and not the configs
    /// because a name is all the rule is about, and a `Vec<ServerConfig>` here
    /// would read as multi-server having quietly arrived.
    server_names: Vec<String>,
    pub conn: ConnState,
    /// The live client, and **only** while [`App::conn`] is
    /// [`ConnState::Connected`]. Both are written in [`App::on_conn`] and
    /// nowhere else, so there is one edge rather than two states to keep in
    /// step.
    pub connection: Option<Connection>,
    /// The last footer note this module put up, so a connection note can be
    /// taken back down without clearing someone else's.
    conn_note: Option<String>,
    /// The add-server panel, when it is up. See [`AddServer`] — and note that
    /// there is no password in it.
    pub add: Option<AddServer>,
    /// A job for `main` to do outside the terminal takeover, waiting to be
    /// collected by [`App::take_prompt`]. See [`Prompt`].
    ///
    /// It is a *request*, not a call: the fold cannot suspend the terminal from
    /// inside `handle`, because the terminal is `main`'s and the whole point of
    /// the split is that the password prompt runs where raw mode and a half-drawn
    /// Deck are not.
    prompt: Option<Prompt>,

    // ── search ───────────────────────────────────────────────────────────
    /// The search pane. Present whether or not there is a server to ask; the
    /// *row* is not. See [`Search`].
    pub search: Search,

    // ── the queue ────────────────────────────────────────────────────────
    /// What plays, and what plays next. See [`crate::queue`].
    pub queue: Queue,
    /// Cursor in the Queue pane. Separate from [`Queue::position`], which is what
    /// is *playing* — you can look at one entry while another plays, exactly as
    /// you can in a library.
    pub queue_cursor: usize,

    // ── transport ────────────────────────────────────────────────────────
    pub now: Option<NowPlaying>,
    pub pos_ms: u64,
    /// How far the engine has decoded into the current track, in ms.
    ///
    /// Straight from [`EngineStatus::buffered_ms`], and read for exactly one
    /// thing: it is the furthest point a **stream** can be seeked to, because
    /// past it there is nothing decoded to play. See [`App::seek_by`].
    pub buffered_ms: u64,
    /// **Unity. Always.** There is no software volume in this application.
    ///
    /// Private, with no setter: the key bindings, the footer meter and
    /// `Config::volume` are all gone, so there is no reachable path to any other
    /// value — which is the whole point, and which
    /// [`tests::nothing_a_user_can_press_takes_the_seal_off_unity`] asserts
    /// against the entire keyboard.
    ///
    /// The two `#[ignore]`d tests that claim the machine's real output device
    /// write it to zero so they do not play out loud. That is the exception, it
    /// is in this module, and it is also the demonstration: silencing every
    /// session — including the one auto-advance starts — took one assignment,
    /// because `started` re-asserts this field rather than a literal.
    ///
    /// It stays as a **field**, and [`App::seal_input`] keeps reading it rather
    /// than passing `1.0`, for the `EqState::default()` reason: a hardcoded
    /// constant standing in for a value is honest right up to the commit that
    /// gives the value somewhere else to come from, and then it is a lie that
    /// compiles. One `f32` is what immunity costs if volume ever comes back —
    /// as a Pro feature, as a mute, as anything.
    volume: f32,
    /// Latest spectrum magnitudes from the engine. Empty until a session
    /// reports one — the footer draws a floor, never a guess.
    pub bands: Vec<f32>,
    /// Held peaks, one per band, for the visualiser's analyser caps.
    ///
    /// **State rather than a decay computed at render time**, which is the rule
    /// [`Sleep`] follows and for the same reason: a frame has to be a function
    /// of [`App`], or every snapshot of a playing Deck depends on how long the
    /// test took to reach its assertion. See
    /// [`crate::ui::visualiser::hold_peaks`].
    pub peaks: Vec<f32>,
    /// The engine's stream snapshot, and **only** once every field the seal
    /// reads has been reported. See [`stream_info`].
    pub stream: Option<StreamInfo>,

    // ── the output device ────────────────────────────────────────────────
    /// Which device the engine has been told to use, and how that was decided.
    ///
    /// The **only** copy of that decision, and the only thing
    /// `Engine::set_device` is ever handed — through
    /// [`OutputDevice::engine_pref`], so a device that is not connected cannot
    /// reach the engine and be silently swapped for the default down inside
    /// `eko-core`. See [`crate::config::OutputDevice`].
    pub device: OutputDevice,
    /// The host's output devices, as last listed.
    ///
    /// Refreshed when the picker opens rather than polled: `list_devices` opens
    /// the audio host, and doing that on a 30fps tick beside a realtime thread is
    /// the wrong trade for a list that changes when someone plugs something in.
    pub devices: Vec<String>,
    /// Whether the picker is on screen. View state, like [`App::eq_open`].
    pub device_open: bool,
    /// Cursor in the picker. Row 0 is the system default; the rest index
    /// [`App::devices`].
    pub device_cursor: usize,
    /// The file [`App::persist_device`] writes to, or `None` for nowhere.
    ///
    /// **`None` in every test build**, and that is the point rather than a
    /// convenience: [`crate::config::config_path`] resolves `$XDG_CONFIG_HOME` and
    /// then the home directory, so a fold that persisted from a test would rewrite
    /// the developer's own `~/.config/eko/config.toml` the first time an assertion
    /// pressed `enter` in the device picker. The write itself is tested where it
    /// lives — see `crate::config::write_output_device` — and end to end against a
    /// temporary path through [`App::set_config_path`].
    config_path: Option<std::path::PathBuf>,

    // ── the sleep timer ──────────────────────────────────────────────────
    /// The countdown, when one is running. See [`Sleep`].
    pub sleep: Option<Sleep>,

    // ── the help overlay ─────────────────────────────────────────────────
    /// Whether `?` is showing the keymap. Pure view state: the overlay renders
    /// [`crate::keys::BINDINGS`] and holds nothing of its own — no cursor, no
    /// scroll, no page. See [`crate::ui::help`].
    pub help_open: bool,

    // ── the EQ ───────────────────────────────────────────────────────────
    /// The graphic EQ, and the **only** copy of it.
    ///
    /// Private, and mutated in exactly one place — [`App::edit_eq`] — because
    /// this is the state the seal reports *and* the state the engine is handed.
    /// Read it through [`App::eq`]; change it through the `Action::Eq*` arms of
    /// [`App::act`]. See [`crate::eq`] for why that single edge is the whole
    /// mitigation.
    eq: crate::eq::GraphicEq,
    /// Which EQ column the panel's cursor is on. See [`crate::eq::COLUMNS`].
    ///
    /// View state, not signal state: moving the cursor changes nothing about
    /// the audio, so it deliberately does not live in [`App::eq`] and never
    /// goes near [`App::apply_eq`].
    pub eq_cursor: usize,
    /// Whether the EQ panel is on screen. Also view state.
    pub eq_open: bool,
    /// How many times [`App::apply_eq`] has handed the EQ to the engine.
    ///
    /// The engine exposes no way to read its EQ back — `EngineStatus` carries
    /// no DSP fields at all — so this counter is the only evidence available
    /// that a state change actually reached it. It exists for
    /// `every_eq_action_hands_the_engine_the_same_state_the_seal_reports`,
    /// which is what stops a future mutation path being added that moves the
    /// seal without moving the audio.
    pub eq_applied: u64,

    // ── the cover ────────────────────────────────────────────────────────
    /// Cover art: which renderer this terminal gets, what is wanted, and what
    /// has arrived. See [`crate::art::Pane`].
    ///
    /// `protocol` is [`crate::art::Protocol::Halfblock`] until `main` sets it,
    /// which is deliberate: [`App::new`] reads no environment variables, so a
    /// test states the protocol rather than inheriting whatever terminal
    /// `cargo test` happened to run in.
    pub art: art::Pane,
    /// The current track's waveform envelope: what is wanted, what has arrived,
    /// and the counter that drops stale answers. See [`crate::wave`].
    ///
    /// Its own pane rather than a field on [`App::art`], because the two are
    /// asked for at different moments and answer on different schedules — the
    /// envelope is only ever wanted while the visualiser is open, and only ever
    /// exists for a local file.
    pub wave: wave::Pane,
    /// The terminal's size, as last reported.
    ///
    /// The fold needs it because the cover cache is keyed on the art block's
    /// grid — see [`crate::ui::art_area`] — and a resize must invalidate that
    /// without waiting for a frame to notice. `(0, 0)` until `main` reports the
    /// first size, which reads as "too small to draw the Deck" and so asks for
    /// no cover at all: the honest answer before anything is known.
    term: (u16, u16),

    /// One engine for the process. See the module docs on async.
    engine: Arc<Engine>,
    /// Handle back into the event channel, so a rescan can be started from a
    /// keypress rather than only from `main`.
    events: Option<Sender<AppEvent>>,
    quit: bool,
    dirty: bool,
}

/// The sidebar's rows, for a given server. **One place, so the row list after
/// adding a server is built by the same code as the row list at startup.**
///
/// Every row here works — see [`SourceRow`] — including the one that is an
/// invitation rather than a library. `+ Add server` is not a promise that a
/// server exists; it is a promise that pressing `enter` starts configuring one,
/// and that promise is kept. The alternative, which this replaces, was no row at
/// all: a Deck with no `[[servers]]` in its config showed nothing about servers
/// anywhere, which is how the owner came to ask how to connect to Navidrome and
/// find the answer was "hand-write a TOML file and quit the app".
fn build_sources(server: Option<&ServerConfig>) -> Vec<SourceRow> {
    let mut sources = vec![SourceRow::source("Local", Source::Local)];
    match server {
        // Labelled with the `name` from `config.toml` rather than a generic
        // "Navidrome": that name is what `eko-cli login` takes, what the keychain
        // files the password under, and what multi-server will list several
        // of. One vocabulary.
        Some(server) => {
            sources.push(SourceRow::source(&server.name, Source::Remote));
            // Search is `search3`, so it is a row only when there is something to
            // ask. A Deck with no server showing a Search source would be the
            // same broken promise a row with nothing behind it always is — and `/`
            // says so in a note rather than opening an input nothing can answer.
            sources.push(SourceRow::source("Search", Source::Search));
        }
        None => sources.push(SourceRow::invitation("Add server", Source::Remote)),
    }
    // The queue is always a row, because it always exists: an empty one is a
    // real, nameable state ("nothing queued") rather than a missing feature.
    // It sits last because it is downstream of the two libraries — you fill
    // it from them.
    sources.push(SourceRow::source("Queue", Source::Queue));
    sources
}

impl App {
    /// Build the initial state.
    ///
    /// `folder` is resolved by the caller rather than in here, so the fold
    /// never touches the filesystem to find out where it is pointing and every
    /// test can state the answer instead of inheriting the machine's.
    #[must_use]
    pub fn new(config: &Config, theme: Theme, folder: MusicFolder) -> Self {
        let engine = Arc::new(Engine::default());
        // The host is only asked when there is a name to check. `App::new`
        // otherwise touches nothing outside this process — a test that builds a
        // default `Config` must not open CoreAudio, and on a machine with no
        // audio at all a Deck still has to start.
        let devices = if config.output_device.is_some() {
            eko_core::engine::list_devices()
        } else {
            Vec::new()
        };
        let device = config::resolve_output_device(config.output_device.clone(), &devices);
        // `engine_pref`, never `config.output_device`: a configured device that is
        // not connected is handed over as `None` *here*, so the fallback is this
        // crate's decision and can be said out loud, rather than `eko-core`'s
        // `.or_else(default_output_device())` happening in silence.
        engine.set_device(device.engine_pref());
        let server = config.server().cloned();
        // Every entry, not `server`: `Config::normalized` has already dropped
        // the half-written and the duplicate ones, so what is left is exactly
        // the set of names the keychain files a password under.
        let server_names = config.servers.iter().map(|s| s.name.clone()).collect();
        let sources = build_sources(server.as_ref());
        // A configured device that is not connected is said out loud, once, at
        // startup. Silence here is the whole failure mode: the Deck would play
        // the laptop speakers and describe them in the seal, which is a true
        // sentence about the wrong device.
        let status = match &device {
            OutputDevice::Missing(name) => Some(format!(
                "{name} is not connected · using the system default"
            )),
            OutputDevice::Default | OutputDevice::Configured(_) => None,
        };
        Self {
            folder,
            theme,
            sources,
            selected: 0,
            focus: Focus::default(),
            playback: Playback::Stopped,
            spectrum_visible: true,
            visualiser: false,
            analyser: config.analyser,
            context: "no source · idle".to_string(),
            status,
            library: Library::default(),
            scan: ScanState::Unconfigured,
            view: View::Albums,
            album_cursor: 0,
            track_cursor: 0,
            remote: remote::Library::default(),
            remote_state: RemoteState::Idle,
            remote_detail: DetailState::Idle,
            remote_view: View::Albums,
            remote_album_cursor: 0,
            remote_track_cursor: 0,
            remote_generation: 0,
            server,
            server_names,
            conn: ConnState::NotConfigured,
            connection: None,
            conn_note: None,
            add: None,
            prompt: None,
            search: Search::default(),
            queue: Queue::default(),
            queue_cursor: 0,
            now: None,
            pos_ms: 0,
            buffered_ms: 0,
            // See the field: pinned, and the only place it is ever written.
            volume: 1.0,
            bands: Vec::new(),
            peaks: Vec::new(),
            stream: None,
            device,
            devices,
            device_open: false,
            device_cursor: 0,
            // See the field. A test never writes the real config file.
            #[cfg(not(test))]
            config_path: config::config_path(),
            #[cfg(test)]
            config_path: None,
            sleep: None,
            help_open: false,
            eq: crate::eq::GraphicEq::default(),
            eq_cursor: 0,
            eq_open: false,
            eq_applied: 0,
            art: art::Pane::default(),
            wave: wave::Pane::default(),
            term: (0, 0),
            engine,
            events: None,
            quit: false,
            // The first frame always has to be drawn.
            dirty: true,
        }
    }

    /// Hand the fold a sender so it can start work of its own, then start the
    /// first scan and the first connection. Called once, from `main`, after the
    /// channel exists.
    ///
    /// Both run on their own workers and neither waits for the other: a cold NFS
    /// mount must not delay the server, and an unreachable server must not
    /// delay the library.
    pub fn attach(&mut self, events: Sender<AppEvent>) {
        self.events = Some(events);
        self.start_scan();
        self.start_connect();
    }

    /// Kick off a scan of the resolved music folder, if there is one.
    ///
    /// Runs on a worker thread. Nothing about the scan touches the render
    /// thread — see [`crate::library`]. The folder is [`App::folder`], not
    /// `config.music_folder`: an absent setting still has a platform default
    /// behind it, and that is the whole reason a first run finds anything.
    pub fn start_scan(&mut self) {
        // `r` during a scan would otherwise stack workers on the same folder,
        // and the second one's progress would fight the first one's.
        if self.scan.is_running() {
            return;
        }
        let root = self.folder.path().map(std::path::Path::to_path_buf);
        let (Some(root), Some(events)) = (root, self.events.clone()) else {
            return;
        };
        self.scan = ScanState::Discovering { found: 0 };
        self.dirty = true;
        library::spawn(root, move |event| {
            events.send(AppEvent::Scan(event)).is_ok()
        });
    }

    /// Connect to the configured server, if there is one.
    ///
    /// **Everything about this is off the render thread.** `Client::new` does no
    /// I/O, but `ping` is a round trip and the keychain read in front of it can
    /// raise a modal authorisation prompt on macOS — either would freeze the
    /// Deck. See [`crate::server::spawn`].
    ///
    /// Re-entrant by design: `r` retries, which is how someone who has just run
    /// `eko-cli login` gets connected without restarting. A second attempt while one
    /// is in flight is refused, for the same reason a second scan is.
    pub fn start_connect(&mut self) {
        if self.conn.is_connecting() {
            return;
        }
        let (Some(server), Some(events)) = (self.server.clone(), self.events.clone()) else {
            return;
        };
        self.conn = ConnState::Connecting {
            name: server.name.clone(),
        };
        // A retry starts from nothing: keeping the previous client would let a
        // failed reconnect leave a stale "connected" behind it, and keeping the
        // previous album list would leave a browsable library hanging off a
        // connection that no longer exists.
        self.connection = None;
        self.forget_remote();
        self.set_conn_note(None);
        self.context = self.describe_context();
        self.dirty = true;
        server::spawn(server, move |event| {
            events.send(AppEvent::Conn(event)).is_ok()
        });
    }

    /// Put up, replace or take down the footer note this module owns.
    ///
    /// Clearing only clears **our** note. The config-parse note set at startup
    /// belongs to someone else, and a connection succeeding is not a reason to
    /// throw it away.
    fn set_conn_note(&mut self, note: Option<String>) {
        match &note {
            Some(_) => self.status = note.clone(),
            None if self.status == self.conn_note => self.status = None,
            None => {}
        }
        self.conn_note = note;
        self.dirty = true;
    }

    /// The tick this state wants. See [`tick_interval`].
    #[must_use]
    pub fn tick_interval(&self) -> Option<Duration> {
        tick_interval(self.playback, self.animating())
    }

    /// Whether anything driven by the FFT is on screen.
    ///
    /// The footer's spectrum, or the visualiser's analyser — the two things in
    /// this program that have to be redrawn thirty times a second. Everything
    /// else on either view changes at most once a second.
    #[must_use]
    pub fn animating(&self) -> bool {
        if self.visualiser {
            return self.analyser;
        }
        self.spectrum_visible
    }

    #[must_use]
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Consume the dirty flag. The loop redraws exactly when this is `true`.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.dirty = true;
    }

    /// The label of the selected source, for the main pane's heading.
    #[must_use]
    pub fn selected_label(&self) -> &str {
        self.sources
            .get(self.selected)
            .map_or("—", |row| row.label.as_str())
    }

    /// Indices of the rows the sidebar should actually draw: every root, plus
    /// the children of the roots that are open.
    ///
    /// Navigation walks this list too, so the cursor can never land on a row
    /// that is not on screen.
    #[must_use]
    pub fn visible_sources(&self) -> Vec<usize> {
        let mut visible = Vec::with_capacity(self.sources.len());
        let mut parent_open = false;
        for (index, row) in self.sources.iter().enumerate() {
            if row.depth == 0 {
                parent_open = row.expanded;
                visible.push(index);
            } else if parent_open {
                visible.push(index);
            }
        }
        visible
    }

    /// Which library the main pane is showing.
    ///
    /// Derived from the sidebar selection and stored nowhere, so there is no
    /// second copy to drift: moving the sidebar cursor *is* switching source, and
    /// the heading, the list and the row count cannot disagree about which one it
    /// is.
    #[must_use]
    pub fn source(&self) -> Source {
        self.sources
            .get(self.selected)
            .map_or(Source::Local, |row| row.source)
    }

    /// The view the current source is in.
    ///
    /// The queue is a flat list with nothing to drill into, so it is always
    /// [`View::Albums`] — "the top level of this source".
    #[must_use]
    pub fn view(&self) -> View {
        match self.source() {
            Source::Local => self.view,
            Source::Remote => self.remote_view,
            Source::Search => self.search.view,
            Source::Queue => View::Albums,
        }
    }

    /// Index of the server's row in [`App::sources`], when there is one.
    fn remote_row(&self) -> Option<usize> {
        self.sources.iter().position(|r| r.source == Source::Remote)
    }

    /// Index of the search row in [`App::sources`] — only there with a server.
    fn search_row(&self) -> Option<usize> {
        self.sources.iter().position(|r| r.source == Source::Search)
    }

    /// Index of the queue's row in [`App::sources`]. Always present.
    fn queue_row(&self) -> Option<usize> {
        self.sources.iter().position(|r| r.source == Source::Queue)
    }

    /// The sidebar label a source is known by — `Local`, the server's configured
    /// name, or `Queue`.
    ///
    /// Read out of [`App::sources`] rather than spelled again, so the Queue pane
    /// cannot name a server differently from the sidebar that lists it.
    #[must_use]
    pub fn source_label(&self, source: Source) -> &str {
        self.sources
            .iter()
            .find(|row| row.source == source)
            .map_or("—", |row| row.label.as_str())
    }

    /// The local album the main pane has open, if any.
    #[must_use]
    pub fn open_album(&self) -> Option<&library::Album> {
        match self.view {
            View::Album(i) => self.library.albums.get(i),
            View::Albums => None,
        }
    }

    /// The remote album the main pane has open, if any.
    #[must_use]
    pub fn open_remote_album(&self) -> Option<&remote::Album> {
        match self.remote_view {
            View::Album(i) => self.remote.album(i),
            View::Albums => None,
        }
    }

    /// How many rows the focused list has.
    #[must_use]
    pub fn row_count(&self) -> usize {
        match (self.source(), self.view()) {
            (Source::Local, View::Albums) => self.library.albums.len(),
            (Source::Local, View::Album(_)) => self.open_album().map_or(0, |a| a.tracks.len()),
            (Source::Remote, View::Albums) => self.remote.albums.len(),
            // `None` tracks is "not fetched yet", not "no tracks" — either way
            // there is nothing to put a cursor on.
            (Source::Remote, View::Album(_)) => self
                .open_remote_album()
                .and_then(|a| a.tracks.as_ref())
                .map_or(0, Vec::len),
            // One flat list: every matched album, then every matched song. Two
            // sections with headings would read better and would put untargetable
            // rows in the middle of a list the cursor walks by index — so the
            // *kind* is a column on the row instead. See `crate::ui::main_pane`.
            (Source::Search, View::Albums) => self.search.results.len(),
            (Source::Search, View::Album(_)) => self
                .search
                .open_album()
                .and_then(|a| a.tracks.as_ref())
                .map_or(0, Vec::len),
            (Source::Queue, _) => self.queue.len(),
        }
    }

    /// The cursor within the focused list.
    #[must_use]
    pub fn cursor(&self) -> usize {
        match (self.source(), self.view()) {
            (Source::Local, View::Albums) => self.album_cursor,
            (Source::Local, View::Album(_)) => self.track_cursor,
            (Source::Remote, View::Albums) => self.remote_album_cursor,
            (Source::Remote, View::Album(_)) => self.remote_track_cursor,
            (Source::Search, View::Albums) => self.search.cursor,
            (Source::Search, View::Album(_)) => self.search.track_cursor,
            (Source::Queue, _) => self.queue_cursor.min(self.queue.len().saturating_sub(1)),
        }
    }

    /// Step the sidebar selection through the visible rows by `delta`, wrapping.
    fn move_selection(&mut self, delta: isize) {
        let visible = self.visible_sources();
        if visible.is_empty() {
            return;
        }
        let current = visible
            .iter()
            .position(|&i| i == self.selected)
            .unwrap_or(0);
        let len = visible.len() as isize;
        let next = (current as isize + delta).rem_euclid(len) as usize;
        self.selected = visible[next];
        // The selection *is* the source, so the top-border context follows it.
        self.context = self.describe_context();
        self.dirty = true;
    }

    /// Step the main pane's cursor, wrapping. A no-op on an empty list.
    fn move_cursor(&mut self, delta: isize) {
        let len = self.row_count();
        if len == 0 {
            return;
        }
        let next = (self.cursor() as isize + delta).rem_euclid(len as isize) as usize;
        match (self.source(), self.view()) {
            (Source::Local, View::Albums) => self.album_cursor = next,
            (Source::Local, View::Album(_)) => self.track_cursor = next,
            (Source::Remote, View::Albums) => self.remote_album_cursor = next,
            (Source::Remote, View::Album(_)) => self.remote_track_cursor = next,
            (Source::Search, View::Albums) => self.search.cursor = next,
            (Source::Search, View::Album(_)) => self.search.track_cursor = next,
            (Source::Queue, _) => self.queue_cursor = next,
        }
        self.dirty = true;
    }

    /// Fold one event into the state.
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Tick => self.on_tick(),
            AppEvent::Scan(event) => self.on_scan(event),
            AppEvent::Conn(event) => self.on_conn(event),
            AppEvent::Remote(event) => self.on_remote(event),
            AppEvent::Search(event) => self.on_search(event),
            AppEvent::Art(event) => self.on_art(event),
            AppEvent::Wave(event) => self.on_wave(event),
            AppEvent::Input(Event::Resize(w, h)) => {
                self.term = (w, h);
                self.dirty = true;
            }
            AppEvent::Input(Event::Key(key)) => self.on_key(key),
            // Bracketed paste, enabled in [`crate::tui`]. It only means anything
            // while the search input is open; everywhere else a paste is not a
            // gesture this application has, and ignoring it is the honest answer.
            AppEvent::Input(Event::Paste(text)) => self.on_paste(&text),
            AppEvent::Input(_) => {}
        }
        // **After** every event, never inside one. What the cover should be is a
        // function of the transport, the queue and the terminal size, and all
        // three can move on almost any event — a track ending on a `Tick`, a
        // resize, `enter` on a row. One place that recomputes it beats a `Some`
        // scattered through a dozen arms, each of which is a chance to forget.
        self.sync_art();
        self.sync_wave();
    }

    /// Poll the engine. The only place transport state is read back.
    ///
    /// The sleep timer is brought down **after** the poll, so a track that ended
    /// on this tick has already been folded in and the timer is measured against
    /// the transport state it really produced.
    fn on_tick(&mut self) {
        self.poll_engine();
        self.tick_sleep();
        self.dirty = true;
    }

    /// Poll the engine once and fold the snapshot in.
    ///
    /// `Engine::status` and `Engine::bands` are each called **at most once** per
    /// tick: `status` takes the session mutex and `bands` clones a `Vec<f32>`
    /// out from under another, and the tick can run at 30fps beside a realtime
    /// audio thread.
    fn poll_engine(&mut self) {
        let Some(status) = self.engine.status() else {
            // No session at all: there is nothing to seal.
            self.stream = None;
            return;
        };
        self.pos_ms = status.pos_ms;
        self.buffered_ms = status.buffered_ms;
        if status.dur_ms > 0 {
            if let Some(now) = self.now.as_mut() {
                now.dur_ms = status.dur_ms;
            }
        }
        // `None` until the engine has reported a whole stream — see
        // [`stream_info`]. Never carried over from the previous track.
        self.stream = stream_info(&status);
        if self.playback == Playback::Playing {
            self.bands = self.engine.bands();
            // The caps follow the bands on the same tick that produced them, so
            // a peak is never a frame behind the bar that set it.
            crate::ui::visualiser::hold_peaks(&mut self.peaks, &self.bands);
            // The engine dropped out on its own. Two very different things look
            // identical from here, and the queue must only follow one of them.
            if !status.playing {
                self.playback = Playback::Stopped;
                self.bands.clear();
                // The caps go with them: a held peak over a stopped transport is a
                // picture of something that is not happening.
                self.peaks.clear();
                if self.session_ran(&status) {
                    // It played, and then it ended. That is the queue's cue.
                    self.advance_queue();
                } else {
                    // It never opened. Advancing here would march through the
                    // whole queue in a few frames, one failure per tick, and end
                    // in silence with a note about the *last* track rather than
                    // the one that broke.
                    self.note_if_it_never_started(&status);
                }
            }
        }
    }

    /// Whether this session ever actually decoded anything.
    ///
    /// The two conditions are what separate "played to the end" from "never
    /// opened": a track that ran reported a rate (so [`App::stream`] is `Some`)
    /// **or** moved the playhead. A track that refused to start has neither —
    /// `Engine::play_url` reports nothing at all on a failed open, so the rates
    /// stay `0` forever after. See [`App::note_if_it_never_started`].
    fn session_ran(&self, status: &EngineStatus) -> bool {
        self.stream.is_some() || status.pos_ms > 0
    }

    /// Start the next queued entry, if there is one.
    ///
    /// **This is auto-advance**, and the only caller is [`App::poll_engine`] on
    /// the frame a track ends. Before Task 4 there was no caller at all: the
    /// transport went to `Stopped` and stayed there, which was honest and was
    /// also the whole gap.
    ///
    /// Sequential, not gapless — see [`App::play_current`]. The transport is
    /// already `Stopped` when this is called, so the end of the queue needs no
    /// branch: [`crate::queue::Queue::peek_next`] answers `None` and the Deck
    /// simply stays stopped.
    fn advance_queue(&mut self) {
        if let Some(next) = self.queue.peek_next() {
            self.play_queue_at(next);
        }
    }

    /// Name a session that ended without ever having begun.
    ///
    /// # The failure this exists to make visible
    ///
    /// `Engine::play_url` returns a stub `EngineStatus` **before** the download
    /// thread has issued a single request, and it reports nothing back when that
    /// request 404s, times out, or is answered with a reverse proxy's HTML login
    /// page. All the engine does on a failed open is `playing.store(false)`; the
    /// rates are never written, so `status()` answers `rate: 1, src_rate: 0,
    /// dev_rate: 0` forever after.
    ///
    /// Two things follow, and both are already right by construction:
    ///
    /// * the transport goes back to `Stopped` above, so the footer says *Nothing
    ///   playing* rather than showing a running clock over silence; and
    /// * [`stream_info`] refuses that half-filled status, so [`App::stream`]
    ///   stays `None`, `derive` returns `active: false`, and the lamp is the
    ///   hollow `○ IDLE` — never the green one. The desktop app's equivalent bug
    ///   leaves a *permanently green* seal on exactly this path.
    ///
    /// What was missing is a word about it. A track that simply refuses to start,
    /// with the Deck returning silently to idle, is the same class of screen this
    /// phase has already had reported once. The two conditions are what separate
    /// "never opened" from "played to the end": nothing was ever decoded
    /// (`stream` is `None`, so no rate was ever reported) **and** the playhead
    /// never moved. A track that finished has both the other way round.
    ///
    /// The note names the track, never the URL: it carries `u`, `t` and `s` and
    /// is a replayable credential. See [`App::play_remote`].
    fn note_if_it_never_started(&mut self, status: &EngineStatus) {
        if self.session_ran(status) {
            return;
        }
        let Some(now) = self.now.as_ref() else {
            return;
        };
        // Already tidied for a remote track — see [`crate::remote`].
        let title = now.title.clone();
        let why = match now.source {
            Source::Local => "the file could not be read",
            Source::Remote => "the server sent nothing playable",
            // Unreachable: `NowPlaying::source` is read off the queue entry's own
            // media, which is only ever local or remote — a search result streams
            // from the server and so reports `Remote`. Stated rather than
            // unwrapped — see [`Source::Queue`].
            Source::Queue | Source::Search => "it could not be opened",
        };
        self.set_status(format!("could not play {title} · {why}"));
    }

    // ── the cover ────────────────────────────────────────────────────────

    /// Tell the fold how big the terminal is.
    ///
    /// Called once from `main` after the takeover, because the first size
    /// arrives from an `ioctl` rather than from a `Resize` event — without it
    /// the Deck would draw a cover only after the window was first resized.
    pub fn set_term_size(&mut self, width: u16, height: u16) {
        if self.term == (width, height) {
            return;
        }
        self.term = (width, height);
        self.dirty = true;
        self.sync_art();
    }

    /// The cover the current state calls for, or `None` for no cover at all.
    ///
    /// `None` in three cases, and each is a real answer rather than a gap: the
    /// transport is stopped (the footer says *Nothing playing*, and a cover over
    /// that would be a claim about something that is not happening); the
    /// terminal is too small for the Deck to be drawn; or the queue has nothing
    /// current.
    ///
    /// The token comes off [`crate::queue::Media`] — the same field the
    /// transport plays from — rather than off `now`'s indices. Indices are
    /// resolved against a list that can be replaced under them by a page of
    /// remote albums landing; the media is what is *actually playing*, so a
    /// cover keyed on it cannot come to belong to a different track without the
    /// track changing.
    /// The block the cover occupies right now — **the footer's, or the
    /// visualiser's**.
    ///
    /// The one place that choice is made. [`App::art_key`] keys the cache on
    /// this rect's size and [`App::art_placement`] positions an out-of-band
    /// image at its corner, and a second copy of the `if` is how an album cover
    /// ends up eight cells wide in the middle of a full-screen view — or, worse,
    /// forty cells tall over the seal.
    ///
    /// Pressing `z` therefore *changes the key*, which is exactly right: the
    /// same picture at a different grid is a different encoding, and
    /// [`art::Pane`] refetches it through the same [`art::spawn`] under the same
    /// generation-drop. Nothing about the renderer is forked; one argument
    /// changed.
    fn cover_rect(&self) -> Option<Rect> {
        let term = Rect::new(0, 0, self.term.0, self.term.1);
        if self.visualiser {
            return crate::ui::visualiser::cover_area(term);
        }
        crate::ui::art_area(term)
    }

    fn art_key(&self) -> Option<art::Key> {
        if self.playback == Playback::Stopped {
            return None;
        }
        let rect = self.cover_rect()?;
        let token = match &self.queue.current()?.media {
            // Prefixed, so a file called `abc` and a server id `abc` cannot be
            // the same cache key.
            Media::Local(path) => format!("local:{path}"),
            Media::Remote(id) => format!("remote:{id}"),
        };
        Some(art::Key {
            token,
            cols: rect.width,
            rows: rect.height,
            protocol: self.art.protocol,
        })
    }

    /// Where the current track's cover bytes come from.
    ///
    /// # The remote id
    ///
    /// `getCoverArt` takes the id of a song, an album or an artist, so the
    /// **track id** the queue already holds is a valid handle and no second
    /// field has to be threaded through [`crate::queue::Entry`] to carry a
    /// separate `coverArt`. A server that has no art for that id answers 404 and
    /// the footer keeps its placeholder, which is the same outcome as a track
    /// with no art at all — see [`crate::art::Outcome`].
    ///
    /// # The URL is minted here and held nowhere
    ///
    /// [`eko_net::urls::cover_art_src_url`], never `cover_art_url`: the latter
    /// returns `stream://localhost/?src=…`, which is Tauri's protocol handler
    /// and resolves to nothing in a terminal process. The direct one carries
    /// `u`, `t` and `s` — a replayable credential — so it goes straight into the
    /// worker's [`art::Fetch`], whose `Debug` refuses to print it, and is
    /// dropped when the worker ends. Exactly the rule [`App::stream_url`]
    /// follows.
    fn art_fetch(&self) -> Option<art::Fetch> {
        match &self.queue.current()?.media {
            Media::Local(path) => Some(art::Fetch::Local(path.clone())),
            Media::Remote(id) => {
                let connection = self.connection.as_ref()?;
                eko_net::urls::cover_art_src_url(
                    connection.client.config(),
                    Some(id),
                    Some(art::REQUEST_EDGE),
                    &eko_net::auth::random_salt(),
                )
                .map(art::Fetch::Remote)
            }
        }
    }

    /// Start a cover fetch if — and **only** if — what is wanted has changed.
    ///
    /// This runs after every single event, so the early return is the load-
    /// bearing line: on all but a handful of frames in a session the key is
    /// identical and nothing at all happens. A version that refetched per frame
    /// would put a Navidrome request and a JPEG decode inside a 30fps loop
    /// running beside a realtime audio thread.
    fn sync_art(&mut self) {
        let want = self.art_key();
        if want.as_ref() == self.art.wants() {
            return;
        }
        // The pixels on screen are now wrong, whatever happens next.
        self.dirty = true;
        let generation = self.art.begin(want.clone());
        let Some(key) = want else {
            return;
        };
        // Nothing to fetch from: a remote track with no live client to sign a
        // URL with. Stated as "no picture" rather than left in `Loading`, which
        // would be a promise of something that is never coming.
        let (Some(fetch), Some(events)) = (self.art_fetch(), self.events.clone()) else {
            self.art.give_up();
            return;
        };
        art::spawn(fetch, key, generation, move |event| {
            events.send(AppEvent::Art(event)).is_ok()
        });
    }

    /// Fold one worker's cover in, unless it is stale.
    fn on_art(&mut self, event: art::Event) {
        if self.art.accept(event) {
            self.dirty = true;
        }
    }

    // ── the waveform overview ────────────────────────────────────────────

    /// The envelope the current state calls for, or `None` for none at all.
    ///
    /// **Only while the visualiser is open, and only for a local file.** Both
    /// halves matter:
    ///
    /// * A whole-track envelope needs the whole track, so a Navidrome stream
    ///   would have to be downloaded in full before the first pixel could be
    ///   drawn. There is no envelope for one, the visualiser draws nothing in
    ///   its place, and the footer's plain scrubber is what says where you are.
    /// * Decoding a file nobody is looking at is a few hundred milliseconds of
    ///   CPU spent on a row of glyphs that is not on screen. The Deck is the
    ///   view someone has open for eight hours; it pays nothing for this.
    ///
    /// The token is the **same one the cover is keyed on**, so the two caches
    /// agree about what the current track is by construction.
    fn wave_key(&self) -> Option<wave::Key> {
        if !self.visualiser || self.playback == Playback::Stopped {
            return None;
        }
        match &self.queue.current()?.media {
            Media::Local(path) => Some(wave::Key {
                token: format!("local:{path}"),
            }),
            Media::Remote(_) => None,
        }
    }

    /// Start an envelope decode if — and **only** if — what is wanted has
    /// changed. [`App::sync_art`]'s rule, and its early return is load-bearing
    /// for the same reason.
    fn sync_wave(&mut self) {
        let want = self.wave_key();
        if want.as_ref() == self.wave.wants() {
            return;
        }
        self.dirty = true;
        let generation = self.wave.begin(want.clone());
        let Some(key) = want else {
            return;
        };
        let (Some(Media::Local(path)), Some(events)) = (
            self.queue.current().map(|e| e.media.clone()),
            self.events.clone(),
        ) else {
            // Stated as "no shape" rather than left in `Loading`, which would be
            // a promise of something that is never coming.
            self.wave.give_up();
            return;
        };
        wave::spawn(path, key, generation, &wave::LIVE, move |event| {
            events.send(AppEvent::Wave(event)).is_ok()
        });
    }

    /// Fold one worker's envelope in, unless it is stale.
    fn on_wave(&mut self, event: wave::Event) {
        if self.wave.accept(event) {
            self.dirty = true;
        }
    }

    /// Where an out-of-band cover goes on the real terminal, if there is one.
    ///
    /// `None` for the halfblock renderer, which needs no help — its pixels are
    /// ratatui cells like any other. See [`crate::art::Painter`] for the
    /// sequencing this feeds.
    #[must_use]
    pub fn art_placement(&self) -> Option<art::Placement<'_>> {
        let art = self.art.escape()?;
        let rect = self.cover_rect()?;
        Some(art::Placement {
            id: self.art.generation(),
            x: rect.x,
            y: rect.y,
            art,
        })
    }

    // ── the seal ─────────────────────────────────────────────────────────

    /// Everything [`signal_path::derive`] needs, from this state.
    ///
    /// # `replaygain_db` is not an `f64` here, on purpose
    ///
    /// [`SealInput::replaygain_db`] is a [`signal_path::SealRgDb`] with a private
    /// field, so `SealInput { replaygain_db: some_db, .. }` does not compile. The
    /// only ways to build one apply the ±0.01 dB dead-band, which is the boundary
    /// that decides whether EKO claims bit-perfect — a front end that skipped it
    /// would report `REPLAYGAIN` where the desktop app reports `BIT-PERFECT` for
    /// identical playback. `eko-cli` has no ReplayGain yet, so it passes the
    /// honest absence through the same constructor rather than reaching for
    /// `Default` and pretending the question was never asked.
    ///
    /// # The EQ is read, never assumed
    ///
    /// This used to be `eq: EqState::default()` — graphic, disabled — and that
    /// was honest for exactly as long as the crate had no EQ: nothing could
    /// enable one, and nothing called `Engine::set_eq`. The moment a panel could
    /// switch it on, that hardcode became a seal that reports `BIT-PERFECT` over
    /// an EQ'd signal — the seventh false-`BIT-PERFECT` path in this project and
    /// the first that would have been created on purpose. So it is
    /// [`crate::eq::GraphicEq::seal_state`], derived from the same three fields
    /// that [`App::apply_eq`] hands to the engine, and it was wired **before**
    /// the panel and the keymap existed. See
    /// [`enabling_the_eq_with_a_raised_band_breaks_the_seal`].
    ///
    /// The parametric fields inside that `EqState` stay at their defaults, which
    /// is again the truth rather than a convenience: parametric EQ is Pro and
    /// this crate is FREE.
    ///
    /// [`enabling_the_eq_with_a_raised_band_breaks_the_seal`]: App::seal_input
    #[must_use]
    pub fn seal_input(&self) -> SealInput {
        SealInput {
            // The engine is the only audio source this client has; when the
            // transport is stopped it is not driving anything.
            engine_active: self.playback != Playback::Stopped,
            info: self.stream.clone(),
            eq: self.eq.seal_state(),
            volume: f64::from(self.volume),
            replaygain_db: signal_path::applied_replaygain_db(None),
            replaygain_mode: RgMode::Off,
        }
    }

    /// The derived signal path. **The single source of the seal.**
    ///
    /// Pure and cheap — a handful of small `String`s per frame — so the footer
    /// calls it once per render rather than caching a copy that could go stale
    /// against the volume or the stream it was derived from.
    ///
    /// Nothing here adjusts, rounds or second-guesses what `derive` returned.
    #[must_use]
    pub fn seal(&self) -> SignalPath {
        signal_path::derive(&self.seal_input())
    }

    fn on_scan(&mut self, event: ScanEvent) {
        self.dirty = true;
        match event {
            ScanEvent::Discovering { found } => self.scan = ScanState::Discovering { found },
            ScanEvent::Reading { done, total } => self.scan = ScanState::Reading { done, total },
            // Not a footer note: the main pane is empty and says so in full,
            // and a second copy of the same sentence in the status row would
            // only crowd the seal.
            ScanEvent::Missing(path) => self.scan = ScanState::Missing(path),
            ScanEvent::Failed(message) => {
                self.scan = ScanState::Failed(message.clone());
                self.set_status(message);
            }
            ScanEvent::Finished(library) => {
                self.library = *library;
                self.scan = ScanState::Ready;
                self.view = View::Albums;
                self.album_cursor = 0;
                self.track_cursor = 0;
                self.sources[LOCAL_ROW].badge = if self.library.is_empty() {
                    None
                } else {
                    Some(self.library.albums.len().to_string())
                };
                self.context = self.describe_context();
            }
        }
    }

    /// Fold a connection result in.
    ///
    /// The **only** place [`App::conn`] and [`App::connection`] are written, so
    /// a live client cannot outlive the state that claims it.
    ///
    /// Nothing here formats a URL into a message. The strings come from
    /// [`crate::server::ServerError`], which is the boundary that guarantees
    /// they carry no credential — `eko-net`'s `safe_message` has already
    /// stripped the signed URL out of any transport error before it gets here.
    fn on_conn(&mut self, event: ConnEvent) {
        self.dirty = true;
        match event {
            ConnEvent::NeedsPassword { name } => {
                self.connection = None;
                // **The fix is a keystroke, and it says so.** This note used to
                // read `run: eko-cli login {name}` — the application telling you
                // to leave it and run something else, which is exactly the
                // journey this feature exists to delete. `eko-cli login` still
                // works, and is still the right answer for a password manager
                // pipe, but it is no longer the only one.
                self.set_conn_note(Some(format!(
                    "{name} · no password stored · press enter on it in sources"
                )));
                self.conn = ConnState::NeedsPassword { name };
                self.forget_remote();
            }
            ConnEvent::Connected(connection) => {
                self.conn = ConnState::Connected {
                    name: connection.name.clone(),
                    username: connection.username.clone(),
                };
                self.connection = Some(*connection);
                // Success is not a note. The seal row belongs to the signal
                // path; a permanent "connected" banner would take it.
                self.set_conn_note(None);
                // A connection is only useful because of what it can fetch, so
                // the album walk starts here rather than waiting to be asked.
                self.start_remote_albums();
            }
            ConnEvent::Failed { name, message } => {
                self.connection = None;
                self.set_conn_note(Some(format!("{name} · {message}")));
                self.conn = ConnState::Failed { name, message };
                self.forget_remote();
            }
        }
        self.context = self.describe_context();
    }

    /// Walk the server's album list on a worker thread.
    ///
    /// Everything about this is off the render thread — a full library is
    /// hundreds of round trips. Pages arrive as [`AppEvent::Remote`] and are
    /// appended as they land, so a large library fills in rather than appearing
    /// all at once when the last page comes back.
    ///
    /// Starting resets: the generation is bumped first, so any page still in
    /// flight from a previous walk is dropped by [`App::on_remote`] rather than
    /// appended to the new list.
    fn start_remote_albums(&mut self) {
        let (Some(connection), Some(events)) = (self.connection.clone(), self.events.clone())
        else {
            return;
        };
        self.remote_generation = self.remote_generation.wrapping_add(1);
        let generation = self.remote_generation;
        self.remote = remote::Library::default();
        self.remote_view = View::Albums;
        self.remote_album_cursor = 0;
        self.remote_track_cursor = 0;
        self.remote_detail = DetailState::Idle;
        self.remote_state = RemoteState::Loading;
        self.set_remote_badge();
        self.dirty = true;
        remote::spawn_albums(connection.client, generation, move |event| {
            events.send(AppEvent::Remote(event)).is_ok()
        });
    }

    /// Drop everything fetched from the server, and stop listening for more.
    ///
    /// Bumping the generation is the point: a walk that is still running cannot
    /// re-populate a list the Deck has just said it no longer has.
    fn forget_remote(&mut self) {
        // A search is a question *of this connection*. Bumping its generation
        // here is what stops an answer to a query asked over a connection the
        // Deck has since dropped from landing in a pane that no longer has one.
        self.search.generation = self.search.generation.wrapping_add(1);
        self.search.results = remote::Results::default();
        self.search.state = SearchState::Idle;
        self.search.detail = DetailState::Idle;
        self.search.view = View::Albums;
        self.search.cursor = 0;
        self.search.track_cursor = 0;
        self.search.query.clear();
        self.remote_generation = self.remote_generation.wrapping_add(1);
        self.remote = remote::Library::default();
        self.remote_state = RemoteState::Idle;
        self.remote_detail = DetailState::Idle;
        self.remote_view = View::Albums;
        self.remote_album_cursor = 0;
        self.remote_track_cursor = 0;
        self.set_remote_badge();
    }

    /// Fetch the open remote album's tracks, unless they are already in hand.
    fn start_remote_tracks(&mut self) {
        let Some(album) = self.open_remote_album() else {
            return;
        };
        if album.tracks.is_some() {
            self.remote_detail = DetailState::Idle;
            return;
        }
        let id = album.id.clone();
        let (Some(connection), Some(events)) = (self.connection.clone(), self.events.clone())
        else {
            // Nothing to ask through. Say so rather than spinning forever.
            self.remote_detail = DetailState::Failed("not connected".to_string());
            return;
        };
        self.remote_detail = DetailState::Loading;
        self.dirty = true;
        remote::spawn_tracks(
            connection.client,
            self.remote_generation,
            id,
            move |event| events.send(AppEvent::Remote(event)).is_ok(),
        );
    }

    /// The server row's right-aligned album count, or none while there is
    /// nothing to count.
    fn set_remote_badge(&mut self) {
        let count = self.remote.albums.len();
        if let Some(row) = self.remote_row() {
            self.sources[row].badge = if count == 0 {
                None
            } else {
                Some(count.to_string())
            };
        }
    }

    /// Fold a page of albums, or one album's tracks, in.
    ///
    /// Anything stamped with an older generation is **dropped**: `r` can start a
    /// second walk while the first still has pages in flight, and appending both
    /// would list every album twice.
    fn on_remote(&mut self, event: RemoteEvent) {
        if event.generation != self.remote_generation {
            return;
        }
        self.dirty = true;
        match event.kind {
            RemoteKind::Page(albums) => {
                // Append only. Indices already handed to `remote_view` and the
                // cursors stay valid, so a list can grow under an open album.
                self.remote.albums.extend(albums);
                self.remote_state = RemoteState::Loading;
                self.set_remote_badge();
            }
            RemoteKind::AlbumsReady => {
                self.remote_state = RemoteState::Ready;
                self.set_remote_badge();
            }
            // The pages that did arrive are kept: showing four of ten pages and
            // saying so beats showing none.
            RemoteKind::AlbumsFailed(message) => {
                self.remote_state = RemoteState::Failed(message);
            }
            RemoteKind::Tracks { album_id, tracks } => {
                if self.remote.set_tracks(&album_id, tracks)
                    && self.open_remote_album().is_some_and(|a| a.id == album_id)
                {
                    self.remote_detail = DetailState::Idle;
                }
            }
            // Only reported when it is the album on screen; a stale failure for
            // an album the user has already left is not news.
            RemoteKind::TracksFailed { album_id, message } => {
                if self.open_remote_album().is_some_and(|a| a.id == album_id) {
                    self.remote_detail = DetailState::Failed(message);
                }
            }
        }
        self.context = self.describe_context();
    }

    /// The right-hand title on the top border. Follows the selected source.
    fn describe_context(&self) -> String {
        match self.source() {
            Source::Local => {
                let root = self.library.root_name().unwrap_or("local");
                if self.library.is_empty() {
                    format!("{root} · empty")
                } else {
                    format!(
                        "{root} · {} albums · {} tracks",
                        self.library.albums.len(),
                        self.library.track_count()
                    )
                }
            }
            Source::Remote => {
                let name = self.selected_label();
                let albums = self.remote.albums.len();
                match &self.remote_state {
                    // The connection state is the interesting fact here, and it
                    // is the one the sidebar and the footer are also carrying.
                    RemoteState::Idle => format!("{name} · not connected"),
                    RemoteState::Loading => format!("{name} · loading · {albums} albums"),
                    RemoteState::Ready if self.remote.is_empty() => format!("{name} · empty"),
                    RemoteState::Ready => format!("{name} · {albums} albums"),
                    // Not "empty": it stopped early, and whatever arrived stands.
                    RemoteState::Failed(_) => format!("{name} · incomplete · {albums} albums"),
                }
            }
            Source::Search => match &self.search.state {
                _ if self.search.input.is_some() => "search · typing".to_string(),
                SearchState::Idle => "search · nothing asked".to_string(),
                SearchState::Searching => format!("search · {}…", self.search.query),
                SearchState::Ready => format!(
                    "search · {} · {} results",
                    self.search.query,
                    self.search.results.len()
                ),
                SearchState::Failed(_) => format!("search · {} · failed", self.search.query),
            },
            Source::Queue => match (self.queue.len(), self.queue.position()) {
                (0, _) => "queue · empty".to_string(),
                (n, Some(at)) => format!("queue · {} of {n}", at + 1),
                (n, None) => format!("queue · {n} tracks"),
            },
        }
    }

    // ── search ───────────────────────────────────────────────────────────

    /// `/` — open the search input, and put the cursor where the answer will
    /// appear.
    ///
    /// Refused, out loud, with no server: `search3` is a request, and a text
    /// input that can only ever be cancelled is worse than a key that says why.
    fn open_search(&mut self) {
        let Some(row) = self.search_row() else {
            self.set_status("no server configured · search asks one");
            return;
        };
        self.selected = row;
        self.focus = Focus::Main;
        // Always empty, never prefilled with the last query. A prefilled box has
        // to be *cleared* before it can be retyped, and `/` is pressed to ask
        // something new far more often than to re-ask the last thing.
        self.search.input = Some(String::new());
        self.context = self.describe_context();
        self.dirty = true;
    }

    // ── adding a server ──────────────────────────────────────────────────

    /// Where `config.toml` is, or `None` when nothing may be written.
    ///
    /// `None` in every test build — see the field — so a test that walks the
    /// whole add-server flow cannot append a `[[servers]]` block to the
    /// developer's own config.
    #[must_use]
    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config_path.as_deref()
    }

    /// `enter` on the server row in the sidebar. **The entry point the whole
    /// feature exists for.**
    ///
    /// Three states, three different next moves:
    ///
    /// * no server at all — open the panel and collect one;
    /// * a server with no usable password ([`ConnState::NeedsPassword`], and
    ///   [`ConnState::Failed`], because "the server said no" is most often the
    ///   password too) — skip the panel, the fields are already right, and go
    ///   straight to the prompt;
    /// * connected or connecting — open the panel to add *another*, which the
    ///   panel says plainly is written to the file and not switched to.
    fn open_server_row(&mut self) {
        match (&self.server, &self.conn) {
            (Some(server), ConnState::NeedsPassword { .. } | ConnState::Failed { .. }) => {
                self.prompt = Some(Prompt::SignIn(server.clone()));
                self.dirty = true;
            }
            (server, _) => {
                let second = server.is_some();
                self.add = Some(AddServer {
                    second,
                    ..AddServer::default()
                });
                // One thing owns the body — the same rule the device picker
                // follows, for the same reason.
                self.eq_open = false;
                self.device_open = false;
                self.help_open = false;
                if self.visualiser {
                    self.close_visualiser();
                }
                self.dirty = true;
            }
        }
    }

    /// Offer a key to the add-server panel. `true` when it was consumed.
    ///
    /// The **same line editor the search input uses** — [`crate::line`] — so
    /// backspace, the paste gate and the length cap behave identically in both
    /// places rather than being written twice with two answers.
    ///
    /// `CONTROL` is declined so `Ctrl-C` still quits, exactly as
    /// [`App::edit_key`] declines it.
    fn add_key(&mut self, key: KeyEvent) -> bool {
        if self.add.is_none() {
            return false;
        }
        if key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return false;
        }
        self.dirty = true;
        // Tab and the arrows walk the fields. `enter` walks them too until the
        // last one, where it submits — so the panel can be filled in and sent
        // without the user's hands leaving the letters.
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                if let Some(add) = self.add.as_mut() {
                    add.field = add.field.next();
                }
                return true;
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(add) = self.add.as_mut() {
                    add.field = add.field.previous();
                }
                return true;
            }
            _ => {}
        }
        let Some(add) = self.add.as_mut() else {
            return true;
        };
        let field = add.field;
        let max = field.max();
        match crate::line::edit(add.value_mut(field), key, max) {
            crate::line::LineStep::Continue => {
                // Typing into a field is the answer to whatever was wrong with
                // it, so the complaint comes down as soon as it is being fixed.
                if add.error_for(field).is_some() {
                    add.error = None;
                }
            }
            crate::line::LineStep::Submit if field == Field::Username => self.submit_add_server(),
            crate::line::LineStep::Submit => add.field = field.next(),
            crate::line::LineStep::Cancel => {
                self.add = None;
            }
            // Not text, and nothing here is bound to it — swallowed rather than
            // passed through, so an unmapped key cannot move a cursor behind an
            // open panel.
            crate::line::LineStep::Unhandled => {}
        }
        true
    }

    /// A paste into the panel. Text only, through the same gate as everything
    /// else typed.
    fn add_paste(&mut self, text: &str) -> bool {
        let Some(add) = self.add.as_mut() else {
            return false;
        };
        let field = add.field;
        let max = field.max();
        crate::line::type_into(add.value_mut(field), text, max);
        add.error = None;
        self.dirty = true;
        true
    }

    /// `enter` on the last field — check the three values and ask for the
    /// password.
    ///
    /// A failure parks the cursor **on the field that is wrong** and leaves the
    /// panel open, because the fix is right there. Only a clean set of three
    /// becomes a [`Prompt`], so the password prompt is never raised for a
    /// server that could not have been saved anyway.
    fn submit_add_server(&mut self) {
        let Some(add) = self.add.as_ref() else {
            return;
        };
        // Checked against [`App::server_names`] here rather than in `main`,
        // because `main`'s next move is the keychain and the keychain is where
        // the damage was: a duplicate name overwrote a working password before
        // anything on screen had a chance to object.
        let checked = add.validate(&self.server_names);
        let Some(add) = self.add.as_mut() else {
            return;
        };
        match checked {
            Err((field, message)) => {
                add.field = field;
                add.error = Some((field, message));
            }
            Ok((server, fixes)) => {
                self.add = None;
                for fix in fixes {
                    self.set_status(fix);
                }
                self.prompt = Some(Prompt::Add(server));
            }
        }
        self.dirty = true;
    }

    /// Take the job `main` has to do outside the terminal takeover, if there is
    /// one. Called once per fold, after the event has been handled.
    #[must_use]
    pub fn take_prompt(&mut self) -> Option<Prompt> {
        self.prompt.take()
    }

    /// Fold the result of a password prompt back in.
    ///
    /// **The password is not a parameter and never was.** By the time this runs
    /// the secret is in the keychain and out of the process's hands; what comes
    /// back is a [`ServerConfig`], which the file already holds in plain text,
    /// and some sentences.
    pub fn finish_prompt(&mut self, outcome: PromptOutcome) {
        self.dirty = true;
        match outcome {
            PromptOutcome::Cancelled => self.set_status("cancelled · nothing was stored"),
            PromptOutcome::Failed(message) => self.set_status(message),
            PromptOutcome::Stored { server, notes } => {
                // The first server becomes *the* server, and the sidebar grows
                // the rows that go with it. A second one is written to the file
                // and left there — `Config::server` is still the first entry, and
                // saying so beats a row that browses somebody else's library.
                let first = self.server.is_none();
                // The name is spoken for from here, whether or not this is the
                // server the fold uses: the keychain now has an entry under it.
                // Without this line the panel would let the *same session* add
                // it twice and overwrite the password it just stored.
                if !self.server_names.contains(&server.name) {
                    self.server_names.push(server.name.clone());
                }
                if first {
                    self.server = Some(server.clone());
                    self.sources = build_sources(Some(&server));
                    self.selected = self.selected.min(self.sources.len() - 1);
                }
                let mut note = notes.join(" · ");
                if first {
                    // Straight into a connection attempt: the whole point of
                    // doing this in the app is that nothing has to be restarted.
                    self.start_connect();
                } else {
                    if !note.is_empty() {
                        note.push_str(" · ");
                    }
                    note.push_str(&format!(
                        "{} is in the config · EKO still uses {}",
                        server.name,
                        self.server.as_ref().map_or("", |s| s.name.as_str())
                    ));
                }
                if !note.is_empty() {
                    self.set_status(note);
                }
                self.context = self.describe_context();
            }
        }
    }

    /// Offer a key to the line editor. `true` when it was consumed.
    ///
    /// **Nothing here touches the network.** Typing mutates a `String`; the
    /// request leaves on `Enter`, from [`App::run_search`], on a worker thread.
    /// That is the whole answer to "a keystroke must never wait on the network" —
    /// there is no debounce timer to get wrong because there is no per-keystroke
    /// request to debounce.
    fn edit_key(&mut self, key: KeyEvent) -> bool {
        if self.search.input.is_none() {
            return false;
        }
        // Declined, so the keymap sees it: `Ctrl-C` has to keep quitting.
        if key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return false;
        }
        let Some(input) = self.search.input.as_mut() else {
            return true;
        };
        self.dirty = true;
        // The same editor the add-server panel uses — see [`crate::line`].
        match crate::line::edit(input, key, MAX_QUERY) {
            crate::line::LineStep::Continue => {}
            crate::line::LineStep::Submit => self.run_search(),
            // Cancel. The results already on screen stay: `esc` undoes the
            // *typing*, not the last answer.
            crate::line::LineStep::Cancel => {
                self.search.input = None;
                self.context = self.describe_context();
            }
            // Anything else is not text and is not bound while typing. Consumed
            // rather than passed through, so an arrow key cannot quietly move a
            // cursor behind an open input.
            crate::line::LineStep::Unhandled => {}
        }
        true
    }

    /// A paste, from bracketed paste. Text only, and only while typing.
    ///
    /// The add-server panel gets first refusal, for the same reason it gets
    /// first refusal on keys: a URL is the field in this application most likely
    /// to arrive on the clipboard rather than through the keyboard.
    fn on_paste(&mut self, text: &str) {
        if self.add_paste(text) {
            return;
        }
        if self.search.input.is_none() {
            return;
        }
        self.type_into_query(text);
    }

    /// Append `text` to the open query, keeping it renderable.
    ///
    /// **The query is echoed** — in the summary row, and in
    /// `"nothing matches …"`. A paste is the one way a control character can
    /// reach it, so it goes through [`crate::line::type_into`], which shares the
    /// character gate with every server-written string rather than keeping a
    /// second copy of the charset. The length cap is [`MAX_QUERY`]; a query that
    /// would overflow it is truncated rather than refused, because a paste that
    /// silently did nothing would read as a broken terminal.
    fn type_into_query(&mut self, text: &str) {
        let Some(input) = self.search.input.as_mut() else {
            return;
        };
        crate::line::type_into(input, text, MAX_QUERY);
        self.dirty = true;
    }

    /// Send the typed query, on a worker thread.
    ///
    /// Bumping the generation **first** is the point, and it is the same rule
    /// [`App::start_remote_albums`] follows: a slow answer to the previous query
    /// is already stamped with the old number by the time this returns, so
    /// [`App::on_search`] drops it instead of overwriting the newer results.
    fn run_search(&mut self) {
        let query = self.search.input.take().unwrap_or_default();
        let query = query.trim().to_string();
        self.context = self.describe_context();
        self.dirty = true;
        if query.is_empty() {
            // Nothing was typed. Closing the box is the whole effect — asking the
            // server for "" would return the library, slowly, and nobody meant it.
            return;
        }
        // Every query invalidates the last, answered or not.
        self.search.generation = self.search.generation.wrapping_add(1);
        self.search.query = query.clone();
        self.search.results = remote::Results::default();
        self.search.view = View::Albums;
        self.search.cursor = 0;
        self.search.track_cursor = 0;
        self.search.detail = DetailState::Idle;
        let (Some(connection), Some(events)) = (self.connection.clone(), self.events.clone())
        else {
            self.search.state = SearchState::Failed("not connected".to_string());
            return;
        };
        self.search.state = SearchState::Searching;
        let generation = self.search.generation;
        remote::spawn_search(connection.client, generation, query, move |event| {
            events.send(AppEvent::Search(event)).is_ok()
        });
    }

    /// Fetch an open *result* album's tracks, unless they are already in hand.
    fn start_search_tracks(&mut self) {
        let Some(album) = self.search.open_album() else {
            return;
        };
        if album.tracks.is_some() {
            self.search.detail = DetailState::Idle;
            return;
        }
        let id = album.id.clone();
        let (Some(connection), Some(events)) = (self.connection.clone(), self.events.clone())
        else {
            self.search.detail = DetailState::Failed("not connected".to_string());
            return;
        };
        self.search.detail = DetailState::Loading;
        self.dirty = true;
        remote::spawn_search_tracks(
            connection.client,
            self.search.generation,
            id,
            move |event| events.send(AppEvent::Search(event)).is_ok(),
        );
    }

    /// Fold a search result in.
    ///
    /// **The stale-answer gate.** Anything stamped with a generation the fold has
    /// moved past is dropped — a second query, a reconnect, or a disconnect has
    /// happened since it left, and an older answer replacing a newer one is the
    /// list claiming to be about a question nobody asked.
    fn on_search(&mut self, event: SearchEvent) {
        if event.generation != self.search.generation {
            return;
        }
        self.dirty = true;
        match event.kind {
            SearchKind::Results(results) => {
                self.search.results = *results;
                self.search.state = SearchState::Ready;
                self.search.cursor = 0;
            }
            SearchKind::Failed(message) => {
                self.search.state = SearchState::Failed(message);
            }
            SearchKind::Tracks { album_id, tracks } => {
                if let Some(album) = self
                    .search
                    .results
                    .albums
                    .iter_mut()
                    .find(|a| a.id == album_id)
                {
                    album.tracks = Some(tracks);
                }
                if self.search.open_album().is_some_and(|a| a.id == album_id) {
                    self.search.detail = DetailState::Idle;
                }
            }
            SearchKind::TracksFailed { album_id, message } => {
                if self.search.open_album().is_some_and(|a| a.id == album_id) {
                    self.search.detail = DetailState::Failed(message);
                }
            }
        }
        self.context = self.describe_context();
    }

    /// `Enter` on a row of the results list.
    ///
    /// **An album row opens; a song row plays.** That is the same sentence
    /// [`App::open_under_cursor`] already writes for both libraries, which is why
    /// there is no second key and no second keymap: the list is mixed, so the row
    /// decides, not the user.
    fn activate_search_row(&mut self, row: usize) {
        if row < self.search.results.albums.len() {
            self.search.view = View::Album(row);
            self.search.track_cursor = 0;
            self.start_search_tracks();
            self.dirty = true;
            return;
        }
        if let Some((index, _)) = self.search.song_at(row) {
            self.play_search_songs(index);
        }
    }

    /// Play song `index` of the results, making the **matched songs** the queue.
    ///
    /// `Enter` in a list means "play this list", exactly as it does in an album —
    /// see [`App::play`]. Here the list is the songs the server matched, which is
    /// the one ordering the user actually asked for.
    fn play_search_songs(&mut self, index: usize) {
        let entries = self.search_song_entries();
        match entries.get(index) {
            None => return,
            Some(entry) if !self.can_start(entry) => {
                self.note_no_client();
                return;
            }
            Some(_) => {}
        }
        if self.queue.replace(entries, index) {
            self.set_queue_badge();
            self.play_current();
        }
    }

    /// The matched songs as queue entries, in the order the server ranked them.
    ///
    /// Every entry carries [`NO_ROW`]: a result is at no index in either library,
    /// and a made-up one would light the `▶` on an unrelated row. See
    /// [`crate::queue::NO_ROW`].
    fn search_song_entries(&self) -> Vec<Entry> {
        self.search
            .results
            .songs
            .iter()
            .map(|t| Entry {
                media: Media::Remote(t.id.clone()),
                album: NO_ROW,
                track: NO_ROW,
                title: t.title.clone(),
                artist: t.artist.clone(),
                album_name: t.album.clone(),
                dur_ms: (t.duration * 1000.0).max(0.0) as u64,
            })
            .collect()
    }

    /// One *result* album's tracks as queue entries. Same [`NO_ROW`] rule.
    fn search_album_entries(&self, album: usize) -> Vec<Entry> {
        let Some(item) = self.search.results.albums.get(album) else {
            return Vec::new();
        };
        let Some(tracks) = item.tracks.as_ref() else {
            return Vec::new();
        };
        tracks
            .iter()
            .map(|t| Entry {
                media: Media::Remote(t.id.clone()),
                album: NO_ROW,
                track: NO_ROW,
                title: t.title.clone(),
                artist: t.artist.clone(),
                album_name: item.name.clone(),
                dur_ms: (t.duration * 1000.0).max(0.0) as u64,
            })
            .collect()
    }

    /// Play track `track` of result album `album`, making that album the queue.
    fn play_search_album(&mut self, album: usize, track: usize) {
        let entries = self.search_album_entries(album);
        match entries.get(track) {
            None => return,
            Some(entry) if !self.can_start(entry) => {
                self.note_no_client();
                return;
            }
            Some(_) => {}
        }
        if self.queue.replace(entries, track) {
            self.set_queue_badge();
            self.play_current();
        }
    }

    /// Dispatch a key through the one keymap. See [`crate::keys`].
    ///
    /// The search input gets first refusal: while it is open a key is *text*, and
    /// text is not an [`Action`]. It declines anything with `CONTROL` held, so
    /// `Ctrl-C` still quits mid-query rather than typing a `c`.
    fn on_key(&mut self, key: KeyEvent) {
        // Windows sends both Press and Release; only act on Press. Repeat is a
        // held key and should act.
        if key.kind == KeyEventKind::Release {
            return;
        }
        // The panel is a form, so while it is up a key is *text* before it is
        // anything else — the same first-refusal rule the search input has, and
        // it declines `CONTROL` for the same reason.
        if self.add_key(key) {
            return;
        }
        if self.edit_key(key) {
            return;
        }
        let Some(action) = keys::action_for(key) else {
            return;
        };
        self.act(action);
    }

    /// Apply one resolved action. The keymap says *what*; this says *where*.
    pub fn act(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Main,
                    Focus::Main => Focus::Sidebar,
                };
                self.dirty = true;
            }
            // With the keymap on screen there is no list under it to move a cursor
            // in, so `j`/`k` are swallowed rather than moving something invisible.
            // First, because only one overlay is ever open (see
            // [`App::open_devices`]) and this is the one that covers the whole
            // body.
            Action::Down | Action::Up if self.help_open => {}
            // The picker is a list, so down and up are the list's — the same verb
            // on the nearest instrument, which is the rule the EQ arms below
            // follow too.
            Action::Down if self.device_open => self.move_device_cursor(1),
            Action::Up if self.device_open => self.move_device_cursor(-1),
            // With the EQ panel up there is no list on screen to move a cursor
            // in, so down and up are the slider. Same verb, different instrument
            // — which is the fold deciding what an action means where it is,
            // exactly as [`crate::keys`] describes.
            Action::Down if self.eq_open => self.nudge_eq(-eq::GAIN_STEP),
            Action::Up if self.eq_open => self.nudge_eq(eq::GAIN_STEP),
            Action::Down => match self.focus {
                Focus::Sidebar => self.move_selection(1),
                Focus::Main => self.move_cursor(1),
            },
            Action::Up => match self.focus {
                Focus::Sidebar => self.move_selection(-1),
                Focus::Main => self.move_cursor(-1),
            },
            // Nothing under the overlay is openable, and `enter` on a list you
            // cannot see would start a track you did not choose.
            Action::Open if self.help_open => {}
            Action::Open if self.device_open => self.choose_device(),
            // `enter` on the server row is the way in to server setup. Every
            // other sidebar row either discloses children or is a leaf, and
            // `toggle_expand` is inert on a leaf — which is what this row used
            // to be too, and why there was no way to configure a server from
            // inside the application at all.
            Action::Open => match self.focus {
                Focus::Sidebar if self.source() == Source::Remote => {
                    self.open_server_row();
                }
                Focus::Sidebar => self.toggle_expand(),
                Focus::Main => self.open_under_cursor(),
            },
            // Esc closes the panel before it means anything else, so the way out
            // is the key everything else in the Deck uses to back out.
            Action::Back if self.help_open => {
                self.help_open = false;
                self.dirty = true;
            }
            Action::Back if self.device_open => {
                self.device_open = false;
                self.dirty = true;
            }
            Action::Back if self.eq_open => {
                self.eq_open = false;
                self.dirty = true;
            }
            // **`esc` leaves the visualiser**, and it is this arm rather than a
            // second row in [`crate::keys::BINDINGS`] that makes it so. Last of
            // the four, because the three above are overlays and this is the
            // view under them — except that opening any of them closes it, so in
            // practice only one arm can ever match.
            Action::Back if self.visualiser => {
                self.close_visualiser();
            }
            Action::Back => self.back(),
            Action::PlayPause => self.play_pause(),
            Action::Next => self.step_track(1),
            Action::Previous => self.step_track(-1),
            Action::Enqueue => self.enqueue_under_cursor(),
            Action::Unqueue => self.unqueue_under_cursor(),
            Action::SeekBack => self.seek_by(-SEEK_STEP_SECS),
            Action::SeekForward => self.seek_by(SEEK_STEP_SECS),
            Action::ToggleSpectrum => {
                self.spectrum_visible = !self.spectrum_visible;
                self.dirty = true;
            }
            // `z` — the full-screen Now Playing view. See
            // [`crate::ui::visualiser`], and [`App::close_visualiser`] for why
            // closing it is a named function and opening it is three lines here.
            Action::Visualiser => {
                if self.visualiser {
                    self.close_visualiser();
                } else {
                    self.visualiser = true;
                    // One thing owns the body. The three overlays are drawn over
                    // the main pane, which does not exist while this is up, so a
                    // state holding both would be an overlay over nothing — and
                    // the alternative, a layering rule, would have to be got
                    // right in the renderer *and* in every guard in this match.
                    self.eq_open = false;
                    self.device_open = false;
                    self.help_open = false;
                    self.dirty = true;
                    // The cover's block just changed size and the envelope just
                    // became wanted. Both are recomputed after every event
                    // anyway; doing it here as well means the first frame of the
                    // view has already asked, rather than showing a placeholder
                    // until the next key or tick.
                    self.sync_art();
                    self.sync_wave();
                }
            }
            // One key, both sources. Without the reconnect half there is no way
            // out of a failed connection except restarting the process — which
            // is exactly what someone who has just run `eko-cli login` would have
            // to do.
            Action::Rescan => {
                self.start_scan();
                self.start_connect();
            }
            Action::Search => self.open_search(),
            Action::Devices => self.open_devices(),
            Action::Sleep => self.cycle_sleep(),
            // `?` — the whole keymap, from the one table. No state of its own: the
            // overlay reads [`crate::keys::BINDINGS`] and holds nothing.
            Action::Help => {
                self.help_open = !self.help_open;
                // One overlay at a time — see the [`Action::EqPanel`] arm.
                if self.help_open {
                    self.eq_open = false;
                    self.device_open = false;
                    // And the view under them. See [`Action::Visualiser`].
                    if self.visualiser {
                        self.close_visualiser();
                    }
                }
                self.dirty = true;
            }

            // ── the EQ ───────────────────────────────────────────────────
            //
            // The band, preset and on/off arms all go through [`App::edit_eq`],
            // which is the only edge that writes [`App::eq`] and the only thing
            // that pushes it to the engine. Opening the panel and moving the
            // cursor do not, because neither touches a sample.
            Action::EqPanel => {
                self.eq_open = !self.eq_open;
                // One overlay at a time. Two modals stacked would need a
                // precedence rule in the renderer *and* in every guard above;
                // opening one closing the others is the same rule stated once.
                if self.eq_open {
                    self.device_open = false;
                    self.help_open = false;
                    if self.visualiser {
                        self.close_visualiser();
                    }
                }
                self.dirty = true;
            }
            Action::EqToggle => self.edit_eq(|eq| eq.set_enabled(!eq.enabled())),
            Action::EqPrev => self.move_eq_cursor(-1),
            Action::EqNext => self.move_eq_cursor(1),
            Action::EqPresetPrev => self.edit_eq(eq::GraphicEq::prev_preset),
            Action::EqPresetNext => self.edit_eq(eq::GraphicEq::next_preset),
        }
    }

    /// Leave the visualiser, whether by `z` or by `esc`.
    ///
    /// A named function with two callers rather than two copies of four lines,
    /// because the two ways out **must** leave the same state: the cover's block
    /// shrinks back to the footer's eight cells and the envelope stops being
    /// wanted. A version that only ran on `z` would leave `esc` with a
    /// full-screen cover encoded for a slot that is now eight cells wide — the
    /// placeholder, until something else happened to move — and a decode still
    /// running for a view nobody is looking at.
    fn close_visualiser(&mut self) {
        self.visualiser = false;
        self.dirty = true;
        self.sync_art();
        self.sync_wave();
    }

    // ── the output device ────────────────────────────────────────────────

    /// `d` — show or hide the picker, listing the host's devices as it opens.
    ///
    /// The list is read here rather than kept up to date, because reading it
    /// opens the audio host: a DAC that was plugged in since launch appears the
    /// moment someone looks, and nothing polls CoreAudio on a 30fps tick.
    fn open_devices(&mut self) {
        self.device_open = !self.device_open;
        self.dirty = true;
        if !self.device_open {
            return;
        }
        // One overlay at a time — see [`Action::EqPanel`]'s arm.
        self.eq_open = false;
        self.help_open = false;
        if self.visualiser {
            self.close_visualiser();
        }
        self.set_devices(eko_core::engine::list_devices());
        // Park the cursor on the current *choice* — not on what is in use — so
        // `enter` straight away is a no-op rather than a change nobody asked for.
        // On a paused Deck those are two different rows, and parking on the row
        // the audio is on would turn `enter` into "configure that device", which
        // is a setting nobody typed.
        self.device_cursor = self.device_row();
    }

    /// Take a freshly read device list, and **resolve the choice against it
    /// again**.
    ///
    /// The two are one operation, and this is the only place the list is
    /// written after construction, because they cannot be allowed to come
    /// apart. [`OutputDevice::Missing`] is a statement about the *list*, not
    /// about the choice: `config.toml` still names the DAC, and all that changed
    /// when it was plugged in is that the host has started listing it. A list
    /// that refreshed without the resolution would put a selectable row for the
    /// device *and* the panel's amber `… is not connected` sentence on the same
    /// screen — the picker denying what the picker says — which is exactly what
    /// happened before this existed: `self.device` was written at construction
    /// and by [`App::set_output_device`] and nowhere else, so a `Missing` device
    /// was missing for the rest of the session however many times the list was
    /// re-read.
    ///
    /// It resolves in both directions, because both are the same fact: unplug
    /// the DAC and open the picker, and the row goes away as the sentence
    /// arrives.
    ///
    /// # What this is not
    ///
    /// Not a *choice*, so it does none of the three things [`App::set_output_device`]
    /// does beyond the first:
    ///
    /// * The engine's preference **does** follow, through
    ///   [`OutputDevice::engine_pref`] as always — it is `None` for a missing
    ///   device and the name for a present one, so leaving it behind would make
    ///   the panel's "from the next track" sentence a promise about a device the
    ///   engine has never been told about.
    /// * Nothing is persisted: the configured *name* has not changed, and
    ///   [`OutputDevice::configured`] is what gets written. A device that is
    ///   merely asleep must not be un-configured by having been absent once.
    /// * A playing track is **not** restarted. Nobody asked for a switch — a
    ///   cable moved — and a track that jumped back to its first frame because a
    ///   DAC was plugged in somewhere would be the application acting on its own.
    ///   The preference applies at the next start, which the panel says.
    fn set_devices(&mut self, devices: Vec<String>) {
        self.devices = devices;
        let resolved = config::resolve_output_device(
            self.device.configured().map(str::to_string),
            &self.devices,
        );
        if resolved == self.device {
            return;
        }
        self.device = resolved;
        self.engine.set_device(self.device.engine_pref());
    }

    /// The picker row the current **choice** sits on. Row 0 is the system default.
    ///
    /// This is the *preference*: what `config.toml` holds and what the next
    /// session will be opened from. It is **not** necessarily what the audio is
    /// coming out of right now — that is [`App::device_in_use_row`], and the
    /// difference is the whole point of there being two functions.
    ///
    /// A [`OutputDevice::Missing`] device is on **no** row, and cannot be: it is
    /// missing precisely because the list does not have it — [`App::set_devices`]
    /// resolves the two together every time the list is read — and the engine
    /// really is on the system default, so the cursor sits there, which is where
    /// the audio is. The panel names the missing device separately rather than
    /// inventing a row for it. See [`crate::ui::device_panel`].
    #[must_use]
    pub fn device_row(&self) -> usize {
        match &self.device {
            OutputDevice::Configured(name) => self
                .devices
                .iter()
                .position(|d| d == name)
                .map_or(0, |i| i + 1),
            OutputDevice::Default | OutputDevice::Missing(_) => 0,
        }
    }

    /// The picker row `▶` belongs on — **the device the audio is actually coming
    /// out of** — or `None` when there is no row that can honestly carry it.
    ///
    /// # Why this is not [`App::device_row`]
    ///
    /// `Engine::set_device` writes a preference that `decode_and_play` reads once
    /// per session, so a **paused** Deck whose preference has just been changed is
    /// still playing out of the old device — deliberately, and
    /// [`App::set_output_device`] says why. The seal knows that: it names
    /// [`App::stream`]'s `device`, which is the one the engine really opened. A
    /// `▶` taken from [`App::device_row`] would then be marking the *other* device
    /// on the same screen as the seal — the picker and the seal disagreeing about
    /// which DAC is playing, which is the exact class of bug this application
    /// exists in order not to have.
    ///
    /// So the marker is read out of [`App::stream`] — the *same* field the seal
    /// reads, not a copy kept alongside it — whenever there is one. The two are
    /// one fact rendered twice and cannot come apart.
    ///
    /// The two `None`-ish cases are different and are treated differently:
    ///
    /// * **No stream at all.** Then the seal is inactive and its chain reads
    ///   `— → —`: it names no device, so there is nothing for a marker to
    ///   contradict, and the preference is the only true statement left. The
    ///   preference's row is returned.
    /// * **A stream on a device the host is no longer listing.** Then there is no
    ///   row for it, and pointing at the preference instead would be the
    ///   disagreement all over again — so nothing is marked. `None`.
    #[must_use]
    pub fn device_in_use_row(&self) -> Option<usize> {
        match &self.stream {
            Some(info) => self
                .devices
                .iter()
                .position(|d| *d == info.device)
                .map(|i| i + 1),
            None => Some(self.device_row()),
        }
    }

    /// The chosen device when the engine is **not** on it yet, for the sentence
    /// the picker prints under the list. `None` when there is nothing to say.
    ///
    /// This is the other half of [`App::device_in_use_row`]: the marker stays
    /// where the audio is, and this names what was chosen and leaves the panel to
    /// say when it applies. Without it a paused Deck would show a `▶` somewhere
    /// the user did not just press `enter` on, with nothing on screen to explain
    /// why.
    ///
    /// Only [`OutputDevice::Configured`] can produce one:
    ///
    /// * [`OutputDevice::Missing`] already has a sentence of its own — it is not
    ///   pending, it is absent.
    /// * [`OutputDevice::Default`] cannot be *known* to disagree. Whatever the
    ///   engine opened for a session with no preference **is** the system default,
    ///   and nothing here can tell that apart from a session left over from an
    ///   earlier preference. A "from the next track" line printed on a guess would
    ///   be this crate inventing a claim, which is worse than saying nothing.
    #[must_use]
    pub fn device_pending(&self) -> Option<&str> {
        let OutputDevice::Configured(name) = &self.device else {
            return None;
        };
        let info = self.stream.as_ref()?;
        (info.device != *name).then_some(name.as_str())
    }

    /// How many rows the picker has: the system default, then one per device.
    #[must_use]
    pub fn device_rows(&self) -> usize {
        self.devices.len() + 1
    }

    fn move_device_cursor(&mut self, delta: isize) {
        let last = self.device_rows().saturating_sub(1);
        let next = (self.device_cursor as isize + delta).clamp(0, last as isize) as usize;
        if next != self.device_cursor {
            self.device_cursor = next;
            self.dirty = true;
        }
    }

    /// `enter` in the picker — take the row under the cursor.
    fn choose_device(&mut self) {
        let choice = match self.device_cursor.checked_sub(1) {
            None => OutputDevice::Default,
            Some(i) => match self.devices.get(i) {
                Some(name) => OutputDevice::Configured(name.clone()),
                // The list moved under the cursor. Nothing to select, and
                // guessing would be worse than doing nothing.
                None => return,
            },
        };
        self.device_open = false;
        self.dirty = true;
        self.set_output_device(choice);
    }

    /// **The only place a device is *chosen*.**
    ///
    /// One other place writes [`App::device`] — [`App::set_devices`], which
    /// re-resolves the same configured name against a freshly read list when a
    /// DAC is plugged in or unplugged. That is not a choice and does none of the
    /// three things below beyond handing the engine its preference; its own doc
    /// comment says why.
    ///
    /// Three things have to happen together here, and this is why they are one
    /// function:
    ///
    /// 1. the engine's preference changes — through
    ///    [`OutputDevice::engine_pref`], so a missing device is never handed over;
    /// 2. the choice is persisted, so it survives the process; and
    /// 3. **the session is restarted, when there is one.**
    ///
    /// # Why (3) is not optional, and what it means for the seal
    ///
    /// `Engine::set_device` writes a *preference*. `decode_and_play` reads it once,
    /// at `play`/`play_url`, and the output stream it opened from it lives for the
    /// life of the session — so a running track keeps coming out of the old device
    /// no matter what this writes. A picker that stopped after (1) would move the
    /// label and not the audio, and `EngineStatus::dev_rate` would keep reporting
    /// the old device: the seal would stay correct about a device the screen no
    /// longer claims to be using, which is the seal and the UI disagreeing.
    ///
    /// So a playing Deck restarts the current entry on the new device. The seal
    /// follows for free and by construction: [`App::started`] drops
    /// [`App::stream`], the fresh session reports `src_rate: 0, dev_rate: 0` until
    /// the device is actually open, and [`stream_info`] refuses that — so the lamp
    /// reads the hollow `○ UNVERIFIED` across the switch rather than carrying the
    /// old device's verdict onto the new one. When the new device's rate lands it
    /// is `derive`'s input, so a 44.1 kHz file on a device locked to 48 kHz reads
    /// `RESAMPLED` the moment the switch completes.
    ///
    /// A **paused or stopped** Deck is not restarted, and that is not a shortcut:
    /// its session — if any — is still on the old device, the seal still describes
    /// that device, and both statements are true. The preference applies at the
    /// next start, which the note says.
    fn set_output_device(&mut self, choice: OutputDevice) {
        if choice == self.device {
            return;
        }
        self.device = choice;
        self.engine.set_device(self.device.engine_pref());
        let name = match self.device.configured() {
            Some(name) => name.to_string(),
            None => "the system default".to_string(),
        };
        let persisted = self.persist_device();
        if self.playback == Playback::Playing && self.queue.current().is_some() {
            // Restarts from the top of the track: `Engine::seek` on a session
            // that has not reported a rate resolves to frame zero, and a stream
            // seeked past what has downloaded parks in silence — see
            // [`App::seek_by`]. Saying so beats a clock that jumps.
            self.play_current();
            // `started` clears the note, so this goes after it.
            self.set_status(format!("output → {name} · restarted the track{persisted}"));
        } else {
            self.set_status(format!("output → {name} · from the next track{persisted}"));
        }
    }

    /// Write the choice to `config.toml`, and say so when it could not be.
    ///
    /// Returns the tail of the footer note: empty when it was written, a reason
    /// when it was not. A setting that silently fails to persist is a setting that
    /// comes back wrong at the next launch with nothing to explain it.
    fn persist_device(&self) -> String {
        let Some(path) = self.config_path.as_deref() else {
            return " · not saved: no config file".to_string();
        };
        match config::write_output_device(path, self.device.configured()) {
            Ok(()) => String::new(),
            Err(e) => format!(" · not saved: {e}"),
        }
    }

    // ── the sleep timer ──────────────────────────────────────────────────

    /// `t` — walk [`SLEEP_STEPS`] and then switch off.
    fn cycle_sleep(&mut self) {
        let next = match &self.sleep {
            None => SLEEP_STEPS.first().copied(),
            Some(sleep) => SLEEP_STEPS
                .iter()
                .position(|m| *m == sleep.minutes)
                .and_then(|i| SLEEP_STEPS.get(i + 1))
                .copied(),
        };
        self.sleep = next.map(Sleep::new);
        self.set_status(match next {
            Some(minutes) => format!("sleep timer · {minutes} min"),
            None => "sleep timer · off".to_string(),
        });
        self.dirty = true;
    }

    /// Bring the countdown down by the time that has really passed, and stop
    /// playback when it runs out.
    ///
    /// Measured against the clock rather than counted in ticks, because a tick is
    /// 33ms with the spectrum up and a second without it — see [`tick_interval`].
    /// It only runs down while something is playing: a timer that emptied itself
    /// during a pause would stop a Deck that was already stopped, and the number
    /// on screen — which only redraws on a tick, and ticks only happen while
    /// playing — would have been wrong the whole time it was paused.
    fn tick_sleep(&mut self) {
        let Some(sleep) = self.sleep.as_mut() else {
            return;
        };
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(sleep.last);
        sleep.last = now;
        if self.playback != Playback::Playing {
            return;
        }
        sleep.remaining = sleep.remaining.saturating_sub(elapsed);
        if !sleep.remaining.is_zero() {
            self.dirty = true;
            return;
        }
        let minutes = sleep.minutes;
        self.sleep = None;
        // Really stopped, not paused: the timer's promise is silence.
        self.engine.stop();
        self.playback = Playback::Stopped;
        self.bands.clear();
        // The caps go with them: a held peak over a stopped transport is a
        // picture of something that is not happening.
        self.peaks.clear();
        // The session is gone, so there is nothing left to describe. Without this
        // the seal would keep the ended session's rates until the next poll.
        self.stream = None;
        self.set_status(format!("sleep timer · stopped after {minutes} min"));
        self.dirty = true;
    }

    /// Restart the countdown's clock, so time spent not playing is not spent.
    ///
    /// Called from the two places [`App::playback`] becomes
    /// [`Playback::Playing`] — [`App::started`] and the resume arm of
    /// [`App::play_pause`]. Without it the first tick after a pause would subtract
    /// the whole pause.
    fn touch_sleep(&mut self) {
        if let Some(sleep) = self.sleep.as_mut() {
            sleep.last = Instant::now();
        }
    }

    // ── the EQ ───────────────────────────────────────────────────────────

    /// The graphic EQ, for the panel to draw. Read-only on purpose — see
    /// [`App::edit_eq`].
    #[must_use]
    pub fn eq(&self) -> &eq::GraphicEq {
        &self.eq
    }

    /// **The only place [`App::eq`] is written.**
    ///
    /// Every change to the EQ is a change to two things at once: what the engine
    /// applies, and what the seal claims. Funnelling them through one function
    /// is what makes "the seal reports what was sent" a property of the code
    /// rather than a habit — a new EQ key added in the wrong place fails to
    /// compile against a private field long before it can ship a lying seal.
    fn edit_eq(&mut self, change: impl FnOnce(&mut eq::GraphicEq)) {
        change(&mut self.eq);
        self.apply_eq();
        self.dirty = true;
    }

    /// Hand the EQ to the engine. **The only caller of `Engine::set_eq` in this
    /// crate**, and its arguments come from
    /// [`crate::eq::GraphicEq::engine_args`] and nowhere else.
    ///
    /// That is the whole anti-drift argument, and it is worth stating in full,
    /// because the engine cannot be interrogated: `EngineStatus` carries no DSP
    /// fields, so nothing downstream can *check* that what was applied is what
    /// is claimed. What can be guaranteed instead is that there is only one set
    /// of numbers. `engine_args` and `seal_state` are two pure views of the same
    /// three private fields; this is the only call site of the first; and
    /// [`App::seal_input`] is the only call site of the second.
    ///
    /// The remaining gap is temporal — a push that never happens leaves the
    /// engine flat while the seal says `EQ`. That is the *over*-reporting
    /// direction, which is the safe one, and it is closed anyway by
    /// [`App::edit_eq`] and by the re-push in [`App::started`].
    fn apply_eq(&mut self) {
        let (enabled, preamp, gains) = self.eq.engine_args();
        self.engine.set_eq(enabled, preamp, gains);
        self.eq_applied = self.eq_applied.wrapping_add(1);
    }

    /// Move the panel's cursor, clamped rather than wrapped.
    ///
    /// Clamped because the columns are a physical row of sliders: running off
    /// the 16k end and reappearing at the pre-amp is not what a slider does.
    fn move_eq_cursor(&mut self, delta: isize) {
        let last = eq::COLUMNS - 1;
        let next = (self.eq_cursor as isize + delta).clamp(0, last as isize) as usize;
        if next != self.eq_cursor {
            self.eq_cursor = next;
            self.dirty = true;
        }
    }

    /// Move the selected column by `delta` dB.
    fn nudge_eq(&mut self, delta: f32) {
        let col = self.eq_cursor.min(eq::COLUMNS - 1);
        self.edit_eq(|eq| eq.nudge(col, delta));
    }

    fn toggle_expand(&mut self) {
        if let Some(row) = self.sources.get_mut(self.selected) {
            if row.expandable {
                row.expanded = !row.expanded;
                self.dirty = true;
            }
        }
    }

    /// Enter on an album opens it; Enter on a track plays it.
    ///
    /// Identical on both sources — this is [`Action::Open`] from the one keymap,
    /// and the only difference is which list it looks in. The remote branch also
    /// starts the track fetch, because a remote album does not carry its tracks
    /// until someone asks.
    fn open_under_cursor(&mut self) {
        match (self.source(), self.view()) {
            (Source::Local, View::Albums) => {
                if self.album_cursor < self.library.albums.len() {
                    self.view = View::Album(self.album_cursor);
                    self.track_cursor = 0;
                    self.dirty = true;
                }
            }
            (Source::Local, View::Album(album)) => {
                self.play(Source::Local, album, self.track_cursor);
            }
            (Source::Remote, View::Albums) => {
                if self.remote_album_cursor < self.remote.albums.len() {
                    self.remote_view = View::Album(self.remote_album_cursor);
                    self.remote_track_cursor = 0;
                    self.start_remote_tracks();
                    self.dirty = true;
                }
            }
            (Source::Remote, View::Album(album)) => {
                self.play(Source::Remote, album, self.remote_track_cursor);
            }
            (Source::Search, View::Albums) => self.activate_search_row(self.search.cursor),
            (Source::Search, View::Album(album)) => {
                self.play_search_album(album, self.search.track_cursor);
            }
            // Enter in the Queue pane jumps to that entry **without rebuilding
            // the list** — the queue is already the list, and replacing it with
            // whatever album the entry came from would be the opposite of what
            // was asked for.
            (Source::Queue, _) => self.play_queue_at(self.cursor()),
        }
    }

    /// `a` — put whatever the cursor is on at the end of the queue.
    ///
    /// On an album that is the whole album, on a track just that track. Never
    /// disturbs what is playing: [`crate::queue::Queue::push`] does not touch the
    /// position, so queueing behind a running track is exactly that.
    ///
    /// It always says what it did. A key whose entire effect is a number in the
    /// sidebar changing by three is a key most people will conclude did nothing.
    fn enqueue_under_cursor(&mut self) {
        let (entries, what) = match (self.source(), self.view()) {
            (Source::Local, View::Albums) => (
                self.local_album_entries(self.album_cursor),
                self.library
                    .albums
                    .get(self.album_cursor)
                    .map(|a| a.name.clone()),
            ),
            (Source::Local, View::Album(album)) => {
                let entries = self.local_album_entries(album);
                let one = entries.get(self.track_cursor).cloned();
                (
                    one.iter().cloned().collect(),
                    one.map(|entry| entry.title.clone()),
                )
            }
            (Source::Remote, View::Albums) => (
                self.remote_album_entries(self.remote_album_cursor),
                self.remote
                    .album(self.remote_album_cursor)
                    .map(|a| a.name.clone()),
            ),
            (Source::Remote, View::Album(album)) => {
                let entries = self.remote_album_entries(album);
                let one = entries.get(self.remote_track_cursor).cloned();
                (
                    one.iter().cloned().collect(),
                    one.map(|entry| entry.title.clone()),
                )
            }
            // A result row is an album or a song, exactly as `Enter` reads it —
            // so `a` on an album queues the album, and `a` on a song queues the
            // one song.
            (Source::Search, View::Albums) => {
                let row = self.search.cursor;
                if row < self.search.results.albums.len() {
                    (
                        self.search_album_entries(row),
                        self.search.results.albums.get(row).map(|a| a.name.clone()),
                    )
                } else {
                    let songs = self.search_song_entries();
                    let one = self
                        .search
                        .song_at(row)
                        .and_then(|(index, _)| songs.get(index).cloned());
                    (
                        one.iter().cloned().collect(),
                        one.map(|entry| entry.title.clone()),
                    )
                }
            }
            (Source::Search, View::Album(album)) => {
                let entries = self.search_album_entries(album);
                let one = entries.get(self.search.track_cursor).cloned();
                (
                    one.iter().cloned().collect(),
                    one.map(|entry| entry.title.clone()),
                )
            }
            // The queue is not a place to add *from*, and adding an entry to the
            // list it is already in would be a duplicate nobody asked for.
            (Source::Queue, _) => (Vec::new(), None),
        };

        let Some(what) = what.filter(|_| !entries.is_empty()) else {
            // A remote album whose tracks have not landed is the common case, and
            // it has a fix: open it, which is what fetches them.
            if matches!(self.source(), Source::Remote | Source::Search)
                && self.view() == View::Albums
            {
                self.set_status("no tracks loaded yet · open the album first");
            }
            return;
        };
        let added = entries.len();
        for entry in entries {
            self.queue.push(entry);
        }
        self.set_queue_badge();
        self.set_status(match added {
            1 => format!("queued {what}"),
            n => format!("queued {n} tracks from {what}"),
        });
    }

    /// `x` — drop the entry under the cursor out of the queue.
    ///
    /// Only in the Queue pane: everywhere else there is no entry under the
    /// cursor, only a track that may or may not be in the queue several times
    /// over, and guessing which copy was meant is worse than doing nothing.
    fn unqueue_under_cursor(&mut self) {
        if self.source() != Source::Queue {
            return;
        }
        let at = self.cursor();
        let title = self.queue.get(at).map(|entry| entry.title.clone());
        let (Some(title), true) = (title, self.queue.remove(at)) else {
            // The one refusal worth explaining: the playing entry stays, because
            // it is what `current` means. See [`crate::queue::Queue::remove`].
            if self.queue.position() == Some(at) {
                self.set_status("that one is playing · press n first");
            }
            return;
        };
        self.queue_cursor = at.min(self.queue.len().saturating_sub(1));
        self.set_queue_badge();
        self.set_status(format!("removed {title}"));
        self.context = self.describe_context();
    }

    /// The queue row's right-aligned count, or none while it is empty.
    fn set_queue_badge(&mut self) {
        let count = self.queue.len();
        if let Some(row) = self.queue_row() {
            self.sources[row].badge = if count == 0 {
                None
            } else {
                Some(count.to_string())
            };
        }
        self.dirty = true;
    }

    fn back(&mut self) {
        match self.source() {
            Source::Local => {
                if let View::Album(album) = self.view {
                    self.view = View::Albums;
                    self.album_cursor = album;
                    self.dirty = true;
                }
            }
            Source::Remote => {
                if let View::Album(album) = self.remote_view {
                    self.remote_view = View::Albums;
                    self.remote_album_cursor = album;
                    // Whatever the open album's fetch was doing is no longer on
                    // screen, so its outcome is no longer news.
                    self.remote_detail = DetailState::Idle;
                    self.dirty = true;
                }
            }
            Source::Search => {
                if let View::Album(album) = self.search.view {
                    self.search.view = View::Albums;
                    self.search.cursor = album;
                    self.search.detail = DetailState::Idle;
                    self.dirty = true;
                }
            }
            // A flat list has nowhere to go back to.
            Source::Queue => {}
        }
    }

    // ── the transport ────────────────────────────────────────────────────

    /// Start `track` of `album` **in `source`'s library**.
    ///
    /// Synchronous by construction — see the module docs. `Engine::play` and
    /// `Engine::play_url` both return as soon as the session exists; the decode,
    /// the download and the output device come up on the engine's own thread.
    ///
    /// # Why the source is a parameter and not a lookup
    ///
    /// This used to be `play(album, track)` and it always indexed
    /// [`App::library`]. That is only safe while nothing else can be on screen,
    /// and Task 2 put a second, independently-indexed library there. The two
    /// guards that stood in for this — `Enter` and `space` on the remote source
    /// putting up a "not wired yet" note — were the only thing between a remote
    /// album list and a completely unrelated local track playing.
    ///
    /// Taking the source as an argument means a call site cannot forget to say
    /// which list it means; there is no default and nothing to fall through to.
    /// The pair travels on into [`NowPlaying`], so `n` / `p` and the `▶` marker
    /// resolve against the same library the track came from.
    ///
    /// # Playing a track queues its album behind it
    ///
    /// Since Task 4 this does not start one track — it makes `album` **the
    /// queue**, positioned at `track`, and starts that. Playing from a list means
    /// playing that list; it is also the only way the queue ever gets filled
    /// without asking, and a player whose queue you had to build by hand before
    /// anything could follow a track would not be one.
    ///
    /// A whole-queue replacement is deliberate rather than incidental: `a` adds
    /// without disturbing what is playing, and `Enter` in the Queue pane moves
    /// within the queue without rebuilding it. `Enter` in a *library* is the one
    /// gesture that means "play this list now".
    ///
    /// Nothing happens at all when the indices resolve to no track, or to one
    /// that cannot be started — the queue is left exactly as it was, so a key
    /// that cannot do anything also cannot throw away what you had.
    pub fn play(&mut self, source: Source, album: usize, track: usize) {
        let entries = match source {
            Source::Local => self.local_album_entries(album),
            Source::Remote => self.remote_album_entries(album),
            Source::Search => self.search_album_entries(album),
            // The queue does not contain albums, so there is no album to make a
            // queue out of. [`App::play_queue_at`] is the Queue pane's path in.
            Source::Queue => return,
        };
        match entries.get(track) {
            None => return,
            Some(entry) if !self.can_start(entry) => {
                self.note_no_client();
                return;
            }
            Some(_) => {}
        }
        if self.queue.replace(entries, track) {
            self.set_queue_badge();
            self.play_current();
        }
    }

    /// Play entry `index` of the queue, leaving the list alone.
    ///
    /// The **only** place [`crate::queue::Queue::set_position`] is called, and it
    /// is called only for an entry that can actually be started. That is what
    /// keeps `current` — and therefore the `▶` in the Queue pane — a claim about
    /// audio rather than about intent.
    fn play_queue_at(&mut self, index: usize) {
        match self.queue.get(index) {
            None => return,
            Some(entry) if !self.can_start(entry) => {
                self.note_no_client();
                return;
            }
            Some(_) => {}
        }
        if self.queue.set_position(index) {
            self.play_current();
        }
    }

    /// Whether this entry could be started **right now**.
    ///
    /// A local file always can be handed to the engine — it may still fail to
    /// open, but that is a failure the engine reports and
    /// [`App::note_if_it_never_started`] names. A remote one needs a live client
    /// to sign its URL with, and without one there is nothing to hand over at
    /// all.
    ///
    /// Asked **before** the queue moves. The queue outlives a dropped connection
    /// for exactly as long as it takes the fold to hear about it, and a `▶` that
    /// had moved onto an entry no session was ever created for would be the list
    /// claiming something the transport is not doing.
    fn can_start(&self, entry: &Entry) -> bool {
        match entry.media {
            Media::Local(_) => true,
            Media::Remote(_) => self.connection.is_some(),
        }
    }

    /// The one note a missing client produces. Saying it beats a key that
    /// silently does nothing.
    fn note_no_client(&mut self) {
        self.set_status("not connected · press r to reconnect");
    }

    /// One album of the scanned folder as queue entries, in list order.
    ///
    /// Empty when the album is not there — which [`crate::queue::Queue::replace`]
    /// then refuses, so an unresolvable index cannot empty the queue.
    fn local_album_entries(&self, album: usize) -> Vec<Entry> {
        let Some(item) = self.library.albums.get(album) else {
            return Vec::new();
        };
        item.tracks
            .iter()
            .enumerate()
            .map(|(track, t)| Entry {
                media: Media::Local(t.path.clone()),
                album,
                track,
                title: t.title.clone(),
                artist: t.artist.clone(),
                album_name: item.name.clone(),
                dur_ms: (t.duration * 1000.0).max(0.0) as u64,
            })
            .collect()
    }

    /// One album of the server as queue entries, in list order.
    ///
    /// Empty both when the album is not there and when its tracks have not landed
    /// — "not fetched yet" and "no tracks" are different states, but neither of
    /// them is something to play, and the pane already says which it is.
    fn remote_album_entries(&self, album: usize) -> Vec<Entry> {
        let Some(item) = self.remote.album(album) else {
            return Vec::new();
        };
        let Some(tracks) = item.tracks.as_ref() else {
            return Vec::new();
        };
        tracks
            .iter()
            .enumerate()
            .map(|(track, t)| Entry {
                // The **id**, never a URL. See [`crate::queue`].
                media: Media::Remote(t.id.clone()),
                album,
                track,
                title: t.title.clone(),
                artist: t.artist.clone(),
                album_name: item.name.clone(),
                dur_ms: (t.duration * 1000.0).max(0.0) as u64,
            })
            .collect()
    }

    /// Hand the engine whatever the queue is pointing at.
    ///
    /// # Sequential, not gapless — and stated rather than left to be noticed
    ///
    /// `eko-core` can join two tracks without a seam via `Engine::enqueue`, and
    /// the desktop app uses it. **`eko-cli` cannot call it at all.** The method's
    /// signature changes arity under the `pro` feature — `(path, url)` free,
    /// `(path, url, track_id, plain_len)` Pro — and the project gate builds this
    /// whole workspace both ways (`cargo test` and `cargo test --features pro`),
    /// with Cargo unifying `eko-core`'s features across it. So a call written for
    /// one build fails to compile in the other, and the only fix is a
    /// `#[cfg(feature = "pro")]` seam. `eko-cli` is FREE: it has no `pro` feature
    /// to gate on and will not grow one.
    ///
    /// So each track starts a fresh engine session, and there is an audible gap at
    /// every boundary — the length of opening a file, or of the first HTTP round
    /// trip for a stream. That is a real cost and it is not disguised anywhere.
    /// The fix is a free-signature `enqueue` in `eko-core`, which is outside this
    /// task's blast radius.
    ///
    /// One thing worth recording while it is fresh: gapless would **not** have
    /// endangered the seal. `eko-core` only continues into the next source when
    /// its native rate matches the current one, and it rewrites `src_rate`, `bits`
    /// and `codec` at the join; a genuine rate change ends the session instead. So
    /// a seam cannot carry a stale verdict — the constraint that makes gapless
    /// work is the same one that keeps the seal honest across it.
    ///
    /// Every caller has already asked [`App::can_start`], so the one way out
    /// without a session is a queue with no current entry at all.
    fn play_current(&mut self) {
        let Some(entry) = self.queue.current().cloned() else {
            return;
        };
        let now = NowPlaying {
            source: entry.source(),
            album: entry.album,
            track: entry.track,
            title: entry.title.clone(),
            artist: entry.artist.clone(),
            album_name: entry.album_name.clone(),
            dur_ms: entry.dur_ms,
        };
        match &entry.media {
            Media::Local(path) => self.engine.play(path.clone()),
            // The URL is minted here and held nowhere.
            //
            // A pre-signed Subsonic URL contains `u`, `t` and `s` — username, the
            // md5 of password + salt, and the salt — and that triple is a
            // **replayable credential** for as long as the password stands. A
            // queue of a few hundred tracks that each carried one would keep a few
            // hundred credentials in memory for the life of the session, any one
            // of which could reach a log, a panic message or a `{:?}`. So the id
            // is what the queue keeps, and the URL exists for the length of this
            // match arm.
            //
            // It is `stream_src_url`, **not** `stream_url`: the latter returns
            // `stream://localhost/?src=…`, which is Tauri's protocol handler and
            // resolves to nothing at all in a terminal process.
            Media::Remote(id) => {
                let Some(url) = self.stream_url(id) else {
                    // Unreachable: [`App::can_start`] has already refused a remote
                    // entry with no client, before the queue moved onto it.
                    self.note_no_client();
                    return;
                };
                self.engine.play_url(url)
            }
        };
        self.started(now);
    }

    /// The direct, authenticated stream URL for a track id, or `None` when there
    /// is no live client to sign it with.
    ///
    /// Never stored, never logged, never rendered — see [`App::play_current`].
    fn stream_url(&self, track_id: &str) -> Option<String> {
        let connection = self.connection.as_ref()?;
        Some(eko_net::urls::stream_src_url(
            connection.client.config(),
            track_id,
            &eko_net::auth::random_salt(),
        ))
    }

    /// The state every start shares, once the engine has been handed a source.
    ///
    /// One function so the local and the remote path cannot drift on the things
    /// that matter — in particular on dropping [`App::stream`], which is what
    /// keeps the previous track's seal from being shown for this one.
    fn started(&mut self, now: NowPlaying) {
        // **Kept, and deliberately not replaced by a literal `1.0`.**
        //
        // With software volume gone this can only ever set unity, so it looks
        // like a line to delete. It is the same argument as the `apply_eq` below,
        // in the same shape: `Engine`'s `new_shared` builds every session from
        // its own defaults, and this crate does not own those defaults. The day
        // one of them stops being unity, an engine would come up attenuated under
        // a seal reading `BIT-PERFECT` — the exact class of false claim this
        // product exists to prevent, and one no test in this crate would catch,
        // because `EngineStatus` carries no volume to read back.
        //
        // It reads [`App::volume`] rather than `1.0` for the reason the field
        // gives: a literal here would be correct until the commit that gave the
        // field another value, and then silently wrong.
        self.engine.set_volume(f64::from(self.volume));
        // And so does the EQ, for a stronger reason than surprise. `Engine`'s
        // `new_shared` builds every session with a fresh `EqParams::default()`,
        // and `set_eq` is a no-op when there is no session to write into — so an
        // EQ switched on while the Deck was stopped would silently not be there
        // on the next track, and one switched on mid-album would be dropped at
        // the next auto-advance. Without this the seal and the engine part
        // company at every track boundary. (The direction they part in is the
        // safe one — the seal would keep saying `EQ` over an engine that had
        // gone flat — but a setting that quietly stops applying is its own bug.)
        self.apply_eq();

        self.now = Some(now);
        self.playback = Playback::Playing;
        // One of the two places the transport becomes `Playing`, so one of the two
        // places the countdown's clock has to be restarted — see
        // [`App::touch_sleep`].
        self.touch_sleep();
        self.pos_ms = 0;
        // …and so does the buffer. `buffered_ms` is the previous session's
        // decode progress until the next poll overwrites it, and [`App::seek_by`]
        // clamps a stream against it — so without this a `]` pressed in the
        // window between one track starting and the first tick would be clamped
        // against the *previous* track's download and park the clock at a
        // position this session has not decoded. The window is 33ms with the
        // spectrum up and a whole second without it, and auto-advance lands
        // squarely inside it.
        self.buffered_ms = 0;
        self.bands.clear();
        // The caps go with them: a held peak over a stopped transport is a
        // picture of something that is not happening.
        self.peaks.clear();
        // The previous track's rates say nothing about this one. Dropping them
        // means the seal reads `UNVERIFIED` for the frames between the start and
        // the engine's first full report, rather than re-showing a stale claim.
        // It matters more on the streaming path than the local one: a URL source
        // has a download to start before it can describe anything, so that window
        // is measured in hundreds of milliseconds rather than tens.
        self.stream = None;
        self.status = None;
        // The Queue pane's heading counts "3 of 12", so moving through the queue
        // is a context change in exactly the way opening an album is.
        self.context = self.describe_context();
        self.dirty = true;
        // A second call site for [`App::sync_art`], and a deliberate one. The
        // one in [`App::handle`] covers the whole application as it stands —
        // every path to here runs inside an event — but [`App::play`] is `pub`,
        // and a caller that reached it directly would otherwise keep the
        // previous track's cover under the new track's title. `sync_art` returns
        // immediately when the key has not moved, so the duplicate costs a
        // comparison. The envelope follows for exactly the same reason: a
        // direct `play` would otherwise leave the previous track's waveform
        // under the new one's scrubber, which is the same lie in a longer shape.
        self.sync_art();
        self.sync_wave();
    }

    fn play_pause(&mut self) {
        match self.playback {
            Playback::Playing => {
                self.engine.pause();
                self.playback = Playback::Paused;
                self.dirty = true;
            }
            Playback::Paused => {
                self.engine.resume();
                self.playback = Playback::Playing;
                // The other one. See [`App::touch_sleep`].
                self.touch_sleep();
                self.dirty = true;
            }
            // Nothing is loaded: Space starts whatever the cursor is on, so the
            // most obvious key does the most obvious thing from a cold start.
            //
            // Both sources take the same three arms, because `play` now takes the
            // source it should resolve the indices against. The remote arm used to
            // be a *guard* rather than a branch — without it `space` on the
            // server's list started a local album that merely happened to sit at
            // the same index.
            Playback::Stopped => match (self.now.clone(), self.source(), self.view()) {
                // Something was loaded and stopped — resume *it*, not whatever
                // the cursor has since wandered onto.
                (Some(_), _, _) => {
                    self.play_current();
                }
                (None, Source::Queue, _) => self.play_queue_at(self.cursor()),
                // A result row is not an album index — it is an album *or* a
                // song — so `space` reads it the same way `Enter` does rather
                // than resolving it as one.
                (None, Source::Search, View::Albums) => {
                    self.activate_search_row(self.search.cursor);
                }
                (None, source, View::Album(album)) => self.play(source, album, self.cursor()),
                (None, source, View::Albums) => self.play(source, self.cursor(), 0),
            },
        }
    }

    /// `n` / `p` — move through the **queue**, wrapping at its ends.
    ///
    /// It used to walk the playing album by index, which meant it could only ever
    /// move within one album and had to re-derive which library that album was in
    /// on every press. The queue already knows both: an entry carries its own
    /// media, so `n` off the end of one album and into the next — or from a local
    /// track straight into a remote one — is the same step as any other.
    ///
    /// Wrapping is [`crate::queue::Queue::peek`]'s, not this function's: a key
    /// someone pressed gets an answer. Auto-advance does not wrap.
    fn step_track(&mut self, delta: isize) {
        if self.now.is_none() {
            return;
        }
        if let Some(next) = self.queue.peek(delta) {
            self.play_queue_at(next);
        }
    }

    /// `[` / `]` — move the playhead, and **never further than the engine can
    /// actually go**.
    ///
    /// # Why a stream is clamped and a file is not
    ///
    /// EKO's seek is over the *decoded* buffer, not the media source, so a stream
    /// really is seekable — backwards always, and forwards as far as the download
    /// has reached. Past that point `eko-core` still accepts the seek and parks
    /// the playhead there, outputting silence until the decoder catches up. The
    /// desktop app can show that honestly because it draws `buffered_ms` as a
    /// second region on its scrubber. This client does not, so the same behaviour
    /// reads as a Deck that has frozen: a clock that stops, a spectrum that flat
    /// lines, and nothing on screen saying why.
    ///
    /// So a remote track is clamped to [`App::buffered_ms`] — the last moment
    /// there is audio for — and the note says what stopped it. Nothing jumps,
    /// nothing snaps back, and the app does not claim a position it cannot play.
    /// A local file is not clamped: it is on disk, the decoder is not racing a
    /// network, and the engine clamps it to the track's own end.
    fn seek_by(&mut self, delta: f64) {
        if self.now.is_none() || self.playback == Playback::Stopped {
            return;
        }
        let wanted = (self.pos_ms as f64 / 1000.0 + delta).max(0.0);
        let target = match self.seek_limit() {
            Some(limit) => wanted.min(limit),
            None => wanted,
        };
        self.engine.seek(target);
        // Move the clock now rather than waiting a tick for the engine to
        // agree — a scrubber that lags the keypress feels broken. Safe to do
        // optimistically *because* of the clamp above: this is a position the
        // engine will honour, not a guess it may refuse.
        self.pos_ms = (target * 1000.0) as u64;
        if target < wanted {
            self.set_status(format!(
                "{} is all that has downloaded so far",
                library::format_duration(target)
            ));
        }
        self.dirty = true;
    }

    /// The furthest second the playhead can be put right now, or `None` when
    /// there is no limit worth imposing. See [`App::seek_by`].
    fn seek_limit(&self) -> Option<f64> {
        match self.now.as_ref()?.source {
            // `Queue` and `Search` are unreachable here for the reason above;
            // neither is a *media* kind, and an unclamped seek is the harmless
            // reading of a state that cannot occur.
            Source::Local | Source::Queue | Source::Search => None,
            Source::Remote => Some(self.buffered_ms as f64 / 1000.0),
        }
    }

    /// **Test-only.** Drop the engine to silence.
    ///
    /// A handful of `#[ignore]`d tests claim the machine's real output device,
    /// and they must not play out loud. This is the only thing in the crate that
    /// writes [`App::volume`] after construction, it does not exist outside
    /// `cfg(test)`, and it is not a volume control: there is no key, no config
    /// key and no public method behind it.
    ///
    /// It writes the field as well as the live session because [`App::started`]
    /// re-asserts the field on every new session — so one call silences the one
    /// that is playing *and* the one auto-advance is about to start. That is the
    /// whole argument for keeping the field, demonstrated.
    #[cfg(test)]
    pub(crate) fn hush(&mut self) {
        self.volume = 0.0;
        self.engine.set_volume(0.0);
    }

    /// **Test-only.** Put [`App::volume`] back to unity, leaving the live
    /// session where [`App::hush`] left it.
    ///
    /// The asymmetry is the point: those tests want a **silent** engine under a
    /// seal derived at **unity**, so that what the seal reports is the EQ or the
    /// rates rather than the fact that the speakers are off.
    #[cfg(test)]
    pub(crate) fn unhush(&mut self) {
        self.volume = 1.0;
    }

    /// Duration of the current track in ms, `0` when nothing is loaded.
    #[must_use]
    pub fn dur_ms(&self) -> u64 {
        self.now.as_ref().map_or(0, |n| n.dur_ms)
    }

    /// Test-only handle on the music folder, so a fixture can point the fold at
    /// a temporary directory — exactly as a config file with a `music_folder`
    /// in it would.
    #[cfg(test)]
    pub(crate) fn set_music_folder(&mut self, root: std::path::PathBuf) {
        self.folder = MusicFolder::Configured(root);
    }

    /// Test-only handle on the file [`App::persist_device`] writes to, so the
    /// round trip can be asserted against a temporary path instead of the
    /// developer's own config. See [`App::config_path`].
    #[cfg(test)]
    pub(crate) fn set_config_path(&mut self, path: std::path::PathBuf) {
        self.config_path = Some(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Album, Track};
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::path::PathBuf;

    /// A Deck with nowhere to look. Stated, not inherited: resolving the real
    /// platform folder here would make every assertion below depend on whether
    /// the machine running the suite happens to have a `~/Music`.
    fn app() -> App {
        App::new(
            &Config::default(),
            Theme::default(),
            MusicFolder::Unset { probed: None },
        )
    }

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    /// A Deck whose sidebar is **entirely synthetic**: one leaf and one
    /// disclosing source with two views under it.
    ///
    /// No shipped source discloses — `Local` and `Queue` are leaves and the
    /// server is one too — but [`App::visible_sources`] and `toggle_expand` are
    /// general and Phase 1b-ii's Subsonic source will have views beneath it, so
    /// the machinery is exercised against a fixture rather than left to rot.
    ///
    /// The real rows are **replaced** rather than appended to, so the assertions
    /// below are about disclosure and nothing else: appending would leave the
    /// `Queue` row sitting after the children, and every index in every one of
    /// these tests would then be about where `Queue` happens to be.
    ///
    /// Rows: `0` Local (leaf), `1` Remote (open), `2` and `3` its views.
    fn with_a_disclosing_source() -> App {
        let mut a = app();
        a.sources = vec![
            SourceRow::source("Local", Source::Local),
            SourceRow::root("Remote", true),
            SourceRow::child("Albums"),
            SourceRow::child("Artists"),
        ];
        a
    }

    fn track(title: &str, n: u32) -> Track {
        Track {
            // A path that cannot be opened: `Engine::play` fails in
            // `open_source`, before it ever reaches the output device, so these
            // tests never touch CoreAudio.
            path: format!("/eko-cli-test/{title}.flac"),
            title: title.to_string(),
            artist: "Test Artist".to_string(),
            track_no: Some(n),
            duration: 120.0,
        }
    }

    /// Two albums, three tracks and two tracks.
    fn stocked() -> App {
        let mut a = app();
        a.library = Library {
            root: Some(PathBuf::from("/tmp/Music")),
            albums: vec![
                Album {
                    id: "Test Artist First".into(),
                    name: "First".into(),
                    artist: "Test Artist".into(),
                    tracks: vec![track("One", 1), track("Two", 2), track("Three", 3)],
                },
                Album {
                    id: "Test Artist Second".into(),
                    name: "Second".into(),
                    artist: "Test Artist".into(),
                    tracks: vec![track("Alpha", 1), track("Beta", 2)],
                },
            ],
        };
        a.scan = ScanState::Ready;
        a
    }

    // ── the adaptive tick ────────────────────────────────────────────────

    #[test]
    fn playing_with_the_spectrum_up_ticks_at_thirty_fps() {
        assert_eq!(tick_interval(Playback::Playing, true), Some(FRAME_INTERVAL));
        assert!(FRAME_INTERVAL.as_millis() >= 30 && FRAME_INTERVAL.as_millis() <= 40);
    }

    #[test]
    fn playing_without_the_spectrum_ticks_only_for_the_clock() {
        assert_eq!(
            tick_interval(Playback::Playing, false),
            Some(CLOCK_INTERVAL)
        );
    }

    #[test]
    fn a_paused_or_stopped_deck_blocks_on_input_instead_of_spinning() {
        assert_eq!(tick_interval(Playback::Paused, true), None);
        assert_eq!(tick_interval(Playback::Paused, false), None);
        assert_eq!(tick_interval(Playback::Stopped, true), None);
        assert_eq!(tick_interval(Playback::Stopped, false), None);
    }

    #[test]
    fn hiding_the_spectrum_while_playing_drops_the_tick_rate() {
        let mut a = app();
        a.playback = Playback::Playing;
        assert_eq!(a.tick_interval(), Some(FRAME_INTERVAL));
        a.handle(key(KeyCode::Char('s')));
        assert!(!a.spectrum_visible);
        assert_eq!(a.tick_interval(), Some(CLOCK_INTERVAL));
    }

    // ── the fold ─────────────────────────────────────────────────────────

    #[test]
    fn q_quits() {
        let mut a = app();
        assert!(!a.should_quit());
        a.handle(key(KeyCode::Char('q')));
        assert!(a.should_quit());
    }

    #[test]
    fn ctrl_c_quits_because_raw_mode_swallows_sigint() {
        let mut a = app();
        a.handle(AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))));
        assert!(a.should_quit());
    }

    #[test]
    fn a_bare_c_does_not_quit() {
        let mut a = app();
        a.handle(key(KeyCode::Char('c')));
        assert!(!a.should_quit());
    }

    #[test]
    fn esc_goes_back_rather_than_quitting() {
        // It used to quit. It cannot any more: Esc is the way out of an album.
        let mut a = stocked();
        a.handle(key(KeyCode::Enter));
        assert_eq!(a.view, View::Album(0));
        a.handle(key(KeyCode::Esc));
        assert_eq!(a.view, View::Albums);
        assert!(!a.should_quit());
        // And at the top level it is simply inert.
        a.handle(key(KeyCode::Esc));
        assert!(!a.should_quit());
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut a = app();
        let mut ev = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        ev.kind = KeyEventKind::Release;
        a.handle(AppEvent::Input(Event::Key(ev)));
        assert!(!a.should_quit());
    }

    // ── sidebar navigation (focus must be moved to it first) ─────────────

    #[test]
    fn tab_moves_focus_between_the_panes() {
        let mut a = app();
        assert_eq!(a.focus, Focus::Main);
        a.handle(key(KeyCode::Tab));
        assert_eq!(a.focus, Focus::Sidebar);
        a.handle(key(KeyCode::Tab));
        assert_eq!(a.focus, Focus::Main);
    }

    /// **Every source in the list does something.**
    ///
    /// The sidebar shipped `Local`, `Navidrome`, `Albums`, `Artists`, `Lists`
    /// and `Queue`; five of the six were decoration, and the owner asked how to
    /// navigate to one of them. A row is a claim, so the list holds only the
    /// sources that are wired.
    ///
    /// `Queue` is a row now because Task 4 put a queue behind it. `Artists` and
    /// `Lists` are not rows and are not prose either — the `NOT YET` heading
    /// that named them is gone. This assertion is the whole of what is left of
    /// that rule, and it is the part that matters: the list is exactly the
    /// sources that work, spelled out, so a row cannot be added without saying
    /// so here.
    #[test]
    fn the_sidebar_holds_only_sources_that_are_wired() {
        let a = app();
        assert_eq!(
            a.sources
                .iter()
                .map(|r| r.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Local", "Add server", "Queue"],
        );
        // And every one of them is reachable by the cursor.
        assert_eq!(a.visible_sources(), vec![0, 1, 2]);
        // `+ Add server` keeps the rule rather than breaking it: it is not a
        // library and does not pretend to be one — it is an invitation, and
        // pressing `enter` on it really does start configuring a server. See
        // [`SourceRow::action`].
        assert!(a.sources[1].action);
        assert!(!a.sources[0].action && !a.sources[2].action);
    }

    /// A caret that opens onto nothing is a control that does nothing, which is
    /// the smaller version of the same lie. Every shipped source is a leaf.
    #[test]
    fn no_shipped_source_pretends_to_disclose() {
        let mut a = app();
        assert!(!a.sources[LOCAL_ROW].expandable);
        a.focus = Focus::Sidebar;
        for row in 0..a.sources.len() {
            a.selected = row;
            a.handle(key(KeyCode::Enter));
            assert!(!a.sources[row].expandable, "{:?}", a.sources[row].label);
            assert!(!a.sources[row].expanded, "{:?}", a.sources[row].label);
            // `enter` on `+ Add server` opens the panel; closing it again keeps
            // this loop about disclosure rather than about the panel.
            a.add = None;
        }
        assert_eq!(a.visible_sources(), vec![0, 1, 2]);
    }

    #[test]
    fn selection_moves_and_wraps_both_ways() {
        let mut a = with_a_disclosing_source();
        a.focus = Focus::Sidebar;
        let last = *a.visible_sources().last().unwrap();
        a.handle(key(KeyCode::Up));
        assert_eq!(a.selected, last);
        a.handle(key(KeyCode::Down));
        assert_eq!(a.selected, 0);
        a.handle(key(KeyCode::Char('j')));
        assert_eq!(a.selected, 1);
        a.handle(key(KeyCode::Char('k')));
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn collapsed_children_are_hidden_and_unreachable() {
        let mut a = with_a_disclosing_source();
        a.focus = Focus::Sidebar;
        // The disclosing root starts open, so its two views are visible.
        assert_eq!(a.visible_sources(), vec![0, 1, 2, 3]);

        a.selected = 1;
        a.handle(key(KeyCode::Enter)); // collapse it
        assert_eq!(a.visible_sources(), vec![0, 1]);

        // Down from the last visible row wraps to the first, skipping the
        // children entirely.
        a.handle(key(KeyCode::Down));
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn expanding_a_root_reveals_its_children() {
        let mut a = with_a_disclosing_source();
        a.focus = Focus::Sidebar;
        a.selected = 1;
        a.handle(key(KeyCode::Enter));
        a.handle(key(KeyCode::Enter));
        assert_eq!(a.visible_sources(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn enter_toggles_disclosure_only_on_expandable_rows() {
        let mut a = with_a_disclosing_source();
        a.focus = Focus::Sidebar;
        a.selected = 1;
        assert!(a.sources[1].expanded);
        a.handle(key(KeyCode::Enter));
        assert!(!a.sources[1].expanded);

        // A leaf source and a child are both inert.
        for row in [0, 2] {
            a.selected = row;
            let before = a.sources[row].expanded;
            a.handle(key(KeyCode::Enter));
            assert_eq!(a.sources[row].expanded, before);
        }
    }

    // ── library navigation ───────────────────────────────────────────────

    #[test]
    fn the_main_pane_cursor_moves_over_albums_and_wraps() {
        let mut a = stocked();
        assert_eq!(a.album_cursor, 0);
        a.handle(key(KeyCode::Char('j')));
        assert_eq!(a.album_cursor, 1);
        a.handle(key(KeyCode::Down));
        assert_eq!(a.album_cursor, 0, "wraps at the end");
        a.handle(key(KeyCode::Char('k')));
        assert_eq!(a.album_cursor, 1, "wraps backwards too");
    }

    #[test]
    fn enter_opens_an_album_and_esc_restores_the_cursor_to_it() {
        let mut a = stocked();
        a.album_cursor = 1;
        a.handle(key(KeyCode::Enter));
        assert_eq!(a.view, View::Album(1));
        assert_eq!(a.track_cursor, 0);
        assert_eq!(a.row_count(), 2);

        a.handle(key(KeyCode::Esc));
        assert_eq!(a.view, View::Albums);
        assert_eq!(a.album_cursor, 1);
    }

    #[test]
    fn navigating_an_empty_library_is_inert_rather_than_a_panic() {
        let mut a = app();
        for code in [KeyCode::Char('j'), KeyCode::Char('k'), KeyCode::Enter] {
            a.handle(key(code));
        }
        assert_eq!(a.view, View::Albums);
        assert_eq!(a.album_cursor, 0);
        assert_eq!(a.row_count(), 0);
    }

    // ── the scan ─────────────────────────────────────────────────────────

    #[test]
    fn scan_progress_lands_in_the_fold() {
        let mut a = app();
        a.handle(AppEvent::Scan(ScanEvent::Discovering { found: 12 }));
        assert_eq!(a.scan, ScanState::Discovering { found: 12 });
        assert!(a.scan.is_running());
        a.handle(AppEvent::Scan(ScanEvent::Reading { done: 3, total: 12 }));
        assert_eq!(a.scan, ScanState::Reading { done: 3, total: 12 });
        assert!(a.scan.is_running());
    }

    #[test]
    fn a_finished_scan_populates_the_library_and_the_context_line() {
        let mut a = app();
        let library = stocked().library;
        a.handle(AppEvent::Scan(ScanEvent::Finished(Box::new(library))));
        assert_eq!(a.scan, ScanState::Ready);
        assert!(!a.scan.is_running());
        assert_eq!(a.library.albums.len(), 2);
        assert_eq!(a.context, "Music · 2 albums · 5 tracks");
        assert_eq!(a.sources[LOCAL_ROW].badge.as_deref(), Some("2"));
    }

    #[test]
    fn an_empty_folder_is_a_normal_finished_scan_not_a_failure() {
        let mut a = app();
        a.handle(AppEvent::Scan(ScanEvent::Finished(Box::new(Library {
            root: Some(PathBuf::from("/tmp/Silence")),
            albums: Vec::new(),
        }))));
        assert_eq!(a.scan, ScanState::Ready);
        assert!(a.status.is_none(), "an empty folder is not an error");
        assert_eq!(a.context, "Silence · empty");
        assert_eq!(a.sources[LOCAL_ROW].badge, None);
    }

    #[test]
    fn a_failed_scan_surfaces_in_the_footer_without_stopping_the_app() {
        let mut a = app();
        a.handle(AppEvent::Scan(ScanEvent::Failed(
            "/nope is not a folder".into(),
        )));
        assert!(matches!(a.scan, ScanState::Failed(_)));
        assert_eq!(a.status.as_deref(), Some("/nope is not a folder"));
        assert!(!a.should_quit());
    }

    #[test]
    fn a_scan_needs_a_configured_folder_and_a_channel() {
        // No music_folder: start_scan is a no-op, not a panic and not an error.
        let mut a = app();
        a.start_scan();
        assert_eq!(a.scan, ScanState::Unconfigured);

        // A folder but no channel yet: still a no-op.
        a.set_music_folder(PathBuf::from("/tmp"));
        a.start_scan();
        assert_eq!(a.scan, ScanState::Unconfigured);
    }

    /// **The navigation bug, at its root.** The platform default is a real
    /// folder to scan, not a decoration: with no config file at all, `attach`
    /// still starts a scan.
    ///
    /// Before this, `start_scan` read `config.music_folder`, found `None`, and
    /// returned — so a first run listed nothing, `row_count()` was 0, and every
    /// `j` and `k` was correctly ignored by a cursor with no rows to move over.
    #[test]
    fn the_platform_default_is_scanned_just_like_a_configured_folder() {
        let dir = std::env::temp_dir().join("eko-cli-platform-default");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        // `Platform` is by construction the "nothing was configured" case —
        // `resolve` only reaches it when `music_folder` is absent.
        let mut a = App::new(
            &Config::default(),
            Theme::default(),
            MusicFolder::Platform(dir.clone()),
        );
        a.attach(tx);
        assert!(
            a.scan.is_running(),
            "the platform default was never scanned"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_folder_is_its_own_state_and_carries_the_path() {
        let gone = PathBuf::from("/Volumes/Archive/Music");
        let mut a = app();
        a.handle(AppEvent::Scan(ScanEvent::Missing(gone.clone())));
        assert_eq!(a.scan, ScanState::Missing(gone));
        assert!(
            a.status.is_none(),
            "the main pane says it in full; the footer need not repeat it"
        );
        assert!(!a.should_quit());
    }

    /// End to end through the worker: a configured folder that is not there
    /// settles on `Missing`, and is never quietly swapped for another one.
    #[test]
    fn scanning_a_configured_folder_that_is_not_there_ends_in_missing() {
        let gone = std::env::temp_dir().join("eko-cli-definitely-not-here");
        std::fs::remove_dir_all(&gone).ok();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut a = app();
        a.set_music_folder(gone.clone());
        a.attach(tx);
        let mut settled = false;
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(10)) {
            a.handle(event);
            if !a.scan.is_running() {
                settled = true;
                break;
            }
        }
        assert!(settled, "the scan never reported a result");
        assert_eq!(a.scan, ScanState::Missing(gone));
    }

    #[test]
    fn r_during_a_scan_does_not_stack_a_second_worker_on_the_same_folder() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut a = app();
        a.set_music_folder(PathBuf::from("/tmp"));
        a.events = Some(tx);
        a.scan = ScanState::Reading {
            done: 5,
            total: 900,
        };
        a.handle(key(KeyCode::Char('r')));
        assert_eq!(
            a.scan,
            ScanState::Reading {
                done: 5,
                total: 900
            }
        );
    }

    #[test]
    fn attaching_a_channel_starts_the_first_scan() {
        let dir = std::env::temp_dir().join("eko-cli-attach");
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut a = app();
        a.set_music_folder(dir.clone());
        a.attach(tx);
        assert!(a.scan.is_running());
        // The worker finishes on its own thread and the result arrives here.
        let mut finished = false;
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(10)) {
            a.handle(event);
            if a.scan == ScanState::Ready {
                finished = true;
                break;
            }
        }
        assert!(finished, "the scan never reported a result");
        assert!(a.library.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── the connection ───────────────────────────────────────────────────

    fn server() -> ServerConfig {
        ServerConfig {
            name: "home".into(),
            base_url: "https://music.example.com".into(),
            username: "rod".into(),
        }
    }

    /// A Deck with a `[[servers]]` entry, and nothing else different.
    fn with_a_server() -> App {
        let mut config = Config::default();
        config.servers.push(server());
        App::new(
            &config,
            Theme::default(),
            MusicFolder::Unset { probed: None },
        )
    }

    #[test]
    fn no_servers_in_the_config_means_nothing_to_connect_to() {
        let a = app();
        assert!(a.server.is_none());
        assert_eq!(a.conn, ConnState::NotConfigured);
        assert!(a.connection.is_none());
        assert!(
            a.status.is_none(),
            "an unconfigured Deck has nothing to say"
        );
    }

    #[test]
    fn the_first_configured_server_is_the_one_the_fold_holds() {
        let a = with_a_server();
        assert_eq!(a.server.as_ref().map(|s| s.name.as_str()), Some("home"));
        // …and it is still not connected until something starts it.
        assert_eq!(a.conn, ConnState::NotConfigured);
    }

    /// A connect needs a channel, exactly as a scan does.
    #[test]
    fn starting_a_connection_needs_a_server_and_a_channel() {
        let mut a = app();
        a.start_connect();
        assert_eq!(a.conn, ConnState::NotConfigured, "no server to connect to");

        let mut a = with_a_server();
        a.start_connect();
        assert_eq!(a.conn, ConnState::NotConfigured, "no channel yet");
    }

    #[test]
    fn attaching_a_channel_starts_the_connection() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut a = with_a_server();
        a.attach(tx);
        assert_eq!(
            a.conn,
            ConnState::Connecting {
                name: "home".into()
            }
        );
        assert!(a.conn.is_connecting());
    }

    /// `r` during a connection does not stack a second worker on the same
    /// server, for the same reason it does not stack a second scan.
    #[test]
    fn r_during_a_connection_does_not_stack_a_second_connector() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut a = with_a_server();
        a.attach(tx);
        a.handle(AppEvent::Conn(ConnEvent::Failed {
            name: "home".into(),
            message: "boom".into(),
        }));
        a.handle(key(KeyCode::Char('r')));
        assert!(
            a.conn.is_connecting(),
            "r did not retry a failed connection"
        );
        let before = a.conn.clone();
        a.handle(key(KeyCode::Char('r')));
        assert_eq!(a.conn, before);
    }

    #[test]
    fn a_missing_password_is_its_own_state_and_names_the_command_that_fixes_it() {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::NeedsPassword {
            name: "home".into(),
        }));
        assert_eq!(
            a.conn,
            ConnState::NeedsPassword {
                name: "home".into()
            }
        );
        assert!(a.connection.is_none());
        let note = a.status.clone().expect("a note");
        assert!(note.contains("no password stored"), "{note}");
        // **The fix is a keystroke, not a second program.** This assertion used
        // to require `eko-cli login home` — the application telling you to quit
        // it — and the sentence it now requires is the one the sidebar can
        // actually deliver on.
        assert!(note.contains("enter"), "{note}");
    }

    // ── adding a server from inside the Deck ─────────────────────────────

    /// The row that used not to exist, and `enter` on it.
    fn open_the_panel() -> App {
        let mut a = app();
        a.focus = Focus::Sidebar;
        a.selected = 1;
        assert_eq!(a.source(), Source::Remote);
        a.handle(key(KeyCode::Enter));
        a
    }

    fn type_into(a: &mut App, text: &str) {
        for c in text.chars() {
            a.handle(key(KeyCode::Char(c)));
        }
    }

    /// Fill the panel in and submit it, the way a person would.
    fn fill_in(a: &mut App, name: &str, url: &str, user: &str) {
        type_into(a, name);
        a.handle(key(KeyCode::Enter));
        type_into(a, url);
        a.handle(key(KeyCode::Enter));
        type_into(a, user);
        a.handle(key(KeyCode::Enter));
    }

    /// **The journey this whole feature exists for**, end to end and without a
    /// restart: a Deck with no config, three fields, and a request for the one
    /// thing the fold is not allowed to hold.
    #[test]
    fn a_deck_with_no_config_can_configure_a_server_without_leaving() {
        let mut a = open_the_panel();
        let panel = a.add.as_ref().expect("the panel did not open");
        assert!(!panel.second, "a first server was announced as a second");
        assert_eq!(panel.field, Field::Name);

        fill_in(&mut a, "home", "https://music.example.com", "rod");
        assert!(
            a.add.is_none(),
            "the panel stayed open after a valid submit"
        );

        let Some(Prompt::Add(server)) = a.take_prompt() else {
            panic!("no password was asked for");
        };
        assert_eq!(server.name, "home");
        assert_eq!(server.base_url, "https://music.example.com");
        assert_eq!(server.username, "rod");
        // Asked for exactly once: a second `take_prompt` must not re-raise a
        // prompt the terminal has already served.
        assert_eq!(a.take_prompt(), None);

        // And the answer coming back makes it *the* server, with the rows that
        // go with one, and starts a connection rather than asking for a restart.
        a.finish_prompt(PromptOutcome::Stored {
            server: server.clone(),
            notes: Vec::new(),
        });
        assert_eq!(a.server.as_ref(), Some(&server));
        assert_eq!(
            a.sources
                .iter()
                .map(|r| r.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Local", "home", "Search", "Queue"]
        );
        assert!(!a.sources[1].action, "the invitation stayed an invitation");
    }

    /// The three fields are checked **in the panel**, against the field that is
    /// wrong, rather than at save time where the only thing left to do is a
    /// footer note. The name rule is the keychain account rule, not a second
    /// copy of it.
    #[test]
    fn a_bad_field_is_refused_in_the_panel_and_the_cursor_lands_on_it() {
        for (name, url, user, at, needle) in [
            (
                "has a space",
                "https://h.example",
                "rod",
                Field::Name,
                "letters",
            ),
            (
                "",
                "https://h.example",
                "rod",
                Field::Name,
                "a name is needed",
            ),
            ("home", "h.example", "rod", Field::BaseUrl, "no scheme"),
            (
                "home",
                "https://h.example",
                "  ",
                Field::Username,
                "username",
            ),
        ] {
            let mut a = open_the_panel();
            fill_in(&mut a, name, url, user);
            let panel = a.add.as_ref().unwrap_or_else(|| {
                panic!("{name:?}/{url:?}/{user:?} was accepted and should not have been")
            });
            assert_eq!(panel.field, at, "the cursor did not land on the bad field");
            let message = panel.error_for(at).expect("no message on the bad field");
            assert!(message.contains(needle), "{message:?}");
            assert_eq!(
                a.take_prompt(),
                None,
                "a bad form still asked for a password"
            );
        }
    }

    /// **A name the config already holds is refused before the keychain is
    /// touched.**
    ///
    /// The failure this is written against destroyed a *working* credential and
    /// kept nothing in exchange. `set_password` is keyed on the name, and the
    /// keychain holds one entry per account, so adding a second server called
    /// `home` overwrote the first `home`'s password with the new one; the
    /// duplicate `[[servers]]` table that followed was then dropped by
    /// `Config::normalized` on the next launch under its own "one name, one
    /// password" rule. The user was left with an original server that could no
    /// longer sign in and a new one that had never been saved.
    ///
    /// The rule is the config's, enforced one step earlier — in the panel, in
    /// front of the person typing, with nothing written anywhere yet.
    #[test]
    fn a_duplicate_server_name_is_refused_before_it_can_overwrite_a_password() {
        let mut a = with_a_server();
        a.focus = Focus::Sidebar;
        a.selected = 1;
        assert_eq!(a.source(), Source::Remote);
        a.handle(key(KeyCode::Enter));
        let panel = a.add.as_ref().expect("the panel did not open");
        assert!(panel.second, "a second server was not announced as one");

        fill_in(&mut a, "home", "https://other.example.com", "rod");
        let panel = a
            .add
            .as_ref()
            .expect("a duplicate name was accepted and the panel closed");
        assert_eq!(panel.field, Field::Name, "the cursor left the bad field");
        let message = panel
            .error_for(Field::Name)
            .expect("no message on the duplicate name");
        assert!(message.contains("already configured"), "{message:?}");
        assert_eq!(
            a.take_prompt(),
            None,
            "a duplicate name still asked for a password — the keychain was one \
             keystroke away"
        );

        // A different name goes through, so the guard is the name and not the
        // panel refusing second servers on principle.
        a.add = Some(AddServer {
            second: true,
            ..AddServer::default()
        });
        fill_in(&mut a, "work", "https://other.example.com", "rod");
        assert!(a.add.is_none(), "a fresh name was refused");
        let Some(Prompt::Add(added)) = a.take_prompt() else {
            panic!("no password was asked for a name that is free");
        };
        assert_eq!(added.name, "work");
    }

    /// The guard survives the round trip: a name stored **this session** is
    /// spoken for, without a restart to re-read the config.
    ///
    /// Without this the same panel could add `work` twice in one run and
    /// overwrite the password it had itself just stored — the original bug,
    /// reachable again through the one path that does not go via `Config`.
    #[test]
    fn a_name_stored_this_session_is_taken_for_the_rest_of_it() {
        let mut a = open_the_panel();
        fill_in(&mut a, "home", "https://music.example.com", "rod");
        let Some(Prompt::Add(stored)) = a.take_prompt() else {
            panic!("no password was asked for");
        };
        a.finish_prompt(PromptOutcome::Stored {
            server: stored,
            notes: Vec::new(),
        });

        a.add = Some(AddServer {
            second: true,
            ..AddServer::default()
        });
        fill_in(&mut a, "home", "https://other.example.com", "rod");
        let panel = a
            .add
            .as_ref()
            .expect("the name stored a moment ago was accepted again");
        assert!(panel
            .error_for(Field::Name)
            .is_some_and(|m| m.contains("already configured")));
        assert_eq!(a.take_prompt(), None);
    }

    /// Typing into a field answers the complaint about it, so the message goes
    /// as soon as it is being fixed rather than sitting there until the next
    /// submit.
    #[test]
    fn typing_into_the_bad_field_takes_the_complaint_down() {
        let mut a = open_the_panel();
        fill_in(&mut a, "bad name", "https://h.example", "rod");
        assert!(a.add.as_ref().unwrap().error.is_some());
        a.handle(key(KeyCode::Backspace));
        assert!(a.add.as_ref().unwrap().error.is_none());
    }

    /// A pasted URL is taken as meant and the correction is **said**, because a
    /// silent fix is indistinguishable from a client that ignored what you typed.
    #[test]
    fn a_pasted_url_is_corrected_out_loud() {
        let mut a = open_the_panel();
        fill_in(&mut a, "home", "https://music.example.com/rest/", "rod");
        let Some(Prompt::Add(server)) = a.take_prompt() else {
            panic!("the corrected URL was refused");
        };
        assert_eq!(server.base_url, "https://music.example.com");
        let note = a.status.clone().expect("nothing was said about the fix");
        assert!(note.contains("/rest"), "{note}");
    }

    /// `esc` closes the panel and asks for nothing.
    #[test]
    fn esc_closes_the_panel_without_asking_for_a_password() {
        let mut a = open_the_panel();
        type_into(&mut a, "home");
        a.handle(key(KeyCode::Esc));
        assert!(a.add.is_none());
        assert_eq!(a.take_prompt(), None);
    }

    /// The panel is a form, so `q` is a letter while it is up — and `Ctrl-C`
    /// still quits, exactly as it does in the search input.
    #[test]
    fn the_panel_takes_letters_as_text_and_still_lets_ctrl_c_out() {
        let mut a = open_the_panel();
        type_into(&mut a, "q");
        assert!(!a.should_quit(), "q quit out of a text field");
        assert_eq!(a.add.as_ref().unwrap().name, "q");
        a.handle(AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))));
        assert!(a.should_quit(), "ctrl-c was swallowed by the panel");
    }

    /// **The state the owner was actually in.** A server in the file with no
    /// password stored: `enter` goes straight to the prompt, because the three
    /// fields are already right and re-typing them would be busywork.
    #[test]
    fn enter_on_a_server_with_no_password_asks_for_one_rather_than_the_form() {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::NeedsPassword {
            name: "home".into(),
        }));
        a.focus = Focus::Sidebar;
        a.selected = 1;
        a.handle(key(KeyCode::Enter));
        assert!(a.add.is_none(), "the form opened over a server that exists");
        assert_eq!(a.take_prompt(), Some(Prompt::SignIn(server())));
    }

    /// A rejected password is most often *the password*, so the same key offers
    /// to retype it rather than sending the user back to the config file.
    #[test]
    fn enter_on_a_failed_server_offers_the_password_again() {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::Failed {
            name: "home".into(),
            message: "Wrong username or password.".into(),
        }));
        a.focus = Focus::Sidebar;
        a.selected = 1;
        a.handle(key(KeyCode::Enter));
        assert_eq!(a.take_prompt(), Some(Prompt::SignIn(server())));
    }

    /// Multi-server is deferred, so a second one is written and **not switched
    /// to** — and the panel says so before it is typed into, rather than the
    /// Deck appearing to ignore it afterwards.
    #[test]
    fn a_second_server_is_saved_and_said_to_be_saved_rather_than_used() {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::NeedsPassword {
            name: "home".into(),
        }));
        // Connected, so `enter` is "add another" rather than "sign in".
        a.conn = ConnState::Connected {
            name: "home".into(),
            username: "rod".into(),
        };
        a.focus = Focus::Sidebar;
        a.selected = 1;
        a.handle(key(KeyCode::Enter));
        assert!(a.add.as_ref().expect("the panel").second);

        fill_in(&mut a, "work", "https://work.example.com", "rod");
        let Some(Prompt::Add(second)) = a.take_prompt() else {
            panic!("the second server was refused");
        };
        a.finish_prompt(PromptOutcome::Stored {
            server: second,
            notes: Vec::new(),
        });
        // Still the first one, and still one server row.
        assert_eq!(a.server.as_ref().unwrap().name, "home");
        assert_eq!(
            a.sources
                .iter()
                .filter(|r| r.source == Source::Remote)
                .count(),
            1
        );
        let note = a.status.clone().expect("nothing was said");
        assert!(note.contains("work") && note.contains("home"), "{note}");
    }

    /// Cancelling at the prompt changes nothing and says so.
    #[test]
    fn a_cancelled_prompt_leaves_the_deck_exactly_as_it_was() {
        let mut a = open_the_panel();
        fill_in(&mut a, "home", "https://music.example.com", "rod");
        let _ = a.take_prompt();
        a.finish_prompt(PromptOutcome::Cancelled);
        assert!(a.server.is_none());
        assert_eq!(a.sources.len(), 3);
        assert!(a
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("cancelled"));
    }

    /// A keychain that refuses is a sentence, not a crash, and it does not
    /// invent a server.
    #[test]
    fn a_failed_prompt_is_a_note_and_not_a_server() {
        let mut a = open_the_panel();
        fill_in(&mut a, "home", "https://music.example.com", "rod");
        let _ = a.take_prompt();
        a.finish_prompt(PromptOutcome::Failed("keychain: denied".into()));
        assert!(a.server.is_none());
        assert_eq!(a.status.as_deref(), Some("keychain: denied"));
    }

    /// **Nothing in the request or the answer can carry a credential**, and it
    /// is the types that say so — `Prompt` and `PromptOutcome` both hold a
    /// `ServerConfig`, which has three fields and no fourth.
    #[test]
    fn the_prompt_handshake_has_nowhere_to_put_a_password() {
        let mut a = open_the_panel();
        fill_in(&mut a, "home", "https://music.example.com", "rod");
        let prompt = a.take_prompt().expect("a prompt");
        let printed = format!("{prompt:?}");
        assert!(!printed.contains("password"), "{printed}");
        let outcome = PromptOutcome::Stored {
            server: prompt.server().clone(),
            notes: vec!["dropped the trailing /".into()],
        };
        assert!(!format!("{outcome:?}").contains("password"));
    }

    /// **A two-word query.** Every keystroke used to go through `server::tidy`
    /// on its own, and `tidy(" ")` is the empty string — so the space was
    /// swallowed and `two words` reached the server as `twowords`. Found while
    /// giving the add-server panel the same line editor; fixed in
    /// [`crate::line::type_into`], which shares the character gate and not the
    /// whitespace cosmetics.
    #[test]
    fn a_search_query_can_have_a_space_in_it() {
        let mut a = with_a_server();
        a.handle(key(KeyCode::Char('/')));
        for c in "two words".chars() {
            a.handle(key(KeyCode::Char(c)));
        }
        assert_eq!(a.search.input.as_deref(), Some("two words"));
    }
    #[test]
    fn a_failed_connection_carries_the_real_message_into_the_footer() {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::Failed {
            name: "home".into(),
            message: "Wrong username or password.".into(),
        }));
        assert!(matches!(a.conn, ConnState::Failed { .. }));
        assert!(a.connection.is_none());
        assert_eq!(
            a.status.as_deref(),
            Some("home · Wrong username or password.")
        );
        assert!(!a.should_quit(), "a bad server must not stop the player");
    }

    /// **The failure surface cannot invent a URL.** `eko-net` strips the signed
    /// URL out of transport errors; this pins that nothing in the fold puts one
    /// back by formatting the server's `base_url` into the note.
    #[test]
    fn no_connection_note_carries_the_base_url_or_a_credential() {
        let mut a = with_a_server();
        for event in [
            ConnEvent::NeedsPassword {
                name: "home".into(),
            },
            ConnEvent::Failed {
                name: "home".into(),
                message: "cannot reach the server · error sending request".into(),
            },
        ] {
            a.handle(AppEvent::Conn(event));
            let note = a.status.clone().unwrap_or_default();
            assert!(!note.contains("music.example.com"), "{note}");
            assert!(!note.contains("http"), "{note}");
        }
    }

    /// The live client exists **exactly** while the state says connected — one
    /// write site, so the two cannot drift.
    #[test]
    fn the_client_is_held_only_while_the_state_says_connected() {
        let mut a = with_a_server();
        let connected = |a: &App| matches!(a.conn, ConnState::Connected { .. });

        a.handle(AppEvent::Conn(ConnEvent::Failed {
            name: "home".into(),
            message: "nope".into(),
        }));
        assert_eq!(connected(&a), a.connection.is_some());

        // A real connection needs a server to ping, so this half is covered in
        // `server.rs` against mockito; here the fold is driven from the event it
        // would have produced.
        a.handle(AppEvent::Conn(ConnEvent::NeedsPassword {
            name: "home".into(),
        }));
        assert_eq!(connected(&a), a.connection.is_some());
        assert!(a.connection.is_none());
    }

    /// A retry drops the previous attempt's note and its client before it
    /// starts, so a failed reconnect cannot leave a stale success behind it.
    #[test]
    fn retrying_clears_the_previous_attempts_note() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut a = with_a_server();
        a.attach(tx);
        a.handle(AppEvent::Conn(ConnEvent::Failed {
            name: "home".into(),
            message: "Wrong username or password.".into(),
        }));
        assert!(a.status.is_some());
        a.start_connect();
        assert!(a.status.is_none(), "the old failure survived the retry");
        assert!(a.connection.is_none());
    }

    /// A connection note is ours to take down; the config-parse note is not.
    #[test]
    fn a_connection_result_does_not_clear_someone_elses_footer_note() {
        let mut a = with_a_server();
        a.set_status("config.toml is not valid TOML");
        a.handle(AppEvent::Conn(ConnEvent::Connected(Box::new(
            crate::server::Connection {
                name: "home".into(),
                username: "rod".into(),
                client: std::sync::Arc::new(
                    eko_net::Client::new(eko_net::Config {
                        base_url: "https://music.example.com".into(),
                        username: "rod".into(),
                        password: "hunter2".into(),
                    })
                    .unwrap(),
                ),
            },
        ))));
        assert_eq!(
            a.status.as_deref(),
            Some("config.toml is not valid TOML"),
            "a successful connection clobbered an unrelated note"
        );
        assert!(matches!(a.conn, ConnState::Connected { .. }));
        assert!(a.connection.is_some());
    }

    /// **The event that travels the channel cannot print the password.**
    /// `AppEvent` derives `Debug`; `Connection`'s is hand-written.
    #[test]
    fn an_app_events_debug_cannot_print_a_server_password() {
        let event = AppEvent::Conn(ConnEvent::Connected(Box::new(crate::server::Connection {
            name: "home".into(),
            username: "rod".into(),
            client: std::sync::Arc::new(
                eko_net::Client::new(eko_net::Config {
                    base_url: "https://music.example.com".into(),
                    username: "rod".into(),
                    password: "hunter2".into(),
                })
                .unwrap(),
            ),
        })));
        let printed = format!("{event:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
    }

    // ── the server as a browsable source ─────────────────────────────────

    /// A live connection, built without a socket. `Client::new` does no I/O.
    ///
    /// It points at **port 1 on the loopback** — reserved, unbound, refused
    /// immediately. These tests drive the fold by handing it the events a worker
    /// *would* have produced; if one of them ever does spawn a real worker, it
    /// has to fail instantly against a closed port rather than reach a hostname
    /// on the internet from a test suite.
    fn connection() -> Connection {
        Connection {
            name: "home".into(),
            username: "rod".into(),
            client: std::sync::Arc::new(
                eko_net::Client::new(eko_net::Config {
                    base_url: "http://127.0.0.1:1".into(),
                    username: "rod".into(),
                    password: "hunter2".into(),
                })
                .unwrap(),
            ),
        }
    }

    fn remote_album(id: &str, name: &str, tracks: Option<Vec<remote::Track>>) -> remote::Album {
        remote::Album {
            id: id.to_string(),
            name: name.to_string(),
            artist: "Test Artist".to_string(),
            song_count: Some(2),
            tracks,
        }
    }

    fn remote_track(id: &str, title: &str, n: u32) -> remote::Track {
        remote::Track {
            id: id.to_string(),
            title: title.to_string(),
            artist: "Test Artist".to_string(),
            album: "Remote Album".to_string(),
            track_no: Some(n),
            duration: 120.0,
        }
    }

    /// A remote event stamped with the generation the fold is listening to.
    fn remote_event(a: &App, kind: RemoteKind) -> AppEvent {
        AppEvent::Remote(RemoteEvent {
            generation: a.remote_generation,
            kind,
        })
    }

    /// Put the sidebar cursor on the server row, the way `tab` + `j` would.
    fn select_server(a: &mut App) {
        let was = a.focus;
        a.focus = Focus::Sidebar;
        while a.source() != Source::Remote {
            a.act(Action::Down);
        }
        a.focus = was;
    }

    /// A Deck connected to `home`, with `albums` fetched, browsing the server.
    fn browsing(albums: Vec<remote::Album>) -> App {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::Connected(Box::new(connection()))));
        let page = remote_event(&a, RemoteKind::Page(albums));
        a.handle(page);
        let done = remote_event(&a, RemoteKind::AlbumsReady);
        a.handle(done);
        select_server(&mut a);
        a
    }

    /// **The server is a real row.**
    ///
    /// The previous phase left it under a dim `NOT YET` heading because nothing
    /// was behind it. Something is now, and a heading that says "not built" over
    /// a live connection is the same inversion in the other direction.
    #[test]
    fn a_configured_server_is_a_real_row_beside_local() {
        let a = with_a_server();
        assert_eq!(
            a.sources
                .iter()
                .map(|r| r.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Local", "home", "Search", "Queue"],
        );
        assert_eq!(a.sources[1].source, Source::Remote);
        // Search is a row only because the server is: it is `search3`, and a
        // Deck with nothing to ask has nothing to put behind it.
        assert_eq!(a.sources[2].source, Source::Search);
        assert_eq!(a.visible_sources(), vec![0, 1, 2, 3]);
    }

    /// No server configured, no *library* row — an invitation instead.
    ///
    /// A row is still a claim there is something behind it; the claim this one
    /// makes is "press enter and you can configure one", and it is kept. What
    /// there is no row for is `Search`, because a question needs something to
    /// ask.
    #[test]
    fn no_configured_server_means_an_invitation_rather_than_a_library() {
        let a = app();
        assert_eq!(a.sources.len(), 3, "Local, the invitation, and Queue");
        let remote: Vec<&SourceRow> = a
            .sources
            .iter()
            .filter(|r| r.source == Source::Remote)
            .collect();
        assert_eq!(remote.len(), 1);
        assert_eq!(remote[0].label, "Add server");
        assert!(remote[0].action, "the row claimed to be a library");
        assert!(
            !a.sources.iter().any(|r| r.source == Source::Search),
            "a search row appeared with nothing to ask"
        );
        assert_eq!(a.source(), Source::Local);
        assert_eq!(a.visible_sources(), vec![0, 1, 2]);
    }

    /// **Moving the sidebar cursor is what switches library.**
    ///
    /// The heading has always followed the selection; the list now follows it
    /// too, so the two cannot disagree about which source is on screen. This is
    /// the answer to *"how do I navigate between Local and Navidrome"*.
    #[test]
    fn moving_the_sidebar_cursor_switches_which_library_the_pane_shows() {
        let mut a = browsing(vec![
            remote_album("al-1", "Remote One", None),
            remote_album("al-2", "Remote Two", None),
        ]);
        a.library = stocked().library;
        a.scan = ScanState::Ready;

        assert_eq!(a.source(), Source::Remote);
        assert_eq!(a.row_count(), 2, "the server's two albums");

        a.focus = Focus::Sidebar;
        a.act(Action::Up); // back to Local
        assert_eq!(a.source(), Source::Local);
        assert_eq!(a.selected_label(), "Local");
        assert_eq!(a.row_count(), 2, "the folder's two albums");
        assert!(a.context.starts_with("Music ·"), "{}", a.context);

        a.act(Action::Down);
        assert_eq!(a.source(), Source::Remote);
        assert_eq!(a.context, "home · 2 albums", "{}", a.context);
    }

    /// `j` / `k` / `Enter` / `Esc` do the same thing on both sources. One
    /// keymap, one set of behaviours.
    #[test]
    fn navigation_is_identical_whichever_source_has_focus() {
        let mut a = browsing(vec![
            remote_album(
                "al-1",
                "Remote One",
                Some(vec![
                    remote_track("t1", "Alpha", 1),
                    remote_track("t2", "Beta", 2),
                ]),
            ),
            remote_album("al-2", "Remote Two", None),
        ]);
        // A local library with the same shape as the remote one, so the same
        // key sequence has to land in the same place on both.
        a.library = Library {
            root: Some(PathBuf::from("/tmp/Music")),
            albums: vec![
                Album {
                    id: "Test Artist Local One".into(),
                    name: "Local One".into(),
                    artist: "Test Artist".into(),
                    tracks: vec![track("Alpha", 1), track("Beta", 2)],
                },
                Album {
                    id: "Test Artist Local Two".into(),
                    name: "Local Two".into(),
                    artist: "Test Artist".into(),
                    tracks: vec![track("Gamma", 1), track("Delta", 2)],
                },
            ],
        };
        a.scan = ScanState::Ready;

        // The exact sequence, run against each source, must produce the same
        // shape of outcome.
        for (what, source) in [("local", Source::Local), ("remote", Source::Remote)] {
            a.focus = Focus::Sidebar;
            while a.source() != source {
                a.act(Action::Down);
            }
            a.focus = Focus::Main;

            assert_eq!(a.view(), View::Albums, "{what}");
            assert_eq!(a.cursor(), 0, "{what}");
            a.act(Action::Down);
            assert_eq!(a.cursor(), 1, "{what}: j did not move");
            a.act(Action::Up);
            assert_eq!(a.cursor(), 0, "{what}: k did not move");
            a.act(Action::Open);
            assert_eq!(a.view(), View::Album(0), "{what}: enter did not open");
            assert_eq!(a.row_count(), 2, "{what}: the open album has two tracks");
            a.act(Action::Down);
            assert_eq!(a.cursor(), 1, "{what}: j did not move in the album");
            a.act(Action::Back);
            assert_eq!(a.view(), View::Albums, "{what}: esc did not go back");
            assert_eq!(a.cursor(), 0, "{what}: esc lost the album cursor");
        }
    }

    /// Each source remembers its own place. An index valid in a 4,000-album
    /// server is not valid in a two-album folder.
    #[test]
    fn each_source_keeps_its_own_cursor() {
        let mut a = browsing(vec![
            remote_album("al-1", "One", None),
            remote_album("al-2", "Two", None),
            remote_album("al-3", "Three", None),
        ]);
        a.library = stocked().library;
        a.scan = ScanState::Ready;

        a.act(Action::Down);
        a.act(Action::Down);
        assert_eq!(a.remote_album_cursor, 2);

        a.focus = Focus::Sidebar;
        a.act(Action::Up);
        a.focus = Focus::Main;
        assert_eq!(a.source(), Source::Local);
        assert_eq!(
            a.cursor(),
            0,
            "the local cursor inherited the remote one's position"
        );
        assert_eq!(a.row_count(), 2, "…which would have been out of range");

        a.focus = Focus::Sidebar;
        a.act(Action::Down);
        a.focus = Focus::Main;
        assert_eq!(a.cursor(), 2, "the remote cursor was not remembered");
    }

    /// Pages append as they land, so a big library fills in rather than
    /// appearing all at once.
    #[test]
    fn album_pages_append_as_they_arrive_and_the_badge_follows() {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::Connected(Box::new(connection()))));
        assert_eq!(a.sources[1].badge, None, "nothing counted yet");

        let page = remote_event(&a, RemoteKind::Page(vec![remote_album("a", "A", None)]));
        a.handle(page);
        assert_eq!(a.remote.albums.len(), 1);
        assert_eq!(a.remote_state, RemoteState::Loading);
        assert_eq!(a.sources[1].badge.as_deref(), Some("1"));

        let page = remote_event(
            &a,
            RemoteKind::Page(vec![
                remote_album("b", "B", None),
                remote_album("c", "C", None),
            ]),
        );
        a.handle(page);
        assert_eq!(a.remote.albums.len(), 3);
        assert_eq!(a.sources[1].badge.as_deref(), Some("3"));

        let done = remote_event(&a, RemoteKind::AlbumsReady);
        a.handle(done);
        assert_eq!(a.remote_state, RemoteState::Ready);
    }

    /// **A retry must not list every album twice.**
    ///
    /// `r` starts a second walk while the first still has pages in flight. The
    /// generation stamp is what drops them.
    #[test]
    fn a_page_from_a_superseded_walk_is_dropped_rather_than_appended() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut a = with_a_server();
        a.attach(tx);
        a.handle(AppEvent::Conn(ConnEvent::Connected(Box::new(connection()))));
        let stale = remote_event(&a, RemoteKind::Page(vec![remote_album("a", "A", None)]));
        a.handle(stale.clone());
        assert_eq!(a.remote.albums.len(), 1);

        // A retry: the connection is dropped and a new walk will be started.
        a.act(Action::Rescan);
        assert!(a.remote.is_empty(), "the retry kept the old library");

        // The old walk's next page arrives late.
        a.handle(stale);
        assert!(
            a.remote.is_empty(),
            "a page from the superseded walk was appended"
        );
    }

    /// A walk that loses the server keeps what it had, and says it is
    /// incomplete rather than empty.
    #[test]
    fn a_walk_that_fails_midway_keeps_its_pages_and_says_it_is_incomplete() {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::Connected(Box::new(connection()))));
        let page = remote_event(&a, RemoteKind::Page(vec![remote_album("a", "A", None)]));
        a.handle(page);
        let failed = remote_event(
            &a,
            RemoteKind::AlbumsFailed("cannot reach the server".into()),
        );
        a.handle(failed);

        assert_eq!(a.remote.albums.len(), 1, "the page that arrived was binned");
        assert_eq!(
            a.remote_state,
            RemoteState::Failed("cannot reach the server".into())
        );
        select_server(&mut a);
        assert_eq!(a.context, "home · incomplete · 1 albums", "{}", a.context);
    }

    /// Losing the connection loses the library with it — a browsable list
    /// hanging off a connection that no longer exists is a lie.
    #[test]
    fn a_failed_connection_forgets_the_remote_library() {
        let mut a = browsing(vec![remote_album("a", "A", None)]);
        assert!(!a.remote.is_empty());
        a.handle(AppEvent::Conn(ConnEvent::Failed {
            name: "home".into(),
            message: "Wrong username or password.".into(),
        }));
        assert!(a.remote.is_empty(), "the library outlived the connection");
        assert_eq!(a.remote_state, RemoteState::Idle);
        assert_eq!(a.remote_view, View::Albums);
        assert_eq!(a.sources[1].badge, None);
    }

    /// Tracks land on the album that was asked for, and only report against the
    /// album on screen.
    #[test]
    fn track_results_land_on_the_album_that_was_asked_for() {
        let mut a = browsing(vec![
            remote_album("al-1", "One", None),
            remote_album("al-2", "Two", None),
        ]);
        a.remote_view = View::Album(0);

        let tracks = remote_event(
            &a,
            RemoteKind::Tracks {
                album_id: "al-2".into(),
                tracks: vec![remote_track("t", "Elsewhere", 1)],
            },
        );
        a.handle(tracks);
        assert_eq!(a.remote.albums[0].tracks, None, "landed on the wrong album");
        assert_eq!(a.remote.albums[1].tracks.as_ref().unwrap().len(), 1);
        assert_eq!(
            a.remote_detail,
            DetailState::Idle,
            "another album's result must not clear this one's state"
        );

        // A failure for an album that is not on screen is not news either.
        let failed = remote_event(
            &a,
            RemoteKind::TracksFailed {
                album_id: "al-2".into(),
                message: "boom".into(),
            },
        );
        a.handle(failed);
        assert_eq!(a.remote_detail, DetailState::Idle);

        // …but one for the open album is.
        let failed = remote_event(
            &a,
            RemoteKind::TracksFailed {
                album_id: "al-1".into(),
                message: "boom".into(),
            },
        );
        a.handle(failed);
        assert_eq!(a.remote_detail, DetailState::Failed("boom".into()));
    }

    /// An album whose tracks are already in hand is not re-fetched, and is
    /// immediately navigable.
    #[test]
    fn opening_an_album_whose_tracks_are_loaded_needs_no_fetch() {
        let mut a = browsing(vec![remote_album(
            "al-1",
            "One",
            Some(vec![remote_track("t1", "Alpha", 1)]),
        )]);
        a.act(Action::Open);
        assert_eq!(a.remote_view, View::Album(0));
        assert_eq!(a.remote_detail, DetailState::Idle);
        assert_eq!(a.row_count(), 1);
    }

    // ── the transport is source-aware ────────────────────────────────────

    /// Two libraries with **deliberately colliding indices**: remote album 0 and
    /// local album 0 both hold three tracks, and every title differs. Any
    /// confusion between the two is therefore a wrong *name*, not a silent
    /// no-op — which is exactly the failure mode being guarded against.
    fn two_colliding_libraries() -> App {
        let mut a = browsing(vec![
            remote_album(
                "al-1",
                "Remote Album Zero",
                Some(vec![
                    remote_track("t1", "Remote Alpha", 1),
                    remote_track("t2", "Remote Beta", 2),
                    remote_track("t3", "Remote Gamma", 3),
                ]),
            ),
            remote_album("al-2", "Remote Album One", None),
        ]);
        // `stocked` is album 0 "First" — One / Two / Three — and album 1
        // "Second". Same shape, same indices, different everything else.
        a.library = stocked().library;
        a.scan = ScanState::Ready;
        a
    }

    /// **`Enter` on a remote track plays *that* track.**
    ///
    /// Before this task `App::play` always indexed [`App::library`], and the only
    /// thing standing between the server's list and a completely unrelated local
    /// track was a "streaming is not wired yet" note. Wire the URL path without
    /// making the transport source-aware and remote track 1 starts **local**
    /// track 1 — a wrong track playing, with a seal on screen for audio nobody
    /// asked for.
    ///
    /// This is the test that fails without [`NowPlaying::source`]: with the old
    /// `play(album, track)` the assertions below read `"Two"` and `"First"`.
    #[test]
    fn enter_on_a_remote_track_plays_the_remote_track_not_the_local_one_beside_it() {
        let mut a = two_colliding_libraries();

        a.act(Action::Open); // open remote album 0
        a.act(Action::Down); // cursor to remote track 1
        a.act(Action::Open); // play it

        assert_eq!(a.playback, Playback::Playing);
        let now = a.now.clone().expect("something is playing");
        assert_eq!(
            now.source,
            Source::Remote,
            "the transport does not know which library it is playing from"
        );
        assert_eq!((now.album, now.track), (0, 1));
        assert_eq!(
            now.title, "Remote Beta",
            "a local track was started from the server's list"
        );
        assert_eq!(now.album_name, "Remote Album Zero");
        assert_eq!(now.artist, "Test Artist");
    }

    /// The mirror: `Enter` on a **local** track is untouched by the server being
    /// on screen at all.
    #[test]
    fn enter_on_a_local_track_still_plays_the_local_one() {
        let mut a = two_colliding_libraries();
        a.focus = Focus::Sidebar;
        a.act(Action::Up); // back to Local
        a.focus = Focus::Main;

        a.act(Action::Open); // open local album 0
        a.act(Action::Down); // cursor to local track 1
        a.act(Action::Open);

        let now = a.now.clone().expect("something is playing");
        assert_eq!(now.source, Source::Local);
        assert_eq!(now.title, "Two");
        assert_eq!(now.album_name, "First");
    }

    /// `space` from a cold start on the remote source starts the remote track
    /// under the cursor — never the local album at the same index.
    #[test]
    fn space_on_the_remote_source_starts_the_remote_track_not_a_local_album() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open); // open remote album 0

        a.handle(key(KeyCode::Char(' ')));
        let now = a.now.clone().expect("space started nothing");
        assert_eq!(now.source, Source::Remote);
        assert_eq!(now.title, "Remote Alpha");
    }

    /// An album whose tracks have not landed has nothing to play, and starts
    /// nothing rather than falling through to the other library.
    #[test]
    fn space_on_a_remote_album_with_no_tracks_yet_starts_nothing_at_all() {
        let mut a = browsing(vec![remote_album("al-1", "One", None)]);
        a.library = stocked().library;
        a.scan = ScanState::Ready;

        a.handle(key(KeyCode::Char(' ')));
        assert_eq!(a.playback, Playback::Stopped);
        assert!(
            a.now.is_none(),
            "space started a local album from a remote list"
        );
    }

    /// `n` and `p` walk the album that is **playing**, not the one on screen.
    ///
    /// The cursor can be moved to the other source mid-track; what is playing
    /// cannot change because of that.
    #[test]
    fn n_and_p_walk_the_playing_library_even_when_the_cursor_has_left_it() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open);
        a.act(Action::Open); // play remote track 0

        // Tab away to the local source entirely.
        a.focus = Focus::Sidebar;
        a.act(Action::Up);
        a.focus = Focus::Main;
        assert_eq!(a.source(), Source::Local);

        a.handle(key(KeyCode::Char('n')));
        let now = a.now.clone().expect("n stopped playback");
        assert_eq!(now.source, Source::Remote, "n jumped library");
        assert_eq!(now.title, "Remote Beta");

        a.handle(key(KeyCode::Char('p')));
        assert_eq!(a.now.as_ref().unwrap().title, "Remote Alpha");
        // …and it wraps within the remote album's three tracks, not the local
        // album's three.
        a.handle(key(KeyCode::Char('p')));
        assert_eq!(a.now.as_ref().unwrap().title, "Remote Gamma");
    }

    // ── the stream URL ───────────────────────────────────────────────────

    /// **The URL handed to the engine is the direct one, not the `stream://`
    /// proxy.**
    ///
    /// `eko_net::urls::stream_url` returns `stream://localhost/?src=…`, which is
    /// Tauri's protocol handler: `reqwest` in a terminal process cannot resolve
    /// that scheme at all, so a track played through it would fail to open with
    /// no explanation. `stream_src_url` is the direct, authenticated upstream.
    #[test]
    fn a_remote_track_plays_from_the_direct_upstream_url_not_the_tauri_proxy() {
        let a = browsing(vec![remote_album("al-1", "One", None)]);
        let url = a
            .stream_url("tr-42")
            .expect("a live connection mints a URL");

        assert!(
            !url.starts_with("stream://"),
            "the Tauri proxy URL cannot resolve in a terminal: {url}"
        );
        assert!(
            url.starts_with("http://127.0.0.1:1/rest/stream?"),
            "not the stream endpoint: {url}"
        );
        assert!(url.contains("id=tr-42"), "{url}");
        // Original bytes, not the server's transcode.
        assert!(url.contains("format=raw"), "{url}");
        // Signed: `u` + `t` + `s` is what makes it playable without a header.
        for param in ["u=rod", "&t=", "&s="] {
            assert!(url.contains(param), "unsigned: {url}");
        }
        // The password itself never travels — only the salted digest of it.
        assert!(!url.contains("hunter2"), "{url}");
    }

    /// No live client, no URL — and therefore nothing played and a note saying
    /// why, rather than a key that silently does nothing.
    #[test]
    fn a_remote_track_with_no_connection_says_so_instead_of_playing_nothing() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open); // open remote album 0
        a.connection = None;
        a.act(Action::Open); // try to play track 0

        assert!(a.stream_url("tr-1").is_none());
        assert_eq!(a.playback, Playback::Stopped);
        assert!(a.now.is_none(), "a local track was started as a fallback");
        let note = a.status.clone().expect("a note");
        assert!(note.contains("not connected"), "{note}");
        // And the queue was not filled with a list nothing can play out of.
        assert!(a.queue.is_empty(), "an unplayable album became the queue");
    }

    /// **A dropped connection does not move the `▶`.**
    ///
    /// `current` is the queue's claim about what is *playing*, and the marker in
    /// the Queue pane is drawn from it. So the position moves only for an entry
    /// that can actually be started: without this, `n` onto a remote entry with
    /// no client left the old session running, the footer naming the old track,
    /// and the queue marking the new one.
    #[test]
    fn n_onto_an_unplayable_remote_entry_leaves_the_queue_where_it_was() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open);
        a.act(Action::Open); // playing remote track 0 of 3
        assert_eq!(a.queue.position(), Some(0));

        a.connection = None;
        a.handle(key(KeyCode::Char('n')));

        assert_eq!(a.queue.position(), Some(0), "the marker moved anyway");
        assert_eq!(
            a.now.as_ref().map(|n| n.title.as_str()),
            Some("Remote Alpha")
        );
        assert_eq!(a.playback, Playback::Playing, "the old session was dropped");
        let note = a.status.clone().expect("a silent dead key");
        assert!(note.contains("not connected"), "{note}");
    }

    /// **Nothing the fold holds after a remote play can carry a credential.**
    ///
    /// The URL is minted inside `play_remote` and dropped there. Anything that
    /// survived — in `now`, in the note, in a `{:?}` of the whole app's public
    /// state — would be a replayable token sitting in memory for the session.
    #[test]
    fn playing_a_remote_track_leaves_no_signed_url_behind_it() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open);
        a.act(Action::Open);

        let printed = format!("{:?} {:?} {:?} {:?}", a.now, a.status, a.remote, a.context);
        for secret in ["hunter2", "/rest/stream", "&t=", "&s=", "127.0.0.1:1"] {
            assert!(!printed.contains(secret), "{secret:?} survived: {printed}");
        }
    }

    /// An empty server is a normal finished walk, and a navigable one.
    #[test]
    fn an_empty_server_is_a_finished_walk_not_a_failure() {
        let a = browsing(Vec::new());
        assert_eq!(a.remote_state, RemoteState::Ready);
        assert_eq!(a.row_count(), 0);
        assert_eq!(a.context, "home · empty");
        assert!(a.status.is_none(), "an empty server is not an error");
    }

    /// Navigating an empty remote list is inert rather than a panic — the same
    /// property the local list has.
    #[test]
    fn navigating_an_empty_remote_library_is_inert() {
        let mut a = browsing(Vec::new());
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Enter,
            KeyCode::Esc,
        ] {
            a.handle(key(code));
        }
        assert_eq!(a.remote_view, View::Albums);
        assert_eq!(a.cursor(), 0);
        assert_eq!(a.row_count(), 0);
    }

    /// An album the server lists but has no tracks for is a nameable state, not
    /// a stuck spinner.
    #[test]
    fn an_album_with_no_tracks_is_open_and_empty_rather_than_loading_forever() {
        let mut a = browsing(vec![remote_album("al-1", "Nothing", None)]);
        a.act(Action::Open);
        let tracks = remote_event(
            &a,
            RemoteKind::Tracks {
                album_id: "al-1".into(),
                tracks: Vec::new(),
            },
        );
        a.handle(tracks);
        assert_eq!(a.remote_view, View::Album(0));
        assert_eq!(a.remote_detail, DetailState::Idle, "still claiming to load");
        assert_eq!(a.remote.albums[0].tracks, Some(Vec::new()));
        assert_eq!(a.row_count(), 0);
    }

    // ── the transport ────────────────────────────────────────────────────

    #[test]
    fn enter_on_a_track_starts_it_and_records_what_is_playing() {
        let mut a = stocked();
        a.handle(key(KeyCode::Enter)); // open album 0
        a.handle(key(KeyCode::Char('j'))); // cursor to track 1
        a.handle(key(KeyCode::Enter)); // play it

        assert_eq!(a.playback, Playback::Playing);
        let now = a.now.clone().expect("something is playing");
        assert_eq!((now.album, now.track), (0, 1));
        assert_eq!(now.title, "Two");
        assert_eq!(now.album_name, "First");
        assert_eq!(a.pos_ms, 0);
    }

    #[test]
    fn space_toggles_pause_and_resume() {
        let mut a = stocked();
        a.play(Source::Local, 0, 0);
        a.handle(key(KeyCode::Char(' ')));
        assert_eq!(a.playback, Playback::Paused);
        a.handle(key(KeyCode::Char(' ')));
        assert_eq!(a.playback, Playback::Playing);
    }

    #[test]
    fn space_from_a_cold_start_plays_the_row_under_the_cursor() {
        let mut a = stocked();
        a.handle(key(KeyCode::Char('j'))); // album 1
        a.handle(key(KeyCode::Char(' ')));
        assert_eq!(a.playback, Playback::Playing);
        assert_eq!(a.now.as_ref().unwrap().album, 1);
        assert_eq!(a.now.as_ref().unwrap().track, 0);
    }

    #[test]
    fn n_and_p_walk_the_album_and_wrap_at_its_ends() {
        let mut a = stocked();
        a.play(Source::Local, 0, 0);
        a.handle(key(KeyCode::Char('n')));
        assert_eq!(a.now.as_ref().unwrap().track, 1);
        a.handle(key(KeyCode::Char('n')));
        assert_eq!(a.now.as_ref().unwrap().track, 2);
        a.handle(key(KeyCode::Char('n')));
        assert_eq!(a.now.as_ref().unwrap().track, 0, "wraps to the top");
        a.handle(key(KeyCode::Char('p')));
        assert_eq!(a.now.as_ref().unwrap().track, 2, "and back round");
    }

    #[test]
    fn n_and_p_do_nothing_when_nothing_is_loaded() {
        let mut a = stocked();
        a.handle(key(KeyCode::Char('n')));
        assert!(a.now.is_none());
        assert_eq!(a.playback, Playback::Stopped);
    }

    #[test]
    fn seeking_moves_the_clock_and_never_goes_negative() {
        let mut a = stocked();
        a.play(Source::Local, 0, 0);
        a.pos_ms = 30_000;
        a.handle(key(KeyCode::Char(']')));
        assert_eq!(a.pos_ms, 35_000);
        a.handle(key(KeyCode::Char('[')));
        assert_eq!(a.pos_ms, 30_000);
        a.pos_ms = 1_000;
        a.handle(key(KeyCode::Left));
        assert_eq!(a.pos_ms, 0, "clamped at the start of the track");
    }

    #[test]
    fn seeking_with_nothing_loaded_is_inert() {
        let mut a = stocked();
        a.handle(key(KeyCode::Char(']')));
        assert_eq!(a.pos_ms, 0);
    }

    // ── a stream that cannot be opened ───────────────────────────────────

    /// **A stream that fails to open must not leave a claim on screen.**
    ///
    /// The connection in these fixtures points at **port 1 on the loopback** —
    /// reserved, unbound, refused immediately — so this is the real
    /// `Engine::play_url` path against a server that is not there. It is the same
    /// shape as a 404, a timeout, or a reverse proxy answering with an HTML login
    /// page: `open_source` returns `None`, `decode_and_play` stores
    /// `playing = false`, and **no rate is ever written**.
    ///
    /// Three things have to be true afterwards, and the third is the one the
    /// desktop app gets wrong — its equivalent path leaves a permanently green
    /// seal:
    ///
    /// 1. the transport is `Stopped`, so the footer says *Nothing playing*;
    /// 2. there is a note naming the track that would not start; and
    /// 3. `stream` is `None`, `derive` is inactive, and the lamp is hollow.
    ///
    /// No audio device is touched: the open fails long before one is asked for.
    #[test]
    fn a_stream_that_cannot_be_opened_stops_and_seals_nothing() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open); // open remote album 0
        a.act(Action::Open); // play track 0 — at a refused port
        assert_eq!(a.playback, Playback::Playing, "the play never started");
        assert!(a.status.is_none(), "a fresh start clears the note");

        // Poll exactly as the event loop does until the engine gives up.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while a.playback == Playback::Playing && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            a.handle(AppEvent::Tick);
        }

        assert_eq!(
            a.playback,
            Playback::Stopped,
            "a stream that never opened is still claiming to play"
        );
        assert!(a.stream.is_none(), "a failed open described a stream");

        // **`active`, not `pure`, is the guard here.** `derive` computes `pure`
        // from the flags alone, and with no `StreamInfo` at all every flag is
        // false — so a failed open derives `active: false` **and `pure: true`**.
        // That is not a bug in `derive`; it is why the footer branches on
        // `seal.active` first and only then looks at `pure`. Anything that read
        // `pure` on its own would paint the green lamp over a stream that never
        // opened, which is precisely the desktop app's bug on this path. The
        // rendered proof is
        // `ui::tests::a_remote_stream_that_never_opened_renders_no_claim`.
        let seal = a.seal();
        assert!(!seal.active, "a failed open derived an active seal");
        assert!(
            seal.pure,
            "`pure` no longer means what this test's premise says; re-read the \
             footer's branch order before relaxing anything"
        );

        let note = a.status.clone().expect("a failed stream said nothing");
        assert!(note.contains("Remote Alpha"), "{note}");
        assert!(note.contains("could not play"), "{note}");
        // And the note cannot be the URL that failed — it is signed.
        for secret in ["hunter2", "&t=", "&s=", "http"] {
            assert!(!note.contains(secret), "{note}");
        }

        // **And the queue did not march.** A track that never opened is not a
        // track that ended, so auto-advance must not fire: otherwise a server
        // that has gone away burns the whole queue in a few frames and the note
        // ends up naming the last entry rather than the one that broke.
        assert_eq!(a.queue.position(), Some(0), "auto-advance ran on a failure");
        assert_eq!(
            a.now.as_ref().map(|n| n.title.as_str()),
            Some("Remote Alpha")
        );
    }

    /// A track that played to the end is **not** a track that never started, and
    /// does not get the failure note.
    #[test]
    fn a_track_that_reached_the_end_is_not_reported_as_a_failure() {
        let mut a = stocked();
        a.play(Source::Local, 0, 0);
        // The engine described the stream and the playhead moved: this session
        // ran. Then it stops.
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        let mut ended = status(44_100, 44_100, 44_100);
        ended.playing = false;
        ended.pos_ms = 120_000;
        a.note_if_it_never_started(&ended);
        assert!(a.status.is_none(), "{:?}", a.status);
    }

    // ── the queue ────────────────────────────────────────────────────────

    /// Put the sidebar cursor on the Queue row.
    fn select_queue(a: &mut App) {
        let was = a.focus;
        a.focus = Focus::Sidebar;
        while a.source() != Source::Queue {
            a.act(Action::Down);
        }
        a.focus = was;
    }

    /// **Playing a track makes its album the queue.**
    ///
    /// This is where the queue comes from without anyone building one, and it is
    /// what gives auto-advance something to advance to.
    #[test]
    fn playing_a_track_queues_the_album_it_came_from() {
        let mut a = stocked();
        a.play(Source::Local, 0, 1);

        assert_eq!(a.queue.len(), 3, "the album did not become the queue");
        assert_eq!(a.queue.position(), Some(1));
        assert_eq!(a.queue.current().map(|e| e.title.as_str()), Some("Two"));
        assert_eq!(
            a.queue
                .entries()
                .iter()
                .map(|e| e.title.as_str())
                .collect::<Vec<_>>(),
            vec!["One", "Two", "Three"],
        );
        // Every entry knows how to play itself, and knows which library from.
        for entry in a.queue.entries() {
            assert_eq!(entry.source(), Source::Local);
            assert!(matches!(&entry.media, Media::Local(p) if p.ends_with(".flac")));
        }
    }

    /// A remote entry carries the **id**. The URL is minted per play and dropped.
    #[test]
    fn a_queued_remote_track_carries_its_id_and_not_a_signed_url() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open); // open remote album 0
        a.act(Action::Open); // play track 0

        assert_eq!(a.queue.len(), 3);
        assert_eq!(a.queue.current().map(Entry::source), Some(Source::Remote));
        assert_eq!(
            a.queue.current().map(|e| e.media.clone()),
            Some(Media::Remote("t1".into()))
        );
        let printed = format!("{:?}", a.queue);
        for secret in ["hunter2", "/rest/stream", "&t=", "&s=", "127.0.0.1:1"] {
            assert!(!printed.contains(secret), "{secret:?} in the queue");
        }
    }

    /// **One list, both libraries.** The thing the model exists for.
    #[test]
    fn a_local_and_a_remote_track_sit_in_the_same_queue() {
        let mut a = two_colliding_libraries();
        // Play remote album 0 — that is the queue.
        a.act(Action::Open);
        a.act(Action::Open);
        // Then add a local track to the end of it.
        a.focus = Focus::Sidebar;
        a.act(Action::Up);
        a.focus = Focus::Main;
        a.act(Action::Open); // open local album 0
        a.act(Action::Down); // cursor to "Two"
        a.act(Action::Enqueue);

        assert_eq!(a.queue.len(), 4);
        let sources: Vec<Source> = a.queue.entries().iter().map(Entry::source).collect();
        assert_eq!(
            sources,
            vec![
                Source::Remote,
                Source::Remote,
                Source::Remote,
                Source::Local
            ],
        );
        assert_eq!(a.queue.entries()[3].title, "Two");
        // And adding behind a playing track did not disturb it.
        assert_eq!(a.queue.position(), Some(0));
        assert_eq!(
            a.now.as_ref().map(|n| n.title.as_str()),
            Some("Remote Alpha")
        );
        assert_eq!(a.status.as_deref(), Some("queued Two"));
    }

    /// `a` on an album list queues the whole album, and says how many.
    #[test]
    fn a_on_an_album_queues_all_of_it() {
        let mut a = stocked();
        a.act(Action::Enqueue); // album 0, three tracks
        assert_eq!(a.queue.len(), 3);
        assert_eq!(a.status.as_deref(), Some("queued 3 tracks from First"));
        assert_eq!(
            a.sources[a.queue_row().unwrap()].badge.as_deref(),
            Some("3")
        );
        // Nothing is playing, so nothing is claimed to be.
        assert_eq!(a.queue.position(), None);
        assert_eq!(a.playback, Playback::Stopped);
    }

    /// A remote album whose tracks have not landed cannot be queued, and says so
    /// rather than queueing nothing in silence.
    #[test]
    fn a_on_a_remote_album_with_no_tracks_says_to_open_it_first() {
        let mut a = browsing(vec![remote_album("al-1", "One", None)]);
        a.act(Action::Enqueue);
        assert!(a.queue.is_empty());
        let note = a.status.clone().expect("a silent no-op");
        assert!(note.contains("open the album first"), "{note}");
    }

    /// `x` removes the entry under the cursor — and refuses the playing one,
    /// because `current` is the queue's claim about the transport.
    #[test]
    fn x_removes_a_queued_entry_but_not_the_one_playing() {
        let mut a = stocked();
        a.play(Source::Local, 0, 0);
        select_queue(&mut a);
        assert_eq!(a.row_count(), 3);

        // Cursor is on entry 0, which is playing.
        a.act(Action::Unqueue);
        assert_eq!(a.queue.len(), 3, "the playing entry was removed");
        assert_eq!(
            a.status.as_deref(),
            Some("that one is playing · press n first")
        );

        a.act(Action::Down); // entry 1
        a.act(Action::Unqueue);
        assert_eq!(a.queue.len(), 2);
        assert_eq!(a.status.as_deref(), Some("removed Two"));
        assert_eq!(
            a.queue
                .entries()
                .iter()
                .map(|e| e.title.as_str())
                .collect::<Vec<_>>(),
            vec!["One", "Three"],
        );
        assert_eq!(a.queue.position(), Some(0), "the playhead moved");
    }

    /// `Enter` in the Queue pane jumps to that entry **without** rebuilding the
    /// queue out of the album the entry came from.
    #[test]
    fn enter_in_the_queue_plays_that_entry_and_leaves_the_list_alone() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open);
        a.act(Action::Open); // remote album 0 is the queue, at 0
        a.focus = Focus::Sidebar;
        a.act(Action::Up);
        a.focus = Focus::Main;
        a.act(Action::Enqueue); // + local album 0 (3 tracks) => 6 entries

        select_queue(&mut a);
        assert_eq!(a.row_count(), 6);
        for _ in 0..4 {
            a.act(Action::Down);
        }
        a.act(Action::Open);

        assert_eq!(a.queue.len(), 6, "the queue was rebuilt from an album");
        assert_eq!(a.queue.position(), Some(4));
        let now = a.now.clone().expect("nothing started");
        assert_eq!(now.source, Source::Local);
        assert_eq!(now.title, "Two");
    }

    /// `n` walks the queue across a library boundary — which is the whole reason
    /// it walks the queue rather than an album.
    #[test]
    fn n_walks_out_of_the_remote_album_and_into_a_queued_local_track() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open);
        a.act(Action::Down);
        a.act(Action::Down);
        a.act(Action::Open); // play remote track 2, last of the album
        a.focus = Focus::Sidebar;
        a.act(Action::Up);
        a.focus = Focus::Main;
        a.act(Action::Open); // open local album 0
        a.act(Action::Enqueue); // queue local "One"

        assert_eq!(a.queue.len(), 4);
        a.handle(key(KeyCode::Char('n')));
        let now = a.now.clone().expect("n stopped playback");
        assert_eq!(now.source, Source::Local, "n did not cross the boundary");
        assert_eq!(now.title, "One");
    }

    /// The Queue pane is a real source with a real pane: a heading, a count and
    /// a cursor that moves over actual entries.
    #[test]
    fn the_queue_is_a_selectable_source_with_rows_in_it() {
        let mut a = stocked();
        select_queue(&mut a);
        assert_eq!(a.source(), Source::Queue);
        assert_eq!(a.row_count(), 0);
        assert_eq!(a.context, "queue · empty");

        a.focus = Focus::Sidebar;
        a.act(Action::Up); // back to Local
        a.focus = Focus::Main;
        a.play(Source::Local, 0, 0);
        select_queue(&mut a);
        assert_eq!(a.row_count(), 3);
        assert_eq!(a.context, "queue · 1 of 3");
        a.act(Action::Down);
        assert_eq!(a.cursor(), 1);
        // …and the pane's cursor is not the playhead.
        assert_eq!(a.queue.position(), Some(0));
    }

    /// `space` from a cold start with the Queue pane open plays the entry under
    /// the cursor, rather than falling through to a library index.
    #[test]
    fn space_on_the_queue_plays_the_entry_under_the_cursor() {
        let mut a = stocked();
        a.act(Action::Enqueue); // album 0 -> three entries, nothing playing
        select_queue(&mut a);
        a.act(Action::Down);
        a.handle(key(KeyCode::Char(' ')));

        assert_eq!(a.playback, Playback::Playing);
        assert_eq!(a.now.as_ref().map(|n| n.title.as_str()), Some("Two"));
        assert_eq!(a.queue.position(), Some(1));
    }

    // ── seeking a stream ─────────────────────────────────────────────────

    /// **A forward seek on a stream stops at the download edge, and says so.**
    ///
    /// It used to run straight past it: `seek_by` advanced `pos_ms` to wherever
    /// the arithmetic landed, and `eko-core` parks the playhead there and outputs
    /// silence until the decoder catches up. With no buffered indicator on this
    /// Deck that reads as a frozen program. See [`App::seek_by`].
    #[test]
    fn seeking_a_stream_stops_where_the_download_does() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open);
        a.act(Action::Open); // playing a remote track
        a.pos_ms = 10_000;
        a.buffered_ms = 12_000;

        a.handle(key(KeyCode::Char(']'))); // +5s, but only 2s have arrived
        assert_eq!(a.pos_ms, 12_000, "the clock ran past the download");
        let note = a.status.clone().expect("a silent clamp");
        assert!(note.contains("downloaded"), "{note}");

        // Backwards is never clamped: those bytes are already decoded.
        a.handle(key(KeyCode::Char('[')));
        assert_eq!(a.pos_ms, 7_000);
    }

    /// **A seek immediately after crossing into a new track is clamped against
    /// the *new* track's buffer, not the last one's.**
    ///
    /// `buffered_ms` is the engine's decode progress for the *current* session,
    /// and [`App::seek_by`] uses it as the furthest a stream may be seeked to.
    /// Every other snapshot of the previous track is dropped in [`App::started`]
    /// — `pos_ms`, `bands`, `stream`, `status` — and this one was not. So for the
    /// window between a track starting and the next poll (33 ms with the spectrum
    /// up, a whole second without it, and auto-advance lands squarely inside it)
    /// `]` would be clamped against a download that belongs to a track that has
    /// already finished, and park the clock at a position this session has not
    /// decoded a byte of.
    ///
    /// Without the reset this reads `5_000`: the old 90-second buffer waves the
    /// seek through.
    #[test]
    fn a_seek_straight_after_a_track_change_cannot_use_the_last_tracks_buffer() {
        let mut a = two_colliding_libraries();
        a.act(Action::Open);
        a.act(Action::Open); // the first remote track is playing
                             // Ninety seconds of it have downloaded, and the playhead is inside that.
        a.buffered_ms = 90_000;
        a.pos_ms = 40_000;

        // Cross into the next one. `n` is auto-advance's own path — see
        // [`App::step_track`] — so this is the same edge the queue crosses on its
        // own at the end of every track.
        a.act(Action::Next);
        assert_eq!(a.pos_ms, 0, "the clock did not restart");

        // Nothing of *this* track has arrived yet, so there is nowhere to seek
        // to, and the note says why rather than the clock silently jumping.
        a.act(Action::SeekForward);
        assert_eq!(
            a.pos_ms, 0,
            "the seek was clamped against the previous track's download"
        );
        let note = a.status.clone().expect("a silent clamp");
        assert!(note.contains("downloaded"), "{note}");
    }

    /// A local file is not clamped by the buffer — it is on disk, and the engine
    /// bounds it by the track's own end.
    #[test]
    fn seeking_a_local_file_is_not_clamped_by_the_buffer() {
        let mut a = stocked();
        a.play(Source::Local, 0, 0);
        a.pos_ms = 30_000;
        a.buffered_ms = 0;
        a.handle(key(KeyCode::Char(']')));
        assert_eq!(a.pos_ms, 35_000);
        assert!(a.status.is_none(), "{:?}", a.status);
    }

    // ── search ───────────────────────────────────────────────────────────

    /// A live [`Connection`] pointing at a mock server.
    ///
    /// The twin of [`connection`], which points at `127.0.0.1:1` precisely so
    /// nothing can answer it. These tests want the opposite: a real client, a real
    /// `search3` round trip, and a real answer folded in — the brief for this task
    /// forbids stubbing results into the model, and every fixture below therefore
    /// gets them off the wire.
    fn connection_to(base_url: String) -> Connection {
        Connection {
            name: "home".into(),
            username: "rod".into(),
            client: std::sync::Arc::new(
                eko_net::Client::new(eko_net::Config {
                    base_url,
                    username: "rod".into(),
                    password: "hunter2".into(),
                })
                .unwrap(),
            ),
        }
    }

    /// A `search3` body holding these albums and songs.
    fn search_body(albums: &[&str], songs: &[&str]) -> String {
        format!(
            r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","searchResult3":{{"album":[{}],"song":[{}]}}}}}}"#,
            albums.join(","),
            songs.join(",")
        )
    }

    /// Two albums and two songs, the fixture most of these tests search for.
    fn eden_body() -> String {
        search_body(
            &[
                r#"{"id":"al-7","name":"Spirit of Eden","artist":"Talk Talk","songCount":6}"#,
                r#"{"id":"al-8","name":"Laughing Stock","artist":"Talk Talk","songCount":6}"#,
            ],
            &[
                r#"{"id":"tr-1","title":"Eden","artist":"Talk Talk","album":"Spirit of Eden","track":2,"duration":386}"#,
                r#"{"id":"tr-2","title":"Desire","artist":"Talk Talk","album":"Spirit of Eden","track":3,"duration":418}"#,
            ],
        )
    }

    /// A connected Deck with a channel of its own, and **no album walk running**.
    ///
    /// The channel is attached *after* the connection lands, which is what keeps
    /// `start_remote_albums` from firing: it needs a sender and there is not one
    /// yet. So the only events on this channel are the ones a test asked for.
    fn connected_to(server: &mockito::ServerGuard) -> (App, std::sync::mpsc::Receiver<AppEvent>) {
        let mut a = with_a_server();
        a.handle(AppEvent::Conn(ConnEvent::Connected(Box::new(
            connection_to(server.url()),
        ))));
        let (tx, rx) = std::sync::mpsc::channel();
        a.events = Some(tx);
        (a, rx)
    }

    /// Type `query` into an open search input and press enter.
    fn ask(a: &mut App, query: &str) {
        a.act(Action::Search);
        for c in query.chars() {
            a.handle(key(KeyCode::Char(c)));
        }
        a.handle(key(KeyCode::Enter));
    }

    /// Ask, wait for the answer, fold it in. **A real round trip.**
    fn searched(a: &mut App, rx: &std::sync::mpsc::Receiver<AppEvent>, query: &str) {
        ask(a, query);
        let event = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the search worker never answered");
        a.handle(event);
    }

    fn mock_search(server: &mut mockito::ServerGuard, query: &str, body: &str) -> mockito::Mock {
        server
            .mock("GET", "/rest/search3")
            .match_query(mockito::Matcher::Regex(format!(r"query={query}&songCount")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    /// `/` opens a text input, and every key that reaches it is **text**.
    #[test]
    fn slash_opens_an_input_that_takes_typing_backspace_and_esc() {
        let mut server = mockito::Server::new();
        let (mut a, _rx) = connected_to(&server);
        let _m = mock_search(&mut server, "ed", &eden_body());

        assert!(a.search.input.is_none(), "the input was already open");
        a.handle(key(KeyCode::Char('/')));
        assert_eq!(a.search.input.as_deref(), Some(""));
        // …and it took you to the pane the answer will appear in.
        assert_eq!(a.source(), Source::Search);
        assert_eq!(a.focus, Focus::Main);

        // Keys that are bound to actions everywhere else are now letters.
        for c in "eden".chars() {
            a.handle(key(KeyCode::Char(c)));
        }
        assert_eq!(a.search.input.as_deref(), Some("eden"));
        assert_eq!(
            a.playback,
            Playback::Stopped,
            "a letter in the query reached the transport"
        );

        a.handle(key(KeyCode::Backspace));
        a.handle(key(KeyCode::Backspace));
        assert_eq!(a.search.input.as_deref(), Some("ed"));

        // Esc closes the box without asking anything.
        a.handle(key(KeyCode::Esc));
        assert_eq!(a.search.input, None);
        assert_eq!(a.search.state, SearchState::Idle);
        assert!(a.search.query.is_empty(), "esc ran the query anyway");
    }

    /// **`Ctrl-C` still quits while a query is being typed.**
    ///
    /// Raw mode swallows `SIGINT`, so this is the only way out of a text field
    /// that has taken every other key. The editor declines anything with
    /// `CONTROL` held and the keymap sees it unchanged.
    #[test]
    fn ctrl_c_quits_out_of_the_search_input() {
        let server = mockito::Server::new();
        let (mut a, _rx) = connected_to(&server);
        a.handle(key(KeyCode::Char('/')));
        a.handle(key(KeyCode::Char('c')));
        assert_eq!(a.search.input.as_deref(), Some("c"), "a bare c is a letter");
        assert!(!a.should_quit());

        let ctrl_c = AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        a.handle(ctrl_c);
        assert!(a.should_quit(), "ctrl-c was swallowed by the search input");
    }

    /// **A pasted query cannot carry an escape into the summary row.**
    ///
    /// The query is echoed — in the row under `SEARCH`, and in "nothing matched
    /// …" — so it is server-grade text the moment a clipboard is involved. It goes
    /// through [`server::tidy`], the one gate, rather than a second rule of its
    /// own.
    #[test]
    fn a_pasted_query_goes_through_the_same_gate_as_the_server() {
        let server = mockito::Server::new();
        let (mut a, _rx) = connected_to(&server);
        a.handle(key(KeyCode::Char('/')));
        a.handle(AppEvent::Input(Event::Paste(
            "tal\u{1b}[2Jk\nta\u{202e}lk".to_string(),
        )));
        let input = a.search.input.clone().expect("the input closed on a paste");
        assert!(
            !input.chars().any(char::is_control),
            "a control character reached the query: {input:?}"
        );
        assert!(!input.contains('\u{202e}'), "{input:?}");
        assert_eq!(input, "tal [2Jk ta lk");

        // A paste with no input open is not a gesture this application has.
        a.handle(key(KeyCode::Esc));
        a.handle(AppEvent::Input(Event::Paste("ignored".to_string())));
        assert_eq!(a.search.input, None);
    }

    /// A query longer than the cap is truncated rather than refused — a paste
    /// that silently did nothing would read as a broken terminal.
    #[test]
    fn an_over_long_query_is_truncated_rather_than_dropped() {
        let server = mockito::Server::new();
        let (mut a, _rx) = connected_to(&server);
        a.handle(key(KeyCode::Char('/')));
        a.handle(AppEvent::Input(Event::Paste("x".repeat(MAX_QUERY * 3))));
        assert_eq!(a.search.input.as_deref().map(str::len), Some(MAX_QUERY));
    }

    /// With no server there is nothing to ask, so `/` says so instead of opening
    /// a box that could only ever be cancelled.
    #[test]
    fn slash_with_no_server_says_so_rather_than_opening_an_input() {
        let mut a = app();
        a.handle(key(KeyCode::Char('/')));
        assert_eq!(a.search.input, None);
        assert!(a.sources.iter().all(|r| r.source != Source::Search));
        let note = a.status.clone().expect("a silent refusal");
        assert!(note.contains("no server configured"), "{note}");
    }

    /// **A real query, a real answer, and both halves in one list.**
    #[test]
    fn a_query_answers_with_albums_and_songs_in_one_list() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let mock = mock_search(&mut server, "eden", &eden_body());

        searched(&mut a, &rx, "eden");
        mock.assert();

        assert_eq!(a.search.state, SearchState::Ready);
        assert_eq!(a.search.query, "eden");
        assert_eq!(a.source(), Source::Search);
        // Two albums then two songs, all reachable by the same cursor.
        assert_eq!(a.row_count(), 4);
        assert_eq!(a.search.results.albums.len(), 2);
        assert_eq!(a.search.results.songs.len(), 2);
        assert_eq!(
            a.search.song_at(2).map(|(_, s)| s.title.as_str()),
            Some("Eden")
        );
        assert_eq!(a.search.song_at(1), None, "row 1 is an album, not a song");
        assert!(a.context.contains("4 results"), "{}", a.context);
    }

    /// A query that matches nothing is a finished search with an empty list —
    /// not a failure, and not a list that is merely blank.
    #[test]
    fn a_query_that_matches_nothing_is_ready_and_empty() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","searchResult3":{}}}"#;
        let _m = mock_search(&mut server, "zzzzz", body);

        searched(&mut a, &rx, "zzzzz");
        assert_eq!(a.search.state, SearchState::Ready);
        assert_eq!(a.row_count(), 0);
        assert!(a.search.results.is_empty());
    }

    /// **A stale answer never overwrites a newer one.**
    ///
    /// Two real round trips, folded **newest first**: exactly the order a slow
    /// first query and a fast second one produce, and the order under which a fold
    /// with no generation check ends up showing the results of a question the user
    /// has already replaced. The older answer is dropped on the way in.
    ///
    /// Mutation-tested: remove the `event.generation != self.search.generation`
    /// guard in `on_search` and this reads `Spirit of Eden` — the *first* query's
    /// answer — under the second query's name.
    #[test]
    fn a_stale_search_answer_is_dropped_rather_than_shown() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let _slow = mock_search(&mut server, "eden", &eden_body());
        let _fast = mock_search(
            &mut server,
            "loveless",
            &search_body(
                &[
                    r#"{"id":"al-9","name":"Loveless","artist":"My Bloody Valentine","songCount":11}"#,
                ],
                &[],
            ),
        );

        // Two questions, back to back, neither answer folded in between.
        ask(&mut a, "eden");
        assert_eq!(a.search.state, SearchState::Searching);
        ask(&mut a, "loveless");
        assert_eq!(a.search.query, "loveless");

        let first = rx.recv_timeout(Duration::from_secs(10)).expect("answer 1");
        let second = rx.recv_timeout(Duration::from_secs(10)).expect("answer 2");
        // Deliver the newer one first and the older one after it — the ordering
        // the bug needs.
        let (newer, older) = match (&first, &second) {
            (AppEvent::Search(f), AppEvent::Search(s)) if f.generation > s.generation => {
                (first.clone(), second.clone())
            }
            _ => (second.clone(), first.clone()),
        };
        a.handle(newer);
        a.handle(older);

        assert_eq!(a.search.query, "loveless");
        let names: Vec<&str> = a
            .search
            .results
            .albums
            .iter()
            .map(|album| album.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Loveless"],
            "an answer to a question the user had already replaced was shown"
        );
        assert!(a.search.results.songs.is_empty());
    }

    /// **A search in flight never blocks a keystroke.**
    ///
    /// The server here takes 400ms to answer. Between pressing enter and that
    /// answer arriving the fold takes a hundred keys — typing a second query,
    /// backspacing it, cancelling it — and every one of them lands. If the request
    /// were on the fold's thread the loop would be inside `reqwest::blocking` for
    /// the whole of it and none of them would be seen at all.
    ///
    /// The bound is deliberately loose (half the server's delay) so this measures
    /// "the fold did not wait for the network", which is a 400ms difference, and
    /// not the speed of a hundred `String::push`es.
    #[test]
    fn a_slow_search_does_not_hold_up_a_single_keystroke() {
        let mut server = mockito::Server::new();
        let body = eden_body();
        let _slow = server
            .mock("GET", "/rest/search3")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_chunked_body(move |w| {
                std::thread::sleep(Duration::from_millis(400));
                w.write_all(body.as_bytes())
            })
            .create();
        let (mut a, rx) = connected_to(&server);

        let started = std::time::Instant::now();
        ask(&mut a, "eden");
        assert_eq!(a.search.state, SearchState::Searching);

        a.handle(key(KeyCode::Char('/')));
        for _ in 0..100 {
            a.handle(key(KeyCode::Char('x')));
        }
        // Capped at [`MAX_QUERY`], and every one of the hundred was *seen*.
        assert_eq!(a.search.input.as_deref().map(str::len), Some(MAX_QUERY));
        for _ in 0..100 {
            a.handle(key(KeyCode::Backspace));
        }
        assert_eq!(a.search.input.as_deref(), Some(""));
        a.handle(key(KeyCode::Esc));
        let typing = started.elapsed();
        assert!(
            typing < Duration::from_millis(200),
            "the fold waited on the network: {typing:?} for 201 keys"
        );
        // The answer really was outstanding for all of that.
        assert_eq!(a.search.state, SearchState::Searching);

        let event = rx.recv_timeout(Duration::from_secs(10)).expect("an answer");
        a.handle(event);
        assert_eq!(a.search.state, SearchState::Ready);
        assert_eq!(a.row_count(), 4);
    }

    /// **Enter opens an album row and plays a song row.** One key, one list.
    #[test]
    fn enter_opens_a_result_album_and_plays_a_result_song() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let _m = mock_search(&mut server, "eden", &eden_body());
        let _detail = server
            .mock("GET", "/rest/getAlbum")
            .match_query(mockito::Matcher::Regex(r"id=al-7$".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"al-7","name":"Spirit of Eden","artist":"Talk Talk","song":[
                    {"id":"tr-r","title":"The Rainbow","artist":"Talk Talk","album":"Spirit of Eden","track":1,"duration":555}
                ]}}}"#,
            )
            .create();
        searched(&mut a, &rx, "eden");

        // Row 0 is an album: Enter opens it and asks for its tracks.
        a.act(Action::Open);
        assert_eq!(a.search.view, View::Album(0));
        assert_eq!(a.search.detail, DetailState::Loading);
        let tracks = rx.recv_timeout(Duration::from_secs(10)).expect("tracks");
        a.handle(tracks);
        assert_eq!(a.search.detail, DetailState::Idle);
        assert_eq!(a.row_count(), 1, "the opened album's tracks are the rows");

        // Enter on one of them plays it, and makes that album the queue.
        a.act(Action::Open);
        let now = a.now.clone().expect("nothing started");
        assert_eq!(now.title, "The Rainbow");
        assert_eq!(now.album_name, "Spirit of Eden");
        assert_eq!(now.source, Source::Remote, "it streams from the server");
        assert_eq!(a.queue.len(), 1);

        // Esc goes back to the results, cursor on the album it came from.
        a.act(Action::Back);
        assert_eq!(a.search.view, View::Albums);
        assert_eq!(a.search.cursor, 0);
        assert_eq!(a.row_count(), 4);
    }

    /// Enter on a **song** row plays that song, and makes the matched songs the
    /// queue — "Enter in a list plays this list", the same sentence as everywhere
    /// else.
    #[test]
    fn enter_on_a_result_song_plays_it_and_queues_the_matches() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let _m = mock_search(&mut server, "eden", &eden_body());
        searched(&mut a, &rx, "eden");

        // Rows 0 and 1 are albums; row 3 is the second song.
        a.act(Action::Down);
        a.act(Action::Down);
        a.act(Action::Down);
        assert_eq!(a.cursor(), 3);
        a.act(Action::Open);

        let now = a.now.clone().expect("nothing started");
        assert_eq!(now.title, "Desire");
        assert_eq!(now.album_name, "Spirit of Eden");
        assert_eq!(a.queue.len(), 2, "the matched songs are the queue");
        assert_eq!(a.queue.position(), Some(1));
        // A result is at no index in either library, so it claims none.
        assert_eq!(now.album, NO_ROW);
        assert_eq!(now.track, NO_ROW);
    }

    /// **A result playing lights no row in either library.**
    ///
    /// The `▶` markers compare indices. A search result is at no index in either
    /// list, so a plausible-looking `0` would put the marker on an unrelated album
    /// while something else played — the same class of bug
    /// [`NowPlaying::source`] exists to prevent. [`NO_ROW`] cannot match, and the
    /// search pane marks by id instead.
    #[test]
    fn a_playing_result_cannot_light_a_row_in_either_library() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let _m = mock_search(&mut server, "eden", &eden_body());
        a.library = stocked().library;
        a.scan = ScanState::Ready;
        searched(&mut a, &rx, "eden");

        a.act(Action::Down);
        a.act(Action::Down);
        a.act(Action::Open); // the first matched song
        let now = a.now.clone().expect("nothing started");
        assert_eq!(now.title, "Eden");
        for real_row in 0..8usize {
            assert_ne!(now.album, real_row, "a real album index was claimed");
            assert_ne!(now.track, real_row, "a real track index was claimed");
        }
    }

    /// `a` queues a result — the whole album on an album row, the one song on a
    /// song row — without disturbing what is playing.
    #[test]
    fn a_queues_a_result_song_and_says_it_did() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let _m = mock_search(&mut server, "eden", &eden_body());
        searched(&mut a, &rx, "eden");

        a.act(Action::Down);
        a.act(Action::Down); // row 2: the first song
        a.act(Action::Enqueue);
        assert_eq!(a.queue.len(), 1);
        assert_eq!(a.queue.get(0).map(|e| e.title.as_str()), Some("Eden"));
        let note = a.status.clone().expect("a silent enqueue");
        assert!(note.contains("queued Eden"), "{note}");

        // An album whose tracks have not been fetched has nothing to add, and
        // says the thing that fetches them.
        a.act(Action::Up);
        a.act(Action::Up);
        a.act(Action::Enqueue);
        assert_eq!(a.queue.len(), 1, "an unopened album queued something");
        let note = a.status.clone().expect("a silent refusal");
        assert!(note.contains("open the album first"), "{note}");
    }

    /// A dropped connection takes the results with it — a list of things you
    /// could open, hanging off a server the Deck no longer has, is a list that
    /// lies about what pressing enter will do.
    #[test]
    fn losing_the_connection_forgets_the_results() {
        let mut server = mockito::Server::new();
        let (mut a, rx) = connected_to(&server);
        let _m = mock_search(&mut server, "eden", &eden_body());
        searched(&mut a, &rx, "eden");
        assert_eq!(a.row_count(), 4);

        a.handle(AppEvent::Conn(ConnEvent::Failed {
            name: "home".into(),
            message: "the server answered HTTP 502".into(),
        }));
        assert_eq!(a.row_count(), 0);
        assert_eq!(a.search.state, SearchState::Idle);
        assert!(a.search.query.is_empty());
    }

    // ── auto-advance, measured ───────────────────────────────────────────

    /// `secs` of a sine as a 16-bit stereo WAV. The same 44-byte header
    /// `tests/streaming.rs` hand-rolls, for the same reason: a header is less
    /// dependency than a dependency.
    fn sine_wav(rate: u32, secs: f64, freq: f64) -> Vec<u8> {
        let channels: u16 = 2;
        let bits: u16 = 16;
        let frames = (f64::from(rate) * secs) as u32;
        let block_align = channels * bits / 8;
        let data_len = frames * u32::from(block_align);
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let t = f64::from(i) / f64::from(rate);
            let s =
                (0.5 * (2.0 * std::f64::consts::PI * freq * t).sin() * f64::from(i16::MAX)) as i16;
            for _ in 0..channels {
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
        out
    }

    /// A library of one album holding the WAVs written into `dir`.
    fn wav_album(dir: &std::path::Path, tracks: &[(&str, f64)]) -> Library {
        std::fs::create_dir_all(dir).unwrap();
        let mut out = Vec::new();
        for (i, (title, secs)) in tracks.iter().enumerate() {
            let path = dir.join(format!("{title}.wav"));
            std::fs::write(&path, sine_wav(44_100, *secs, 220.0 * (i + 1) as f64)).unwrap();
            out.push(Track {
                path: path.to_string_lossy().into_owned(),
                title: (*title).to_string(),
                artist: "Test Tone".to_string(),
                track_no: Some(i as u32 + 1),
                duration: *secs,
            });
        }
        Library {
            root: Some(dir.to_path_buf()),
            albums: vec![Album {
                id: "Test Tone Tones".into(),
                name: "Tones".into(),
                artist: "Test Tone".into(),
                tracks: out,
            }],
        }
    }

    /// Drive the fold's own event loop until `f` holds, or give up.
    fn tick_until(a: &mut App, limit: Duration, mut f: impl FnMut(&App) -> bool) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < limit {
            if f(a) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
            a.handle(AppEvent::Tick);
        }
        f(a)
    }

    /// **Auto-advance, local to local, measured.**
    ///
    /// The gap this task closed: before it, `poll_engine` saw the first tone end
    /// and honestly went to `Stopped`. Two real WAVs are written, the first is
    /// played, and the fold is ticked exactly as the event loop ticks it. What is
    /// asserted is not a flag this crate set — it is `Engine::status()` reporting
    /// a **second** session that is `playing` with a playhead that moves.
    ///
    /// `#[ignore]`d: it claims the machine's default output device. Silent —
    /// engine volume goes to zero the moment the session exists, and `pos_ms`
    /// comes from the output callback's own counter.
    ///
    /// ```text
    /// cargo test -p eko-cli -- --ignored --nocapture auto_advance
    /// ```
    #[test]
    #[ignore = "claims the default audio output device; run it deliberately"]
    fn auto_advance_starts_the_next_local_track_when_the_first_one_ends() {
        let dir = std::env::temp_dir().join("eko-cli-auto-advance-local");
        std::fs::remove_dir_all(&dir).ok();
        let mut a = app();
        a.library = wav_album(&dir, &[("One", 1.5), ("Two", 6.0)]);
        a.scan = ScanState::Ready;

        a.play(Source::Local, 0, 0);
        a.hush();
        assert_eq!(a.queue.len(), 2);
        assert_eq!(a.now.as_ref().unwrap().title, "One");

        // 1. the first tone really is playing, and its playhead moves.
        assert!(
            tick_until(&mut a, Duration::from_secs(10), |a| a.pos_ms > 200),
            "the first track never started: pos_ms={} playback={:?}",
            a.pos_ms,
            a.playback
        );
        let first = a.engine.status().expect("a session");
        println!(
            "1. ONE   playing={} pos_ms={} dur_ms={} rate={} src={} dev={}",
            first.playing, first.pos_ms, first.dur_ms, first.rate, first.src_rate, first.dev_rate
        );
        assert!(first.playing);

        // 2. it ends, and the **second** entry starts on its own.
        assert!(
            tick_until(&mut a, Duration::from_secs(20), |a| a
                .now
                .as_ref()
                .is_some_and(|n| n.title == "Two")),
            "the queue never advanced: now={:?} playback={:?}",
            a.now.as_ref().map(|n| n.title.clone()),
            a.playback
        );
        assert_eq!(a.queue.position(), Some(1));
        assert_eq!(a.playback, Playback::Playing);

        // 3. and the second session is really running, not merely claimed.
        let before = a.engine.status().expect("a session").pos_ms;
        std::thread::sleep(Duration::from_millis(1_200));
        a.handle(AppEvent::Tick);
        let after = a.engine.status().expect("a session");
        println!(
            "2. TWO   playing={} pos_ms {before} -> {} over ~1200ms  dur_ms={}",
            after.playing, after.pos_ms, after.dur_ms
        );
        assert!(
            after.playing,
            "the second track claimed to play and did not"
        );
        assert!(
            after.pos_ms > before,
            "the second track's playhead did not move: {before} -> {}",
            after.pos_ms
        );

        a.engine.stop();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **Auto-advance across the library boundary, measured.**
    ///
    /// A local tone, then a *streamed* one served by `mockito` — the mixed queue
    /// the model exists for, proven end to end: the second entry is played from a
    /// URL minted at the moment it starts, from an id the queue was holding.
    ///
    /// The connection points at the mock server, so `stream_src_url` signs a real
    /// reachable URL. `#[ignore]`d for the same reason as its twin.
    #[test]
    #[ignore = "claims the default audio output device; run it deliberately"]
    fn auto_advance_carries_the_queue_from_a_local_track_into_a_streamed_one() {
        let dir = std::env::temp_dir().join("eko-cli-auto-advance-mixed");
        std::fs::remove_dir_all(&dir).ok();

        let mut server = mockito::Server::new();
        let _m = server
            .mock("GET", "/rest/stream")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "audio/wav")
            .with_body(sine_wav(44_100, 6.0, 660.0))
            .expect_at_least(1)
            .create();

        let mut config = Config::default();
        config.servers.push(ServerConfig {
            name: "home".into(),
            base_url: server.url(),
            username: "rod".into(),
        });
        let mut a = App::new(
            &config,
            Theme::default(),
            MusicFolder::Unset { probed: None },
        );
        a.handle(AppEvent::Conn(ConnEvent::Connected(Box::new(Connection {
            name: "home".into(),
            username: "rod".into(),
            client: std::sync::Arc::new(
                eko_net::Client::new(eko_net::Config {
                    base_url: server.url(),
                    username: "rod".into(),
                    password: "hunter2".into(),
                })
                .unwrap(),
            ),
        }))));
        a.remote = remote::Library {
            albums: vec![remote_album(
                "al-1",
                "Streamed",
                Some(vec![remote_track("t1", "Streamed Tone", 1)]),
            )],
        };
        a.library = wav_album(&dir, &[("Local Tone", 1.5)]);
        a.scan = ScanState::Ready;

        // A queue of one local track, with the streamed one behind it.
        a.play(Source::Local, 0, 0);
        a.hush();
        select_server(&mut a);
        a.act(Action::Open); // open the remote album
        a.act(Action::Enqueue); // queue its one track
        assert_eq!(a.queue.len(), 2);
        assert_eq!(a.queue.entries()[1].media, Media::Remote("t1".into()));

        assert!(
            tick_until(&mut a, Duration::from_secs(10), |a| a.pos_ms > 200),
            "the local track never started"
        );
        println!(
            "1. LOCAL  {:?} pos_ms={}",
            a.now.as_ref().map(|n| n.title.clone()),
            a.pos_ms
        );

        assert!(
            tick_until(&mut a, Duration::from_secs(25), |a| a
                .now
                .as_ref()
                .is_some_and(|n| n.title == "Streamed Tone")),
            "the queue never crossed into the stream: now={:?} playback={:?}",
            a.now.as_ref().map(|n| n.title.clone()),
            a.playback
        );
        assert_eq!(a.now.as_ref().unwrap().source, Source::Remote);
        assert_eq!(a.queue.position(), Some(1));

        // It is genuinely streaming: bytes arrived, decoded, and are being
        // consumed in real time.
        assert!(
            tick_until(&mut a, Duration::from_secs(20), |a| a.pos_ms > 200),
            "the streamed track claimed to play but the playhead never moved"
        );
        let before = a.engine.status().expect("a session").pos_ms;
        std::thread::sleep(Duration::from_millis(1_200));
        a.handle(AppEvent::Tick);
        let after = a.engine.status().expect("a session");
        println!(
            "2. STREAM playing={} pos_ms {before} -> {} over ~1200ms  rate={} src={} dev={}",
            after.playing, after.pos_ms, after.rate, after.src_rate, after.dev_rate
        );
        assert!(after.playing);
        assert!(after.pos_ms > before);

        // And nothing the fold kept afterwards is a credential.
        let printed = format!("{:?} {:?} {:?}", a.queue, a.now, a.status);
        for secret in ["hunter2", "&t=", "&s=", "/rest/stream"] {
            assert!(!printed.contains(secret), "{secret:?} survived: {printed}");
        }

        a.engine.stop();
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── the seal's inputs ────────────────────────────────────────────────

    /// A status with every field the seal reads populated.
    fn status(rate: u32, src_rate: u32, dev_rate: u32) -> EngineStatus {
        EngineStatus {
            playing: true,
            pos_ms: 1_000,
            dur_ms: 120_000,
            buffered_ms: 120_000,
            rate,
            channels: 2,
            device: "Topping E30".into(),
            src_rate,
            dev_rate,
            bits: 24,
            codec: "flac".into(),
            seg: 0,
            uid: String::new(),
        }
    }

    #[test]
    fn a_fully_reported_status_becomes_a_stream_the_seal_can_read() {
        let info = stream_info(&status(44_100, 44_100, 44_100)).expect("a stream");
        assert_eq!(info.rate, 44_100);
        assert_eq!(info.src_rate, 44_100);
        assert_eq!(info.dev_rate, 44_100);
        assert_eq!(info.codec, "flac");
        assert_eq!(info.device, "Topping E30");
    }

    /// **The false-`BIT-PERFECT` guard.** A half-filled status is not a stream.
    ///
    /// `Engine::play` returns a session before the decode thread has opened
    /// anything, and `Engine::status` floors `rate` at `1` — so a track that has
    /// only just started, or one that could not be opened at all, answers with
    /// `rate: 1, src_rate: 0, dev_rate: 0`. `derive` reads that as active
    /// (`rate != 0`) with both resample checks skipped (they need `> 0`), i.e.
    /// **`BIT-PERFECT`**. The gate is what stops it.
    #[test]
    fn a_partially_reported_status_never_reaches_the_seal() {
        use eko_core::signal_path::derive;

        for (rate, src, dev, what) in [
            (1, 0, 0, "a session that has not opened anything yet"),
            (44_100, 0, 44_100, "no source rate"),
            (44_100, 44_100, 0, "no device rate"),
            (0, 44_100, 44_100, "no output rate"),
        ] {
            assert!(
                stream_info(&status(rate, src, dev)).is_none(),
                "{what} was treated as a stream"
            );
            // And the proof that the gate is load-bearing: handed over raw, that
            // same status seals the stream.
            let mut a = app();
            a.playback = Playback::Playing;
            a.stream = Some(StreamInfo::from(&status(rate, src, dev)));
            let ungated = derive(&a.seal_input());
            a.stream = stream_info(&status(rate, src, dev));
            let gated = derive(&a.seal_input());
            assert!(!gated.active, "{what}: the gated seal was active");
            assert_ne!(
                (ungated.active, ungated.pure),
                (gated.active, gated.pure),
                "{what}: the gate changed nothing, so it is not guarding anything"
            );
        }
    }

    #[test]
    fn losing_the_session_drops_the_stream_rather_than_keeping_the_last_one() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        // Nothing was ever played, so `Engine::status` has no session to report.
        a.handle(AppEvent::Tick);
        assert!(a.stream.is_none());
    }

    #[test]
    fn starting_a_track_drops_the_previous_tracks_stream() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(96_000, 96_000, 96_000)));
        a.play(Source::Local, 0, 0);
        assert!(
            a.stream.is_none(),
            "the previous track's rates survived into the next one"
        );
    }

    #[test]
    fn a_stopped_deck_is_not_an_active_engine() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Stopped;
        assert!(!a.seal_input().engine_active);
        assert!(!a.seal().active, "a stopped deck derived an active seal");
    }

    /// **Nothing this application can be told to do attenuates the signal.**
    ///
    /// This used to be `the_volume_the_seal_sees_is_the_volume_the_meter_shows`,
    /// and what it asserted was that the two agreed: press `-`, watch the seal
    /// go from `BIT-PERFECT` to `VOLUME`, watch the meter drop to 95. Both
    /// halves were correct and the feature underneath them was not. The first
    /// press of `-` took a bit-perfect player off bit-perfect, and
    /// `signal_path.rs` sets `attenuated: volume < 1.0` — so it was not a
    /// cosmetic downgrade, it was the truth. Anyone with a DAC has a volume
    /// control on the DAC.
    ///
    /// So the assertion is inverted: there is **no reachable input** that moves
    /// [`App::seal_input`] off unity. Not a key — every key in the binding table
    /// and then the whole printable keyboard, on a Deck that is playing. Not the
    /// config — see
    /// [`a_config_asking_for_attenuation_is_ignored_rather_than_obeyed`].
    ///
    /// The keyboard sweep rather than the binding table alone is deliberate: a
    /// binding removed from `BINDINGS` but left wired in `act` would pass a
    /// table-driven test and fail this one.
    #[test]
    fn nothing_a_user_can_press_takes_the_seal_off_unity() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        assert!(a.seal().pure, "unity with no EQ is bit-perfect");

        let unity = |a: &App, what: &str| {
            assert!(
                (a.seal_input().volume - 1.0).abs() < f64::EPSILON,
                "{what} left the seal at {}",
                a.seal_input().volume
            );
            assert!(
                !a.seal().flags.attenuated,
                "{what} made the seal claim attenuation"
            );
        };

        for binding in crate::keys::BINDINGS {
            for k in binding.keys {
                for _ in 0..3 {
                    let modifiers = if k.ctrl {
                        KeyModifiers::CONTROL
                    } else {
                        KeyModifiers::NONE
                    };
                    a.handle(AppEvent::Input(Event::Key(KeyEvent::new(
                        k.code, modifiers,
                    ))));
                }
                unity(&a, binding.description);
            }
        }

        for ch in 0x20u8..=0x7e {
            a.handle(key(KeyCode::Char(char::from(ch))));
            unity(&a, "a printable key");
        }
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::Backspace,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
        ] {
            a.handle(key(code));
            unity(&a, "a navigation key");
        }

        // And the seal is still the one it started on, rather than having been
        // broken by something else along the way and read as a pass.
        a.playback = Playback::Playing;
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        assert!(
            a.seal().pure,
            "the sweep left the path impure for some other reason"
        );
    }

    /// **The seal *reads* [`App::volume`], and this is the test that says so.**
    ///
    /// Every other volume assertion in this file — the keyboard sweep above, the
    /// config one below — asserts that `seal_input().volume` **is 1.0**. A
    /// literal `volume: 1.0` in [`App::seal_input`] satisfies all of them: make
    /// that edit and the whole suite still passes, which means the field's
    /// immunity was documented and not enforced.
    ///
    /// The field is kept so the seal follows *automatically* if volume ever
    /// comes back — as a Pro feature, as a mute, as anything — and an immunity
    /// nothing enforces is not one. So this writes the private field directly,
    /// from the module that owns it, and demands the seal follow it somewhere
    /// other than unity. Replace the read with a constant and this goes red.
    #[test]
    fn the_seal_reads_the_volume_field_rather_than_a_literal() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        assert!(a.seal().pure, "unity with no EQ is not bit-perfect");

        // Neither 1.0 nor 0.0: a hardcoded unity and a hardcoded silence are
        // both mutations this has to catch, and 0.5 is neither of them.
        a.volume = 0.5;
        assert!(
            (a.seal_input().volume - 0.5).abs() < f64::EPSILON,
            "the field held 0.5 and the seal reported {} — the read is a literal",
            a.seal_input().volume
        );
        // And it is not merely carried: `derive` acts on it, so the claim on
        // screen changes with it rather than only the input struct.
        assert!(
            !a.seal().pure,
            "an attenuated path still claimed bit-perfect"
        );
        assert!(a.seal().flags.attenuated, "the seal did not say attenuated");

        a.volume = 1.0;
        assert!(a.seal().pure, "unity did not come back");
    }

    /// A `config.toml` carrying `volume = 0.72` **starts at unity**.
    ///
    /// This is the half that had no keypress behind it. Before the key and the
    /// meter were removed, that line started the player quietly down with
    /// nothing on screen to say why; removing only the key and the meter would
    /// have left it doing that with the one indicator that told you gone.
    ///
    /// The key is not an error either. `Config` is `#[serde(default)]` with no
    /// `deny_unknown_fields`, so an unrecognised key is *ignored* — an existing
    /// file keeps working, it just stops meaning anything. See
    /// `unknown_keys_are_ignored_so_a_newer_eko_can_add_them`.
    #[test]
    fn a_config_asking_for_attenuation_is_ignored_rather_than_obeyed() {
        let (cfg, _) =
            Config::from_toml("volume = 0.72").expect("an old key must not fail to parse");
        let mut a = App::new(&cfg, Theme::default(), MusicFolder::Unset { probed: None });
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        assert!((a.seal_input().volume - 1.0).abs() < f64::EPSILON);
        assert!(a.seal().pure, "a config key attenuated a bit-perfect path");
    }

    /// A fresh Deck has a flat, bypassed EQ and no ReplayGain, and says so
    /// honestly rather than by omission.
    #[test]
    fn the_seal_input_reports_no_eq_and_no_replaygain() {
        let a = stocked();
        let input = a.seal_input();
        assert!(!input.eq.active());
        assert_eq!(input.replaygain_db.get(), None);
        assert_eq!(input.replaygain_mode, RgMode::Off);
    }

    // ── the EQ, and the seal that has to follow it ───────────────────────

    /// **The seventh false-`BIT-PERFECT` path, closed before it could open.**
    ///
    /// Six of them have been found and fixed across this project, and every one
    /// was the same shape: a modifier engaged, and a seal that had no way to
    /// know. `App::seal_input` used to hardcode `eq: EqState::default()`. That
    /// was *honest* only for as long as this client had no EQ at all — the
    /// moment one could be switched on, the hardcode would have reported
    /// `BIT-PERFECT` over an EQ'd signal, and it would have been the first such
    /// path created deliberately.
    ///
    /// So this test was written, and watched to fail, **before** anything in
    /// the crate could enable the EQ: before the keymap had an `Action::Eq*`,
    /// before the panel existed, and before `Engine::set_eq` had a caller. It
    /// fails against the hardcode with `assertion failed: !seal.pure`.
    #[test]
    fn enabling_the_eq_with_a_raised_band_breaks_the_seal() {
        use crate::eq::band;

        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;

        // 1. flat and bypassed — the green lamp is earned.
        let flat = a.seal();
        assert!(flat.pure, "an untouched path is bit-perfect");
        assert_eq!(flat.seal_label, "BIT-PERFECT");

        // 2. the EQ shaping the samples.
        a.eq.set_enabled(true);
        a.eq.nudge(band(2), 6.0);

        let input = a.seal_input();
        assert!(
            input.eq.enabled,
            "the seal was handed a disabled EQ while the EQ was on"
        );
        assert_eq!(
            input.eq.gains[2], 6.0,
            "the seal was handed gains that are not this client's"
        );
        let eqd = a.seal();
        assert!(
            !eqd.pure,
            "the EQ is shaping the samples and the seal still says bit-perfect"
        );
        assert!(eqd.flags.eq_active);
        assert_eq!(eqd.seal_label, "EQ");

        // 3. flattened again — and back to where it started.
        a.eq.nudge(band(2), -6.0);
        let again = a.seal();
        assert!(again.pure, "a flattened EQ did not give the seal back");
        assert_eq!(again.seal_label, flat.seal_label);
    }

    /// The pre-amp is part of the claim too. A preset that only buys headroom
    /// still moves every sample, and the seal has to say so.
    #[test]
    fn the_preamp_alone_is_enough_to_break_the_seal() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        a.eq.set_enabled(true);
        a.eq.set_value(crate::eq::PREAMP, -3.0);
        let seal = a.seal();
        assert!((a.seal_input().eq.preamp - -3.0).abs() < f64::EPSILON);
        assert!(!seal.pure, "a pre-amp is not a free operation");
        assert_eq!(seal.seal_label, "EQ");
    }

    /// A curve dialled in with the EQ **bypassed** changes nothing, and the
    /// seal must not downgrade for it. Over-reporting is the safe direction, but
    /// it is still wrong — a seal that cried `EQ` at a bypassed curve would
    /// teach people to ignore it.
    #[test]
    fn a_bypassed_curve_does_not_downgrade_the_seal() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        a.eq.load_preset(1);
        assert!(!a.eq.enabled());
        let seal = a.seal();
        assert!(seal.pure, "a bypassed EQ took the seal: {seal:?}");
    }

    /// Every preset the client can select is reported to the seal as the
    /// numbers `eko_core` holds — asserted against
    /// [`eko_core::eq_presets::PRESETS`] itself, never a copy of it.
    #[test]
    fn the_seal_is_told_eko_cores_own_preset_numbers() {
        use eko_core::eq_presets::PRESETS;

        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        a.eq.set_enabled(true);
        for (i, preset) in PRESETS.iter().enumerate() {
            a.eq.load_preset(i);
            let input = a.seal_input();
            assert!(
                (input.eq.preamp - f64::from(preset.preamp)).abs() < f64::EPSILON,
                "{} preamp",
                preset.name
            );
            let claimed: Vec<f32> = input.eq.gains.iter().map(|&g| g as f32).collect();
            assert_eq!(claimed, preset.gains.to_vec(), "{} gains", preset.name);
            // `Flat` is the one preset that touches nothing, so it is the one
            // preset that keeps the seal.
            assert_eq!(
                a.seal().pure,
                preset.name == "Flat",
                "{} and the seal disagree about whether it does anything",
                preset.name
            );
        }
    }

    // ── the EQ: keys, the panel, and the push to the engine ──────────────

    /// Every action that changes the EQ hands the new state to the engine, and
    /// every action that does not, does not.
    ///
    /// **This is the drift test, and it is indirect for a reason.**
    /// `EngineStatus` carries no DSP fields, so nothing can read the engine's EQ
    /// back and compare it — there is no assertion available of the form "what
    /// the engine holds equals what the seal says". What *can* be pinned is the
    /// one gap that would let them differ: a mutation that never reaches
    /// `Engine::set_eq`. [`App::apply_eq`] counts its own calls, and this walks
    /// every EQ action to check the count moved exactly when the state did.
    ///
    /// The other half of the argument is structural and lives in
    /// [`crate::eq`]: one private set of fields, one pure view of it per
    /// consumer, one call site each.
    #[test]
    fn every_eq_action_hands_the_engine_the_same_state_the_seal_reports() {
        // Every action in the keymap, so a new one cannot be added without
        // being classified here.
        for binding in keys::BINDINGS {
            let mut a = stocked();
            a.eq_open = true;
            let before_eq = a.eq.clone();
            let before_pushes = a.eq_applied;
            let before_seal = a.seal_input().eq;

            a.act(binding.action);

            let changed = a.eq != before_eq;
            let pushed = a.eq_applied > before_pushes;
            assert_eq!(
                changed,
                a.seal_input().eq != before_seal,
                "{:?}: the seal and the state disagree about whether anything moved",
                binding.action
            );
            if changed {
                assert!(
                    pushed,
                    "{:?} changed the EQ without handing it to the engine — \
                     the seal would report an EQ the samples never met",
                    binding.action
                );
            }
            // The converse is allowed: `EqToggle` on an already-flat curve
            // pushes without changing what `derive` sees. Pushing more than
            // needed is inert; pushing less is the bug.
            let (enabled, preamp, gains) = a.eq.engine_args();
            let claimed = a.seal_input().eq;
            assert_eq!(enabled, claimed.enabled, "{:?}", binding.action);
            assert!((preamp - claimed.preamp).abs() < f64::EPSILON);
            let claimed_gains: Vec<f32> = claimed.gains.iter().map(|&g| g as f32).collect();
            assert_eq!(gains, claimed_gains, "{:?}", binding.action);
        }

        // And the actions that *must* move it, do.
        let mut a = stocked();
        a.eq_open = true;
        for action in [
            Action::EqToggle,
            Action::EqPresetNext,
            Action::EqPresetPrev,
            Action::Up,
            Action::Down,
        ] {
            let before = a.eq_applied;
            a.act(action);
            assert!(a.eq_applied > before, "{action:?} never reached the engine");
        }

        // Opening the panel and moving its cursor are view changes. They must
        // not touch the audio at all — not the state, and not the engine.
        let mut a = stocked();
        a.eq.set_enabled(true);
        a.eq.load_preset(1);
        let (quiet, untouched) = (a.eq_applied, a.eq.clone());
        for action in [Action::EqPanel, Action::EqNext, Action::EqPrev] {
            a.act(action);
        }
        assert_eq!(a.eq_applied, quiet, "looking at the EQ changed the EQ");
        assert_eq!(a.eq, untouched);
    }

    /// `E` engages and bypasses, and the seal follows it both ways.
    #[test]
    fn shift_e_routes_the_eq_and_the_seal_follows_it_back_out_again() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        a.act(Action::EqPresetNext); // Flat → Rock, still bypassed
        assert!(a.seal().pure, "loading a preset must not engage the EQ");

        a.handle(key(KeyCode::Char('E')));
        assert!(a.eq.enabled());
        assert_eq!(a.seal().seal_label, "EQ");

        a.handle(key(KeyCode::Char('E')));
        assert!(!a.eq.enabled());
        assert!(a.seal().pure, "bypassing did not give the seal back");
    }

    /// With the panel up, `j`/`k` move the slider and leave the list alone.
    #[test]
    fn with_the_panel_open_up_and_down_move_the_slider_not_the_list() {
        use crate::eq::band;

        let mut a = stocked();
        a.view = View::Album(0);
        a.track_cursor = 0;
        a.eq_open = true;
        a.eq_cursor = band(0);

        a.handle(key(KeyCode::Char('k')));
        a.handle(key(KeyCode::Char('k')));
        assert_eq!(a.eq.value(band(0)), 2.0 * crate::eq::GAIN_STEP);
        a.handle(key(KeyCode::Char('j')));
        assert_eq!(a.eq.value(band(0)), crate::eq::GAIN_STEP);
        assert_eq!(a.track_cursor, 0, "the list cursor moved under the panel");

        // Closed again, the same keys are the list again.
        a.eq_open = false;
        a.handle(key(KeyCode::Char('j')));
        assert_eq!(a.track_cursor, 1);
        assert_eq!(a.eq.value(band(0)), crate::eq::GAIN_STEP);
    }

    /// Esc closes the panel before it means "back", so the way out is the key
    /// everything else in the Deck backs out with.
    #[test]
    fn esc_closes_the_panel_and_leaves_the_view_where_it_was() {
        let mut a = stocked();
        a.view = View::Album(0);
        a.eq_open = true;
        a.handle(key(KeyCode::Esc));
        assert!(!a.eq_open);
        assert_eq!(a.view, View::Album(0), "esc backed out of the album too");
        a.handle(key(KeyCode::Esc));
        assert_eq!(a.view, View::Albums);
    }

    /// The cursor clamps rather than wrapping — the columns are a physical row
    /// of sliders, and running off 16k into the pre-amp is not what one does.
    #[test]
    fn the_panel_cursor_clamps_at_both_ends() {
        let mut a = stocked();
        a.eq_open = true;
        for _ in 0..40 {
            a.act(Action::EqPrev);
        }
        assert_eq!(a.eq_cursor, crate::eq::PREAMP);
        for _ in 0..40 {
            a.act(Action::EqNext);
        }
        assert_eq!(a.eq_cursor, crate::eq::COLUMNS - 1);
    }

    /// **Every new session gets the EQ again.**
    ///
    /// `Engine::new_shared` builds each session with a fresh
    /// `EqParams::default()`, and `set_eq` is a no-op with no session to write
    /// into — so an EQ engaged while the Deck was stopped, or engaged mid-album,
    /// would silently stop applying at the next track. [`App::started`] re-pushes
    /// for the same reason it re-applies the volume.
    #[test]
    fn starting_a_track_re_applies_the_eq_to_the_fresh_session() {
        let mut a = stocked();
        a.act(Action::EqToggle);
        a.act(Action::EqPresetNext);
        let before = a.eq_applied;
        a.play(Source::Local, 0, 0);
        assert!(
            a.eq_applied > before,
            "a new session was started without being told about the EQ"
        );
        // And the state itself is untouched by starting a track — the push is a
        // re-send, not a reset.
        assert!(a.eq.enabled());
        assert_eq!(a.eq.preset_name(), eko_core::eq_presets::PRESETS[1].name);
    }

    /// The panel is view state and survives nothing about it reaching the seal.
    #[test]
    fn opening_the_panel_does_not_move_the_seal() {
        let mut a = stocked();
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.playback = Playback::Playing;
        let before = a.seal();
        a.act(Action::EqPanel);
        assert!(a.eq_open);
        assert_eq!(a.seal(), before, "opening a panel changed the claim");
    }

    // ── redraw-on-change ─────────────────────────────────────────────────

    #[test]
    fn the_first_frame_is_always_dirty() {
        assert!(app().take_dirty());
    }

    #[test]
    fn taking_the_dirty_flag_clears_it() {
        let mut a = app();
        assert!(a.take_dirty());
        assert!(!a.take_dirty());
    }

    // ── the visualiser ───────────────────────────────────────────────────

    /// **`z` changes the cover's key, and `esc` changes it back.**
    ///
    /// The whole of "reuse `art.rs`, only the rect changes". The token — what
    /// picture — is identical; `cols` and `rows` are not, so the cache asks the
    /// same worker for the same file at a different grid rather than stretching
    /// eight cells of footer art across half a terminal.
    #[test]
    fn opening_the_visualiser_asks_for_the_same_cover_at_a_bigger_grid() {
        let mut a = playing_locally();
        a.set_term_size(100, 30);
        let footer = a.art.wants().cloned().expect("no cover in the footer");
        assert_eq!(footer.cols, crate::ui::footer::ART_WIDTH);

        a.act(Action::Visualiser);
        let big = a.art.wants().cloned().expect("no cover in the visualiser");
        assert_eq!(big.token, footer.token, "it is not the same picture");
        assert_eq!(big.protocol, footer.protocol, "it is not the same renderer");
        assert!(big.cols > footer.cols && big.rows > footer.rows, "{big:?}");
        assert_eq!(big.cols, big.rows * 2, "the big cover is not square");

        a.act(Action::Back);
        assert!(!a.visualiser);
        assert_eq!(
            a.art.wants(),
            Some(&footer),
            "leaving did not put the cover back in the footer"
        );
    }

    /// **An envelope is wanted only with the view open, and only for a file.**
    ///
    /// Four states, one answer each, and the remote one is stated as *no shape*
    /// rather than left loading forever.
    #[test]
    fn an_envelope_is_wanted_for_a_local_track_with_the_view_open_and_never_otherwise() {
        let mut a = playing_locally();
        a.set_term_size(100, 30);
        // Closed: nothing is decoded for a row that is not on screen.
        assert_eq!(a.wave.wants(), None);

        a.act(Action::Visualiser);
        let key = a.wave.wants().cloned().expect("no envelope was asked for");
        assert_eq!(key.token, "local:/eko-cli-test/one-a.flac");
        // Keyed on the same token as the cover, so the two cannot disagree
        // about which track is current.
        assert_eq!(key.token, a.art.wants().unwrap().token);

        // A new track asks again, under a new generation.
        a.play(Source::Local, 1, 0);
        let next = a
            .wave
            .wants()
            .cloned()
            .expect("no envelope for the new track");
        assert_ne!(next, key);

        // Closing stops wanting it, so an abandoned decode is dropped.
        a.act(Action::Visualiser);
        assert_eq!(a.wave.wants(), None);

        // A resize does **not** re-ask: the envelope is resolution-independent
        // and a decode is far too expensive to redo for a wider window.
        a.act(Action::Visualiser);
        let before = a.wave.wants().cloned().unwrap();
        a.handle(AppEvent::Input(Event::Resize(240, 80)));
        assert_eq!(a.wave.wants(), Some(&before), "a resize restarted a decode");
    }

    /// A remote track has no envelope, and says so rather than waiting forever.
    #[test]
    fn a_remote_track_never_asks_for_an_envelope() {
        let mut a = app();
        a.queue.replace(
            vec![Entry {
                media: Media::Remote("42".into()),
                album: NO_ROW,
                track: NO_ROW,
                title: "Something".into(),
                artist: "Someone".into(),
                album_name: "An album".into(),
                dur_ms: 1000,
            }],
            0,
        );
        a.playback = Playback::Playing;
        a.set_term_size(100, 30);
        a.act(Action::Visualiser);
        assert_eq!(a.wave.wants(), None, "a stream was queued for a decode");
        assert!(a.wave.envelope().is_none());
    }

    /// The tick follows whatever is animated, not whichever flag was checked
    /// first. Four combinations, and the two flags are independent.
    #[test]
    fn the_tick_follows_the_bars_that_are_actually_on_screen() {
        let mut a = playing_locally();
        a.set_term_size(100, 30);
        // The Deck, spectrum up.
        assert_eq!(a.tick_interval(), Some(FRAME_INTERVAL));
        a.act(Action::ToggleSpectrum);
        assert_eq!(a.tick_interval(), Some(CLOCK_INTERVAL));

        // The visualiser's analyser is its own thing: `s` is off and the
        // analyser is still thirty-two bars that have to move.
        a.act(Action::Visualiser);
        assert!(!a.spectrum_visible);
        assert_eq!(a.tick_interval(), Some(FRAME_INTERVAL));

        // And with the analyser configured off, the view is a big cover and a
        // clock, which change once a second at most.
        a.analyser = false;
        assert_eq!(a.tick_interval(), Some(CLOCK_INTERVAL));

        // Nothing playing is still nothing to draw, in either view.
        a.playback = Playback::Stopped;
        assert_eq!(a.tick_interval(), None);
    }

    #[test]
    fn a_tick_and_a_resize_both_request_a_redraw() {
        let mut a = app();
        a.take_dirty();
        a.handle(AppEvent::Tick);
        assert!(a.take_dirty());
        a.handle(AppEvent::Input(Event::Resize(100, 30)));
        assert!(a.take_dirty());
    }

    /// `z` used to be the unbound key this asserted with, and it now opens the
    /// visualiser — so the assertion needs a key that is still bound to
    /// nothing. `y` is one, and
    /// [`crate::keys::tests::unbound_keys_resolve_to_nothing`] is what keeps it
    /// one.
    #[test]
    fn a_key_that_changes_nothing_does_not_request_a_redraw() {
        let mut a = app();
        a.take_dirty();
        a.handle(key(KeyCode::Char('y')));
        assert!(!a.take_dirty());
    }

    #[test]
    fn scan_progress_requests_a_redraw() {
        let mut a = app();
        a.take_dirty();
        a.handle(AppEvent::Scan(ScanEvent::Discovering { found: 1 }));
        assert!(a.take_dirty());
    }

    // ── the cover cache ──────────────────────────────────────────────────

    /// A local Deck with two albums, playing the first track of the first.
    fn playing_locally() -> App {
        let mut a = app();
        a.library = Library {
            root: Some(PathBuf::from("/eko-cli-test")),
            albums: vec![
                Album {
                    id: "a".into(),
                    name: "One".into(),
                    artist: "Someone".into(),
                    tracks: vec![
                        Track {
                            path: "/eko-cli-test/one-a.flac".into(),
                            title: "A".into(),
                            artist: "Someone".into(),
                            track_no: Some(1),
                            duration: 10.0,
                        },
                        Track {
                            path: "/eko-cli-test/one-b.flac".into(),
                            title: "B".into(),
                            artist: "Someone".into(),
                            track_no: Some(2),
                            duration: 10.0,
                        },
                    ],
                },
                Album {
                    id: "b".into(),
                    name: "Two".into(),
                    artist: "Someone".into(),
                    tracks: vec![Track {
                        path: "/eko-cli-test/two-a.flac".into(),
                        title: "C".into(),
                        artist: "Someone".into(),
                        track_no: Some(1),
                        duration: 10.0,
                    }],
                },
            ],
        };
        a.scan = ScanState::Ready;
        a.play(Source::Local, 0, 0);
        a.set_term_size(100, 30);
        a
    }

    /// **The cache.** A four-thousand-track library at thirty frames a second
    /// must not refetch, so the request is keyed and the key must be stable
    /// across every event that does not change the track or the grid.
    #[test]
    fn the_cover_is_asked_for_once_and_not_again_per_frame() {
        let mut a = playing_locally();
        let want = a.art.wants().cloned().expect("no cover was asked for");
        let generation = a.art.generation();
        assert_eq!(want.token, "local:/eko-cli-test/one-a.flac");
        // The block is square in device pixels — see
        // [`crate::ui::footer::ART_WIDTH`] — not a shape written down twice.
        assert_eq!(
            (want.cols, want.rows),
            (crate::ui::footer::ART_WIDTH, crate::ui::FOOTER_HEIGHT)
        );

        // Two hundred events' worth of everything that is not a track change.
        // Deliberately no `Tick`: the engine in this fixture never opened the
        // file, so the first tick correctly stops the transport and correctly
        // takes the cover down — see [`stopping_takes_the_cover_down`].
        for _ in 0..100 {
            a.handle(key(KeyCode::Char('j')));
            a.handle(AppEvent::Input(Event::Resize(100, 30)));
        }
        assert_eq!(a.art.wants(), Some(&want), "the key moved on its own");
        assert_eq!(
            a.art.generation(),
            generation,
            "the cover was requested again"
        );
    }

    /// …and it *is* asked for again when either half of the key moves.
    #[test]
    fn a_new_track_or_a_new_grid_asks_for_a_new_cover() {
        let mut a = playing_locally();
        let first = a.art.wants().cloned().unwrap();
        let generation = a.art.generation();

        a.play(Source::Local, 1, 0);
        let after_track = a.art.wants().cloned().expect("no cover for the new track");
        assert_eq!(after_track.token, "local:/eko-cli-test/two-a.flac");
        assert!(after_track != first);
        assert!(a.art.generation() > generation);

        // A width change moves the block's right edge, so the grid is the same
        // and nothing is refetched; a *narrower* Deck than the minimum has no
        // block at all, and the request has to be dropped rather than left
        // pointing at a rect that is not on screen.
        let generation = a.art.generation();
        a.handle(AppEvent::Input(Event::Resize(120, 40)));
        assert_eq!(a.art.wants(), Some(&after_track), "a resize refetched");
        assert_eq!(a.art.generation(), generation);

        a.handle(AppEvent::Input(Event::Resize(40, 10)));
        assert_eq!(
            a.art.wants(),
            None,
            "a Deck that is not drawn wants a cover"
        );
        assert!(a.art_placement().is_none());
    }

    /// Stopping takes the cover down. A picture over *Nothing playing* is a
    /// claim about something that is not happening.
    #[test]
    fn stopping_takes_the_cover_down() {
        let mut a = playing_locally();
        assert!(a.art.wants().is_some());
        a.playback = Playback::Stopped;
        a.handle(AppEvent::Tick);
        assert_eq!(a.art.wants(), None);
        assert!(a.art.rendered().is_none());
        assert!(a.art_placement().is_none());
    }

    /// **The staleness rule.** A slow fetch for the previous track must not
    /// paint its cover under the new track's title.
    #[test]
    fn a_covers_answer_is_dropped_once_the_track_has_moved_on() {
        let mut a = playing_locally();
        let stale_key = a.art.wants().cloned().unwrap();
        let stale_generation = a.art.generation();
        let outcome = crate::art::encode(&one_pixel_png(), &stale_key);
        assert!(matches!(outcome, crate::art::Outcome::Ready(_)));

        // The track changes while that fetch is still in flight.
        a.play(Source::Local, 1, 0);

        // …and then it lands.
        a.handle(AppEvent::Art(crate::art::Event {
            generation: stale_generation,
            key: stale_key.clone(),
            outcome,
        }));
        assert!(
            a.art.rendered().is_none(),
            "a stale cover was drawn under the new track"
        );

        // An answer for what is actually wanted is taken.
        let key = a.art.wants().cloned().unwrap();
        let generation = a.art.generation();
        a.handle(AppEvent::Art(crate::art::Event {
            generation,
            key: key.clone(),
            outcome: crate::art::encode(&one_pixel_png(), &key),
        }));
        assert!(a.art.rendered().is_some());

        // And so is an answer whose *generation* matches but whose key does not
        // — which is what a future second in-flight request would look like.
        let generation = a.art.generation();
        a.handle(AppEvent::Art(crate::art::Event {
            generation,
            key: stale_key,
            outcome: crate::art::Outcome::Unavailable,
        }));
        assert!(a.art.rendered().is_some(), "a mismatched key was accepted");
    }

    /// A remote track with no live client has nothing to sign a URL with. That
    /// is "no picture", not "still loading" — the placeholder must settle rather
    /// than promise something that is never coming.
    #[test]
    fn a_remote_track_with_no_connection_settles_on_no_picture() {
        let mut a = with_a_server();
        a.queue.replace(
            vec![Entry {
                media: Media::Remote("song-1".into()),
                album: 0,
                track: 0,
                title: "T".into(),
                artist: "A".into(),
                album_name: "Al".into(),
                dur_ms: 1000,
            }],
            0,
        );
        a.playback = Playback::Playing;
        a.set_term_size(100, 30);
        assert_eq!(
            a.art.wants().map(|k| k.token.clone()),
            Some("remote:song-1".to_string())
        );
        assert!(a.art.rendered().is_none());
        assert!(a.art_placement().is_none());
    }

    /// **A real round trip for a remote cover**, against a mock Navidrome: the
    /// fold mints the direct, signed `getCoverArt` URL, a worker fetches it, and
    /// the answer comes back through the channel and is drawn.
    #[test]
    fn a_remote_cover_is_fetched_decoded_and_folded_in() {
        let mut server = mockito::Server::new();
        let png = one_pixel_png();
        let _m = server
            .mock("GET", "/rest/getCoverArt")
            // `id` is the track id and `size` is asked for on the wire, because
            // a direct URL has no proxy behind it to add one.
            .match_query(mockito::Matcher::Regex(r"id=song-1&size=300".into()))
            .with_status(200)
            .with_header("content-type", "image/png")
            .with_body(png)
            .create();

        let (mut a, rx) = connected_to(&server);
        a.queue.replace(
            vec![Entry {
                media: Media::Remote("song-1".into()),
                album: 0,
                track: 0,
                title: "T".into(),
                artist: "A".into(),
                album_name: "Al".into(),
                dur_ms: 1000,
            }],
            0,
        );
        a.playback = Playback::Playing;
        a.set_term_size(100, 30);

        let event = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the cover worker never answered");
        a.handle(event);
        assert!(
            a.art.rendered().is_some(),
            "the cover never reached the fold"
        );
    }

    /// A server that 404s, or answers with something that is not an image, ends
    /// in the placeholder — not in a broken cell and not in a status line.
    #[test]
    fn a_cover_the_server_will_not_serve_degrades_to_no_picture() {
        for (what, status, body) in [
            ("404", 404, "not found"),
            ("a login page", 200, "<html><body>Sign in</body></html>"),
        ] {
            let mut server = mockito::Server::new();
            let _m = server
                .mock("GET", "/rest/getCoverArt")
                .match_query(mockito::Matcher::Any)
                .with_status(status)
                .with_body(body)
                .create();

            let (mut a, rx) = connected_to(&server);
            a.queue.replace(
                vec![Entry {
                    media: Media::Remote("song-1".into()),
                    album: 0,
                    track: 0,
                    title: "T".into(),
                    artist: "A".into(),
                    album_name: "Al".into(),
                    dur_ms: 1000,
                }],
                0,
            );
            a.playback = Playback::Playing;
            a.set_term_size(100, 30);

            let event = rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("{what}: the cover worker never answered"));
            a.handle(event);
            assert!(a.art.rendered().is_none(), "{what} produced pixels");
            assert!(a.status.is_none(), "{what} pushed a note onto the seal row");
        }
    }

    // ── the output device ────────────────────────────────────────────────

    /// A Deck with a synthetic device list, so nothing here opens CoreAudio.
    ///
    /// `App::new` only asks the host when a device is *configured*, so a default
    /// `Config` gives an empty list; these tests state one instead. Every
    /// assertion below is then about the fold rather than about which DAC happens
    /// to be plugged into the machine running the suite.
    fn with_devices() -> App {
        let mut a = stocked();
        a.devices = vec![
            "MacBook Pro Speakers".to_string(),
            "Topping E30".to_string(),
        ];
        a
    }

    #[test]
    fn a_fresh_deck_is_on_the_system_default_and_says_nothing_about_it() {
        let a = app();
        assert_eq!(a.device, OutputDevice::Default);
        assert_eq!(a.device.engine_pref(), None);
        assert!(!a.device_open);
        assert!(a.status.is_none(), "the default device raised a note");
    }

    /// **The honest degradation.** A configured device that is not connected is
    /// named in the footer at startup and the engine is handed `None` — because
    /// `eko-core` would otherwise substitute the system default in silence.
    #[test]
    fn a_configured_device_that_is_not_there_is_named_at_startup() {
        let config = Config {
            // A name no host will ever list, so this is deterministic on any
            // machine — including one with the real device plugged in.
            output_device: Some("Nonexistent DAC 9000".to_string()),
            ..Config::default()
        };
        let a = App::new(
            &config,
            Theme::default(),
            MusicFolder::Unset { probed: None },
        );
        assert_eq!(
            a.device,
            OutputDevice::Missing("Nonexistent DAC 9000".into())
        );
        assert_eq!(a.device.engine_pref(), None);
        let note = a.status.as_deref().unwrap_or_default();
        assert!(note.contains("Nonexistent DAC 9000"), "{note}");
        assert!(note.contains("system default"), "{note}");
    }

    #[test]
    fn d_opens_the_picker_with_the_cursor_on_the_current_choice() {
        let mut a = with_devices();
        a.device = OutputDevice::Configured("Topping E30".into());
        // `open_devices` re-reads the host, which would replace the fixture's
        // list, so the cursor is asked for directly — the same function the
        // panel and `open_devices` both call.
        assert_eq!(a.device_row(), 2);
        a.device = OutputDevice::Default;
        assert_eq!(a.device_row(), 0);
        assert_eq!(a.device_rows(), 3);
        // A missing device is on no row: the engine really is on the default, so
        // that is where the cursor belongs. The list has to *agree* that it is
        // missing — this used to set `Missing("Topping E30")` while the fixture's
        // list still held `Topping E30`, which is the contradiction the panel
        // then drew: a selectable row for the DAC and `is not connected` under
        // it, in the same frame. See
        // [`a_device_that_appears_in_a_refreshed_list_stops_being_missing`].
        a.set_devices(vec!["MacBook Pro Speakers".to_string()]);
        a.device = OutputDevice::Missing("Topping E30".into());
        assert_eq!(a.device_row(), 0);
        assert_eq!(a.device_rows(), 2);

        a.act(Action::Devices);
        assert!(a.device_open);
        a.act(Action::Devices);
        assert!(!a.device_open);
    }

    /// **A device is missing because the list says so, and for no other reason.**
    ///
    /// Launch with the DAC unplugged and it resolves to
    /// [`OutputDevice::Missing`]; plug it in and press `d`, and the picker
    /// re-reads the host. Before [`App::set_devices`] existed, only that *list*
    /// was refreshed — `self.device` was written at construction and by
    /// `choose_device` and nowhere else — so the panel drew a selectable
    /// `Topping E30` row from the new list and the amber `Topping E30 is not
    /// connected` sentence from the stale enum, together, while the cursor sat
    /// on `System default`.
    ///
    /// It resolves the other way too: unplug it and the row goes away as the
    /// sentence arrives. And the engine's preference follows in both directions,
    /// because a `Configured` device the engine was never told about would make
    /// the panel's "from the next track" line a lie.
    #[test]
    fn a_device_that_appears_in_a_refreshed_list_stops_being_missing() {
        let mut a = stocked();
        a.set_devices(vec!["MacBook Pro Speakers".to_string()]);
        a.device = OutputDevice::Missing("Topping E30".into());

        // Plugged in. The host lists it, so it is not missing any more.
        a.set_devices(vec![
            "MacBook Pro Speakers".to_string(),
            "Topping E30".to_string(),
        ]);
        assert_eq!(a.device, OutputDevice::Configured("Topping E30".into()));
        assert_eq!(a.device.engine_pref(), Some("Topping E30".to_string()));
        // And it now has a row, which is the whole point: the sentence and the
        // row can never both be on screen.
        assert_eq!(a.device_row(), 2);

        // Unplugged again. The row goes, the sentence comes back, and the engine
        // is handed the system default rather than a name the host cannot open.
        a.set_devices(vec!["MacBook Pro Speakers".to_string()]);
        assert_eq!(a.device, OutputDevice::Missing("Topping E30".into()));
        assert_eq!(a.device.engine_pref(), None);
        assert_eq!(a.device_row(), 0);

        // The configured name survives both trips: a DAC that was asleep once
        // must not be un-configured by it.
        assert_eq!(a.device.configured(), Some("Topping E30"));
    }

    /// The system default is not a name, so no list can make it missing.
    #[test]
    fn refreshing_the_list_leaves_the_system_default_alone() {
        let mut a = stocked();
        a.set_devices(vec!["MacBook Pro Speakers".to_string()]);
        assert_eq!(a.device, OutputDevice::Default);
        a.set_devices(Vec::new());
        assert_eq!(a.device, OutputDevice::Default);
        assert_eq!(a.device_row(), 0);
    }

    #[test]
    fn the_pickers_cursor_is_clamped_rather_than_wrapped() {
        let mut a = with_devices();
        a.device_open = true;
        a.device_cursor = 0;
        a.act(Action::Up);
        assert_eq!(a.device_cursor, 0, "the cursor wrapped off the top");
        for _ in 0..8 {
            a.act(Action::Down);
        }
        assert_eq!(a.device_cursor, 2, "the cursor ran off the end of the list");
    }

    /// `enter` on a row takes it, and the engine is handed exactly what
    /// [`OutputDevice::engine_pref`] says — never the raw name.
    #[test]
    fn enter_in_the_picker_selects_the_row_and_closes_it() {
        let mut a = with_devices();
        a.device_open = true;
        a.device_cursor = 2;
        a.act(Action::Open);
        assert!(!a.device_open);
        assert_eq!(a.device, OutputDevice::Configured("Topping E30".into()));
        let note = a.status.as_deref().unwrap_or_default();
        assert!(note.contains("Topping E30"), "{note}");

        // Row 0 is the system default, which is the *absence* of a preference.
        a.device_open = true;
        a.device_cursor = 0;
        a.act(Action::Open);
        assert_eq!(a.device, OutputDevice::Default);
        assert_eq!(a.device.engine_pref(), None);
    }

    #[test]
    fn esc_closes_the_picker_before_it_means_anything_else() {
        let mut a = with_devices();
        a.view = View::Album(0);
        a.device_open = true;
        a.act(Action::Back);
        assert!(!a.device_open);
        assert_eq!(a.view, View::Album(0), "esc also backed out of the album");
    }

    /// One overlay at a time, so no renderer and no guard needs a precedence
    /// rule for two modals at once.
    #[test]
    fn opening_the_eq_panel_closes_the_picker_and_the_other_way_round() {
        let mut a = with_devices();
        a.device_open = true;
        a.act(Action::EqPanel);
        assert!(a.eq_open);
        assert!(!a.device_open, "two modals were open at once");

        a.act(Action::Devices);
        assert!(a.device_open);
        assert!(!a.eq_open, "two modals were open at once");
    }

    /// While the picker is up, `j`/`k` move **its** cursor and not the library's
    /// — the same overload the EQ panel's sliders take.
    #[test]
    fn the_picker_takes_the_list_keys_while_it_is_open() {
        let mut a = with_devices();
        let before = a.cursor();
        a.device_open = true;
        a.act(Action::Down);
        assert_eq!(a.device_cursor, 1);
        assert_eq!(
            a.cursor(),
            before,
            "the album cursor moved behind the picker"
        );
    }

    /// **The seal cannot be left stale by a device change.**
    ///
    /// This is the assertion that the picker is wired to the *audio* and not only
    /// to a label. `Engine::set_device` writes a preference that
    /// `decode_and_play` reads once per session, so a running track keeps the old
    /// device until the session is replaced. Choosing a device while playing
    /// therefore restarts the entry, and the restart drops [`App::stream`] — so
    /// the seal reads `○ UNVERIFIED` across the switch rather than carrying the
    /// old device's `dev_rate` onto the new one.
    ///
    /// The rate the *new* device reports cannot be asserted without a real device
    /// — see `switching_output_device_moves_the_seal_on_a_real_track`, which is
    /// `#[ignore]`d and does exactly that. What is asserted here is the thing that
    /// would be a lying seal if it were missing: nothing survives the switch.
    #[test]
    fn switching_device_while_playing_restarts_the_session_and_unverifies_the_seal() {
        let mut a = with_devices();
        a.play(Source::Local, 0, 0);
        assert_eq!(a.playback, Playback::Playing);
        // A fully-reported stream, as a live 44.1 kHz track would have.
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        let before = a.seal();
        assert!(before.active && before.pure, "{before:?}");

        a.device_open = true;
        a.device_cursor = 2;
        a.act(Action::Open);

        assert_eq!(a.device, OutputDevice::Configured("Topping E30".into()));
        assert!(
            a.stream.is_none(),
            "the previous device's rates survived the switch — the seal is stale"
        );
        let after = a.seal();
        assert!(
            !after.active,
            "the seal claimed something across a device switch: {after:?}"
        );
        // `after.pure` is deliberately **not** asserted. `derive` leaves `pure` at
        // whatever it derives when `active` is false — there is nothing to be pure
        // about — and the footer reads `active` first, so the lamp is the hollow
        // `○ UNVERIFIED` regardless. Asserting it here would be asserting
        // `derive`'s internals rather than this crate's behaviour; what the
        // *screen* shows across a switch is pinned by
        // `crate::ui::tests::switching_the_output_device_never_leaves_a_green_lamp`.
        // Still playing, and it says which device and that the track restarted.
        assert_eq!(a.playback, Playback::Playing);
        let note = a.status.as_deref().unwrap_or_default();
        assert!(note.contains("restarted"), "{note}");
    }

    /// **A paused Deck's picker cannot disagree with its seal.**
    ///
    /// The third switch case, and the one that has something the other two do not:
    /// a *live session on the old device* the whole time. Pausing does not drop
    /// [`App::stream`], and [`App::set_output_device`] deliberately does not
    /// restart a paused entry — so the seal goes on naming the device the engine
    /// really opened, correctly, while the preference has moved.
    ///
    /// If `▶` came from the preference the two halves of one screen would name two
    /// different DACs. It comes from [`App::stream`] instead, so the marker stays
    /// on the device that is playing and the choice is stated as a sentence.
    #[test]
    fn switching_device_while_paused_leaves_the_marker_where_the_audio_is() {
        let mut a = with_devices();
        a.play(Source::Local, 0, 0);
        // A live session on the laptop speakers, as a paused track would have.
        let mut on_speakers = status(44_100, 44_100, 44_100);
        on_speakers.device = "MacBook Pro Speakers".to_string();
        a.stream = stream_info(&on_speakers);
        a.act(Action::PlayPause);
        assert_eq!(a.playback, Playback::Paused);

        a.device_open = true;
        a.device_cursor = 2; // Topping E30
        a.act(Action::Open);

        // The preference moved and the session did not — both deliberate.
        assert_eq!(a.device, OutputDevice::Configured("Topping E30".into()));
        assert_eq!(a.playback, Playback::Paused, "a paused Deck was restarted");
        let info = a
            .stream
            .as_ref()
            .expect("the paused session lost its stream");
        assert_eq!(
            info.device, "MacBook Pro Speakers",
            "the session moved device without being restarted"
        );

        // The screen's two statements, and they are about the same device.
        assert_eq!(
            a.device_in_use_row(),
            Some(1),
            "▶ left the device the engine is actually on"
        );
        assert_eq!(a.device_row(), 2, "the preference is not on its own row");
        assert_eq!(
            a.device_pending(),
            Some("Topping E30"),
            "the chosen device was taken with nothing on screen to say so"
        );
        let note = a.status.as_deref().unwrap_or_default();
        assert!(note.contains("next track"), "{note}");

        // And once the next session really is on it, there is nothing pending and
        // the marker moves — one fact, not two that have to be kept in step.
        let mut on_topping = status(44_100, 44_100, 44_100);
        on_topping.device = "Topping E30".to_string();
        a.stream = stream_info(&on_topping);
        assert_eq!(a.device_in_use_row(), Some(2));
        assert_eq!(a.device_pending(), None);
    }

    /// With no session there is nothing to read a marker out of — and nothing to
    /// contradict either, because the seal names no device at all. The preference
    /// carries `▶`, which is then the only true statement available.
    #[test]
    fn with_no_session_the_marker_falls_back_to_the_choice() {
        let mut a = with_devices();
        assert!(a.stream.is_none());
        a.device = OutputDevice::Configured("Topping E30".into());
        assert_eq!(a.device_in_use_row(), Some(2));
        assert_eq!(
            a.device_pending(),
            None,
            "nothing is pending on a dead deck"
        );

        // A session on a device the host is no longer listing marks **nothing**:
        // pointing at the preference instead would be the disagreement again.
        let mut gone = status(44_100, 44_100, 44_100);
        gone.device = "A DAC that was unplugged".to_string();
        a.stream = stream_info(&gone);
        assert_eq!(a.device_in_use_row(), None);
    }

    /// A **stopped** Deck is not restarted, and nothing pretends it was.
    #[test]
    fn switching_device_while_stopped_applies_at_the_next_track() {
        let mut a = with_devices();
        assert_eq!(a.playback, Playback::Stopped);
        a.device_open = true;
        a.device_cursor = 1;
        a.act(Action::Open);
        assert_eq!(a.playback, Playback::Stopped);
        let note = a.status.as_deref().unwrap_or_default();
        assert!(note.contains("next track"), "{note}");
    }

    /// Choosing what is already in use changes nothing at all — in particular it
    /// does not restart the track someone is listening to.
    #[test]
    fn choosing_the_device_already_in_use_is_inert() {
        let mut a = with_devices();
        a.play(Source::Local, 0, 0);
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.set_status("something else");
        a.device_open = true;
        a.device_cursor = 0; // already the system default
        a.act(Action::Open);
        assert_eq!(a.device, OutputDevice::Default);
        assert!(
            a.stream.is_some(),
            "an inert selection restarted the session"
        );
        assert_eq!(a.status.as_deref(), Some("something else"));
    }

    /// **The choice survives the process.** End to end: press `enter` on a row,
    /// read the file back, and build a second Deck from it.
    ///
    /// Against a temporary path, never [`crate::config::config_path`] — see
    /// [`App::config_path`] for why that distinction is load-bearing rather than
    /// tidy.
    #[test]
    fn the_chosen_device_is_written_back_and_read_again_next_launch() {
        let dir = std::env::temp_dir().join("eko-cli-device-persist");
        std::fs::remove_dir_all(&dir).ok();
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "# mine\nvolume = 0.5\n").unwrap();

        let mut a = with_devices();
        a.set_config_path(path.clone());
        a.device_open = true;
        a.device_cursor = 2;
        a.act(Action::Open);
        let note = a.status.as_deref().unwrap_or_default();
        assert!(!note.contains("not saved"), "{note}");

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("# mine"),
            "the comment was lost: {written}"
        );
        // The retired `volume` line is still there, untouched: the write is
        // string surgery on the file rather than a re-serialisation, so a key
        // this build no longer understands survives it. `Config::read` ignores
        // the key, which is the other half — see
        // `config::tests::unknown_keys_are_ignored_so_a_newer_eko_can_add_them`.
        assert!(written.contains("volume = 0.5"), "{written}");
        let (config, _) = Config::read(&path).unwrap();
        assert_eq!(config.output_device.as_deref(), Some("Topping E30"));

        // The next launch resolves it against the host, and the host does not
        // list it — so the second Deck degrades honestly rather than pretending.
        let next = App::new(
            &config,
            Theme::default(),
            MusicFolder::Unset { probed: None },
        );
        assert_eq!(next.device.configured(), Some("Topping E30"));

        // Choosing the system default takes the line back out.
        a.device_open = true;
        a.device_cursor = 0;
        a.act(Action::Open);
        assert_eq!(Config::read(&path).unwrap().0.output_device, None);
        assert!(std::fs::read_to_string(&path).unwrap().contains("# mine"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── the sleep timer ──────────────────────────────────────────────────

    #[test]
    fn t_walks_the_steps_and_then_switches_off() {
        let mut a = stocked();
        assert!(a.sleep.is_none());
        for minutes in SLEEP_STEPS {
            a.act(Action::Sleep);
            let sleep = a.sleep.as_ref().expect("a timer");
            assert_eq!(sleep.minutes, *minutes);
            assert_eq!(sleep.remaining, Duration::from_secs(minutes * 60));
            let note = a.status.as_deref().unwrap_or_default();
            assert!(note.contains(&minutes.to_string()), "{note}");
        }
        a.act(Action::Sleep);
        assert!(a.sleep.is_none(), "the cycle never came back to off");
        assert_eq!(a.status.as_deref(), Some("sleep timer · off"));
    }

    /// **It really counts down, and it really stops playback.**
    ///
    /// The remaining time is state the fold owns, so a test can set it to almost
    /// nothing and tick once rather than waiting fifteen minutes.
    #[test]
    fn the_sleep_timer_expiring_stops_playback() {
        let mut a = stocked();
        // A live transport **without** an engine session. `App::play` here would
        // hand the engine an unopenable path and race the decode thread's
        // `playing.store(false)` — `poll_engine` would then sometimes reach
        // `Stopped` on its own and this test would be asserting nothing. Stated,
        // so what stops playback below is the timer and only the timer.
        a.playback = Playback::Playing;
        a.bands = vec![0.5; 32];
        crate::ui::visualiser::hold_peaks(&mut a.peaks, &a.bands.clone());
        a.stream = Some(StreamInfo::from(&status(44_100, 44_100, 44_100)));
        a.act(Action::Sleep);
        // One millisecond left, and the clock already a moment behind.
        let sleep = a.sleep.as_mut().unwrap();
        sleep.remaining = Duration::from_millis(1);
        sleep.last = Instant::now() - Duration::from_millis(50);

        a.handle(AppEvent::Tick);
        assert_eq!(a.playback, Playback::Stopped, "the timer did not stop it");
        assert!(a.sleep.is_none(), "the timer stayed armed after firing");
        assert!(a.bands.is_empty());
        // And the analyser's held caps with them. A peak-hold over a stopped
        // transport is a picture of something that is not happening.
        assert!(a.peaks.is_empty(), "the caps outlived the transport");
        assert!(
            a.stream.is_none(),
            "the ended session's rates outlived it, so the seal still describes it"
        );
        assert!(!a.seal().active, "a stopped deck derived an active seal");
        let note = a.status.as_deref().unwrap_or_default();
        assert!(note.contains("sleep timer"), "{note}");
    }

    /// The displayed number is the real one: the countdown is reduced by the time
    /// that actually passed, not by one tick's worth.
    #[test]
    fn the_countdown_is_measured_against_the_clock_not_counted_in_ticks() {
        let mut a = stocked();
        // Stated rather than played, for the reason above.
        a.playback = Playback::Playing;
        a.act(Action::Sleep);
        let full = a.sleep.as_ref().unwrap().remaining;
        a.sleep.as_mut().unwrap().last = Instant::now() - Duration::from_secs(120);
        a.handle(AppEvent::Tick);
        let left = a.sleep.as_ref().unwrap().remaining;
        let spent = full - left;
        assert!(
            spent >= Duration::from_secs(119) && spent <= Duration::from_secs(122),
            "two minutes of wall clock cost {spent:?}"
        );
    }

    /// **Paused time is not spent.** Ticks only fire while playing, so a timer
    /// that counted a pause would have been counting time nobody could see.
    #[test]
    fn a_paused_deck_does_not_spend_the_sleep_timer() {
        let mut a = stocked();
        // Stated rather than played, for the reason above — and here it matters
        // twice, because a decode thread that gave up would put the transport at
        // `Stopped` and the resume half of this test would be about nothing.
        a.playback = Playback::Playing;
        a.act(Action::Sleep);
        let full = a.sleep.as_ref().unwrap().remaining;
        a.act(Action::PlayPause);
        assert_eq!(a.playback, Playback::Paused);
        a.sleep.as_mut().unwrap().last = Instant::now() - Duration::from_secs(600);
        a.handle(AppEvent::Tick);
        assert_eq!(
            a.sleep.as_ref().unwrap().remaining,
            full,
            "ten paused minutes came off the timer"
        );

        // And resuming does not then bill the pause to the first tick.
        a.act(Action::PlayPause);
        assert_eq!(a.playback, Playback::Playing);
        a.handle(AppEvent::Tick);
        let left = a.sleep.as_ref().unwrap().remaining;
        assert!(
            full - left < Duration::from_secs(2),
            "the pause was charged on resume: {:?} of {full:?} left",
            left
        );
    }

    /// **The seal moves with the output device, on a real one.**
    ///
    /// The one thing no unit test can do: put two real CoreAudio devices in the
    /// path and read `derive`'s verdict on either side of a switch. `dev_rate` is
    /// one of the three rates `signal_path::derive` reads, and this crate's whole
    /// claim is that the label follows it — so a picker that moved the engine's
    /// preference without moving the session would leave a seal describing a
    /// device the Deck is no longer using.
    ///
    /// It prints every reading verbatim, including the transient across the
    /// switch, and skips itself with a message when the machine has fewer than two
    /// output devices — a Deck cannot demonstrate a device change on a machine
    /// with one device, and a test that quietly passed in that case would be
    /// claiming otherwise.
    ///
    /// ```text
    /// cargo test -p eko-cli --bin eko-cli -- --ignored --nocapture switching_output_device
    /// ```
    ///
    /// It is **silent**: the engine's gain goes to zero before anything is played,
    /// and the seal is read off `App::volume`, which stays at unity.
    #[test]
    #[ignore = "claims real audio output devices; run it deliberately"]
    fn switching_output_device_moves_the_seal_on_a_real_track() {
        let devices = eko_core::engine::list_devices();
        println!("devices: {devices:?}");
        if devices.len() < 2 {
            println!(
                "SKIPPED: this machine lists {} output device(s); a device change \
                 cannot be constructed with fewer than two.",
                devices.len()
            );
            return;
        }

        let dir = std::env::temp_dir().join("eko-cli-device-switch");
        std::fs::remove_dir_all(&dir).ok();
        let mut a = stocked();
        a.library = wav_album(&dir, &[("Device Tone", 20.0)]);
        a.devices = devices.clone();

        a.hush();
        a.play(Source::Local, 0, 0);
        a.unhush();
        assert!(
            tick_until(&mut a, Duration::from_secs(10), |a| a.stream.is_some()),
            "the tone never opened a device"
        );

        let report = |a: &App, label: &str| {
            let seal = a.seal();
            let info = a.stream.clone();
            println!(
                "{label}\n  device={:?} src_rate={:?} rate={:?} dev_rate={:?}\n  \
                 active={} pure={} seal_label={:?} flags={:?}\n  src={:?} output={:?}",
                info.as_ref().map(|i| i.device.clone()),
                info.as_ref().map(|i| i.src_rate),
                info.as_ref().map(|i| i.rate),
                info.as_ref().map(|i| i.dev_rate),
                seal.active,
                seal.pure,
                seal.seal_label,
                seal.flags,
                seal.src,
                seal.output,
            );
            seal
        };

        let before = report(&a, "1. BEFORE the switch");
        assert!(before.active, "a live tone derived no seal");

        // Pick a device that is *not* the one in use.
        let in_use = a
            .stream
            .as_ref()
            .map(|i| i.device.clone())
            .unwrap_or_default();
        let (row, target) = devices
            .iter()
            .enumerate()
            .map(|(i, name)| (i + 1, name.clone()))
            .find(|(_, name)| *name != in_use)
            .expect("a second device");
        println!("switching from {in_use:?} to {target:?} (row {row})");

        a.device_open = true;
        a.device_cursor = row;
        a.hush();
        a.act(Action::Open);
        a.unhush();

        // **The transient.** The new session exists but has opened nothing, so
        // `dev_rate` is 0 and the gate refuses it. This is the frame at which a
        // seal that carried the old device's verdict would be lying.
        let across = report(&a, "2. ACROSS the switch (before the device opens)");
        assert!(
            a.stream.is_none(),
            "the old device's rates survived the switch"
        );
        assert!(!across.active, "the seal claimed something mid-switch");

        assert!(
            tick_until(&mut a, Duration::from_secs(15), |a| a.stream.is_some()),
            "the new device never opened: playback={:?} status={:?}",
            a.playback,
            a.status
        );
        let after = report(&a, "3. AFTER the switch");
        assert!(after.active, "the new session derived no seal");
        let reported = a.stream.as_ref().unwrap().device.clone();
        println!("engine reports device={reported:?}, we asked for {target:?}");
        println!(
            "VERDICT before={:?} (pure={})  after={:?} (pure={})",
            before.seal_label, before.pure, after.seal_label, after.pure
        );

        a.engine.stop();
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── the help overlay ─────────────────────────────────────────────────

    #[test]
    fn question_mark_toggles_the_help_and_esc_closes_it() {
        let mut a = stocked();
        assert!(!a.help_open);
        a.handle(key(KeyCode::Char('?')));
        assert!(a.help_open);
        a.handle(key(KeyCode::Char('?')));
        assert!(!a.help_open);

        a.handle(key(KeyCode::Char('?')));
        a.act(Action::Back);
        assert!(!a.help_open, "esc did not close the overlay");
    }

    /// `esc` closes the overlay **before** it means anything else, so the way out
    /// is the key everything else in the Deck backs out with — and it does not also
    /// leave the album you were in.
    #[test]
    fn esc_closes_the_help_before_it_backs_out_of_anything() {
        let mut a = stocked();
        a.view = View::Album(0);
        a.help_open = true;
        a.act(Action::Back);
        assert!(!a.help_open);
        assert_eq!(a.view, View::Album(0), "esc also backed out of the album");
    }

    /// **It holds no state of its own.** The overlay renders
    /// [`keys::BINDINGS`], so there is no cursor, no scroll offset and nothing
    /// that could go stale against the table — opening and closing it changes one
    /// bool and nothing else the fold owns.
    #[test]
    fn the_help_overlay_changes_nothing_but_its_own_flag() {
        let mut a = stocked();
        a.view = View::Album(0);
        a.track_cursor = 2;
        let (cursor, selected, focus) = (a.cursor(), a.selected, a.focus);
        a.act(Action::Help);
        a.act(Action::Help);
        assert_eq!((a.cursor(), a.selected, a.focus), (cursor, selected, focus));
        assert_eq!(a.view, View::Album(0));
        assert!(!a.help_open);
    }

    /// One overlay at a time, in every direction — so no renderer and no guard in
    /// [`App::act`] needs a precedence rule for two modals at once.
    #[test]
    fn only_one_overlay_is_ever_open() {
        let mut a = with_devices();
        for (open, check) in [
            (Action::Help, 0usize),
            (Action::EqPanel, 1),
            (Action::Devices, 2),
        ] {
            a.help_open = true;
            a.eq_open = true;
            a.device_open = true;
            // Toggling from "already open" would close it, so start from closed.
            match check {
                0 => a.help_open = false,
                1 => a.eq_open = false,
                _ => a.device_open = false,
            }
            a.act(open);
            let open_count =
                usize::from(a.help_open) + usize::from(a.eq_open) + usize::from(a.device_open);
            assert_eq!(
                open_count, 1,
                "{open:?} left {open_count} overlays open: help={} eq={} devices={}",
                a.help_open, a.eq_open, a.device_open
            );
        }
    }

    /// The transport keeps working behind the overlay — it is a reference card, not
    /// a mode. Only the keys that would move something *hidden* are swallowed.
    #[test]
    fn the_transport_still_works_behind_the_help_overlay() {
        let mut a = stocked();
        a.playback = Playback::Playing;
        a.act(Action::Help);
        a.act(Action::PlayPause);
        assert_eq!(a.playback, Playback::Paused);
        // A second non-list key, so this is about the overlay's filter rather
        // than about `PlayPause` in particular.
        let spectrum = a.spectrum_visible;
        a.act(Action::ToggleSpectrum);
        assert_ne!(a.spectrum_visible, spectrum);
        assert!(a.help_open, "a transport key closed the overlay");
    }

    /// The smallest legal PNG this suite can build without a fixture file.
    fn one_pixel_png() -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            8,
            image::Rgb([200, 40, 40]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
        out
    }
}
