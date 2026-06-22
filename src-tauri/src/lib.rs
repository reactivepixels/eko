#[cfg(target_os = "macos")]
mod coreaudio;
mod engine;
#[cfg(target_os = "macos")]
mod media;
mod metadata;
mod stream;

use std::sync::Mutex;
use tauri::menu::{AboutMetadata, CheckMenuItem, MenuBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{Emitter, Manager};

/// The configured Navidrome origin the `stream://` proxy is allowed to fetch (SSRF guard).
#[derive(Default)]
struct StreamOrigin(Mutex<Option<String>>);

/// Set/clear the allowed origin; called by the frontend on Navidrome connect/disconnect.
#[tauri::command]
fn set_stream_origin(origin: Option<String>, state: tauri::State<StreamOrigin>) {
    *state.0.lock().unwrap() = origin.filter(|s| !s.is_empty());
}

/// Holds the skin/accent/theme check-menu items so the frontend can keep their checkmarks in sync.
#[derive(Default)]
struct MenuItems(Mutex<std::collections::HashMap<String, CheckMenuItem<tauri::Wry>>>);

/// Sync the native "Skins" menu checkmarks to the frontend's current skin / accent / theme.
#[tauri::command]
fn sync_menu(skin: String, accent: String, theme: String, items: tauri::State<MenuItems>) {
    let map = items.0.lock().unwrap();
    for (id, item) in map.iter() {
        let checked = match id.split_once(':') {
            Some(("skin", v)) => v == skin,
            Some(("accent", v)) => v == accent,
            Some(("theme", "dark")) => theme == "dark",
            _ => false,
        };
        let _ = item.set_checked(checked);
    }
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

/// Push current-track metadata to the OS "Now Playing" card. No-op off macOS.
#[tauri::command]
fn media_metadata(
    app: tauri::AppHandle,
    title: String,
    artist: String,
    album: String,
    cover_url: Option<String>,
    duration: Option<f64>,
) {
    #[cfg(target_os = "macos")]
    media::set_metadata(&app, title, artist, album, cover_url, duration);
    #[cfg(not(target_os = "macos"))]
    let _ = (app, title, artist, album, cover_url, duration);
}

/// Push the OS "Now Playing" play/pause state + elapsed position. No-op off macOS.
#[tauri::command]
fn media_playback(app: tauri::AppHandle, playing: bool, elapsed: f64) {
    #[cfg(target_os = "macos")]
    media::set_playback(&app, playing, elapsed);
    #[cfg(not(target_os = "macos"))]
    let _ = (app, playing, elapsed);
}

/// Clear the OS "Now Playing" card. No-op off macOS.
#[tauri::command]
fn media_stopped(app: tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    media::set_stopped(&app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .manage(engine::Engine::default())
        .manage(StreamOrigin::default())
        .manage(MenuItems::default());
    #[cfg(target_os = "macos")]
    {
        builder = builder.manage(media::Media::default());
    }
    builder
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

            // Native macOS "About EKO" panel — version + standard info.
            let about = AboutMetadata {
                name: Some("EKO".into()),
                version: Some(app.package_info().version.to_string()),
                copyright: Some("© 2026 Reactive Pixels".into()),
                comments: Some("A bit-perfect audiophile music player for macOS.".into()),
                website: Some("https://github.com/reactivepixels/eko".into()),
                website_label: Some("GitHub".into()),
                ..Default::default()
            };

            // Standard macOS app submenu (About / Hide / Quit).
            let app_menu = SubmenuBuilder::new(h, "EKO")
                .item(&PredefinedMenuItem::about(
                    h,
                    Some("About EKO"),
                    Some(about),
                )?)
                .separator()
                .item(&PredefinedMenuItem::hide(h, None)?)
                .separator()
                .item(&PredefinedMenuItem::quit(h, None)?)
                .build()?;

            // ── Skins menu: skin + accent + dark-mode, mirrored to the frontend store ──
            let porcelain =
                CheckMenuItem::with_id(h, "skin:porcelain", "Porcelain", true, true, None::<&str>)?;
            let studio =
                CheckMenuItem::with_id(h, "skin:studio", "Studio", true, false, None::<&str>)?;
            let acc_orange =
                CheckMenuItem::with_id(h, "accent:orange", "Orange", true, true, None::<&str>)?;
            let acc_violet =
                CheckMenuItem::with_id(h, "accent:violet", "Violet", true, false, None::<&str>)?;
            let acc_blue =
                CheckMenuItem::with_id(h, "accent:blue", "Blue", true, false, None::<&str>)?;
            let acc_teal =
                CheckMenuItem::with_id(h, "accent:teal", "Teal", true, false, None::<&str>)?;
            let acc_graphite = CheckMenuItem::with_id(
                h,
                "accent:graphite",
                "Graphite",
                true,
                false,
                None::<&str>,
            )?;
            let theme_dark =
                CheckMenuItem::with_id(h, "theme:dark", "Dark Mode", true, false, None::<&str>)?;

            let accent_menu = SubmenuBuilder::new(h, "Accent")
                .item(&acc_orange)
                .item(&acc_violet)
                .item(&acc_blue)
                .item(&acc_teal)
                .item(&acc_graphite)
                .build()?;

            let skins_menu = SubmenuBuilder::new(h, "Skins")
                .item(&porcelain)
                .item(&studio)
                .separator()
                .item(&accent_menu)
                .separator()
                .item(&theme_dark)
                .build()?;

            // Keep the check-item handles so `sync_menu` can update the checkmarks from the frontend.
            {
                let st = app.state::<MenuItems>();
                let mut map = st.0.lock().unwrap();
                for it in [
                    &porcelain,
                    &studio,
                    &acc_orange,
                    &acc_violet,
                    &acc_blue,
                    &acc_teal,
                    &acc_graphite,
                    &theme_dark,
                ] {
                    map.insert(it.id().as_ref().to_string(), it.clone());
                }
            }

            let menu = MenuBuilder::new(h)
                .items(&[&app_menu, &skins_menu])
                .build()?;
            app.set_menu(menu)?;

            // Menu clicks → tell the frontend; it updates the store (source of truth) + re-syncs checks.
            app.on_menu_event(move |app, event| {
                let id = event.id().as_ref().to_string();
                if id.starts_with("skin:") || id.starts_with("accent:") || id.starts_with("theme:")
                {
                    let _ = app.emit("menu-action", id);
                }
            });

            // System "Now Playing" + hardware media keys (macOS). Must be set up on the
            // main thread, which `setup` runs on.
            #[cfg(target_os = "macos")]
            media::init(&h.clone());
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
            engine::engine_set_crossfade,
            engine::engine_set_now_playing,
            engine::engine_now_playing,
            engine::engine_list_devices,
            engine::engine_set_device,
            set_stream_origin,
            sync_menu,
            secret_set,
            secret_get,
            secret_delete,
            media_metadata,
            media_playback,
            media_stopped
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
