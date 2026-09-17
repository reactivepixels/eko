//! Engine-owned playback: the queue, what plays next, and what has to happen when a
//! track changes.
//!
//! Nothing in here waits on a UI, and that is the point. The desktop app's webview is
//! throttled, and can be suspended outright, whenever its window is hidden. When track
//! advance lived there, playback stopped at the end of each track until the window
//! came back (reactivepixels/eko#6).

pub(crate) mod driver;
mod item;
mod policy;

pub use item::{
    scrobble_threshold_ms, ItemMedia, PlayerObserver, PlayerSnapshot, QueueItem, SourceResolver,
    SCROBBLE_MAX_MS, SCROBBLE_MIN_MS,
};
pub use policy::{Command, Effect, EndReason, PlayerCore, SessionView};
