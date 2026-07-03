mod biquad;
#[cfg(target_os = "macos")]
mod broadcast;
#[cfg(target_os = "macos")]
mod coreaudio;
mod engine;
#[cfg(target_os = "macos")]
mod media;
mod metadata;
mod stream;

// Pro-only modules — compiled only when the `pro` feature is enabled.
// The free build compiles cleanly without these.
// `pub` so examples (e.g. verify_license, gen_keypair) can access crate::pro::license.
#[cfg(feature = "pro")]
pub mod pro;

use std::sync::Mutex;
use tauri::menu::{
    AboutMetadata, MenuBuilder, MenuItem, PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::Manager;
// Emitter (app.emit for menu-action events) is needed in BOTH builds: the free build
// still emits `sleep:*` events from the native "Controls ▸ Sleep Timer" menu.
use tauri::Emitter;

// Only the Pro Skins/Visualizer menus use CheckMenuItem (radio checkmarks).
#[cfg(feature = "pro")]
use tauri::menu::CheckMenuItem;

/// The configured Navidrome origin the `stream://` proxy is allowed to fetch (SSRF guard).
#[derive(Default)]
struct StreamOrigin(Mutex<Option<String>>);

/// Set/clear the allowed origin; called by the frontend on Navidrome connect/disconnect.
#[tauri::command]
fn set_stream_origin(origin: Option<String>, state: tauri::State<StreamOrigin>) {
    *state.0.lock().unwrap() = origin.filter(|s| !s.is_empty());
}

// ── Pro: native "Skins" menu — skin + accent + dark-mode, mirrored to the frontend store ──
//
// In the free build none of this is compiled: no MenuItems state, no sync_menu command,
// no Skins submenu, and no menu event handler for skin/accent/theme actions.

/// Holds the skin/accent/theme check-menu items so the frontend can keep their checkmarks
/// in sync. Pro build only — the free build has no Skins menu.
#[cfg(feature = "pro")]
#[derive(Default)]
struct MenuItems(Mutex<std::collections::HashMap<String, CheckMenuItem<tauri::Wry>>>);

/// Sync the native "Skins" menu checkmarks to the frontend's current skin / accent / theme.
/// Pro build only — the free build never calls this; the command is not registered.
#[cfg(feature = "pro")]
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

/// Sync the native "Visualizer" menu checkmarks to the frontend's current visualizer state.
/// Pro build only — the free build never calls this; the command is not registered.
#[cfg(feature = "pro")]
#[tauri::command]
fn sync_visualizer(open: bool, preset: String, items: tauri::State<MenuItems>) {
    let map = items.0.lock().unwrap();
    for (id, item) in map.iter() {
        let checked = match id.as_str() {
            "visualizer:on" => open,
            other => match other.split_once(':') {
                Some(("visualizer", v)) => v == preset,
                _ => continue,
            },
        };
        let _ = item.set_checked(checked);
    }
}

/// Build the "Controls" menu (Sleep Timer submenu). FREE feature — present in every build
/// and skin. The items just emit `menu-action` events (`sleep:off|15|30|45|60|eot`); the
/// frontend owns the timer state and shows the live countdown pill in the transport.
fn build_controls_menu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let sleep = SubmenuBuilder::new(app, "Sleep Timer")
        .item(&MenuItem::with_id(
            app,
            "sleep:off",
            "Off",
            true,
            None::<&str>,
        )?)
        .separator()
        .item(&MenuItem::with_id(
            app,
            "sleep:15",
            "15 Minutes",
            true,
            None::<&str>,
        )?)
        .item(&MenuItem::with_id(
            app,
            "sleep:30",
            "30 Minutes",
            true,
            None::<&str>,
        )?)
        .item(&MenuItem::with_id(
            app,
            "sleep:45",
            "45 Minutes",
            true,
            None::<&str>,
        )?)
        .item(&MenuItem::with_id(
            app,
            "sleep:60",
            "60 Minutes",
            true,
            None::<&str>,
        )?)
        .separator()
        .item(&MenuItem::with_id(
            app,
            "sleep:eot",
            "End of Track",
            true,
            None::<&str>,
        )?)
        .build()?;
    SubmenuBuilder::new(app, "Controls").item(&sleep).build()
}

/// Build the native menu for the current license tier and apply it. The Pro menus (Skins,
/// Visualizer) are present only for a licensed user; the free tier gets just the app menu
/// (which still carries Enter License Key… / Get EKO Pro). Called at startup and live on a
/// license change (via `refresh_menu`), so activating a key reveals the Pro menus without a
/// relaunch.
#[cfg(feature = "pro")]
fn apply_pro_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let about = AboutMetadata {
        name: Some("EKO".into()),
        version: Some(app.package_info().version.to_string()),
        copyright: Some("© 2026 Reactive Pixels".into()),
        comments: Some("A bit-perfect audiophile music player for macOS.".into()),
        website: Some("https://github.com/reactivepixels/eko".into()),
        website_label: Some("GitHub".into()),
        ..Default::default()
    };
    let licensed = crate::pro::license::compute_status(app).tier == crate::pro::license::Tier::Pro;

    // Licensing entries adapt to the tier: licensed → "Remove License"; free → "Enter
    // License Key…" + "Get EKO Pro" (no purchase link once activated).
    let mut app_menu_b = SubmenuBuilder::new(app, "EKO")
        .item(&PredefinedMenuItem::about(
            app,
            Some("About EKO"),
            Some(about),
        )?)
        .separator();
    if licensed {
        app_menu_b = app_menu_b.item(&MenuItem::with_id(
            app,
            "license:remove",
            "Remove License",
            true,
            None::<&str>,
        )?);
    } else {
        app_menu_b = app_menu_b
            .item(&MenuItem::with_id(
                app,
                "license:enter",
                "Enter License Key…",
                true,
                None::<&str>,
            )?)
            .item(&MenuItem::with_id(
                app,
                "license:get",
                "Get EKO Pro",
                true,
                None::<&str>,
            )?);
    }
    let app_menu = app_menu_b
        .separator()
        .item(&PredefinedMenuItem::hide(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, None)?)
        .build()?;

    // Controls (Sleep Timer) — FREE, present regardless of tier.
    let controls_menu = build_controls_menu(app)?;

    // Reset the checkmark-sync map; repopulate only when the Pro menus are present.
    app.state::<MenuItems>().0.lock().unwrap().clear();

    if !licensed {
        app.set_menu(
            MenuBuilder::new(app)
                .items(&[&app_menu, &controls_menu])
                .build()?,
        )?;
        return Ok(());
    }

    let porcelain =
        CheckMenuItem::with_id(app, "skin:porcelain", "Porcelain", true, true, None::<&str>)?;
    let studio = CheckMenuItem::with_id(app, "skin:studio", "Studio", true, false, None::<&str>)?;
    let aether = CheckMenuItem::with_id(app, "skin:aether", "Aether", true, false, None::<&str>)?;
    let acc_orange =
        CheckMenuItem::with_id(app, "accent:orange", "Orange", true, true, None::<&str>)?;
    let acc_violet =
        CheckMenuItem::with_id(app, "accent:violet", "Violet", true, false, None::<&str>)?;
    let acc_blue = CheckMenuItem::with_id(app, "accent:blue", "Blue", true, false, None::<&str>)?;
    let acc_teal = CheckMenuItem::with_id(app, "accent:teal", "Teal", true, false, None::<&str>)?;
    let acc_graphite = CheckMenuItem::with_id(
        app,
        "accent:graphite",
        "Graphite",
        true,
        false,
        None::<&str>,
    )?;
    let acc_cyan = CheckMenuItem::with_id(app, "accent:cyan", "Cyan", true, false, None::<&str>)?;
    let theme_dark =
        CheckMenuItem::with_id(app, "theme:dark", "Dark Mode", true, false, None::<&str>)?;
    let accent_menu = SubmenuBuilder::new(app, "Accent")
        .item(&acc_orange)
        .item(&acc_violet)
        .item(&acc_blue)
        .item(&acc_teal)
        .item(&acc_graphite)
        .item(&acc_cyan)
        .build()?;
    let skins_menu = SubmenuBuilder::new(app, "Skins")
        .item(&porcelain)
        .item(&studio)
        .item(&aether)
        .separator()
        .item(&accent_menu)
        .separator()
        .item(&theme_dark)
        .build()?;
    let viz_on = CheckMenuItem::with_id(app, "visualizer:on", "On", true, false, None::<&str>)?;
    let viz_galaxy =
        CheckMenuItem::with_id(app, "visualizer:galaxy", "Galaxy", true, true, None::<&str>)?;
    let viz_cymatics = CheckMenuItem::with_id(
        app,
        "visualizer:cymatics",
        "Cymatics",
        true,
        false,
        None::<&str>,
    )?;
    let viz_murmuration = CheckMenuItem::with_id(
        app,
        "visualizer:murmuration",
        "Murmuration",
        true,
        false,
        None::<&str>,
    )?;
    let visualizer_menu = SubmenuBuilder::new(app, "Visualizer")
        .item(&viz_on)
        .separator()
        .item(&viz_galaxy)
        .item(&viz_cymatics)
        .item(&viz_murmuration)
        .build()?;

    {
        let st = app.state::<MenuItems>();
        let mut map = st.0.lock().unwrap();
        for it in [
            &porcelain,
            &studio,
            &aether,
            &acc_orange,
            &acc_violet,
            &acc_blue,
            &acc_teal,
            &acc_graphite,
            &acc_cyan,
            &theme_dark,
        ] {
            map.insert(it.id().as_ref().to_string(), it.clone());
        }
        for it in [&viz_on, &viz_galaxy, &viz_cymatics, &viz_murmuration] {
            map.insert(it.id().as_ref().to_string(), it.clone());
        }
    }

    app.set_menu(
        MenuBuilder::new(app)
            .items(&[&app_menu, &controls_menu, &skins_menu, &visualizer_menu])
            .build()?,
    )?;
    Ok(())
}

/// Rebuild the native menu for the current license (called from the frontend after a key
/// is activated or deactivated). Pro build only.
#[cfg(feature = "pro")]
#[tauri::command]
fn refresh_menu(app: tauri::AppHandle) -> Result<(), String> {
    apply_pro_menu(&app).map_err(|e| e.to_string())
}

// ── Credential persistence (Keychain primary, file mirror fallback) ─────────────
//
// The macOS Keychain is the primary, secure credential store. But an UNSIGNED build
// run under Gatekeeper app-translocation (e.g. a DMG downloaded from GitHub before we
// have an Apple Developer ID) is mounted from a random read-only path and gets an
// unstable code identity — so Keychain items written in one launch cannot be read
// back in the next, and logins appear "not persisted". (Server *metadata* in the
// WebView's localStorage survives because it's keyed to the bundle id, not the path;
// the Keychain ACL is keyed to the code signature, which is what breaks.)
//
// To make credentials survive relaunch TODAY we mirror them to a 0600 JSON file under
// the app data dir — which is bundle-id-keyed and therefore stable across
// translocation — and fall back to it when the Keychain read misses. Values are
// lightly obfuscated (XOR + base64) so the file isn't casually readable; this is NOT
// cryptography — the OS file permissions are the real protection. Once Developer ID
// signing lands, the Keychain round-trips and this becomes a redundant mirror we can
// encrypt or drop. This whole path is FREE (multi-server credential storage).

const SECRET_PAD: &[u8] = b"eko-secret-pad-v1-not-for-real-security";

/// Detect Gatekeeper app-translocation: a translocated (unsigned/quarantined) bundle
/// runs from `…/AppTranslocation/…`, where the Keychain neither persists nor reads
/// reliably (and can raise auth prompts). When translocated we skip the Keychain
/// entirely and use the file mirror.
fn keychain_usable() -> bool {
    std::env::current_exe()
        .map(|p| !p.to_string_lossy().contains("/AppTranslocation/"))
        .unwrap_or(true)
}

fn obfuscate(value: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = value
        .as_bytes()
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ SECRET_PAD[i % SECRET_PAD.len()])
        .collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn deobfuscate(encoded: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let plain: Vec<u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ SECRET_PAD[i % SECRET_PAD.len()])
        .collect();
    String::from_utf8(plain).ok()
}

fn secrets_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("secrets.json"))
}

fn read_file_secrets(app: &tauri::AppHandle) -> std::collections::HashMap<String, String> {
    secrets_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_file_secrets(app: &tauri::AppHandle, map: &std::collections::HashMap<String, String>) {
    let Some(path) = secrets_path(app) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(raw) = serde_json::to_string(map) else {
        return;
    };
    if std::fs::write(&path, raw).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// Store `value` for `key` — in the Keychain (when usable) and always in the file
/// mirror so it survives relaunch on unsigned builds. Free feature.
#[tauri::command]
fn secret_set(app: tauri::AppHandle, key: String, value: String) -> Result<(), String> {
    if keychain_usable() {
        if let Ok(entry) = keyring::Entry::new("com.reactivepixels.eko", &key) {
            let _ = entry.set_password(&value);
        }
    }
    let mut map = read_file_secrets(&app);
    map.insert(key, obfuscate(&value));
    write_file_secrets(&app, &map);
    Ok(())
}

/// Retrieve a secret: prefer the Keychain (authoritative on signed builds), fall back
/// to the file mirror. Returns `None` when neither has it. Free feature.
#[tauri::command]
fn secret_get(app: tauri::AppHandle, key: String) -> Result<Option<String>, String> {
    if keychain_usable() {
        if let Ok(entry) = keyring::Entry::new("com.reactivepixels.eko", &key) {
            if let Ok(v) = entry.get_password() {
                return Ok(Some(v));
            }
        }
    }
    Ok(read_file_secrets(&app)
        .get(&key)
        .and_then(|v| deobfuscate(v)))
}

/// Delete a secret from both stores; idempotent. Free feature.
#[tauri::command]
fn secret_delete(app: tauri::AppHandle, key: String) -> Result<(), String> {
    if keychain_usable() {
        if let Ok(entry) = keyring::Entry::new("com.reactivepixels.eko", &key) {
            let _ = entry.delete_credential(); // ignore NoEntry / errors — best-effort
        }
    }
    let mut map = read_file_secrets(&app);
    map.remove(&key);
    write_file_secrets(&app, &map);
    Ok(())
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

/// Post a macOS distributed notification (`com.reactivepixels.eko.playbackState`) so a
/// companion app can react to play/pause/stop/track-change — mirrors Spotify's
/// `com.spotify.client.PlaybackStateChanged` shape. No-op off macOS.
#[tauri::command]
fn broadcast_playback(state: String, name: String, artist: String) {
    #[cfg(target_os = "macos")]
    broadcast::post(&state, &name, &artist);
    #[cfg(not(target_os = "macos"))]
    let _ = (state, name, artist);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .manage(engine::Engine::default())
        .manage(StreamOrigin::default());

    // Pro build: register the MenuItems state for Skins menu sync.
    #[cfg(feature = "pro")]
    {
        builder = builder.manage(MenuItems::default());
    }

    #[cfg(target_os = "macos")]
    {
        builder = builder.manage(media::Media::default());
    }

    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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

            // ── Pro: full menu (app + Skins + Visualizer), license-aware ──────────────
            // `apply_pro_menu` builds the whole menu from the on-disk license: licensed
            // users get the Skins/Visualizer menus + a "Remove License" entry; free users
            // get only the app menu with "Enter License Key…" / "Get EKO Pro". The same
            // function is re-run via the `refresh_menu` command after activate/deactivate
            // so the menus appear (or disappear) live, with no relaunch.
            #[cfg(feature = "pro")]
            {
                apply_pro_menu(h)?;
            }

            // ── Free build: app menu + Controls (Sleep Timer); no Pro entries ─────────
            #[cfg(not(feature = "pro"))]
            {
                let about = AboutMetadata {
                    name: Some("EKO".into()),
                    version: Some(app.package_info().version.to_string()),
                    copyright: Some("© 2026 Reactive Pixels".into()),
                    comments: Some("A bit-perfect audiophile music player for macOS.".into()),
                    website: Some("https://github.com/reactivepixels/eko".into()),
                    website_label: Some("GitHub".into()),
                    ..Default::default()
                };
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
                let controls_menu = build_controls_menu(h)?;
                let menu = MenuBuilder::new(h)
                    .items(&[&app_menu, &controls_menu])
                    .build()?;
                app.set_menu(menu)?;
            }

            // Menu clicks → tell the frontend (both builds; sleep:* is a free menu). The
            // frontend updates its store (the source of truth) + re-syncs any checkmarks.
            app.on_menu_event(move |app, event| {
                let id = event.id().as_ref().to_string();
                if id.starts_with("sleep:")
                    || id.starts_with("skin:")
                    || id.starts_with("accent:")
                    || id.starts_with("theme:")
                    || id.starts_with("visualizer:")
                    || id.starts_with("license:")
                {
                    let _ = app.emit("menu-action", id);
                }
            });

            // System "Now Playing" + hardware media keys (macOS). Must be set up on the
            // main thread, which `setup` runs on.
            #[cfg(target_os = "macos")]
            media::init(&h.clone());

            // Offline cache — only compiled + initialised in Pro builds.
            // The AppHandle is needed to resolve the cache directory.
            #[cfg(feature = "pro")]
            app.manage(pro::offline::OfflineCache::init(h));

            Ok(())
        })
        // ── Unified command handler ─────────────────────────────────────────────
        // Free commands are always present.  Pro commands are wrapped in
        // `#[cfg(feature = "pro")]` so they compile away in the free build.
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
            engine::engine_set_eq_mode,
            engine::engine_set_volume,
            engine::engine_set_replaygain,
            engine::engine_enqueue,
            engine::engine_set_now_playing,
            engine::engine_now_playing,
            engine::engine_list_devices,
            engine::engine_set_device,
            set_stream_origin,
            media_metadata,
            media_playback,
            media_stopped,
            broadcast_playback,
            // ── Keychain commands (free — multi-server credential storage) ──
            secret_set,
            secret_get,
            secret_delete,
            // ── Pro commands (compiled away in free build) ──────────────────
            #[cfg(feature = "pro")]
            sync_menu,
            #[cfg(feature = "pro")]
            sync_visualizer,
            #[cfg(feature = "pro")]
            refresh_menu,
            #[cfg(feature = "pro")]
            engine::engine_play_cached,
            #[cfg(feature = "pro")]
            engine::engine_set_param_eq,
            #[cfg(feature = "pro")]
            engine::engine_eq_curve,
            #[cfg(feature = "pro")]
            engine::engine_parse_autoeq,
            #[cfg(feature = "pro")]
            engine::engine_import_autoeq_file,
            #[cfg(feature = "pro")]
            pro::offline::cache_track,
            #[cfg(feature = "pro")]
            pro::offline::cache_album,
            #[cfg(feature = "pro")]
            pro::offline::remove_offline,
            #[cfg(feature = "pro")]
            pro::offline::offline_list,
            #[cfg(feature = "pro")]
            pro::offline::offline_stats,
            #[cfg(feature = "pro")]
            pro::offline::set_cache_limit,
            #[cfg(feature = "pro")]
            pro::offline::set_cache_bitrate,
            #[cfg(feature = "pro")]
            pro::license::license_status,
            #[cfg(feature = "pro")]
            pro::license::license_activate,
            #[cfg(feature = "pro")]
            pro::license::license_deactivate,
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
