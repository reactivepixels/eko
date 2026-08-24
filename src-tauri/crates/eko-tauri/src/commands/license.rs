//! `#[tauri::command]` wrappers over `eko_core::pro::license`.
//!
//! Each wrapper does exactly two things: resolve the app config directory from the
//! `AppHandle` (something `eko-core` cannot see) and forward. No error mapping, no
//! defaulting, no branching — every decision about what a missing config dir means
//! stays inside `eko_core::pro::license`.
//!
//! ⚠️ The resolved directory IS the licence file's location. It must stay byte-for-byte
//! the expression that used to live in `pro/license.rs`:
//!
//! ```text
//! before (eko-tauri, pro/license.rs):  app.path().app_config_dir().ok()
//! after  (eko-tauri, this file):       app.path().app_config_dir().ok()
//! ```
//!
//! Changing it — to `app_data_dir()`, `app_local_data_dir()`, or a `.join(..)` of any
//! kind — silently deactivates every existing paying customer on their next launch.
//!
//! Command names, parameter names and return types are the IPC contract with the
//! TypeScript frontend (`src/pro/useLicenseStore.ts`). They are unchanged.

use std::path::PathBuf;

use eko_core::pro::license::{self, LicenseStatus};
use tauri::Manager;

/// Resolve the app config directory that holds `license.key`.
///
/// This is the whole of the former `pro::license::config_dir(app)` helper, unchanged:
/// `Some(dir)` when the platform reports one, `None` when it does not. The `None` case
/// is handled by `eko-core` exactly as before — it is deliberately NOT turned into an
/// error or a fallback path here.
pub(crate) fn config_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

/// Return the current license status.
#[tauri::command]
pub fn license_status(app: tauri::AppHandle) -> LicenseStatus {
    license::compute_status(config_dir(&app).as_deref())
}

/// Attempt to activate a license key. Verifies offline, persists on success.
/// Returns the new status on success, or an error string on failure.
#[tauri::command]
pub fn license_activate(app: tauri::AppHandle, key: String) -> Result<LicenseStatus, String> {
    license::activate(config_dir(&app).as_deref(), &key)
}

/// Remove the stored license key. The tier falls back to free.
#[tauri::command]
pub fn license_deactivate(app: tauri::AppHandle) -> LicenseStatus {
    license::deactivate(config_dir(&app).as_deref())
}
