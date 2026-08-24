//! `#[tauri::command]` wrapper over `eko_core::signal_path`. See the `commands`
//! module docs — forward and nothing else.
//!
//! The seal is the product's central claim, so it has exactly one derivation
//! (`eko_core::signal_path::derive`) shared by the GUI and the terminal client. This
//! wrapper must never default, branch, or reshape: any decision it made here would be
//! a decision the CLI does not make, and the two front ends would then disagree about
//! whether EKO is bit-perfect.
//!
//! The function name IS the IPC command string and the parameter name IS the JSON
//! key the TypeScript frontend sends. Do not rename either.

use eko_core::signal_path::{
    self, ReplayGainDecision, ReplayGainTags, RgMode, SealInput, SignalPath,
};

/// Derive the reported signal path (the bit-perfect seal, its breakdown, and the
/// `SOURCE → OUTPUT` display strings) from a stream snapshot plus the DSP settings.
#[tauri::command]
pub fn signal_path(input: SealInput) -> SignalPath {
    signal_path::derive(&input)
}

/// Decide both ReplayGain values for a track's tags under `mode`: the peak-limited dB the
/// engine receives, and the dead-banded dB the seal reports.
///
/// Returned together on purpose — applying one without the other is exactly how one front
/// end ends up reporting REPLAYGAIN where another reports BIT-PERFECT for the same track.
#[tauri::command]
pub fn signal_replaygain(tags: ReplayGainTags, mode: RgMode) -> ReplayGainDecision {
    signal_path::replaygain_decision(&tags, mode)
}
