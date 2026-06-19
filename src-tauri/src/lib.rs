#[cfg(target_os = "macos")]
mod coreaudio;
mod engine;
mod metadata;
mod stream;

use std::sync::Mutex;
use tauri::menu::{MenuBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::Manager;

/// The configured Navidrome origin the `stream://` proxy is allowed to fetch (SSRF guard).
#[derive(Default)]
struct StreamOrigin(Mutex<Option<String>>);

/// Set/clear the allowed origin; called by the frontend on Navidrome connect/disconnect.
#[tauri::command]
fn set_stream_origin(origin: Option<String>, state: tauri::State<StreamOrigin>) {
    *state.0.lock().unwrap() = origin.filter(|s| !s.is_empty());
}

/// Store `value` in the macOS Keychain under the EKO service name + `key`.
#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    let entry = keyring::Entry::new("com.reactivepixels.eko", &key).map_err(|e| e.to_string())?;
    entry.set_password(&value).map_err(|e| e.to_string())
}

/// Retrieve a secret from the Keychain; returns `None` when no entry exists yet.
#[tauri::command]
fn secret_get(key: String) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new("com.reactivepixels.eko", &key).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Delete a Keychain secret; treats "no entry" as success (idempotent).
#[tauri::command]
fn secret_delete(key: String) -> Result<(), String> {
    let entry = keyring::Entry::new("com.reactivepixels.eko", &key).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(engine::Engine::default())
        .manage(StreamOrigin::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        // Progressive audio streaming proxy — see src/stream.rs (restricted to the configured origin).
        .register_asynchronous_uri_scheme_protocol("stream", |ctx, request, responder| {
            let uri = request.uri().to_string();
            let range = request
                .headers()
                .get(tauri::http::header::RANGE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let allowed = ctx
                .app_handle()
                .state::<StreamOrigin>()
                .0
                .lock()
                .unwrap()
                .clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(stream::proxy(uri, range, allowed).await);
            });
        })
        .setup(|app| {
            let h = app.handle();

            // Standard macOS app submenu (Hide / Quit).
            let app_menu = SubmenuBuilder::new(h, "EKO")
                .item(&PredefinedMenuItem::hide(h, None)?)
                .separator()
                .item(&PredefinedMenuItem::quit(h, None)?)
                .build()?;

            let menu = MenuBuilder::new(h).items(&[&app_menu]).build()?;
            app.set_menu(menu)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            metadata::read_metadata,
            metadata::scan_music_folder,
            metadata::read_cover,
            engine::engine_play,
            engine::engine_play_url,
            engine::engine_pause,
            engine::engine_resume,
            engine::engine_seek,
            engine::engine_stop,
            engine::engine_status,
            engine::engine_bands,
            engine::engine_set_eq,
            engine::engine_set_volume,
            engine::engine_set_replaygain,
            engine::engine_enqueue,
            engine::engine_set_now_playing,
            engine::engine_now_playing,
            engine::engine_list_devices,
            engine::engine_set_device,
            set_stream_origin,
            secret_set,
            secret_get,
            secret_delete
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                #[cfg(target_os = "macos")]
                crate::coreaudio::restore_all();
            }
        });
}
