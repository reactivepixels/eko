//! Thin `#[tauri::command]` wrappers over `eko_core`.
//!
//! Permitted in a wrapper: unwrapping `tauri::State` / `tauri::AppHandle`, resolving a
//! path or a licence tier from the `AppHandle` (things `eko-core` cannot see), and
//! forwarding the arguments.
//!
//! Forbidden: error mapping, defaulting, branching on argument *values*, type
//! conversion, or any other decision that could drift from `eko-core`'s behaviour.
//! If a wrapper starts making decisions about audio, that decision belongs in `eko-core`.
//!
//! Command names and parameter names are the IPC contract with the TypeScript frontend.
//! Renaming either silently breaks the app at runtime — the Rust test suite cannot
//! catch it.

pub mod engine;
#[cfg(feature = "pro")]
pub mod license;
pub mod metadata;
#[cfg(feature = "pro")]
pub mod offline;
pub mod signal;
pub mod subsonic;
