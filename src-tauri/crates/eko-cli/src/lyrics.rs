// This module is complete but not yet called: the Deck does not thread a
// [`Lyrics`] into its render path, so nothing here has a caller outside the
// tests. The layout it is built for already exists and is covered —
// [`crate::ui::visualiser::panels_for`] lays out both arrangements, and the
// sweep there asserts the cover is identical in each.
//
// The attribute goes when the render path picks the module up; keeping it after
// that would hide genuinely unreachable code here.
#![allow(dead_code)]

//! The words, and **only the offsets somebody actually wrote down**.
//!
//! One track's lyrics: fetched off the render thread, keyed on the track, and
//! either *synced* — every line stamped with a millisecond offset the source
//! supplied — or *unsynced*, which is a block of text with no positions in it at
//! all.
//!
//! ## The one rule this module exists to hold
//!
//! **A timing that was not given is not invented.** An unsynced lyric sheet is
//! shown as a scrollable block that does not move with the transport, and there
//! is no karaoke effect on it: not "spread the lines evenly over the duration",
//! not "advance a line every `dur / n` milliseconds", not "guess from the line
//! lengths". Every one of those produces a highlight that is right twice a track
//! and confidently wrong the rest of the time.
//!
//! It is the same refusal [`crate::ui::visualiser`] makes about the analyser:
//! `eko_core::engine` reports thirty-two bands, so thirty-two bars are drawn and
//! never a hundred and twenty-eight interpolated ones. Prettier and a lie is
//! still a lie. What this module can do honestly is *read* timestamps that are
//! genuinely in the data — the `[mm:ss.xx]` in an LRC file is a number a human
//! typed, and parsing it is not synthesising it.
//!
//! ## Where lyrics come from, and what symphonia will and will not give us
//!
//! * **A server track** — `eko_net::Client::get_lyrics_by_song_id`, whose
//!   [`LyricsResult`] carries `synced` (per-line ms offsets) or `unsynced`
//!   (plain text) or neither. It never fails; it degrades to "no lyrics".
//! * **A local file, sidecar** — a `.lrc` beside the audio file, sharing its
//!   stem. This is where synced lyrics for a local library actually live.
//! * **A local file, embedded tag** — symphonia **does** expose one, as
//!   [`StandardTagKey::Lyrics`]: it is populated from Vorbis `LYRICS` /
//!   `UNSYNCEDLYRICS`, from ID3v2 `USLT`, and from MP4 `©lyr`. What it does
//!   **not** expose is ID3v2 `SYLT`, the *synchronised* lyric frame — the reader
//!   for it is commented out in `symphonia-metadata`'s frame table, so a `SYLT`
//!   frame is invisible to this program no matter what is in it.
//!
//!   So "embedded lyrics" here means an unsynchronised text tag. It is run
//!   through the same [`parse`] as a sidecar, which means a tag whose text
//!   happens to be LRC-formatted — which is how foobar2000, Picard and Navidrome
//!   all write synced lyrics into a FLAC — comes out synced, because the
//!   timestamps are really there. A tag that is plain prose comes out unsynced.
//!   Nothing is stamped that was not stamped already.
//!
//! ## Every line is server text, and goes through the one gate
//!
//! A lyric line arrives from a Navidrome instance or from a file on disk, and it
//! is written into a terminal. [`crate::server::tidy`] is the gate — the same one
//! [`crate::remote`]'s constructors use — and it is applied **per line, after the
//! text has been split on newlines**, which is the whole subtlety:
//!
//! * `tidy` collapses control characters (and the bidi overrides) to spaces,
//!   squeezes runs of spaces, and caps the length. Handing it a whole multi-line
//!   blob would fold the entire lyric sheet onto one row, because a newline is
//!   exactly one of the things it neutralises.
//! * The **whole** of `tidy` is right for a lyric line, and its whitespace half
//!   is *not* a hazard here. That half was split out as
//!   [`crate::server::is_unsafe_in_a_row`] because [`crate::line::type_into`]
//!   feeds it one keystroke at a time and `tidy(" ")` is the empty string, which
//!   is why the search box could not take a two-word query. A lyric line is not
//!   typed; it arrives whole, `"walking in the rain"` survives `tidy` unchanged,
//!   and the squeeze is exactly what stops a server padding a line into a
//!   pseudo-indent. So: **`tidy` per line, not `is_unsafe_in_a_row` per char.**
//!
//! ## The shape of this module
//!
//! [`crate::art`] and [`crate::wave`] again, deliberately, down to the
//! vocabulary: a [`Key`] that says what is wanted, a [`Pane`] that holds what
//! arrived and the generation counter that drops stale answers, a blocking
//! [`load`] a test can drive with no thread in the way, and a [`spawn`] that is
//! four lines around it. The generation-drop is [`crate::remote::spawn_albums`]'s
//! rule and exists for its reason: a fetch for track A landing after track B has
//! started must not put A's words under B's title.
//!
//! [`LyricsResult`]: eko_net::types::LyricsResult
//! [`StandardTagKey::Lyrics`]: symphonia::core::meta::StandardTagKey::Lyrics

use std::sync::Arc;

use eko_net::types::LyricsResult;
use eko_net::Client;

use crate::server;

/// The longest lyric sheet this will hold.
///
/// A lyric sheet is a few dozen lines. This is two orders of magnitude past any
/// real one and exists so a server answering with a megabyte of text cannot make
/// the fold allocate it — the same spirit as [`crate::server::tidy`]'s own
/// length cap, applied to the count rather than to one line.
pub const MAX_LINES: usize = 2_000;

/// One line, and the offset it was given — **or was not**.
///
/// `start` is milliseconds from the start of the track. It is `None` for every
/// line of an unsynced sheet and `Some` for every line of a synced one; there is
/// no third case, because a sheet that is half-timed is resolved at parse time
/// (see [`parse`]) rather than left for the renderer to guess at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub start: Option<u64>,
    pub text: String,
}

/// One track's words.
///
/// [`Lyrics::synced`] is the whole of the difference: synced lines follow the
/// transport, unsynced ones are scrolled by hand and nothing pretends otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lyrics {
    lines: Vec<Line>,
    synced: bool,
}

impl Lyrics {
    /// Whether every line carries an offset the source supplied.
    #[must_use]
    pub fn synced(&self) -> bool {
        self.synced
    }

    #[must_use]
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The index of the line that is sounding at `pos_ms`, if one is.
    ///
    /// The **last** line whose offset has passed — so a line stays lit until the
    /// next one is due, which is what a lyric sheet means by "the current line".
    ///
    /// `None` before the first offset (an intro is not the first line arriving
    /// early) and `None` for an unsynced sheet at every position, because an
    /// unsynced sheet has no current line and saying it does is the lie this
    /// module is built around not telling.
    #[must_use]
    pub fn current(&self, pos_ms: u64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let mut found = None;
        for (i, line) in self.lines.iter().enumerate() {
            match line.start {
                Some(start) if start <= pos_ms => found = Some(i),
                _ => break,
            }
        }
        found
    }

    /// Build a synced sheet from lines that already carry offsets.
    ///
    /// Sorted by offset — the wire order is the server's and a stable sort here
    /// costs nothing, where a sheet delivered out of order would make
    /// [`Lyrics::current`]'s scan stop at the first line in the future and light
    /// the wrong row for the rest of the track.
    fn synced_from(mut lines: Vec<(u64, String)>) -> Option<Self> {
        lines.sort_by_key(|(start, _)| *start);
        let lines: Vec<Line> = lines
            .into_iter()
            .take(MAX_LINES)
            .map(|(start, text)| Line {
                start: Some(start),
                text,
            })
            .collect();
        (!lines.is_empty()).then_some(Self {
            lines,
            synced: true,
        })
    }

    /// Build an unsynced sheet: text, in the order it was written, with no
    /// offsets attached to any of it.
    fn unsynced_from(lines: Vec<String>) -> Option<Self> {
        // Blank lines inside the sheet are kept — a verse break is part of the
        // text — but a sheet that is *only* blanks is not a sheet.
        if lines.iter().all(|l| l.is_empty()) {
            return None;
        }
        Some(Self {
            lines: lines
                .into_iter()
                .take(MAX_LINES)
                .map(|text| Line { start: None, text })
                .collect(),
            synced: false,
        })
    }
}

/// What one lyric request answered.
///
/// [`Outcome::Unavailable`] is "asked, and there are none" — a real answer, and
/// the one the view shows the old layout for. There is no error variant: a
/// server that is down, a file with no tag and a track that simply has no words
/// are the same thing to a reader, and `eko_net`'s own lyrics endpoints already
/// collapse every failure into "no lyrics" before this sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ready(Lyrics),
    Unavailable,
}

/// What identifies one track's lyrics.
///
/// The **same token the cover and the envelope are keyed on** — `local:{path}`
/// or `remote:{id}` — so all three caches agree about what "the current track"
/// is by construction rather than by three pieces of arithmetic that have to be
/// kept in step. There is no size in it: words are not re-fetched by a resize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub token: String,
}

/// One worker's answer, stamped with the request it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub generation: u64,
    pub key: Key,
    pub outcome: Outcome,
}

/// Where one track's words are to be read from.
///
/// # It is not `Debug`-derivable, and that is not tidiness
///
/// [`Fetch::Remote`] holds an `Arc<Client>`, and `eko_net::Client` holds the
/// password. A derived `Debug` would put it into any `{fetch:?}` — a panic
/// message, a failing `assert_eq!`, a log line somebody adds next year. Exactly
/// [`crate::server::Connection`]'s reasoning, and [`crate::art::Fetch`]'s.
///
/// This type never reaches [`crate::app::App`]: it is built inside
/// `App::sync_lyrics`, moved onto the worker, and dropped there. What comes back
/// is an [`Event`], which carries text and a number.
pub enum Fetch {
    /// A file on disk. The audio file's own path — the sidecar is derived from
    /// it, not stored beside it.
    Local(String),
    /// A track on the server, by id.
    Remote {
        client: Arc<Client>,
        song_id: String,
    },
}

impl std::fmt::Debug for Fetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(path) => f.debug_tuple("Local").field(path).finish(),
            // The id is safe to print; the client is not printable at all.
            Self::Remote { song_id, .. } => f
                .debug_struct("Remote")
                .field("song_id", song_id)
                .field("client", &"<connected>")
                .finish(),
        }
    }
}

// ── the LRC parser ───────────────────────────────────────────────────────────

/// The file extension a sidecar has.
pub const SIDECAR_EXT: &str = "lrc";

/// One `[mm:ss.xx]` bracket, in milliseconds — or `None` if it is not one.
///
/// Accepts what LRC files in the wild actually contain: minutes of one or two
/// digits, seconds of two, and a fractional part of one to three digits that is
/// scaled to milliseconds by its own length (`.5` is 500 ms, `.34` is 340 ms,
/// `.345` is 345 ms). Both `.` and `:` are taken as the fractional separator,
/// because both are written.
///
/// An `[ar:Artist]` or `[ti:Title]` header fails every one of those and comes
/// back `None`, which is how [`parse`] tells a tag line from a timed one without
/// a second list of known tag names.
#[must_use]
fn timestamp(tag: &str) -> Option<u64> {
    let (mins, rest) = tag.split_once(':')?;
    let mins: u64 = mins.trim().parse().ok()?;
    let (secs, frac) = match rest.split_once(['.', ':']) {
        Some((secs, frac)) => (secs, Some(frac)),
        None => (rest, None),
    };
    if secs.len() != 2 {
        return None;
    }
    let secs: u64 = secs.parse().ok()?;
    if secs >= 60 {
        return None;
    }
    let millis = match frac {
        None => 0,
        Some(frac) => {
            if frac.is_empty() || frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let value: u64 = frac.parse().ok()?;
            value * 10u64.pow(3 - u32::try_from(frac.len()).ok()?)
        }
    };
    Some((mins * 60 + secs) * 1000 + millis)
}

/// Split one raw line into its leading `[…]` groups and whatever follows them.
fn brackets(line: &str) -> (Vec<&str>, &str) {
    let mut tags = Vec::new();
    let mut rest = line.trim_start();
    while let Some(body) = rest.strip_prefix('[') {
        let Some(end) = body.find(']') else { break };
        tags.push(&body[..end]);
        rest = &body[end + 1..];
    }
    (tags, rest)
}

/// Read a lyric sheet out of text — **LRC if it is LRC, plain text if it is not.**
///
/// One entry point for a sidecar, for an embedded tag and for the `unsynced`
/// half of a server's answer, because all three are the same thing: a blob that
/// may or may not have timestamps in it. Which it is decides the answer:
///
/// * **Any line carries a timestamp** → the sheet is synced, and it is made of
///   exactly those lines. A line with no timestamp in a timed sheet is dropped
///   rather than placed: it is almost always an `[ar:…]` header or a blank, and
///   the alternative — putting it at the offset of its neighbour — is inventing
///   a timing, which is the one thing this module will not do.
/// * **No line does** → the sheet is unsynced and is the text as written, blank
///   lines and all.
///
/// A line may carry several timestamps (`[00:12.00][01:04.00]a refrain`), which
/// is the format's way of writing a repeated line; it becomes one entry per
/// offset. Every line is put through [`crate::server::tidy`] — see the module
/// docs for why per line and why the whole of it.
///
/// `None` when there is nothing readable left.
#[must_use]
pub fn parse(text: &str) -> Option<Lyrics> {
    let mut timed: Vec<(u64, String)> = Vec::new();
    let mut plain: Vec<String> = Vec::new();

    for raw in text.lines() {
        let (tags, rest) = brackets(raw);
        let stamps: Vec<u64> = tags.iter().filter_map(|t| timestamp(t)).collect();
        if stamps.is_empty() {
            // Not a timed line. A line that was *entirely* `[…]` groups is a
            // header (`[ar:…]`, `[length:…]`) and is dropped; anything else is
            // text, kept verbatim — including its brackets, which is why `raw`
            // rather than `rest` is tidied here.
            if !tags.is_empty() && rest.trim().is_empty() {
                continue;
            }
            plain.push(server::tidy(raw));
            continue;
        }
        let text = server::tidy(rest);
        for start in stamps {
            timed.push((start, text.clone()));
        }
    }

    if !timed.is_empty() {
        return Lyrics::synced_from(timed);
    }
    Lyrics::unsynced_from(plain)
}

/// A server's [`LyricsResult`], narrowed to a sheet. **The remote boundary.**
///
/// The `synced` half is taken as it comes — those offsets are the server's own
/// measurements and there is nothing to parse. The `unsynced` half goes through
/// [`parse`], because Navidrome will hand back LRC-formatted text under
/// `unsynced` when it read it out of a tag, and timestamps that are really in
/// the string are timestamps.
#[must_use]
pub fn from_result(result: &LyricsResult) -> Outcome {
    if let Some(lines) = &result.synced {
        let timed: Vec<(u64, String)> = lines
            .iter()
            .map(|l| (u64::from(l.start), server::tidy(&l.value)))
            .collect();
        if let Some(lyrics) = Lyrics::synced_from(timed) {
            return Outcome::Ready(lyrics);
        }
    }
    if let Some(text) = &result.unsynced {
        if let Some(lyrics) = parse(text) {
            return Outcome::Ready(lyrics);
        }
    }
    Outcome::Unavailable
}

// ── reading a local file ─────────────────────────────────────────────────────

/// The `.lrc` beside `path`: the same directory, the same stem.
#[must_use]
pub fn sidecar_path(path: &str) -> std::path::PathBuf {
    std::path::Path::new(path).with_extension(SIDECAR_EXT)
}

/// The embedded lyrics tag, if symphonia can see one.
///
/// [`StandardTagKey::Lyrics`] and nothing else — see the module docs for what
/// that covers (Vorbis `LYRICS`/`UNSYNCEDLYRICS`, ID3v2 `USLT`, MP4 `©lyr`) and
/// what it does not (ID3v2 `SYLT`, whose reader symphonia has commented out).
///
/// Both metadata sources are consulted: tags found *before* the format was
/// identified — an ID3v2 block in front of a FLAC stream — live on
/// `ProbedMetadata`, and the container's own live on the reader. A file can have
/// either or both, and reading only one is how a tag goes missing on exactly one
/// of the two formats somebody happens to own.
///
/// [`StandardTagKey::Lyrics`]: symphonia::core::meta::StandardTagKey::Lyrics
#[must_use]
fn embedded(path: &str) -> Option<String> {
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
    {
        hint.with_extension(ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;

    let pick = |rev: &MetadataRevision| -> Option<String> {
        rev.tags()
            .iter()
            .find(|t| t.std_key == Some(StandardTagKey::Lyrics))
            .map(|t| t.value.to_string())
            .filter(|v| !v.trim().is_empty())
    };

    if let Some(found) = probed
        .metadata
        .get()
        .as_ref()
        .and_then(|m| m.current())
        .and_then(pick)
    {
        return Some(found);
    }
    probed.format.metadata().current().and_then(pick)
}

/// One track's words. **Blocking**; [`spawn`] is the way in.
///
/// For a local file the sidecar wins over the tag, and deliberately: a `.lrc` a
/// person put next to the file is a correction, and a tag they cannot edit
/// without a tag editor is not.
#[must_use]
pub fn load(fetch: &Fetch) -> Outcome {
    match fetch {
        Fetch::Local(path) => {
            if let Some(lyrics) = std::fs::read_to_string(sidecar_path(path))
                .ok()
                .as_deref()
                .and_then(parse)
            {
                return Outcome::Ready(lyrics);
            }
            match embedded(path).as_deref().and_then(parse) {
                Some(lyrics) => Outcome::Ready(lyrics),
                None => Outcome::Unavailable,
            }
        }
        // `get_lyrics_by_song_id` never surfaces an error — it falls back to the
        // legacy endpoint and then to an empty result — so there is nothing to
        // describe here and no failure to render. See `eko_net::Client`.
        Fetch::Remote { client, song_id } => match client.get_lyrics_by_song_id(song_id) {
            Ok(result) => from_result(&result),
            Err(_) => Outcome::Unavailable,
        },
    }
}

/// Fetch one track's words on a worker thread.
///
/// The generation stamp travels out and back exactly as [`crate::art::spawn`]'s
/// does; `emit` returning `false` means the fold has hung up.
pub fn spawn<F>(fetch: Fetch, key: Key, generation: u64, emit: F)
where
    F: Fn(Event) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        let outcome = load(&fetch);
        emit(Event {
            generation,
            key,
            outcome,
        });
    });
}

// ── the pane ─────────────────────────────────────────────────────────────────

/// Where a lyric request has got to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum State {
    #[default]
    Idle,
    /// A worker is fetching. The view keeps the no-lyrics layout meanwhile —
    /// there is no spinner and no reserved column, because a column held open
    /// for something that may never arrive is the dead column this whole layout
    /// change exists to remove.
    Loading,
    Ready(Lyrics),
    /// Asked, and this track has no words.
    Unavailable,
}

/// What is wanted, what has arrived, the counter that drops stale answers, and
/// how far an unsynced sheet has been scrolled.
///
/// [`crate::wave::Pane`] with a scroll offset added. Its own type rather than
/// generic over the payload, for [`crate::wave::Pane`]'s stated reason: two
/// caches that look alike but drop staleness by *different* rules is the bug.
#[derive(Debug, Default)]
pub struct Pane {
    want: Option<Key>,
    state: State,
    generation: u64,
    /// First visible line of an **unsynced** sheet. Always `0` for a synced one,
    /// which is scrolled by the transport rather than by a key.
    scroll: usize,
}

impl Pane {
    /// The key currently being asked for, if any.
    #[must_use]
    pub fn wants(&self) -> Option<&Key> {
        self.want.as_ref()
    }

    /// The generation an answer must carry to be believed. `#[cfg(test)]` for
    /// [`crate::wave::Pane::generation_for_test`]'s reason.
    #[cfg(test)]
    #[must_use]
    pub fn generation_for_test(&self) -> u64 {
        self.generation
    }

    /// The sheet, if one has arrived for what is currently wanted.
    #[must_use]
    pub fn lyrics(&self) -> Option<&Lyrics> {
        match &self.state {
            State::Ready(lyrics) => Some(lyrics),
            _ => None,
        }
    }

    /// **Whether the view switches layout.** True only once real words are in
    /// hand — not while a fetch is in flight, and not for a track with none.
    #[must_use]
    pub fn showing(&self) -> bool {
        matches!(self.state, State::Ready(_))
    }

    /// Whether `j` / `k` have something to move.
    ///
    /// An unsynced sheet only. A synced one follows the transport, and letting a
    /// key push it out of step with the audio would make the highlight wrong in
    /// a way the reader caused and cannot see.
    #[must_use]
    pub fn scrollable(&self) -> bool {
        matches!(&self.state, State::Ready(lyrics) if !lyrics.synced())
    }

    /// First visible line of an unsynced sheet.
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Move an unsynced sheet by `delta` lines, stopping at both ends.
    ///
    /// `visible` is the rect's height, so the last screenful is the floor rather
    /// than the last line: scrolling into empty space below the words is a state
    /// with nothing on screen to get out of.
    pub fn scroll_by(&mut self, delta: isize, visible: usize) {
        let Some(lyrics) = self.lyrics() else { return };
        if lyrics.synced() {
            return;
        }
        let last = lyrics.len().saturating_sub(visible.max(1));
        let next = self.scroll as isize + delta;
        self.scroll = next.clamp(0, last as isize) as usize;
    }

    /// Ask for `want`, abandoning whatever was in flight.
    pub fn begin(&mut self, want: Option<Key>) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.state = if want.is_some() {
            State::Loading
        } else {
            State::Idle
        };
        self.want = want;
        // A new track starts at the top of its own sheet, never at the row the
        // last one had been scrolled to.
        self.scroll = 0;
        self.generation
    }

    /// Give up without a request having been made — no live client to ask
    /// through. Stated as "no words" rather than left in `Loading`, which would
    /// be a promise of something that is never coming.
    pub fn give_up(&mut self) {
        if self.want.is_some() {
            self.state = State::Unavailable;
        }
    }

    /// Fold one worker's answer in. `false` when it was stale and dropped.
    pub fn accept(&mut self, event: Event) -> bool {
        if event.generation != self.generation || self.want.as_ref() != Some(&event.key) {
            return false;
        }
        self.state = match event.outcome {
            Outcome::Ready(lyrics) => State::Ready(lyrics),
            Outcome::Unavailable => State::Unavailable,
        };
        self.scroll = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eko_net::types::SyncedLyricLine;

    fn texts(lyrics: &Lyrics) -> Vec<&str> {
        lyrics.lines().iter().map(|l| l.text.as_str()).collect()
    }

    fn starts(lyrics: &Lyrics) -> Vec<Option<u64>> {
        lyrics.lines().iter().map(|l| l.start).collect()
    }

    #[test]
    fn a_timestamp_is_read_in_every_form_a_real_lrc_writes_it() {
        assert_eq!(timestamp("00:00.00"), Some(0));
        assert_eq!(timestamp("00:12.34"), Some(12_340));
        assert_eq!(timestamp("01:04.5"), Some(64_500));
        assert_eq!(timestamp("01:04.345"), Some(64_345));
        assert_eq!(timestamp("1:04"), Some(64_000));
        // Some writers use a colon for the fraction.
        assert_eq!(timestamp("00:12:34"), Some(12_340));
        // Long tracks: minutes are not capped at two digits.
        assert_eq!(timestamp("123:45.00"), Some(7_425_000));

        // And the headers, which must not read as times.
        assert_eq!(timestamp("ar:The Innocence Mission"), None);
        assert_eq!(timestamp("ti:When You Were Young"), None);
        assert_eq!(timestamp("offset:+250"), None);
        assert_eq!(timestamp(""), None);
        assert_eq!(timestamp("00:99.00"), None, "sixty-nine seconds");
        assert_eq!(timestamp("00:5.00"), None, "one-digit seconds");
        assert_eq!(timestamp("00:05.0000"), None, "four-digit fraction");
    }

    #[test]
    fn an_lrc_file_parses_to_the_offsets_it_was_written_with() {
        let lrc = "[ar:The Innocence Mission]\n\
                   [ti:Befriended]\n\
                   [00:12.34]the first line\n\
                   [00:15.00]the second line\n\
                   [01:00.00][01:30.00]the refrain\n";
        let lyrics = parse(lrc).expect("no sheet");
        assert!(lyrics.synced());
        assert_eq!(
            texts(&lyrics),
            [
                "the first line",
                "the second line",
                "the refrain",
                "the refrain"
            ]
        );
        assert_eq!(
            starts(&lyrics),
            [Some(12_340), Some(15_000), Some(60_000), Some(90_000)]
        );
        // The headers are gone: they are not lines of the song.
        assert!(!texts(&lyrics).iter().any(|t| t.contains("Innocence")));
    }

    /// **The whole point of the module, as an assertion.**
    #[test]
    fn plain_text_stays_plain_and_is_never_given_a_timing() {
        let lyrics = parse("the first line\n\nthe second line\n").expect("no sheet");
        assert!(!lyrics.synced());
        assert_eq!(texts(&lyrics), ["the first line", "", "the second line"]);
        assert!(
            starts(&lyrics).iter().all(Option::is_none),
            "a timing was invented"
        );
        // And no position on the transport lights a line.
        for pos in [0, 1, 1_000, 60_000, u64::MAX] {
            assert_eq!(lyrics.current(pos), None, "at {pos}ms");
        }
    }

    /// A sheet with *some* timings does not get the rest filled in — the untimed
    /// lines are dropped, not placed.
    #[test]
    fn a_half_timed_sheet_keeps_only_what_was_timed() {
        let lyrics = parse("[00:10.00]timed\nuntimed\n[00:20.00]timed again\n").expect("no sheet");
        assert!(lyrics.synced());
        assert_eq!(texts(&lyrics), ["timed", "timed again"]);
        assert_eq!(starts(&lyrics), [Some(10_000), Some(20_000)]);
    }

    #[test]
    fn the_current_line_is_the_last_one_whose_offset_has_passed() {
        let lyrics = parse("[00:10.00]one\n[00:20.00]two\n[00:30.00]three\n").expect("no sheet");
        assert_eq!(lyrics.current(0), None, "an intro is not the first line");
        assert_eq!(lyrics.current(9_999), None);
        assert_eq!(lyrics.current(10_000), Some(0));
        assert_eq!(
            lyrics.current(19_999),
            Some(0),
            "a line holds until the next"
        );
        assert_eq!(lyrics.current(20_000), Some(1));
        assert_eq!(lyrics.current(u64::MAX), Some(2));
    }

    /// An out-of-order sheet is sorted, so the scan cannot stop early and light
    /// the wrong row for the rest of the track.
    #[test]
    fn a_sheet_delivered_out_of_order_is_put_in_order() {
        let lyrics = parse("[00:30.00]three\n[00:10.00]one\n[00:20.00]two\n").expect("no sheet");
        assert_eq!(texts(&lyrics), ["one", "two", "three"]);
        assert_eq!(lyrics.current(25_000), Some(1));
    }

    #[test]
    fn nothing_readable_is_no_sheet_at_all() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("\n\n\n"), None);
        assert_eq!(parse("[ar:only a header]\n"), None);
        // A bracket that never closes is text, not a header.
        assert!(parse("[unclosed\n").is_some());
    }

    /// **Every line goes through the one gate, per line.**
    ///
    /// The blob is split on newlines *first* — handing the whole thing to
    /// `tidy` would fold the sheet onto one row, because a newline is one of
    /// the things `tidy` neutralises.
    #[test]
    fn a_hostile_lyric_cannot_carry_an_escape_or_a_newline_into_a_line() {
        let hostile = "[00:01.00]clear:\u{1b}[2J\n\
                       [00:02.00]retitle:\u{1b}]0;pwned\u{7}\n\
                       [00:03.00]reversed:\u{202e}drawkcab\n";
        let lyrics = parse(hostile).expect("no sheet");
        assert_eq!(lyrics.len(), 3);
        for line in lyrics.lines() {
            assert!(
                !line.text.chars().any(|c| c.is_control()),
                "a control character survived: {:?}",
                line.text
            );
            assert!(
                !line.text.contains('\u{202e}'),
                "a bidi override survived: {:?}",
                line.text
            );
        }
        // And the sheet is still three rows, not one: the split happened before
        // the gate did.
        assert!(lyrics.lines().iter().all(|l| !l.text.contains('\n')));
    }

    /// The other half of the gate question: `tidy`, not `is_unsafe_in_a_row`.
    /// A multi-word line keeps its spaces — which is the fault that split the
    /// two apart in the first place (see `crate::line::type_into`).
    #[test]
    fn a_multi_word_lyric_keeps_its_spaces() {
        let lyrics = parse("[00:01.00]walking in the rain again\n").expect("no sheet");
        assert_eq!(texts(&lyrics), ["walking in the rain again"]);
        // And a run of padding is squeezed, because a server does not get to
        // indent a row.
        let padded = parse("[00:01.00]a        b\n").expect("no sheet");
        assert_eq!(texts(&padded), ["a b"]);
    }

    #[test]
    fn a_servers_synced_answer_is_taken_as_given() {
        let result = LyricsResult {
            synced: Some(vec![
                SyncedLyricLine {
                    start: 20_000,
                    value: "two".into(),
                },
                SyncedLyricLine {
                    start: 10_000,
                    value: "one".into(),
                },
            ]),
            unsynced: Some("ignored".into()),
        };
        let Outcome::Ready(lyrics) = from_result(&result) else {
            panic!("no sheet");
        };
        assert!(lyrics.synced());
        assert_eq!(texts(&lyrics), ["one", "two"]);
    }

    /// A server that answers with LRC text under `unsynced` — which Navidrome
    /// does when it read the sheet out of a tag — is read as synced, because the
    /// timestamps are genuinely in the string.
    #[test]
    fn a_servers_unsynced_answer_is_parsed_rather_than_assumed_flat() {
        let lrc = LyricsResult {
            synced: None,
            unsynced: Some("[00:10.00]one\n[00:20.00]two".into()),
        };
        let Outcome::Ready(lyrics) = from_result(&lrc) else {
            panic!("no sheet");
        };
        assert!(lyrics.synced());
        assert_eq!(starts(&lyrics), [Some(10_000), Some(20_000)]);

        let prose = LyricsResult {
            synced: None,
            unsynced: Some("one\ntwo".into()),
        };
        let Outcome::Ready(lyrics) = from_result(&prose) else {
            panic!("no sheet");
        };
        assert!(!lyrics.synced());

        assert_eq!(from_result(&LyricsResult::EMPTY), Outcome::Unavailable);
        // An empty synced list is not a sheet, and does not shadow the text.
        assert_eq!(
            from_result(&LyricsResult {
                synced: Some(Vec::new()),
                unsynced: Some("words".into()),
            }),
            Outcome::Ready(Lyrics::unsynced_from(vec!["words".into()]).unwrap())
        );
    }

    #[test]
    fn the_sidecar_is_the_audio_files_own_stem() {
        assert_eq!(
            sidecar_path("/music/Befriended/03 Tomorrow On The Runway.flac"),
            std::path::Path::new("/music/Befriended/03 Tomorrow On The Runway.lrc")
        );
        assert_eq!(
            sidecar_path("/music/no-extension"),
            std::path::Path::new("/music/no-extension.lrc")
        );
    }

    /// A real file on disk, with a real sidecar beside it.
    #[test]
    fn a_sidecar_beside_a_file_is_found_and_a_missing_one_is_not() {
        let dir = std::env::temp_dir().join("eko-cli-lyrics-sidecar");
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("track.flac");
        std::fs::write(&audio, b"not really a flac").unwrap();
        let path = audio.to_string_lossy().to_string();

        // No sidecar, and nothing symphonia can read: no words.
        let _ = std::fs::remove_file(dir.join("track.lrc"));
        assert_eq!(load(&Fetch::Local(path.clone())), Outcome::Unavailable);

        std::fs::write(dir.join("track.lrc"), "[00:05.00]a line\n").unwrap();
        let Outcome::Ready(lyrics) = load(&Fetch::Local(path)) else {
            panic!("the sidecar was not read");
        };
        assert!(lyrics.synced());
        assert_eq!(texts(&lyrics), ["a line"]);

        let _ = std::fs::remove_file(dir.join("track.lrc"));
        let _ = std::fs::remove_file(&audio);
    }

    /// The generation-drop, which is what stops track A's words landing under
    /// track B's title.
    #[test]
    fn a_stale_answer_is_dropped_and_a_current_one_is_not() {
        let mut pane = Pane::default();
        let a = Key {
            token: "local:a".into(),
        };
        let b = Key {
            token: "local:b".into(),
        };
        let gen_a = pane.begin(Some(a.clone()));
        let gen_b = pane.begin(Some(b.clone()));
        assert_ne!(gen_a, gen_b);
        assert!(!pane.showing(), "loading is not showing");

        // A's answer, arriving after B was asked for.
        assert!(!pane.accept(Event {
            generation: gen_a,
            key: a,
            outcome: Outcome::Ready(parse("[00:01.00]stale").unwrap()),
        }));
        assert!(!pane.showing());

        // The right generation but the wrong key is also dropped.
        assert!(!pane.accept(Event {
            generation: gen_b,
            key: Key {
                token: "local:c".into()
            },
            outcome: Outcome::Ready(parse("[00:01.00]wrong").unwrap()),
        }));

        assert!(pane.accept(Event {
            generation: gen_b,
            key: b,
            outcome: Outcome::Ready(parse("[00:01.00]fresh").unwrap()),
        }));
        assert!(pane.showing());
        assert_eq!(pane.lyrics().unwrap().lines()[0].text, "fresh");
    }

    #[test]
    fn only_an_unsynced_sheet_can_be_scrolled_and_it_stops_at_both_ends() {
        let mut pane = Pane::default();
        let key = Key {
            token: "local:a".into(),
        };
        let generation = pane.begin(Some(key.clone()));
        let text = (0..10).map(|i| format!("line {i}")).collect::<Vec<_>>();
        assert!(pane.accept(Event {
            generation,
            key: key.clone(),
            outcome: Outcome::Ready(parse(&text.join("\n")).unwrap()),
        }));
        assert!(pane.scrollable());

        pane.scroll_by(-1, 4);
        assert_eq!(pane.scroll(), 0, "scrolled above the first line");
        pane.scroll_by(3, 4);
        assert_eq!(pane.scroll(), 3);
        pane.scroll_by(100, 4);
        assert_eq!(pane.scroll(), 6, "scrolled past the last screenful");

        // A synced sheet refuses: it belongs to the transport.
        let generation = pane.begin(Some(key.clone()));
        assert_eq!(pane.scroll(), 0, "the new track kept the old scroll");
        assert!(pane.accept(Event {
            generation,
            key,
            outcome: Outcome::Ready(parse("[00:01.00]a\n[00:02.00]b\n[00:03.00]c").unwrap()),
        }));
        assert!(!pane.scrollable());
        pane.scroll_by(5, 2);
        assert_eq!(pane.scroll(), 0);
    }

    #[test]
    fn giving_up_says_no_words_rather_than_waiting_forever() {
        let mut pane = Pane::default();
        assert!(!pane.showing());
        pane.give_up();
        assert!(!pane.showing(), "a pane that never asked answered anyway");
        pane.begin(Some(Key {
            token: "remote:1".into(),
        }));
        pane.give_up();
        assert!(!pane.showing());
        assert!(pane.lyrics().is_none());
    }

    /// A sheet longer than any real one is truncated rather than held whole.
    #[test]
    fn an_absurd_sheet_is_capped() {
        let huge = (0..MAX_LINES + 500)
            .map(|i| format!("[00:{:02}.00]line {i}", i % 60))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse(&huge).unwrap().len(), MAX_LINES);
        let prose = "x\n".repeat(MAX_LINES + 500);
        assert_eq!(parse(&prose).unwrap().len(), MAX_LINES);
    }

    /// `Fetch`'s `Debug` cannot print a client, and therefore cannot print a
    /// password. The type-level half of the containment the app relies on.
    #[test]
    fn a_fetch_never_prints_its_client() {
        let config = eko_net::Config {
            base_url: "https://music.example.com".into(),
            username: "rod".into(),
            password: "hunter2".into(),
        };
        let fetch = Fetch::Remote {
            client: Arc::new(Client::new(config).expect("the mock config is a valid one")),
            song_id: "abc".into(),
        };
        let printed = format!("{fetch:?}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(printed.contains("abc"));
        assert!(printed.contains("<connected>"));
    }
}
