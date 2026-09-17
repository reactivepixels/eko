//! `#[tauri::command]` wrappers over the engine-owned player (`eko_core::player`).
//!
//! Every function name and every parameter name here is load-bearing: the name IS the
//! IPC command string and the parameter names ARE the JSON keys the frontend sends
//! (camelCased by Tauri). Do not rename either.

use eko_core::engine::{Engine, EngineStatus};
use eko_core::player::{PlayerSnapshot, QueueItem};
use eko_core::queue::Repeat;
use eko_core::signal_path::{ReplayGainDecision, RgMode};

/// Everything the main window's poll needs, in one round trip.
#[derive(serde::Serialize)]
pub struct Poll {
    pub status: Option<EngineStatus>,
    pub player: PlayerSnapshot,
}

/// Tick the player, then report the session and the player together.
#[tauri::command]
pub fn engine_poll(engine: tauri::State<Engine>) -> Poll {
    let player = engine.player_poll();
    Poll {
        status: engine.status(),
        player,
    }
}

/// The frontend's whole queue, after any change to it.
#[tauri::command]
pub fn queue_sync(items: Vec<QueueItem>, engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.queue_sync(items)
}

/// A queue restored from the last run, positioned on `index`, resuming at `pos_ms`.
#[tauri::command]
pub fn queue_restore(
    items: Vec<QueueItem>,
    index: usize,
    pos_ms: u64,
    engine: tauri::State<Engine>,
) -> PlayerSnapshot {
    engine.queue_restore(items, index, pos_ms)
}

#[tauri::command]
pub fn queue_clear(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.queue_clear()
}

#[tauri::command]
pub fn player_play(uid: String, engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_play(uid)
}

#[tauri::command]
pub fn player_next(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_next()
}

#[tauri::command]
pub fn player_prev(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_prev()
}

#[tauri::command]
pub fn player_toggle(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_toggle()
}

#[tauri::command]
pub fn player_pause(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_pause()
}

#[tauri::command]
pub fn player_resume(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_resume()
}

#[tauri::command]
pub fn player_stop(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_stop()
}

#[tauri::command]
pub fn player_seek(ms: u64, engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_seek(ms)
}

#[tauri::command]
pub fn player_set_modes(
    repeat: Repeat,
    shuffle: bool,
    engine: tauri::State<Engine>,
) -> PlayerSnapshot {
    engine.player_set_modes(repeat, shuffle)
}

/// Set the ReplayGain mode; returns the decision for the playing item.
#[tauri::command]
pub fn player_set_replaygain(mode: RgMode, engine: tauri::State<Engine>) -> ReplayGainDecision {
    engine.player_set_replaygain(mode)
}

#[tauri::command]
pub fn player_set_scrobble(on: bool, engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_set_scrobble(on)
}

#[tauri::command]
pub fn player_sleep_after_track(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_sleep_after_track()
}

#[tauri::command]
pub fn player_sleep_in(ms: u64, engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_sleep_in(ms)
}

#[tauri::command]
pub fn player_cancel_sleep(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_cancel_sleep()
}

/// Start the playing item again (the output device changed).
#[tauri::command]
pub fn player_restart(engine: tauri::State<Engine>) -> PlayerSnapshot {
    engine.player_restart()
}
