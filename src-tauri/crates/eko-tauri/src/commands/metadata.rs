//! `#[tauri::command]` wrappers over `eko_core::metadata`. See `commands` module docs —
//! no logic here, only forwarding.

use eko_core::metadata;

#[tauri::command]
pub fn read_metadata(path: String) -> Result<metadata::TrackMetadata, String> {
    metadata::read_metadata(path)
}

#[tauri::command]
pub fn scan_music_folder(path: String) -> Result<Vec<metadata::TrackMetadata>, String> {
    metadata::scan_music_folder(path)
}

#[tauri::command]
pub fn read_cover(path: String) -> Result<Option<String>, String> {
    metadata::read_cover(path)
}
