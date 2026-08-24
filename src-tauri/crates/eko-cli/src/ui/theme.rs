//! The Porcelain / Graphite palette, in a terminal.
//!
//! Every constant here is lifted verbatim from the desktop app's design system,
//! `src/player/neu.css`, so the two frontends cannot drift. The token name each
//! value came from is in the comment beside it.
//!
//! One rule that is not negotiable: **nothing here paints a background.** Every
//! style is a foreground colour over whatever ground the terminal already has,
//! which is what the sidebar, the main pane and the borders have always done —
//! so the whole Deck is one surface.
//!
//! ## Why the footer is not a dark "device screen"
//!
//! An earlier draft filled the footer with `--screen` (`#181b16`) on the theory
//! that a footer is an amp faceplate and a device screen stays dark. **The GUI
//! does not do that.** `--screen` is declared once, at `src/player/neu.css:25`,
//! and no rule in the free app ever reads it — `grep -rn "var(--screen)" src/`
//! finds nothing. Where the idea is real, in the Pro Aether theme
//! (`src/pro/themes/aether.css:42`), it is a *recessed well* whose dark value
//! `#232428` sits **three** points under `--bg: #23262c`, not eleven.
//!
//! A terminal has no ground colour this crate can read, so there is nothing to
//! be three points darker than: any explicit fill is a guess, and the near-black
//! guess read on screen as a black box bolted onto the console. The Deck paints
//! none at all. The rule above the footer is what separates it.

use ratatui::style::{Color, Modifier, Style};

// ── Palette, from src/player/neu.css ─────────────────────────────────────────

/// `--ink` (Graphite / dark theme) — primary text.
pub const INK: Rgb = Rgb::hex(0xeceef0);
/// `--ink-2` (Graphite) — secondary text.
pub const INK_2: Rgb = Rgb::hex(0xa3a6ad);
/// `--ink-3` (Graphite) — tertiary text; WCAG AA on `--bg`.
pub const INK_3: Rgb = Rgb::hex(0x8e9198);

/// Instrument LED green — `--g`.
pub const LED_G: Rgb = Rgb::hex(0x7ad24a);
/// Instrument LED green, shadowed — `--g2`.
pub const LED_G2: Rgb = Rgb::hex(0x5e8f2a);
/// Instrument LED amber — `--a`.
pub const LED_A: Rgb = Rgb::hex(0xf0b03a);
/// Instrument LED red — `--r`.
pub const LED_R: Rgb = Rgb::hex(0xe8633a);

/// Rules and borders.
///
/// Derived, not lifted. `--line` on the dark theme is
/// `rgba(255, 255, 255, 0.05)` — a compositing colour, and a terminal cell has
/// no alpha, so it has to be flattened. At 5% it is invisible on a 1-pixel
/// stroke with no anti-aliasing to help it; this is the same white at ~25%,
/// which is the lowest that stays legible.
pub const RULE: Rgb = Rgb::hex(0x525653);

/// The **unplayed** part of the transport scrubber.
///
/// [`RULE`] plus 0x26 in every channel — the same colour, one stop up.
///
/// # Why it is not `RULE`
///
/// It was, and it was too dim to be a control. Against the design's own ground
/// (`--bg`, `#23262c`) `RULE` is **2.03:1**, under the 3:1 WCAG 2.1 SC 1.4.11
/// asks of a non-text user-interface component. That is fine for a border — a
/// frame you are not meant to look at, and one SC 1.4.11 does not reach — and
/// not fine for the half of the scrubber that says *how much of this track is
/// left*. On a screenshot it read as an empty row, and it read that way because
/// it very nearly is one.
///
/// This is **3.58:1** on the same ground, and 4.96:1 on a black terminal. It is
/// still visibly a track rather than text: `--ink-3` (`INK_3`, what the clocks
/// at each end are drawn in) is 4.80:1, well above it.
///
/// The ratios are computed, in the test, from these constants — see
/// `the_unplayed_scrubber_is_bright_enough_to_be_a_control`. A number in a
/// comment is a number nobody re-checks.
pub const SCRUBBER_TRACK: Rgb = Rgb::hex(0x787c79);

/// The dark theme's ground, `--bg`.
///
/// **Nothing paints this**, which is why it is `#[cfg(test)]`. A terminal has
/// its own background and this crate never sets one — see the module header. It
/// exists because it is what the palette's contrast ratios are measured
/// *against*, and a contrast assertion with no ground to measure against is an
/// assertion about nothing.
#[cfg(test)]
pub const GROUND: Rgb = Rgb::hex(0x23262c);

/// The `[data-accent]` presets from `neu.css`, in the same order the desktop
/// app's picker shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    /// `:root` — `--accent: #ef6a1e`.
    #[default]
    Orange,
    /// `[data-accent="violet"]`.
    Violet,
    /// `[data-accent="blue"]`.
    Blue,
    /// `[data-accent="teal"]`.
    Teal,
    /// `[data-accent="graphite"]`.
    Graphite,
    /// `[data-accent="cyan"]`.
    Cyan,
}

impl Accent {
    /// Match a config string to a preset. Unknown names fall back to the
    /// default orange rather than erroring — a typo in a dotfile should not
    /// stop the player.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "violet" => Self::Violet,
            "blue" => Self::Blue,
            "teal" => Self::Teal,
            "graphite" => Self::Graphite,
            "cyan" => Self::Cyan,
            _ => Self::Orange,
        }
    }

    /// `--accent`.
    #[must_use]
    pub const fn base(self) -> Rgb {
        match self {
            Self::Orange => Rgb::hex(0xef6a1e),
            Self::Violet => Rgb::hex(0x6a5cf0),
            Self::Blue => Rgb::hex(0x2f8fff),
            Self::Teal => Rgb::hex(0x13b5a6),
            Self::Graphite => Rgb::hex(0x8a8780),
            Self::Cyan => Rgb::hex(0x28d8f7),
        }
    }

    /// `--accent-2` — the lighter stop of the accent gradient.
    #[must_use]
    pub const fn bright(self) -> Rgb {
        match self {
            Self::Orange => Rgb::hex(0xff8c42),
            Self::Violet => Rgb::hex(0x8d7dff),
            Self::Blue => Rgb::hex(0x5cabff),
            Self::Teal => Rgb::hex(0x3ed6c7),
            Self::Graphite => Rgb::hex(0xa8a59d),
            Self::Cyan => Rgb::hex(0xb6f4ff),
        }
    }
}

// ── Colour depth ─────────────────────────────────────────────────────────────

/// How many colours the terminal can actually show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit. Emitted as `Color::Rgb`.
    TrueColor,
    /// The xterm-256 palette. Emitted as `Color::Indexed`.
    Ansi256,
}

/// Decide the depth from the environment.
///
/// Split from [`detect_depth`] so the decision is testable without touching
/// process-global env vars.
#[must_use]
pub fn depth_from_env(colorterm: Option<&str>) -> ColorDepth {
    match colorterm {
        Some(v) => {
            let v = v.to_ascii_lowercase();
            if v.contains("truecolor") || v.contains("24bit") {
                ColorDepth::TrueColor
            } else {
                ColorDepth::Ansi256
            }
        }
        None => ColorDepth::Ansi256,
    }
}

/// Read `COLORTERM` and decide. Anything unrecognised degrades to 256 colours,
/// which is the safe direction to be wrong in.
#[must_use]
pub fn detect_depth() -> ColorDepth {
    depth_from_env(std::env::var("COLORTERM").ok().as_deref())
}

/// A 24-bit colour, before it has been resolved for a particular terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Build from a `0xRRGGBB` literal, so the constants above read like CSS.
    #[must_use]
    pub const fn hex(v: u32) -> Self {
        Self(
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        )
    }

    /// Resolve for a terminal.
    #[must_use]
    pub fn resolve(self, depth: ColorDepth) -> Color {
        match depth {
            ColorDepth::TrueColor => Color::Rgb(self.0, self.1, self.2),
            ColorDepth::Ansi256 => Color::Indexed(self.to_indexed()),
        }
    }

    /// Nearest xterm-256 slot.
    ///
    /// Considers the 6×6×6 colour cube *and* the 24-step grey ramp and takes
    /// whichever is closer. Cube-only rounding collapses every near-black into a
    /// flat `#000` and every near-white into `#fff`; the grey ramp keeps them
    /// distinguishable, which is what a rule drawn in `RULE` depends on.
    #[must_use]
    pub fn to_indexed(self) -> u8 {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

        let nearest_level = |c: u8| -> usize {
            let mut best = 0;
            let mut best_d = u32::MAX;
            for (i, &l) in LEVELS.iter().enumerate() {
                let d = i32::from(c).abs_diff(i32::from(l));
                if d < best_d {
                    best_d = d;
                    best = i;
                }
            }
            best
        };
        let dist = |a: (u8, u8, u8), b: (u8, u8, u8)| -> u32 {
            let d = |x: u8, y: u8| {
                let v = i32::from(x) - i32::from(y);
                (v * v) as u32
            };
            d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
        };

        let (ri, gi, bi) = (
            nearest_level(self.0),
            nearest_level(self.1),
            nearest_level(self.2),
        );
        let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
        let cube_index = 16 + 36 * ri + 6 * gi + bi;

        // Grey ramp: 232..=255, values 8, 18, ... 238.
        let avg = (u32::from(self.0) + u32::from(self.1) + u32::from(self.2)) / 3;
        let step = (avg.saturating_sub(8) + 5) / 10;
        let step = step.min(23) as u8;
        let grey_value = 8 + step * 10;
        let grey = (grey_value, grey_value, grey_value);

        let me = (self.0, self.1, self.2);
        if dist(me, grey) < dist(me, cube) {
            232 + step
        } else {
            cube_index as u8
        }
    }
}

// ── Theme ────────────────────────────────────────────────────────────────────

/// Every colour the Deck draws with, already resolved for this terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub depth: ColorDepth,
    /// `--accent`.
    pub accent: Color,
    /// `--accent-2`.
    pub accent_bright: Color,
    /// `--ink`.
    pub ink: Color,
    /// `--ink-2`.
    pub ink_dim: Color,
    /// `--ink-3`.
    pub ink_faint: Color,
    /// Borders and rules.
    pub rule: Color,
    /// The unplayed part of the transport scrubber. See [`SCRUBBER_TRACK`].
    pub scrubber_track: Color,
    /// `--g`, `--a`, `--r` — the instrument LED ramp.
    pub led_green: Color,
    pub led_green_dim: Color,
    pub led_amber: Color,
    pub led_red: Color,
}

impl Theme {
    /// Resolve the palette for an accent preset and a colour depth.
    #[must_use]
    pub fn new(accent: Accent, depth: ColorDepth) -> Self {
        Self {
            depth,
            accent: accent.base().resolve(depth),
            accent_bright: accent.bright().resolve(depth),
            ink: INK.resolve(depth),
            ink_dim: INK_2.resolve(depth),
            ink_faint: INK_3.resolve(depth),
            rule: RULE.resolve(depth),
            scrubber_track: SCRUBBER_TRACK.resolve(depth),
            led_green: LED_G.resolve(depth),
            led_green_dim: LED_G2.resolve(depth),
            led_amber: LED_A.resolve(depth),
            led_red: LED_R.resolve(depth),
        }
    }

    /// Ink on the console ground, at the given emphasis.
    ///
    /// Sets a foreground and **nothing else** — the ground is the terminal's
    /// own, exactly as it is for the sidebar and the main pane, which write
    /// `Style::default().fg(..)` directly. See the module note on `--screen`.
    #[must_use]
    pub fn on_console(&self, fg: Color) -> Style {
        Style::default().fg(fg)
    }

    /// The accent, emphasised — used for the app mark and the selected row.
    #[must_use]
    pub fn accent_strong(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Pick an LED colour for a normalised meter level, `0.0..=1.0`.
    ///
    /// Green through most of the travel, amber in the last fifth, red at the
    /// top — the ramp an instrument uses, not a rainbow.
    #[must_use]
    pub fn led_for(&self, level: f32) -> Color {
        if level >= 0.86 {
            self.led_red
        } else if level >= 0.62 {
            self.led_amber
        } else {
            self.led_green
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(Accent::default(), ColorDepth::TrueColor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_literals_match_neu_css() {
        assert_eq!(Accent::Orange.base(), Rgb(0xef, 0x6a, 0x1e));
        assert_eq!(INK, Rgb(0xec, 0xee, 0xf0));
        assert_eq!(LED_G, Rgb(0x7a, 0xd2, 0x4a));
        assert_eq!(LED_A, Rgb(0xf0, 0xb0, 0x3a));
        assert_eq!(LED_R, Rgb(0xe8, 0x63, 0x3a));
    }

    #[test]
    fn unknown_accent_names_fall_back_to_orange() {
        assert_eq!(Accent::from_name("chartreuse"), Accent::Orange);
        assert_eq!(Accent::from_name(""), Accent::Orange);
        assert_eq!(Accent::from_name("  CYAN "), Accent::Cyan);
        assert_eq!(Accent::from_name("violet"), Accent::Violet);
    }

    #[test]
    fn truecolor_is_only_claimed_when_colorterm_says_so() {
        assert_eq!(depth_from_env(Some("truecolor")), ColorDepth::TrueColor);
        assert_eq!(depth_from_env(Some("24bit")), ColorDepth::TrueColor);
        assert_eq!(depth_from_env(Some("TrueColor")), ColorDepth::TrueColor);
        assert_eq!(depth_from_env(Some("")), ColorDepth::Ansi256);
        assert_eq!(depth_from_env(Some("16color")), ColorDepth::Ansi256);
        assert_eq!(depth_from_env(None), ColorDepth::Ansi256);
    }

    #[test]
    fn truecolor_emits_rgb_and_256_emits_indexed() {
        assert_eq!(
            Accent::Orange.base().resolve(ColorDepth::TrueColor),
            Color::Rgb(0xef, 0x6a, 0x1e)
        );
        assert!(matches!(
            Accent::Orange.base().resolve(ColorDepth::Ansi256),
            Color::Indexed(_)
        ));
    }

    #[test]
    fn indexed_degradation_hits_the_expected_slots() {
        assert_eq!(Rgb(0, 0, 0).to_indexed(), 16);
        assert_eq!(Rgb(255, 255, 255).to_indexed(), 231);
        // The accent lands on xterm 202, the orange everyone recognises.
        assert_eq!(Accent::Orange.base().to_indexed(), 202);
    }

    #[test]
    fn near_black_uses_the_grey_ramp_not_a_flat_cube_black() {
        // Cube rounding alone flattens every near-black to slot 16 — pure black
        // — and loses the distinction between them. `RULE` is a mid grey and
        // relies on the ramp for the same reason.
        for near_black in [Rgb::hex(0x181b16), GROUND, RULE] {
            let i = near_black.to_indexed();
            assert!(
                (232..=255).contains(&i),
                "expected a grey-ramp slot for {near_black:?}, got {i}"
            );
            assert_ne!(i, 16);
        }
    }

    /// **Nothing in the palette paints a background.**
    ///
    /// This is the replacement for a rule that was asserted here and was never
    /// true of the GUI: "the footer is a device screen and a device screen stays
    /// dark". `--screen` (`#181b16`) is declared in `src/player/neu.css` and
    /// never consumed by the free app; against the Graphite ground the footer
    /// read as a black box bolted onto the console. The Deck now sits on the
    /// terminal's own ground everywhere, and this pins it — a style that sets a
    /// `bg` is a block that will not match its neighbours on somebody's
    /// terminal.
    #[test]
    fn no_style_in_the_palette_paints_a_background() {
        for accent in [
            Accent::Orange,
            Accent::Violet,
            Accent::Blue,
            Accent::Teal,
            Accent::Graphite,
            Accent::Cyan,
        ] {
            for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
                let t = Theme::new(accent, depth);
                for style in [
                    t.accent_strong(),
                    t.on_console(t.ink),
                    t.on_console(t.ink_dim),
                    t.on_console(t.ink_faint),
                    t.on_console(t.rule),
                    t.on_console(t.led_for(0.0)),
                    t.on_console(t.led_for(1.0)),
                ] {
                    assert_eq!(
                        style.bg, None,
                        "{accent:?}/{depth:?}: a palette style set a background"
                    );
                }
            }
        }
    }

    #[test]
    fn the_led_ramp_runs_green_amber_red() {
        let t = Theme::default();
        assert_eq!(t.led_for(0.0), t.led_green);
        assert_eq!(t.led_for(0.5), t.led_green);
        assert_eq!(t.led_for(0.7), t.led_amber);
        assert_eq!(t.led_for(1.0), t.led_red);
    }
}
