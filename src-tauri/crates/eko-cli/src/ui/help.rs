//! The help overlay — `?`.
//!
//! The whole keyboard, on one screen, and **the only complete rendering of it**.
//!
//! ## One table, one layout
//!
//! Every row comes from [`crate::keys::BINDINGS`], and it is laid out by
//! [`crate::ui::main_pane::key_hints`] — the *same* function the empty pane uses,
//! including its column-width measurement and its `…`. Nothing here hand-lists a
//! binding, and nothing here is a second, prettier rendering of the table that
//! could disagree with the first about how many columns fit or whether a row was
//! dropped. This module chooses a rectangle and hands it over.
//!
//! That matters immediately rather than in principle: Task 4 of this phase added
//! two bindings and Task 5 a third, and the empty pane's copy of the table now
//! truncates at 100×30 because the arithmetic ran out. A hand-written overlay
//! would have gone stale on exactly that change.
//!
//! ## Where it is drawn, and what it therefore cannot touch
//!
//! Over the **body** — the sidebar, its rule and the main pane — and never the
//! footer. Two things follow, and neither is a promise this module has to keep by
//! being careful:
//!
//! * **The seal is untouched.** It is the last row of the footer, the footer is
//!   drawn after this and is in no rect this module computes, and
//!   `crate::ui::tests::the_help_overlay_never_covers_the_seal_at_any_usable_size`
//!   asserts it at five sizes. A full-frame overlay was the alternative — it would
//!   fit the entire table at 80×20, which this does not — and it was refused: the
//!   governing constraint of this application is that the seal is never
//!   off-screen, and the one screen that would be allowed to break it must not be
//!   the one whose only content is a list of keys.
//! * **The cover art is untouched, and the [`crate::art::Painter`] needs no help.**
//!   kitty and iTerm2 covers are written straight to fd 1, outside ratatui's
//!   buffer, positioned by [`crate::ui::art_area`] — so an overlay drawn over
//!   those cells would have the image sitting on top of it with nothing in the
//!   buffer model to notice, and `Painter::sync` would write zero bytes because
//!   the placement had not changed. The reason that cannot happen here is
//!   geometric rather than careful: `art_area` resolves inside `footer_area`, and
//!   this rect is inside `body`. They do not intersect at any terminal size, which
//!   `crate::ui::tests::no_overlay_can_reach_the_cover_art_or_the_seal` asserts.
//!   No `invalidate`, no take-down-and-redraw, and nothing to restore when the
//!   overlay closes — because nothing was ever covered.
//!
//! ## Not fitting
//!
//! Below 80×20 there is no overlay, because there is no Deck: [`crate::ui::draw`]
//! renders the "needs 80 × 20" card and returns before any of this runs. At and
//! above it the overlay is drawn, and the table is cut with a `…` when the rows do
//! not fit — 80×20 leaves eleven rows for a fourteen-row table, and there is no
//! column count that fixes that without truncating the descriptions, which would
//! be a worse lie than a visible cut. It degrades; it does not garble.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::ui::main_pane::{hint_column_width, key_hints};

/// Columns of padding inside the border, on each side. **The Deck's own** —
/// [`crate::ui::GUTTER`], not a second opinion about the same number.
const PAD: u16 = crate::ui::GUTTER;

/// How wide the overlay wants to be: the border, the padding, and as many hint
/// columns as [`hint_column_width`] says will fit.
///
/// Derived, never written down. A binding with a longer description widens
/// [`hint_column_width`] and therefore this, so the overlay cannot silently start
/// clipping its own second column — the property the footer's hint block already
/// has, and the reason this reads the same function rather than a constant.
fn want_width(available: u16) -> u16 {
    let chrome = 2 + PAD * 2;
    let column = hint_column_width() as u16;
    let body = available.saturating_sub(chrome);
    let columns = (body / column).clamp(1, 2);
    (columns * column + chrome).min(available)
}

/// Draw the overlay over `area`, which is the whole body.
///
/// Draws nothing at all when there is not room for the border, the title rule and
/// a row of hints — a state [`crate::ui::draw`] has already ruled out above 80×20,
/// kept because this function is not the place to assume its caller's minimum.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let want_w = want_width(area.width);
    // The border, and at least one row to put a binding on.
    if area.width < want_w || area.height < 3 || want_w < 2 + PAD * 2 {
        return;
    }
    // As tall as it needs, never taller than the body. The body is the ceiling,
    // and the footer is not in it.
    let text_w = want_w - 2 - PAD * 2;
    let columns = (text_w / hint_column_width() as u16).max(1) as usize;
    let rows = crate::keys::BINDINGS.len().div_ceil(columns) as u16;
    let want_h = (rows + 2).min(area.height);

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
            Span::styled("KEYS", theme.accent_strong()),
            Span::raw(" "),
        ]));
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    let text = Rect {
        x: inner.x + PAD,
        y: inner.y,
        width: inner.width.saturating_sub(PAD * 2),
        height: inner.height,
    };
    if text.is_empty() {
        return;
    }

    // The one layout for the keymap. `key_hints` decides the columns, the order
    // and the `…`; this only says how much room there is.
    let mut lines: Vec<Line<'static>> = Vec::new();
    key_hints(app, &mut lines, text.height as usize, text.width as usize);
    frame.render_widget(Paragraph::new(lines), text);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlay fits the body at the Deck's minimum size, and takes two
    /// columns the moment there is room for them.
    #[test]
    fn the_width_is_measured_off_the_table_and_never_exceeds_the_body() {
        let column = hint_column_width() as u16;
        let chrome = 2 + PAD * 2;
        // The body at 80 and at 100 columns: the terminal less the outer border.
        for available in [crate::ui::MIN_WIDTH - 2, 98, 238] {
            let w = want_width(available);
            assert!(w <= available, "{available}: overlay wants {w}");
            assert!(
                w >= column + chrome || available < column + chrome,
                "{available}: overlay took {w}, less than one column"
            );
        }
        // Narrower than one column: it still asks for one rather than zero, and
        // still does not exceed what it was given — including at zero, where the
        // arithmetic has the most room to underflow.
        for available in 0..=column + chrome {
            assert!(
                want_width(available) <= available,
                "{available}: overlay wants {}",
                want_width(available)
            );
        }
        // Two columns as soon as two fit, never three — three would need the
        // descriptions cut, and a cut description is worse than a cut row.
        assert_eq!(want_width(2 * column + chrome), 2 * column + chrome);
        assert_eq!(want_width(9 * column + chrome), 2 * column + chrome);
    }
}
