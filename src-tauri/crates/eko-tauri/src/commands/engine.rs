//! `#[tauri::command]` wrappers over `eko_core::engine`. See `commands` module docs —
//! unwrap the Tauri state and forward; the audio behaviour lives in `eko-core`.
//!
//! Every function name and every parameter name here is load-bearing: the name IS the
//! IPC command string and the parameter names ARE the JSON keys the TypeScript
//! frontend sends. Do not rename either.

use eko_core::engine::{self, Engine, EngineStatus, EqMode, NowPlaying};

/// Play a local file.
#[tauri::command]
pub fn engine_play(path: String, engine: tauri::State<Engine>) -> EngineStatus {
    engine.play(path)
}

/// Play a remote URL (Navidrome stream) — downloaded + decoded natively (bit-perfect).
#[tauri::command]
pub fn engine_play_url(url: String, engine: tauri::State<Engine>) -> EngineStatus {
    engine.play_url(url)
}

/// Play a cached offline track by Subsonic track ID (Pro only).
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_play_cached(
    track_id: String,
    plain_len: u64,
    engine: tauri::State<Engine>,
) -> EngineStatus {
    engine.play_cached(track_id, plain_len)
}

/// List available output devices (DACs) by name.
#[tauri::command]
pub fn engine_list_devices() -> Vec<String> {
    engine::list_devices()
}

/// Choose the output device by name (None / empty = system default).
#[tauri::command]
pub fn engine_set_device(name: Option<String>, engine: tauri::State<Engine>) {
    engine.set_device(name)
}

/// Pause playback.
#[tauri::command]
pub fn engine_pause(engine: tauri::State<Engine>) {
    engine.pause()
}

/// Resume playback from the current position.
#[tauri::command]
pub fn engine_resume(engine: tauri::State<Engine>) {
    engine.resume()
}

/// Seek to `secs` seconds from the beginning of the track.
#[tauri::command]
pub fn engine_seek(secs: f64, engine: tauri::State<Engine>) {
    engine.seek(secs)
}

/// Stop playback and tear down the current session.
#[tauri::command]
pub fn engine_stop(engine: tauri::State<Engine>) {
    engine.stop()
}

/// Update the 10-band graphic EQ parameters.
#[tauri::command]
pub fn engine_set_eq(enabled: bool, preamp: f64, gains: Vec<f32>, engine: tauri::State<Engine>) {
    engine.set_eq(enabled, preamp, gains)
}

/// Switch which EQ mode is routed to the DSP path.
#[tauri::command]
pub fn engine_set_eq_mode(mode: EqMode, engine: tauri::State<Engine>) {
    engine.set_eq_mode(mode)
}

/// Set the parametric EQ configuration (Pro only).
///
/// The Pro gate (defense-in-depth) stays here rather than in `eko-core`: it needs the
/// `AppHandle` to locate the licence file, so a frontend-only patch (forcing `useIsPro`)
/// can't enable the parametric EQ. Mirrors offline.rs.
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_set_param_eq(
    app: tauri::AppHandle,
    enabled: bool,
    preamp: f64,
    bands: Vec<eko_core::pro::param_eq::ParamBand>,
    engine: tauri::State<Engine>,
) -> Result<(), String> {
    if eko_core::pro::license::compute_status(crate::commands::license::config_dir(&app).as_deref())
        .tier
        == eko_core::pro::license::Tier::Free
    {
        return Err("EKO Pro is required for the parametric EQ.".to_string());
    }
    engine.set_param_eq(enabled, preamp, bands);
    Ok(())
}

/// Compute the parametric EQ frequency-response curve for the on-screen preview (Pro only).
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_eq_curve(bands: Vec<eko_core::pro::param_eq::ParamBand>, preamp: f64) -> Vec<f32> {
    engine::eq_curve(bands, preamp)
}

/// Parse an AutoEQ `ParametricEQ.txt` text and return the bands + preamp (Pro only).
///
/// The Pro gate stays here — see [`engine_set_param_eq`].
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_parse_autoeq(
    app: tauri::AppHandle,
    text: String,
) -> Result<serde_json::Value, String> {
    if eko_core::pro::license::compute_status(crate::commands::license::config_dir(&app).as_deref())
        .tier
        == eko_core::pro::license::Tier::Free
    {
        return Err("EKO Pro is required for AutoEQ import.".to_string());
    }
    engine::parse_autoeq(text)
}

/// Read an AutoEQ `ParametricEQ.txt` file at the given absolute path and parse it (Pro only).
///
/// The Pro gate stays here — see [`engine_set_param_eq`].
#[cfg(feature = "pro")]
#[tauri::command]
pub fn engine_import_autoeq_file(
    app: tauri::AppHandle,
    path: String,
) -> Result<serde_json::Value, String> {
    if eko_core::pro::license::compute_status(crate::commands::license::config_dir(&app).as_deref())
        .tier
        == eko_core::pro::license::Tier::Free
    {
        return Err("EKO Pro is required for AutoEQ import.".to_string());
    }
    engine::import_autoeq_file(path)
}

/// Set playback volume from the dial position (0..1).
#[tauri::command]
pub fn engine_set_volume(vol: f64, engine: tauri::State<Engine>) {
    engine.set_volume(vol)
}

/// Apply a ReplayGain adjustment, in dB, as an output gain (off by default).
#[tauri::command]
pub fn engine_set_replaygain(gain_db: Option<f32>, engine: tauri::State<Engine>) {
    engine.set_replaygain(gain_db)
}

/// Store the current track metadata for the mini player to read.
#[tauri::command]
pub fn engine_set_now_playing(np: NowPlaying, engine: tauri::State<Engine>) {
    engine.set_now_playing(np)
}

/// Read the current track metadata (used by the mini-player window).
#[tauri::command]
pub fn engine_now_playing(engine: tauri::State<Engine>) -> NowPlaying {
    engine.now_playing()
}

/// Return the latest 32-band spectrum magnitudes (0.0–1.0, log-spaced).
#[tauri::command]
pub fn engine_bands(engine: tauri::State<Engine>) -> Vec<f32> {
    engine.bands()
}

/// Return an [`EngineStatus`] snapshot for the currently-active session.
#[tauri::command]
pub fn engine_status(engine: tauri::State<Engine>) -> Option<EngineStatus> {
    engine.status()
}
