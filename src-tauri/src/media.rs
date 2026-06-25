//! System "Now Playing" + hardware media keys (macOS).
//!
//! Bridges EKO's transport to macOS `MPNowPlayingInfoCenter` (the lock-screen /
//! Control-Center now-playing card) and `MPRemoteCommandCenter` (the F7/F8/F9 hardware
//! keys and headphone controls) via the `souvlaki` crate.
//!
//! Remote commands are translated into the same `eko:cmd` events the mini player already
//! emits, so the frontend's existing listener (App.tsx) drives playback — one command
//! path for the mini window and the OS alike.
//!
//! Threading: souvlaki's macOS controls are bound to the main thread + its run loop, so
//! the object is created in `init` (called from Tauri's `setup`, on the main thread) and
//! every later mutation is marshaled back onto the main thread with `run_on_main_thread`.
//! `SendControls` carries the non-Send handle into shared state; it is only ever touched
//! on the main thread, which upholds the safety contract.

use std::sync::Mutex;
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use tauri::{AppHandle, Emitter, Manager};

struct SendControls(MediaControls);
// Safety: the handle is constructed on the main thread and every access below is funneled
// through `run_on_main_thread`, so it is never actually touched off the main thread.
unsafe impl Send for SendControls {}

#[derive(Default)]
pub struct Media(Mutex<Option<SendControls>>);

/// Create the system media controls and wire remote commands → `eko:cmd`. Must run on the
/// main thread (it is — called from `setup`). A failure here is non-fatal: EKO just runs
/// without OS now-playing integration.
pub fn init(app: &AppHandle) {
    let config = PlatformConfig {
        dbus_name: "eko",
        display_name: "EKO",
        hwnd: None,
    };
    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(_) => return,
    };

    let ah = app.clone();
    let attached = controls.attach(move |event: MediaControlEvent| {
        let (action, value): (&str, Option<f64>) = match event {
            MediaControlEvent::Play => ("play", None),
            MediaControlEvent::Pause => ("pause", None),
            MediaControlEvent::Toggle => ("toggle", None),
            MediaControlEvent::Next => ("next", None),
            MediaControlEvent::Previous => ("prev", None),
            MediaControlEvent::Stop => ("stop", None),
            MediaControlEvent::SetPosition(MediaPosition(pos)) => ("seek", Some(pos.as_secs_f64())),
            _ => return,
        };
        let _ = ah.emit(
            "eko:cmd",
            serde_json::json!({ "action": action, "value": value }),
        );
    });
    if attached.is_err() {
        return;
    }

    *app.state::<Media>().0.lock().unwrap() = Some(SendControls(controls));
}

/// Update the now-playing card's metadata (title / artist / album / artwork / duration).
///
/// `cover_url` is honoured only when it's a local `file://` URL. souvlaki 0.7 loads the
/// artwork via `NSImage initWithContentsOfURL:` and dereferences the result with **no
/// nil-check**, so any URL macOS can't load — a webview-only `stream://` scheme, an
/// ATS-blocked http host, a 404, a non-image body — yields nil and `abort()`s the whole
/// process. We therefore drop every non-`file://` cover here (server art still renders
/// inside the app; it's just absent from the OS now-playing widget). Local files pass
/// `None` because their art is embedded.
pub fn set_metadata(
    app: &AppHandle,
    title: String,
    artist: String,
    album: String,
    cover_url: Option<String>,
    duration: Option<f64>,
) {
    // Only a local file:// URL is safe to hand souvlaki (see the doc comment): every other
    // scheme can resolve to a nil NSImage and crash the app via its missing nil-check.
    let cover_url =
        cover_url.filter(|u| u.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("file:")));
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        if let Some(c) = app.state::<Media>().0.lock().unwrap().as_mut() {
            let _ = c.0.set_metadata(MediaMetadata {
                title: Some(&title),
                artist: Some(&artist),
                album: Some(&album),
                cover_url: cover_url.as_deref(),
                duration: duration.map(Duration::from_secs_f64),
            });
        }
    });
}

/// Update the now-playing playback state (play/pause) and elapsed position. macOS
/// extrapolates the running clock from this, so it only needs pushing on transitions.
pub fn set_playback(app: &AppHandle, playing: bool, elapsed: f64) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        if let Some(c) = app.state::<Media>().0.lock().unwrap().as_mut() {
            let progress = Some(MediaPosition(Duration::from_secs_f64(elapsed.max(0.0))));
            let pb = if playing {
                MediaPlayback::Playing { progress }
            } else {
                MediaPlayback::Paused { progress }
            };
            let _ = c.0.set_playback(pb);
        }
    });
}

/// Clear the now-playing card (transport stopped / queue cleared).
pub fn set_stopped(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        if let Some(c) = app.state::<Media>().0.lock().unwrap().as_mut() {
            let _ = c.0.set_playback(MediaPlayback::Stopped);
        }
    });
}
