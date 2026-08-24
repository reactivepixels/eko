//! The 10-band graphic EQ — **one copy of the numbers, two readers.**
//!
//! This is the client's whole EQ state: on/off, a pre-amp, and ten band gains in
//! dB. It is deliberately not a view model and not an engine wrapper; it is the
//! single set of values that both of the things downstream read.
//!
//! ## Why the two readers matter
//!
//! An EQ has two consumers that must never disagree:
//!
//! * [`GraphicEq::engine_args`] — what `Engine::set_eq` is handed, i.e. what is
//!   actually applied to the samples; and
//! * [`GraphicEq::seal_state`] — what `eko_core::signal_path::derive` is told,
//!   i.e. what EKO **claims** is being applied.
//!
//! If those two can drift, the seal can read `BIT-PERFECT` over an EQ'd signal —
//! the exact class of bug this project has already found and fixed six times.
//! They cannot drift here because neither of them holds any state: both are pure
//! functions of the same three private fields, and there is no third field for
//! one of them to read and the other to miss. [`crate::app::App::apply_eq`] is
//! the only caller of `Engine::set_eq` in the crate, and it takes its arguments
//! from `engine_args` and nowhere else.
//!
//! The one drift the types cannot rule out is *temporal*: `engine_args` is a
//! push and `seal_state` is a pull, so a mutation that forgets to push leaves the
//! engine flat while the seal says `EQ`. That direction is the safe one — it
//! over-reports — and it is closed anyway, because every mutation goes through
//! [`crate::app::App::edit_eq`], which pushes.
//!
//! ## The presets are `eko_core`'s, not a copy of them
//!
//! [`eko_core::eq_presets::PRESETS`] is read directly. Nothing in this crate
//! writes a preset gain down: the panel's labels come from `PRESETS[i].name`,
//! the values from `PRESETS[i].gains`, and the tests assert against `PRESETS`
//! itself rather than against a hand transcription. A third copy of that table
//! would be a third thing to keep in step with the GUI's — and the desktop
//! already learned that lesson the expensive way.
//!
//! ## Parametric EQ is not here
//!
//! `eko-cli` is FREE. [`GraphicEq::seal_state`] reports
//! [`SealEqMode::Graphic`] and leaves `param_enabled`, `param_preamp` and
//! `param_bands` at their defaults, because this client has no parametric EQ to
//! describe. That is the truth, not a placeholder.

use eko_core::eq_presets::{EqPreset, EQ_BANDS, EQ_BAND_COUNT, EQ_GAIN_MAX, EQ_GAIN_MIN, PRESETS};
use eko_core::signal_path::{EqState, SealEqMode};

/// Adjustable columns: the pre-amp, then one per band.
///
/// The pre-amp is a column rather than a special case because it is one on
/// screen and one in the seal — `EqState::active` reads a non-zero pre-amp as a
/// modification exactly as it reads a non-zero band gain.
pub const COLUMNS: usize = EQ_BAND_COUNT + 1;

/// The pre-amp's column index. Column 0, as on every graphic EQ ever shipped.
pub const PREAMP: usize = 0;

/// How much one press moves a gain, in dB.
pub const GAIN_STEP: f32 = 1.0;

/// The column index of band `i` (0-based, low frequency → high).
#[must_use]
pub const fn band(i: usize) -> usize {
    i + 1
}

/// How a column is labelled: `pre`, then the band frequency.
///
/// Derived from [`EQ_BANDS`] through `eko_core`'s own `band_label`, so a label
/// cannot drift from the band it sits under.
#[must_use]
pub fn column_label(col: usize) -> String {
    if col == PREAMP {
        "pre".to_string()
    } else {
        EQ_BANDS
            .get(col - 1)
            .map_or_else(String::new, |&hz| eko_core::eq_presets::band_label(hz))
    }
}

/// The client's graphic EQ.
///
/// Fields are private on purpose: the invariant is that every gain stays inside
/// [`EQ_GAIN_MIN`]`..=`[`EQ_GAIN_MAX`], and that whatever is in here is what both
/// the engine and the seal are told. Public fields would be two more ways to
/// break that.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphicEq {
    /// Whether the EQ is routed into the DSP path at all.
    ///
    /// Separate from "the curve is flat", because they are different states: a
    /// bypassed EQ with a curve dialled in is a thing people want, and the seal
    /// treats it correctly either way — `EqState::active` is `enabled && the
    /// curve does something`.
    enabled: bool,
    /// Pre-amp, in dB.
    preamp: f32,
    /// Per-band gains in dB, low frequency → high.
    gains: [f32; EQ_BAND_COUNT],
}

impl Default for GraphicEq {
    /// Off and flat — the bit-perfect starting point.
    fn default() -> Self {
        Self {
            enabled: false,
            preamp: 0.0,
            gains: [0.0; EQ_BAND_COUNT],
        }
    }
}

impl GraphicEq {
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The dB in one column — [`PREAMP`], or a band.
    ///
    /// Out-of-range columns answer `0.0` rather than panicking: the cursor is
    /// clamped where it is written, and a render is not the place to discover a
    /// bug by crashing the terminal out of raw mode.
    #[must_use]
    pub fn value(&self, col: usize) -> f32 {
        if col == PREAMP {
            self.preamp
        } else {
            self.gains.get(col - 1).copied().unwrap_or(0.0)
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Set one column, clamped to the slider range.
    pub fn set_value(&mut self, col: usize, db: f32) {
        let db = if db.is_nan() {
            0.0
        } else {
            db.clamp(EQ_GAIN_MIN, EQ_GAIN_MAX)
        };
        if col == PREAMP {
            self.preamp = db;
        } else if let Some(g) = self.gains.get_mut(col - 1) {
            *g = db;
        }
    }

    /// Move one column by `delta` dB, clamped.
    pub fn nudge(&mut self, col: usize, delta: f32) {
        self.set_value(col, self.value(col) + delta);
    }

    /// Load a preset by index into [`PRESETS`]. Out of range does nothing.
    pub fn load_preset(&mut self, index: usize) {
        let Some(preset) = PRESETS.get(index) else {
            return;
        };
        self.preamp = preset.preamp;
        self.gains = preset.gains;
    }

    /// The preset these values *are*, if any.
    ///
    /// Derived rather than remembered. A stored "current preset" index would be
    /// a fourth field that the gains could contradict — dial a band and the
    /// stored name would still claim `Rock`. Comparing the values means the name
    /// on screen is always a fact about the curve.
    #[must_use]
    pub fn preset_index(&self) -> Option<usize> {
        PRESETS
            .iter()
            .position(|p| p.preamp == self.preamp && p.gains == self.gains)
    }

    /// What to call the current curve: a preset's name, or `custom`.
    #[must_use]
    pub fn preset_name(&self) -> &'static str {
        self.preset_index()
            .and_then(|i| PRESETS.get(i))
            .map_or("custom", |p: &EqPreset| p.name)
    }

    /// Step to the next preset, wrapping. A hand-edited curve steps to the
    /// first one — `Flat` — which is also the way back to a clean signal path.
    pub fn next_preset(&mut self) {
        let next = self.preset_index().map_or(0, |i| (i + 1) % PRESETS.len());
        self.load_preset(next);
    }

    /// Step to the previous preset, wrapping.
    pub fn prev_preset(&mut self) {
        let prev = self
            .preset_index()
            .map_or(0, |i| (i + PRESETS.len() - 1) % PRESETS.len());
        self.load_preset(prev);
    }

    /// What `Engine::set_eq` is handed. **The only source of that call's
    /// arguments** — see [`crate::app::App::apply_eq`].
    ///
    /// `preamp` is widened to `f64` because that is the engine's signature; it
    /// narrows it straight back to `f32`, so the value the DSP thread uses is
    /// bit-identical to the one [`Self::seal_state`] reports.
    #[must_use]
    pub fn engine_args(&self) -> (bool, f64, Vec<f32>) {
        (self.enabled, f64::from(self.preamp), self.gains.to_vec())
    }

    /// What `signal_path::derive` is told.
    ///
    /// The parametric fields stay at their defaults: `eko-cli` is FREE and has
    /// no parametric EQ, so there is nothing to describe. `mode` is stated
    /// rather than defaulted so that a change to `SealEqMode`'s default cannot
    /// silently re-route what this client claims.
    #[must_use]
    pub fn seal_state(&self) -> EqState {
        EqState {
            mode: SealEqMode::Graphic,
            enabled: self.enabled,
            preamp: f64::from(self.preamp),
            gains: self.gains.iter().copied().map(f64::from).collect(),
            ..EqState::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ten band gains, read back one column at a time.
    fn curve(eq: &GraphicEq) -> [f32; EQ_BAND_COUNT] {
        let mut out = [0.0; EQ_BAND_COUNT];
        for (i, g) in out.iter_mut().enumerate() {
            *g = eq.value(band(i));
        }
        out
    }

    #[test]
    fn a_fresh_eq_is_off_and_flat_and_therefore_inert() {
        let eq = GraphicEq::default();
        assert!(!eq.enabled());
        assert_eq!(eq.value(PREAMP), 0.0);
        assert_eq!(curve(&eq), [0.0; EQ_BAND_COUNT]);
        assert!(!eq.seal_state().active(), "a fresh EQ claimed the signal");
        assert_eq!(eq.preset_name(), "Flat");
    }

    /// Enabled but flat is **not** a modification, and the seal must not say it
    /// is. `EqState::active` is the arbiter; this pins that this crate's state
    /// reaches it in the shape it expects.
    #[test]
    fn an_enabled_but_flat_eq_still_touches_nothing() {
        let mut eq = GraphicEq::default();
        eq.set_enabled(true);
        assert!(!eq.seal_state().active());
    }

    #[test]
    fn one_raised_band_is_a_modification_and_a_disabled_one_is_not() {
        let mut eq = GraphicEq::default();
        eq.nudge(band(3), 6.0);
        assert!(
            !eq.seal_state().active(),
            "a bypassed curve claimed the signal"
        );
        eq.set_enabled(true);
        assert!(eq.seal_state().active());
    }

    /// A non-zero pre-amp alone is a modification, because it is one.
    #[test]
    fn the_preamp_alone_breaks_the_signal_path() {
        let mut eq = GraphicEq::default();
        eq.set_enabled(true);
        eq.nudge(PREAMP, -3.0);
        assert_eq!(eq.value(PREAMP), -3.0);
        assert!(eq.seal_state().active());
    }

    #[test]
    fn gains_are_clamped_to_the_slider_range() {
        let mut eq = GraphicEq::default();
        for col in 0..COLUMNS {
            eq.set_value(col, 99.0);
            assert_eq!(eq.value(col), EQ_GAIN_MAX);
            eq.set_value(col, -99.0);
            assert_eq!(eq.value(col), EQ_GAIN_MIN);
            eq.set_value(col, f32::NAN);
            assert_eq!(eq.value(col), 0.0);
        }
    }

    #[test]
    fn a_column_past_the_last_band_is_ignored_rather_than_panicking() {
        let mut eq = GraphicEq::default();
        eq.set_value(COLUMNS, 6.0);
        eq.nudge(COLUMNS + 40, 6.0);
        assert_eq!(eq.value(COLUMNS), 0.0);
        assert_eq!(curve(&eq), [0.0; EQ_BAND_COUNT]);
    }

    /// **The presets are `eko_core`'s.** Loading preset `i` must reproduce
    /// `PRESETS[i]` exactly — asserted against `PRESETS` itself, so this test
    /// cannot pass by agreeing with a copy that has drifted.
    #[test]
    fn every_preset_loads_eko_cores_own_numbers() {
        for (i, preset) in PRESETS.iter().enumerate() {
            let mut eq = GraphicEq::default();
            eq.load_preset(i);
            assert_eq!(eq.value(PREAMP), preset.preamp, "{} preamp", preset.name);
            assert_eq!(curve(&eq), preset.gains, "{} gains", preset.name);
            assert_eq!(eq.preset_index(), Some(i), "{} index", preset.name);
            assert_eq!(eq.preset_name(), preset.name);
        }
    }

    #[test]
    fn stepping_forward_through_the_presets_wraps_and_visits_all_of_them() {
        let mut eq = GraphicEq::default();
        let mut seen = Vec::new();
        for _ in 0..PRESETS.len() {
            eq.next_preset();
            seen.push(eq.preset_name());
        }
        let expected: Vec<&str> = PRESETS
            .iter()
            .skip(1)
            .chain(PRESETS.iter().take(1))
            .map(|p| p.name)
            .collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn stepping_back_from_flat_wraps_to_the_last_preset() {
        let mut eq = GraphicEq::default();
        eq.prev_preset();
        assert_eq!(
            eq.preset_index(),
            Some(PRESETS.len() - 1),
            "prev from the first preset did not wrap"
        );
    }

    /// A hand-edited curve has no preset, and stepping from it lands on the
    /// first one rather than pretending it was somewhere in the list.
    #[test]
    fn a_hand_edited_curve_is_custom_and_steps_to_the_first_preset() {
        let mut eq = GraphicEq::default();
        eq.load_preset(1);
        eq.nudge(band(0), 1.0);
        assert_eq!(eq.preset_index(), None);
        assert_eq!(eq.preset_name(), "custom");
        eq.next_preset();
        assert_eq!(eq.preset_name(), PRESETS[0].name);
    }

    /// **The two readers cannot disagree.** Whatever the engine is handed is
    /// what the seal is told, for every state this type can be put in.
    #[test]
    fn what_the_engine_is_handed_is_what_the_seal_is_told() {
        let mut eq = GraphicEq::default();
        let check = |eq: &GraphicEq| {
            let (enabled, preamp, gains) = eq.engine_args();
            let seal = eq.seal_state();
            assert_eq!(enabled, seal.enabled);
            assert!((preamp - seal.preamp).abs() < f64::EPSILON);
            assert_eq!(gains.len(), seal.gains.len());
            for (i, (&sent, &claimed)) in gains.iter().zip(seal.gains.iter()).enumerate() {
                assert!(
                    (f64::from(sent) - claimed).abs() < f64::EPSILON,
                    "band {i}: engine {sent} vs seal {claimed}"
                );
            }
            // And the engine's own narrowing round-trips: what the DSP thread
            // ends up with is the number the seal reported.
            assert_eq!(preamp as f32, eq.value(PREAMP));
        };
        check(&eq);
        eq.set_enabled(true);
        check(&eq);
        for i in 0..PRESETS.len() {
            eq.load_preset(i);
            check(&eq);
        }
        for col in 0..COLUMNS {
            eq.nudge(col, -GAIN_STEP);
            check(&eq);
        }
    }

    /// The seal is told this is a graphic EQ, and told nothing about a
    /// parametric one — because this crate has none.
    #[test]
    fn the_seal_state_is_graphic_and_carries_no_parametric_claim() {
        let mut eq = GraphicEq::default();
        eq.set_enabled(true);
        eq.load_preset(1);
        let state = eq.seal_state();
        assert_eq!(state.mode, SealEqMode::Graphic);
        assert!(!state.param_enabled);
        assert_eq!(state.param_preamp, 0.0);
        assert!(state.param_bands.is_empty());
        assert_eq!(state.gains.len(), EQ_BAND_COUNT);
    }

    /// Labels come off `EQ_BANDS`, so they cannot name a band that is not there.
    #[test]
    fn the_columns_are_the_preamp_and_then_every_band_in_order() {
        assert_eq!(COLUMNS, EQ_BAND_COUNT + 1);
        assert_eq!(column_label(PREAMP), "pre");
        let labels: Vec<String> = (1..COLUMNS).map(column_label).collect();
        let expected: Vec<String> = EQ_BANDS
            .iter()
            .map(|&hz| eko_core::eq_presets::band_label(hz))
            .collect();
        assert_eq!(labels, expected);
        assert_eq!(band(0), 1);
        assert_eq!(band(EQ_BAND_COUNT - 1), COLUMNS - 1);
    }
}
