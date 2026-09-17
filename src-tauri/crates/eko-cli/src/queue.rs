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

/// The play queue, shared with the desktop app. See [`eko_core::queue`].
pub type Queue = eko_core::queue::Queue<Entry>;

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
}
