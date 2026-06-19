#[cfg(target_os = "macos")]
mod coreaudio;
mod engine;
mod metadata;
mod stream;

use tauri::menu::{MenuBuilder, PredefinedMenuItem, SubmenuBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(engine::Engine::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        // Progressive audio streaming proxy — see src/stream.rs.
        .register_asynchronous_uri_scheme_protocol("stream", |_ctx, request, responder| {
            let uri = request.uri().to_string();
            let range = request
                .headers()
                .get(tauri::http::header::RANGE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            tauri::async_runtime::spawn(async move {
                responder.respond(stream::proxy(uri, range).await);
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
            engine::engine_set_device
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
