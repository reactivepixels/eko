//! Audio file metadata extraction via the `lofty` crate.
//! Used by the frontend to populate the track title, time display, and playlist rows.

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::ItemKey;
use lofty::probe::Probe;
use lofty::tag::Accessor;
use serde::Serialize;
use std::path::Path;

/// Tag + stream properties for a single audio file.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrackMetadata {
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    /// Track number within the album (for ordering/grouping).
    pub track: Option<u32>,
    /// Track duration in seconds.
    pub duration: f64,
    /// Average bitrate in kbps.
    pub bitrate: Option<u32>,
    /// Sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: Option<u8>,
    /// ReplayGain track gain in dB (from `REPLAYGAIN_TRACK_GAIN`), if tagged.
    pub rg_track_gain: Option<f32>,
    /// ReplayGain album gain in dB (from `REPLAYGAIN_ALBUM_GAIN`), if tagged.
    pub rg_album_gain: Option<f32>,
    /// ReplayGain track peak (linear, ~0..1), if tagged.
    pub rg_track_peak: Option<f32>,
    /// ReplayGain album peak (linear, ~0..1), if tagged.
    pub rg_album_peak: Option<f32>,
}

/// Parse a ReplayGain gain string into dB. Tolerant of the optional `dB` suffix
/// (any case), surrounding whitespace, and a leading `+` — e.g. `"-6.34 dB"`,
/// `"+3 dB"`, `"-6.34"`. Returns `None` for non-numeric input.
fn parse_rg_gain_db(s: &str) -> Option<f32> {
    let t = s.trim();
    let num = match t.to_ascii_lowercase().strip_suffix("db") {
        Some(prefix) => &t[..prefix.len()],
        None => t,
    };
    num.trim()
        .trim_start_matches('+')
        .trim()
        .parse::<f32>()
        .ok()
}

/// Read tag + stream metadata for one file. Falls back to a filename-derived
/// title if the file has no tags, so the UI always has something to show.
#[tauri::command]
pub fn read_metadata(path: String) -> Result<TrackMetadata, String> {
    let p = Path::new(&path);

    let tagged = Probe::open(p)
        .map_err(|e| format!("probe: {e}"))?
        .read()
        .map_err(|e| format!("read: {e}"))?;

    let properties = tagged.properties();
    let mut meta = TrackMetadata {
        path: path.clone(),
        duration: properties.duration().as_secs_f64(),
        bitrate: properties.audio_bitrate(),
        sample_rate: properties.sample_rate(),
        channels: properties.channels(),
        ..Default::default()
    };

    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        meta.title = tag.title().map(|s| s.to_string());
        meta.artist = tag.artist().map(|s| s.to_string());
        meta.album = tag.album().map(|s| s.to_string());
        meta.track = tag.track();
        meta.album_artist = tag.get_string(&ItemKey::AlbumArtist).map(|s| s.to_string());
        // Some files keep album artist only — fall back to it.
        if meta.artist.is_none() {
            meta.artist = meta.album_artist.clone();
        }
        // ReplayGain tags (used by the optional, off-by-default volume normalisation).
        meta.rg_track_gain = tag
            .get_string(&ItemKey::ReplayGainTrackGain)
            .and_then(parse_rg_gain_db);
        meta.rg_album_gain = tag
            .get_string(&ItemKey::ReplayGainAlbumGain)
            .and_then(parse_rg_gain_db);
        meta.rg_track_peak = tag
            .get_string(&ItemKey::ReplayGainTrackPeak)
            .and_then(|s| s.trim().parse::<f32>().ok());
        meta.rg_album_peak = tag
            .get_string(&ItemKey::ReplayGainAlbumPeak)
            .and_then(|s| s.trim().parse::<f32>().ok());
    }

    // Fallback title from the file stem.
    if meta.title.is_none() {
        meta.title = p
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
    }

    Ok(meta)
}

/// Decode image bytes, downscale to a ~320px thumbnail, re-encode as a JPEG data URI
/// (keeps cover payloads small so big libraries stay responsive).
fn encode_cover(bytes: &[u8]) -> Option<String> {
    use base64::Engine as _;
    let img = image::load_from_memory(bytes).ok()?;
    let thumb = img.thumbnail(320, 320);
    let mut out = Vec::new();
    thumb
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .ok()?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&out)
    ))
}

/// Cover art for a local file: embedded picture first, then a sidecar image
/// (cover.jpg / folder.jpg / …) in the same folder. Returns a thumbnail data URI.
#[tauri::command]
pub fn read_cover(path: String) -> Result<Option<String>, String> {
    // 1) embedded picture
    if let Ok(tagged) = Probe::open(&path).and_then(|p| p.read()) {
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(pic) = tag.pictures().first() {
                if let Some(uri) = encode_cover(pic.data()) {
                    return Ok(Some(uri));
                }
            }
        }
    }
    // 2) sidecar image in the track's folder
    if let Some(dir) = Path::new(&path).parent() {
        const NAMES: &[&str] = &["cover", "folder", "front", "album", "artwork", "albumart"];
        const EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];
        if let Ok(entries) = std::fs::read_dir(dir) {
            // collect candidate filenames (case-insensitive) once
            let files: Vec<(String, std::path::PathBuf)> = entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| (n.to_lowercase(), p.clone()))
                })
                .collect();
            for name in NAMES {
                for ext in EXTS {
                    let target = format!("{name}.{ext}");
                    if let Some((_, p)) = files.iter().find(|(n, _)| n == &target) {
                        if let Ok(bytes) = std::fs::read(p) {
                            if let Some(uri) = encode_cover(&bytes) {
                                return Ok(Some(uri));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

const AUDIO_EXTS: &[&str] = &[
    "flac", "mp3", "m4a", "aac", "wav", "aiff", "aif", "ogg", "opus", "wma", "alac",
];

fn walk(dir: &Path, out: &mut Vec<String>, depth: u32) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, depth + 1);
        } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            if AUDIO_EXTS.contains(&ext.to_lowercase().as_str()) {
                if let Some(s) = p.to_str() {
                    out.push(s.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_rg_gain_db;

    #[test]
    fn parses_replaygain_gain_strings() {
        assert_eq!(parse_rg_gain_db("-6.34 dB"), Some(-6.34));
        assert_eq!(parse_rg_gain_db("+3 dB"), Some(3.0));
        assert_eq!(parse_rg_gain_db("  -6.34dB "), Some(-6.34));
        assert_eq!(parse_rg_gain_db("0.00 dB"), Some(0.0));
        assert_eq!(parse_rg_gain_db("-6.34"), Some(-6.34)); // no suffix
        assert_eq!(parse_rg_gain_db("4.18 DB"), Some(4.18)); // upper-case suffix
        assert_eq!(parse_rg_gain_db("abc"), None);
        assert_eq!(parse_rg_gain_db(""), None);
    }
}

/// Recursively scan a folder for audio files and return their metadata (for the
/// Local source). Album grouping/ordering happens on the frontend from these tags.
#[tauri::command]
pub fn scan_music_folder(path: String) -> Result<Vec<TrackMetadata>, String> {
    let root = Path::new(&path);
    if !root.is_dir() {
        return Err("not a directory".into());
    }
    let mut files = Vec::new();
    walk(root, &mut files, 0);
    files.sort();
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        if let Ok(m) = read_metadata(f) {
            out.push(m);
        }
    }
    Ok(out)
}
