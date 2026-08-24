//! The output-device picker.
//!
//! One row per device the host lists, with the system default first. A modal
//! overlay **over the main pane only**, for the same reason
//! [`crate::ui::eq_panel`] is: the footer is not in its area and cannot be,
//! because the seal is never off-screen and the device is precisely what the seal
//! is describing. A picker that covered the claim it changes would be the worst
//! possible overlay in this application.
//!
//! ## What the drawing is allowed to know
//!
//! Nothing here decides anything. The rows are [`crate::app::App::devices`] as
//! the host listed them; which row is in use is
//! [`crate::app::App::device_in_use_row`], which reads the engine's live stream —
//! the same field the seal names its device from, so the marker and the seal are
//! one fact drawn twice; and a device that is chosen but not in use, or
//! configured and **not** in the list at all, gets a sentence rather than a row —
//! because neither is a device you can point a `▶` at right now, and drawing one
//! as a marked line among the others would be the screen claiming the DAC is
//! playing.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::config::OutputDevice;
use crate::ui::footer::fit;
use crate::ui::{pane_body, GUTTER};

/// How the system default is named. Not a device name — the *absence* of a
/// preference — so it is spelled differently from anything the host could return.
pub const SYSTEM_DEFAULT: &str = "System default";

/// Widest row the panel will lay out for. A device name longer than this is cut
/// with `…` rather than allowed to widen the panel past the main pane.
const MAX_NAME: usize = 34;

/// ` ▶ ` or ` ▸ ` or `   ` — the marker column.
const MARKER_W: usize = 3;

/// The border, the blank, and the hint row — everything that is not a device.
const CHROME_ROWS: usize = 4;

/// Draw the picker centred in `area`, which is the main pane.
///
/// # How it does not fit
///
/// Unlike [`crate::ui::eq_panel`], whose size is fixed by eleven sliders, this
/// list is as long as the machine's device count — which can be twelve on a
/// laptop with a dock, and there is no height at which that is guaranteed to fit.
/// So the *list* is windowed rather than the panel refused: the window follows the
/// cursor, so every device stays reachable with `j`/`k`, and the hint row says
/// `4 of 12` so a short list is never mistaken for the whole list. Drawing nothing
/// would leave `d` looking like a broken key.
///
/// It still draws nothing when there is no room for a single row plus its chrome.
/// Below that there is nothing to window.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // At most one of these is ever non-empty — a device cannot be both absent and
    // merely not-yet-in-use — but they are appended rather than chosen between so
    // the budget below is right whichever it is.
    let mut note = missing_note(app);
    note.extend(pending_note(app));
    // Rows left for devices once the chrome and the missing-device sentence have
    // had theirs.
    let Some(budget) = (area.height as usize).checked_sub(CHROME_ROWS + note.len()) else {
        return;
    };
    if budget == 0 {
        return;
    }
    let total = app.device_rows();
    let shown = total.min(budget);
    let first = window(total, app.device_cursor, budget);

    // Every line, including the hint row, **before** the panel is measured. The
    // hint is the widest line whenever the device names are short, and a panel
    // sized on the rows alone clips it to `esc cl` — the same class of silent
    // overflow the footer's hint block derives its width to avoid.
    let mut lines = rows(app, first, shown);
    lines.extend(note);
    lines.push(Line::default());
    lines.push(hint_row(app, shown, total));

    let want_w = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        // The border, then the Deck's own gutter on each side.
        .saturating_add(2 + usize::from(GUTTER) * 2) as u16;
    let want_h = (lines.len() + 2) as u16;
    if area.width < want_w.max(8) || area.height < want_h {
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
            Span::styled("OUTPUT", theme.accent_strong()),
            Span::raw(" "),
        ]));
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    frame.render_widget(Paragraph::new(lines), pane_body(inner));
}

/// The first row of the window: the whole list when it fits, else a window
/// centred on the cursor and clamped to the ends.
///
/// Pure, so the scrolling is a table of cases rather than something to be found
/// by resizing a terminal.
fn window(total: usize, cursor: usize, budget: usize) -> usize {
    if total <= budget {
        return 0;
    }
    cursor
        .saturating_sub(budget / 2)
        .min(total.saturating_sub(budget))
}

/// `shown` device rows starting at `first`.
fn rows(app: &App, first: usize, shown: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let in_use = app.device_in_use_row();
    (first..first + shown)
        .map(|row| {
            let name = match row.checked_sub(1) {
                None => SYSTEM_DEFAULT.to_string(),
                Some(i) => app.devices.get(i).cloned().unwrap_or_default(),
            };
            let selected = row == app.device_cursor;
            // `▶` for what the engine is actually on, `▸` for the cursor — the
            // same two markers the album and queue lists use, meaning the same
            // two things.
            let marker = if in_use == Some(row) {
                "▶"
            } else if selected {
                "▸"
            } else {
                " "
            };
            let style = match (in_use == Some(row), selected) {
                (true, _) => Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                (false, true) => Style::default().fg(theme.ink).add_modifier(Modifier::BOLD),
                (false, false) => Style::default().fg(theme.ink_dim),
            };
            Line::from(Span::styled(
                format!(" {marker} {}", fit(&name, MAX_NAME)),
                style,
            ))
        })
        .collect()
}

/// The sentence a configured-but-absent device gets **instead of a row**, and an
/// empty vector when there is not one.
///
/// Not a row, because it is not a device you can choose right now, and a
/// selectable line for a DAC that is asleep would be the screen claiming it is
/// there. Said as the fact plus what is happening instead, because the engine
/// really is on the system default and the seal really is describing that.
fn missing_note(app: &App) -> Vec<Line<'static>> {
    let OutputDevice::Missing(name) = &app.device else {
        return Vec::new();
    };
    vec![
        Line::default(),
        Line::from(Span::styled(
            format!(
                "{}{} is not connected",
                " ".repeat(MARKER_W),
                fit(name, MAX_NAME)
            ),
            Style::default().fg(app.theme.led_amber),
        )),
    ]
}

/// The sentence a device that has been **chosen but is not what is playing** gets,
/// and an empty vector when there is not one.
///
/// A paused Deck keeps its session — and therefore its output device — until the
/// next track starts, so the `▶` stays where the audio is (see
/// [`crate::app::App::device_in_use_row`]) and this line carries the other half of
/// the truth: the choice was taken, and here is when it applies. The wording is
/// the footer note's, because it is the same statement and a second phrasing of
/// it would be a second thing to keep true.
fn pending_note(app: &App) -> Vec<Line<'static>> {
    let Some(name) = app.device_pending() else {
        return Vec::new();
    };
    vec![
        Line::default(),
        Line::from(Span::styled(
            format!(
                "{}{} from the next track",
                " ".repeat(MARKER_W),
                fit(name, MAX_NAME)
            ),
            Style::default().fg(app.theme.ink_dim),
        )),
    ]
}

/// `enter select  esc close`, spelled the way [`crate::keys::BINDINGS`] spells it,
/// plus `4 of 12` whenever the list is windowed.
///
/// The keys are read out of the table rather than written down, for the same
/// reason [`crate::ui::eq_panel`]'s hint row is: a rebinding cannot leave this
/// line describing a keyboard the application no longer has. The count is there
/// because a windowed list that did not say so would be a short list presenting
/// itself as the whole one.
fn hint_row(app: &App, shown: usize, total: usize) -> Line<'static> {
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
    let mut hint = format!("{} select  {} close", key(Action::Open), key(Action::Back));
    if shown < total {
        hint.push_str(&format!("  {shown} of {total}"));
    }
    Line::from(vec![
        Span::raw(" ".repeat(MARKER_W)),
        Span::styled(hint, Style::default().fg(theme.ink_faint)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel has to fit the main pane at the Deck's minimum size even with the
    /// longest name it will lay out — 80 columns less the border, the sidebar and
    /// its rule.
    ///
    /// Every kind of line is measured, not just the device rows: the panel's width
    /// is the *widest* line in it, and both sentences are a truncated name plus a
    /// clause, so either of them is wider than any row. A test that only measured
    /// the rows would pass a panel that the sentences push off the pane.
    #[test]
    fn the_widest_line_still_fits_an_eighty_column_deck() {
        let main_w = crate::ui::MIN_WIDTH - 2 - crate::ui::SIDEBAR_WIDTH - 1;
        // The border and one gutter column on each side, as `render` adds.
        let chrome = 2 + usize::from(GUTTER) * 2;
        let row = MARKER_W + MAX_NAME + 1;
        let missing = MARKER_W + MAX_NAME + " is not connected".len();
        let pending = MARKER_W + MAX_NAME + " from the next track".len();
        for (what, w) in [
            ("a row", row),
            ("the absent note", missing),
            ("the pending note", pending),
        ] {
            let widest = (w + chrome) as u16;
            assert!(
                widest <= main_w,
                "{what} with a {MAX_NAME}-column device name makes a {widest}-column \
                 panel, and the main pane is {main_w}"
            );
        }
    }

    /// A list that fits is not windowed, and one that does not follows the cursor
    /// to both ends without ever running past them.
    #[test]
    fn the_window_follows_the_cursor_and_stops_at_both_ends() {
        // Everything fits: no window at all.
        for cursor in 0..5 {
            assert_eq!(window(5, cursor, 9), 0, "a list that fits was scrolled");
        }
        // Twelve rows into four. The cursor is always inside the window.
        for cursor in 0..12 {
            let first = window(12, cursor, 4);
            assert!(first <= 8, "{cursor}: window ran past the end at {first}");
            assert!(
                (first..first + 4).contains(&cursor),
                "{cursor}: the cursor fell outside the window at {first}"
            );
        }
        assert_eq!(window(12, 0, 4), 0);
        assert_eq!(window(12, 11, 4), 8);
        // Degenerate budgets must not panic or wrap.
        assert_eq!(window(0, 0, 1), 0);
        assert_eq!(window(3, 2, 1), 2);
    }
}
