//! The instrument footer — four rows that never leave the screen.
//!
//! Row 1  cover · transport glyph, title · spectrum
//! Row 2  cover · artist and album · queue position, or the sleep timer
//! Row 3  cover · the scrubber, a clock at each end and nothing else
//! Row 4  cover · signal path and the seal
//!
//! ## What each row's right-hand side is for
//!
//! Every row is a pair: something that names what is playing on the left, and
//! something that measures it on the right. Row 2 was the one row with nothing
//! on the right, and row 3 was carrying two things — the scrubber *and* the
//! sleep timer, which shortened the scrubber by eleven columns to show a number
//! that has nothing to do with the position in the track. So the sleep timer
//! moved up to row 2, where it takes the place of the queue position while it
//! runs, and the scrubber took the whole width back.
//!
//! The transport glyph moved with it, up to the head of the title. It is the
//! one thing on screen that separates *paused* from *playing*, so it could not
//! simply be dropped; and next to the title is where it says something, because
//! `▶` beside a name reads as "this is playing" while `▶` beside a clock reads
//! as a button.
//!
//! The fourth row is the reason the footer is pinned. **The seal never lies and
//! the seal is never off-screen** — it is what makes this EKO rather than
//! another ncurses player. See [`signal_path`] for what that costs.
//!
//! The footer paints **no background of its own**. It used to fill `area` with
//! `--screen` (`#181b16`) so that it would read as a device screen; the GUI
//! never consumes that token, and eleven points under the Graphite ground is a
//! black box, not a recess — see the note in [`crate::ui::theme`]. Everything
//! below is a foreground colour on the same ground the sidebar and the main pane
//! sit on, and the rule above the footer is what separates the two.

use eko_core::signal_path::SignalPath;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, Playback};
use crate::art;
use crate::ui::{justified, GUTTER};

/// Cover art width, in cells.
///
/// **Eight, because eight is square.** A terminal cell is twice as tall as it
/// is wide — [`crate::art`]'s `CELL_PX_W` is 10 and its `CELL_PX_H` is 20 — so a
/// block of [`crate::ui::FOOTER_HEIGHT`] rows is square only at `2 × rows`
/// cells. At six it was 60×80 device pixels, a 3:4 portrait, and
/// `resize_to_fill` trimmed a quarter of a square cover's width away to reach
/// it. Nothing about that was visible in a text dump; it was visible in every
/// screenshot.
pub const ART_WIDTH: u16 = 8;
/// Gap between the art and the text column.
pub const ART_GAP: u16 = 2;
/// Bands in the spectrum, matching the engine's `engine_bands`.
const SPECTRUM_BANDS: usize = 32;

// There is no volume meter. It read `App::volume`, which is now pinned at
// unity, so the meter could only ever draw seven full segments and `100` — a
// control-shaped thing that reports a constant. See [`crate::keys`] for why the
// control went.

/// The bar glyphs, floor to ceiling.
const BAR_GLYPHS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Render the footer into `area`, which is exactly [`FOOTER_HEIGHT`] rows tall.
///
/// [`FOOTER_HEIGHT`]: crate::ui::FOOTER_HEIGHT
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    if area.is_empty() {
        return;
    }

    render_art(frame, art_rect(area), app);

    let text_x = area.x + GUTTER + ART_WIDTH + ART_GAP;
    let Some(text_area) = text_column(area, text_x) else {
        return;
    };
    let w = text_area.width;

    let playing = app.playback != Playback::Stopped;
    let ink = if playing { theme.ink } else { theme.ink_dim };

    let (title_text, artist_text) = match &app.now {
        Some(now) if playing => (
            now.title.clone(),
            format!("{} · {}", now.artist, now.album_name),
        ),
        _ => ("Nothing playing".to_string(), "—".to_string()),
    };

    // Both text rows are laid out right group first and truncated on the left,
    // the rule the seal row has always used: nothing a file is called can push
    // the thing beside it off the row. See [`justify_left_truncated`].
    let glyph = match app.playback {
        Playback::Playing => "▶ ",
        Playback::Paused => "⏸ ",
        Playback::Stopped => "⏹ ",
    };
    let title = justify_left_truncated(
        vec![Span::styled(glyph, theme.on_console(theme.accent))],
        &title_text,
        Style::default().fg(ink).add_modifier(Modifier::BOLD),
        spectrum(app),
        w,
    );
    let artist = justify_left_truncated(
        Vec::new(),
        &artist_text,
        theme.on_console(theme.ink_faint),
        elapsed_or_sleep(app),
        w,
    );

    // Derived once for the frame — see [`App::seal`].
    let seal = app.seal();

    frame.render_widget(
        Paragraph::new(vec![
            title,
            artist,
            scrubber(app, w),
            signal_path(app, &seal, w),
        ]),
        text_area,
    );
}

/// One row: a fixed prefix, a piece of text that gives way, and a right group
/// that does not.
///
/// **The right group is measured first and the text is truncated into whatever
/// is left.** This is the seal row's rule generalised to the two rows above it,
/// which did not have it: they laid the text out at full length and let the
/// rect clip it, so a long enough track title silently took the spectrum with
/// it and a long enough album name ran to the frame with nothing to say it had
/// been cut. There is now no arrangement in which a name pushes anything off
/// any row of this footer.
fn justify_left_truncated(
    prefix: Vec<Span<'static>>,
    text: &str,
    style: Style,
    right: Vec<Span<'static>>,
    width: u16,
) -> Line<'static> {
    let prefix_w: usize = prefix.iter().map(Span::width).sum();
    let right_w: usize = right.iter().map(Span::width).sum();
    // One column of gap when the text fits as it is; four when it has to be
    // cut, so `…▁▁▁` does not read as one thing. The seal row's arithmetic.
    let room = (width as usize).saturating_sub(prefix_w + right_w);
    let room = if Span::raw(text).width() < room {
        room.saturating_sub(1)
    } else {
        room.saturating_sub(4)
    };
    let mut left = prefix;
    left.push(Span::styled(fit(text, room), style));
    crate::ui::justified(left, right, width)
}

/// The right-hand side of the artist row: **`3 of 10`, or the sleep timer.**
///
/// The queue position is the fact a queue is consulted for, and this was the
/// only row in the footer with nothing on its right. A running sleep timer
/// takes the slot instead — it is the more urgent of the two, it is transient,
/// and a Deck that is about to stop playing should say so louder than it says
/// where it is in a list.
///
/// Empty when there is no timer and no position: an unplayed queue has no
/// "3 of 10" to report, and inventing one is inventing a claim.
fn elapsed_or_sleep(app: &App) -> Vec<Span<'static>> {
    let theme = &app.theme;
    if let Some(sleep) = &app.sleep {
        return vec![
            Span::styled("sleep ", theme.on_console(theme.ink_faint)),
            Span::styled(
                clock(sleep.remaining.as_millis().min(u128::from(u64::MAX)) as u64),
                theme.on_console(theme.led_amber),
            ),
        ];
    }
    match app.queue.position() {
        Some(at) => vec![Span::styled(
            format!("{} of {}", at + 1, app.queue.len()),
            theme.on_console(theme.ink_faint),
        )],
        None => Vec::new(),
    }
}

/// The text column, or `None` if the footer is too narrow to hold one.
fn text_column(area: Rect, x: u16) -> Option<Rect> {
    // The right-hand gutter, mirroring the one the art block sits behind.
    let right = area.right().saturating_sub(GUTTER);
    if x >= right {
        return None;
    }
    Some(Rect {
        x,
        y: area.y,
        width: right - x,
        height: area.height,
    })
}

/// The block the cover occupies, given the footer's `area`.
///
/// **The one definition of that rect.** [`render`] lays the text column out
/// `ART_GAP` columns past its right edge, and [`crate::ui::art_area`] resolves
/// the same rect from the terminal size so the fold can key its cache on it and
/// [`crate::art::Painter`] can position an out-of-band image in it. A second
/// arithmetic anywhere is how an image ends up over the seal.
#[must_use]
pub fn art_rect(area: Rect) -> Rect {
    Rect {
        x: area.x + GUTTER,
        y: area.y,
        width: ART_WIDTH.min(area.width.saturating_sub(GUTTER)),
        height: area.height,
    }
}

/// The cover-art block: real pixels when there are any, the placeholder when
/// there are not.
///
/// Three cases, and the choice between them is the whole of "degrade honestly".
///
/// * **[`art::Rendered::Cells`]** — the halfblock renderer. Each cell is a `▀`
///   whose foreground is the upper pixel and whose background is the lower one,
///   so one cell carries two. These are the only cells in the Deck that set a
///   background, and they set it because that *is* the lower pixel; see
///   `crate::ui::tests::no_cell_in_the_deck_paints_its_own_background`, which
///   asserts the rule everywhere art is not loaded.
/// * **[`art::Rendered::Escape`]** — kitty or iTerm2. The pixels are written
///   outside this buffer by [`crate::art::Painter`] *after* the frame is
///   flushed, so what goes in the buffer is **blank cells**: ratatui then owns
///   them, its diff leaves them alone while the image is up, and it repaints
///   them the moment the state changes back. Drawing the placeholder underneath
///   instead would show through on any terminal that lied about its protocol.
/// * **anything else** — loading, unavailable, no track, no art: the
///   placeholder, at exactly the size the art would have been.
///
/// The size check is not defensive padding. `cells` was rendered for the grid
/// the fold asked for; if a resize has happened since and the answer has not
/// caught up, the grid on screen is the wrong shape and the placeholder is the
/// honest thing to show for the one frame it takes to catch up.
fn render_art(frame: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() {
        return;
    }
    // **Nothing, while the visualiser is up.** The cover is on screen, at forty
    // times this area, four rows above — and the cache holds it at *that* grid,
    // so the size check below would fail and this block would draw the
    // placeholder underneath the real thing. An empty eight columns is the
    // honest answer: the picture has not gone, it has moved. The rect is still
    // reserved so the text column does not shift when `z` is pressed.
    if app.visualiser {
        frame.render_widget(Clear, area);
        return;
    }
    match app.art.rendered() {
        Some(art::Rendered::Cells(cells))
            if cells.cols == area.width && cells.rows == area.height =>
        {
            paint_cells(frame, area, cells, app.theme.depth);
        }
        // The pixels arrive from outside the buffer; these cells stay ratatui's.
        Some(art::Rendered::Escape(_)) => frame.render_widget(Clear, area),
        _ => {
            let style = app.theme.on_console(app.theme.rule);
            let block = "░".repeat(area.width as usize);
            let rows: Vec<Line> = (0..area.height)
                .map(|_| Line::from(Span::styled(block.clone(), style)))
                .collect();
            frame.render_widget(Paragraph::new(rows), area);
        }
    }
}

/// Paint a halfblock grid, cell by cell.
///
/// Straight into the buffer rather than through a widget: a `▀` needs a
/// per-cell foreground *and* background, which no ratatui text primitive
/// expresses without one `Span` per cell and a `Style` allocation with it.
///
/// Every colour goes through [`crate::ui::theme::Rgb::resolve`], the same
/// quantiser the palette uses, so a 256-colour terminal gets the nearest slot
/// rather than a truecolor escape it would print as text.
///
/// `pub(crate)` because [`crate::ui::visualiser`] draws **the same cover** at a
/// different rect. It is one renderer with one rect argument, not two renderers
/// that would have to be kept agreeing about what a `▀` means.
pub(crate) fn paint_cells(
    frame: &mut Frame,
    area: Rect,
    cells: &art::Cells,
    depth: crate::ui::theme::ColorDepth,
) {
    let buf = frame.buffer_mut();
    for row in 0..area.height {
        for col in 0..area.width {
            let Some((top, bottom)) = cells.at(col, row) else {
                continue;
            };
            let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) else {
                continue;
            };
            cell.set_symbol("▀").set_style(
                Style::default()
                    .fg(top.resolve(depth))
                    .bg(bottom.resolve(depth)),
            );
        }
    }
}

/// The 32-band spectrum, right-aligned on the title row.
///
/// Empty when hidden or when the row is too narrow — the title is the more
/// important thing on that row, and a clipped spectrum is worse than none.
///
/// Bands come from `Engine::bands`. When the engine has not reported any — not
/// playing, or a session that has not filled its first FFT — every bar sits on
/// the floor. A floor is honest; an animated placeholder would not be.
fn spectrum(app: &App) -> Vec<Span<'static>> {
    if !app.spectrum_visible {
        return Vec::new();
    }
    let theme = &app.theme;
    (0..SPECTRUM_BANDS)
        .map(|i| {
            let level = app.bands.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let glyph = BAR_GLYPHS[((level * (BAR_GLYPHS.len() - 1) as f32).round() as usize)
                .min(BAR_GLYPHS.len() - 1)];
            Span::styled(glyph, theme.on_console(theme.led_for(level)))
        })
        .collect()
}

/// `mm:ss` from milliseconds, saturating at 99:59 so the column cannot grow.
fn clock(ms: u64) -> String {
    let secs = (ms / 1000).min(99 * 60 + 59);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// `00:00 ━━━━━━━━──────────────── 04:51`
///
/// **A clock at each end and nothing else between them.** The row used to carry
/// the transport glyph as well, and the sleep timer, which between them took
/// thirteen columns out of the one measurement on the screen that is *about*
/// width — the further along the bar, the further along the track. Both have
/// moved up a row; see the module header.
///
/// The unplayed half is [`crate::ui::theme::SCRUBBER_TRACK`] rather than the
/// border colour it used to be. It is a control, not a frame, and at `RULE` it
/// was 2.03:1 on the design's own ground — under the 3:1 a non-text component
/// is meant to clear, and it looked it.
fn scrubber(app: &App, width: u16) -> Line<'static> {
    let theme = &app.theme;
    let dur_ms = app.dur_ms();

    let left = vec![Span::styled(
        format!("{} ", clock(app.pos_ms)),
        theme.on_console(theme.ink_faint),
    )];
    let right = vec![Span::styled(
        format!(" {}", clock(dur_ms)),
        theme.on_console(theme.ink_faint),
    )];

    let left_w: usize = left.iter().map(Span::width).sum();
    let right_w: usize = right.iter().map(Span::width).sum();
    let bar_w = (width as usize).saturating_sub(left_w + right_w);
    if bar_w == 0 {
        return justified(left, right, width);
    }

    // The elapsed portion of the scrubber, in the accent; the rest is track.
    let played = if dur_ms == 0 {
        0
    } else {
        ((app.pos_ms.min(dur_ms) as f64 / dur_ms as f64) * bar_w as f64).round() as usize
    }
    .min(bar_w);

    let mut spans = left;
    spans.push(Span::styled(
        "━".repeat(played),
        theme.on_console(theme.accent),
    ));
    spans.push(Span::styled(
        "─".repeat(bar_w - played),
        theme.on_console(theme.scrubber_track),
    ));
    spans.extend(right);
    Line::from(spans)
}

/// `FLAC · 44.1 kHz · 24-bit → Topping E30 · 44.1 kHz    ● BIT-PERFECT    eq flat    rg off`
///
/// The seal row. A transient status note takes the left half; the seal keeps the
/// right, and **the right group is laid out first** — the left is truncated to
/// whatever is left over. There is no arrangement in which a long device name or
/// a long error message pushes the seal off the row.
///
/// # The seal is derived, never assumed
///
/// Everything rendered here comes from [`SignalPath`], which came from
/// `eko_core::signal_path::derive` — a pure function differentially fuzzed
/// against the original TypeScript. This module:
///
/// * **never constructs a seal string.** `BIT-PERFECT`, `RESAMPLED`, `EQ`,
///   `VOLUME`, `REPLAYGAIN` are not literals anywhere in this file; the label is
///   `seal.seal_label`, whatever `derive` put in it.
/// * **never upgrades one.** The lamp is green *only* on `seal.pure`, and the
///   colour is chosen from `seal.flags`, which are `derive`'s own booleans.
/// * **never fills in a blank.** With `seal.active == false` — no engine, or an
///   engine that has not reported a whole stream yet — there is nothing to claim,
///   so the lamp stays hollow: `IDLE` when stopped, `UNVERIFIED` when a transport
///   is live but unverified. Neither is a claim, and neither can be mistaken for
///   the green lamp.
///
/// That last case is not hypothetical. An earlier draft of this row mapped every
/// non-`Stopped` state to a green `● BIT-PERFECT`; it was inert only for as long
/// as nothing could construct [`Playback::Playing`]. Three separate
/// false-`BIT-PERFECT` paths have already been found and fixed in the desktop
/// app. "Audio is coming out of the speakers" is not evidence of bit-perfection
/// — a resampled, EQ'd, attenuated stream sounds exactly as present.
fn signal_path(app: &App, seal: &SignalPath, width: u16) -> Line<'static> {
    let theme = &app.theme;

    // ── the seal, straight out of `derive` ───────────────────────────────
    let (lamp, label, lamp_style) = if !seal.active {
        match app.playback {
            // Nothing is loaded. A hollow lamp, not a green one.
            Playback::Stopped => ("○", "IDLE".to_string(), theme.on_console(theme.ink_faint)),
            // A live transport the engine has not described yet.
            Playback::Playing | Playback::Paused => (
                "○",
                "UNVERIFIED".to_string(),
                theme.on_console(theme.led_amber),
            ),
        }
    } else if seal.pure {
        // The only green lamp in the application.
        (
            "●",
            seal.seal_label.clone(),
            theme.on_console(theme.led_green),
        )
    } else {
        // A rate change is the loudest downgrade, so it gets the red LED; the
        // others are amber. Both are `derive`'s label verbatim either way — the
        // colour is emphasis, never the claim.
        let colour = if seal.flags.resampled || seal.flags.os_resampled {
            theme.led_red
        } else {
            theme.led_amber
        };
        ("●", seal.seal_label.clone(), theme.on_console(colour))
    };

    // ── the modifier chips ───────────────────────────────────────────────
    //
    // **Lowercase, and deliberately so.** They report the state of a stage in
    // the chain; the seal reports a *verdict* `derive` returned about the whole
    // of it. In the same case, `◆ EQ` beside a seal reading `EQ · VOLUME`
    // shouts the same word twice at two completely different weights, and the
    // one that matters loses.
    //
    // `eq` and `flat`/`on` are this module's own words for this module's own
    // booleans. `rg_label` is **not** — it is `derive`'s string, verbatim, in
    // whatever case `derive` chose (`Track · -7.3 dB`). Only the chip's own
    // label is lowered. Re-casing a derived value is how a rendering starts
    // disagreeing with the thing it renders.
    let eq_note = if seal.flags.eq_active {
        "    eq on"
    } else {
        "    eq flat"
    };
    let rg_note = if seal.flags.rg_active {
        format!("    rg {}", seal.rg_label)
    } else {
        "    rg off".to_string()
    };

    let right = vec![
        Span::styled(format!("{lamp} "), lamp_style),
        Span::styled(label, lamp_style.add_modifier(Modifier::BOLD)),
        Span::styled(eq_note, theme.on_console(theme.ink_faint)),
        Span::styled(rg_note, theme.on_console(theme.ink_faint)),
    ];

    // ── the chain, in whatever room the seal left ────────────────────────
    let (chain, chain_style) = match (&app.status, seal.active) {
        // A transient note outranks the chain, never the seal.
        (Some(note), _) => (note.clone(), theme.on_console(theme.led_amber)),
        (None, true) => (
            format!("{} → {}", seal.src, seal.output),
            theme.on_console(theme.ink_dim),
        ),
        (None, false) => ("— → —".to_string(), theme.on_console(theme.ink_faint)),
    };

    let right_w: usize = right.iter().map(Span::width).sum();
    let left_of_the_seal = (width as usize).saturating_sub(right_w);
    // The chain keeps a single column of gap when it fits as it is. When it has
    // to be cut, the cut leaves the four-column gap the rest of the row uses —
    // otherwise `…kHz ● VOLUME` reads as one phrase instead of two things.
    let room = if Span::raw(&chain).width() < left_of_the_seal {
        left_of_the_seal.saturating_sub(1)
    } else {
        left_of_the_seal.saturating_sub(4)
    };

    justified(
        vec![Span::styled(fit(&chain, room), chain_style)],
        right,
        width,
    )
}

/// Truncate `text` to at most `max` display columns, marking the cut with `…`.
///
/// Measured in columns rather than `char`s because a device name can be
/// double-width, and a chain that overran by two cells would take the seal with
/// it.
///
/// `pub(crate)` because [`crate::ui::device_panel`] truncates the *same* device
/// names in the *same* frame. A second copy of this arithmetic is a second chance
/// to get the `max == 0` case wrong, which is how a `…` ends up one column wider
/// than the room it was given.
pub(crate) fn fit(text: &str, max: usize) -> String {
    if Span::raw(text).width() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    // One column goes to the ellipsis.
    let budget = max - 1;
    let mut cut = 0;
    for (i, _) in text.char_indices().skip(1) {
        if Span::raw(&text[..i]).width() > budget {
            break;
        }
        cut = i;
    }
    format!("{}…", &text[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_leaves_anything_that_already_fits_alone() {
        assert_eq!(fit("FLAC · 44.1 kHz", 40), "FLAC · 44.1 kHz");
        assert_eq!(fit("exact", 5), "exact");
        assert_eq!(fit("", 0), "");
    }

    #[test]
    fn fit_never_exceeds_its_budget() {
        for max in 0..24usize {
            let out = fit("FLAC · 96 kHz · 24-bit → Topping E30 · 96 kHz", max);
            assert!(
                Span::raw(&out).width() <= max,
                "{max}: {out:?} is {} columns",
                Span::raw(&out).width()
            );
        }
    }

    #[test]
    fn fit_counts_columns_not_chars_so_wide_glyphs_cannot_overrun() {
        // Four CJK ideographs are eight columns, not four.
        let wide = "音楽再生";
        assert_eq!(Span::raw(wide).width(), 8);
        let out = fit(wide, 5);
        assert!(Span::raw(&out).width() <= 5, "{out:?}");
        assert!(out.ends_with('…'), "{out:?}");
    }

    #[test]
    fn the_clock_is_always_two_by_two_and_cannot_overflow_its_column() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(1_000), "00:01");
        assert_eq!(clock(291_400), "04:51");
        assert_eq!(clock(u64::MAX), "99:59");
    }
}
