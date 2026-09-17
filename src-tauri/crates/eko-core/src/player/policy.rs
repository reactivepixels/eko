//! The player's decisions, with no audio, no threads and no clock of its own.
//!
//! Every input is a [`Command`] (someone asked for something), a tick (the session
//! moved on by itself) or the decoder asking [`PlayerCore::next_after`]. Every output
//! is a list of [`Effect`]s for the driver to carry out. Keeping it pure is what lets
//! every rule here be tested without a sound card.

use super::item::{scrobble_threshold_ms, ItemMedia, PlayerSnapshot, QueueItem};
use crate::queue::{Queue, Repeat};
use crate::signal_path::{replaygain_decision, ReplayGainDecision, RgMode};

/// Why the decoder stopped offering more audio.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum EndReason {
    /// It hasn't: still decoding, or waiting to ask what follows.
    #[default]
    None,
    /// Nothing follows: the end of the queue, or the sleep timer's end of track.
    QueueEnd,
    /// The next item needs a different output rate. The decoder opened it and left it
    /// for the next session.
    RateChange { uid: String },
    /// The next item could not be resolved or opened.
    OpenFailed { uid: String },
}

/// The engine's session, as the player needs to see it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionView {
    /// A session exists and has not been stopped.
    pub exists: bool,
    /// Its first source opened and the output started.
    pub opened: bool,
    /// The output is still consuming audio. False once it ran off the end, or failed.
    pub playing: bool,
    pub paused: bool,
    /// Uid of the item under the playhead.
    pub uid: String,
    pub pos_ms: u64,
    pub dur_ms: u64,
    pub end: EndReason,
}

/// Something asked of the player.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// The frontend's whole queue, after any change to it.
    Sync(Vec<QueueItem>),
    /// A queue restored from the last run: positioned on `index` and not playing. Its
    /// first start of that item resumes at `pos_ms`.
    Restore {
        items: Vec<QueueItem>,
        index: usize,
        pos_ms: u64,
    },
    Play {
        uid: String,
    },
    Next,
    Prev,
    Toggle,
    Pause,
    Resume,
    Stop,
    Seek {
        ms: u64,
    },
    Clear,
    SetModes {
        repeat: Repeat,
        shuffle: bool,
    },
    SetReplayGain(RgMode),
    SetScrobble(bool),
    SleepAfterTrack,
    SleepIn {
        ms: u64,
    },
    CancelSleep,
    /// Start the playing item again from the top, because the output device changed.
    Restart,
}

/// Something for the driver to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Replace any session with one playing `uid` from `start_ms`.
    Start {
        uid: String,
        media: ItemMedia,
        start_ms: u64,
        rg_db: Option<f64>,
    },
    /// Start `uid` from the source the finished session already opened. `media` is the
    /// fallback when that source is gone.
    Handover {
        uid: String,
        media: ItemMedia,
        rg_db: Option<f64>,
    },
    Stop,
    Pause,
    Resume,
    Seek {
        ms: u64,
    },
    /// Throw away decoded audio after the playing item, and ask again what follows it.
    Rearm,
    SetGain {
        db: Option<f64>,
    },
    TrackStarted(QueueItem),
    Playback {
        item: Option<QueueItem>,
        playing: bool,
        pos_ms: u64,
    },
    Stopped(Option<QueueItem>),
    Scrobble {
        item: QueueItem,
        submission: bool,
    },
}

impl Effect {
    /// Whether this is news for the observer rather than work for the engine.
    #[must_use]
    pub fn is_note(&self) -> bool {
        matches!(
            self,
            Effect::TrackStarted(_)
                | Effect::Playback { .. }
                | Effect::Stopped(_)
                | Effect::Scrobble { .. }
        )
    }
}

/// Where play goes next when the playing item was removed from the queue.
#[derive(Clone, Debug, PartialEq)]
enum Detached {
    /// Something that followed it is still queued: this one.
    At(String),
    /// Nothing that followed it survived: the top of the queue.
    Top,
}

/// What the decoder was last told follows `after`.
#[derive(Clone, Debug, PartialEq)]
struct Answer {
    after: String,
    next: Option<String>,
}

/// The player's state and every rule about what plays.
pub struct PlayerCore {
    queue: Queue<QueueItem>,
    repeat: Repeat,
    shuffle: bool,
    rg_mode: RgMode,
    scrobble: bool,
    /// The item the session is playing. Kept even after it leaves the queue.
    playing: Option<QueueItem>,
    detached: Option<Detached>,
    answer: Option<Answer>,
    rg: ReplayGainDecision,
    /// A restored position, for the first start of that item only.
    resume: Option<(String, u64)>,
    stop_after_current: bool,
    /// The playing item's submission scrobble has been sent.
    scrobbled: bool,
    sleep_deadline_ms: Option<u64>,
    error: Option<String>,
    rng: u64,
}

impl PlayerCore {
    /// `seed` drives shuffle. Any value works.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            queue: Queue::default(),
            repeat: Repeat::Off,
            shuffle: false,
            rg_mode: RgMode::Off,
            scrobble: false,
            playing: None,
            detached: None,
            answer: None,
            rg: ReplayGainDecision::default(),
            resume: None,
            stop_after_current: false,
            scrobbled: false,
            sleep_deadline_ms: None,
            error: None,
            rng: seed | 1,
        }
    }

    /// The queued item with this uid.
    #[must_use]
    pub fn item(&self, uid: &str) -> Option<&QueueItem> {
        self.queue.entries().iter().find(|e| e.uid == uid)
    }

    /// The item the session is playing.
    #[must_use]
    pub fn playing(&self) -> Option<&QueueItem> {
        self.playing.as_ref()
    }

    /// What to show as now playing, with its index and the queue length: the playing
    /// item, else the queue's position.
    #[must_use]
    pub fn now_showing(&self) -> Option<(&QueueItem, usize, usize)> {
        let item = self.playing.as_ref().or_else(|| self.queue.current())?;
        let index = self.index_of(&item.uid).unwrap_or(0);
        Some((item, index, self.queue.len()))
    }

    /// Whether the player needs ticking even with no session, because a sleep timer is due.
    #[must_use]
    pub fn has_deadline(&self) -> bool {
        self.sleep_deadline_ms.is_some()
    }

    fn index_of(&self, uid: &str) -> Option<usize> {
        self.queue.entries().iter().position(|e| e.uid == uid)
    }

    fn pick(&mut self, n: usize) -> usize {
        // xorshift64*: plenty for shuffling a queue, and no dependency.
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) % n as u64) as usize
    }

    /// What the rules say follows `from`, as a uid (`None` inside: nothing follows).
    /// The outer `None` means shuffle makes it a draw.
    fn expected_after(&self, from: &str) -> Option<Option<String>> {
        if self.stop_after_current {
            return Some(None);
        }
        let idx = self.index_of(from);
        if self.repeat == Repeat::One {
            if let Some(i) = idx {
                return Some(Some(self.queue.entries()[i].uid.clone()));
            }
        }
        if self.shuffle {
            return None;
        }
        let len = self.queue.len();
        let wrap = if self.repeat == Repeat::All && len > 0 {
            Some(0)
        } else {
            None
        };
        let next = match idx {
            Some(c) if c + 1 < len => Some(c + 1),
            Some(_) => wrap,
            None => match &self.detached {
                Some(Detached::At(uid)) => self.index_of(uid),
                Some(Detached::Top) if len > 0 => Some(0),
                _ => None,
            },
        };
        Some(next.map(|i| self.queue.entries()[i].uid.clone()))
    }

    /// What plays after `from`, drawing when shuffle is on.
    fn successor(&mut self, from: &str) -> Option<QueueItem> {
        if let Some(next) = self.expected_after(from) {
            return next.and_then(|uid| self.item(&uid).cloned());
        }
        let len = self.queue.len();
        if len == 0 {
            return None;
        }
        let i = match self.index_of(from) {
            Some(c) if len > 1 => {
                // Never the same item twice in a row.
                let k = self.pick(len - 1);
                if k >= c {
                    k + 1
                } else {
                    k
                }
            }
            _ => self.pick(len),
        };
        self.queue.get(i).cloned()
    }

    /// The decoder finished `after` and asks what to join onto it.
    pub fn next_after(&mut self, after: &str) -> Option<QueueItem> {
        let next = self.successor(after);
        self.answer = Some(Answer {
            after: after.to_string(),
            next: next.as_ref().map(|i| i.uid.clone()),
        });
        next
    }

    /// Whether the decoder was told something about the playing item that the queue, or
    /// the modes, no longer agree with.
    fn answer_is_stale(&self, view: &SessionView) -> bool {
        let Some(answer) = &self.answer else {
            return false;
        };
        if !view.exists || answer.after != view.uid {
            // Not about the playing item. The decoder asks again when it gets there.
            return false;
        }
        match self.expected_after(&answer.after) {
            Some(expected) => expected != answer.next,
            // A shuffle draw stands while its item is still queued.
            None => match answer.next.as_deref() {
                Some(uid) => self.index_of(uid).is_none(),
                None => true,
            },
        }
    }

    fn rearm_if_stale(&mut self, view: &SessionView) -> Vec<Effect> {
        if self.answer_is_stale(view) {
            self.answer = None;
            vec![Effect::Rearm]
        } else {
            Vec::new()
        }
    }

    fn track_effects(&mut self, item: &QueueItem) -> Vec<Effect> {
        self.scrobbled = false;
        let mut fx = vec![Effect::TrackStarted(item.clone())];
        if self.scrobble && item.is_remote() {
            fx.push(Effect::Scrobble {
                item: item.clone(),
                submission: false,
            });
        }
        fx
    }

    fn start(&mut self, index: usize, start_ms: u64) -> Vec<Effect> {
        let Some(item) = self.queue.get(index).cloned() else {
            return Vec::new();
        };
        self.queue.set_position(index);
        self.detached = None;
        self.answer = None;
        self.resume = None;
        self.error = None;
        // A track someone chose is not the one the sleep timer was waiting to finish.
        self.stop_after_current = false;
        self.rg = replaygain_decision(&item.rg, self.rg_mode);
        let mut fx = vec![Effect::Start {
            uid: item.uid.clone(),
            media: item.media.clone(),
            start_ms,
            rg_db: self.rg.engine_db,
        }];
        fx.extend(self.track_effects(&item));
        self.playing = Some(item);
        fx
    }

    fn start_uid(&mut self, uid: &str) -> Vec<Effect> {
        let Some(i) = self.index_of(uid) else {
            return Vec::new();
        };
        let start_ms = match &self.resume {
            Some((resume_uid, ms)) if resume_uid == uid => *ms,
            _ => 0,
        };
        self.start(i, start_ms)
    }

    /// With no session: the queue's position, else the top.
    fn start_from_rest(&mut self) -> Vec<Effect> {
        match self.queue.current().map(|e| e.uid.clone()) {
            Some(uid) => self.start_uid(&uid),
            None if !self.queue.is_empty() => self.start(0, 0),
            None => Vec::new(),
        }
    }

    fn stop(&mut self) -> Vec<Effect> {
        self.answer = None;
        vec![Effect::Stop, Effect::Stopped(self.playing.take())]
    }

    fn pause(&self, view: &SessionView) -> Vec<Effect> {
        vec![
            Effect::Pause,
            Effect::Playback {
                item: self.playing.clone(),
                playing: false,
                pos_ms: view.pos_ms,
            },
        ]
    }

    fn resume(&self, view: &SessionView) -> Vec<Effect> {
        vec![
            Effect::Resume,
            Effect::Playback {
                item: self.playing.clone(),
                playing: true,
                pos_ms: view.pos_ms,
            },
        ]
    }

    /// The uid "next" and "previous" are relative to.
    fn relative_uid(&self, view: &SessionView) -> String {
        if view.exists && !view.uid.is_empty() {
            return view.uid.clone();
        }
        self.playing
            .as_ref()
            .or_else(|| self.queue.current())
            .map(|e| e.uid.clone())
            .unwrap_or_default()
    }

    fn sync(&mut self, items: Vec<QueueItem>, view: &SessionView) -> Vec<Effect> {
        let followers: Vec<String> = match self.queue.position() {
            Some(c) => self.queue.entries()[c + 1..]
                .iter()
                .map(|e| e.uid.clone())
                .collect(),
            None => Vec::new(),
        };
        if !self.queue.sync_by(items, |e| e.uid.clone()) {
            // The playing item left the queue. It plays on; after it comes whatever
            // followed it and is still queued, else the top.
            let survivor = followers
                .iter()
                .find(|uid| self.index_of(uid).is_some())
                .cloned();
            self.detached = Some(match survivor {
                Some(uid) => Detached::At(uid),
                None => Detached::Top,
            });
        } else {
            let gone =
                matches!(&self.detached, Some(Detached::At(uid)) if self.index_of(uid).is_none());
            if gone {
                self.detached = Some(Detached::Top);
            }
        }
        self.rearm_if_stale(view)
    }

    /// Carry out a request.
    pub fn command(&mut self, cmd: Command, view: &SessionView, now_ms: u64) -> Vec<Effect> {
        match cmd {
            Command::Sync(items) => self.sync(items, view),
            Command::Restore {
                items,
                index,
                pos_ms,
            } => {
                let fx = if view.exists { self.stop() } else { Vec::new() };
                let index = index.min(items.len().saturating_sub(1));
                self.resume = items
                    .get(index)
                    .filter(|_| pos_ms > 0)
                    .map(|i| (i.uid.clone(), pos_ms));
                self.queue = Queue::default();
                self.queue.replace(items, index);
                self.detached = None;
                self.answer = None;
                self.playing = None;
                self.error = None;
                fx
            }
            Command::Play { uid } => self.start_uid(&uid),
            Command::Next => {
                // A press is not the sleep timer's business.
                self.stop_after_current = false;
                let from = self.relative_uid(view);
                let next = if from.is_empty() && self.detached.is_none() {
                    self.queue.get(0).cloned()
                } else {
                    self.successor(&from)
                };
                match next.and_then(|item| self.index_of(&item.uid)) {
                    Some(i) => self.start(i, 0),
                    None => self.stop(),
                }
            }
            Command::Prev => {
                if view.exists && view.pos_ms > 3_000 {
                    return vec![
                        Effect::Seek { ms: 0 },
                        Effect::Playback {
                            item: self.playing.clone(),
                            playing: view.playing && !view.paused,
                            pos_ms: 0,
                        },
                    ];
                }
                match self.queue.position() {
                    Some(c) => self.start(c.saturating_sub(1), 0),
                    None => Vec::new(),
                }
            }
            Command::Toggle => {
                if !view.exists {
                    self.start_from_rest()
                } else if view.paused {
                    self.resume(view)
                } else {
                    self.pause(view)
                }
            }
            Command::Pause => {
                if view.exists && !view.paused {
                    self.pause(view)
                } else {
                    Vec::new()
                }
            }
            Command::Resume => {
                if !view.exists {
                    self.start_from_rest()
                } else if view.paused {
                    self.resume(view)
                } else {
                    Vec::new()
                }
            }
            Command::Stop => self.stop(),
            Command::Seek { ms } => {
                if !view.exists {
                    return Vec::new();
                }
                vec![
                    Effect::Seek { ms },
                    Effect::Playback {
                        item: self.playing.clone(),
                        playing: view.playing && !view.paused,
                        pos_ms: ms,
                    },
                ]
            }
            Command::Clear => {
                self.queue = Queue::default();
                self.detached = None;
                self.resume = None;
                self.stop_after_current = false;
                self.sleep_deadline_ms = None;
                self.error = None;
                self.stop()
            }
            Command::SetModes { repeat, shuffle } => {
                self.repeat = repeat;
                self.shuffle = shuffle;
                self.rearm_if_stale(view)
            }
            Command::SetReplayGain(mode) => {
                self.rg_mode = mode;
                self.rg = self
                    .playing
                    .as_ref()
                    .map(|p| replaygain_decision(&p.rg, mode))
                    .unwrap_or_default();
                vec![Effect::SetGain {
                    db: self.rg.engine_db,
                }]
            }
            Command::SetScrobble(on) => {
                self.scrobble = on;
                Vec::new()
            }
            Command::SleepAfterTrack => {
                self.sleep_deadline_ms = None;
                self.stop_after_current = true;
                self.rearm_if_stale(view)
            }
            Command::SleepIn { ms } => {
                self.sleep_deadline_ms = Some(now_ms.saturating_add(ms));
                self.stop_after_current = false;
                self.rearm_if_stale(view)
            }
            Command::CancelSleep => {
                self.sleep_deadline_ms = None;
                self.stop_after_current = false;
                self.rearm_if_stale(view)
            }
            Command::Restart => {
                let index = self.playing.as_ref().and_then(|p| self.index_of(&p.uid));
                match index {
                    Some(i) if view.exists && view.playing && !view.paused => self.start(i, 0),
                    _ => Vec::new(),
                }
            }
        }
    }

    /// The session moved on by itself. Called on every player tick and every poll.
    pub fn tick(&mut self, view: &SessionView, now_ms: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        if view.exists {
            fx.extend(self.follow_playhead(view));
            if !view.playing {
                fx.extend(self.session_ended(view));
            } else if !view.paused {
                fx.extend(self.submit_scrobble_when_due(view));
            }
        }
        if let Some(deadline) = self.sleep_deadline_ms {
            if now_ms >= deadline {
                self.sleep_deadline_ms = None;
                if view.exists && view.playing && !view.paused {
                    fx.extend(self.pause(view));
                }
            }
        }
        fx
    }

    /// A gapless seam: the playhead is in a different item from the one last seen.
    fn follow_playhead(&mut self, view: &SessionView) -> Vec<Effect> {
        if view.uid.is_empty() || self.playing.as_ref().is_some_and(|p| p.uid == view.uid) {
            return Vec::new();
        }
        let Some(i) = self.index_of(&view.uid) else {
            return Vec::new();
        };
        let item = self.queue.entries()[i].clone();
        self.queue.set_position(i);
        self.detached = None;
        self.rg = replaygain_decision(&item.rg, self.rg_mode);
        let mut fx = vec![Effect::SetGain {
            db: self.rg.engine_db,
        }];
        fx.extend(self.track_effects(&item));
        self.playing = Some(item);
        fx
    }

    /// The output ran out of audio, or never started.
    fn session_ended(&mut self, view: &SessionView) -> Vec<Effect> {
        if !view.opened {
            let title = self
                .playing
                .as_ref()
                .map(|p| p.title.clone())
                .unwrap_or_default();
            self.error = Some(format!("Couldn't play {title}"));
            return self.stop();
        }
        match &view.end {
            EndReason::RateChange { uid } => {
                let Some(i) = self.index_of(uid) else {
                    return self.stop();
                };
                let item = self.queue.entries()[i].clone();
                self.queue.set_position(i);
                self.detached = None;
                self.answer = None;
                self.rg = replaygain_decision(&item.rg, self.rg_mode);
                let mut fx = vec![Effect::Handover {
                    uid: item.uid.clone(),
                    media: item.media.clone(),
                    rg_db: self.rg.engine_db,
                }];
                fx.extend(self.track_effects(&item));
                self.playing = Some(item);
                fx
            }
            EndReason::OpenFailed { uid } => {
                let title = self.item(uid).map(|i| i.title.clone()).unwrap_or_default();
                if let Some(i) = self.index_of(uid) {
                    self.queue.set_position(i);
                    self.detached = None;
                }
                self.error = Some(format!("Couldn't play {title}"));
                self.stop()
            }
            EndReason::QueueEnd | EndReason::None => {
                if self.stop_after_current {
                    // The sleep timer's end of track: stop here, but leave the queue on
                    // what would have played next so that Play carries on from it.
                    self.stop_after_current = false;
                    if let Some(Some(next)) = self.expected_after(&view.uid) {
                        if let Some(i) = self.index_of(&next) {
                            self.queue.set_position(i);
                            self.detached = None;
                        }
                    }
                }
                self.stop()
            }
        }
    }

    fn submit_scrobble_when_due(&mut self, view: &SessionView) -> Vec<Effect> {
        if !self.scrobble || self.scrobbled {
            return Vec::new();
        }
        let Some(item) = self
            .playing
            .as_ref()
            .filter(|p| p.uid == view.uid && p.is_remote())
            .cloned()
        else {
            return Vec::new();
        };
        let duration = if view.dur_ms > 0 {
            view.dur_ms
        } else {
            item.duration_ms
        };
        match scrobble_threshold_ms(duration) {
            Some(at) if view.pos_ms >= at => {
                self.scrobbled = true;
                vec![Effect::Scrobble {
                    item,
                    submission: true,
                }]
            }
            _ => Vec::new(),
        }
    }

    /// What the frontend is told.
    #[must_use]
    pub fn snapshot(&self, view: &SessionView, now_ms: u64) -> PlayerSnapshot {
        let uid = self
            .playing
            .as_ref()
            .or_else(|| self.queue.current())
            .map(|e| e.uid.clone());
        PlayerSnapshot {
            index: uid
                .as_deref()
                .and_then(|u| self.index_of(u))
                .map(|i| i as u32),
            uid,
            active: view.exists,
            playing: view.exists && view.playing && !view.paused,
            error: self.error.clone(),
            rg_engine_db: self.rg.engine_db,
            rg_seal_db: self.rg.seal_db,
            stop_after_current: self.stop_after_current,
            sleep_remaining_ms: self.sleep_deadline_ms.map(|d| d.saturating_sub(now_ms)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal_path::ReplayGainTags;

    fn file(uid: &str) -> QueueItem {
        QueueItem {
            uid: uid.into(),
            media: ItemMedia::File {
                path: format!("/m/{uid}.flac"),
            },
            title: uid.to_uppercase(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            cover_url: String::new(),
            rg: ReplayGainTags::default(),
        }
    }

    fn remote(uid: &str) -> QueueItem {
        QueueItem {
            media: ItemMedia::Remote {
                id: format!("id-{uid}"),
                server: "https://m.example".into(),
            },
            ..file(uid)
        }
    }

    fn items(uids: &[&str]) -> Vec<QueueItem> {
        uids.iter().map(|u| file(u)).collect()
    }

    /// A live session with the playhead `pos_ms` into `uid`.
    fn playing(uid: &str, pos_ms: u64) -> SessionView {
        SessionView {
            exists: true,
            opened: true,
            playing: true,
            paused: false,
            uid: uid.into(),
            pos_ms,
            dur_ms: 200_000,
            end: EndReason::None,
        }
    }

    fn idle() -> SessionView {
        SessionView::default()
    }

    fn started(fx: &[Effect]) -> Option<(&str, u64)> {
        fx.iter().find_map(|e| match e {
            Effect::Start { uid, start_ms, .. } => Some((uid.as_str(), *start_ms)),
            _ => None,
        })
    }

    /// `uids` queued and `current` started, as the driver would have left it.
    fn core_playing(uids: &[&str], current: &str) -> PlayerCore {
        let mut c = PlayerCore::new(7);
        c.command(Command::Sync(items(uids)), &idle(), 0);
        c.command(
            Command::Play {
                uid: current.into(),
            },
            &idle(),
            0,
        );
        c
    }

    #[test]
    fn play_starts_the_item_and_announces_it() {
        let mut c = PlayerCore::new(7);
        c.command(Command::Sync(items(&["a", "b"])), &idle(), 0);
        let fx = c.command(Command::Play { uid: "b".into() }, &idle(), 0);
        assert_eq!(started(&fx), Some(("b", 0)));
        assert!(fx.contains(&Effect::TrackStarted(file("b"))));
        assert_eq!(c.snapshot(&idle(), 0).index, Some(1));
    }

    #[test]
    fn the_decoder_is_told_the_next_item_in_order_and_nothing_at_the_end() {
        let mut c = core_playing(&["a", "b", "c"], "a");
        assert_eq!(c.next_after("a").map(|i| i.uid), Some("b".into()));
        assert_eq!(c.next_after("c").map(|i| i.uid), None);
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_repeats() {
        let mut c = core_playing(&["a", "b"], "b");
        let view = playing("b", 0);
        c.command(
            Command::SetModes {
                repeat: Repeat::All,
                shuffle: false,
            },
            &view,
            0,
        );
        assert_eq!(c.next_after("b").unwrap().uid, "a");
        c.command(
            Command::SetModes {
                repeat: Repeat::One,
                shuffle: false,
            },
            &view,
            0,
        );
        assert_eq!(c.next_after("b").unwrap().uid, "b");
    }

    #[test]
    fn shuffle_never_plays_the_same_item_twice_in_a_row_and_reaches_every_other() {
        let mut c = core_playing(&["a", "b", "c", "d"], "c");
        c.command(
            Command::SetModes {
                repeat: Repeat::Off,
                shuffle: true,
            },
            &playing("c", 0),
            0,
        );
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..200 {
            let next = c.next_after("c").unwrap().uid;
            assert_ne!(next, "c");
            seen.insert(next);
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn a_restored_queue_resumes_its_own_track_where_it_left_off() {
        let mut c = PlayerCore::new(7);
        c.command(
            Command::Restore {
                items: items(&["a", "b"]),
                index: 1,
                pos_ms: 42_000,
            },
            &idle(),
            0,
        );
        assert_eq!(c.snapshot(&idle(), 0).uid.as_deref(), Some("b"));
        assert_eq!(
            started(&c.command(Command::Toggle, &idle(), 0)),
            Some(("b", 42_000))
        );
    }

    #[test]
    fn a_restored_position_does_not_leak_onto_a_different_track() {
        let mut c = PlayerCore::new(7);
        c.command(
            Command::Restore {
                items: items(&["a", "b"]),
                index: 1,
                pos_ms: 42_000,
            },
            &idle(),
            0,
        );
        let fx = c.command(Command::Play { uid: "a".into() }, &idle(), 0);
        assert_eq!(started(&fx), Some(("a", 0)));
    }

    #[test]
    fn next_at_the_end_stops() {
        let mut c = core_playing(&["a", "b"], "b");
        let fx = c.command(Command::Next, &playing("b", 1_000), 0);
        assert!(fx.contains(&Effect::Stop));
        assert_eq!(started(&fx), None);
    }

    #[test]
    fn prev_restarts_after_three_seconds_and_goes_back_before() {
        let mut c = core_playing(&["a", "b"], "b");
        assert_eq!(
            c.command(Command::Prev, &playing("b", 3_001), 0)[0],
            Effect::Seek { ms: 0 }
        );
        assert_eq!(
            started(&c.command(Command::Prev, &playing("b", 2_000), 0)),
            Some(("a", 0))
        );
    }

    #[test]
    fn toggle_pauses_resumes_and_starts_from_rest() {
        let mut c = core_playing(&["a"], "a");
        assert_eq!(
            c.command(Command::Toggle, &playing("a", 5), 0)[0],
            Effect::Pause
        );
        let paused = SessionView {
            paused: true,
            ..playing("a", 5)
        };
        assert_eq!(c.command(Command::Toggle, &paused, 0)[0], Effect::Resume);
        c.command(Command::Stop, &playing("a", 5), 0);
        assert_eq!(
            started(&c.command(Command::Toggle, &idle(), 0)),
            Some(("a", 0))
        );
    }

    #[test]
    fn reordering_rearms_a_decoder_that_was_told_the_wrong_next() {
        let mut c = core_playing(&["a", "b", "c"], "a");
        c.next_after("a");
        let fx = c.command(Command::Sync(items(&["a", "c", "b"])), &playing("a", 0), 0);
        assert_eq!(fx, vec![Effect::Rearm]);
        assert_eq!(c.next_after("a").unwrap().uid, "c");
    }

    #[test]
    fn a_change_that_keeps_the_next_item_leaves_the_decoder_alone() {
        let mut c = core_playing(&["a", "b", "c"], "a");
        c.next_after("a");
        let fx = c.command(
            Command::Sync(items(&["a", "b", "c", "d"])),
            &playing("a", 0),
            0,
        );
        assert!(fx.is_empty());
    }

    #[test]
    fn appending_rearms_a_decoder_that_was_told_nothing_follows() {
        let mut c = core_playing(&["a"], "a");
        assert!(c.next_after("a").is_none());
        let fx = c.command(Command::Sync(items(&["a", "b"])), &playing("a", 0), 0);
        assert_eq!(fx, vec![Effect::Rearm]);
    }

    #[test]
    fn an_answer_about_a_track_no_longer_under_the_playhead_is_left_alone() {
        let mut c = core_playing(&["a", "b", "c"], "a");
        c.next_after("a");
        // The playhead is in b now; the decoder will ask about b when it gets there.
        let fx = c.command(
            Command::Sync(items(&["a", "b", "d", "c"])),
            &playing("b", 0),
            0,
        );
        assert!(fx.is_empty());
    }

    #[test]
    fn removing_the_playing_track_plays_on_then_continues_with_what_followed_it() {
        let mut c = core_playing(&["a", "b", "c"], "b");
        c.command(Command::Sync(items(&["a", "c"])), &playing("b", 0), 0);
        assert_eq!(c.next_after("b").unwrap().uid, "c");
    }

    #[test]
    fn removing_the_playing_track_and_everything_after_it_goes_back_to_the_top() {
        let mut c = core_playing(&["a", "b", "c"], "b");
        c.command(Command::Sync(items(&["a"])), &playing("b", 0), 0);
        assert_eq!(c.next_after("b").unwrap().uid, "a");
    }

    #[test]
    fn the_sleep_timers_end_of_track_tells_the_decoder_nothing_follows() {
        let mut c = core_playing(&["a", "b"], "a");
        c.next_after("a");
        let fx = c.command(Command::SleepAfterTrack, &playing("a", 0), 0);
        assert_eq!(fx, vec![Effect::Rearm]);
        assert!(c.next_after("a").is_none());
    }

    #[test]
    fn pressing_next_overrides_the_sleep_timers_end_of_track() {
        let mut c = core_playing(&["a", "b"], "a");
        c.command(Command::SleepAfterTrack, &playing("a", 0), 0);
        let fx = c.command(Command::Next, &playing("a", 0), 0);
        assert_eq!(started(&fx), Some(("b", 0)));
        assert!(!c.snapshot(&idle(), 0).stop_after_current);
    }

    #[test]
    fn replaygain_follows_the_playing_item_and_the_mode() {
        let mut loud = file("a");
        loud.rg.track_gain = Some(-6.0);
        let mut c = PlayerCore::new(7);
        c.command(Command::Sync(vec![loud]), &idle(), 0);
        c.command(Command::SetReplayGain(RgMode::Track), &idle(), 0);
        let fx = c.command(Command::Play { uid: "a".into() }, &idle(), 0);
        assert!(matches!(fx[0], Effect::Start { rg_db: Some(db), .. } if db == -6.0));
        assert_eq!(
            c.command(Command::SetReplayGain(RgMode::Off), &playing("a", 0), 0),
            vec![Effect::SetGain { db: None }]
        );
    }

    #[test]
    fn scrobbling_waits_for_the_frontend_and_skips_local_files() {
        let mut c = PlayerCore::new(7);
        c.command(Command::Sync(vec![remote("a"), file("b")]), &idle(), 0);
        let is_scrobble = |e: &Effect| matches!(e, Effect::Scrobble { .. });
        let fx = c.command(Command::Play { uid: "a".into() }, &idle(), 0);
        assert!(!fx.iter().any(is_scrobble));
        c.command(Command::SetScrobble(true), &idle(), 0);
        let fx = c.command(Command::Play { uid: "a".into() }, &idle(), 0);
        assert!(fx.contains(&Effect::Scrobble {
            item: remote("a"),
            submission: false
        }));
        let fx = c.command(Command::Play { uid: "b".into() }, &idle(), 0);
        assert!(!fx.iter().any(is_scrobble));
    }

    #[test]
    fn clearing_forgets_the_queue_and_the_sleep_timer() {
        let mut c = core_playing(&["a"], "a");
        c.command(Command::SleepIn { ms: 60_000 }, &playing("a", 0), 0);
        let fx = c.command(Command::Clear, &playing("a", 0), 0);
        assert!(fx.contains(&Effect::Stop));
        let snap = c.snapshot(&idle(), 0);
        assert_eq!(snap.uid, None);
        assert_eq!(snap.sleep_remaining_ms, None);
        assert!(!c.has_deadline());
    }
    #[test]
    fn crossing_a_gapless_seam_moves_the_queue_and_announces_the_new_track_once() {
        let mut c = core_playing(&["a", "b"], "a");
        let fx = c.tick(&playing("b", 10), 0);
        assert!(fx.contains(&Effect::TrackStarted(file("b"))));
        assert!(fx.contains(&Effect::SetGain { db: None }));
        assert_eq!(c.snapshot(&playing("b", 10), 0).index, Some(1));
        assert!(c.tick(&playing("b", 20), 0).is_empty(), "announced twice");
    }

    #[test]
    fn the_end_of_the_queue_stops_the_session() {
        let mut c = core_playing(&["a"], "a");
        let ended = SessionView {
            playing: false,
            end: EndReason::QueueEnd,
            ..playing("a", 200_000)
        };
        assert_eq!(
            c.tick(&ended, 0),
            vec![Effect::Stop, Effect::Stopped(Some(file("a")))]
        );
    }

    #[test]
    fn a_rate_change_hands_the_next_item_to_a_new_session() {
        let mut c = core_playing(&["a", "b"], "a");
        let ended = SessionView {
            playing: false,
            end: EndReason::RateChange { uid: "b".into() },
            ..playing("a", 200_000)
        };
        let fx = c.tick(&ended, 0);
        assert!(matches!(&fx[0], Effect::Handover { uid, .. } if uid == "b"));
        assert!(fx.contains(&Effect::TrackStarted(file("b"))));
        assert_eq!(c.snapshot(&idle(), 0).uid.as_deref(), Some("b"));
    }

    #[test]
    fn a_track_that_will_not_open_stops_with_its_name_rather_than_skipping_ahead() {
        let mut c = core_playing(&["a", "b", "c"], "a");
        let ended = SessionView {
            playing: false,
            end: EndReason::OpenFailed { uid: "b".into() },
            ..playing("a", 200_000)
        };
        let fx = c.tick(&ended, 0);
        assert_eq!(started(&fx), None);
        assert!(fx.contains(&Effect::Stop));
        let snap = c.snapshot(&idle(), 0);
        assert_eq!(snap.error.as_deref(), Some("Couldn't play B"));
        assert_eq!(snap.uid.as_deref(), Some("b"));
    }

    #[test]
    fn a_session_that_never_opened_is_an_error() {
        let mut c = core_playing(&["a"], "a");
        let dead = SessionView {
            exists: true,
            ..idle()
        };
        assert!(c.tick(&dead, 0).contains(&Effect::Stop));
        assert_eq!(
            c.snapshot(&idle(), 0).error.as_deref(),
            Some("Couldn't play A")
        );
    }

    #[test]
    fn the_sleep_timers_end_of_track_stops_and_leaves_play_on_the_next_track() {
        let mut c = core_playing(&["a", "b"], "a");
        c.command(Command::SleepAfterTrack, &playing("a", 0), 0);
        let ended = SessionView {
            playing: false,
            end: EndReason::QueueEnd,
            ..playing("a", 200_000)
        };
        assert!(c.tick(&ended, 0).contains(&Effect::Stop));
        let snap = c.snapshot(&idle(), 0);
        assert_eq!(snap.uid.as_deref(), Some("b"));
        assert!(!snap.stop_after_current);
    }

    #[test]
    fn a_submission_scrobble_fires_once_at_the_threshold() {
        let mut c = PlayerCore::new(7);
        c.command(Command::SetScrobble(true), &idle(), 0);
        c.command(Command::Sync(vec![remote("a")]), &idle(), 0);
        c.command(Command::Play { uid: "a".into() }, &idle(), 0);
        assert!(c.tick(&playing("a", 99_999), 0).is_empty());
        assert_eq!(
            c.tick(&playing("a", 100_000), 0),
            vec![Effect::Scrobble {
                item: remote("a"),
                submission: true
            }]
        );
        assert!(c.tick(&playing("a", 150_000), 0).is_empty());
    }

    #[test]
    fn a_track_reached_through_a_seam_is_scrobbled_too() {
        let mut c = PlayerCore::new(7);
        c.command(Command::SetScrobble(true), &idle(), 0);
        c.command(Command::Sync(vec![remote("a"), remote("b")]), &idle(), 0);
        c.command(Command::Play { uid: "a".into() }, &idle(), 0);
        c.tick(&playing("a", 150_000), 0);
        let fx = c.tick(&playing("b", 0), 0);
        assert!(fx.contains(&Effect::Scrobble {
            item: remote("b"),
            submission: false
        }));
        assert!(c
            .tick(&playing("b", 100_000), 0)
            .contains(&Effect::Scrobble {
                item: remote("b"),
                submission: true
            }));
    }

    #[test]
    fn a_due_sleep_timer_pauses_playback() {
        let mut c = core_playing(&["a"], "a");
        c.command(Command::SleepIn { ms: 1_000 }, &playing("a", 0), 5_000);
        assert!(c.tick(&playing("a", 10), 5_999).is_empty());
        assert_eq!(c.tick(&playing("a", 10), 6_000)[0], Effect::Pause);
        assert_eq!(
            c.snapshot(&playing("a", 10), 6_000).sleep_remaining_ms,
            None
        );
    }
}
