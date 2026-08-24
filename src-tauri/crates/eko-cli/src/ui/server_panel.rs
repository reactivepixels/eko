//! The add-server panel — three lines of text and the way out.
//!
//! A modal overlay **over the main pane only**, for the same reason
//! [`crate::ui::device_panel`] is: the footer is never covered, because the seal
//! is never off-screen and nothing in this application is allowed to be the
//! exception.
//!
//! ## There is no password field here
//!
//! This panel collects the three things that go in `config.toml` — a name, a
//! URL and a username — and stops. The password is asked for afterwards, on the
//! restored terminal, by `main`. That split is not squeamishness: everything in
//! this file is drawn from [`crate::app::AddServer`], `AddServer` lives on
//! `App`, and `App` is what every frame — and every snapshot, and every future
//! crash report — is made of. A fourth field here would put somebody's password
//! in the ratatui buffer the moment they typed it.
//!
//! See [`crate::app::AddServer`] for the state, and `main`'s `serve_prompt` for
//! where the password actually goes.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, Field};
use crate::line::columns;
use crate::ui::footer::fit;
use crate::ui::{pane_body, GUTTER};

/// Widest value the panel lays out, in **columns**. A longer one scrolls its own
/// tail into view rather than widening the panel past the pane — see [`window`].
///
/// Columns rather than characters, because that is the unit everything else in
/// this file is in: [`Line::width`] below, [`fit`], and the terminal cell. This
/// number and [`crate::app::Field::max`] used to be characters while the panel's
/// width was columns, and the gap between the two is what made the panel vanish
/// on a name typed in a wide script — see [`crate::line::columns`].
pub(crate) const VALUE_W: usize = 38;

/// `name  ` — the label column, wide enough for the longest of the three.
const LABEL_W: usize = 6;

/// Draw the panel centred in `area`, which is the main pane.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let Some(add) = &app.add else {
        return;
    };
    let theme = &app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();

    for field in Field::ALL {
        let focused = add.field == field;
        let value = window(add.value(field), VALUE_W);
        // A block after the text, because the terminal cursor is hidden for the
        // life of the Deck (`tui::init` sends `Hide`) and a text field with no
        // caret does not look like one.
        let caret = if focused { "▏" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<LABEL_W$}", field.label()),
                Style::default().fg(if focused { theme.ink } else { theme.ink_faint }),
            ),
            Span::styled(
                format!("{value}{caret}"),
                if focused {
                    theme.accent_strong()
                } else {
                    Style::default().fg(theme.ink_dim)
                },
            ),
        ]));
        // The complaint goes under the field it is about, in front of the person
        // fixing it, rather than in a footer note after a failed save.
        let note = add.error_for(field).map_or_else(
            || (focused.then(|| field.hint().to_string()), theme.ink_faint),
            |message| (Some(message.to_string()), theme.led_red),
        );
        if let (Some(text), colour) = note {
            lines.push(Line::from(Span::styled(
                format!(" {:<LABEL_W$}{}", "", fit(&text, VALUE_W)),
                Style::default().fg(colour),
            )));
        }
    }

    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        " a password is asked for next, on a plain terminal".to_string(),
        Style::default().fg(theme.ink_faint),
    )));
    // **Said before it happens, not discovered afterwards.** Multi-server is
    // deferred: `Config::server` is still the first entry, so a second one is
    // written to the file and not switched to. A panel that let someone type a
    // whole server in and then quietly did nothing visible would be worse than
    // one that never opened.
    if add.second {
        lines.push(Line::from(Span::styled(
            " saved to the config · EKO still uses the first".to_string(),
            Style::default().fg(theme.led_amber),
        )));
    }
    lines.push(Line::default());
    lines.push(hint_row(app));

    // ── the panel can never be bigger than what it was handed ─────────────
    //
    // **Clamped, not refused.** This used to compute the width it wanted and
    // `return` when the area was smaller — which is how a panel with a wide
    // name in it drew *nothing at all* while `App::add` stayed `Some` and went
    // on eating every keystroke. A modal that is open and invisible is the worst
    // state this application can be in: the Deck looks frozen, and only Esc,
    // backspace or Ctrl-C get out.
    //
    // With `window` and the field caps now both in columns, `want_w` is bounded
    // by `VALUE_W` and cannot exceed the pane at any supported size — see
    // `the_widest_line_still_fits_an_eighty_column_deck`. The clamp is the
    // structural half of the same guarantee: whatever any future line does, the
    // rectangle is `min`'d into the area rather than compared against it, so
    // "too big to fit" degrades to a clipped panel instead of an absent one.
    // `Paragraph` drops the rows and columns that fall outside on its own.
    let want_w = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        // The border, then the Deck's own gutter on each side.
        .saturating_add(2 + usize::from(GUTTER) * 2);
    let want_w = u16::try_from(want_w).unwrap_or(u16::MAX).min(area.width);
    let want_h = u16::try_from(lines.len() + 2)
        .unwrap_or(u16::MAX)
        .min(area.height);
    // Nowhere to draw at all. Not the vanishing case above: there is no cell.
    if want_w == 0 || want_h == 0 {
        return;
    }
    let panel = Rect {
        x: area.x + (area.width - want_w) / 2,
        y: area.y + (area.height - want_h) / 2,
        width: want_w,
        height: want_h,
    };

    frame.render_widget(Clear, panel);
    let block = Block::bordered()
        .border_style(Style::default().fg(theme.rule))
        .title(Line::from(vec![
            Span::styled("─ ", Style::default().fg(theme.rule)),
            Span::styled(
                if add.second { "ADD SERVER" } else { "SERVER" },
                theme.accent_strong(),
            ),
            Span::raw(" "),
        ]));
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    frame.render_widget(Paragraph::new(lines), pane_body(inner));
}

/// The tail of a value that is wider than the field, in **columns**.
///
/// A URL is typed left to right and the interesting end is the one being typed,
/// so an over-long value shows its **end** with a leading `…` rather than its
/// start — the opposite of [`fit`], which is right for names and wrong for
/// something you are in the middle of entering.
///
/// **Columns, because the panel's width is columns.** This counted characters
/// while [`render`] sized itself from [`Line::width`], and the two disagree by a
/// factor of two on any wide script: `window(&"音".repeat(40), 38)` returned 38
/// characters — 76 columns — into a field the layout had budgeted 38 for, and
/// the panel then asked for a rectangle wider than the pane. The postcondition
/// is now stated rather than assumed: the result is **never wider than `width`**,
/// which is what makes the panel's width bounded by construction.
fn window(value: &str, width: usize) -> String {
    if columns(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    // One column goes to the leading ellipsis, as `fit` gives one to its
    // trailing one.
    let budget = width - 1;
    let mut start = value.len();
    let mut used = 0;
    for (at, c) in value.char_indices().rev() {
        let mut buf = [0u8; 4];
        let w = columns(c.encode_utf8(&mut buf));
        // A wide character that would straddle the budget is left out rather
        // than allowed one column past it.
        if used + w > budget {
            break;
        }
        used += w;
        start = at;
    }
    format!("…{}", &value[start..])
}

/// `enter next  esc cancel`, spelled the way [`crate::keys::BINDINGS`] spells the
/// two keys it shares with the rest of the Deck.
///
/// Read out of the table rather than written down, for the same reason
/// [`crate::ui::device_panel`]'s hint row is: a rebinding cannot leave this line
/// describing a keyboard the application no longer has.
fn hint_row(app: &App) -> Line<'static> {
    use crate::keys::Action;

    let key = |action: Action| -> String {
        crate::keys::BINDINGS
            .iter()
            .find(|b| b.action == action)
            .and_then(|b| b.label.split(' ').next())
            .unwrap_or_default()
            .to_string()
    };
    Line::from(Span::styled(
        format!(
            " tab field  {} next  {} cancel",
            key(Action::Open),
            key(Action::Back)
        ),
        Style::default().fg(app.theme.ink_faint),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AddServer;

    /// The panel has to fit the main pane at the Deck's minimum size — 80
    /// columns less the border, the sidebar and its rule — with the widest line
    /// it will ever lay out.
    #[test]
    fn the_widest_line_still_fits_an_eighty_column_deck() {
        let main_w = crate::ui::MIN_WIDTH - 2 - crate::ui::SIDEBAR_WIDTH - 1;
        let chrome = 2 + usize::from(GUTTER) * 2;
        for (what, w) in [
            ("a field row", 1 + LABEL_W + VALUE_W + 1),
            (
                "the note row",
                " a password is asked for next, on a plain terminal".len(),
            ),
            (
                "the second-server row",
                " saved to the config · EKO still uses the first"
                    .chars()
                    .count(),
            ),
            ("the hint row", " tab field  enter next  esc cancel".len()),
        ] {
            let widest = (w + chrome) as u16;
            assert!(
                widest <= main_w,
                "{what} makes a {widest}-column panel, and the main pane is {main_w}"
            );
        }
    }

    /// A long URL shows the end being typed, not the scheme.
    #[test]
    fn an_over_long_value_shows_its_tail_rather_than_its_head() {
        assert_eq!(window("short", 10), "short");
        assert_eq!(window("0123456789", 10), "0123456789");
        assert_eq!(window("0123456789x", 10), "…23456789x");
        assert_eq!(columns(&window("0123456789x", 10)), 10);
        // Multi-byte, one column each.
        let long = "ä".repeat(20);
        assert_eq!(columns(&window(&long, 5)), 5);
    }

    /// **The window is never wider than the field**, whatever is in it.
    ///
    /// This is the postcondition [`render`]'s width arithmetic depends on, and
    /// the one that did not hold: `window` counted characters while the panel
    /// measured columns, so 38 CJK characters came back as 76 columns for a
    /// 38-column field and the panel asked for more room than the pane had.
    #[test]
    fn a_window_is_never_wider_than_the_field_however_wide_the_script() {
        for value in [
            "音楽再生".repeat(40),
            "https://音楽.example.com/very/long/path".to_string(),
            "ä".repeat(200),
            "x音".repeat(60),
            "音".to_string(),
            String::new(),
        ] {
            for width in [0usize, 1, 2, 5, VALUE_W, 120] {
                let out = window(&value, width);
                assert!(
                    columns(&out) <= width,
                    "window({value:?}, {width}) is {} columns: {out:?}",
                    columns(&out)
                );
            }
        }
    }

    /// The widest panel any *typed value* can produce still fits an 80-column
    /// Deck.
    ///
    /// The sibling of [`the_widest_line_still_fits_an_eighty_column_deck`],
    /// which measures the constants. This measures what a person can put in the
    /// fields: [`crate::app::Field::max`] is columns and [`window`] is columns,
    /// so a field row is bounded by `1 + LABEL_W + VALUE_W + 1` whatever is
    /// typed or pasted — which is what makes [`render`]'s clamp a backstop
    /// rather than the only defence.
    #[test]
    fn no_typed_value_can_widen_a_field_row_past_the_main_pane() {
        let main_w = usize::from(crate::ui::MIN_WIDTH - 2 - crate::ui::SIDEBAR_WIDTH - 1);
        let chrome = 2 + usize::from(GUTTER) * 2;
        for field in Field::ALL {
            let mut typed = String::new();
            crate::line::type_into(&mut typed, &"音楽再生".repeat(200), field.max());
            assert!(
                columns(&typed) <= field.max(),
                "{field:?} took {} columns for a cap of {}",
                columns(&typed),
                field.max()
            );
            // ` ` + label + value + caret, the widest a field row can be.
            let row = 1 + LABEL_W + columns(&window(&typed, VALUE_W)) + 1;
            assert!(
                row + chrome <= main_w,
                "{field:?} full of wide characters makes a {}-column panel, and \
                 the main pane is {main_w}",
                row + chrome
            );
        }
    }

    /// The panel is drawn from `AddServer`, and `AddServer` has three fields.
    /// If a fourth is ever added, this is the line that has to be edited by
    /// hand — which is the point.
    #[test]
    fn the_panel_draws_exactly_the_three_fields_that_go_in_the_config() {
        assert_eq!(Field::ALL.len(), 3);
        let add = AddServer::default();
        for field in Field::ALL {
            assert!(add.value(field).is_empty());
        }
    }
}
