//! The source sidebar — the left column of the Deck.
//!
//! Two things, in order: the sources that work, and the key that gets you here.
//!
//! **Only working sources are rows.** They come from [`App::sources`], and every
//! one of them is selectable and does something: `Local` always, the configured
//! server and its search when there is one, and the queue. Moving the cursor
//! between them is what switches the main pane's library.
//!
//! ## The `NOT YET` section is gone
//!
//! Two names — `Artists` and `Lists` — used to sit under a dim `NOT YET`
//! heading, on the theory that naming an unbuilt view makes its absence read as
//! *unbuilt* rather than as *broken*. That was the right instinct applied to the
//! wrong thing. It was written when five of six rows did nothing, and it fixed
//! that by relabelling them; since then the server, the search and the queue all
//! became real rows, and what was left was two lines of furniture advertising
//! features that do not exist. A nav item that does nothing is worse than an
//! absent one, and "degrade honestly" means not promising what is not there.
//!
//! Nothing navigated to them: they were never in [`App::sources`], so no cursor
//! could reach them, and no key did anything with them. They were four columns
//! of prose and a heading, and they are four columns of air now.
//!
//! ## The lamp
//!
//! Every source carries one, in place of the bullet it used to have: `●` when
//! there is something behind the row *right now*, `○` when there is not. The
//! colour is the instrument vocabulary the seal already speaks — green for good,
//! amber for in flight, red for broken, dim for a step nobody has taken — so the
//! user has met it before. See [`lamp`].

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, ConnState, Focus, ScanState, SearchState, Source};
use crate::ui::pane_body;

/// Render the sources column.
pub fn render(frame: &mut Frame, outer: Rect, app: &App) {
    // Every row below is written flush left and laid out in the body, so the
    // gutter is [`crate::ui::GUTTER`] once rather than a leading space on each
    // of a dozen `format!`s.
    let area = pane_body(outer);
    let theme = &app.theme;
    let focused = app.focus == Focus::Sidebar;
    let heading = |text: &'static str| {
        Line::from(Span::styled(
            text,
            Style::default()
                .fg(theme.ink_faint)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let mut lines = vec![heading("SOURCES"), Line::default()];

    for index in app.visible_sources() {
        let row = &app.sources[index];
        let selected = index == app.selected;
        // The cursor is only *the* cursor when this pane has the keyboard;
        // elsewhere it is a memory of where you were, and is drawn as one.
        let style = if selected && focused {
            theme.accent_strong()
        } else if selected {
            Style::default().fg(theme.accent)
        } else if row.depth == 0 {
            Style::default().fg(theme.ink)
        } else {
            Style::default().fg(theme.ink_dim)
        };

        let mut spans = Vec::with_capacity(4);
        if row.depth > 0 {
            spans.push(Span::raw("  "));
        }
        if row.action {
            // **A `+`, not a lamp.** A lamp answers "is there something behind
            // this row right now", and for `+ Add server` the answer is "no, and
            // that is what the row is for" — a hollow lamp beside it would read
            // as a source that is broken rather than one that is not there yet.
            // See [`crate::app::SourceRow::action`].
            spans.push(Span::styled("+ ", style));
        } else if row.expandable {
            // A caret says there is something to open. A lamp would say there is
            // something here, which is a different claim.
            let caret = if row.expanded { "▾" } else { "▸" };
            spans.push(Span::styled(format!("{caret} "), style));
        } else {
            let (glyph, colour) = lamp(app, row.source);
            spans.push(Span::styled(
                format!("{glyph} "),
                Style::default().fg(colour),
            ));
        }
        spans.push(Span::styled(row.label.clone(), style));

        // The badge is the source's real album count, right-aligned. Dropped
        // rather than clipped when the label already fills the column.
        if let Some(badge) = &row.badge {
            let used: usize = spans.iter().map(Span::width).sum::<usize>() + badge.chars().count();
            // The body already excludes the gutter, so the count lands one cell
            // clear of the pane rule without any arithmetic of its own.
            let width = area.width as usize;
            if used < width {
                spans.push(Span::raw(" ".repeat(width - used)));
                spans.push(Span::styled(
                    badge.clone(),
                    Style::default().fg(theme.ink_faint),
                ));
            }
        }
        lines.push(Line::from(spans));

        // The server's connection state, as an annotation on the server's own
        // row rather than a separate entry — and only when it says more than the
        // lamp beside the name already does. A green lamp over the word
        // `connected` is the same shout twice.
        if row.source == Source::Remote {
            if let Some((word, colour)) = connection_word(app) {
                // One space. This is an annotation on the row above rather than
                // another row, so a narrower indent is the honest shape; and
                // `connecting` is ten characters, so it is also the one that
                // keeps the longest word clear of the pane rule.
                lines.push(Line::from(Span::styled(
                    format!(" {word}"),
                    Style::default().fg(colour),
                )));
            }
        }
    }

    let used = lines.len() as u16;
    frame.render_widget(Paragraph::new(lines), area);
    render_focus_hint(frame, area, app, used);
}

/// One word for the server's connection, in the instrument-lamp colours.
///
/// A sidebar column is not room for a sentence, so the column carries the
/// *state* and the footer carries the *reason* — the failure's real message,
/// which is the part with a fix in it. The colours are the ones the seal already
/// uses, so the vocabulary is one the user has met: amber for in flight, red for
/// broken. `sign in` is dim rather than red because nothing is wrong; a step has
/// not been taken.
///
/// `None` for [`ConnState::NotConfigured`] — a Deck nobody asked to connect must
/// not grow a line about connecting — and **`None` for
/// [`ConnState::Connected`]**, because the green lamp on the row above is that
/// word. The other three say something the lamp's colour does not.
///
/// Every word here is at most ten characters, so ` word` fits the sidebar body
/// — [`crate::ui::SIDEBAR_WIDTH`] less its two [`crate::ui::GUTTER`] columns —
/// with room to spare. `the_sidebar_never_touches_its_own_gutters` holds that.
fn connection_word(app: &App) -> Option<(&'static str, Color)> {
    let theme = &app.theme;
    match &app.conn {
        ConnState::NotConfigured | ConnState::Connected { .. } => None,
        ConnState::NeedsPassword { .. } => Some(("sign in", theme.ink_dim)),
        ConnState::Connecting { .. } => Some(("connecting", theme.led_amber)),
        ConnState::Failed { .. } => Some(("failed", theme.led_red)),
    }
}

/// One source's lamp: **is there something behind this row right now?**
///
/// `●` yes, `○` no, and the colour says which kind of no. It is deliberately the
/// same four-colour vocabulary the seal and the connection word use — green
/// good, amber in flight, red broken, dim *nobody has done the thing yet* — so
/// the column can be read at a glance without learning a second alphabet.
///
/// It is a lamp, not a claim about audio: the seal is the only thing in this
/// application that says anything about the signal, and nothing here can be
/// mistaken for it — it is in another pane, beside a source name, and it never
/// carries a word.
///
/// Search reports its **own** state rather than the server's. It is a question,
/// not a library: a connected server nobody has asked anything is an empty
/// pane, and a hollow lamp is the truthful thing to put beside it.
fn lamp(app: &App, source: Source) -> (&'static str, Color) {
    let theme = &app.theme;
    match source {
        Source::Local => match &app.scan {
            ScanState::Discovering { .. } | ScanState::Reading { .. } => ("○", theme.led_amber),
            ScanState::Missing(_) | ScanState::Failed(_) => ("○", theme.led_red),
            ScanState::Ready if !app.library.is_empty() => ("●", theme.led_green),
            ScanState::Ready | ScanState::Unconfigured => ("○", theme.ink_dim),
        },
        Source::Remote => match &app.conn {
            ConnState::Connected { .. } => ("●", theme.led_green),
            ConnState::Connecting { .. } => ("○", theme.led_amber),
            ConnState::Failed { .. } => ("○", theme.led_red),
            ConnState::NotConfigured | ConnState::NeedsPassword { .. } => ("○", theme.ink_dim),
        },
        Source::Search => match &app.search.state {
            SearchState::Searching => ("○", theme.led_amber),
            SearchState::Failed(_) => ("○", theme.led_red),
            SearchState::Ready if !app.search.results.is_empty() => ("●", theme.led_green),
            SearchState::Ready | SearchState::Idle => ("○", theme.ink_dim),
        },
        Source::Queue if app.queue.is_empty() => ("○", theme.ink_dim),
        Source::Queue => ("●", theme.led_green),
    }
}

/// `tab` — pinned to the bottom of the column, always.
///
/// `Action::ToggleFocus` has been bound to `Tab` since the first draft and
/// nothing on screen said so: the keymap in the main pane names it, but the
/// keymap only appears when the library is *empty*, so the moment the Deck
/// started working the binding became invisible. That is how the owner came to
/// ask how to reach the sources column at all.
///
/// It names the pane the key will move you **to**, not the binding's own
/// description, so it teaches what happens rather than restating the keymap. It
/// lives here rather than in the footer because the footer's four rows belong to
/// the transport and the seal, and because this is the column the key reveals.
///
/// Drawn only when there is a row spare under the content — a hint that
/// overwrites a source is worse than no hint.
fn render_focus_hint(frame: &mut Frame, area: Rect, app: &App, used: u16) {
    if area.height == 0 || used >= area.height {
        return;
    }
    let theme = &app.theme;
    let target = match app.focus {
        Focus::Sidebar => "library",
        Focus::Main => "sources",
    };
    let row = Rect {
        x: area.x,
        y: area.bottom() - 1,
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("tab ", Style::default().fg(theme.ink_dim)),
            Span::styled(target, Style::default().fg(theme.ink_faint)),
        ])),
        row,
    );
}
