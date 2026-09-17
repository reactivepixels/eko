//! Runs [`PlayerCore`] against the engine: from commands, and from a thread of its own,
//! so that auto-advance, ReplayGain, scrobbles and the sleep timer happen with no UI
//! awake at all.

use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use super::item::{ItemMedia, PlayerObserver, PlayerSnapshot, QueueItem, SourceResolver};
use super::policy::{Command, Effect, PlayerCore};
use crate::engine::{Continuer, Engine, First, Inner, Next, NowPlaying, Source};
use crate::queue::Repeat;
use crate::signal_path::{ReplayGainDecision, RgMode};

/// How often the player looks at the session while there is something to watch.
const TICK: Duration = Duration::from_millis(25);

/// The player's half of the engine's state.
pub(crate) struct PlayerState {
    core: Mutex<PlayerCore>,
    resolver: RwLock<Option<Arc<dyn SourceResolver>>>,
    observer: RwLock<Option<Arc<dyn PlayerObserver>>>,
    thread: Mutex<Option<std::thread::Thread>>,
    epoch: Instant,
}

impl Default for PlayerState {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        PlayerState {
            core: Mutex::new(PlayerCore::new(seed)),
            resolver: RwLock::new(None),
            observer: RwLock::new(None),
            thread: Mutex::new(None),
            epoch: Instant::now(),
        }
    }
}

impl PlayerState {
    /// Overwrite what the mini player shows with the player's item, when it has one.
    pub(crate) fn fill_now_playing(&self, np: &mut NowPlaying) {
        let core = self.core.lock().unwrap();
        let Some((item, index, total)) = core.now_showing() else {
            return;
        };
        np.title = item.title.clone();
        np.artist = item.artist.clone();
        // The same sizing `coverAt(url, 160)` does in the frontend.
        np.cover_url = if item.cover_url.is_empty() {
            String::new()
        } else {
            format!("{}&size=160", item.cover_url)
        };
        np.cover_path = match &item.media {
            ItemMedia::File { path } => path.clone(),
            ItemMedia::Remote { .. } => String::new(),
        };
        np.index = index as i64;
        np.total = total as i64;
    }
}

/// Answers the decoder for sessions the player started.
struct PlayerNext(Weak<Inner>);

impl Continuer for PlayerNext {
    fn next_after(&self, after: &str) -> Next {
        let Some(inner) = self.0.upgrade() else {
            return Next::Nothing;
        };
        let engine = Engine::from_inner(inner);
        let next = engine.player.core.lock().unwrap().next_after(after);
        let Some(item) = next else {
            return Next::Nothing;
        };
        match engine.resolve(&item.media) {
            Some(source) => Next::Open {
                uid: item.uid,
                source,
            },
            None => Next::Unplayable { uid: item.uid },
        }
    }
}

impl Engine {
    /// How queued items become sources. The host app sets this once at startup.
    pub fn set_resolver(&self, resolver: Arc<dyn SourceResolver>) {
        *self.player.resolver.write().unwrap() = Some(resolver);
    }

    /// Who hears about track changes. The host app sets this once at startup.
    pub fn set_observer(&self, observer: Arc<dyn PlayerObserver>) {
        *self.player.observer.write().unwrap() = Some(observer);
    }

    /// The frontend's whole queue, after any change to it.
    pub fn queue_sync(&self, items: Vec<QueueItem>) -> PlayerSnapshot {
        self.drive(Some(Command::Sync(items)))
    }

    /// A queue restored from the last run, positioned on `index`, resuming at `pos_ms`.
    pub fn queue_restore(
        &self,
        items: Vec<QueueItem>,
        index: usize,
        pos_ms: u64,
    ) -> PlayerSnapshot {
        self.drive(Some(Command::Restore {
            items,
            index,
            pos_ms,
        }))
    }

    pub fn queue_clear(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Clear))
    }

    pub fn player_play(&self, uid: String) -> PlayerSnapshot {
        self.drive(Some(Command::Play { uid }))
    }

    pub fn player_next(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Next))
    }

    pub fn player_prev(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Prev))
    }

    pub fn player_toggle(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Toggle))
    }

    pub fn player_pause(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Pause))
    }

    pub fn player_resume(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Resume))
    }

    pub fn player_stop(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Stop))
    }

    pub fn player_seek(&self, ms: u64) -> PlayerSnapshot {
        self.drive(Some(Command::Seek { ms }))
    }

    pub fn player_set_modes(&self, repeat: Repeat, shuffle: bool) -> PlayerSnapshot {
        self.drive(Some(Command::SetModes { repeat, shuffle }))
    }

    /// Set the ReplayGain mode and return the decision for the playing item, so the
    /// seal can be updated from the reply rather than a later poll.
    pub fn player_set_replaygain(&self, mode: RgMode) -> ReplayGainDecision {
        let snap = self.drive(Some(Command::SetReplayGain(mode)));
        ReplayGainDecision {
            engine_db: snap.rg_engine_db,
            seal_db: snap.rg_seal_db,
        }
    }

    pub fn player_set_scrobble(&self, on: bool) -> PlayerSnapshot {
        self.drive(Some(Command::SetScrobble(on)))
    }

    pub fn player_sleep_after_track(&self) -> PlayerSnapshot {
        self.drive(Some(Command::SleepAfterTrack))
    }

    pub fn player_sleep_in(&self, ms: u64) -> PlayerSnapshot {
        self.drive(Some(Command::SleepIn { ms }))
    }

    pub fn player_cancel_sleep(&self) -> PlayerSnapshot {
        self.drive(Some(Command::CancelSleep))
    }

    /// Start the playing item again (the output device changed).
    pub fn player_restart(&self) -> PlayerSnapshot {
        self.drive(Some(Command::Restart))
    }

    /// Tick and report. What the frontend polls.
    pub fn player_poll(&self) -> PlayerSnapshot {
        self.drive(None)
    }

    /// A command (or none), then a tick; wakes the player's thread afterwards.
    fn drive(&self, cmd: Option<Command>) -> PlayerSnapshot {
        let snap = self.run(cmd);
        self.wake();
        snap
    }

    fn run(&self, cmd: Option<Command>) -> PlayerSnapshot {
        let now = self.player.epoch.elapsed().as_millis() as u64;
        let mut notes = Vec::new();
        let snap = {
            let mut core = self.player.core.lock().unwrap();
            if let Some(cmd) = cmd {
                let fx = core.command(cmd, &self.session_view(), now);
                self.apply(fx, &mut notes);
            }
            let fx = core.tick(&self.session_view(), now);
            self.apply(fx, &mut notes);
            core.snapshot(&self.session_view(), now)
        };
        self.notify(notes);
        snap
    }

    fn apply(&self, effects: Vec<Effect>, notes: &mut Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Start {
                    uid,
                    media,
                    start_ms,
                    rg_db,
                } => self.start_item(uid, &media, start_ms, rg_db),
                Effect::Handover { uid, media, rg_db } => match self.take_handover() {
                    Some((opened_uid, opened)) if opened_uid == uid => self.start_player_session(
                        uid,
                        First::Opened(opened),
                        0,
                        rg_db,
                        self.continuer(),
                    ),
                    _ => self.start_item(uid, &media, 0, rg_db),
                },
                Effect::Stop => self.stop(),
                Effect::Pause => self.pause(),
                Effect::Resume => self.resume(),
                Effect::Seek { ms } => self.seek(ms as f64 / 1000.0),
                Effect::Rearm => self.rearm(),
                Effect::SetGain { db } => self.set_replaygain(db.map(|d| d as f32)),
                note => notes.push(note),
            }
        }
    }

    fn start_item(&self, uid: String, media: &ItemMedia, start_ms: u64, rg_db: Option<f64>) {
        match self.resolve(media) {
            Some(source) => self.start_player_session(
                uid,
                First::Source(source),
                start_ms,
                rg_db,
                self.continuer(),
            ),
            None => self.start_dead_session(uid),
        }
    }

    fn resolve(&self, media: &ItemMedia) -> Option<Source> {
        let resolver = self.player.resolver.read().unwrap().clone()?;
        resolver.resolve(media)
    }

    fn continuer(&self) -> Arc<dyn Continuer> {
        Arc::new(PlayerNext(self.weak()))
    }

    fn notify(&self, notes: Vec<Effect>) {
        if notes.is_empty() {
            return;
        }
        let Some(observer) = self.player.observer.read().unwrap().clone() else {
            return;
        };
        for note in notes {
            match note {
                Effect::TrackStarted(item) => observer.track_started(&item),
                Effect::Playback {
                    item,
                    playing,
                    pos_ms,
                } => observer.playback(item.as_ref(), playing, pos_ms),
                Effect::Stopped(item) => observer.stopped(item.as_ref()),
                Effect::Scrobble { item, submission } => observer.scrobble(&item, submission),
                _ => {}
            }
        }
    }

    /// Start the player's thread, or wake it to look again.
    fn wake(&self) {
        let mut slot = self.player.thread.lock().unwrap();
        match slot.as_ref() {
            Some(thread) => thread.unpark(),
            None => {
                let weak = self.weak();
                let handle = std::thread::Builder::new()
                    .name("eko-player".into())
                    .spawn(move || conduct(weak))
                    .expect("the player thread starts");
                *slot = Some(handle.thread().clone());
            }
        }
    }
}

/// The player's own loop: tick while there is something to watch, park otherwise, and
/// end with the engine. It calls `run`, never `drive`: waking itself would spin.
fn conduct(weak: Weak<Inner>) {
    loop {
        let busy = {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let engine = Engine::from_inner(inner);
            engine.run(None);
            engine.session_view().exists || engine.player.core.lock().unwrap().has_deadline()
        };
        if busy {
            std::thread::park_timeout(TICK);
        } else {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal_path::ReplayGainTags;

    struct Files;

    impl SourceResolver for Files {
        fn resolve(&self, media: &ItemMedia) -> Option<Source> {
            match media {
                ItemMedia::File { path } => Some(Source::File(path.clone())),
                ItemMedia::Remote { .. } => None,
            }
        }
    }

    #[derive(Default)]
    struct Heard(Mutex<Vec<String>>);

    impl PlayerObserver for Heard {
        fn track_started(&self, item: &QueueItem) {
            self.0.lock().unwrap().push(format!("started {}", item.uid));
        }
        fn playback(&self, _item: Option<&QueueItem>, playing: bool, _pos_ms: u64) {
            self.0.lock().unwrap().push(format!("playing {playing}"));
        }
        fn stopped(&self, item: Option<&QueueItem>) {
            let uid = item.map_or("-", |i| i.uid.as_str());
            self.0.lock().unwrap().push(format!("stopped {uid}"));
        }
        fn scrobble(&self, item: &QueueItem, submission: bool) {
            self.0
                .lock()
                .unwrap()
                .push(format!("scrobble {} {submission}", item.uid));
        }
    }

    fn item(uid: &str, media: ItemMedia) -> QueueItem {
        QueueItem {
            uid: uid.into(),
            media,
            title: uid.to_uppercase(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 1_000,
            cover_url: String::new(),
            rg: ReplayGainTags::default(),
        }
    }

    /// Poll until `done`, for at most two seconds.
    fn wait_for(engine: &Engine, done: impl Fn(&PlayerSnapshot) -> bool) -> PlayerSnapshot {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let snap = engine.player_poll();
            if done(&snap) || Instant::now() > until {
                return snap;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_file_that_will_not_open_stops_the_player_with_its_name() {
        let engine = Engine::default();
        let heard = Arc::new(Heard::default());
        engine.set_resolver(Arc::new(Files));
        engine.set_observer(heard.clone());
        engine.queue_sync(vec![item(
            "a",
            ItemMedia::File {
                path: "/nonexistent/eko-queue-test.flac".into(),
            },
        )]);
        engine.player_play("a".into());
        let snap = wait_for(&engine, |s| s.error.is_some());
        assert_eq!(snap.error.as_deref(), Some("Couldn't play A"));
        assert!(!snap.active);
        let heard = heard.0.lock().unwrap();
        assert_eq!(heard.first().map(String::as_str), Some("started a"));
        assert!(heard.iter().any(|h| h == "stopped a"), "{heard:?}");
    }

    #[test]
    fn an_item_nothing_can_resolve_is_reported_the_same_way() {
        let engine = Engine::default();
        engine.set_resolver(Arc::new(Files));
        engine.queue_sync(vec![item(
            "r",
            ItemMedia::Remote {
                id: "1".into(),
                server: "https://music.example.com".into(),
            },
        )]);
        engine.player_play("r".into());
        let snap = wait_for(&engine, |s| s.error.is_some());
        assert_eq!(snap.error.as_deref(), Some("Couldn't play R"));
    }

    #[test]
    fn a_restored_queue_is_there_with_no_session_and_names_the_mini_player() {
        let engine = Engine::default();
        engine.queue_restore(
            vec![item(
                "a",
                ItemMedia::File {
                    path: "/x.flac".into(),
                },
            )],
            0,
            5_000,
        );
        let snap = engine.player_poll();
        assert_eq!(snap.uid.as_deref(), Some("a"));
        assert!(!snap.active);
        let np = engine.now_playing();
        assert_eq!(
            (np.title.as_str(), np.cover_path.as_str()),
            ("A", "/x.flac")
        );
        assert_eq!((np.index, np.total), (0, 1));
    }
}
