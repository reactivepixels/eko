//! The desktop app's side of engine-owned playback: how a queued item becomes a source,
//! and what macOS and the server hear when a track changes.
//!
//! The engine calls all of this from its own threads, so none of it depends on the
//! webview being awake. That is what keeps the lock screen, companion apps and scrobbles
//! right while the window is hidden.

use std::sync::Mutex;

use eko_core::engine::Source;
use eko_core::player::{ItemMedia, PlayerObserver, QueueItem, SourceResolver};
use tauri::{AppHandle, Manager};

use crate::commands::subsonic::SubsonicClient;

/// Resolves queued items: local files as they are, server tracks against the Pro offline
/// cache first, then the connected server.
pub struct AppResolver {
    app: AppHandle,
}

impl AppResolver {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl SourceResolver for AppResolver {
    fn resolve(&self, media: &ItemMedia) -> Option<Source> {
        match media {
            ItemMedia::File { path } => Some(Source::File(path.clone())),
            ItemMedia::Remote { id, server } => {
                #[cfg(feature = "pro")]
                if let Some(source) = cached(&self.app, id) {
                    return Some(source);
                }
                let client = self.app.state::<SubsonicClient>().current()?;
                stream_source(client.config(), id, server)
            }
        }
    }
}

/// A freshly signed stream URL for `id`, but only on the server the item came from: the
/// same id on another server is a different song, or none at all.
fn stream_source(cfg: &eko_net::Config, id: &str, server: &str) -> Option<Source> {
    if eko_net::urls::server_key(cfg) != server {
        return None;
    }
    Some(Source::Url(eko_net::urls::stream_src_url(
        cfg,
        id,
        &eko_net::auth::random_salt(),
    )))
}

/// A complete copy in the offline cache (Pro), exactly as `offlineEntry` chose one.
#[cfg(feature = "pro")]
fn cached(app: &AppHandle, id: &str) -> Option<Source> {
    let cache = app.try_state::<eko_core::pro::offline::OfflineCache>()?;
    let entry = cache.get(id).filter(|e| !e.partial)?;
    Some(Source::Cached {
        track_id: entry.track_id,
        plain_len: entry.bytes,
    })
}

/// Tells macOS (the Now Playing card, companion apps) and the server about playback.
pub struct AppObserver {
    app: AppHandle,
    /// The last companion broadcast, so exact repeats are skipped.
    last_broadcast: Mutex<Option<(String, String, String)>>,
    #[cfg(target_os = "macos")]
    activity: Mutex<Option<activity::Activity>>,
}

impl AppObserver {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            last_broadcast: Mutex::new(None),
            #[cfg(target_os = "macos")]
            activity: Mutex::new(None),
        }
    }

    fn broadcast(&self, state: &str, item: Option<&QueueItem>) {
        let next = (
            state.to_string(),
            item.map(|i| i.title.clone()).unwrap_or_default(),
            item.map(|i| i.artist.clone()).unwrap_or_default(),
        );
        let mut last = self.last_broadcast.lock().unwrap();
        if last.as_ref() == Some(&next) {
            return;
        }
        #[cfg(target_os = "macos")]
        crate::broadcast::post(&next.0, &next.1, &next.2);
        *last = Some(next);
    }

    /// While audio is going out, keep macOS from App Napping EKO, so the player's thread
    /// keeps its timing with every window hidden.
    fn hold_activity(&self, playing: bool) {
        #[cfg(target_os = "macos")]
        {
            let mut held = self.activity.lock().unwrap();
            if playing && held.is_none() {
                *held = Some(activity::Activity::begin());
            } else if !playing {
                if let Some(activity) = held.take() {
                    activity.end();
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = playing;
    }
}

impl PlayerObserver for AppObserver {
    fn track_started(&self, item: &QueueItem) {
        // No cover: souvlaki only survives a local file:// cover, and a queued item never
        // carries one (see `media::set_metadata`).
        #[cfg(target_os = "macos")]
        {
            crate::media::set_metadata(
                &self.app,
                item.title.clone(),
                item.artist.clone(),
                item.album.clone(),
                None,
                Some(item.duration_ms as f64 / 1000.0),
            );
            crate::media::set_playback(&self.app, true, 0.0);
        }
        self.broadcast("Playing", Some(item));
        self.hold_activity(true);
    }

    fn playback(&self, item: Option<&QueueItem>, playing: bool, pos_ms: u64) {
        #[cfg(target_os = "macos")]
        crate::media::set_playback(&self.app, playing, pos_ms as f64 / 1000.0);
        #[cfg(not(target_os = "macos"))]
        let _ = pos_ms;
        self.broadcast(if playing { "Playing" } else { "Paused" }, item);
        self.hold_activity(playing);
    }

    fn stopped(&self, item: Option<&QueueItem>) {
        #[cfg(target_os = "macos")]
        crate::media::set_stopped(&self.app);
        self.broadcast("Stopped", item);
        self.hold_activity(false);
    }

    fn scrobble(&self, item: &QueueItem, submission: bool) {
        let ItemMedia::Remote { id, .. } = &item.media else {
            return;
        };
        let Some(client) = self.app.state::<SubsonicClient>().current() else {
            return;
        };
        let id = id.clone();
        // A plain thread: the blocking client must never be called from inside the async
        // runtime (see `commands::subsonic`), and the player's thread must not wait on the
        // network.
        std::thread::spawn(move || {
            let _ = client.scrobble(&id, submission);
        });
    }
}

#[cfg(target_os = "macos")]
mod activity {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

    /// An `NSProcessInfo` activity. While one is held, macOS does not App Nap the app.
    pub struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

    // Safety: the token is an opaque, immutable Objective-C object that is only ever
    // handed back to `endActivity`, which is safe to call from any thread.
    unsafe impl Send for Activity {}

    impl Activity {
        pub fn begin() -> Self {
            let reason = NSString::from_str("Playing audio");
            Activity(
                NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                    NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                    &reason,
                ),
            )
        }

        pub fn end(self) {
            // Safety: `self.0` is the token `beginActivityWithOptions_reason` returned,
            // which is exactly what `endActivity` expects.
            unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> eko_net::Config {
        eko_net::Config {
            base_url: "https://music.example.com/".into(),
            username: "rod".into(),
            password: "hunter2".into(),
        }
    }

    #[test]
    fn a_remote_item_gets_a_fresh_stream_url_on_its_own_server() {
        let server = eko_net::urls::server_key(&cfg());
        let Some(Source::Url(first)) = stream_source(&cfg(), "tr-1", &server) else {
            panic!("no URL for an item on the connected server");
        };
        let Some(Source::Url(second)) = stream_source(&cfg(), "tr-1", &server) else {
            panic!("no URL for an item on the connected server");
        };
        assert!(first.contains("id=tr-1"));
        assert_ne!(first, second, "each start must sign with a fresh salt");
    }

    #[test]
    fn a_remote_item_from_another_server_is_unplayable() {
        assert!(stream_source(&cfg(), "tr-1", "https://other.example.com").is_none());
    }
}
