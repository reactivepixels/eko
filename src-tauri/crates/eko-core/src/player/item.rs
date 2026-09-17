use crate::engine::Source;
use crate::signal_path::{ReplayGainTags, SealRgDb};

/// Where a queued item's audio comes from.
///
/// A remote item is an id and the server it belongs to, **never a URL**. A signed
/// Subsonic URL carries `u`, `t` and `s` and replays as the user, so a queue of them
/// would be a queue of credentials. The URL is minted when the item starts.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ItemMedia {
    /// A local file, by absolute path.
    File { path: String },
    /// A track on a Subsonic server.
    Remote {
        /// The server's track id.
        id: String,
        /// `eko_net::urls::server_key` of the server the track was listed by.
        server: String,
    },
}

/// One slot in the play queue.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    /// Unique per slot. The same song queued twice has two uids.
    pub uid: String,
    pub media: ItemMedia,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    /// Length from the tags or the server. The decoded length wins once the engine has one.
    #[serde(default)]
    pub duration_ms: u64,
    /// Size-agnostic `stream://` cover URL for a server track; empty for a local file.
    #[serde(default)]
    pub cover_url: String,
    #[serde(default)]
    pub rg: ReplayGainTags,
}

impl QueueItem {
    /// Whether this item streams from a server (and so can be scrobbled).
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self.media, ItemMedia::Remote { .. })
    }
}

/// Tracks shorter than this are never scrobbled.
pub const SCROBBLE_MIN_MS: u64 = 30_000;
/// A play is submitted after this much, however long the track.
pub const SCROBBLE_MAX_MS: u64 = 240_000;

/// Elapsed ms at which a play is submitted as a scrobble: half the track or four
/// minutes, whichever comes first (the Last.fm convention). `None` for a track too
/// short to scrobble. Ported from `scrobbleThreshold` in `usePlayerStore.ts`.
#[must_use]
pub fn scrobble_threshold_ms(duration_ms: u64) -> Option<u64> {
    (duration_ms >= SCROBBLE_MIN_MS).then(|| (duration_ms / 2).min(SCROBBLE_MAX_MS))
}

/// Turns a queued item into something the decoder can open.
pub trait SourceResolver: Send + Sync {
    /// `None` when the item can't be played right now: no connection, or a different
    /// server from the one it was queued from.
    fn resolve(&self, media: &ItemMedia) -> Option<Source>;
}

/// Hears about the playback changes the OS and the server need to know about.
///
/// Called from whichever thread drove the player (a command, or the player's own
/// thread), and never while the player's lock is held.
pub trait PlayerObserver: Send + Sync {
    /// A new item started, by any route: a press, auto-advance, or a gapless seam.
    fn track_started(&self, item: &QueueItem);
    /// Playback paused, resumed or jumped.
    fn playback(&self, item: Option<&QueueItem>, playing: bool, pos_ms: u64);
    /// Playback stopped: the end of the queue, a stop, or a failure.
    fn stopped(&self, item: Option<&QueueItem>);
    /// Tell the server about a play: "now playing", or at the threshold, a submission.
    fn scrobble(&self, item: &QueueItem, submission: bool);
}

/// What the frontend reads on each poll.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    /// The playing item, or the queue's position when nothing is playing.
    pub uid: Option<String>,
    /// Where `uid` sits in the queue, if it is still queued.
    pub index: Option<u32>,
    /// A session exists.
    pub active: bool,
    /// Audio is going out: a session exists and is neither paused nor finished.
    pub playing: bool,
    /// Why playback last stopped on its own, when that was a failure.
    pub error: Option<String>,
    /// The ReplayGain applied to the playing item, as the engine received it.
    pub rg_engine_db: Option<f64>,
    /// The same, dead-banded, as the seal must report it.
    pub rg_seal_db: SealRgDb,
    /// The sleep timer is waiting for the end of this track.
    pub stop_after_current: bool,
    /// Time left on a fixed sleep timer.
    pub sleep_remaining_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_item_travels_as_an_id_and_a_server_never_a_url() {
        let json = r#"{"uid":"q1","media":{"kind":"remote","id":"tr-9","server":"https://music.example.com"},"title":"T"}"#;
        let item: QueueItem = serde_json::from_str(json).unwrap();
        assert_eq!(
            item.media,
            ItemMedia::Remote {
                id: "tr-9".into(),
                server: "https://music.example.com".into()
            }
        );
        assert!(item.is_remote());
        assert_eq!(item.duration_ms, 0);
        assert_eq!(item.rg, ReplayGainTags::default());
    }

    #[test]
    fn a_file_item_carries_its_path_and_its_replaygain_tags() {
        let json = r#"{"uid":"q2","media":{"kind":"file","path":"/m/a.flac"},"durationMs":1000,"rg":{"trackGain":-6.5,"trackPeak":null,"albumGain":null,"albumPeak":null}}"#;
        let item: QueueItem = serde_json::from_str(json).unwrap();
        assert_eq!(
            item.media,
            ItemMedia::File {
                path: "/m/a.flac".into()
            }
        );
        assert!(!item.is_remote());
        assert_eq!(item.duration_ms, 1000);
        assert_eq!(item.rg.track_gain, Some(-6.5));
    }

    #[test]
    fn scrobble_threshold_is_half_the_track_or_four_minutes() {
        assert_eq!(scrobble_threshold_ms(120_000), Some(60_000));
        assert_eq!(scrobble_threshold_ms(180_000), Some(90_000));
        assert_eq!(scrobble_threshold_ms(480_000), Some(SCROBBLE_MAX_MS));
        assert_eq!(scrobble_threshold_ms(1_200_000), Some(240_000));
        assert_eq!(scrobble_threshold_ms(60_000), Some(30_000));
    }

    #[test]
    fn a_track_shorter_than_thirty_seconds_never_scrobbles() {
        assert_eq!(SCROBBLE_MIN_MS, 30_000);
        assert_eq!(scrobble_threshold_ms(30_000), Some(15_000));
        assert_eq!(scrobble_threshold_ms(29_999), None);
        assert_eq!(scrobble_threshold_ms(0), None);
    }

    #[test]
    fn the_snapshot_speaks_camel_case_with_a_plain_nullable_seal_db() {
        let v = serde_json::to_value(PlayerSnapshot::default()).unwrap();
        for key in [
            "uid",
            "index",
            "active",
            "playing",
            "error",
            "rgEngineDb",
            "rgSealDb",
            "stopAfterCurrent",
            "sleepRemainingMs",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert!(v["rgSealDb"].is_null());
    }
}
