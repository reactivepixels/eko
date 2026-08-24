//! The play queue — **one ordered list, both libraries**.
//!
//! The queue is what makes playback continue: [`crate::app::App::poll_engine`]
//! used to see a finished track and honestly go to `Stopped`, because there was
//! nothing to say what came next. This is that "next".
//!
//! ## Local and remote entries sit in the same list
//!
//! Modelled that way from the first line rather than retrofitted, because the
//! retrofit is exactly the bug Task 3 spent its review closing: a queue that held
//! only `(album, track)` pairs would resolve them against whichever library the
//! reader happened to reach for, and a remote entry would start an unrelated
//! local file. So an [`Entry`] does not *refer* to a track — it **carries** the
//! one thing needed to play it, in [`Media`], and its [`Entry::source`] is read
//! off that rather than stored beside it. The two cannot disagree.
//!
//! The album/track indices are still here, but only for the things that are
//! genuinely about position in a library: the `▶` marker in the two list panes,
//! and [`crate::app::NowPlaying`]. Nothing resolves audio through them.
//!
//! ## A remote entry holds an id, never a URL
//!
//! [`Media::Remote`] is a Subsonic track id. The URL is minted at play time by
//! `App::stream_url` and dropped there, because a signed Subsonic URL carries
//! `u`, `t` and `s` — a replayable credential for as long as the password
//! stands. A queue of a few hundred tracks that each held one would keep a few
//! hundred credentials in memory for the life of the session, any of which could
//! reach a log or a `{:?}`. `remote::Track` deliberately holds no URL for the
//! same reason; this list inherits the rule rather than quietly breaking it.

use crate::app::Source;

/// Where an entry's audio comes from — and everything needed to fetch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Media {
    /// A file the scan found. Absolute, and openable as it stands.
    Local(String),
    /// A track id on the server. **Not** a URL — see the module docs.
    Remote(String),
}

impl Media {
    /// Which library this came out of.
    ///
    /// Derived, never stored: a second copy of this beside the media is a second
    /// thing to get wrong, and getting it wrong plays the wrong track.
    #[must_use]
    pub fn source(&self) -> Source {
        match self {
            Self::Local(_) => Source::Local,
            Self::Remote(_) => Source::Remote,
        }
    }
}

/// The row index of an entry that came from **no browsable list**.
///
/// [`Entry::album`] and [`Entry::track`] are positions in one of the two library
/// panes, and a search result is in neither: it is a track the server matched,
/// which may or may not also sit somewhere in the walked album list, at an index
/// nothing here knows. Writing a *plausible* index would be worse than writing
/// none — the `▶` marker compares indices, so a made-up `0` would light row 0 of
/// the remote album list while something else entirely played.
///
/// So search entries carry this, and the marker comparisons simply never match:
/// a list of `usize::MAX` rows cannot exist. The search pane marks its own rows
/// by **track id** instead, which is exact rather than positional — see
/// `crate::ui::main_pane`.
pub const NO_ROW: usize = usize::MAX;

/// One queued track.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// How to play it. The only field the transport reads.
    pub media: Media,
    /// Index of the album it came from, **within [`Entry::source`]'s library**,
    /// or [`NO_ROW`] when it came from no list at all.
    /// For the `▶` marker and [`crate::app::NowPlaying`] only.
    pub album: usize,
    /// Index of the track within that album. Same caveat.
    pub track: usize,
    pub title: String,
    pub artist: String,
    pub album_name: String,
    /// Duration from the server or the file's tags, in ms. The engine overwrites
    /// [`crate::app::NowPlaying::dur_ms`] with the decoded length once it knows
    /// one; this stays as the list's own best guess.
    pub dur_ms: u64,
}

impl Entry {
    /// Which library this entry came out of. See [`Media::source`].
    #[must_use]
    pub fn source(&self) -> Source {
        self.media.source()
    }
}

/// An ordered list of entries and a position in it.
///
/// `current` is *the entry that is playing*, not a cursor — the Queue pane has
/// its own cursor. Nothing moves `current` except starting something.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Queue {
    entries: Vec<Entry>,
    current: Option<usize>,
}

impl Queue {
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Index of the entry that is playing, if any.
    #[must_use]
    pub fn position(&self) -> Option<usize> {
        self.current
    }

    /// The entry that is playing, if any.
    #[must_use]
    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.current?)
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    /// Throw the list away and start again at `current`.
    ///
    /// What `Enter` on a track in a library does: playing from a list means
    /// playing *that list*, so the album becomes the queue. Refused — and the
    /// old queue kept — when `current` is not a real index, so a caller that
    /// resolved nothing cannot empty the queue as a side effect.
    pub fn replace(&mut self, entries: Vec<Entry>, current: usize) -> bool {
        if current >= entries.len() {
            return false;
        }
        self.entries = entries;
        self.current = Some(current);
        true
    }

    /// Point at `index` without disturbing the list. What `Enter` in the Queue
    /// pane does.
    pub fn set_position(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.current = Some(index);
        true
    }

    /// Where `delta` entries away is, **wrapping**. Where `n` and `p` go.
    ///
    /// Wrapping is the right thing for a key someone pressed: `n` on the last
    /// track is a request, and refusing it silently is the sort of dead key this
    /// project keeps finding. [`Queue::peek_next`] — which nobody pressed — does
    /// not wrap.
    ///
    /// A **peek**, not a move, and the caller commits with
    /// [`Queue::set_position`]. That separation is what stops `current` landing
    /// on an entry that then turns out to be unplayable: `current` is the
    /// queue's claim about what is *playing*, and the `▶` marker is drawn from
    /// it, so moving it before a session exists would be a claim made in
    /// advance. See `App::play_queue_at`.
    #[must_use]
    pub fn peek(&self, delta: isize) -> Option<usize> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        let current = self.current?;
        Some((current as isize + delta).rem_euclid(len as isize) as usize)
    }

    /// Where the entry after the current one is, or `None` at the end.
    /// **Does not wrap.**
    ///
    /// This is auto-advance's question. A queue that looped forever at its end
    /// would be a player nobody asked to keep playing; the end of the queue is
    /// the end of playback, and the transport says so.
    #[must_use]
    pub fn peek_next(&self) -> Option<usize> {
        let next = self.current.map_or(0, |c| c + 1);
        (next < self.entries.len()).then_some(next)
    }

    /// Add to the end. Never moves `current`, so queueing something behind a
    /// playing track cannot interrupt it.
    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// Drop the entry at `index`.
    ///
    /// **The playing entry cannot be removed**, and this says so by returning
    /// `false`. `current` is the queue's claim about what is playing; removing it
    /// would either have to move the claim to a track that is *not* playing, or
    /// drop it and leave auto-advance with nowhere to go — both of which are the
    /// list lying about the transport. Press `n` first, then remove it.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.entries.len() || self.current == Some(index) {
            return false;
        }
        self.entries.remove(index);
        if let Some(current) = self.current {
            if index < current {
                self.current = Some(current - 1);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, media: Media) -> Entry {
        Entry {
            media,
            album: 0,
            track: 0,
            title: title.to_string(),
            artist: "Artist".to_string(),
            album_name: "Album".to_string(),
            dur_ms: 120_000,
        }
    }

    fn local(title: &str) -> Entry {
        entry(title, Media::Local(format!("/music/{title}.flac")))
    }

    fn remote(title: &str) -> Entry {
        entry(title, Media::Remote(format!("id-{title}")))
    }

    /// **The whole point of the type.** A local and a remote entry live in one
    /// list, and each still knows which library it came from — from the media it
    /// carries, not from a field beside it that could be set wrong.
    #[test]
    fn local_and_remote_entries_coexist_and_keep_their_own_source() {
        let mut q = Queue::default();
        assert!(q.replace(vec![local("One"), remote("Two"), local("Three")], 0));
        assert_eq!(q.len(), 3);
        assert_eq!(q.get(0).unwrap().source(), Source::Local);
        assert_eq!(q.get(1).unwrap().source(), Source::Remote);
        assert_eq!(q.get(2).unwrap().source(), Source::Local);
    }

    /// A remote entry carries an id. Nothing in the queue is a signed URL.
    #[test]
    fn a_remote_entry_carries_an_id_and_never_a_url() {
        let q = {
            let mut q = Queue::default();
            q.replace(vec![remote("Two")], 0);
            q
        };
        let printed = format!("{q:?}");
        for secret in ["http", "&t=", "&s=", "u="] {
            assert!(!printed.contains(secret), "{secret:?} in {printed}");
        }
        assert!(printed.contains("id-Two"));
    }

    #[test]
    fn an_empty_queue_has_nothing_to_play_and_nothing_to_advance_to() {
        let mut q = Queue::default();
        assert!(q.is_empty());
        assert_eq!(q.position(), None);
        assert!(q.current().is_none());
        assert_eq!(q.peek_next(), None);
        assert_eq!(q.peek(1), None);
        assert!(!q.set_position(0));
    }

    #[test]
    fn replacing_with_an_index_off_the_end_keeps_the_old_queue() {
        let mut q = Queue::default();
        q.replace(vec![local("One")], 0);
        assert!(!q.replace(vec![local("A"), local("B")], 7));
        assert_eq!(q.len(), 1);
        assert_eq!(q.current().unwrap().title, "One");
        // And an empty list can never be "started at 0" either.
        assert!(!q.replace(Vec::new(), 0));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn peek_next_walks_forward_and_stops_at_the_end_rather_than_looping() {
        let mut q = Queue::default();
        q.replace(vec![local("One"), remote("Two")], 0);
        assert_eq!(q.peek_next(), Some(1));
        q.set_position(1);
        assert_eq!(q.current().unwrap().title, "Two");
        assert_eq!(q.peek_next(), None, "the end of the queue looped");
        assert_eq!(q.position(), Some(1), "peeking moved the queue");
    }

    #[test]
    fn peek_wraps_both_ways_because_someone_pressed_a_key() {
        let mut q = Queue::default();
        q.replace(vec![local("One"), remote("Two"), local("Three")], 0);
        assert_eq!(q.peek(-1), Some(2));
        q.set_position(2);
        assert_eq!(q.peek(1), Some(0));
        // And a peek on its own never moves anything.
        assert_eq!(q.position(), Some(2));
    }

    #[test]
    fn pushing_adds_to_the_end_without_disturbing_what_is_playing() {
        let mut q = Queue::default();
        q.replace(vec![local("One"), local("Two")], 1);
        q.push(remote("Three"));
        assert_eq!(q.len(), 3);
        assert_eq!(q.position(), Some(1));
        assert_eq!(q.current().unwrap().title, "Two");
        assert_eq!(q.entries().last().unwrap().title, "Three");
    }

    #[test]
    fn removing_before_the_playing_entry_keeps_it_playing() {
        let mut q = Queue::default();
        q.replace(vec![local("One"), local("Two"), local("Three")], 2);
        assert!(q.remove(0));
        assert_eq!(q.position(), Some(1));
        assert_eq!(q.current().unwrap().title, "Three");
    }

    #[test]
    fn removing_after_the_playing_entry_leaves_the_position_alone() {
        let mut q = Queue::default();
        q.replace(vec![local("One"), local("Two"), local("Three")], 0);
        assert!(q.remove(2));
        assert_eq!(q.position(), Some(0));
        assert_eq!(q.len(), 2);
    }

    /// The queue cannot be made to disagree with the transport.
    #[test]
    fn the_playing_entry_cannot_be_removed() {
        let mut q = Queue::default();
        q.replace(vec![local("One"), local("Two")], 0);
        assert!(!q.remove(0));
        assert_eq!(q.len(), 2);
        assert_eq!(q.current().unwrap().title, "One");
        // And nothing off the end can be removed either.
        assert!(!q.remove(9));
    }
}
