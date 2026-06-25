//! Shared biquad filter — the single source of truth for all EQ DSP math.
//!
//! Direct-form II transposed second-order IIR section. Coefficients follow the RBJ
//! Audio EQ Cookbook notation (`b0/b1/b2` = feed-forward, `a1/a2` = feedback; `a0`
//! is normalised out). Both the free 10-band graphic EQ (`engine.rs`) and the Pro
//! parametric EQ (`pro/param_eq.rs`) build their coefficients here, so the two paths
//! can never drift. This module is FREE — it has no `pro` dependency and ships to the
//! public `eko` repo.

use std::f32::consts::PI;

/// Direct-form II transposed biquad filter.
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

// `peaking` + `process` serve the free graphic EQ; the shelf/pass/notch constructors
// and `magnitude_db` are used only by the Pro parametric EQ. Allow dead_code so the
// free build stays clippy-clean under `-D warnings`.
#[allow(dead_code)]
impl Biquad {
    /// Compute RBJ peaking EQ coefficients.
    pub fn peaking(freq: f32, q: f32, gain_db: f32, fs: f32) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Biquad {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cw) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }

    /// Compute RBJ low-shelf coefficients.
    pub fn low_shelf(freq: f32, q: f32, gain_db: f32, fs: f32) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / 2.0 * ((a + 1.0 / a) * (1.0 / q - 1.0) + 2.0).sqrt();
        let a0 = (a + 1.0) + (a - 1.0) * cw + 2.0 * alpha * a.sqrt();
        Biquad {
            b0: a * ((a + 1.0) - (a - 1.0) * cw + 2.0 * alpha * a.sqrt()) / a0,
            b1: 2.0 * a * ((a - 1.0) - (a + 1.0) * cw) / a0,
            b2: a * ((a + 1.0) - (a - 1.0) * cw - 2.0 * alpha * a.sqrt()) / a0,
            a1: -2.0 * ((a - 1.0) + (a + 1.0) * cw) / a0,
            a2: ((a + 1.0) + (a - 1.0) * cw - 2.0 * alpha * a.sqrt()) / a0,
        }
    }

    /// Compute RBJ high-shelf coefficients.
    pub fn high_shelf(freq: f32, q: f32, gain_db: f32, fs: f32) -> Biquad {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / 2.0 * ((a + 1.0 / a) * (1.0 / q - 1.0) + 2.0).sqrt();
        let a0 = (a + 1.0) - (a - 1.0) * cw + 2.0 * alpha * a.sqrt();
        Biquad {
            b0: a * ((a + 1.0) + (a - 1.0) * cw + 2.0 * alpha * a.sqrt()) / a0,
            b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * cw) / a0,
            b2: a * ((a + 1.0) + (a - 1.0) * cw - 2.0 * alpha * a.sqrt()) / a0,
            a1: 2.0 * ((a - 1.0) - (a + 1.0) * cw) / a0,
            a2: ((a + 1.0) - (a - 1.0) * cw - 2.0 * alpha * a.sqrt()) / a0,
        }
    }

    /// Compute RBJ 2nd-order Butterworth low-pass coefficients.
    pub fn low_pass(freq: f32, q: f32, fs: f32) -> Biquad {
        let w0 = 2.0 * PI * freq / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: (1.0 - cw) / 2.0 / a0,
            b1: (1.0 - cw) / a0,
            b2: (1.0 - cw) / 2.0 / a0,
            a1: -2.0 * cw / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    /// Compute RBJ 2nd-order Butterworth high-pass coefficients.
    pub fn high_pass(freq: f32, q: f32, fs: f32) -> Biquad {
        let w0 = 2.0 * PI * freq / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: (1.0 + cw) / 2.0 / a0,
            b1: -(1.0 + cw) / a0,
            b2: (1.0 + cw) / 2.0 / a0,
            a1: -2.0 * cw / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    /// Compute RBJ notch (band-reject) coefficients.
    pub fn notch(freq: f32, q: f32, fs: f32) -> Biquad {
        let w0 = 2.0 * PI * freq / fs;
        let (sw, cw) = (w0.sin(), w0.cos());
        let alpha = sw / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: 1.0 / a0,
            b1: -2.0 * cw / a0,
            b2: 1.0 / a0,
            a1: -2.0 * cw / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    /// Process one sample through the filter using direct-form II transposed state.
    #[inline]
    pub fn process(&self, z: &mut (f32, f32), x: f32) -> f32 {
        let y = self.b0 * x + z.0;
        z.0 = self.b1 * x - self.a1 * y + z.1;
        z.1 = self.b2 * x - self.a2 * y;
        y
    }

    /// Magnitude response in dB at `freq` (Hz), evaluating `|H(e^jω)|` from the
    /// normalised coefficients (`a0 = 1`). Used to draw the parametric EQ response
    /// curve so the preview shares the exact coefficients that produce the audio.
    pub fn magnitude_db(&self, freq: f32, fs: f32) -> f32 {
        let w = 2.0 * PI * freq / fs;
        let (cw, sw) = (w.cos(), w.sin());
        let (c2w, s2w) = ((2.0 * w).cos(), (2.0 * w).sin());
        // Numerator B(e^jω) = b0 + b1·z⁻¹ + b2·z⁻²
        let b_re = self.b0 + self.b1 * cw + self.b2 * c2w;
        let b_im = -self.b1 * sw - self.b2 * s2w;
        let b_mag2 = b_re * b_re + b_im * b_im;
        // Denominator A(e^jω) = 1 + a1·z⁻¹ + a2·z⁻²
        let a_re = 1.0 + self.a1 * cw + self.a2 * c2w;
        let a_im = -self.a1 * sw - self.a2 * s2w;
        let a_mag2 = a_re * a_re + a_im * a_im;
        if a_mag2 < 1e-30 {
            return 0.0;
        }
        let ratio = b_mag2 / a_mag2;
        if ratio <= 0.0 {
            return -120.0;
        }
        // 20·log10(|H|) == 10·log10(|H|²) == 10·log10(ratio)
        10.0 * ratio.log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_shelf_0db_is_unity() {
        let bq = Biquad::low_shelf(200.0, 0.707, 0.0, 48_000.0);
        let mut z = (0.0f32, 0.0f32);
        let mut y = 0.0f32;
        for _ in 0..2000 {
            y = bq.process(&mut z, 1.0);
        }
        assert!((y - 1.0).abs() < 1e-3, "low_shelf 0dB steady state y={y}");
    }

    #[test]
    fn low_shelf_boost_raises_dc() {
        let bq = Biquad::low_shelf(200.0, 0.707, 12.0, 48_000.0);
        let mut z = (0.0f32, 0.0f32);
        let mut y = 0.0f32;
        for _ in 0..4000 {
            y = bq.process(&mut z, 1.0);
        }
        assert!(y > 2.5, "low_shelf +12dB DC should be >2.5, got {y}");
    }

    #[test]
    fn high_shelf_0db_is_unity() {
        let bq = Biquad::high_shelf(8000.0, 0.707, 0.0, 48_000.0);
        let mut z = (0.0f32, 0.0f32);
        let mut y = 0.0f32;
        for _ in 0..2000 {
            y = bq.process(&mut z, 1.0);
        }
        assert!((y - 1.0).abs() < 1e-3, "high_shelf 0dB steady state y={y}");
    }

    #[test]
    fn low_pass_passes_dc() {
        let bq = Biquad::low_pass(1000.0, 0.707, 48_000.0);
        let mut z = (0.0f32, 0.0f32);
        let mut y = 0.0f32;
        for _ in 0..2000 {
            y = bq.process(&mut z, 1.0);
        }
        assert!((y - 1.0).abs() < 0.1, "low_pass DC passthrough y={y}");
    }

    #[test]
    fn high_pass_attenuates_dc() {
        let bq = Biquad::high_pass(1000.0, 0.707, 48_000.0);
        let mut z = (0.0f32, 0.0f32);
        let mut y = 0.0f32;
        for _ in 0..2000 {
            y = bq.process(&mut z, 1.0);
        }
        assert!(y.abs() < 0.01, "high_pass DC should be ≈0, got {y}");
    }

    #[test]
    fn notch_passes_dc() {
        let bq = Biquad::notch(1000.0, 10.0, 48_000.0);
        let mut z = (0.0f32, 0.0f32);
        let mut y = 0.0f32;
        for _ in 0..2000 {
            y = bq.process(&mut z, 1.0);
        }
        assert!((y - 1.0).abs() < 0.05, "notch passes DC y={y}");
    }

    /// Measure a filter's steady-state gain (dB) at `freq` by driving a unit-amplitude
    /// sine and extracting the output amplitude with a single-bin DFT (robust to the
    /// sampling phase, unlike peak-picking).
    fn measured_gain_db(bq: &Biquad, freq: f32, fs: f32) -> f32 {
        let w = 2.0 * PI * freq / fs;
        let mut z = (0.0f32, 0.0f32);
        // Settle the filter past its transient.
        for i in 0..8_000 {
            bq.process(&mut z, (w * i as f32).sin());
        }
        // Quadrature accumulation over N samples → amplitude at exactly `freq`.
        let n = 16_000usize;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for k in 0..n {
            let phase = w * (8_000 + k) as f32;
            let y = bq.process(&mut z, phase.sin()) as f64;
            re += y * phase.cos() as f64;
            im += y * phase.sin() as f64;
        }
        let amp = 2.0 / n as f64 * (re * re + im * im).sqrt();
        20.0 * (amp as f32).log10()
    }

    #[test]
    fn magnitude_db_matches_measured_response() {
        // Drift guard: the curve preview (`magnitude_db`) MUST reflect the actual audio
        // (`process`). If the two ever disagree, the on-screen EQ curve would lie about
        // what's audible — exactly the bug that unifying the DSP into one module kills.
        let fs = 48_000.0;
        let cases = [
            (Biquad::peaking(1000.0, 1.0, 6.0, fs), 1000.0f32),
            (Biquad::peaking(3000.0, 2.0, -4.0, fs), 3000.0),
            (Biquad::low_shelf(120.0, 0.707, 5.0, fs), 120.0),
            (Biquad::high_shelf(8000.0, 0.707, -3.0, fs), 8000.0),
            (Biquad::low_pass(2000.0, 0.707, fs), 500.0),
        ];
        for (bq, f) in cases {
            let predicted = bq.magnitude_db(f, fs);
            let measured = measured_gain_db(&bq, f, fs);
            assert!(
                (predicted - measured).abs() < 0.3,
                "magnitude_db {predicted:.3} dB vs measured {measured:.3} dB at {f} Hz"
            );
        }
    }
}
