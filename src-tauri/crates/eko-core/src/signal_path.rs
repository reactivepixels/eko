//! The user-facing signal-path seal: what EKO *reports* about the samples reaching
//! the DAC, and why.
//!
//! Ported from `src/hooks/useSignalPath.ts` so the GUI and the terminal client read
//! ONE implementation. This is deliberately NOT the same thing as
//! `engine::is_bitperfect`, which is the audio callback's private bypass decision —
//! this module produces the *reported* state together with the reasons it is not
//! bit-perfect.
//!
//! Everything here is pure: no I/O, no `Engine` handle. That is what makes the tests
//! at the bottom of this file the whole guard against the CLI and the GUI drifting.
//!
//! ## The contract, in one sentence
//!
//! Playback is reported bit-perfect only when NONE of five modifiers is engaged:
//! EQ, volume attenuation, ReplayGain, engine resampling, OS-device resampling.
//!
//! ## Free, and ungated on purpose
//!
//! The seal must be derivable in both the free and the Pro build, so nothing here is
//! behind `feature = "pro"` — including [`ParamFilter`], which mirrors the Pro
//! `pro::param_eq::ParamBandType`. `param_band_touches_signal_agrees_with_pro_param_band`
//! pins the two together in Pro builds so the mirror cannot drift.

use crate::engine::EngineStatus;

// ── The bit-perfect contract ──────────────────────────────────────────────────

/// The signal-path modifiers that, if any are engaged, forgo the bit-perfect
/// (untouched-samples) path.
///
/// Port of the TypeScript `SignalFlags` interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalFlags {
    /// An EQ (graphic or parametric) is shaping the signal.
    pub eq_active: bool,
    /// EKO's software volume is below unity.
    pub attenuated: bool,
    /// ReplayGain is applying a non-trivial adjustment.
    pub rg_active: bool,
    /// The engine resampled the source to the output rate.
    pub resampled: bool,
    /// The OS device runs at a different rate than the engine stream.
    pub os_resampled: bool,
}

/// Single definition of the bit-perfect contract: playback is bit-perfect only when
/// NONE of the modifiers are engaged.
///
/// Port of the TypeScript `isBitPerfect`.
pub fn is_bit_perfect(f: &SignalFlags) -> bool {
    !f.eq_active && !f.attenuated && !f.rg_active && !f.resampled && !f.os_resampled
}

// ── Inputs ────────────────────────────────────────────────────────────────────

/// The [`EngineStatus`] fields the seal reads, as a standalone owned snapshot.
///
/// `derive` takes this rather than an `&EngineStatus` for two reasons: `EngineStatus`
/// is `Serialize`-only (it cannot cross IPC inbound), and the GUI holds a *cached*
/// per-track copy of these fields rather than the live status. Use the
/// [`From<&EngineStatus>`] impl to go from a live engine snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    /// The rate the engine's output stream runs at, in Hz.
    pub rate: u32,
    /// The source file's native rate, in Hz. `0` when unknown.
    pub src_rate: u32,
    /// The OS output device's rate, in Hz. `0` when unknown.
    pub dev_rate: u32,
    /// Source bit depth. `0` when unknown (and then omitted from the SOURCE string).
    pub bits: u32,
    /// Source codec, as the engine reported it.
    pub codec: String,
    /// The output device the engine actually opened.
    pub device: String,
}

impl From<&EngineStatus> for StreamInfo {
    fn from(s: &EngineStatus) -> Self {
        Self {
            rate: s.rate,
            src_rate: s.src_rate,
            dev_rate: s.dev_rate,
            bits: s.bits,
            codec: s.codec.clone(),
            device: s.device.clone(),
        }
    }
}

/// Which EQ is routed to the DSP path, as the seal sees it.
///
/// A free mirror of `engine::EqMode`, whose `Parametric` variant is `feature = "pro"`.
/// Both variants exist in both builds so a free build can still deserialize — and
/// still report — a parametric configuration honestly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SealEqMode {
    /// 10-band graphic EQ (the free / default mode).
    #[default]
    Graphic,
    /// N-band parametric EQ (Pro).
    Parametric,
}

/// Filter type of one parametric band, as the seal sees it.
///
/// A free mirror of `pro::param_eq::ParamBandType`; the names match the TypeScript
/// `ParamBandType` union so the wire shape is unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParamFilter {
    /// Peaking bell.
    #[default]
    Peaking,
    /// Low shelf.
    LowShelf,
    /// High shelf.
    HighShelf,
    /// Low-pass.
    LowPass,
    /// High-pass.
    HighPass,
    /// Notch.
    Notch,
}

/// One parametric band, reduced to the fields that decide whether it touches the
/// samples. Extra keys on the wire (`freq`, `q`) are ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamBandState {
    /// The band's filter type.
    pub filter_type: ParamFilter,
    /// Gain in dB. Ignored for low-pass / high-pass / notch.
    pub gain_db: f64,
    /// Whether the band is switched on. An off band is pass-through.
    pub enabled: bool,
}

/// True when this band has a non-trivial effect on the signal.
///
/// Low-pass, high-pass and notch always colour the signal regardless of gain; the
/// peaking and shelf types only do so at a non-zero gain. Mirrors
/// `pro::param_eq::ParamBand::is_active`.
pub fn param_band_touches_signal(b: &ParamBandState) -> bool {
    if !b.enabled {
        return false;
    }
    match b.filter_type {
        ParamFilter::Peaking | ParamFilter::LowShelf | ParamFilter::HighShelf => b.gain_db != 0.0,
        ParamFilter::LowPass | ParamFilter::HighPass | ParamFilter::Notch => true,
    }
}

/// The EQ half of the DSP settings, exactly as the player store holds it.
///
/// Both EQs are carried because `mode` decides which one is routed — an engaged but
/// unrouted EQ does not touch the samples and must not break the seal.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqState {
    /// Which EQ is routed to the DSP path.
    pub mode: SealEqMode,
    /// Graphic EQ on/off.
    pub enabled: bool,
    /// Graphic EQ pre-amp, in dB.
    pub preamp: f64,
    /// Graphic EQ per-band gains, in dB.
    pub gains: Vec<f64>,
    /// Parametric EQ on/off.
    pub param_enabled: bool,
    /// Parametric EQ pre-amp, in dB.
    pub param_preamp: f64,
    /// Parametric EQ bands.
    pub param_bands: Vec<ParamBandState>,
}

impl EqState {
    /// True when the *routed* EQ is shaping the signal.
    ///
    /// Graphic: on, and either the pre-amp or any band gain is non-zero.
    /// Parametric: on, and either the pre-amp is non-zero or any band touches the
    /// signal (see [`param_band_touches_signal`]).
    pub fn active(&self) -> bool {
        match self.mode {
            SealEqMode::Graphic => {
                self.enabled && (self.preamp != 0.0 || self.gains.iter().any(|&g| g != 0.0))
            }
            SealEqMode::Parametric => {
                self.param_enabled
                    && (self.param_preamp != 0.0
                        || self.param_bands.iter().any(param_band_touches_signal))
            }
        }
    }
}

/// ReplayGain mode, for the RG picker's label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RgMode {
    /// Normalisation off — keeps the bit-perfect path.
    #[default]
    Off,
    /// Per-track normalisation.
    Track,
    /// Per-album normalisation.
    Album,
}

// ── ReplayGain: tags → the dB the engine gets, and the dB the seal reports ────
//
// These live beside `derive` because they decide the ONE boundary that settles whether
// EKO claims bit-perfect. They used to be `rgGainDbFor` and an inline `Math.abs(db) >
// 0.01` dead-band in `usePlayerStore.ts`, i.e. TypeScript-only — so a terminal client
// computing its own ReplayGain would have reported REPLAYGAIN where the GUI reported
// BIT-PERFECT for identical playback.

/// The ReplayGain tags a track carries. Gains in dB, peaks linear. All optional — a
/// server track, or a file with no tags, carries none, and then no adjustment applies.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGainTags {
    /// Per-track gain, in dB.
    pub track_gain: Option<f64>,
    /// Per-track peak, linear (1.0 = full scale).
    pub track_peak: Option<f64>,
    /// Per-album gain, in dB.
    pub album_gain: Option<f64>,
    /// Per-album peak, linear.
    pub album_peak: Option<f64>,
}

/// Adjustments smaller than this are inaudible and are reported as no adjustment at all,
/// keeping the bit-perfect path. Ported from `usePlayerStore.ts`'s `Math.abs(db) > 0.01`.
pub const RG_DEADBAND_DB: f64 = 0.01;

/// The ReplayGain adjustment for `mode`, in dB, **peak-limited** so a positive gain
/// cannot push the file's peak past full scale (clipping). `None` = no adjustment.
///
/// Album mode falls back to the track values when the album tags are absent; track mode
/// never falls back to album. Port of `rgGainDbFor`.
///
/// This is the value the ENGINE should receive. For what the SEAL reports, pass it
/// through [`applied_replaygain_db`] — or use [`seal_replaygain_db`], which does both.
pub fn replaygain_db(tags: &ReplayGainTags, mode: RgMode) -> Option<f64> {
    if mode == RgMode::Off {
        return None;
    }
    let (gain, peak) = if mode == RgMode::Album {
        (
            tags.album_gain.or(tags.track_gain),
            tags.album_peak.or(tags.track_peak),
        )
    } else {
        (tags.track_gain, tags.track_peak)
    };
    let gain = gain?;
    match peak {
        // Headroom (dB) before the peak clips.
        Some(p) if p > 0.0 => Some(gain.min(-20.0 * p.log10())),
        _ => Some(gain),
    }
}

/// The ReplayGain adjustment **as the seal may report it** — dead-banded.
///
/// A newtype rather than a bare `Option<f64>` so that handing [`derive`] a raw gain is a
/// *compile error* instead of a silent divergence.
///
/// This matters because the dead-band is the single boundary that settles whether EKO
/// claims bit-perfect: a front end that skipped it would report `REPLAYGAIN` where another
/// reports `BIT-PERFECT` for identical playback. A doc comment could not prevent that —
/// `SealInput` is a public-field struct, so `SealInput { replaygain_db: db, .. }` compiled
/// fine and read as correct.
///
/// # What the private field actually guarantees, and where it stops
///
/// The field is private, so **outside this module** the only ways to build one are
/// [`applied_replaygain_db`], [`seal_replaygain_db`] and [`replaygain_decision`], each of
/// which applies ±[`RG_DEADBAND_DB`]. That is the guarantee, and it is a guarantee about
/// *Rust callers* — the CLI included.
///
/// It is **not** the only constructor. `Deserialize` is derived, and a derived impl
/// expands inside the defining module, where the field is in scope: it therefore builds
/// `SealRgDb` from any bare number with no dead-band at all. `serde_json::from_str::<
/// SealRgDb>("-6.5")` is exercised directly in this file's tests. So the boundary is not
/// "no caller can skip the dead-band" but "no caller can skip it **in Rust**" — anything
/// arriving over the wire is outside the newtype's protection.
///
/// The desktop app is on that far side. [`SealInput`] also derives `Deserialize`, so
/// `signal_path(input: SealInput)` in `eko-tauri`'s `commands/signal.rs` takes
/// `replaygainDb` straight off the IPC payload without passing it through any of the three
/// functions above.
///
/// This is safe, and safe by direction rather than by accident: skipping the dead-band can
/// only turn a negligible 0.004 dB into `rg_active`, i.e. `REPLAYGAIN` where the dead-band
/// would have said `BIT-PERFECT`. It **under**-claims, and it cannot manufacture a
/// `BIT-PERFECT` — for that a caller would have to send `null`, which is the honest value
/// for "no adjustment" anyway. In practice the point is moot: `usePlayerStore.ts` sets
/// `rgAppliedDb` from the `sealDb` that [`replaygain_decision`] returned, so the number the
/// frontend sends back has already been dead-banded by this module.
///
/// `#[serde(transparent)]` keeps the wire shape a plain nullable number, so the IPC
/// payload and the frontend's `replaygainDb` key are unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SealRgDb(Option<f64>);

impl SealRgDb {
    /// The dead-banded dB, or `None` when ReplayGain is inactive for the seal.
    pub fn get(self) -> Option<f64> {
        self.0
    }
}

/// Normalise an adjustment to what the seal should report: anything inside
/// ±[`RG_DEADBAND_DB`] becomes `None`, so a negligible correction does not break the
/// seal. Port of `rgAppliedDb: db != null && Math.abs(db) > 0.01 ? db : null`.
pub fn applied_replaygain_db(db: Option<f64>) -> SealRgDb {
    SealRgDb(db.filter(|d| d.abs() > RG_DEADBAND_DB))
}

/// Tags + mode → the dB the seal should report. The whole ReplayGain pipeline in one
/// call, so a front end cannot apply the peak limit without the dead-band or vice versa.
pub fn seal_replaygain_db(tags: &ReplayGainTags, mode: RgMode) -> SealRgDb {
    applied_replaygain_db(replaygain_db(tags, mode))
}

/// Both ReplayGain values a front end needs, from one derivation.
///
/// Returned as a pair because they are NOT interchangeable and computing one without the
/// other is the bug this type exists to prevent: `engine_db` is peak-limited but raw, and
/// `seal_db` additionally has the dead-band applied.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGainDecision {
    /// The dB to hand the engine. `None` = disable ReplayGain.
    pub engine_db: Option<f64>,
    /// The dB the seal reports. `None` = the seal treats ReplayGain as inactive.
    pub seal_db: SealRgDb,
}

/// Decide both ReplayGain values for a track under `mode`.
pub fn replaygain_decision(tags: &ReplayGainTags, mode: RgMode) -> ReplayGainDecision {
    let engine_db = replaygain_db(tags, mode);
    ReplayGainDecision {
        engine_db,
        seal_db: applied_replaygain_db(engine_db),
    }
}

/// Everything [`derive`] needs: a stream snapshot plus the DSP settings.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealInput {
    /// True while the native engine is the audio source.
    pub engine_active: bool,
    /// The live stream snapshot; `None` before the engine reports one.
    pub info: Option<StreamInfo>,
    /// EQ settings.
    pub eq: EqState,
    /// EKO's software volume, `0.0`–`1.0`. Unity is `1.0`.
    pub volume: f64,
    /// The ReplayGain adjustment the seal should report. `None` = none applied.
    ///
    /// A [`SealRgDb`], so it can only have come from [`seal_replaygain_db`] /
    /// [`applied_replaygain_db`] / [`replaygain_decision`] and the dead-band cannot be
    /// skipped. `derive` itself stays an exact port of the TypeScript's
    /// `rgAppliedDb != null` and treats any `Some` as active — including `Some(0.0)`.
    pub replaygain_db: SealRgDb,
    /// The ReplayGain mode the user selected.
    pub replaygain_mode: RgMode,
}

// ── Output ────────────────────────────────────────────────────────────────────

/// The reported signal path: the seal, its breakdown, and every display string the
/// `SOURCE → OUTPUT` chain renders.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalPath {
    /// Whether a full signal-path display has live data to show. When false the
    /// consumer must render nothing — never a default seal.
    pub active: bool,
    /// The seal itself: true only when the samples are untouched.
    pub pure: bool,
    /// Breakdown — which modifiers are engaged.
    #[serde(flatten)]
    pub flags: SignalFlags,
    /// Source codec, upper-cased (`"AUDIO"` when unknown).
    pub codec: String,
    /// The SOURCE node's value, e.g. `"FLAC · 96 kHz · 24-bit"`.
    pub src: String,
    /// The OUTPUT node's value, e.g. `"Topping E30 · 96 kHz"`.
    pub output: String,
    /// Long-form processing description, e.g. `"Resampled → 48 kHz · EQ"`.
    pub engine_label: String,
    /// The seal's label, e.g. `"BIT-PERFECT"` or `"EQ · VOLUME"`.
    pub seal_label: String,
    /// The ReplayGain node's value, e.g. `"Album · -6.5 dB"`.
    pub rg_label: String,
}

// ── Display helpers ───────────────────────────────────────────────────────────

/// Format to one decimal place **exactly as JavaScript's `Number.prototype.toFixed(1)`
/// does**, because that is what the shipping app's dB labels are.
///
/// Rust's `{:.1}` rounds half-to-even; `toFixed` rounds half **away from zero**, applied
/// to the magnitude (ECMA-262 sets the sign aside first, then picks the larger candidate
/// on a tie). They differ on exact binary ties: a ReplayGain tag of `-7.25` dB labels
/// `-7.3` in the browser and would label `-7.2` under `{:.1}`.
///
/// The tie must be judged on the double's **exact** decimal expansion, not on `x * 10.0`
/// — that multiplication rounds `0.15` (stored as `0.1499999…`) up to exactly `1.5` and
/// would produce `0.2` where JavaScript gives `0.1`. So the digits are read from a
/// high-precision rendering of the exact value: round up iff the second fractional digit
/// is `>= 5`, which covers both "past half" and "exactly half" in one test.
///
/// # Known divergences from `toFixed(1)`
///
/// Both are far outside any reachable dB or kHz value, and neither can produce a false
/// `BIT-PERFECT` — only an odd-looking label:
///
/// * `|x| >= 1e21` — ECMA-262 makes `toFixed` fall back to `ToString`, so JavaScript
///   returns `"1e+21"` where this returns the digits. Nothing here reproduces that.
/// * non-finite — JavaScript renders `"Infinity"` / `"NaN"`; Rust renders `"inf"` / `"NaN"`.
fn to_fixed_1(x: f64) -> String {
    if !x.is_finite() {
        // Not reachable for a dB value; never panic on one.
        return format!("{x:.1}");
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let m = x.abs();
    // 30 places is far beyond the ~17 significant decimals a f64 can distinguish, so the
    // two digits read below are never affected by rounding at the cut.
    let exact = format!("{m:.30}");
    let Some((int_part, frac)) = exact.split_once('.') else {
        return format!("{x:.1}");
    };
    let bytes = frac.as_bytes();
    // A hostile server can reach this with an absurd gain: `eko-net` parses ReplayGain
    // tags leniently from strings, so `"1e38"` arrives as a real f64. Both the parse and
    // the ×10 must therefore be fallible — `int_val * 10` panicked in debug and wrapped
    // silently in release for `|x|` in roughly [1e38, 3.4e38].
    let Ok(int_val) = int_part.parse::<u128>() else {
        return format!("{x:.1}");
    };
    let Some(tenths) = int_val
        .checked_mul(10)
        .and_then(|t| t.checked_add(u128::from(bytes[0] - b'0')))
    else {
        return format!("{x:.1}");
    };
    let Some(rounded) = (if bytes[1] >= b'5' {
        tenths.checked_add(1)
    } else {
        Some(tenths)
    }) else {
        return format!("{x:.1}");
    };
    format!("{sign}{}.{}", rounded / 10, rounded % 10)
}

/// Format a sample rate in kHz, one decimal only when the rate is not a whole kHz.
/// `0` renders as an em-dash. Port of the TypeScript `khz`.
fn khz(n: u32) -> String {
    if n == 0 {
        return "—".to_string();
    }
    let k = f64::from(n) / 1000.0;
    if n.is_multiple_of(1000) {
        // A whole number of kHz — `k` is exact, so no tie-breaking is possible.
        format!("{k:.0} kHz")
    } else {
        format!("{} kHz", to_fixed_1(k))
    }
}

/// Join the non-empty parts with the chain separator the UI uses.
fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

// ── The derivation ────────────────────────────────────────────────────────────

/// Derive the reported signal path from a stream snapshot and the DSP settings.
///
/// Pure — the single source of bit-perfect truth for every EKO front end.
///
/// The five modifiers are independent, and the labels **concatenate in a fixed
/// order**; no modifier outranks another. EQ at half volume reports
/// `"EQ · VOLUME"`, not one or the other.
pub fn derive(input: &SealInput) -> SignalPath {
    let info = input.info.as_ref();

    // A full display needs the engine to be the source AND to have reported a rate.
    let active = input.engine_active && info.is_some_and(|i| i.rate != 0);

    let flags = SignalFlags {
        eq_active: input.eq.active(),
        attenuated: input.volume < 1.0,
        rg_active: input.replaygain_db.get().is_some(),
        resampled: info.is_some_and(|i| i.src_rate > 0 && i.src_rate != i.rate),
        os_resampled: info.is_some_and(|i| i.dev_rate > 0 && i.dev_rate != i.rate),
    };
    let pure = is_bit_perfect(&flags);

    let codec = info
        .map(|i| {
            if i.codec.is_empty() {
                "AUDIO".to_string()
            } else {
                i.codec.to_uppercase()
            }
        })
        .unwrap_or_else(|| "AUDIO".to_string());

    let src = match info {
        Some(i) => {
            let bits = if i.bits != 0 {
                format!(" · {}-bit", i.bits)
            } else {
                String::new()
            };
            format!("{codec} · {}{bits}", khz(i.src_rate))
        }
        None => String::new(),
    };

    let output = match info {
        Some(i) => {
            let device = if i.device.is_empty() {
                "Output"
            } else {
                i.device.as_str()
            };
            format!("{device} · {}", khz(i.rate))
        }
        None => String::new(),
    };

    let engine_label = if pure {
        "Bit-perfect".to_string()
    } else {
        join(&[
            match (flags.resampled, info) {
                (true, Some(i)) => format!("Resampled → {}", khz(i.rate)),
                _ => String::new(),
            },
            match (flags.os_resampled, info) {
                (true, Some(i)) => format!("OS resample → {}", khz(i.dev_rate)),
                _ => String::new(),
            },
            if flags.eq_active { "EQ" } else { "" }.to_string(),
            if flags.attenuated { "Volume" } else { "" }.to_string(),
            match (flags.rg_active, input.replaygain_db.get()) {
                (true, Some(db)) => format!("ReplayGain {} dB", to_fixed_1(db)),
                _ => String::new(),
            },
        ])
    };

    let seal_label = if pure {
        "BIT-PERFECT".to_string()
    } else {
        let joined = join(&[
            if flags.resampled || flags.os_resampled {
                "RESAMPLED"
            } else {
                ""
            }
            .to_string(),
            if flags.eq_active { "EQ" } else { "" }.to_string(),
            if flags.attenuated { "VOLUME" } else { "" }.to_string(),
            if flags.rg_active { "REPLAYGAIN" } else { "" }.to_string(),
        ]);
        // Unreachable given `pure`: every modifier that clears `pure` contributes a
        // label above. Kept as the TypeScript's defensive fallback so the seal can
        // never render empty even if that invariant is ever broken.
        if joined.is_empty() {
            "PROCESSED".to_string()
        } else {
            joined
        }
    };

    let rg_label = match input.replaygain_mode {
        RgMode::Off => "Off".to_string(),
        mode => {
            let base = if mode == RgMode::Album {
                "Album"
            } else {
                "Track"
            };
            match input.replaygain_db.get() {
                Some(db) => format!("{base} · {} dB", to_fixed_1(db)),
                None => base.to_string(),
            }
        }
    };

    SignalPath {
        active,
        pure,
        flags,
        codec,
        src,
        output,
        engine_label,
        seal_label,
        rg_label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Ported 1:1 from `src/signalPath.test.ts`'s `isBitPerfect` describe ────────
    //
    // The TypeScript built each case from a `clean` literal and spread one override
    // over it; `SignalFlags::default()` is that `clean`.

    #[test]
    fn is_true_only_when_no_signal_path_modifier_is_engaged() {
        assert!(is_bit_perfect(&SignalFlags::default()));
    }

    #[test]
    fn eq_active_engaged_is_not_bit_perfect() {
        assert!(!is_bit_perfect(&SignalFlags {
            eq_active: true,
            ..Default::default()
        }));
    }

    #[test]
    fn attenuated_engaged_is_not_bit_perfect() {
        assert!(!is_bit_perfect(&SignalFlags {
            attenuated: true,
            ..Default::default()
        }));
    }

    #[test]
    fn rg_active_engaged_is_not_bit_perfect() {
        assert!(!is_bit_perfect(&SignalFlags {
            rg_active: true,
            ..Default::default()
        }));
    }

    #[test]
    fn resampled_engaged_is_not_bit_perfect() {
        assert!(!is_bit_perfect(&SignalFlags {
            resampled: true,
            ..Default::default()
        }));
    }

    #[test]
    fn os_resampled_engaged_is_not_bit_perfect() {
        assert!(!is_bit_perfect(&SignalFlags {
            os_resampled: true,
            ..Default::default()
        }));
    }

    #[test]
    fn any_combination_of_modifiers_is_not_bit_perfect() {
        assert!(!is_bit_perfect(&SignalFlags {
            eq_active: true,
            attenuated: true,
            ..Default::default()
        }));
        assert!(!is_bit_perfect(&SignalFlags {
            rg_active: true,
            resampled: true,
            ..Default::default()
        }));
    }

    // ── `derive` ─────────────────────────────────────────────────────────────────

    /// A clean bit-perfect input: playing 96/24 FLAC to a device running at the
    /// stream rate, unity volume, no EQ, no ReplayGain.
    fn clean_input() -> SealInput {
        SealInput {
            engine_active: true,
            info: Some(StreamInfo {
                rate: 96_000,
                src_rate: 96_000,
                dev_rate: 96_000,
                bits: 24,
                codec: "flac".into(),
                device: "Topping E30".into(),
            }),
            eq: EqState {
                gains: vec![0.0; 10],
                ..Default::default()
            },
            volume: 1.0,
            replaygain_db: SealRgDb::default(),
            replaygain_mode: RgMode::Off,
        }
    }

    #[test]
    fn a_clean_path_reports_the_bit_perfect_seal_and_the_full_chain() {
        let p = derive(&clean_input());
        assert!(p.active);
        assert!(p.pure);
        assert_eq!(p.seal_label, "BIT-PERFECT");
        assert_eq!(p.engine_label, "Bit-perfect");
        assert_eq!(p.codec, "FLAC");
        assert_eq!(p.src, "FLAC · 96 kHz · 24-bit");
        assert_eq!(p.output, "Topping E30 · 96 kHz");
        assert_eq!(p.rg_label, "Off");
    }

    #[test]
    fn nothing_is_active_until_the_engine_reports_a_rate() {
        // No engine.
        let mut i = clean_input();
        i.engine_active = false;
        assert!(!derive(&i).active);

        // Engine, but no stream info yet.
        let mut i = clean_input();
        i.info = None;
        assert!(!derive(&i).active);
        assert_eq!(derive(&i).src, "");
        assert_eq!(derive(&i).output, "");

        // Engine and info, but rate 0.
        let mut i = clean_input();
        i.info.as_mut().unwrap().rate = 0;
        assert!(!derive(&i).active);
    }

    /// The precedence question: several modifiers at once do NOT pick a winner —
    /// they concatenate, in a fixed order.
    #[test]
    fn several_modifiers_at_once_concatenate_in_a_fixed_order() {
        let mut i = clean_input();
        i.eq.enabled = true;
        i.eq.preamp = -3.0;
        i.volume = 0.5;
        i.replaygain_db = applied_replaygain_db(Some(-6.5));
        i.info.as_mut().unwrap().src_rate = 44_100;
        i.info.as_mut().unwrap().dev_rate = 48_000;

        let p = derive(&i);
        assert!(!p.pure);
        // RESAMPLED covers both resample reasons and appears once, first.
        assert_eq!(p.seal_label, "RESAMPLED · EQ · VOLUME · REPLAYGAIN");
        assert_eq!(
            p.engine_label,
            "Resampled → 96 kHz · OS resample → 48 kHz · EQ · Volume · ReplayGain -6.5 dB"
        );
    }

    #[test]
    fn eq_and_volume_together_report_both_neither_outranks_the_other() {
        let mut i = clean_input();
        i.eq.enabled = true;
        i.eq.gains[3] = 2.0;
        i.volume = 0.8;
        assert_eq!(derive(&i).seal_label, "EQ · VOLUME");
        assert_eq!(derive(&i).engine_label, "EQ · Volume");
    }

    #[test]
    fn either_resample_reason_alone_reports_resampled_once() {
        let mut i = clean_input();
        i.info.as_mut().unwrap().src_rate = 44_100;
        let p = derive(&i);
        assert!(p.flags.resampled && !p.flags.os_resampled);
        assert_eq!(p.seal_label, "RESAMPLED");
        assert_eq!(p.engine_label, "Resampled → 96 kHz");

        let mut i = clean_input();
        i.info.as_mut().unwrap().dev_rate = 44_100;
        let p = derive(&i);
        assert!(!p.flags.resampled && p.flags.os_resampled);
        assert_eq!(p.seal_label, "RESAMPLED");
        assert_eq!(p.engine_label, "OS resample → 44.1 kHz");
    }

    #[test]
    fn an_unknown_rate_is_not_treated_as_a_resample() {
        let mut i = clean_input();
        i.info.as_mut().unwrap().src_rate = 0;
        i.info.as_mut().unwrap().dev_rate = 0;
        let p = derive(&i);
        assert!(p.pure, "rate 0 means unknown, not different");
        assert_eq!(p.src, "FLAC · — · 24-bit");
    }

    #[test]
    fn volume_breaks_the_seal_only_below_unity() {
        let mut i = clean_input();
        i.volume = 1.0;
        assert!(derive(&i).pure);
        i.volume = 0.999;
        assert!(!derive(&i).pure);
        assert_eq!(derive(&i).seal_label, "VOLUME");
    }

    #[test]
    fn replaygain_labels_carry_the_applied_db_and_the_mode() {
        let mut i = clean_input();
        i.replaygain_mode = RgMode::Track;
        // Mode chosen but nothing applied (no tags) → still bit-perfect.
        assert!(derive(&i).pure);
        assert_eq!(derive(&i).rg_label, "Track");

        i.replaygain_db = applied_replaygain_db(Some(-7.25));
        let p = derive(&i);
        assert!(!p.pure);
        assert_eq!(p.seal_label, "REPLAYGAIN");
        // -7.3, not -7.2: JS `toFixed(1)` rounds the magnitude away from zero on a tie,
        // and this label must match what the shipping app renders.
        assert_eq!(p.rg_label, "Track · -7.3 dB");

        i.replaygain_mode = RgMode::Album;
        i.replaygain_db = applied_replaygain_db(Some(-6.5));
        assert_eq!(derive(&i).rg_label, "Album · -6.5 dB");
        assert_eq!(derive(&i).engine_label, "ReplayGain -6.5 dB");

        // Off never shows a value, even if one is somehow applied.
        i.replaygain_mode = RgMode::Off;
        assert_eq!(derive(&i).rg_label, "Off");
    }

    // ── ReplayGain ───────────────────────────────────────────────────────────────

    /// Port check for `rgGainDbFor`: mode selection and the album→track fallback.
    #[test]
    fn replaygain_reads_the_tags_for_the_chosen_mode() {
        let tags = ReplayGainTags {
            track_gain: Some(-6.0),
            track_peak: None,
            album_gain: Some(-4.0),
            album_peak: None,
        };
        assert_eq!(
            replaygain_db(&tags, RgMode::Off),
            None,
            "off applies nothing"
        );
        assert_eq!(replaygain_db(&tags, RgMode::Track), Some(-6.0));
        assert_eq!(replaygain_db(&tags, RgMode::Album), Some(-4.0));

        // Album mode falls back to the TRACK gain when there is no album gain.
        let no_album = ReplayGainTags {
            album_gain: None,
            ..tags
        };
        assert_eq!(replaygain_db(&no_album, RgMode::Album), Some(-6.0));

        // Track mode never falls back to the album gain.
        let no_track = ReplayGainTags {
            track_gain: None,
            ..tags
        };
        assert_eq!(replaygain_db(&no_track, RgMode::Track), None);

        // No usable tags at all → no adjustment, so the seal stays bit-perfect.
        assert_eq!(
            replaygain_db(&ReplayGainTags::default(), RgMode::Album),
            None
        );
    }

    #[test]
    fn replaygain_is_peak_limited_so_a_boost_cannot_clip() {
        // peak 0.5 → 6.02 dB of headroom, so a +10 dB gain is clamped to it.
        let tags = ReplayGainTags {
            track_gain: Some(10.0),
            track_peak: Some(0.5),
            ..Default::default()
        };
        let db = replaygain_db(&tags, RgMode::Track).unwrap();
        assert!((db - 6.0206).abs() < 0.001, "expected ~6.02 dB, got {db}");

        // A gain already below the headroom is untouched.
        let quiet = ReplayGainTags {
            track_gain: Some(-6.0),
            track_peak: Some(0.5),
            ..Default::default()
        };
        assert_eq!(replaygain_db(&quiet, RgMode::Track), Some(-6.0));

        // A peak at full scale leaves no headroom at all.
        let hot = ReplayGainTags {
            track_gain: Some(3.0),
            track_peak: Some(1.0),
            ..Default::default()
        };
        assert_eq!(replaygain_db(&hot, RgMode::Track), Some(0.0));

        // A missing or nonsensical peak disables the limit rather than the gain.
        for peak in [None, Some(0.0), Some(-1.0)] {
            let t = ReplayGainTags {
                track_gain: Some(4.0),
                track_peak: peak,
                ..Default::default()
            };
            assert_eq!(replaygain_db(&t, RgMode::Track), Some(4.0), "peak {peak:?}");
        }

        // Album mode falls back to the TRACK peak when the album peak is absent.
        let album = ReplayGainTags {
            track_gain: Some(10.0),
            track_peak: Some(0.5),
            album_gain: Some(10.0),
            album_peak: None,
        };
        let db = replaygain_db(&album, RgMode::Album).unwrap();
        assert!((db - 6.0206).abs() < 0.001, "album fell back to track peak");
    }

    /// The dead-band is the boundary that decides whether EKO claims bit-perfect, so it
    /// is pinned on both sides of ±0.01 dB.
    #[test]
    fn the_replaygain_dead_band_normalises_inaudible_adjustments_away() {
        assert_eq!(RG_DEADBAND_DB, 0.01);
        assert_eq!(applied_replaygain_db(None).get(), None);
        // Inside the band, inclusive of the boundary itself (`> 0.01`, not `>=`).
        for db in [0.0, -0.0, 0.001, -0.001, 0.004, -0.004, 0.01, -0.01] {
            assert_eq!(
                applied_replaygain_db(Some(db)).get(),
                None,
                "{db} dB is inaudible"
            );
        }
        // Outside it, the value passes through unchanged.
        for db in [0.011, -0.011, 0.02, -6.5, 12.0] {
            assert_eq!(
                applied_replaygain_db(Some(db)).get(),
                Some(db),
                "{db} dB is real"
            );
        }
    }

    /// The whole pipeline, and the divergence it exists to prevent: a front end that
    /// computes its own ReplayGain must land on the SAME seal as the GUI.
    #[test]
    fn the_seal_replaygain_pipeline_agrees_with_the_gui_at_the_dead_band() {
        // A track whose tags produce a negligible correction. The GUI reports BIT-PERFECT
        // here, so a CLI must too.
        let negligible = ReplayGainTags {
            track_gain: Some(0.004),
            ..Default::default()
        };
        assert_eq!(replaygain_db(&negligible, RgMode::Track), Some(0.004));
        assert_eq!(seal_replaygain_db(&negligible, RgMode::Track).get(), None);

        let mut i = clean_input();
        i.replaygain_mode = RgMode::Track;
        i.replaygain_db = seal_replaygain_db(&negligible, RgMode::Track);
        let p = derive(&i);
        assert!(p.pure, "an inaudible correction must not break the seal");
        assert_eq!(p.seal_label, "BIT-PERFECT");

        // And a real correction still does break it.
        let real = ReplayGainTags {
            track_gain: Some(-6.5),
            ..Default::default()
        };
        i.replaygain_db = seal_replaygain_db(&real, RgMode::Track);
        assert_eq!(derive(&i).seal_label, "REPLAYGAIN");
    }

    #[test]
    fn the_replaygain_decision_carries_the_engine_and_seal_values_separately() {
        // The engine gets the peak-limited value; the seal gets it dead-banded away.
        let negligible = ReplayGainTags {
            track_gain: Some(0.004),
            ..Default::default()
        };
        let d = replaygain_decision(&negligible, RgMode::Track);
        assert_eq!(d.engine_db, Some(0.004));
        assert_eq!(d.seal_db.get(), None);

        let real = ReplayGainTags {
            track_gain: Some(10.0),
            track_peak: Some(0.5),
            ..Default::default()
        };
        let d = replaygain_decision(&real, RgMode::Album);
        assert_eq!(
            d.engine_db,
            d.seal_db.get(),
            "a real gain is reported as applied"
        );
        assert!((d.engine_db.unwrap() - 6.0206).abs() < 0.001);

        let d = replaygain_decision(&real, RgMode::Off);
        assert_eq!(d, ReplayGainDecision::default(), "off decides nothing");
    }

    /// `SealRgDb` is a newtype, so it MUST stay `#[serde(transparent)]` — the frontend
    /// reads `sealDb` / `replaygainDb` as a plain nullable number. Without `transparent`
    /// these would serialize as a wrapper and the seal would silently lose its dB.
    #[test]
    fn the_seal_replaygain_newtype_is_transparent_on_the_wire() {
        let d = replaygain_decision(
            &ReplayGainTags {
                track_gain: Some(-6.5),
                ..Default::default()
            },
            RgMode::Track,
        );
        assert_eq!(
            serde_json::to_string(&d).unwrap(),
            r#"{"engineDb":-6.5,"sealDb":-6.5}"#
        );
        assert_eq!(
            serde_json::to_string(&ReplayGainDecision::default()).unwrap(),
            r#"{"engineDb":null,"sealDb":null}"#
        );

        // And the seal's own input round-trips from a bare number.
        let back: SealRgDb = serde_json::from_str("-6.5").unwrap();
        assert_eq!(back.get(), Some(-6.5));
        let back: SealRgDb = serde_json::from_str("null").unwrap();
        assert_eq!(back.get(), None);
    }

    /// `derive` is an exact port of `rgAppliedDb != null` — NOT truthiness — so a raw
    /// `Some(0.0)` reports as active. This pins that port semantics, which the
    /// differential fuzz against the pre-move TypeScript is calibrated against.
    ///
    /// Note the tuple constructor is reachable only from inside this module: [`SealRgDb`]'s
    /// field is private, so no front end can build this state. That is Fix 3's whole point
    /// — the case below is pinned, and simultaneously unconstructible by a caller.
    #[test]
    fn derive_treats_an_explicit_zero_db_as_applied_replaygain() {
        let mut i = clean_input();
        i.replaygain_mode = RgMode::Track;
        i.replaygain_db = SealRgDb(Some(0.0));
        let p = derive(&i);
        assert!(p.flags.rg_active, "Some(0.0) is not None");
        assert!(!p.pure);
        assert_eq!(p.seal_label, "REPLAYGAIN");
        assert_eq!(p.engine_label, "ReplayGain 0.0 dB");
        assert_eq!(p.rg_label, "Track · 0.0 dB");

        // Routed through the pipeline instead, the same zero normalises away — which is
        // what the GUI does, and therefore what a CLI must do.
        i.replaygain_db = applied_replaygain_db(Some(0.0));
        assert_eq!(derive(&i).seal_label, "BIT-PERFECT");
    }

    // ── EQ activity ──────────────────────────────────────────────────────────────

    #[test]
    fn a_switched_on_but_flat_graphic_eq_stays_bit_perfect() {
        let mut i = clean_input();
        i.eq.enabled = true;
        assert!(derive(&i).pure, "flat EQ touches nothing");

        i.eq.preamp = -3.0;
        assert!(!derive(&i).pure, "a preamp alone breaks the seal");

        i.eq.preamp = 0.0;
        i.eq.gains[9] = -1.0;
        assert!(!derive(&i).pure, "one non-zero band breaks the seal");

        // Switched off, the same shaping is inert.
        i.eq.enabled = false;
        assert!(derive(&i).pure);
    }

    #[test]
    fn an_engaged_but_unrouted_eq_does_not_break_the_seal() {
        // Parametric EQ fully engaged, but the graphic EQ is the routed mode.
        let mut i = clean_input();
        i.eq.mode = SealEqMode::Graphic;
        i.eq.param_enabled = true;
        i.eq.param_preamp = -6.0;
        i.eq.param_bands = vec![ParamBandState {
            filter_type: ParamFilter::Peaking,
            gain_db: 4.0,
            enabled: true,
        }];
        assert!(derive(&i).pure, "an unrouted EQ touches nothing");

        // Route it and the seal breaks.
        i.eq.mode = SealEqMode::Parametric;
        assert_eq!(derive(&i).seal_label, "EQ");

        // The reverse: graphic engaged while parametric is routed.
        let mut i = clean_input();
        i.eq.mode = SealEqMode::Parametric;
        i.eq.enabled = true;
        i.eq.preamp = -6.0;
        assert!(derive(&i).pure);
    }

    #[test]
    fn cut_filters_break_the_seal_at_zero_gain_but_bells_do_not() {
        let bell = |t: ParamFilter| ParamBandState {
            filter_type: t,
            gain_db: 0.0,
            enabled: true,
        };
        for t in [
            ParamFilter::LowPass,
            ParamFilter::HighPass,
            ParamFilter::Notch,
        ] {
            assert!(
                param_band_touches_signal(&bell(t)),
                "{t:?} colours the signal regardless of gain"
            );
        }
        for t in [
            ParamFilter::Peaking,
            ParamFilter::LowShelf,
            ParamFilter::HighShelf,
        ] {
            assert!(
                !param_band_touches_signal(&bell(t)),
                "{t:?} at 0 dB is a no-op"
            );
            assert!(param_band_touches_signal(&ParamBandState {
                gain_db: 1.5,
                ..bell(t)
            }));
        }
        // A disabled band is inert whatever its type.
        for t in [
            ParamFilter::LowPass,
            ParamFilter::Peaking,
            ParamFilter::Notch,
        ] {
            assert!(!param_band_touches_signal(&ParamBandState {
                enabled: false,
                gain_db: 6.0,
                filter_type: t,
            }));
        }
    }

    /// The free [`ParamFilter`] mirror must agree with the Pro band type it mirrors.
    /// If a filter type is ever added to one and not the other, this fails.
    #[cfg(feature = "pro")]
    #[test]
    fn param_band_touches_signal_agrees_with_pro_param_band() {
        use crate::pro::param_eq::{ParamBand, ParamBandType};

        let pairs = [
            (ParamFilter::Peaking, ParamBandType::Peaking),
            (ParamFilter::LowShelf, ParamBandType::LowShelf),
            (ParamFilter::HighShelf, ParamBandType::HighShelf),
            (ParamFilter::LowPass, ParamBandType::LowPass),
            (ParamFilter::HighPass, ParamBandType::HighPass),
            (ParamFilter::Notch, ParamBandType::Notch),
        ];
        for (free, pro) in pairs {
            for gain in [0.0f32, 3.0, -3.0] {
                for enabled in [true, false] {
                    let seal = param_band_touches_signal(&ParamBandState {
                        filter_type: free,
                        gain_db: f64::from(gain),
                        enabled,
                    });
                    let engine = ParamBand {
                        filter_type: pro,
                        freq: 1000.0,
                        gain_db: gain,
                        q: 1.0,
                        enabled,
                    }
                    .is_active();
                    assert_eq!(
                        seal, engine,
                        "seal and engine disagree for {free:?} gain={gain} enabled={enabled}"
                    );
                }
            }
        }
    }

    // ── Display strings ──────────────────────────────────────────────────────────

    /// Every value here was taken from Node's actual `toFixed(1)` output, not derived
    /// from the spec — the tie-break is on the double's exact binary expansion, which is
    /// easy to reason about wrongly.
    #[test]
    fn to_fixed_1_matches_javascripts_to_fixed() {
        // Exact binary ties round the MAGNITUDE away from zero (Rust's `{:.1}` would
        // round half-to-even and give -7.2 / 7.2 / -6.2 / 6.2 here).
        assert_eq!(to_fixed_1(-7.25), "-7.3");
        assert_eq!(to_fixed_1(7.25), "7.3");
        assert_eq!(to_fixed_1(-6.25), "-6.3");
        assert_eq!(to_fixed_1(6.25), "6.3");
        assert_eq!(to_fixed_1(-12.75), "-12.8");
        assert_eq!(to_fixed_1(-0.05), "-0.1");
        assert_eq!(to_fixed_1(0.05), "0.1");

        // NOT ties: these decimals are not representable, and the stored double sits just
        // below the halfway point — so they round DOWN. Scaling by 10 first would round
        // 0.15 up to exactly 1.5 and wrongly produce "0.2".
        assert_eq!(to_fixed_1(0.15), "0.1");
        assert_eq!(to_fixed_1(-0.15), "-0.1");
        assert_eq!(to_fixed_1(7.35), "7.3");
        assert_eq!(to_fixed_1(-7.35), "-7.3");
        assert_eq!(to_fixed_1(-6.05), "-6.0");
        assert_eq!(to_fixed_1(1.005), "1.0");

        // Nothing to round.
        assert_eq!(to_fixed_1(-6.5), "-6.5");
        assert_eq!(to_fixed_1(2.5), "2.5");
        assert_eq!(to_fixed_1(0.0), "0.0");
        assert_eq!(to_fixed_1(-0.0), "0.0", "JS renders negative zero as 0.0");

        // Carry out of the tenths digit.
        assert_eq!(to_fixed_1(6.96), "7.0");
        assert_eq!(to_fixed_1(-9.99), "-10.0");
    }

    /// `eko-net` parses ReplayGain tags leniently from strings, so a hostile or broken
    /// server can put an absurd gain in front of this. It must not panic (it did, in debug,
    /// for `|x|` in roughly [1e38, 3.4e38]) and must not wrap silently (it did, in release).
    #[test]
    fn to_fixed_1_survives_absurd_magnitudes() {
        for x in [
            1e38,
            -1e38,
            2e38,
            f64::MAX,
            f64::MIN,
            1e21,
            -1e21,
            1e300,
            f64::MIN_POSITIVE,
        ] {
            let s = to_fixed_1(x);
            assert!(!s.is_empty(), "{x} produced nothing");
            // Whatever the fallback renders, it must never silently wrap to a small number.
            if x.abs() >= 1e38 {
                assert!(
                    s.len() > 30,
                    "{x} wrapped to a short value: {s} — checked_mul is not holding"
                );
            }
        }
        // Non-finite must not panic either. These are the documented divergences from
        // `toFixed` (JS renders "Infinity"); unreachable for a dB or a sample rate.
        for x in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            let _ = to_fixed_1(x);
        }
    }

    #[test]
    fn khz_uses_a_decimal_only_for_fractional_kilohertz() {
        assert_eq!(khz(0), "—");
        assert_eq!(khz(44_100), "44.1 kHz");
        assert_eq!(khz(48_000), "48 kHz");
        assert_eq!(khz(88_200), "88.2 kHz");
        assert_eq!(khz(96_000), "96 kHz");
        assert_eq!(khz(192_000), "192 kHz");
        assert_eq!(khz(352_800), "352.8 kHz");
    }

    #[test]
    fn the_source_string_omits_an_unknown_bit_depth_and_names_an_unknown_codec() {
        let mut i = clean_input();
        i.info.as_mut().unwrap().bits = 0;
        assert_eq!(derive(&i).src, "FLAC · 96 kHz");

        i.info.as_mut().unwrap().codec = String::new();
        let p = derive(&i);
        assert_eq!(p.codec, "AUDIO");
        assert_eq!(p.src, "AUDIO · 96 kHz");
    }

    #[test]
    fn the_output_string_falls_back_to_a_generic_device_name() {
        let mut i = clean_input();
        i.info.as_mut().unwrap().device = String::new();
        assert_eq!(derive(&i).output, "Output · 96 kHz");
    }

    #[test]
    fn a_live_engine_status_converts_into_the_seal_input() {
        let s = EngineStatus {
            playing: true,
            pos_ms: 1,
            dur_ms: 2,
            buffered_ms: 2,
            rate: 48_000,
            channels: 2,
            device: "DAC".into(),
            src_rate: 44_100,
            dev_rate: 48_000,
            bits: 16,
            codec: "mp3".into(),
            seg: 0,
        };
        let info = StreamInfo::from(&s);
        assert_eq!(info.rate, 48_000);
        assert_eq!(info.src_rate, 44_100);
        assert_eq!(info.dev_rate, 48_000);
        assert_eq!(info.bits, 16);
        assert_eq!(info.codec, "mp3");
        assert_eq!(info.device, "DAC");

        let p = derive(&SealInput {
            engine_active: true,
            info: Some(info),
            eq: EqState::default(),
            volume: 1.0,
            replaygain_db: SealRgDb::default(),
            replaygain_mode: RgMode::Off,
        });
        assert_eq!(p.seal_label, "RESAMPLED");
        assert_eq!(p.src, "MP3 · 44.1 kHz · 16-bit");
        assert_eq!(p.output, "DAC · 48 kHz");
    }

    /// The seal's JSON keys are the frontend's property names. Renaming one silently
    /// blanks part of the seal in the GUI, which no other test would catch.
    #[test]
    fn the_seal_serializes_with_the_keys_the_frontend_reads() {
        let v = serde_json::to_value(derive(&clean_input())).unwrap();
        for key in [
            "active",
            "pure",
            "eqActive",
            "attenuated",
            "rgActive",
            "resampled",
            "osResampled",
            "codec",
            "src",
            "output",
            "engineLabel",
            "sealLabel",
            "rgLabel",
        ] {
            assert!(v.get(key).is_some(), "missing seal key `{key}`");
        }
    }

    /// The input's JSON keys are what the frontend sends. Both EQ modes and all six
    /// filter types must round-trip from the TypeScript spelling.
    #[test]
    fn the_input_deserializes_from_the_frontends_wire_shape() {
        let json = serde_json::json!({
            "engineActive": true,
            "info": {
                "rate": 48000, "srcRate": 48000, "devRate": 48000,
                "bits": 24, "codec": "alac", "device": "DAC"
            },
            "eq": {
                "mode": "parametric",
                "enabled": false, "preamp": 0.0,
                "gains": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                "paramEnabled": true, "paramPreamp": 0.0,
                // `freq` and `q` are extra keys the store carries — ignored here.
                "paramBands": [
                    { "filterType": "highPass", "freq": 30.0, "gainDb": 0.0, "q": 0.707, "enabled": true },
                    { "filterType": "lowShelf", "freq": 100.0, "gainDb": 0.0, "q": 0.707, "enabled": true }
                ]
            },
            "volume": 1.0,
            "replaygainDb": null,
            "replaygainMode": "album"
        });
        let input: SealInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.eq.mode, SealEqMode::Parametric);
        assert_eq!(input.eq.param_bands[0].filter_type, ParamFilter::HighPass);
        let p = derive(&input);
        assert_eq!(p.seal_label, "EQ", "the high-pass alone breaks the seal");
        assert_eq!(p.rg_label, "Album");
    }
}
