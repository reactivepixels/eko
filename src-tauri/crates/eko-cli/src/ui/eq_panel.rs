//! The 10-band graphic EQ panel.
//!
//! Eleven columns — the pre-amp, then the ten bands low → high — drawn as the
//! sliders they are, with the 0 dB axis running through the middle. It is a
//! modal overlay **over the main pane only**: the footer is not in its area and
//! cannot be, because the seal is never off-screen and a panel that could cover
//! it would be the one piece of chrome in this application allowed to hide the
//! claim. See `docs/architecture/eko-cli.md` §5.1.
//!
//! ## What the drawing is allowed to know
//!
//! Nothing here decides anything. The gains come from
//! [`crate::eq::GraphicEq`], the preset name is derived from those gains rather
//! than remembered, the band labels come from `eko_core`'s own `band_label`, and
//! the on/off state is the same `enabled` the engine was handed. There is no
//! second copy of the preset table in this file — the panel never needs one,
//! because it only ever renders the numbers the model is holding.
//!
//! A **bypassed** EQ is drawn in the faint ink rather than the accent. That is
//! the panel telling the same truth the seal does: a curve that is not routed is
//! not shaping anything, and it should not look like it is.

use eko_core::eq_presets::{EQ_BAND_COUNT, EQ_GAIN_MAX, EQ_GAIN_MIN};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::eq::{self, COLUMNS};
use crate::ui::{justified, pane_body, GUTTER};

/// Width of one slider column, in cells. Three, because `170` is three.
const COL_W: usize = 3;
/// Gap between two slider columns.
const COL_GAP: usize = 1;
/// The dB scale down the left edge: `+12 `, `  0 `, `-12 `.
const SCALE_W: usize = 4;
/// Rows of slider. Odd, so one of them is the 0 dB axis.
const BAR_ROWS: usize = 7;
/// Frequency labels, the readout, and the key hints.
const TEXT_ROWS: usize = 3;

/// Content width the panel wants: the scale, then eleven columns.
const CONTENT_W: usize = SCALE_W + COLUMNS * COL_W + (COLUMNS - 1) * COL_GAP;

/// The panel's full size including its border and its two gutters.
#[must_use]
pub fn size() -> (u16, u16) {
    let chrome = 2 + usize::from(GUTTER) * 2;
    (
        (CONTENT_W + chrome) as u16,
        (BAR_ROWS + TEXT_ROWS + 2) as u16,
    )
}

/// Draw the panel centred in `area`, which is the main pane.
///
/// Draws nothing at all when it does not fit. A clipped EQ is a misleading one —
/// half a slider column reads as a gain that is not there — and the Deck already
/// has a well-tested answer for "this does not fit": don't draw it.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let (want_w, want_h) = size();
    if area.width < want_w || area.height < want_h {
        return;
    }
    let panel = Rect {
        x: area.x + (area.width - want_w) / 2,
        y: area.y + (area.height - want_h) / 2,
        width: want_w,
        height: want_h,
    };

    let theme = &app.theme;
    frame.render_widget(Clear, panel);
    let block = Block::bordered()
        .border_style(Style::default().fg(theme.rule))
        .title(Line::from(vec![
            Span::styled("─ ", Style::default().fg(theme.rule)),
            Span::styled("EQ", theme.accent_strong()),
            Span::raw(" "),
        ]));
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    // The Deck's own gutter, inside this border as well as inside the frame.
    let text = pane_body(inner);

    let mut lines: Vec<Line<'static>> = (0..BAR_ROWS).map(|row| bar_row(app, row)).collect();
    lines.push(frequency_row(app));
    lines.push(readout_row(app, text.width));
    lines.push(hint_row(app));
    frame.render_widget(Paragraph::new(lines), text);
}

/// The dB one slider row stands for. Row 0 is the ceiling, the last is the
/// floor, and the middle one is exactly 0 dB because [`BAR_ROWS`] is odd.
fn row_db(row: usize) -> f32 {
    let span = EQ_GAIN_MAX - EQ_GAIN_MIN;
    EQ_GAIN_MAX - (row as f32) * span / ((BAR_ROWS - 1) as f32)
}

/// The row a gain sits on.
fn db_row(db: f32) -> usize {
    let span = EQ_GAIN_MAX - EQ_GAIN_MIN;
    let row = ((EQ_GAIN_MAX - db) / span * ((BAR_ROWS - 1) as f32)).round();
    (row.max(0.0) as usize).min(BAR_ROWS - 1)
}

/// The 0 dB axis row.
const fn axis_row() -> usize {
    BAR_ROWS / 2
}

/// The columns, left to right: the pre-amp, then one per band.
///
/// Spelled as what it is rather than as `0..COLUMNS`, so the panel's geometry is
/// tied to [`EQ_BAND_COUNT`] and to [`crate::eq::band`] — an eleventh band in
/// `eko_core` would move this row without anyone having to remember to.
fn columns() -> impl Iterator<Item = usize> {
    std::iter::once(eq::PREAMP).chain((0..EQ_BAND_COUNT).map(eq::band))
}

/// One row of sliders, with its dB label in the left gutter.
fn bar_row(app: &App, row: usize) -> Line<'static> {
    let theme = &app.theme;
    let eq = app.eq();

    // Only the ceiling, the axis and the floor are labelled — a number on every
    // row would be four more things to read and no more information.
    let label = if row == 0 || row == axis_row() || row == BAR_ROWS - 1 {
        let db = row_db(row) as i32;
        // Signed, because a `12` above a `-12` reads as a different scale.
        let text = if db == 0 {
            "0".to_string()
        } else {
            format!("{db:+}")
        };
        format!("{text:>3} ")
    } else {
        " ".repeat(SCALE_W)
    };
    let mut spans = vec![Span::styled(label, Style::default().fg(theme.ink_faint))];

    let axis = axis_row();
    for col in columns() {
        if col > 0 {
            spans.push(Span::raw(" ".repeat(COL_GAP)));
        }
        let selected = col == app.eq_cursor;
        let gain = db_row(eq.value(col));
        let (lo, hi) = (gain.min(axis), gain.max(axis));
        let filled = (lo..=hi).contains(&row);

        let (glyph, colour) = if filled {
            // The bar. Faint when the EQ is bypassed, because a bypassed curve
            // is not doing anything and must not look like it is.
            let colour = match (eq.enabled(), selected) {
                (true, true) => theme.accent_bright,
                (true, false) => theme.accent,
                (false, true) => theme.ink_dim,
                (false, false) => theme.rule,
            };
            ("███", colour)
        } else if row == axis {
            ("───", theme.rule)
        } else {
            (" · ", theme.rule)
        };
        let mut style = Style::default().fg(colour);
        if selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(glyph, style));
    }
    Line::from(spans)
}

/// `pre  60 170 310 600  1k  3k  6k 12k 14k 16k`
fn frequency_row(app: &App) -> Line<'static> {
    let theme = &app.theme;
    let mut spans = vec![Span::raw(" ".repeat(SCALE_W))];
    for col in columns() {
        if col > 0 {
            spans.push(Span::raw(" ".repeat(COL_GAP)));
        }
        let selected = col == app.eq_cursor;
        let style = if selected {
            Style::default()
                .fg(theme.accent_bright)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.ink_faint)
        };
        spans.push(Span::styled(
            format!("{:>width$}", eq::column_label(col), width = COL_W),
            style,
        ));
    }
    Line::from(spans)
}

/// `Rock · EQ on                                  310  -1.0 dB`
///
/// The preset name is [`crate::eq::GraphicEq::preset_name`], which is derived
/// from the gains — so it says `custom` the moment the curve stops being one,
/// and can never claim a preset the sliders are not sitting on.
fn readout_row(app: &App, width: u16) -> Line<'static> {
    let theme = &app.theme;
    let eq = app.eq();
    let state = if eq.enabled() { "EQ on" } else { "EQ off" };
    let state_colour = if eq.enabled() {
        theme.led_amber
    } else {
        theme.ink_faint
    };
    let left = vec![
        Span::styled(
            eq.preset_name(),
            Style::default().fg(theme.ink).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · ", Style::default().fg(theme.rule)),
        Span::styled(state, Style::default().fg(state_colour)),
    ];
    let col = app.eq_cursor.min(COLUMNS - 1);
    let right = vec![
        Span::styled(eq::column_label(col), Style::default().fg(theme.ink_faint)),
        Span::styled(
            format!("  {:+.1} dB", eq.value(col)),
            Style::default().fg(theme.ink),
        ),
    ];
    justified(left, right, width)
}

/// The keys, spelled the way [`crate::keys::BINDINGS`] spells them.
///
/// Read out of the table rather than written down, so a rebinding cannot leave
/// this row describing a keyboard the application no longer has. Only the
/// *first* spelling of each key is shown — the table lists alternatives
/// (`j / ↓`) and one row of a panel has room for one of them, so it takes the
/// label up to its first space rather than inventing a second name for the key.
fn hint_row(app: &App) -> Line<'static> {
    use crate::keys::Action;

    let theme = &app.theme;
    let key = |action: Action| -> String {
        crate::keys::BINDINGS
            .iter()
            .find(|b| b.action == action)
            .and_then(|b| b.label.split(' ').next())
            .unwrap_or_default()
            .to_string()
    };
    let hint = format!(
        "{}/{} band  {}/{} gain  {}/{} preset  {} on/off",
        key(Action::EqPrev),
        key(Action::EqNext),
        key(Action::Up),
        key(Action::Down),
        key(Action::EqPresetPrev),
        key(Action::EqPresetNext),
        key(Action::EqToggle),
    );
    Line::from(vec![
        Span::raw(" ".repeat(SCALE_W)),
        Span::styled(hint, Style::default().fg(theme.ink_faint)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_slider_row_is_exactly_zero_db() {
        assert_eq!(BAR_ROWS % 2, 1, "an even slider needs no axis row");
        assert_eq!(row_db(axis_row()), 0.0);
        assert_eq!(row_db(0), EQ_GAIN_MAX);
        assert_eq!(row_db(BAR_ROWS - 1), EQ_GAIN_MIN);
    }

    /// Every gain the model can hold lands on a row, and the extremes land on
    /// the extremes — a bar that ran off the top would be drawn as a lie.
    #[test]
    fn every_gain_maps_onto_a_row_and_the_ends_are_the_ends() {
        assert_eq!(db_row(EQ_GAIN_MAX), 0);
        assert_eq!(db_row(EQ_GAIN_MIN), BAR_ROWS - 1);
        assert_eq!(db_row(0.0), axis_row());
        let mut db = EQ_GAIN_MIN;
        while db <= EQ_GAIN_MAX {
            assert!(db_row(db) < BAR_ROWS, "{db} dB fell off the slider");
            db += 0.25;
        }
        // Out of range cannot happen through `GraphicEq`, but the clamp is what
        // makes that a fact about this function rather than about its caller.
        assert_eq!(db_row(400.0), 0);
        assert_eq!(db_row(-400.0), BAR_ROWS - 1);
    }

    #[test]
    fn the_panel_is_wide_enough_for_the_preamp_and_ten_bands() {
        let (w, h) = size();
        assert_eq!(usize::from(w), CONTENT_W + 2 + usize::from(GUTTER) * 2);
        assert_eq!(usize::from(h), BAR_ROWS + TEXT_ROWS + 2);
        // It has to fit the main pane at the Deck's minimum size: 80 columns
        // less the border, the sidebar and its rule; 20 rows less the border,
        // the footer and its rule.
        let main_w = crate::ui::MIN_WIDTH - 2 - crate::ui::SIDEBAR_WIDTH - 1;
        let main_h = crate::ui::MIN_HEIGHT - 2 - crate::ui::FOOTER_HEIGHT - 1;
        assert!(w <= main_w, "the panel does not fit an 80-column Deck");
        assert!(h <= main_h, "the panel does not fit a 20-row Deck");
    }
}
