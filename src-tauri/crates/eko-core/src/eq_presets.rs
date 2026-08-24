//! The 10-band graphic-EQ preset table.
//!
//! Ported verbatim from `src/audio/constants.ts` so a terminal client can offer the
//! same presets as the GUI. **The names and the gain values are behaviour** — a
//! changed gain is an audible change, so [`preset_values_are_the_ported_table`] pins
//! every number.
//!
//! Because the bands are broad (Q ≈ 1) and overlapping, boosts add up — so each
//! preset carries a negative `preamp` for headroom, keeping the signal under 0 dBFS.
//! Gains run low frequency → high.
//!
//! The GUI still reads its own synchronous copy in `src/audio/constants.ts` (that
//! table is a skin data-feed binding, `constants.EQ_PRESETS`). Both sides are pinned
//! by tests; change one and you must change the other deliberately.

/// Classic Winamp 10-band graphic-EQ centre frequencies, in Hz.
///
/// The same frequencies the engine's `EQ_FREQS` uses.
pub const EQ_BANDS: [u32; 10] = [60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000];

/// Number of graphic-EQ bands.
pub const EQ_BAND_COUNT: usize = EQ_BANDS.len();

/// Per-band gain floor, in dB. Matches Winamp's ±12 dB sliders.
pub const EQ_GAIN_MIN: f32 = -12.0;

/// Per-band gain ceiling, in dB.
pub const EQ_GAIN_MAX: f32 = 12.0;

/// Q for each peaking filter. ~1.0 gives the broad, musical curves of the original.
pub const EQ_Q: f32 = 1.0;

/// One graphic-EQ preset.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqPreset {
    /// Display name, as the GUI and the CLI both show it.
    pub name: &'static str,
    /// Pre-amplifier gain, in dB. Negative values buy headroom for the boosts.
    pub preamp: f32,
    /// Per-band gains in dB, low frequency → high.
    pub gains: [f32; EQ_BAND_COUNT],
}

/// The preset table, in menu order. `Flat` is first and is the neutral reset.
pub const PRESETS: [EqPreset; 10] = [
    EqPreset {
        name: "Flat",
        preamp: 0.0,
        gains: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    EqPreset {
        name: "Rock",
        preamp: -5.0,
        gains: [5.0, 3.0, -1.0, -2.0, -1.0, 1.0, 3.0, 4.0, 5.0, 5.0],
    },
    EqPreset {
        name: "Pop",
        preamp: -3.0,
        gains: [-1.0, 1.0, 3.0, 4.0, 3.0, 1.0, -1.0, -1.0, 0.0, 1.0],
    },
    EqPreset {
        name: "Bass Boost",
        preamp: -5.0,
        gains: [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    EqPreset {
        name: "Treble Boost",
        preamp: -4.0,
        gains: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 4.0, 5.0, 5.0],
    },
    EqPreset {
        name: "Vocal",
        preamp: -3.0,
        gains: [-2.0, -2.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0],
    },
    EqPreset {
        name: "Jazz",
        preamp: -3.0,
        gains: [3.0, 2.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0],
    },
    EqPreset {
        name: "Acoustic",
        preamp: -3.0,
        gains: [3.0, 3.0, 1.0, 0.0, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0],
    },
    EqPreset {
        name: "Classical",
        preamp: -2.0,
        gains: [3.0, 2.0, 0.0, 0.0, 0.0, 0.0, -1.0, -1.0, -2.0, -3.0],
    },
    EqPreset {
        name: "Loudness",
        preamp: -5.0,
        gains: [6.0, 4.0, 0.0, -1.0, -2.0, -1.0, 0.0, 3.0, 5.0, 5.0],
    },
];

/// Display label for a band frequency — 1 kHz and up abbreviated as `"k"`.
///
/// Derived from the frequency so a label can never drift from its band.
pub fn band_label(hz: u32) -> String {
    if hz >= 1000 {
        let k = f64::from(hz) / 1000.0;
        if hz.is_multiple_of(1000) {
            format!("{k:.0}k")
        } else {
            format!("{k:.1}k")
        }
    } else {
        hz.to_string()
    }
}

/// A flat (all-zero) gain array — the neutral starting point.
pub fn flat_gains() -> [f32; EQ_BAND_COUNT] {
    [0.0; EQ_BAND_COUNT]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_exactly_ten_bands() {
        assert_eq!(EQ_BAND_COUNT, 10);
        for p in &PRESETS {
            assert_eq!(
                p.gains.len(),
                EQ_BAND_COUNT,
                "{} must carry one gain per band",
                p.name
            );
        }
    }

    /// Pins every name, preamp and gain against the transcription from
    /// `src/audio/constants.ts`. A diff here is an audible behaviour change.
    // Kept as a readable table — one row per preset, aligned with `constants.ts`.
    #[rustfmt::skip]
    #[test]
    fn preset_values_are_the_ported_table() {
        let expected: [(&str, f32, [f32; 10]); 10] = [
            ("Flat", 0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ("Rock", -5.0, [5.0, 3.0, -1.0, -2.0, -1.0, 1.0, 3.0, 4.0, 5.0, 5.0]),
            ("Pop", -3.0, [-1.0, 1.0, 3.0, 4.0, 3.0, 1.0, -1.0, -1.0, 0.0, 1.0]),
            ("Bass Boost", -5.0, [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ("Treble Boost", -4.0, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 4.0, 5.0, 5.0]),
            ("Vocal", -3.0, [-2.0, -2.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
            ("Jazz", -3.0, [3.0, 2.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0]),
            ("Acoustic", -3.0, [3.0, 3.0, 1.0, 0.0, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0]),
            ("Classical", -2.0, [3.0, 2.0, 0.0, 0.0, 0.0, 0.0, -1.0, -1.0, -2.0, -3.0]),
            ("Loudness", -5.0, [6.0, 4.0, 0.0, -1.0, -2.0, -1.0, 0.0, 3.0, 5.0, 5.0]),
        ];
        assert_eq!(PRESETS.len(), expected.len());
        for (got, (name, preamp, gains)) in PRESETS.iter().zip(expected) {
            assert_eq!(got.name, name);
            assert_eq!(got.preamp, preamp, "{name} preamp");
            assert_eq!(got.gains, gains, "{name} gains");
        }
    }

    #[test]
    fn flat_is_the_only_preset_that_touches_nothing() {
        for p in &PRESETS {
            let inert = p.preamp == 0.0 && p.gains.iter().all(|&g| g == 0.0);
            assert_eq!(inert, p.name == "Flat", "{} inertness", p.name);
        }
    }

    #[test]
    fn every_gain_is_inside_the_slider_range() {
        for p in &PRESETS {
            for &g in &p.gains {
                assert!(
                    (EQ_GAIN_MIN..=EQ_GAIN_MAX).contains(&g),
                    "{} has an out-of-range gain {g}",
                    p.name
                );
            }
        }
    }

    /// Every boosting preset must buy headroom, or overlapping broad bands clip.
    #[test]
    fn any_preset_that_boosts_carries_negative_preamp() {
        for p in &PRESETS {
            if p.gains.iter().any(|&g| g > 0.0) {
                assert!(p.preamp < 0.0, "{} boosts without headroom", p.name);
            }
        }
    }

    #[test]
    fn band_frequencies_and_labels_match_the_gui() {
        assert_eq!(
            EQ_BANDS,
            [60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000]
        );
        let labels: Vec<String> = EQ_BANDS.iter().map(|&hz| band_label(hz)).collect();
        assert_eq!(
            labels,
            ["60", "170", "310", "600", "1k", "3k", "6k", "12k", "14k", "16k"]
        );
        assert_eq!(EQ_Q, 1.0);
        assert_eq!(flat_gains(), [0.0; 10]);
    }
}
