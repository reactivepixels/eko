//! The local library: a folder of audio files, grouped into albums.
//!
//! ## A deliberate twin
//!
//! [`group`] is a **deliberate reimplementation** of the desktop app's
//! `group()` in `src/local/useLocal.ts` — not a port of it, and not a shared
//! module. The two frontends must agree on what an album *is*, so the rules are
//! copied out one for one and each is named in a comment beside the code that
//! implements it. What is not copied is the store around it: no zustand, no
//! `Track` shape from `src/types.ts`, no cover fetching. The GUI code is read,
//! never imported.
//!
//! The rules, from `useLocal.ts`:
//!
//! | rule | `useLocal.ts` | here |
//! |---|---|---|
//! | album name | `s.album?.trim() \|\| "Unknown Album"` | [`album_name`] |
//! | album artist | `(s.albumArtist \|\| s.artist \|\| "Unknown Artist").trim()` | [`album_artist`] |
//! | grouping key / id | `` `${artist} ${albumName}` `` | [`group`] |
//! | track order | by `s.track ?? 9999`, stable | [`group`] |
//! | album order | `artist.localeCompare` then `name.localeCompare` | [`locale_cmp`] |
//!
//! If either side changes, the other is wrong. There is a test here for every
//! row of that table.
//!
//! ## Scanning
//!
//! `eko_core::metadata::scan_music_folder` is the one-shot the GUI calls over
//! IPC: it walks the tree *and* reads every tag in a single blocking call that
//! reports nothing until it is finished. A terminal has no spinner window to
//! put in front of that, so [`scan`] performs the same two phases separately —
//! a walk, then `eko_core::metadata::read_metadata` per file — and reports
//! progress between them. The file-extension list and the depth cap are matched
//! to `eko-core`'s deliberately; see [`AUDIO_EXTS`].
//!
//! [`spawn`] runs all of that on a worker thread. **Nothing in this module may
//! be called from the render thread** — a cold NFS mount can take minutes, and
//! the Deck has to stay responsive across the whole of it.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eko_core::metadata::{self, TrackMetadata};

/// Extensions treated as audio.
///
/// Mirrors [`metadata::AUDIO_EXTS`], and
/// [`the_extension_list_matches_eko_cores`] asserts that against the real
/// constant — not against a copy of it — so a format added on either side fails
/// the build until both agree.
///
/// [`the_extension_list_matches_eko_cores`]: tests::the_extension_list_matches_eko_cores
pub const AUDIO_EXTS: &[&str] = &[
    "flac", "mp3", "m4a", "aac", "wav", "aiff", "aif", "ogg", "opus", "wma", "alac",
];

/// Directory depth cap, matching `eko-core`'s walk. Deep enough for
/// `Artist/Album/Disc 1`, shallow enough that a symlink loop cannot hang the
/// scan forever.
///
/// Public because the empty state quotes it: "nothing here, and I looked eight
/// folders deep" is a different claim from "nothing here", and the number has
/// to come from the walk rather than from a sentence someone typed.
pub const MAX_DEPTH: u32 = 8;

/// How often the scan is allowed to report progress.
///
/// Every file would be thousands of redraws a second on a large library, which
/// is the render-thread stall this module exists to avoid — just moved into the
/// channel.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(80);

// ── The model ────────────────────────────────────────────────────────────────

/// One playable file.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub path: String,
    pub title: String,
    pub artist: String,
    /// Track number within the album, when tagged.
    pub track_no: Option<u32>,
    /// Duration in seconds, `0.0` when unknown.
    pub duration: f64,
}

/// Tracks that share an album artist and an album name.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    /// `"{artist} {name}"` — the same key `useLocal.ts` uses as its album id.
    pub id: String,
    pub name: String,
    pub artist: String,
    pub tracks: Vec<Track>,
}

/// A scanned folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    /// The folder that was scanned. `None` before any scan.
    pub root: Option<PathBuf>,
    pub albums: Vec<Album>,
}

impl Library {
    /// The scanned folder's own name — `Music` for `~/Music` — for headings.
    #[must_use]
    pub fn root_name(&self) -> Option<&str> {
        self.root
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.albums.is_empty()
    }

    #[must_use]
    pub fn track_count(&self) -> usize {
        self.albums.iter().map(|a| a.tracks.len()).sum()
    }
}

// ── Progress ─────────────────────────────────────────────────────────────────

/// What a running scan reports back to the fold.
///
/// `Finished` with an empty [`Library`] is the **normal** outcome for a folder
/// with no audio in it. It is not `Failed`: "you pointed me at a folder with no
/// music in it" is a thing the user did, not a thing that went wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanEvent {
    /// Still walking the tree. `found` files so far.
    Discovering { found: usize },
    /// Reading tags — `done` of `total` files.
    Reading { done: usize, total: usize },
    /// The scan completed. The library may be empty.
    Finished(Box<Library>),
    /// The root is not there. Separate from [`ScanEvent::Failed`] because it
    /// has one cause and one fix, and the empty state can name both.
    Missing(PathBuf),
    /// The root could not be scanned at all.
    Failed(String),
}

/// Scan `root` on a worker thread, reporting through `emit`.
///
/// `emit` returns `false` once the fold has hung up; the scan then abandons
/// itself rather than finishing work nobody will read. The thread is detached:
/// it holds nothing that needs dropping in order, and the process exits when
/// the fold does.
pub fn spawn<F>(root: PathBuf, emit: F)
where
    F: Fn(ScanEvent) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        let event = scan(&root, &emit);
        emit(event);
    });
}

/// Walk, read and group, calling `progress` as it goes.
///
/// Separated from [`spawn`] so it can be driven synchronously in a test with a
/// recording `progress` closure.
pub fn scan<F>(root: &Path, progress: &F) -> ScanEvent
where
    F: Fn(ScanEvent) -> bool,
{
    // The `is_dir` stat happens here, on the worker, and not where the scan is
    // started: a configured folder can be an unmounted network share, and one
    // cold `stat` on the render thread is a frozen Deck.
    if !root.is_dir() {
        return ScanEvent::Missing(root.to_path_buf());
    }

    let mut files = Vec::new();
    let mut throttle = Throttle::new();
    let mut alive = true;
    walk(root, &mut files, 0, &mut |found| {
        if throttle.ready() {
            alive = progress(ScanEvent::Discovering { found });
        }
        alive
    });
    if !alive {
        return ScanEvent::Failed("cancelled".into());
    }
    // `scan_music_folder` sorts the paths before reading, so a folder full of
    // untagged files still comes out in a sensible order. Same here.
    files.sort();

    let total = files.len();
    let mut scanned = Vec::with_capacity(total);
    let mut throttle = Throttle::new();
    for (i, path) in files.into_iter().enumerate() {
        // `read_metadata` takes an owned String and fails per file; a single
        // unreadable file must not abort a 40,000-track scan.
        if let Ok(meta) = metadata::read_metadata(path) {
            scanned.push(meta);
        }
        if throttle.ready() && !progress(ScanEvent::Reading { done: i + 1, total }) {
            return ScanEvent::Failed("cancelled".into());
        }
    }

    ScanEvent::Finished(Box::new(Library {
        root: Some(root.to_path_buf()),
        albums: group(scanned),
    }))
}

/// Rate limiter for progress reports. See [`PROGRESS_INTERVAL`].
struct Throttle(Instant);

impl Throttle {
    fn new() -> Self {
        // Start "due" so the first file reports immediately and the UI leaves
        // its blank state at once.
        Self(Instant::now() - PROGRESS_INTERVAL)
    }

    fn ready(&mut self) -> bool {
        if self.0.elapsed() >= PROGRESS_INTERVAL {
            self.0 = Instant::now();
            true
        } else {
            false
        }
    }
}

// ── The walk ─────────────────────────────────────────────────────────────────

/// Is this an audio file by extension? Case-insensitive, like `eko-core`'s.
#[must_use]
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| AUDIO_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// `on_found` is called after every file added; returning `false` stops the
/// walk.
fn walk(dir: &Path, out: &mut Vec<String>, depth: u32, on_found: &mut dyn FnMut(usize) -> bool) {
    if depth > MAX_DEPTH {
        return;
    }
    // An unreadable directory is skipped, not fatal — a permissions hole in one
    // corner of a library should not cost you the rest of it.
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out, depth + 1, on_found);
        } else if is_audio_file(&path) {
            if let Some(s) = path.to_str() {
                out.push(s.to_string());
                if !on_found(out.len()) {
                    return;
                }
            }
        }
    }
}

// ── Grouping — the twin of `useLocal.ts` ─────────────────────────────────────

/// `s.album?.trim() || "Unknown Album"`.
fn album_name(meta: &TrackMetadata) -> String {
    match meta.album.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "Unknown Album".to_string(),
    }
}

/// `(s.albumArtist || s.artist || "Unknown Artist").trim()`.
///
/// One deliberate divergence: JavaScript's `||` trims *after* choosing, so a
/// tag of `"   "` is truthy there and yields an album artist of `""`. An empty
/// artist would render as a blank column and group every such album together,
/// so a whitespace-only tag falls through here as if it were absent.
fn album_artist(meta: &TrackMetadata) -> String {
    for candidate in [&meta.album_artist, &meta.artist] {
        if let Some(value) = candidate.as_deref() {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "Unknown Artist".to_string()
}

/// Approximate `String.prototype.localeCompare` in the default locale.
///
/// `localeCompare` is case- and accent-insensitive at its primary strength, so
/// `"abba"` sorts before `"Beatles"`. Rust's `Ord for str` is codepoint order,
/// which would file every capitalised artist ahead of every lower-case one and
/// visibly disagree with the desktop app's list. Comparing lower-cased forms is
/// the closest match available without an ICU dependency; the raw comparison
/// breaks ties so the order stays total and deterministic.
fn locale_cmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Group scanned tracks into albums. The twin of `group()` in `useLocal.ts`.
#[must_use]
pub fn group(scanned: Vec<TrackMetadata>) -> Vec<Album> {
    // Insertion-ordered so the grouping is deterministic before the final sort,
    // exactly as a JS `Map` is.
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, (Album, Vec<u32>)> =
        std::collections::HashMap::new();

    for meta in scanned {
        let name = album_name(&meta);
        let artist = album_artist(&meta);
        // `${artist} ${albumName}` — the album id on both sides.
        let key = format!("{artist} {name}");
        let entry = groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            (
                Album {
                    id: key.clone(),
                    name,
                    artist: artist.clone(),
                    tracks: Vec::new(),
                },
                Vec::new(),
            )
        });
        entry.0.tracks.push(Track {
            title: meta
                .title
                .clone()
                .unwrap_or_else(|| meta.path.clone())
                .trim()
                .to_string(),
            artist: meta.artist.clone().unwrap_or_else(|| artist.clone()),
            track_no: meta.track,
            duration: meta.duration,
            path: meta.path,
        });
        // `s.track ?? 9999` — untagged tracks sink to the bottom in file order.
        entry.1.push(meta.track.unwrap_or(9999));
    }

    let mut albums: Vec<Album> = order
        .into_iter()
        .filter_map(|key| groups.remove(&key))
        .map(|(mut album, numbers)| {
            // A stable sort by track number, matching JS's stable `Array.sort`:
            // two files tagged `1` keep the order the walk found them in.
            let mut rows: Vec<(u32, Track)> = numbers.into_iter().zip(album.tracks).collect();
            rows.sort_by_key(|(n, _)| *n);
            album.tracks = rows.into_iter().map(|(_, t)| t).collect();
            album
        })
        .collect();

    albums.sort_by(|a, b| {
        locale_cmp(&a.artist, &b.artist).then_with(|| locale_cmp(&a.name, &b.name))
    });
    albums
}

/// `m:ss`, or `—` when the duration is unknown or nonsensical.
#[must_use]
pub fn format_duration(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.5 {
        return "—".to_string();
    }
    let total = secs.round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(
        path: &str,
        album: Option<&str>,
        artist: Option<&str>,
        track: Option<u32>,
    ) -> TrackMetadata {
        TrackMetadata {
            path: path.to_string(),
            title: Some(path.to_string()),
            artist: artist.map(str::to_string),
            album: album.map(str::to_string),
            track,
            ..Default::default()
        }
    }

    // ── the grouping twin ────────────────────────────────────────────────

    #[test]
    fn an_untagged_album_becomes_unknown_album_by_unknown_artist() {
        let albums = group(vec![meta("/a.flac", None, None, None)]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name, "Unknown Album");
        assert_eq!(albums[0].artist, "Unknown Artist");
        // The id is `${artist} ${albumName}` on both sides of the twin.
        assert_eq!(albums[0].id, "Unknown Artist Unknown Album");
    }

    #[test]
    fn a_blank_or_whitespace_album_tag_is_treated_as_absent() {
        let albums = group(vec![
            meta("/a.flac", Some("   "), Some("Boards of Canada"), None),
            meta("/b.flac", Some(""), Some("Boards of Canada"), None),
        ]);
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name, "Unknown Album");
    }

    #[test]
    fn album_artist_wins_over_track_artist() {
        let mut m = meta("/a.flac", Some("Blue"), Some("Guest Vocalist"), Some(1));
        m.album_artist = Some("The Band".into());
        let albums = group(vec![m]);
        assert_eq!(albums[0].artist, "The Band");
        // …but the track keeps its own artist, so a compilation still reads right.
        assert_eq!(albums[0].tracks[0].artist, "Guest Vocalist");
    }

    #[test]
    fn a_whitespace_only_album_artist_falls_through_to_the_track_artist() {
        // The one deliberate divergence from `useLocal.ts` — see `album_artist`.
        let mut m = meta("/a.flac", Some("Blue"), Some("Joni Mitchell"), Some(1));
        m.album_artist = Some("   ".into());
        assert_eq!(group(vec![m])[0].artist, "Joni Mitchell");
    }

    #[test]
    fn albums_split_on_artist_as_well_as_name() {
        // Two different "Greatest Hits" are two albums, not one.
        let albums = group(vec![
            meta("/a.flac", Some("Greatest Hits"), Some("Queen"), Some(1)),
            meta("/b.flac", Some("Greatest Hits"), Some("ABBA"), Some(1)),
        ]);
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn tracks_order_by_track_number_and_untagged_ones_sink() {
        let albums = group(vec![
            meta("/c.flac", Some("Kid A"), Some("Radiohead"), None),
            meta("/b.flac", Some("Kid A"), Some("Radiohead"), Some(2)),
            meta("/a.flac", Some("Kid A"), Some("Radiohead"), Some(1)),
        ]);
        let paths: Vec<&str> = albums[0].tracks.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, vec!["/a.flac", "/b.flac", "/c.flac"]);
    }

    #[test]
    fn equal_track_numbers_keep_their_input_order() {
        let albums = group(vec![
            meta("/first.flac", Some("X"), Some("Y"), Some(1)),
            meta("/second.flac", Some("X"), Some("Y"), Some(1)),
        ]);
        let paths: Vec<&str> = albums[0].tracks.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, vec!["/first.flac", "/second.flac"]);
    }

    #[test]
    fn albums_sort_by_artist_then_name_case_insensitively() {
        // Codepoint order would put "Zola" before "abba"; localeCompare does not.
        let albums = group(vec![
            meta("/1.flac", Some("Voyage"), Some("abba"), Some(1)),
            meta("/2.flac", Some("Aftermath"), Some("Zola"), Some(1)),
            meta("/3.flac", Some("Arrival"), Some("abba"), Some(1)),
        ]);
        let order: Vec<&str> = albums.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(order, vec!["Arrival", "Voyage", "Aftermath"]);
    }

    #[test]
    fn a_missing_title_falls_back_to_the_path() {
        let mut m = meta("/deep/track.flac", Some("X"), Some("Y"), Some(1));
        m.title = None;
        assert_eq!(group(vec![m])[0].tracks[0].title, "/deep/track.flac");
    }

    // ── the walk ─────────────────────────────────────────────────────────

    /// If `eko-core`'s `AUDIO_EXTS` gains a format, this list has to as well —
    /// otherwise the CLI silently cannot see files the GUI can.
    ///
    /// This compares against **`eko-core`'s constant**, not against a literal.
    /// It used to do the latter, which made it tautological: the list it checked
    /// and the list it checked *against* were both written here, so `eko-core`
    /// could gain a format and the assertion would still pass. Verified by
    /// adding `"dsf"` to `eko-core`'s list and watching this fail.
    #[test]
    fn the_extension_list_matches_eko_cores() {
        assert_eq!(
            AUDIO_EXTS,
            metadata::AUDIO_EXTS,
            "eko-cli's audio extensions have drifted from eko-core's"
        );
    }

    #[test]
    fn audio_files_are_recognised_case_insensitively() {
        assert!(is_audio_file(Path::new("/a/b.FLAC")));
        assert!(is_audio_file(Path::new("/a/b.Mp3")));
        assert!(!is_audio_file(Path::new("/a/cover.jpg")));
        assert!(!is_audio_file(Path::new("/a/README")));
    }

    #[test]
    fn a_folder_with_no_audio_scans_to_an_empty_library_not_an_error() {
        let dir = tempdir("eko-cli-empty");
        std::fs::write(dir.join("notes.txt"), "no music here").unwrap();
        std::fs::write(dir.join("cover.jpg"), [0u8; 4]).unwrap();

        let event = scan(&dir, &|_| true);
        match event {
            ScanEvent::Finished(lib) => {
                assert!(lib.is_empty());
                assert_eq!(lib.track_count(), 0);
                assert_eq!(lib.root_name(), Some("eko-cli-empty"));
            }
            other => panic!("expected an empty Finished, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A root that is not there reports as *missing*, by name — not as a
    /// generic failure. The path is what the user has to fix, so the path is
    /// what the event carries.
    #[test]
    fn a_root_that_is_not_a_folder_reports_missing_by_name() {
        let event = scan(Path::new("/definitely/not/here"), &|_| true);
        assert_eq!(
            event,
            ScanEvent::Missing(PathBuf::from("/definitely/not/here")),
            "expected Missing, got {event:?}"
        );
    }

    /// A file is not a folder either, and says so the same way.
    #[test]
    fn a_root_that_is_a_file_is_missing_rather_than_scanned() {
        let dir = tempdir("eko-cli-not-a-dir");
        let file = dir.join("music.flac");
        std::fs::write(&file, [0u8; 4]).unwrap();
        assert_eq!(scan(&file, &|_| true), ScanEvent::Missing(file.clone()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_walk_descends_and_ignores_non_audio() {
        let dir = tempdir("eko-cli-walk");
        let nested = dir.join("Artist").join("Album");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("01.flac"), [0u8; 4]).unwrap();
        std::fs::write(nested.join("cover.jpg"), [0u8; 4]).unwrap();
        std::fs::write(dir.join("loose.mp3"), [0u8; 4]).unwrap();

        let mut found = Vec::new();
        walk(&dir, &mut found, 0, &mut |_| true);
        found.sort();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].ends_with("01.flac"), "{found:?}");
        assert!(found[1].ends_with("loose.mp3"), "{found:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_scan_reports_progress_before_it_finishes() {
        let dir = tempdir("eko-cli-progress");
        // Not decodable — `read_metadata` fails on each, which is the point:
        // the scan must still walk, still report, and still finish.
        for i in 0..3 {
            std::fs::write(dir.join(format!("{i}.flac")), [0u8; 4]).unwrap();
        }
        let seen = std::sync::Mutex::new(Vec::new());
        let event = scan(&dir, &|e| {
            seen.lock().unwrap().push(e);
            true
        });
        let seen = seen.into_inner().unwrap();
        assert!(
            seen.iter()
                .any(|e| matches!(e, ScanEvent::Discovering { .. })),
            "{seen:?}"
        );
        assert!(matches!(event, ScanEvent::Finished(_)), "{event:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_hung_up_fold_cancels_the_scan_instead_of_finishing_it() {
        let dir = tempdir("eko-cli-cancel");
        std::fs::write(dir.join("a.flac"), [0u8; 4]).unwrap();
        let event = scan(&dir, &|_| false);
        assert!(matches!(event, ScanEvent::Failed(_)), "{event:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── formatting ───────────────────────────────────────────────────────

    #[test]
    fn durations_format_as_minutes_and_seconds() {
        assert_eq!(format_duration(0.0), "—");
        assert_eq!(format_duration(f64::NAN), "—");
        assert_eq!(format_duration(-4.0), "—");
        assert_eq!(format_duration(61.0), "1:01");
        assert_eq!(format_duration(291.4), "4:51");
        assert_eq!(format_duration(3600.0), "60:00");
    }

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
