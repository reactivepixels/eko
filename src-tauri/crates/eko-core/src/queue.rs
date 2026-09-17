//! The play queue: an ordered list, and the entry in it that is playing.
//!
//! Shared by `eko-cli` and the desktop app's engine-owned player
//! ([`crate::player`]). It holds no audio and does no I/O.
//!
//! `current` is the entry that is **playing**, not a cursor. Nothing moves it
//! except starting something, so the list can never claim a track is playing
//! when no session was created for it.

/// What happens when a track ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Repeat {
    /// Stop at the end of the queue.
    #[default]
    Off,
    /// Go back to the top at the end of the queue.
    All,
    /// Play the same track again.
    One,
}

/// An ordered list of entries and the position of the one playing.
#[derive(Debug, Clone, PartialEq)]
pub struct Queue<E> {
    entries: Vec<E>,
    current: Option<usize>,
}

impl<E> Default for Queue<E> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            current: None,
        }
    }
}

impl<E> Queue<E> {
    #[must_use]
    pub fn entries(&self) -> &[E] {
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
    pub fn current(&self) -> Option<&E> {
        self.entries.get(self.current?)
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&E> {
        self.entries.get(index)
    }

    /// Throw the list away and start again at `current`.
    ///
    /// Refused, and the old queue kept, when `current` is not a real index, so a
    /// caller that resolved nothing cannot empty the queue as a side effect.
    pub fn replace(&mut self, entries: Vec<E>, current: usize) -> bool {
        if current >= entries.len() {
            return false;
        }
        self.entries = entries;
        self.current = Some(current);
        true
    }

    /// Point at `index` without disturbing the list.
    pub fn set_position(&mut self, index: usize) -> bool {
        if index >= self.entries.len() {
            return false;
        }
        self.current = Some(index);
        true
    }

    /// Where `delta` entries away is, **wrapping**. For a key someone pressed:
    /// `n` on the last track is a request, and refusing it silently is a dead key.
    ///
    /// A peek, not a move. The caller commits with [`Queue::set_position`], so
    /// `current` never lands on an entry that then turns out to be unplayable.
    #[must_use]
    pub fn peek(&self, delta: isize) -> Option<usize> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        let current = self.current?;
        Some((current as isize + delta).rem_euclid(len as isize) as usize)
    }

    /// Where the entry after the current one is, or `None` at the end. Does not
    /// wrap: the end of the queue is the end of playback.
    #[must_use]
    pub fn peek_next(&self) -> Option<usize> {
        let next = self.current.map_or(0, |c| c + 1);
        (next < self.entries.len()).then_some(next)
    }

    /// Add to the end. Never moves `current`.
    pub fn push(&mut self, entry: E) {
        self.entries.push(entry);
    }

    /// Drop the entry at `index`. **The playing entry cannot be removed**: that
    /// would leave the list lying about the transport.
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

    /// Replace the whole list, keeping the playing entry playing wherever it moved
    /// to. Entries are matched by `key`, which must be unique per entry.
    ///
    /// Returns `false` when an entry was playing and the new list no longer holds
    /// it. The position is then cleared, and the caller decides what that means
    /// for what plays next.
    pub fn sync_by<K: PartialEq>(&mut self, entries: Vec<E>, key: impl Fn(&E) -> K) -> bool {
        let playing = self.current().map(&key);
        self.entries = entries;
        self.current = playing
            .as_ref()
            .and_then(|k| self.entries.iter().position(|e| key(e) == *k));
        playing.is_none() || self.current.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(entries: &[&'static str], current: usize) -> Queue<&'static str> {
        let mut q = Queue::default();
        assert!(q.replace(entries.to_vec(), current));
        q
    }

    #[test]
    fn an_empty_queue_has_nothing_to_play_and_nothing_to_advance_to() {
        let mut q: Queue<&str> = Queue::default();
        assert!(q.is_empty());
        assert_eq!(q.position(), None);
        assert!(q.current().is_none());
        assert_eq!(q.peek_next(), None);
        assert_eq!(q.peek(1), None);
        assert!(!q.set_position(0));
    }

    #[test]
    fn replacing_with_an_index_off_the_end_keeps_the_old_queue() {
        let mut q = q(&["one"], 0);
        assert!(!q.replace(vec!["a", "b"], 7));
        assert_eq!(q.len(), 1);
        assert_eq!(q.current(), Some(&"one"));
        assert!(!q.replace(Vec::new(), 0));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn peek_next_walks_forward_and_stops_at_the_end_rather_than_looping() {
        let mut q = q(&["one", "two"], 0);
        assert_eq!(q.peek_next(), Some(1));
        q.set_position(1);
        assert_eq!(q.peek_next(), None, "the end of the queue looped");
        assert_eq!(q.position(), Some(1), "peeking moved the queue");
    }

    #[test]
    fn peek_wraps_both_ways() {
        let mut q = q(&["one", "two", "three"], 0);
        assert_eq!(q.peek(-1), Some(2));
        q.set_position(2);
        assert_eq!(q.peek(1), Some(0));
        assert_eq!(q.position(), Some(2));
    }

    #[test]
    fn pushing_adds_to_the_end_without_disturbing_what_is_playing() {
        let mut q = q(&["one", "two"], 1);
        q.push("three");
        assert_eq!(q.len(), 3);
        assert_eq!(q.current(), Some(&"two"));
        assert_eq!(q.entries().last(), Some(&"three"));
    }

    #[test]
    fn removing_before_the_playing_entry_keeps_it_playing() {
        let mut q = q(&["one", "two", "three"], 2);
        assert!(q.remove(0));
        assert_eq!(q.position(), Some(1));
        assert_eq!(q.current(), Some(&"three"));
    }

    #[test]
    fn removing_after_the_playing_entry_leaves_the_position_alone() {
        let mut q = q(&["one", "two", "three"], 0);
        assert!(q.remove(2));
        assert_eq!(q.position(), Some(0));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn the_playing_entry_cannot_be_removed() {
        let mut q = q(&["one", "two"], 0);
        assert!(!q.remove(0));
        assert_eq!(q.len(), 2);
        assert!(!q.remove(9));
    }

    #[test]
    fn sync_follows_the_playing_entry_to_its_new_slot() {
        let mut q = q(&["a", "b", "c"], 1);
        assert!(q.sync_by(vec!["c", "a", "b"], |e| *e));
        assert_eq!(q.position(), Some(2));
        assert_eq!(q.current(), Some(&"b"));
    }

    #[test]
    fn sync_that_drops_the_playing_entry_says_so_and_clears_the_position() {
        let mut q = q(&["a", "b", "c"], 1);
        assert!(!q.sync_by(vec!["a", "c"], |e| *e));
        assert_eq!(q.position(), None);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn sync_with_nothing_playing_loses_nothing() {
        let mut q: Queue<&str> = Queue::default();
        assert!(q.sync_by(vec!["a"], |e| *e));
        assert_eq!(q.position(), None);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn repeat_travels_the_way_the_frontend_spells_it() {
        assert_eq!(serde_json::to_string(&Repeat::All).unwrap(), "\"all\"");
        assert_eq!(
            serde_json::from_str::<Repeat>("\"one\"").unwrap(),
            Repeat::One
        );
        assert_eq!(Repeat::default(), Repeat::Off);
    }
}
