//! EKO's audio core: decode, output, DSP, and metadata — with no UI dependency.

pub mod biquad;
#[cfg(target_os = "macos")]
pub mod coreaudio;
pub mod engine;
pub mod eq_presets;
pub mod metadata;
pub mod player;
#[cfg(feature = "pro")]
pub mod pro;
pub mod queue;
pub mod signal_path;
