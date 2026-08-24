//! The Deck: a source sidebar, a main list pane, and a persistent footer that
//! acts as an instrument panel.
//!
//! The layout mirrors the desktop app's `DeckShell` so the two applications
//! share a mental model, a vocabulary and a set of docs. The full mockup lives
//! in `docs/architecture/eko-cli.md` §5.1; this module reproduces it.
//!
//! The governing constraint: **the signal-path seal is never off-screen.** It
//! is the last line of the footer, and the footer is not scrollable, not
//! collapsible and not optional.

pub mod device_panel;
pub mod eq_panel;
pub mod footer;
pub mod help;
pub mod main_pane;
pub mod server_panel;
pub mod sidebar;
pub mod theme;
pub mod visualiser;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;

/// Below this the Deck cannot be drawn honestly, so it is not drawn at all.
pub const MIN_WIDTH: u16 = 80;
/// Below this the Deck cannot be drawn honestly, so it is not drawn at all.
pub const MIN_HEIGHT: u16 = 20;

/// Sidebar column width, in cells.
///
/// Sixteen rather than thirteen: with [`GUTTER`] taking a column at each edge a
/// thirteen-cell column left eleven for text, and `connecting` is ten of them —
/// the longest source name and the longest connection word were both one
/// resize away from being cut. Three more columns is the difference between a
/// column that fits its content and one that happens to.
pub const SIDEBAR_WIDTH: u16 = 16;
/// The footer is exactly four rows: title, artist, transport, signal path.
pub const FOOTER_HEIGHT: u16 = 4;

/// One column of clear space inside every pane border, on **both** sides.
///
/// The Deck has three panes and three panels, and before this was a constant
/// each of them spelled its own padding: a `Span::raw(" ")` at the head of a
/// row here, a `saturating_sub(1)` on a width there, an `inner.x + 1` in the
/// panels. Six copies of the number one, and nothing to fail if one of them
/// changed.
///
/// It is not only cosmetic. A pane that lays its content out in `area` and
/// leaves the padding to the strings has no *right* gutter at all when a row
/// overruns: the row is clipped by the pane rect, which ends at the border, so
/// an album title long enough runs flush into the frame. Content is laid out in
/// [`pane_body`] instead, so the clip happens a column early and the frame keeps
/// its air whatever the library is called.
pub const GUTTER: u16 = 1;

/// The rect a pane may draw in: `area`, less one [`GUTTER`] column at each edge.
///
/// Empty when `area` is too narrow to hold both gutters — which is the honest
/// answer, and one ratatui renders as nothing rather than as a clipped row.
#[must_use]
pub(crate) fn pane_body(area: Rect) -> Rect {
    Rect {
        x: area.x + GUTTER,
        y: area.y,
        width: area.width.saturating_sub(GUTTER * 2),
        height: area.height,
    }
}

/// Where the cover art will be drawn on a terminal of `area`, or `None` when
/// the Deck will not be drawn at all.
///
/// # Why the fold needs this and cannot wait for a frame
///
/// Two things outside the render path need the art's rect *before* anything is
/// drawn. [`App`] keys its cover cache on the grid size, so a resize has to
/// invalidate it without waiting for a frame to notice; and
/// [`crate::art::Painter`] writes kitty and iTerm2 sequences to the terminal
/// after the frame is flushed, positioned by absolute cursor address, so it
/// needs coordinates ratatui's buffer will never tell it.
///
/// It reproduces [`render_deck`]'s layout arithmetic, which is a duplication and
/// therefore a drift risk — so a test renders real frames at several sizes and
/// asserts the placeholder occupies precisely these cells. Change the layout
/// without changing this and it fails.
#[must_use]
pub fn art_area(area: Rect) -> Option<Rect> {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return None;
    }
    // The outer border takes one cell on every side; the footer is the last
    // FOOTER_HEIGHT rows of what is left, under a one-row rule.
    let footer = Rect {
        x: area.x + 1,
        y: area.y + area.height - 1 - FOOTER_HEIGHT,
        width: area.width - 2,
        height: FOOTER_HEIGHT,
    };
    let rect = footer::art_rect(footer);
    (!rect.is_empty()).then_some(rect)
}

/// Render one frame.
///
/// Pure in everything that matters: it reads `app` and writes the buffer, so
/// `TestBackend` can render it at any size without a terminal.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area, app);
        return;
    }
    render_deck(frame, area, app);
}

fn render_deck(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let rule = Style::default().fg(theme.rule);

    // `┌─ EKO ────… ─ context ┐` — the leading rule glyph is part of the title
    // so the corner does not sit flush against the mark.
    let block = Block::bordered()
        .border_style(rule)
        .title(Line::from(vec![
            Span::styled("─ ", rule),
            Span::styled("EKO", theme.accent_strong()),
            Span::raw(" "),
        ]))
        .title(
            Line::from(format!(" {} ", app.context))
                .style(Style::default().fg(theme.ink_faint))
                .right_aligned(),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [body, rule_row, footer_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .areas(inner);

    // **The visualiser is a view, not an overlay.** It replaces the body — the
    // sidebar and the main pane are not drawn at all while it is up — and
    // `footer_area` below is untouched by it, so the seal is on exactly the row
    // it is on in every other state. The separator column and the rule row
    // below are drawn either way: the rule joins the outer border and belongs to
    // the frame, and the separator is skipped because there is nothing to
    // separate.
    let visualiser = app.visualiser;
    let [sidebar_area, separator, main_area] = Layout::horizontal([
        Constraint::Length(SIDEBAR_WIDTH),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(body);

    if visualiser {
        visualiser::render(frame, body, app);
    } else {
        sidebar::render(frame, sidebar_area, app);
        main_pane::render(frame, main_area, app);
    }
    // Over the main pane, and only ever the main pane. `footer_area` is drawn
    // after it and is not in its rect either way — the seal is never off-screen,
    // and no overlay in this application is allowed to be the exception.
    //
    // Only one of these can be open — the fold closes the others when one opens
    // (see `App::open_devices`) — but the order is fixed anyway, so a state that
    // somehow held two would draw deterministically rather than half of each.
    if app.eq_open {
        eq_panel::render(frame, main_area, app);
    }
    if app.device_open {
        device_panel::render(frame, main_area, app);
    }
    // Over the main pane like the other two, and mutually exclusive with them by
    // the same rule — `App::open_server_row` closes whatever was open.
    if app.add.is_some() {
        server_panel::render(frame, main_area, app);
    }
    footer::render(frame, footer_area, app);

    // The sidebar rule and the footer rule are drawn by hand rather than with
    // nested blocks: they have to join the outer border with `├ ┴ ┤`, and a
    // block cannot reach outside its own area to do that.
    let buf = frame.buffer_mut();
    if !visualiser {
        for y in body.top()..body.bottom() {
            if let Some(cell) = buf.cell_mut((separator.x, y)) {
                cell.set_symbol("│").set_style(rule);
            }
        }
    }
    for x in area.left()..area.right() {
        if let Some(cell) = buf.cell_mut((x, rule_row.y)) {
            cell.set_symbol("─").set_style(rule);
        }
    }
    // `┴` is where the sidebar's rule meets this one. With the visualiser up
    // there is no sidebar rule, so there is no join to draw — a `┴` under
    // nothing is a frame describing a pane that is not there.
    for (x, symbol) in [
        (area.left(), "├"),
        (separator.x, if visualiser { "─" } else { "┴" }),
        (area.right().saturating_sub(1), "┤"),
    ] {
        if let Some(cell) = buf.cell_mut((x, rule_row.y)) {
            cell.set_symbol(symbol).set_style(rule);
        }
    }

    // **Last, and after the rules.** The help overlay covers the whole body, and
    // the sidebar's vertical rule above is painted straight into the buffer over
    // `body.top()..body.bottom()` — so an overlay drawn before it comes back with
    // a `│` down the middle of the keymap. The panels above are over `main_area`
    // only and so never meet that column; this one does, which is why it is here
    // rather than beside them.
    //
    // `body`, never `area`: the footer and its rule are outside it, so the seal is
    // in no rect this call can reach. See [`help`].
    if app.help_open {
        help::render(frame, body, app);
    }
}

/// The below-minimum card.
///
/// A garbled grid reads as a broken program; a card reads as a terminal that is
/// too small. It also has to survive being drawn into almost nothing, so every
/// dimension is clamped and every line is optional.
fn render_too_small(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    if area.is_empty() {
        return;
    }
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(Span::styled("EKO", theme.accent_strong())).centered(),
        Line::default(),
        Line::from(format!("needs {MIN_WIDTH} × {MIN_HEIGHT}"))
            .style(Style::default().fg(theme.ink))
            .centered(),
        Line::from(format!("this terminal is {} × {}", area.width, area.height))
            .style(Style::default().fg(theme.ink_faint))
            .centered(),
        Line::default(),
        Line::from("resize, or q to quit")
            .style(Style::default().fg(theme.ink_faint))
            .centered(),
    ];

    // Card width: wide enough for the longest line, never wider than the
    // terminal, and never so tall it clips its own border away.
    let widest = lines.iter().map(Line::width).max().unwrap_or(0) as u16;
    let card_w = (widest + 6).clamp(1, area.width);
    let card_h = ((lines.len() as u16) + 2).min(area.height);
    let card = Rect {
        x: area.x + (area.width - card_w) / 2,
        y: area.y + (area.height - card_h) / 2,
        width: card_w,
        height: card_h,
    };

    let block = Block::bordered().border_style(Style::default().fg(theme.rule));
    let text_area = block.inner(card);
    frame.render_widget(block, card);
    // Below about five rows there is no room for the blank spacer lines, so
    // drop them rather than clipping the message.
    let lines: Vec<Line> = if text_area.height < lines.len() as u16 {
        lines.into_iter().filter(|l| l.width() > 0).collect()
    } else {
        lines
    };
    frame.render_widget(Paragraph::new(lines), text_area);
}

/// Lay a left group and a right group on one row, with the gap between them.
///
/// Used wherever the mockup has something pinned to each edge — the transport
/// row, the signal-path row, the now-playing title against the spectrum.
/// Returns just the left group if there is not room for both, because the left
/// group is always the more important one.
pub(crate) fn justified<'a>(left: Vec<Span<'a>>, right: Vec<Span<'a>>, width: u16) -> Line<'a> {
    let left_w: usize = left.iter().map(Span::width).sum();
    let right_w: usize = right.iter().map(Span::width).sum();
    let width = width as usize;
    if left_w + right_w + 1 > width {
        return Line::from(left);
    }
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(width - left_w - right_w)));
    spans.extend(right);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, NowPlaying, Playback, ScanState, Source, View};
    use crate::config::{Config, MusicFolder};
    use crate::library::{Album, Library, Track};
    use crate::ui::theme::{Accent, ColorDepth, Theme};
    use eko_core::signal_path::StreamInfo;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use ratatui::Terminal;
    use std::path::PathBuf;

    /// A Deck with nowhere to look, stated rather than resolved: a fixture that
    /// asked the machine where its music lives would render one frame on a
    /// laptop with a `~/Music` and a different one in CI.
    fn app() -> App {
        with_folder(MusicFolder::Unset { probed: None })
    }

    fn with_folder(folder: MusicFolder) -> App {
        App::new(
            &Config::default(),
            Theme::new(Accent::Orange, ColorDepth::TrueColor),
            folder,
        )
    }

    fn track(title: &str, n: u32, secs: f64) -> Track {
        Track {
            // Unopenable on purpose: `Engine::play` fails in `open_source`,
            // before it can reach an output device, so no test here touches
            // CoreAudio.
            path: format!("/eko-cli-test/{title}.flac"),
            title: title.to_string(),
            artist: "Aphex Twin".to_string(),
            track_no: Some(n),
            duration: secs,
        }
    }

    /// A scanned library, so the panes have something real to draw.
    fn scanned() -> App {
        let mut a = app();
        a.library = Library {
            root: Some(PathBuf::from("/Users/rod/Music")),
            albums: vec![
                Album {
                    id: "Aphex Twin Selected Ambient Works 85-92".into(),
                    name: "Selected Ambient Works 85-92".into(),
                    artist: "Aphex Twin".into(),
                    tracks: vec![
                        track("Xtal", 1, 291.0),
                        track("Tha", 2, 549.0),
                        track("Pulsewidth", 3, 232.0),
                    ],
                },
                Album {
                    id: "Boards of Canada Music Has the Right to Children".into(),
                    name: "Music Has the Right to Children".into(),
                    artist: "Boards of Canada".into(),
                    tracks: vec![track("Wildlife Analysis", 1, 78.0)],
                },
            ],
        };
        a.scan = ScanState::Ready;
        a.context = "Music · 2 albums · 4 tracks".into();
        a.sources[0].badge = Some("2".into());
        a
    }

    /// The buffer as plain text, one string per row, trailing blanks kept so
    /// column alignment is part of what the snapshot pins.
    fn render(width: u16, height: u16, app: &App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        lines_of(terminal.backend().buffer())
    }

    /// The seal's lamp glyph **and its colour**, read off the rendered buffer.
    ///
    /// [`render`] throws styles away, and the active lamp is `●` for a pure seal
    /// and for an impure one alike — so on screen the **only** thing separating
    /// `● BIT-PERFECT` from `● VOLUME` at a glance is the colour of that cell. A
    /// text-only assertion cannot see it: a build that painted every active lamp
    /// green would pass every other test in this file.
    fn lamp(width: u16, height: u16, app: &App) -> (String, Color) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        // The seal row is the last content row, immediately above the border.
        let y = height - 2;
        for x in 0..width {
            let cell = &buf[(x, y)];
            if matches!(cell.symbol(), "●" | "○") {
                return (cell.symbol().to_string(), cell.fg);
            }
        }
        panic!("no lamp on the seal row at {width}×{height}");
    }

    /// The frame row the main pane's first list row lands on.
    ///
    /// Border, heading, summary, column headers, rule. Named because a dozen
    /// assertions below index it, and because it moved by one when the headers
    /// arrived — a magic `4` in twelve places is twelve edits, and one of them
    /// gets missed.
    const FIRST_LIST_ROW: usize = 5;

    /// The album column of a 100×30 frame row.
    ///
    /// The frame is border · [`SIDEBAR_WIDTH`] · rule · [`GUTTER`], then the
    /// pane body, whose left half is the album column. Its right edge is what a
    /// track count is right-aligned to now.
    fn album_column(row: &str) -> String {
        let x = 1 + usize::from(SIDEBAR_WIDTH) + 1 + usize::from(GUTTER);
        // 100 wide: 98 inside the border, less the sidebar, its rule and the two
        // gutters, is a 79-column body, whose left half is 38.
        row.chars().skip(x).take(38).collect::<String>()
    }

    /// The foreground of the cell where `needle` starts on row `y`.
    ///
    /// [`render`] throws styles away, and some of what the footer says is *only*
    /// in the colour: an amber sleep timer and a dim queue position are two
    /// different messages sharing one slot and one shape.
    fn colour_of(app: &App, width: u16, height: u16, y: u16, needle: &str) -> Option<Color> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        let row: String = (0..width).map(|x| buf[(x, y)].symbol()).collect();
        let at = row.find(needle)?;
        Some(buf[(row[..at].chars().count() as u16, y)].fg)
    }

    fn lines_of(buf: &Buffer) -> Vec<String> {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// Snapshot of a cold Deck at 100×30 — nowhere to look at all.
    ///
    /// Regenerated for the nav fix. It used to print the keyboard here and
    /// nothing else, which is precisely how an unconfigured Deck came to look
    /// like a broken one: fifteen bindings on screen, every one of them inert,
    /// and not a word about why `j` and `k` did nothing. The pane now names the
    /// file to create and the line to put in it, and the keymap steps aside
    /// because at this height it no longer fits underneath.
    ///
    /// Regenerated again for the sources fix: the five unbuilt source rows
    /// became a `NOT YET` note, `Local` lost a caret it had nothing to open, and
    /// the bottom row of the column gained the `tab` hint.
    ///
    /// Regenerated once more for the geometry. [`SIDEBAR_WIDTH`] went from
    /// thirteen to sixteen, so the main pane's text starts three columns further
    /// right and its right-aligned column is where it was; and
    /// [`footer::ART_WIDTH`] went from six to eight, so the cover block is two
    /// wider, the footer's text column two narrower, and the scrubber two
    /// shorter. Nothing here changed *what* is on screen. The footer's
    /// background change is invisible in a text dump, which carries no colour —
    /// see [`no_cell_in_the_deck_paints_its_own_background`] for the assertion
    /// that can see it.
    #[test]
    fn deck_at_100x30_with_no_music_folder() {
        let expected = vec![
            "┌─ EKO ────────────────────────────────────────────────────────────────────────── no source · idle ┐",
            "│ SOURCES        │ LOCAL                                                                           │",
            "│                │ no music folder · set music_folder in ~/.config/eko/config.toml                 │",
            "│ ○ Local        │                                                                                 │",
            "│ + Add server   │ No music folder.                                                                │",
            "│ ○ Queue        │                                                                                 │",
            "│                │ Put this in ~/.config/eko/config.toml:                                          │",
            "│                │                                                                                 │",
            "│                │     music_folder = \"/path/to/music\"                                             │",
            "│                │                                                                                 │",
            "│                │ Then press r to scan it.                                                        │",
            "│                │                                                                                 │",
            "│                │       q  quit                         s  spectrum on / off                      │",
            "│                │     tab  sources ⇄ library            z  now playing                            │",
            "│                │   j / ↓  down                         r  rescan · reconnect                     │",
            "│                │   k / ↑  up                           /  search the server                      │",
            "│                │   enter  open album · play track      d  output device                          │",
            "│                │     esc  back                         t  sleep timer                            │",
            "│                │   space  play / pause                 ?  keys · this list                       │",
            "│                │       n  next track                   e  EQ panel                               │",
            "│                │       p  previous track               E  EQ on / off                            │",
            "│                │       a  add to queue                 h  EQ band left                           │",
            "│                │       x  remove from queue            l  EQ band right                          │",
            "│ tab sources    │          …                                                                      │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ⏹ Nothing playing                                     ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │",
            "│ ░░░░░░░░  —                                                                                      │",
            "│ ░░░░░░░░  00:00 ────────────────────────────────────────────────────────────────────────── 00:00 │",
            "│ ░░░░░░░░  — → —                                                      ○ IDLE    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &app()), expected);
    }

    // ── the empty states ─────────────────────────────────────────────────

    /// **The regression this whole fix exists to prevent.** An empty pane must
    /// never be *only* a keymap: that is a screen which looks fully alive and
    /// silently ignores every key, and it is what the owner reported as "the
    /// nav doesn't work".
    #[test]
    fn an_empty_pane_explains_itself_rather_than_reciting_the_keyboard() {
        let cases: Vec<(&str, App)> = vec![
            ("nowhere to look", app()),
            ("no ~/Music to fall back to", unset_with_probe()),
            ("the configured folder is gone", missing_folder()),
            ("scanned, nothing in it", empty_folder()),
        ];
        for (what, a) in cases {
            assert_eq!(a.row_count(), 0, "{what}: the fixture must have no rows");
            let rows = render(100, 30, &a);
            // The first line under the chrome is a sentence, not a binding.
            let first = rows[4].trim_matches(|c| c == '│' || c == ' ');
            assert!(
                first.ends_with('.'),
                "{what}: the pane opens with {first:?} instead of a statement"
            );
            assert!(
                !rows[4].contains("quit"),
                "{what}: the keymap is still the first thing on screen"
            );
        }
    }

    /// A `~/Music` that is not there is named, so "unconfigured" cannot be
    /// mistaken for "empty folder".
    fn unset_with_probe() -> App {
        with_folder(MusicFolder::Unset {
            probed: Some(PathBuf::from("/Users/rod/Music")),
        })
    }

    #[test]
    fn a_missing_platform_default_is_named_rather_than_implied() {
        let rows = render(100, 30, &unset_with_probe());
        assert!(rows[4].contains("No music folder."), "{}", rows[4]);
        assert!(
            rows[6].contains("There is no /Users/rod/Music to fall back to."),
            "{}",
            rows[6]
        );
        assert!(
            rows[7].contains("Put this in ~/.config/eko/config.toml:"),
            "{}",
            rows[7]
        );
        assert!(
            rows[9].contains(r#"music_folder = "/path/to/music""#),
            "{}",
            rows[9]
        );
    }

    /// The Deck whose configured `music_folder` has gone away.
    fn missing_folder() -> App {
        let mut a = with_folder(MusicFolder::Configured(PathBuf::from(
            "/Volumes/Archive/Music",
        )));
        a.scan = ScanState::Missing(PathBuf::from("/Volumes/Archive/Music"));
        a
    }

    /// Snapshot: a configured folder that is not there. The path is on screen
    /// twice — once as the fact, once as the thing to fix.
    ///
    /// **The keymap used to follow it here and no longer does.** [`empty_state`]
    /// prints the hints only when the whole table fits under the message, and
    /// Task 4's two new bindings (`a`, `x`) took the count from fifteen to
    /// seventeen — one more than the fifteen rows this four-line message leaves
    /// spare. The rule is unchanged, and is the one already documented there:
    /// when both do not fit, the message wins, because the keys are no use to
    /// someone with nothing to move a cursor over. A shorter message still gets
    /// them — see [`a_scan_in_flight_names_the_folder_it_is_reading`].
    ///
    /// [`empty_state`]: crate::ui::main_pane
    #[test]
    fn deck_at_100x30_with_a_music_folder_that_does_not_exist() {
        let expected = vec![
            "┌─ EKO ────────────────────────────────────────────────────────────────────────── no source · idle ┐",
            "│ SOURCES        │ LOCAL                                                                           │",
            "│                │ music_folder is missing · /Volumes/Archive/Music                                │",
            "│ ○ Local        │                                                                                 │",
            "│ + Add server   │ /Volumes/Archive/Music does not exist.                                          │",
            "│ ○ Queue        │                                                                                 │",
            "│                │ That path is music_folder in ~/.config/eko/config.toml.                         │",
            "│                │ Fix it there, or reconnect the drive, then press r.                             │",
            "│                │                                                                                 │",
            "│                │       q  quit                         s  spectrum on / off                      │",
            "│                │     tab  sources ⇄ library            z  now playing                            │",
            "│                │   j / ↓  down                         r  rescan · reconnect                     │",
            "│                │   k / ↑  up                           /  search the server                      │",
            "│                │   enter  open album · play track      d  output device                          │",
            "│                │     esc  back                         t  sleep timer                            │",
            "│                │   space  play / pause                 ?  keys · this list                       │",
            "│                │       n  next track                   e  EQ panel                               │",
            "│                │       p  previous track               E  EQ on / off                            │",
            "│                │       a  add to queue                 h  EQ band left                           │",
            "│                │       x  remove from queue            l  EQ band right                          │",
            "│                │   [ / ←  seek back 5s                 ,  EQ preset back                         │",
            "│                │   ] / →  seek forward 5s              .  EQ preset forward                      │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ⏹ Nothing playing                                     ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │",
            "│ ░░░░░░░░  —                                                                                      │",
            "│ ░░░░░░░░  00:00 ────────────────────────────────────────────────────────────────────────── 00:00 │",
            "│ ░░░░░░░░  — → —                                                      ○ IDLE    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &missing_folder()), expected);
    }

    /// A real folder, scanned, with nothing readable in it.
    fn empty_folder() -> App {
        let mut a = with_folder(MusicFolder::Platform(PathBuf::from("/Users/rod/Music")));
        a.scan = ScanState::Ready;
        a.library = Library {
            root: Some(PathBuf::from("/Users/rod/Music")),
            albums: Vec::new(),
        };
        a
    }

    /// An empty folder names the folder *and* the formats — the difference
    /// between "this app is broken" and "ah, those are all .dsf".
    #[test]
    fn an_empty_folder_names_the_path_and_the_formats_it_looked_for() {
        let rows = render(100, 30, &empty_folder());
        assert!(rows[2].contains("no audio files found"), "{}", rows[2]);
        assert!(
            rows[4].contains("No audio in /Users/rod/Music."),
            "{}",
            rows[4]
        );
        assert!(
            rows[6].contains("EKO reads these, up to 8 folders deep:"),
            "{}",
            rows[6]
        );
        // Every extension the walk accepts is on screen, from the same
        // constant the walk reads — the sentence cannot drift from the code.
        for ext in crate::library::AUDIO_EXTS {
            assert!(
                rows[7].contains(&format!(".{ext}")),
                "{ext} missing: {}",
                rows[7]
            );
        }
        assert!(
            rows[10].contains("~/.config/eko/config.toml, then press r to rescan."),
            "{}",
            rows[10]
        );
    }

    /// A scan in flight says what it is reading, and keeps the keymap — there
    /// is nothing to fix, so there is nothing to crowd out.
    #[test]
    fn a_scan_in_flight_names_the_folder_it_is_reading() {
        let mut a = with_folder(MusicFolder::Platform(PathBuf::from("/Users/rod/Music")));
        a.scan = ScanState::Discovering { found: 7 };
        let rows = render(100, 30, &a);
        assert!(rows[2].contains("scanning · 7 files found"), "{}", rows[2]);
        assert!(rows[4].contains("Reading /Users/rod/Music."), "{}", rows[4]);
        assert!(
            rows[6].contains("quit"),
            "the keymap should still fit: {}",
            rows[6]
        );
    }

    /// A failed scan does not print the scanner's sentence twice: the summary
    /// row carries it, the body carries the way out.
    #[test]
    fn a_failed_scan_says_how_to_retry_without_repeating_itself() {
        let mut a = app();
        a.scan = ScanState::Failed("cancelled".into());
        let rows = render(100, 30, &a);
        assert!(rows[2].contains("cancelled"), "{}", rows[2]);
        assert!(
            rows[4].contains("The scan did not finish. Press r to try again."),
            "{}",
            rows[4]
        );
        assert_eq!(
            rows.iter().filter(|r| r.contains("cancelled")).count(),
            1,
            "the scanner's message is on screen twice"
        );
    }

    /// Every empty-state sentence survives the **minimum** supported terminal.
    ///
    /// At 80×20 the main pane's body is 59 columns and ten rows — four fewer
    /// than before [`SIDEBAR_WIDTH`] grew and [`GUTTER`] became real, which is
    /// why the missing-folder message lost its trailing `to rescan`. A message
    /// that clips there is a message the user has to guess the end of, which is
    /// most of the way back to the problem this fix exists to solve.
    #[test]
    fn the_empty_states_fit_the_smallest_supported_terminal() {
        let cases: Vec<(App, Vec<&str>)> = vec![
            (
                unset_with_probe(),
                vec![
                    "There is no /Users/rod/Music to fall back to.",
                    "Put this in ~/.config/eko/config.toml:",
                    r#"music_folder = "/path/to/music""#,
                    "Then press r to scan it.",
                ],
            ),
            (
                missing_folder(),
                vec![
                    "/Volumes/Archive/Music does not exist.",
                    "That path is music_folder in ~/.config/eko/config.toml.",
                    "Fix it there, or reconnect the drive, then press r.",
                ],
            ),
            (
                empty_folder(),
                vec![
                    "No audio in /Users/rod/Music.",
                    "EKO reads these, up to 8 folders deep:",
                    "Add some there, or point music_folder somewhere else in",
                    "~/.config/eko/config.toml, then press r to rescan.",
                ],
            ),
        ];
        for (a, sentences) in cases {
            let rows = render(MIN_WIDTH, MIN_HEIGHT, &a);
            for sentence in sentences {
                assert!(
                    rows.iter().any(|r| r.contains(sentence)),
                    "clipped at {MIN_WIDTH}×{MIN_HEIGHT}: {sentence:?}\n{}",
                    rows.join("\n")
                );
            }
        }
    }

    /// **The keymap shrinks rather than vanishing — and says when it was cut.**
    ///
    /// The rule it replaces was all-or-nothing: the whole table under the
    /// message, or none of it. Under that rule *adding a binding* made the help
    /// less likely to appear, and it had already stopped appearing at 100×30 —
    /// which the owner reported. Two columns halve the rows the table needs, so
    /// the full keyboard was back at 100×30 with a seven-line message above it;
    /// at the 80×20 minimum, where two rows is all there is, two rows is what it
    /// takes, and the second one is a `…` rather than a silent half-truth.
    ///
    /// ## What Phase 1b-ii-b changed, and what it deliberately did not
    ///
    /// At twenty-four bindings the whole table fitted under this seven-line
    /// message with **nothing spare** — twelve rows into twelve. So *any*
    /// twenty-fifth binding cuts it, whatever it is: the arithmetic had already
    /// run out, and no amount of care about which key was added next would have
    /// changed that. Task 4 added two.
    ///
    /// The property asserted here is therefore the one that was always the point
    /// and is unchanged: **the message wins, the keymap still appears, and a cut
    /// says so.** A pane with nothing in it is not a keyboard reference, and it
    /// should not be sized as though it were.
    ///
    /// It is still asserted for the empty state at a terminal tall enough to hold
    /// the table, below, so "shrinks to fit" is not quietly allowed to become
    /// "always cut". (Task 5 of this phase gives the complete keyboard a screen
    /// whose only job is the table, which is where that guarantee belongs.)
    #[test]
    fn the_keymap_shrinks_to_fit_instead_of_disappearing() {
        let wide = render(100, 30, &app());
        // The message is still the first thing read.
        assert!(wide[4].contains("No music folder."), "{}", wide[4]);
        // Cut, and it says so on its last row rather than reading as the whole
        // keyboard. This is the honest half of "shrinks to fit".
        assert!(
            wide.iter().any(|r| r.contains('…')),
            "the keymap was cut at 100×30 without saying so:\n{}",
            wide.join("\n")
        );
        // Given the room, the table is complete — every binding, from the table,
        // including whichever one was added last.
        let tall = render(100, 50, &app());
        for binding in crate::keys::BINDINGS {
            assert!(
                tall.iter().any(|r| r.contains(binding.description)),
                "{:?} is missing from the two-column keymap:\n{}",
                binding.description,
                tall.join("\n")
            );
        }
        assert!(
            !tall.iter().any(|r| r.contains('…')),
            "the whole table fitted and the keymap still claimed to be cut:\n{}",
            tall.join("\n")
        );
        // Two columns, column-major: the first binding heads the left column and
        // the one halfway down the table heads the right, on the same row. Both
        // descriptions are read off `BINDINGS` rather than written here, because
        // this assertion used to name two of them — and naming them meant that
        // adding a binding, which moves the right column's head, broke a test
        // about *layout* for a reason that had nothing to do with layout.
        let rows = crate::keys::BINDINGS.len().div_ceil(2);
        let (left, right) = (
            crate::keys::BINDINGS[0].description,
            crate::keys::BINDINGS[rows].description,
        );
        assert!(
            wide.iter().any(|r| r.contains(left) && r.contains(right)),
            "the hints were not laid out in two columns ({left:?} beside {right:?}):\n{}",
            wide.join("\n")
        );

        // The minimum terminal: the message still wins, the keymap still shows.
        let tight = render(MIN_WIDTH, MIN_HEIGHT, &app());
        assert!(tight[4].contains("No music folder."), "{}", tight[4]);
        assert!(
            tight.iter().any(|r| r.contains("quit")),
            "the keymap vanished entirely at {MIN_WIDTH}×{MIN_HEIGHT}:\n{}",
            tight.join("\n")
        );
        assert!(
            !tight.iter().any(|r| r.contains("rescan · reconnect")),
            "the whole table was squeezed into two rows"
        );
        assert!(
            tight.iter().any(|r| r.contains('…')),
            "a keymap that was cut short did not say so:\n{}",
            tight.join("\n")
        );
    }

    /// Snapshot of the below-minimum card at 40×10.
    #[test]
    fn too_small_card_at_40x10() {
        let expected = vec![
            "                                        ",
            "     ┌────────────────────────────┐     ",
            "     │             EKO            │     ",
            "     │                            │     ",
            "     │        needs 80 × 20       │     ",
            "     │  this terminal is 40 × 10  │     ",
            "     │                            │     ",
            "     │    resize, or q to quit    │     ",
            "     └────────────────────────────┘     ",
            "                                        ",
        ];
        assert_eq!(render(40, 10, &app()), expected);
    }

    #[test]
    fn exactly_eighty_by_twenty_draws_the_deck_not_the_card() {
        let rows = render(MIN_WIDTH, MIN_HEIGHT, &app());
        assert!(rows[0].starts_with("┌─ EKO"), "{}", rows[0]);
        assert!(rows.iter().any(|r| r.contains("SOURCES")));
        assert!(rows.iter().any(|r| r.contains("IDLE")));
    }

    #[test]
    fn one_column_short_of_the_minimum_draws_the_card() {
        let rows = render(MIN_WIDTH - 1, MIN_HEIGHT, &app());
        assert!(rows.iter().any(|r| r.contains("needs 80 × 20")));
        let rows = render(MIN_WIDTH, MIN_HEIGHT - 1, &app());
        assert!(rows.iter().any(|r| r.contains("needs 80 × 20")));
    }

    #[test]
    fn the_seal_is_never_off_screen_at_any_usable_size() {
        for (w, h) in [(80, 20), (80, 60), (100, 30), (200, 24), (240, 80)] {
            let rows = render(w, h, &app());
            let last_content = &rows[rows.len() - 2];
            assert!(
                last_content.contains("IDLE"),
                "seal missing at {w}×{h}: {last_content}"
            );
        }
    }

    #[test]
    fn the_card_survives_absurdly_small_terminals() {
        // Not a supported size — just must not panic or index out of bounds.
        for (w, h) in [(1, 1), (2, 3), (10, 4), (20, 2), (40, 5), (79, 19)] {
            let rows = render(w, h, &app());
            assert_eq!(rows.len(), h as usize);
        }
    }

    #[test]
    fn the_footer_shows_the_spectrum_only_when_it_is_visible() {
        let mut a = app();
        a.playback = Playback::Playing;
        assert!(render(100, 30, &a)[25].contains('▁'));
        a.spectrum_visible = false;
        assert!(!render(100, 30, &a)[25].contains('▁'));
    }

    #[test]
    fn the_main_pane_heading_follows_the_sidebar_selection() {
        // One source, so one heading — and it is the source's own label, not a
        // constant. The fixture used to select row 1, `Navidrome`, which was
        // never a source this build has.
        let a = app();
        assert_eq!(a.selected_label(), "Local");
        assert!(render(100, 30, &a)[1].contains("LOCAL"));
    }

    // ── the password, and every frame it must not be in ──────────────────

    /// **The one thing in this feature that must not be got wrong.**
    ///
    /// A password is typed on a plain terminal, handed to the keychain, and
    /// dropped. It is never in [`App`], so it can never be in a frame — and this
    /// is the assertion that would fail if that ever stopped being true.
    ///
    /// It walks the whole flow with a sentinel, rendering after **every** step,
    /// and then does the thing a broken suspend would do: pushes the sentinel at
    /// the fold as raw keystrokes, as though the terminal had never been handed
    /// back and the reader thread had gone on delivering to the Deck. Even then
    /// nothing of it reaches the screen — because `submit_add_server` closes the
    /// panel *before* the prompt is requested, so there is no open text field for
    /// it to land in.
    ///
    /// The failure it is written against is concrete: add a fourth field to
    /// `AddServer`, or leave the panel open across the prompt, and this test goes
    /// red. Both were real options while this was being built.
    ///
    /// Its sibling lives in `server.rs` —
    /// `a_server_that_echoes_the_password_back_cannot_put_it_in_the_footer` —
    /// and covers the other direction: a message this client did not write.
    #[test]
    fn a_password_can_never_reach_a_rendered_frame() {
        use crate::app::{AppEvent, Field, Focus, Prompt, PromptOutcome};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        const SENTINEL: &str = "hunter2-never-render";

        let press = |code: KeyCode| {
            AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
                code,
                KeyModifiers::NONE,
            )))
        };
        let typing = |text: &str| -> Vec<AppEvent> {
            text.chars().map(|c| press(KeyCode::Char(c))).collect()
        };

        let mut a = app();
        // Every frame the flow can produce, collected as it goes.
        let mut frames: Vec<String> = vec![render(100, 30, &a).join("\n")];
        let step = |a: &mut App, event: AppEvent, frames: &mut Vec<String>| {
            a.handle(event);
            frames.push(render(100, 30, a).join("\n"));
        };

        // Open the panel from the sidebar's invitation row.
        a.focus = Focus::Sidebar;
        a.selected = 1;
        step(&mut a, press(KeyCode::Enter), &mut frames);
        assert!(a.add.is_some(), "enter on the invitation opened nothing");

        for event in typing("home") {
            step(&mut a, event, &mut frames);
        }
        step(&mut a, press(KeyCode::Enter), &mut frames);
        for event in typing("https://music.example.com") {
            step(&mut a, event, &mut frames);
        }
        step(&mut a, press(KeyCode::Enter), &mut frames);
        for event in typing("rod") {
            step(&mut a, event, &mut frames);
        }
        // Submit. The panel closes here, which is load-bearing rather than tidy.
        step(&mut a, press(KeyCode::Enter), &mut frames);
        assert!(
            a.add.is_none(),
            "the panel stayed open across the password prompt — a text field the \
             password could be typed into"
        );

        let prompt = a.take_prompt().expect("a prompt was asked for");
        assert!(matches!(prompt, Prompt::Add(_)));
        // What crosses the boundary: three config fields and no fourth.
        let server = prompt.server().clone();
        assert_eq!(server.name, "home");
        assert_eq!(server.username, "rod");
        assert!(
            !format!("{prompt:?}").contains(SENTINEL),
            "the request to prompt could carry a password"
        );

        // The password is typed — into `server::apply_key`, the one editor in
        // this crate that takes one, which is not reachable from `App`.
        let mut secret = String::new();
        for c in SENTINEL.chars() {
            let _ = crate::server::apply_key(
                &mut secret,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            );
        }
        assert_eq!(secret, SENTINEL, "the fixture typed the wrong thing");

        // **The broken-suspend case.** Push the same characters at the fold as
        // keystrokes, the way they would arrive if the Deck were still on screen
        // and still reading input.
        for event in typing(SENTINEL) {
            step(&mut a, event, &mut frames);
        }

        // And the only value that ever comes back from the prompt.
        step(
            &mut a,
            AppEvent::Tick, // a frame between, so the outcome is not the last word
            &mut frames,
        );
        a.finish_prompt(PromptOutcome::Stored {
            server,
            notes: vec!["dropped the trailing /".to_string()],
        });
        frames.push(render(100, 30, &a).join("\n"));

        for (i, frame) in frames.iter().enumerate() {
            assert!(
                !frame.contains(SENTINEL),
                "frame {i} of the add-server flow rendered the password:\n{frame}"
            );
            // …and no prefix of it either, which is what a field would show
            // mid-typing.
            assert!(
                !frame.contains("hunter2"),
                "frame {i} rendered part of the password:\n{frame}"
            );
        }
        // The state that survives holds none of it either.
        assert!(!format!("{:?}", a.add).contains("hunter"));
        assert!(!format!("{:?}", a.status).contains("hunter"));
        // Three fields, and the panel draws exactly them.
        assert_eq!(Field::ALL.len(), 3);
    }

    /// **An open panel is a drawn panel, whatever is typed into it.**
    ///
    /// The failure this is written against had no error message and no crash:
    /// the panel sized itself from `Line::width` — *columns* — while the field
    /// caps and the value window counted *characters*, so a name in a wide
    /// script made a value twice as wide as the layout had budgeted. The panel
    /// asked for a rectangle wider than the main pane, the guard against that
    /// `return`ed, and **nothing was drawn**. `App::add` stayed `Some`, and
    /// `add_key` goes on consuming every keystroke while it is — so the Deck
    /// looked frozen, with Esc, backspace and Ctrl-C the only ways out. One
    /// paste reached it, because `add_paste` capped in characters too.
    ///
    /// Both sizes are here because the reviewer reproduced it at both: 38 CJK
    /// characters typed killed it at 80×30, and a 60-character paste killed it
    /// at 100×30 as well.
    #[test]
    fn a_panel_full_of_wide_characters_is_still_drawn_and_still_fits() {
        use crate::app::{AppEvent, Focus};
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

        let open = || {
            let mut a = app();
            a.focus = Focus::Sidebar;
            a.selected = 1;
            a.handle(AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))));
            assert!(a.add.is_some(), "the panel did not open");
            a
        };

        // Typed, one wide character at a time, into the name field; and pasted,
        // in one go, into the URL field — the two ways it was reachable.
        let mut a = open();
        for c in "音".repeat(38).chars() {
            a.handle(AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            ))));
        }
        a.handle(AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        ))));
        a.handle(AppEvent::Input(Event::Paste("音楽".repeat(30))));

        for (w, h) in [(MIN_WIDTH, 30), (100, 30), (MIN_WIDTH, MIN_HEIGHT)] {
            let rows = render(w, h, &a);
            assert!(
                rows.iter().any(|r| r.contains("┌─ SERVER")),
                "the panel vanished at {w}×{h} — and `add` is still open, so the \
                 Deck is eating keystrokes with nothing on screen:\n{}",
                rows.join("\n")
            );
            // Drawn *inside* the frame: the outer border is cell-for-cell what
            // it is with no panel up, so nothing overran the edge.
            let plain = render(w, h, &app());
            for (y, row) in rows.iter().enumerate() {
                let bare = &plain[y];
                assert_eq!(
                    (row.chars().next(), row.chars().last()),
                    (bare.chars().next(), bare.chars().last()),
                    "row {y} at {w}×{h} overran the frame:\n{row}"
                );
            }
            // The seal is never covered — the rule every modal in this crate
            // follows, and a clipped panel must not become the exception.
            let seal_row = &rows[rows.len() - 2];
            assert!(
                seal_row.contains('●') || seal_row.contains('○'),
                "the panel covered the seal at {w}×{h}:\n{seal_row}"
            );
        }
    }

    /// **A duplicate name is refused on screen, not only in a struct.**
    ///
    /// The rule is `AddServer::validate`'s and the assertion for it is in
    /// `app.rs`; this is the half that says it reaches a frame. A guard that
    /// only set `AddServer::error` would be indistinguishable, from the user's
    /// side, from a submit key that had stopped working.
    #[test]
    fn a_duplicate_server_name_says_so_in_the_panel() {
        use crate::app::{AppEvent, Focus};
        use crate::server::ServerConfig;
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

        let mut config = Config::default();
        config.servers.push(ServerConfig {
            name: "home".into(),
            base_url: "https://music.example.com".into(),
            username: "rod".into(),
        });
        let mut a = App::new(
            &config,
            Theme::new(Accent::Orange, ColorDepth::TrueColor),
            MusicFolder::Unset { probed: None },
        );
        let press = |code| AppEvent::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));

        a.focus = Focus::Sidebar;
        a.selected = 1;
        a.handle(press(KeyCode::Enter));
        for text in ["home", "https://other.example.com", "rod"] {
            for c in text.chars() {
                a.handle(press(KeyCode::Char(c)));
            }
            a.handle(press(KeyCode::Enter));
        }

        let rows = render(MIN_WIDTH, 30, &a);
        assert!(
            rows.iter().any(|r| r.contains("already configured")),
            "the duplicate name was refused silently:\n{}",
            rows.join("\n")
        );
        // Whole, not ellipsised: the fix is at the end of the sentence.
        assert!(
            rows.iter().any(|r| r.contains("pick another name")),
            "the message was cut off before the part that says what to do:\n{}",
            rows.join("\n")
        );
    }

    // ── the sources that do not exist ────────────────────────────────────

    /// **The sidebar cannot name a source the product does not have.**
    ///
    /// It shipped `Local`, `Navidrome`, `Albums`, `Artists`, `Lists` and
    /// `Queue`. Only `Local` did anything; `Enter` on Navidrome toggled a
    /// disclosure caret onto four rows that were equally inert. The owner's
    /// report was the predictable one — *"how do I navigate between Local and
    /// Navidrome?"* — and the answer was that you could not, because there was
    /// nothing to navigate to.
    ///
    /// The first fix moved the dead names under a dim `NOT YET` heading. The
    /// second, this one, removed them: the heading was down to `Artists` and
    /// `Lists`, and a nav item that does nothing is worse than an absent one
    /// however dimly it is drawn. So the column is now exactly `App::sources`
    /// and nothing else — every visible name is a row, and every row works.
    #[test]
    fn the_sources_column_names_nothing_that_is_not_a_source() {
        let a = scanned();
        let rows = render(100, 30, &a);
        // The sources column only: the cells up to and including the pane rule.
        // Sliced by char, because the border glyphs are multi-byte.
        let sidebar: Vec<String> = rows
            .iter()
            .map(|r| {
                r.chars()
                    .take(usize::from(SIDEBAR_WIDTH) + 1)
                    .collect::<String>()
            })
            .collect();

        for gone in ["NOT YET", "Artists", "Lists"] {
            assert!(
                !sidebar.iter().any(|r| r.contains(gone)),
                "{gone:?} is still in the sources column"
            );
        }

        // Every word in the column is the heading, a source's label, or the
        // `tab` hint pinned to the bottom.
        let known: Vec<String> = [
            "SOURCES".to_string(),
            "tab sources".to_string(),
            "tab library".to_string(),
        ]
        .into_iter()
        .chain(a.sources.iter().map(|s| s.label.clone()))
        .collect();
        // The body only: the footer's rows start with the same `│` and are not
        // the sources column.
        for row in sidebar
            .iter()
            .take_while(|r| !r.starts_with('├'))
            .filter(|r| r.starts_with('│'))
        {
            let text = row
                .trim_matches(|c| c == '│' || c == ' ')
                // `+` joins the two lamps as a *glyph* rather than a word: it is
                // the marker on the `Add server` invitation, whose label is a
                // row in `App::sources` like every other name in this column.
                .trim_start_matches(['●', '○', '+', ' '])
                .trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ')
                .to_string();
            assert!(
                text.is_empty() || known.contains(&text),
                "{text:?} is in the sources column and is not a source"
            );
        }

        // And the working source carries a lit lamp and its real album count.
        let local = sidebar
            .iter()
            .position(|r| r.contains("Local"))
            .expect("no Local row");
        assert!(sidebar[local].contains('●'), "{}", sidebar[local]);
        assert!(sidebar[local].contains('2'), "{}", sidebar[local]);
    }

    /// Every source the cursor can reach has a pane behind it — which is the
    /// invariant `NOT YET` used to protect from the other side.
    #[test]
    fn no_key_can_move_the_cursor_onto_a_source_with_no_pane() {
        use crate::app::Focus;

        let mut a = scanned();
        a.focus = Focus::Sidebar;
        for _ in 0..8 {
            a.act(crate::keys::Action::Down);
            a.act(crate::keys::Action::Open);
            // The pane shows the source the cursor is actually on — and every
            // source the cursor can reach is one that has a pane.
            let heading = render(100, 30, &a)[1].clone();
            assert!(
                heading.contains(&a.selected_label().to_uppercase()),
                "{heading:?} is not {:?}'s pane",
                a.selected_label()
            );
        }
    }

    // ── the server's connection ──────────────────────────────────────────

    /// A Deck with a `[[servers]]` entry, in the given connection state.
    fn connecting_to(conn: crate::app::ConnState) -> App {
        let mut config = Config::default();
        config.servers.push(crate::server::ServerConfig {
            name: "home".into(),
            base_url: "https://music.example.com".into(),
            username: "rod".into(),
        });
        let mut a = App::new(
            &config,
            Theme::new(Accent::Orange, ColorDepth::TrueColor),
            MusicFolder::Unset { probed: None },
        );
        a.conn = conn;
        a
    }

    /// The lamp beside a named source row, **and its colour**.
    ///
    /// The colour is the whole point, as it is on the seal: `●` green and `●`
    /// amber are the same glyph in a text dump and two different claims on
    /// screen. See [`lamp`] for the seal's own version of this.
    fn source_lamp(a: &App, label: &str) -> Option<(String, Color)> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, a)).unwrap();
        let buf = terminal.backend().buffer();
        for y in 0..30u16 {
            let row: String = (0..=SIDEBAR_WIDTH).map(|x| buf[(x, y)].symbol()).collect();
            if !row.contains(label) {
                continue;
            }
            let x = row.chars().position(|c| matches!(c, '●' | '○'))? as u16;
            return Some((buf[(x, y)].symbol().to_string(), buf[(x, y)].fg));
        }
        None
    }

    /// The sources column's state word, whatever row it landed on.
    fn conn_word(a: &App) -> Option<(String, Color)> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, a)).unwrap();
        let buf = terminal.backend().buffer();
        for y in 0..30u16 {
            let row: String = (0..14u16).map(|x| buf[(x, y)].symbol()).collect();
            let word = row.trim_matches(|c| c == '│' || c == ' ').to_string();
            if ["sign in", "connecting", "connected", "failed"].contains(&word.as_str()) {
                // The colour of the word's first cell — the thing a user reads
                // before they read the word.
                let x = row.chars().position(|c| !matches!(c, '│' | ' ')).unwrap() as u16;
                return Some((word, buf[(x, y)].fg));
            }
        }
        None
    }

    /// **The four states are four different screens.**
    ///
    /// This is the lesson of the previous phase, applied before the mistake:
    /// an unconfigured Deck that looked exactly like a working one was read as a
    /// broken one. A user must be able to tell "nothing configured" from
    /// "talking to it now" from "it said no" without reading a manual — so the
    /// word is in the sources column and the reason is in the footer.
    #[test]
    fn every_connection_state_renders_differently() {
        use crate::app::ConnState;

        let states = [
            ConnState::NotConfigured,
            ConnState::NeedsPassword {
                name: "home".into(),
            },
            ConnState::Connecting {
                name: "home".into(),
            },
            ConnState::Connected {
                name: "home".into(),
                username: "rod".into(),
            },
            ConnState::Failed {
                name: "home".into(),
                message: "Wrong username or password.".into(),
            },
        ];

        let mut frames: Vec<Vec<String>> = Vec::new();
        for state in states {
            let mut a = connecting_to(state.clone());
            // The footer note the fold would have put up alongside the state.
            match &state {
                ConnState::NeedsPassword { name } => {
                    a.set_status(format!(
                        "{name} · no password stored · run: eko-cli login {name}"
                    ));
                }
                ConnState::Failed { name, message } => a.set_status(format!("{name} · {message}")),
                _ => {}
            }
            let frame = render(100, 30, &a);
            assert!(
                !frames.contains(&frame),
                "{state:?} renders identically to an earlier state"
            );
            frames.push(frame);
        }
    }

    /// The lamp on the server's row, and the word under it — the seal's own
    /// vocabulary, so the user has met it before.
    ///
    /// The lamp is the assertion that carries the weight, because the lamp is
    /// what is there in **every** state: `connected` has no word at all, and a
    /// text-only test of the word would have nothing to say about the one state
    /// that matters most.
    #[test]
    fn the_sources_column_carries_the_connection_in_the_lamp_colours() {
        use crate::app::ConnState;

        let theme = Theme::new(Accent::Orange, ColorDepth::TrueColor);
        let cases = [
            (
                ConnState::NeedsPassword {
                    name: "home".into(),
                },
                ("○", theme.ink_dim),
                Some(("sign in", theme.ink_dim)),
            ),
            (
                ConnState::Connecting {
                    name: "home".into(),
                },
                ("○", theme.led_amber),
                Some(("connecting", theme.led_amber)),
            ),
            (
                ConnState::Connected {
                    name: "home".into(),
                    username: "rod".into(),
                },
                ("●", theme.led_green),
                None,
            ),
            (
                ConnState::Failed {
                    name: "home".into(),
                    message: "nope".into(),
                },
                ("○", theme.led_red),
                Some(("failed", theme.led_red)),
            ),
        ];
        for (state, lamp, word) in cases {
            let a = connecting_to(state.clone());
            assert_eq!(
                source_lamp(&a, "home"),
                Some((lamp.0.to_string(), lamp.1)),
                "{state:?} did not light the server's lamp"
            );
            assert_eq!(
                conn_word(&a),
                word.map(|(w, c)| (w.to_string(), c)),
                "{state:?} did not render the word it should have"
            );
        }

        // And a Deck nobody asked to connect grows no line about connecting.
        assert_eq!(conn_word(&connecting_to(ConnState::NotConfigured)), None);
        assert_eq!(conn_word(&app()), None);
    }

    // ── the server as a browsable source ─────────────────────────────────

    fn remote_album(id: &str, name: &str, artist: &str, songs: u32) -> crate::remote::Album {
        crate::remote::Album {
            id: id.to_string(),
            name: name.to_string(),
            artist: artist.to_string(),
            song_count: Some(songs),
            tracks: None,
        }
    }

    fn remote_track(id: &str, title: &str, n: u32, secs: f64) -> crate::remote::Track {
        crate::remote::Track {
            id: id.to_string(),
            title: title.to_string(),
            artist: "Talk Talk".to_string(),
            album: "Spirit of Eden".to_string(),
            track_no: Some(n),
            duration: secs,
        }
    }

    /// A Deck connected to `home`, browsing the server's album list.
    ///
    /// Stated rather than fetched, exactly as [`scanned`] states the local
    /// library: the fetch itself is covered against mockito in `remote.rs`, and a
    /// render fixture that needed a socket would be a render fixture that could
    /// fail for reasons that have nothing to do with rendering.
    fn remote_browsing() -> App {
        use crate::app::{ConnState, RemoteState};

        let mut a = connecting_to(ConnState::Connected {
            name: "home".into(),
            username: "rod".into(),
        });
        a.remote = crate::remote::Library {
            albums: vec![
                remote_album("al-1", "Spirit of Eden", "Talk Talk", 6),
                remote_album("al-2", "Laughing Stock", "Talk Talk", 6),
                remote_album("al-3", "Loveless", "My Bloody Valentine", 11),
            ],
        };
        a.remote_state = RemoteState::Ready;
        a.sources[1].badge = Some("3".into());
        // The sidebar cursor is on the server row: that is what selects it.
        a.selected = 1;
        a.context = "home · 3 albums".into();
        a
    }

    /// [`remote_browsing`], with album 0 open and its tracks fetched.
    fn remote_album_open() -> App {
        let mut a = remote_browsing();
        a.remote.albums[0].tracks = Some(vec![
            remote_track("t1", "The Rainbow", 1, 555.0),
            remote_track("t2", "Eden", 2, 386.0),
            remote_track("t3", "Desire", 3, 418.0),
        ]);
        a.remote_view = View::Album(0);
        a
    }

    /// Snapshot: **a browsable remote album list.** The deliverable of Task 2.
    ///
    /// `home` is a row beside `Local`, with its album count in the badge column
    /// and `connected` under it in the seal's green. `NOT YET` now holds only the
    /// views that really are unbuilt.
    #[test]
    fn deck_at_100x30_browsing_the_server() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────────────────── home · 3 albums ┐",
            "│ SOURCES        │ HOME                                                                            │",
            "│                │ 3 albums                                                                        │",
            "│ ○ Local        │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ ● home       3 │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ○ Search       │ ▌     Talk Talk — Spirit of Eden     6        enter to load                     │",
            "│ ○ Queue        │       Talk Talk — Laughing Stock     6                                          │",
            "│                │       My Bloody Valentine — Lovel…  11                                          │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ⏹ Nothing playing                                     ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │",
            "│ ░░░░░░░░  —                                                                                      │",
            "│ ░░░░░░░░  00:00 ────────────────────────────────────────────────────────────────────────── 00:00 │",
            "│ ░░░░░░░░  — → —                                                      ○ IDLE    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",

        ];
        assert_eq!(render(100, 30, &remote_browsing()), expected);
    }

    /// Snapshot: **an open remote album.** Identical chrome to a local one.
    #[test]
    fn deck_at_100x30_with_a_remote_album_open() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────────────────── home · 3 albums ┐",
            "│ SOURCES        │ ‹ SPIRIT OF EDEN                                                                │",
            "│                │ Talk Talk · 3 tracks                                                            │",
            "│ ○ Local        │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ ● home       3 │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ○ Search       │ ▌     Talk Talk — Spirit of Eden     3  ▌  1  The Rainbow                  9:15 │",
            "│ ○ Queue        │       Talk Talk — Laughing Stock     6     2  Eden                         6:26 │",
            "│                │       My Bloody Valentine — Lovel…  11     3  Desire                       6:58 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ⏹ Nothing playing                                     ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │",
            "│ ░░░░░░░░  —                                                                                      │",
            "│ ░░░░░░░░  00:00 ────────────────────────────────────────────────────────────────────────── 00:00 │",
            "│ ░░░░░░░░  — → —                                                      ○ IDLE    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",

        ];
        assert_eq!(render(100, 30, &remote_album_open()), expected);
    }

    // ── the queue ────────────────────────────────────────────────────────

    /// Move the sidebar cursor to `source`, the way `tab` and `j` would — so the
    /// context line and the pane are the ones the gesture really produces.
    fn select(a: &mut App, source: crate::app::Source) {
        use crate::app::Focus;
        let was = a.focus;
        a.focus = Focus::Sidebar;
        while a.source() != source {
            a.act(crate::keys::Action::Down);
        }
        a.focus = was;
    }

    // ── search ───────────────────────────────────────────────────────────

    /// The main pane's list rows of a 100×30 frame — chrome, rule and footer cut
    /// away. The footer draws a `▶` for the transport, and that one is a claim
    /// about the engine rather than about a row.
    fn list_rows(rows: &[String]) -> impl Iterator<Item = &String> {
        rows[FIRST_LIST_ROW..24].iter()
    }

    /// Give the Deck a live client, so a result row can actually be started.
    ///
    /// The address answers nothing — the engine fails to open the stream, exactly
    /// as the unopenable `.flac` paths in [`track`] do — which is all these render
    /// tests need: `App::started` has run, so the transport and the markers are in
    /// the state a real start leaves them in.
    fn with_a_live_client(a: &mut App) {
        a.connection = Some(crate::server::Connection {
            name: "home".into(),
            username: "rod".into(),
            client: std::sync::Arc::new(
                eko_net::Client::new(eko_net::Config {
                    base_url: "http://127.0.0.1:1".into(),
                    username: "rod".into(),
                    password: "hunter2".into(),
                })
                .unwrap(),
            ),
        });
    }

    /// A Deck showing the answer to `eden`: two matched albums and two matched
    /// songs, in one list.
    ///
    /// Stated rather than fetched, for the reason [`remote_browsing`] gives — and
    /// the fetch that produces exactly this shape is covered against mockito in
    /// both `remote.rs` and `app.rs`, end to end through the worker thread.
    fn searched() -> App {
        use crate::app::SearchState;

        let mut a = remote_browsing();
        with_a_live_client(&mut a);
        a.search.query = "eden".into();
        a.search.state = SearchState::Ready;
        a.search.results = crate::remote::Results {
            albums: vec![
                remote_album("al-1", "Spirit of Eden", "Talk Talk", 6),
                remote_album("al-2", "Laughing Stock", "Talk Talk", 6),
            ],
            songs: vec![
                remote_track("t2", "Eden", 2, 386.0),
                remote_track("t3", "Desire", 3, 418.0),
            ],
        };
        select(&mut a, crate::app::Source::Search);
        a
    }

    /// **Snapshot: the results list at 100×30.**
    ///
    /// Albums first, then songs, in one flat list the cursor walks by index —
    /// with the *kind* in the dim right-hand column rather than as section
    /// headings, because a heading is a row the cursor would have to refuse and
    /// this Deck does not have rows like that. An album row carries its track
    /// count; a song row carries its duration.
    #[test]
    fn deck_at_100x30_showing_search_results() {
        let expected = vec![
            "┌─ EKO ───────────────────────────────────────────────────────────────── search · eden · 4 results ┐",
            "│ SOURCES        │ SEARCH                                                                          │",
            "│                │ eden · 2 albums · 2 songs                                                       │",
            "│ ○ Local        │       RESULT                                                                    │",
            "│ ● home       3 │ ────────────────────────────────────────────────────────────────                │",
            "│ ● Search       │ ▌     Talk Talk — Spirit of Eden                        album  6                │",
            "│ ○ Queue        │       Talk Talk — Laughing Stock                        album  6                │",
            "│                │       Eden — Talk Talk                                song  6:26                │",
            "│                │       Desire — Talk Talk                              song  6:58                │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ⏹ Nothing playing                                     ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │",
            "│ ░░░░░░░░  —                                                                                      │",
            "│ ░░░░░░░░  00:00 ────────────────────────────────────────────────────────────────────────── 00:00 │",
            "│ ░░░░░░░░  — → —                                                      ○ IDLE    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",

        ];
        assert_eq!(render(100, 30, &searched()), expected);
    }

    /// The query is typed into the summary row, with a caret, and the pane below
    /// says what enter and esc will do.
    #[test]
    fn an_open_search_input_shows_the_query_and_what_the_keys_do() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

        let mut a = remote_browsing();
        a.act(crate::keys::Action::Search);
        for c in "talk".chars() {
            a.handle(crate::app::AppEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            ))));
        }
        let rows = render(100, 30, &a);
        assert!(rows[1].contains("SEARCH"), "{}", rows[1]);
        assert!(rows[2].contains("/talk▏"), "{}", rows[2]);
        assert!(
            rows.iter()
                .any(|r| r.contains("Type a query and press enter.")),
            "{}",
            rows.join("\n")
        );
    }

    /// Every state of the pane says what happened and what to do — and the two
    /// that would otherwise look identical, "nobody has asked" and "the server
    /// matched nothing", say different things.
    #[test]
    fn every_search_state_says_what_is_wrong_and_what_to_do() {
        use crate::app::SearchState;

        let cases: Vec<(SearchState, &str)> = vec![
            (SearchState::Idle, "Nothing asked of home yet."),
            (SearchState::Searching, "Asking home about eden."),
            (SearchState::Ready, "home has nothing matching eden."),
            (
                SearchState::Failed("the server answered HTTP 502".into()),
                "Could not search home.",
            ),
        ];
        for (state, sentence) in cases {
            let mut a = remote_browsing();
            select(&mut a, crate::app::Source::Search);
            a.search.query = "eden".into();
            a.search.state = state.clone();
            let rows = render(100, 30, &a);
            assert!(
                rows.iter().any(|r| r.contains(sentence)),
                "{state:?} did not say {sentence:?}:\n{}",
                rows.join("\n")
            );
        }
    }

    /// **A result playing lights the row it is actually playing — by id.**
    ///
    /// The other panes mark by index and cannot here: a result is at no index in
    /// either library, so its queue entry carries [`crate::queue::NO_ROW`] and a
    /// positional test would never fire. Identity is exact instead.
    #[test]
    fn a_playing_result_is_marked_by_id_not_by_position() {
        let mut a = searched();
        // Rows 0 and 1 are albums; row 3 is the second matched song.
        for _ in 0..3 {
            a.act(crate::keys::Action::Down);
        }
        a.act(crate::keys::Action::Open);
        assert_eq!(a.now.as_ref().map(|n| n.title.as_str()), Some("Desire"));

        let rows = render(100, 30, &a);
        // The list only — the footer's transport row draws a `▶` of its own, and
        // that one is about the engine rather than about a row.
        let marked: Vec<&String> = list_rows(&rows).filter(|r| r.contains('▶')).collect();
        assert_eq!(
            marked.len(),
            1,
            "exactly one row should claim to be playing:\n{}",
            rows.join("\n")
        );
        assert!(marked[0].contains("Desire"), "{}", marked[0]);

        // …and the *other* library, which has an album at every one of the
        // indices a result does not have, lights nothing at all.
        let mut local = a;
        local.library = scanned().library;
        local.scan = crate::app::ScanState::Ready;
        select(&mut local, crate::app::Source::Local);
        let rows = render(100, 30, &local);
        assert!(
            list_rows(&rows).all(|r| !r.contains('▶')),
            "a search result lit a row in the local library:\n{}",
            rows.join("\n")
        );
    }

    /// **A hostile search result cannot write an escape into the rendered
    /// frame.**
    ///
    /// [`crate::remote::Album`] and [`crate::remote::Track`] are only
    /// constructible through `tidy`, so a genuinely hostile value cannot be stated
    /// here at all — which is the guarantee, and `remote.rs` pins it against a
    /// mock server that really does send one. What this pins is the other half:
    /// the tidied output renders as inert text, in rows that are still exactly
    /// 100 cells wide.
    #[test]
    fn a_hostile_search_result_cannot_write_an_escape_into_the_rendered_frame() {
        let mut a = searched();
        a.search.results.albums[0].name = "[2J [31mRED".into();
        a.search.results.songs[0].title = "[5mBLINK".into();
        a.search.query = "]0;pwned".into();
        let rows = render(100, 30, &a);
        for row in &rows {
            assert!(
                !row.chars().any(char::is_control),
                "a control character reached the frame: {row:?}"
            );
            assert!(
                !row.contains(['\u{202e}', '\u{2028}', '\u{2029}']),
                "a bidi override reached the frame: {row:?}"
            );
            assert_eq!(row.chars().count(), 100, "a row was not 100 cells: {row:?}");
        }
        assert!(rows.iter().any(|r| r.contains("[2J [31mRED")));
    }

    /// **A queue holding tracks from both libraries**, with the Queue pane open.
    ///
    /// Built with the real gestures rather than by writing into the queue: the
    /// local album is *played* (which is what makes it the queue), then two of
    /// the server's tracks are added with `a`. If either path stopped working
    /// this fixture would stop being a mixed queue, which is exactly what the
    /// snapshot below is for.
    fn mixed_queue() -> App {
        let mut a = remote_album_open();
        a.library = scanned().library;
        a.scan = ScanState::Ready;
        a.sources[0].badge = Some("2".into());

        // The local album becomes the queue, playing its second track.
        select(&mut a, Source::Local);
        a.view = View::Album(0);
        a.play(Source::Local, 0, 1);
        a.pos_ms = 124_000;
        a.bands = (0..32).map(|i| (i as f32) / 31.0).collect();

        // Two of the server's tracks, queued behind it.
        select(&mut a, Source::Remote);
        a.remote_track_cursor = 0;
        a.act(crate::keys::Action::Enqueue);
        a.remote_track_cursor = 1;
        a.act(crate::keys::Action::Enqueue);

        select(&mut a, Source::Queue);
        a.queue_cursor = 3;
        // The `a` note has had its moment; the seal row is not a log.
        a.status = None;
        a
    }

    /// Snapshot: **the Queue pane, holding a local track and a remote one.**
    ///
    /// The deliverable of Task 4, and the frame that shows the model working: one
    /// ordered list, five entries, three from the scanned folder and two from
    /// `home`, the `▶` on the one that is playing and the cursor somewhere else.
    /// The right-hand column names which library each entry came from, because
    /// this is the only list in the Deck where that can differ row to row.
    #[test]
    fn deck_at_100x30_with_a_mixed_queue() {
        let expected = vec![
            "┌─ EKO ──────────────────────────────────────────────────────────────────────────── queue · 2 of 5 ┐",
            "│ SOURCES        │ QUEUE                                                                           │",
            "│                │ 5 tracks · playing 2 of 5                                                       │",
            "│ ● Local      2 │    #  TITLE                                                 TIME                │",
            "│ ● home       3 │ ────────────────────────────────────────────────────────────────                │",
            "│ ○ Search       │    1  Xtal — Aphex Twin                              local  4:51                │",
            "│ ● Queue      5 │    ▶  Tha — Aphex Twin                               local  9:09                │",
            "│                │    3  Pulsewidth — Aphex Twin                        local  3:52                │",
            "│                │ ▌  4  The Rainbow — Talk Talk                         home  9:15                │",
            "│                │    5  Eden — Talk Talk                                home  6:26                │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ Tha                                                 ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Aphex Twin · Selected Ambient Works 85-92                                       2 of 5 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:09 │",
            "│ ░░░░░░░░  — → —                                                ○ UNVERIFIED    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",

        ];
        assert_eq!(render(100, 30, &mixed_queue()), expected);
    }

    /// The Queue pane with nothing in it names the two keys that fill it, rather
    /// than showing a blank list that reads as a broken one.
    #[test]
    fn an_empty_queue_says_how_to_fill_it() {
        let mut a = scanned();
        select(&mut a, Source::Queue);
        assert_eq!(a.row_count(), 0);
        let rows = render(100, 30, &a);
        assert!(rows[1].contains("QUEUE"), "{}", rows[1]);
        assert!(rows[2].contains("nothing queued"), "{}", rows[2]);
        assert!(rows[4].contains("Nothing queued."), "{}", rows[4]);
        assert!(
            rows.iter().any(|r| r.contains("Press a on an album")),
            "the empty queue does not say how to fill it"
        );
    }

    /// The server's rows are rows: a cursor lands on them, and the heading and
    /// the list agree about which source is on screen.
    #[test]
    fn the_server_is_a_selectable_source_and_the_pane_follows_it() {
        use crate::app::{Focus, Source};

        let mut a = remote_browsing();
        a.library = scanned().library;
        a.selected = 0;
        let rows = render(100, 30, &a);
        assert!(rows[1].contains("LOCAL"), "{}", rows[1]);
        assert!(
            rows[FIRST_LIST_ROW].contains("Aphex Twin"),
            "{}",
            rows[FIRST_LIST_ROW]
        );

        a.focus = Focus::Sidebar;
        a.act(crate::keys::Action::Down);
        assert_eq!(a.source(), Source::Remote);
        let rows = render(100, 30, &a);
        assert!(rows[1].contains("HOME"), "{}", rows[1]);
        assert!(rows[2].contains("3 albums"), "{}", rows[2]);
        assert!(
            rows[FIRST_LIST_ROW].contains("Talk Talk — Spirit of Eden"),
            "{}",
            rows[FIRST_LIST_ROW]
        );
        // The count is right-aligned at the album column's own edge now, not at
        // the frame's — see `the_right_hand_column_stops_at_a_fixed_edge`.
        assert!(
            album_column(&rows[FIRST_LIST_ROW]).ends_with('6'),
            "{}",
            rows[FIRST_LIST_ROW]
        );
    }

    /// **The connection word sits on the server's own row.**
    ///
    /// It used to hang beneath a `NOT YET` heading because that was the only
    /// place the server appeared at all. That heading is gone entirely now; what
    /// this still holds is the part that was ever load-bearing — the word is an
    /// annotation on the row above it, in the row immediately below, and the
    /// server is a row with a lamp on it rather than prose.
    #[test]
    fn the_connection_word_annotates_the_server_row() {
        use crate::app::ConnState;

        let a = connecting_to(ConnState::Connecting {
            name: "home".into(),
        });
        let sidebar: Vec<String> = render(100, 30, &a)
            .iter()
            .map(|r| {
                r.chars()
                    .take(usize::from(SIDEBAR_WIDTH) + 1)
                    .collect::<String>()
            })
            .collect();

        let server = sidebar
            .iter()
            .position(|r| r.contains("home"))
            .expect("no server row");
        let word = sidebar
            .iter()
            .position(|r| r.contains("connecting"))
            .expect("no connection word");

        assert_eq!(word, server + 1, "the word is not on the server's own row");
        assert!(
            sidebar[server].contains('○'),
            "the server is not a row with a lamp: {}",
            sidebar[server]
        );
    }

    /// Every remote state is its own screen, with its own fix on it. The lesson
    /// of the previous phase, applied to the second source.
    #[test]
    fn every_remote_state_says_what_is_wrong_and_what_to_do() {
        use crate::app::{ConnState, RemoteState};

        let cases: Vec<(&str, App, Vec<&str>)> = vec![
            (
                "no password",
                connecting_to(ConnState::NeedsPassword {
                    name: "home".into(),
                }),
                vec!["No password stored for home.", "eko-cli login home"],
            ),
            (
                "connecting",
                connecting_to(ConnState::Connecting {
                    name: "home".into(),
                }),
                vec!["Talking to home."],
            ),
            (
                "refused",
                connecting_to(ConnState::Failed {
                    name: "home".into(),
                    message: "Wrong username or password.".into(),
                }),
                vec!["home did not answer.", "Wrong username or password."],
            ),
            (
                "empty server",
                {
                    let mut a = remote_browsing();
                    a.remote = crate::remote::Library::default();
                    a.sources[1].badge = None;
                    a
                },
                vec!["No albums on home."],
            ),
            (
                "walk failed",
                {
                    let mut a = remote_browsing();
                    a.remote = crate::remote::Library::default();
                    a.remote_state = RemoteState::Failed("cannot reach the server".into());
                    a.sources[1].badge = None;
                    a
                },
                vec!["Could not read home's albums.", "cannot reach the server"],
            ),
        ];

        for (what, mut a, sentences) in cases {
            // Every one of these is the *server's* pane, so select it.
            a.selected = 1;
            assert_eq!(a.row_count(), 0, "{what}: the fixture must have no rows");
            let rows = render(100, 30, &a);
            for sentence in sentences {
                assert!(
                    rows.iter().any(|r| r.contains(sentence)),
                    "{what}: {sentence:?} is not on screen\n{}",
                    rows.join("\n")
                );
            }
            // …and it opens with a statement, not with the keymap.
            let first = rows[4].trim_matches(|c| c == '│' || c == ' ');
            assert!(
                !first.contains("quit"),
                "{what}: the keymap is the first thing on screen"
            );
        }
    }

    /// A part-loaded library says how far it got rather than looking finished.
    #[test]
    fn a_walk_in_flight_says_how_many_albums_have_landed() {
        use crate::app::RemoteState;

        let mut a = remote_browsing();
        a.remote_state = RemoteState::Loading;
        let rows = render(100, 30, &a);
        assert!(rows[2].contains("loading albums · 3 so far"), "{}", rows[2]);
        // …and the albums that have landed are already browsable.
        assert!(
            rows[FIRST_LIST_ROW].contains("Spirit of Eden"),
            "{}",
            rows[FIRST_LIST_ROW]
        );
    }

    /// A half-fetched library that then lost the server is *incomplete*, not
    /// empty — and it says so without hiding what it has.
    #[test]
    fn an_incomplete_walk_says_so_beside_the_albums_it_did_get() {
        use crate::app::RemoteState;

        let mut a = remote_browsing();
        a.remote_state = RemoteState::Failed("cannot reach the server".into());
        let rows = render(100, 30, &a);
        assert!(rows[2].contains("3 albums · incomplete"), "{}", rows[2]);
        assert!(rows[2].contains("cannot reach the server"), "{}", rows[2]);
        assert!(
            rows[FIRST_LIST_ROW].contains("Spirit of Eden"),
            "{}",
            rows[FIRST_LIST_ROW]
        );
    }

    /// An open album that is still fetching says so, and an empty one says
    /// *that* — two different states, two different sentences.
    #[test]
    fn an_open_remote_album_distinguishes_loading_from_empty() {
        use crate::app::DetailState;

        let mut a = remote_browsing();
        a.remote_view = View::Album(0);
        a.remote_detail = DetailState::Loading;
        let rows = render(100, 30, &a);
        assert!(rows[1].contains("SPIRIT OF EDEN"), "{}", rows[1]);
        assert!(rows[2].contains("loading tracks…"), "{}", rows[2]);
        assert!(
            rows[FIRST_LIST_ROW].contains("Loading Spirit of Eden."),
            "{}",
            rows[FIRST_LIST_ROW]
        );

        // Fetched, and genuinely empty.
        a.remote_detail = DetailState::Idle;
        a.remote.albums[0].tracks = Some(Vec::new());
        let rows = render(100, 30, &a);
        assert!(rows[2].contains("0 tracks"), "{}", rows[2]);
        assert!(
            rows[FIRST_LIST_ROW].contains("Spirit of Eden has no tracks."),
            "{}",
            rows[FIRST_LIST_ROW]
        );

        // And a fetch that failed carries the reason.
        a.remote_detail = DetailState::Failed("the server answered HTTP 500".into());
        let rows = render(100, 30, &a);
        assert!(
            rows[2].contains("the server answered HTTP 500"),
            "{}",
            rows[2]
        );
        assert!(
            rows[FIRST_LIST_ROW].contains("Could not load Spirit of Eden."),
            "{}",
            rows[FIRST_LIST_ROW]
        );
    }

    /// **A hostile server cannot write to the terminal, end to end.**
    ///
    /// Everything above this point states its remote library. This one *fetches*
    /// one, from a mock server whose album names are packed with terminal control
    /// sequences — a clear-screen, an SGR that would repaint the seal row, an OSC
    /// that would retitle the window, a bidi override — then folds the result
    /// into the Deck and renders a real frame.
    ///
    /// The assertion is on the rendered buffer, not on the model: every cell of
    /// every row must be a printable symbol. That is the property that actually
    /// matters, and it is the only place the whole chain — parse, sanitise, fold,
    /// draw — is checked at once.
    #[test]
    fn a_hostile_server_cannot_write_an_escape_into_the_rendered_frame() {
        use crate::app::RemoteState;
        use mockito::{Matcher, Server};

        let mut server = Server::new();
        // Written with JSON `\u` escapes because a raw control character is not
        // legal JSON; `serde_json` turns them into real control characters
        // before `eko-cli` sees them.
        let page = concat!(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{"album":[{"#,
            r#""id":"al-0001","#,
            r#""name":"\u001b[2J\u001b[31mSCREAM\u0007","#,
            r#""artist":"\u001b]0;pwned\u0007Corp\u202e","#,
            r#""songCount":3}]}}}"#
        );
        let _p0 = server
            .mock("GET", "/rest/getAlbumList2")
            .match_query(Matcher::Regex(r"size=500&offset=0$".into()))
            .with_status(200)
            .with_body(page)
            .create();
        let _end = server
            .mock("GET", "/rest/getAlbumList2")
            .match_query(Matcher::Regex(r"size=500&offset=1$".into()))
            .with_status(200)
            .with_body(r#"{"subsonic-response":{"status":"ok","albumList2":{}}}"#)
            .create();

        let client = eko_net::Client::new(eko_net::Config {
            base_url: server.url(),
            username: "rod".into(),
            password: "hunter2".into(),
        })
        .unwrap();

        let fetched = std::sync::Mutex::new(Vec::new());
        crate::remote::walk_albums(&client, &|kind| {
            if let crate::remote::RemoteKind::Page(albums) = kind {
                fetched.lock().unwrap().extend(albums);
            }
            true
        });
        let albums = fetched.into_inner().unwrap();
        assert_eq!(albums.len(), 1, "the fixture must produce one album");

        let mut a = remote_browsing();
        a.remote = crate::remote::Library { albums };
        a.remote_state = RemoteState::Ready;

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        for y in 0..30u16 {
            for x in 0..100u16 {
                let symbol = buf[(x, y)].symbol();
                assert!(
                    !symbol.chars().any(char::is_control),
                    "a control character reached cell ({x}, {y}): {symbol:?}"
                );
                assert!(
                    !symbol.contains(['\u{202e}', '\u{2028}', '\u{2029}']),
                    "a bidi override reached cell ({x}, {y}): {symbol:?}"
                );
            }
        }
        // …and the seal row is still the seal row.
        assert!(lines_of(buf)[28].contains("IDLE"));
    }

    /// The word cannot be so long it is clipped by the sources column, at any
    /// supported size.
    ///
    /// `Connected` is not in this list. It has no word any more — the green lamp
    /// on the server's own row is the whole of what `connected` said, and
    /// printing both is the `◆ EQ` beside `EQ +3.0` mistake in the other pane.
    #[test]
    fn the_connection_word_fits_the_sources_column() {
        use crate::app::ConnState;

        for state in [
            ConnState::NeedsPassword {
                name: "home".into(),
            },
            ConnState::Connecting {
                name: "home".into(),
            },
            ConnState::Failed {
                name: "home".into(),
                message: "nope".into(),
            },
        ] {
            let a = connecting_to(state.clone());
            let word = conn_word(&a).map(|(w, _)| w);
            assert!(word.is_some(), "{state:?} lost its word at 100×30");
            for (w, h) in [(80, 20), (240, 80)] {
                let rows = render(w, h, &a);
                assert!(
                    rows.iter()
                        .any(|r| r.contains(word.as_deref().unwrap_or(""))),
                    "{state:?} clipped at {w}×{h}"
                );
            }
        }
    }

    /// **The failure's reason reaches the user, and the seal survives it.**
    #[test]
    fn a_connection_failure_reads_its_reason_without_hiding_the_seal() {
        use crate::app::ConnState;

        for message in [
            "Wrong username or password.",
            "cannot reach the server · error sending request",
        ] {
            let mut a = connecting_to(ConnState::Failed {
                name: "home".into(),
                message: message.into(),
            });
            a.set_status(format!("home · {message}"));
            let rows = render(100, 30, &a);
            let seal = &rows[28];
            // Enough of the sentence to tell the two apart at a glance.
            let head: String = message.chars().take(20).collect();
            assert!(seal.contains(&head), "{seal}");
            assert!(seal.contains("IDLE"), "the seal was pushed off: {seal}");
        }
    }

    /// Snapshot: a server configured, nothing stored for it yet. **First run.**
    #[test]
    fn deck_at_100x30_with_a_server_that_needs_a_password() {
        use crate::app::ConnState;

        let mut a = connecting_to(ConnState::NeedsPassword {
            name: "home".into(),
        });
        a.set_status("home · no password stored · run: eko-cli login home");
        let expected = vec![
            "┌─ EKO ────────────────────────────────────────────────────────────────────────── no source · idle ┐",
            "│ SOURCES        │ LOCAL                                                                           │",
            "│                │ no music folder · set music_folder in ~/.config/eko/config.toml                 │",
            "│ ○ Local        │                                                                                 │",
            "│ ○ home         │ No music folder.                                                                │",
            "│  sign in       │                                                                                 │",
            "│ ○ Search       │ Put this in ~/.config/eko/config.toml:                                          │",
            "│ ○ Queue        │                                                                                 │",
            "│                │     music_folder = \"/path/to/music\"                                             │",
            "│                │                                                                                 │",
            "│                │ Then press r to scan it.                                                        │",
            "│                │                                                                                 │",
            "│                │       q  quit                         s  spectrum on / off                      │",
            "│                │     tab  sources ⇄ library            z  now playing                            │",
            "│                │   j / ↓  down                         r  rescan · reconnect                     │",
            "│                │   k / ↑  up                           /  search the server                      │",
            "│                │   enter  open album · play track      d  output device                          │",
            "│                │     esc  back                         t  sleep timer                            │",
            "│                │   space  play / pause                 ?  keys · this list                       │",
            "│                │       n  next track                   e  EQ panel                               │",
            "│                │       p  previous track               E  EQ on / off                            │",
            "│                │       a  add to queue                 h  EQ band left                           │",
            "│                │       x  remove from queue            l  EQ band right                          │",
            "│ tab sources    │          …                                                                      │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ⏹ Nothing playing                                     ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │",
            "│ ░░░░░░░░  —                                                                                      │",
            "│ ░░░░░░░░  00:00 ────────────────────────────────────────────────────────────────────────── 00:00 │",
            "│ ░░░░░░░░  home · no password stored · run: eko-cli login home        ○ IDLE    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &a), expected);
    }

    // ── tab ──────────────────────────────────────────────────────────────

    /// **`Tab` is on screen, always.**
    ///
    /// `Action::ToggleFocus` was bound to `Tab` from the first draft and nothing
    /// said so anywhere the owner would look: the keymap names it, but the
    /// keymap only renders when the library is empty, so it vanished the moment
    /// the Deck had something in it. The hint is pinned to the bottom of the
    /// sources column and names the pane the key will take you *to*.
    #[test]
    fn the_tab_hint_is_on_screen_whatever_is_playing() {
        for (what, a) in [
            ("cold", app()),
            ("a library", scanned()),
            ("playing", playing()),
            ("sealed", sealed()),
        ] {
            let rows = render(100, 30, &a);
            let hint = &rows[rows.len() - 7]; // last body row, above the rule
            assert!(hint.contains("tab"), "{what}: no tab hint: {hint}");
        }
    }

    #[test]
    fn the_tab_hint_names_the_pane_it_will_move_to() {
        use crate::app::Focus;

        let mut a = scanned();
        assert_eq!(a.focus, Focus::Main);
        let hint = |a: &App| render(100, 30, a)[23].clone();
        assert!(hint(&a).contains("tab sources"), "{}", hint(&a));

        a.act(crate::keys::Action::ToggleFocus);
        assert_eq!(a.focus, Focus::Sidebar);
        assert!(hint(&a).contains("tab library"), "{}", hint(&a));
    }

    /// It survives the smallest supported terminal, where the column is ten
    /// rows shorter — and it is dropped rather than drawn over a source.
    #[test]
    fn the_tab_hint_fits_the_smallest_supported_terminal() {
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let rows = render(w, h, &scanned());
            let hint = &rows[rows.len() - 7];
            assert!(hint.contains("tab"), "no tab hint at {w}×{h}: {hint}");
            // The source is still there; the hint did not land on it.
            assert!(
                rows.iter().any(|r| r.contains("● Local")),
                "the hint overwrote the source list at {w}×{h}"
            );
        }
    }

    // ── the gutters ───────────────────────────────────────────────────────

    /// A library whose album is named at greater length than any pane is wide.
    ///
    /// Not a straw man. `main_pane` does not truncate its rows — a row that
    /// overruns is *clipped* by the rect it is drawn into — so the width of that
    /// rect is the only thing standing between a long name and the frame, and
    /// long names are ordinary. This is the fixture that told the difference
    /// between a pane that has a right-hand gutter and one that merely usually
    /// looks like it does.
    fn overlong() -> App {
        let mut a = scanned();
        a.library.albums[0].name = "Selected Ambient Works 85-92 ".repeat(20);
        a.library.albums[0].artist = "Aphex Twin ".repeat(40);
        a
    }

    /// **One blank column inside every pane border, on both sides.**
    ///
    /// The four columns are the ones [`GUTTER`] pays for: inside the frame on
    /// the left, either side of the sidebar's rule, and inside the frame on the
    /// right. Every body row and every footer row, with content that wants more
    /// room than there is.
    ///
    /// No overlay is open in any of these fixtures, and that is deliberate
    /// rather than convenient: the help overlay is centred over the *body* and
    /// legitimately crosses the sidebar's rule, so it is out of this rule's
    /// scope. Its own padding is the same constant — see [`help`] — and
    /// [`the_help_overlay_never_covers_the_seal_at_any_usable_size`] holds the
    /// property that actually matters about where it lands.
    #[test]
    fn every_pane_keeps_a_blank_column_inside_its_border() {
        for (what, a) in [
            ("cold", app()),
            ("a library", scanned()),
            ("playing", playing()),
            ("overlong names", overlong()),
        ] {
            for (w, h) in [(80, 20), (100, 30), (240, 80)] {
                let rows = render(w, h, &a);
                let separator = usize::from(SIDEBAR_WIDTH) + 1;
                let gutters = [
                    ("the frame's left", 1usize),
                    ("the sidebar rule's left", separator - 1),
                    ("the sidebar rule's right", separator + 1),
                    ("the frame's right", usize::from(w) - 2),
                ];
                // Everything but the frame's own top and bottom rows, and the
                // horizontal rule between the body and the footer — that rule is
                // `─` from edge to edge by design.
                let rule = usize::from(h) - 2 - usize::from(FOOTER_HEIGHT);
                for (y, row) in rows.iter().enumerate() {
                    if y == 0 || y == usize::from(h) - 1 || y == rule {
                        continue;
                    }
                    let cells: Vec<char> = row.chars().collect();
                    for (which, x) in gutters {
                        // The footer is one pane across the full width, so the
                        // sidebar's columns are not gutters down there.
                        if y > rule && x != 1 && x != usize::from(w) - 2 {
                            continue;
                        }
                        assert_eq!(
                            cells[x],
                            ' ',
                            "{what} at {w}×{h}: {which} gutter, column {x} of row {y}, \
                             holds {:?}\n{}",
                            cells[x],
                            rows.join("\n")
                        );
                    }
                }
            }
        }
    }

    // ── the library split ─────────────────────────────────────────────────

    /// The track column of a 100×30 frame row. See [`album_column`].
    fn track_column(row: &str) -> String {
        // The album column, its two-column gap, then the rest of the body.
        let x = 1 + usize::from(SIDEBAR_WIDTH) + 1 + usize::from(GUTTER) + 38 + 2;
        row.chars().skip(x).take(39).collect::<String>()
    }

    /// **The threshold is a number, not an accident.**
    ///
    /// A layout that collapses when the arithmetic happens to run out is one
    /// nobody can predict and no test can state. This asserts the constant
    /// itself and then both sides of each of its two edges, on real frames.
    #[test]
    fn the_library_is_two_columns_at_exactly_one_hundred_by_thirty() {
        use ratatui::layout::Rect;

        let at = |w, h| {
            main_pane::splits(Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            })
        };
        assert!(at(100, 30));
        assert!(!at(99, 30), "one column short of the threshold split");
        assert!(!at(100, 29), "one row short of the threshold split");
        assert!(at(240, 80));

        // And the frames agree with the function.
        let header = |w, h| render(w, h, &scanned())[FIRST_LIST_ROW - 2].clone();
        let split = header(100, 30);
        assert!(
            split.contains("ALBUM") && split.contains("TITLE"),
            "{split}"
        );
        for (w, h) in [(99, 30), (100, 29), (80, 20)] {
            let one = header(w, h);
            assert!(
                one.contains("ALBUM") && !one.contains("TITLE"),
                "{w}×{h} did not collapse to one column: {one}"
            );
        }
    }

    /// Both columns are headed on **one** row, over **one** rule, so the eye
    /// reads straight across the split rather than meeting two tables that
    /// happen to be side by side.
    #[test]
    fn the_two_columns_share_a_header_row_and_a_rule() {
        let rows = render(100, 30, &scanned());
        let header = &rows[FIRST_LIST_ROW - 2];
        let rule = &rows[FIRST_LIST_ROW - 1];
        assert!(header.contains("ALBUM"), "{header}");
        assert!(header.contains("TRACKS"), "{header}");
        assert!(header.contains("TITLE"), "{header}");
        assert!(header.contains("TIME"), "{header}");
        // One unbroken rule under both, not one per column.
        let ruled: String = rule.chars().filter(|c| *c == '─').collect();
        assert_eq!(ruled.chars().count(), 79, "{rule}");
        assert!(rule.contains(&ruled), "the rule is in two pieces: {rule}");

        // The headers sit over the columns they name: `#` in the number field,
        // `TITLE` and `ALBUM` at the same offset into their own columns.
        assert_eq!(album_column(header).find("ALBUM"), Some(6));
        let track = track_column(header);
        assert_eq!(track.find('#'), Some(3));
        assert_eq!(track.find("TITLE"), Some(6));
    }

    /// **The single worst thing in the previous build.**
    ///
    /// A track count was right-aligned to the *frame*, so on a 240-column
    /// terminal it sat a hundred and thirty cells from the title it belonged to
    /// and the table could not be read across. The columns stop at
    /// [`main_pane::COLUMN_MAX`] instead, and the rule row is the visible proof
    /// of where they stop.
    #[test]
    fn the_right_hand_column_stops_at_a_fixed_edge() {
        let rule = |w, h| {
            render(w, h, &scanned())[FIRST_LIST_ROW - 1]
                .chars()
                .filter(|c| *c == '─')
                .count()
        };
        // Below the cap the two columns take what the pane has…
        assert_eq!(rule(100, 30), 79);
        // …and above it they stop, whatever the terminal goes on to do.
        let capped = 2 * usize::from(main_pane::COLUMN_MAX) + 2;
        assert_eq!(rule(160, 40), capped);
        assert_eq!(rule(240, 80), capped);

        // So the row itself ends well clear of the frame, rather than on it.
        let row = &render(240, 80, &scanned())[FIRST_LIST_ROW];
        assert!(row.ends_with("          │"), "{row}");
    }

    /// **`▌` is the cursor. `▶` is what is coming out of the speakers.**
    ///
    /// They used to be `▸` and `▶` in the same cell in the same orange, which is
    /// a distinction you have to already know to see. Now they are different
    /// glyphs in different columns, and a row can carry either, both or neither.
    #[test]
    fn the_cursor_and_what_is_playing_are_two_different_marks() {
        let mut a = playing();
        // Track 2 is playing; put the cursor on track 1 so the two marks are on
        // different rows and neither can stand in for the other.
        a.track_cursor = 0;
        let rows = render(100, 30, &a);

        let selected = track_column(&rows[FIRST_LIST_ROW]);
        assert!(selected.contains("Xtal"), "{selected}");
        assert!(
            selected.starts_with('▌'),
            "the cursor lost its mark: {selected}"
        );
        assert!(
            !selected.contains('▶'),
            "a row that is not playing claims to be: {selected}"
        );

        let live = track_column(&rows[FIRST_LIST_ROW + 1]);
        assert!(live.contains("Tha"), "{live}");
        assert!(
            !live.starts_with('▌'),
            "a row without the cursor carries its mark: {live}"
        );
        assert!(live.contains('▶'), "what is playing is unmarked: {live}");
        // The `▶` is in the number field, where the track number would be.
        assert_eq!(live.find('▶'), Some(3));
    }

    /// A row of text too long for its column ends in `…` rather than simply
    /// stopping.
    ///
    /// The summary row was the case that shipped: at 80×20 the unconfigured
    /// pane's own sentence is sixty-three columns in a fifty-nine-column body,
    /// and it was cut with nothing to say it had been — so the path it names
    /// read as a *different, shorter* path.
    #[test]
    fn text_too_long_for_its_column_says_it_was_cut() {
        let tight = render(80, 20, &app());
        assert!(tight[2].contains("no music folder"), "{}", tight[2]);
        assert!(tight[2].contains('…'), "cut with no ellipsis: {}", tight[2]);
        // And it is cut inside the pane, not over its gutter.
        assert!(tight[2].ends_with(" │"), "{}", tight[2]);

        // Given room it fits, and there is nothing to explain.
        let wide = render(100, 30, &app());
        assert!(wide[2].contains("config.toml"), "{}", wide[2]);
        assert!(!wide[2].contains('…'), "{}", wide[2]);

        // The same rule on a list row: an album name wider than the column.
        let row = &render(100, 30, &scanned())[FIRST_LIST_ROW];
        assert!(album_column(row).contains('…'), "{row}");
    }

    /// Every source carries a lamp, and it says whether there is anything
    /// behind the row **right now**.
    #[test]
    fn every_source_carries_a_lamp_for_whether_there_is_anything_behind_it() {
        let theme = Theme::new(Accent::Orange, ColorDepth::TrueColor);
        let lit = |glyph: &str, colour| Some((glyph.to_string(), colour));

        // Nothing configured, nothing queued: every lamp hollow and dim.
        let cold = app();
        assert_eq!(source_lamp(&cold, "Local"), lit("○", theme.ink_dim));
        assert_eq!(source_lamp(&cold, "Queue"), lit("○", theme.ink_dim));

        // A scanned library and a filled queue light theirs.
        let warm = playing();
        assert_eq!(source_lamp(&warm, "Local"), lit("●", theme.led_green));
        assert_eq!(source_lamp(&warm, "Queue"), lit("●", theme.led_green));

        // A scan in flight is amber; a folder that has gone is red.
        let mut scanning = app();
        scanning.scan = ScanState::Discovering { found: 3 };
        assert_eq!(source_lamp(&scanning, "Local"), lit("○", theme.led_amber));
        assert_eq!(
            source_lamp(&missing_folder(), "Local"),
            lit("○", theme.led_red)
        );

        // And Search reports its own state rather than the server's: a
        // connected server nobody has asked anything is still an empty pane.
        assert_eq!(
            source_lamp(&searched(), "Search"),
            lit("●", theme.led_green)
        );
        assert_eq!(
            source_lamp(&remote_browsing(), "Search"),
            lit("○", theme.ink_dim)
        );
    }

    // ── the footer's ground ──────────────────────────────────────────────

    /// **The footer sits on the same ground as the rest of the console.**
    ///
    /// It used to fill its whole area with `--screen` (`#181b16`) on the theory
    /// that a footer is a device screen and a device screen stays dark. The GUI
    /// never consumed that token — `grep -rn "var(--screen)" src/` finds nothing
    /// — and eleven points under the Graphite ground read on screen as a black
    /// box bolted onto the console, which is what the owner reported. The main
    /// pane and the sidebar set no background at all, and now neither does the
    /// footer.
    ///
    /// A text snapshot cannot see this: every glyph is unchanged. So this walks
    /// the cells.
    ///
    /// # Why this is scoped rather than absolute, and why it was not deleted
    ///
    /// **Exactly one thing in this program paints a cell background on purpose,
    /// and it is a picture**: the halfblock cover, whose `▀`'s background *is*
    /// the lower of the cell's two pixels.
    ///
    /// It used to be two. The visualiser's ambient backdrop washed the whole
    /// body with that cover at eleven percent, and this test had to exempt the
    /// entire body of the `z` view to let it through — an exemption large enough
    /// that half (1) below was asserting almost nothing while the view was open.
    /// The backdrop is gone, so the exemption is back down to **the cover block
    /// and nothing else**, at every size and in every state. That is the
    /// strengthening: the rect this test forgives is now the smallest it has
    /// ever been.
    ///
    /// The temptation, when a guard stands between the product and a new
    /// feature, is to delete it. Deleting this one would remove the only thing
    /// keeping **the seal legible**, which is the one claim in this application
    /// that must never end up drawn over something that can change its colour.
    ///
    /// So it is scoped, in three parts that have to be read together:
    ///
    /// 1. **Negatively** — every cell outside the exempt rects is `Reset`.
    /// 2. **Positively** — the footer's text column is `Reset` on all four of
    ///    its rows, *and* no exempt rect may overlap that column at any size.
    ///    Half (1) alone would still pass if someone widened an exemption to the
    ///    whole screen; half (2) is what makes that widening fail.
    /// 3. **By size** — the exemption is never more than the cover's own cells.
    ///    Half (2) only forbids reaching the footer's text; this forbids
    ///    exempting the rest of the body, which is what the backdrop did.
    ///
    /// The footer's **text column** rather than the whole footer, because the
    /// halfblock cover legitimately occupies the footer's first eight columns on
    /// all four of its rows, the seal's row included. Everything the seal row
    /// *says* is in the text column, and that column is clean or this fails.
    #[test]
    fn no_cell_in_the_deck_paints_its_own_background() {
        for (what, a) in [
            ("cold", app()),
            ("playing", playing()),
            ("sealed", sealed()),
            (
                "a halfblock cover",
                with_cover(art::Protocol::Halfblock, 100, 30),
            ),
            ("the visualiser", visualising(100, 30)),
            (
                "the visualiser over a cover",
                visualising_with_cover(art::Protocol::Halfblock, 100, 30),
            ),
        ] {
            // The two fixtures with a cover encoded it for one grid; at any
            // other size the renderer correctly falls back to the placeholder,
            // which is not what this test is about.
            let sizes: &[(u16, u16)] = if what.contains("cover") {
                &[(100, 30)]
            } else {
                &[(80, 20), (100, 30), (240, 80)]
            };
            for &(w, h) in sizes {
                let exempt = paints_its_own_pixels(&a, w, h);

                // ── the positive half ────────────────────────────────────
                //
                // The footer's text column, on every one of its rows. Nothing
                // may be exempted here — not the cover, not the backdrop, not
                // whatever comes next.
                let text_x = 1 + GUTTER + footer::ART_WIDTH + footer::ART_GAP;
                let seal_column = Rect {
                    x: text_x,
                    y: h - 1 - FOOTER_HEIGHT,
                    width: w - 1 - text_x,
                    height: FOOTER_HEIGHT,
                };
                for r in &exempt {
                    assert!(
                        r.right() <= seal_column.x
                            || r.bottom() <= seal_column.y
                            || seal_column.right() <= r.x
                            || seal_column.bottom() <= r.y,
                        "{what} at {w}×{h}: {r:?} reaches the footer's text column"
                    );
                }

                // ── the size half ────────────────────────────────────────
                //
                // The exemption is the cover's own cells. Nothing may be
                // forgiven a background because it is *near* a picture.
                let exempt_cells: u32 = exempt.iter().map(|r| r.area()).sum();
                let body_cells = u32::from(w - 2) * u32::from(h - 2 - 1 - FOOTER_HEIGHT);
                assert!(
                    exempt_cells * 2 <= body_cells,
                    "{what} at {w}×{h}: {exempt_cells} exempt cells is most of a \
                     {body_cells}-cell body — that is a wash, not a picture"
                );

                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal.draw(|f| draw(f, &a)).unwrap();
                let buf = terminal.backend().buffer();

                for y in seal_column.y..seal_column.bottom() {
                    for x in seal_column.x..seal_column.right() {
                        assert_eq!(
                            buf[(x, y)].bg,
                            Color::Reset,
                            "{what} at {w}×{h}: the footer's text column was painted at ({x}, {y})"
                        );
                    }
                }

                // ── the negative half ────────────────────────────────────
                for y in 0..h {
                    for x in 0..w {
                        if exempt
                            .iter()
                            .any(|r| r.x <= x && x < r.right() && r.y <= y && y < r.bottom())
                        {
                            continue;
                        }
                        assert_eq!(
                            buf[(x, y)].bg,
                            Color::Reset,
                            "{what} at {w}×{h}: cell ({x}, {y}) painted its own background"
                        );
                    }
                }
            }
        }
    }

    /// The rects allowed to paint a background, for this state and this size.
    ///
    /// **Derived from the layout functions, never hand-written**, so an
    /// exemption cannot quietly stop matching what is drawn: if the cover moves
    /// and this does not follow, the negative half above fails on the cells it
    /// left behind.
    fn paints_its_own_pixels(app: &App, w: u16, h: u16) -> Vec<Rect> {
        let term = Rect::new(0, 0, w, h);
        if app.visualiser {
            // **The cover block, and nothing else.** This used to be the whole
            // body, because the ambient backdrop washed every cell of it. The
            // backdrop is gone and the `z` view now paints backgrounds in
            // exactly the same place every other view does: inside the picture.
            return visualiser::cover_area(term).into_iter().collect();
        }
        art_area(term).into_iter().collect()
    }

    /// And the footer's ink is the *same* ink the panes above it use, so the
    /// two surfaces cannot drift apart in some future edit either.
    #[test]
    fn the_footer_and_the_main_pane_draw_on_the_same_ground() {
        let a = sealed();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        // A cell in the track list, a cell in the sidebar, a cell in the
        // footer's seal row.
        let main = &buf[(20, 5)];
        let side = &buf[(3, 3)];
        let foot = &buf[(20, 28)];
        assert_eq!(main.bg, side.bg);
        assert_eq!(main.bg, foot.bg);
    }

    #[test]
    fn a_status_note_replaces_the_signal_path_row_without_hiding_the_seal() {
        let mut a = app();
        a.set_status("config.toml is not valid TOML");
        let rows = render(100, 30, &a);
        let seal_row = &rows[28];
        assert!(seal_row.contains("not valid TOML"), "{seal_row}");
        assert!(seal_row.contains("IDLE"), "{seal_row}");
    }

    /// **The nav fix, against the real machine.**
    ///
    /// Everything above uses a hand-built library, which is exactly the blind
    /// spot the bug lived in: every list test passed while a first run listed
    /// nothing, because nothing ever resolved a folder to scan. This does what
    /// `main` does on a machine with no `~/.config/eko/config.toml` at all —
    /// resolve, scan, then walk the cursor with `j` and `k` — and prints the
    /// frame it ends up with.
    ///
    /// `#[ignore]`d because it reads whatever is really in the user's music
    /// folder, so its result depends on the machine. Run it deliberately:
    ///
    /// ```text
    /// cargo test -p eko-cli --bin eko-cli -- --ignored --nocapture no_config_file
    /// ```
    #[test]
    #[ignore = "scans the machine's real music folder; run it deliberately"]
    fn with_no_config_file_the_real_music_folder_scans_and_the_cursor_moves() {
        use crate::app::Focus;
        use crate::config::resolve_music_folder;
        use std::time::Duration;

        // Precisely `main`'s first three lines, minus the terminal takeover.
        let config = Config::default();
        assert!(
            config.music_folder.is_none(),
            "the fixture must be an unconfigured Deck"
        );
        let folder = resolve_music_folder(&config);
        println!("resolved: {folder:?}");
        let root = folder
            .path()
            .expect("no platform music folder on this machine")
            .to_path_buf();

        let mut a = App::new(
            &config,
            Theme::new(Accent::Orange, ColorDepth::TrueColor),
            folder,
        );
        let (tx, rx) = std::sync::mpsc::channel();
        a.attach(tx);
        assert!(a.scan.is_running(), "attach did not start a scan");
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(120)) {
            a.handle(event);
            if !a.scan.is_running() {
                break;
            }
        }
        assert_eq!(a.scan, ScanState::Ready, "the scan never finished");

        println!(
            "\n{} · {} albums · {} tracks",
            root.display(),
            a.library.albums.len(),
            a.library.track_count()
        );
        for album in &a.library.albums {
            println!(
                "  {} — {} ({} tracks)",
                album.artist,
                album.name,
                album.tracks.len()
            );
        }
        assert!(!a.library.is_empty(), "no music in {}", root.display());

        // ── the bug, directly ────────────────────────────────────────────
        // `move_cursor` was never wrong; it was correctly refusing to move a
        // cursor over zero rows. Give it rows and it walks.
        assert_eq!(a.focus, Focus::Main);
        assert_eq!(a.cursor(), 0);
        let rows = a.row_count();
        let walked: Vec<usize> = (1..rows)
            .map(|_| {
                a.act(crate::keys::Action::Down);
                a.cursor()
            })
            .collect();
        assert_eq!(
            walked,
            (1..rows).collect::<Vec<_>>(),
            "j did not walk the album list"
        );
        a.act(crate::keys::Action::Down);
        assert_eq!(a.cursor(), 0, "j did not wrap at the end");
        a.act(crate::keys::Action::Up);
        assert_eq!(a.cursor(), rows - 1, "k did not wrap backwards");
        a.act(crate::keys::Action::Up);
        assert_eq!(a.cursor(), rows.saturating_sub(2), "k did not move");

        a.album_cursor = 0;
        println!("\nalbum list:\n{}\n", render(100, 30, &a).join("\n"));
        a.act(crate::keys::Action::Open);
        assert_eq!(a.view, View::Album(0), "enter did not open an album");
        println!("album 0 open:\n{}\n", render(100, 30, &a).join("\n"));
    }

    // ── real data ────────────────────────────────────────────────────────

    /// Snapshot of the album list at 100×30, with a scanned library in it.
    #[test]
    fn deck_at_100x30_with_a_library() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────── Music · 2 albums · 4 tracks ┐",
            "│ SOURCES        │ LOCAL · Music                                                                   │",
            "│                │ 2 albums · 4 tracks                                                             │",
            "│ ● Local      2 │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ + Add server   │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ○ Queue        │ ▌     Aphex Twin — Selected Ambien…  3     1  Xtal                         4:51 │",
            "│                │       Boards of Canada — Music Has…  1     2  Tha                          9:09 │",
            "│                │                                            3  Pulsewidth                   3:52 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ⏹ Nothing playing                                     ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁ │",
            "│ ░░░░░░░░  —                                                                                      │",
            "│ ░░░░░░░░  00:00 ────────────────────────────────────────────────────────────────────────── 00:00 │",
            "│ ░░░░░░░░  — → —                                                      ○ IDLE    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &scanned()), expected);
    }

    /// Snapshot of an open album with a track playing — the frame Task 3 set
    /// out to produce.
    ///
    /// Note the seal: `○ UNVERIFIED`, amber, on a Deck that is unmistakably
    /// playing. See [`super::footer::signal_path`].
    #[test]
    fn deck_at_100x30_playing_a_track() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────── Music · 2 albums · 4 tracks ┐",
            "│ SOURCES        │ ‹ SELECTED AMBIENT WORKS 85-92                                                  │",
            "│                │ Aphex Twin · 3 tracks                                                           │",
            "│ ● Local      2 │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ + Add server   │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ● Queue      3 │ ▌  ▶  Aphex Twin — Selected Ambien…  3     1  Xtal                         4:51 │",
            "│                │       Boards of Canada — Music Has…  1  ▌  ▶  Tha                          9:09 │",
            "│                │                                            3  Pulsewidth                   3:52 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ Tha                                                 ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Aphex Twin · Selected Ambient Works 85-92                                       2 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:09 │",
            "│ ░░░░░░░░  — → —                                                ○ UNVERIFIED    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &playing()), expected);
    }

    /// The scanned library, album 0 open, track 1 playing 2:04 in.
    ///
    /// Started through [`App::play`] rather than by writing [`App::now`] by
    /// hand, so the fixture is a Deck that really is playing: the queue is the
    /// album, and the sidebar's queue count is the one the transport would
    /// actually have. A hand-set `now` beside an empty queue is a state the
    /// application cannot reach, and a snapshot of one would pin a frame nobody
    /// can see. The track's path is unopenable, so the engine session dies in
    /// `open_source` without ever asking for an output device.
    fn playing() -> App {
        let mut a = scanned();
        a.view = View::Album(0);
        a.track_cursor = 1;
        a.play(Source::Local, 0, 1);
        a.pos_ms = 124_000;
        // A fixed ramp so the frame is deterministic; the real ones come from
        // `Engine::bands`.
        a.bands = (0..32).map(|i| (i as f32) / 31.0).collect();
        a
    }

    #[test]
    fn the_album_list_shows_artists_names_and_track_counts() {
        let rows = render(100, 30, &scanned());
        assert!(rows[1].contains("LOCAL · Music"), "{}", rows[1]);
        assert!(rows[2].contains("2 albums · 4 tracks"), "{}", rows[2]);
        // At 100 wide the album column is thirty-eight cells, so this name does
        // not fit — and says so, rather than stopping mid-word.
        assert!(
            rows[FIRST_LIST_ROW].contains("Aphex Twin — Selected Ambien…"),
            "{}",
            rows[FIRST_LIST_ROW]
        );
        // The track count is right-aligned against the album column's own edge.
        assert!(
            album_column(&rows[FIRST_LIST_ROW]).ends_with('3'),
            "{}",
            rows[FIRST_LIST_ROW]
        );
        assert!(
            rows[FIRST_LIST_ROW + 1].contains("Boards of Canada"),
            "{}",
            rows[FIRST_LIST_ROW + 1]
        );
        // Given room, the whole name is there.
        let wide = render(240, 80, &scanned());
        assert!(
            wide[FIRST_LIST_ROW].contains("Aphex Twin — Selected Ambient Works 85-92"),
            "{}",
            wide[FIRST_LIST_ROW]
        );
    }

    #[test]
    fn an_open_album_shows_its_tracks_with_numbers_and_durations() {
        let mut a = scanned();
        a.view = View::Album(0);
        let rows = render(100, 30, &a);
        assert!(
            rows[1].contains("SELECTED AMBIENT WORKS 85-92"),
            "{}",
            rows[1]
        );
        assert!(rows[2].contains("Aphex Twin · 3 tracks"), "{}", rows[2]);
        let (one, two) = (&rows[FIRST_LIST_ROW], &rows[FIRST_LIST_ROW + 1]);
        assert!(one.contains(" 1  Xtal"), "{one}");
        assert!(one.contains("4:51"), "{one}");
        assert!(two.contains(" 2  Tha"), "{two}");
        assert!(two.contains("9:09"), "{two}");
    }

    #[test]
    fn an_empty_scan_result_reads_as_empty_rather_than_broken() {
        let mut a = app();
        a.scan = ScanState::Ready;
        a.library = Library {
            root: Some(PathBuf::from("/Users/rod/Silence")),
            albums: Vec::new(),
        };
        let rows = render(100, 30, &a);
        assert!(rows[2].contains("no audio files found"), "{}", rows[2]);
        // Not an error, so nothing is shouting in the footer.
        assert!(rows[28].contains("— → —"), "{}", rows[28]);
    }

    #[test]
    fn a_running_scan_reports_progress_in_the_main_pane() {
        let mut a = app();
        a.scan = ScanState::Discovering { found: 1_204 };
        assert!(render(100, 30, &a)[2].contains("scanning · 1204 files found"));
        a.scan = ScanState::Reading {
            done: 812,
            total: 1_204,
        };
        assert!(render(100, 30, &a)[2].contains("reading tags · 812 of 1204"));
    }

    #[test]
    fn the_footer_names_the_track_that_is_playing() {
        let rows = render(100, 30, &playing());
        // The transport glyph is at the head of the title now, not on the
        // scrubber row — see the header of `crate::ui::footer`.
        assert!(rows[25].contains("▶ Tha"), "{}", rows[25]);
        assert!(
            rows[26].contains("Aphex Twin · Selected Ambient Works 85-92"),
            "{}",
            rows[26]
        );
        assert!(rows[27].contains("02:04"), "{}", rows[27]);
        assert!(rows[27].contains("09:09"), "{}", rows[27]);
    }

    // ── the seal ─────────────────────────────────────────────────────────

    /// **The load-bearing test.** Starting playback must not seal the stream.
    ///
    /// Before Task 3 the footer mapped every non-`Stopped` transport state to a
    /// green `● BIT-PERFECT`. Nothing could construct `Playback::Playing`, so
    /// it was inert — and Task 3, which constructs it, would have turned the
    /// seal into a lamp that lights for every track the player touches.
    ///
    /// This drives the real play path: a cursor on a track, `Enter`, then the
    /// frame. The claim it forbids is the *unverified* one. In Task 4 the label
    /// comes from `eko_core::signal_path::derive`, which reports
    /// `active: false` for exactly this state — an app with no stream info —
    /// so this assertion holds there too.
    #[test]
    fn starting_playback_never_claims_bit_perfect() {
        let mut a = scanned();
        a.act(crate::keys::Action::Open); // open album 0
        a.act(crate::keys::Action::Open); // play track 0
        assert_eq!(a.playback, Playback::Playing, "the fixture must be playing");

        for rows in [render(100, 30, &a), render(80, 20, &a), render(240, 80, &a)] {
            for row in &rows {
                assert!(
                    !row.contains("BIT-PERFECT"),
                    "playback alone sealed the stream: {row}"
                );
            }
        }
    }

    /// No transport state may reach the green lamp on its own.
    #[test]
    fn no_transport_state_by_itself_reaches_the_seal() {
        for playback in [Playback::Stopped, Playback::Paused, Playback::Playing] {
            let mut a = playing();
            a.playback = playback;
            let seal = render(100, 30, &a).swap_remove(28);
            assert!(!seal.contains("BIT-PERFECT"), "{playback:?}: {seal}");
        }
    }

    /// A live transport the engine has not described yet is `○ UNVERIFIED`.
    ///
    /// This replaces Task 3's provisional-lamp test. The label survives, but it
    /// now means something narrower and it is reached a different way: not
    /// "playing, and this crate cannot derive a seal", but "playing, and
    /// `derive` returned `active: false` because there is no stream to read".
    /// A hollow amber lamp is the honest rendering of that — obviously not a
    /// claim, and impossible to mistake for the green one.
    #[test]
    fn a_live_transport_the_engine_has_not_described_shows_no_seal() {
        for playback in [Playback::Playing, Playback::Paused] {
            let mut a = playing();
            a.playback = playback;
            assert!(a.stream.is_none(), "the fixture must have no stream info");
            assert!(!a.seal().active, "derive must report an inactive seal");
            let seal = render(100, 30, &a).swap_remove(28);
            assert!(seal.contains("○ UNVERIFIED"), "{playback:?}: {seal}");
            assert!(!seal.contains('●'), "{playback:?}: {seal}");
        }
    }

    // ── the real seal ────────────────────────────────────────────────────

    /// A stream the engine has fully described. 44.1 kHz FLAC, out at the same
    /// rate, on a device running at the same rate: nothing touches the samples.
    fn stream(src_rate: u32, rate: u32, dev_rate: u32) -> StreamInfo {
        StreamInfo {
            rate,
            src_rate,
            dev_rate,
            bits: 24,
            codec: "flac".into(),
            device: "Topping E30".into(),
        }
    }

    /// [`playing`], with the engine reporting an unmodified 44.1 kHz path.
    fn sealed() -> App {
        let mut a = playing();
        a.stream = Some(stream(44_100, 44_100, 44_100));
        a
    }

    /// The seal row, as text, off a terminal `w` columns wide.
    fn seal_row_at(w: u16, a: &App) -> String {
        render(w, 30, a).swap_remove(28)
    }

    /// The seal row at the size most of this file works in.
    ///
    /// A hundred columns is *not* wide enough for every seal the client can
    /// reach — a resampled, attenuated path has an eighteen-character label, and
    /// the chain beside it is cut to fit. That is the row behaving correctly,
    /// and it is why the test that compares the row against `derive`'s own
    /// strings asks for a wider one. See
    /// [`the_rendered_seal_is_the_label_derive_returned`].
    fn seal_row(a: &App) -> String {
        seal_row_at(100, a)
    }

    /// Snapshot: a fully described, unmodified stream. **The green lamp.**
    ///
    /// This is the frame the string `BIT-PERFECT` is here for, and it got there
    /// from `derive`, not from this crate — see
    /// [`the_rendered_seal_is_the_label_derive_returned`].
    ///
    /// It was once the *only* frame in the suite containing that string, and it
    /// is not any more: the real-file seal, the device picker and the help
    /// overlay all snapshot a green lamp too. So grepping for it finds several
    /// frames rather than this one — and every one of them got the label the
    /// same way, from `derive`.
    #[test]
    fn deck_at_100x30_sealed_bit_perfect() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────── Music · 2 albums · 4 tracks ┐",
            "│ SOURCES        │ ‹ SELECTED AMBIENT WORKS 85-92                                                  │",
            "│                │ Aphex Twin · 3 tracks                                                           │",
            "│ ● Local      2 │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ + Add server   │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ● Queue      3 │ ▌  ▶  Aphex Twin — Selected Ambien…  3     1  Xtal                         4:51 │",
            "│                │       Boards of Canada — Music Has…  1  ▌  ▶  Tha                          9:09 │",
            "│                │                                            3  Pulsewidth                   3:52 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ Tha                                                 ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Aphex Twin · Selected Ambient Works 85-92                                       2 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:09 │",
            "│ ░░░░░░░░  FLAC · 44.1 kHz · 24-bit → Topping E30 · 44.1 kHz   ● BIT-PERFECT    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &sealed()), expected);
    }

    /// The rendered label is `derive`'s, character for character, across every
    /// combination this client can reach.
    ///
    /// This is the assertion the whole task turns on: a correct derivation
    /// rendered wrongly is still a lying seal. It also pins the negative — the
    /// row says `BIT-PERFECT` **only** when `derive` said `pure`.
    /// The states the seal can reach in this client, as
    /// `(src_rate, output rate, device rate, what)`.
    ///
    /// Shared by the text assertion and the colour assertion so the two cannot
    /// cover different ground.
    ///
    /// It used to carry a volume column, and three of its seven rows were
    /// attenuated ones. None of them is reachable now: there is no key, no
    /// config key and no meter, and `App::volume` is pinned at unity — so a
    /// `VOLUME` case here would be a case for a state this client cannot
    /// produce, which is a test of `eko_core` written in the wrong crate. See
    /// `crate::app::tests::nothing_a_user_can_press_takes_the_seal_off_unity`.
    fn seal_cases() -> [(u32, u32, u32, &'static str); 4] {
        [
            (44_100, 44_100, 44_100, "unity, no resample"),
            (96_000, 48_000, 48_000, "engine resample"),
            (44_100, 44_100, 48_000, "OS resample"),
            (96_000, 96_000, 96_000, "unity at 96 kHz"),
        ]
    }

    #[test]
    fn the_rendered_seal_is_the_label_derive_returned() {
        for (src, rate, dev, what) in seal_cases() {
            let mut a = sealed();
            a.stream = Some(stream(src, rate, dev));

            let derived = a.seal();
            // Wide enough that `fit` has nothing to cut: this asserts the row
            // carries `derive`'s strings, and a truncated chain would be a
            // different, weaker claim about the same code.
            let row = seal_row_at(120, &a);
            assert!(
                derived.active,
                "{what}: the fixture must derive an active seal"
            );
            assert!(
                row.contains(&derived.seal_label),
                "{what}: the row does not carry derive's label {:?}: {row}",
                derived.seal_label
            );
            assert_eq!(
                row.contains("BIT-PERFECT"),
                derived.pure,
                "{what}: the rendered seal and `pure` disagree: {row}"
            );
            // The chain is `derive`'s too, not a re-format of the status.
            assert!(row.contains(&derived.src), "{what}: {row}");
            assert!(row.contains(&derived.output), "{what}: {row}");
        }
    }

    /// **The lamp is green if and only if `derive` said `pure`.**
    ///
    /// The companion to [`the_rendered_seal_is_the_label_derive_returned`], which
    /// can only see text. Colour is what a user actually reads at a glance —
    /// `● BIT-PERFECT` and `● VOLUME` share a glyph, so if the impure branch
    /// were painted green the two would be indistinguishable on screen and every
    /// text assertion in this file would still pass. It was mutation-tested that
    /// way: replacing the impure branch's red/amber choice with `led_green` left
    /// all 121 other tests green.
    ///
    /// The assertion is an `==` against `pure`, not two one-sided checks, so it
    /// catches an over-claim (green when impure) and an under-claim (not green
    /// when pure) alike.
    #[test]
    fn the_lamp_is_green_if_and_only_if_derive_said_pure() {
        let theme = Theme::new(Accent::Orange, ColorDepth::TrueColor);

        for (src, rate, dev, what) in seal_cases() {
            let mut a = sealed();
            a.stream = Some(stream(src, rate, dev));

            let derived = a.seal();
            let (glyph, colour) = lamp(100, 30, &a);
            assert_eq!(glyph, "●", "{what}: an active seal wants a filled lamp");
            assert_eq!(
                colour == theme.led_green,
                derived.pure,
                "{what}: lamp colour and `pure` disagree (colour {colour:?}, pure {})",
                derived.pure
            );
            if !derived.pure {
                // And it is one of the two downgrade colours, not some third
                // thing that merely happens not to be green.
                assert!(
                    colour == theme.led_red || colour == theme.led_amber,
                    "{what}: impure lamp is neither red nor amber: {colour:?}"
                );
                assert_eq!(
                    colour == theme.led_red,
                    derived.flags.resampled || derived.flags.os_resampled,
                    "{what}: red is for a rate change and nothing else"
                );
            }
        }

        // The two lamps that make no claim at all are hollow, and neither is
        // green: a seal with nothing behind it must never look like a pass.
        let mut idle = sealed();
        idle.playback = Playback::Stopped;
        assert_eq!(lamp(100, 30, &idle), ("○".into(), theme.ink_faint));

        let unverified = playing(); // playing, but no stream reported
        assert!(!unverified.seal().active);
        assert_eq!(lamp(100, 30, &unverified), ("○".into(), theme.led_amber));
    }

    #[test]
    fn a_resampled_stream_renders_resampled() {
        let mut a = sealed();
        a.stream = Some(stream(96_000, 48_000, 48_000));
        let row = seal_row(&a);
        assert!(row.contains("● RESAMPLED"), "{row}");
        assert!(!row.contains("BIT-PERFECT"), "{row}");
        // And the chain names both ends of the conversion.
        assert!(row.contains("96 kHz"), "{row}");
        assert!(row.contains("48 kHz"), "{row}");
    }

    /// A long device name eats the chain, never the seal.
    #[test]
    fn nothing_on_the_left_can_push_the_seal_off_the_row() {
        let mut a = sealed();
        a.stream = Some(StreamInfo {
            device: "Some Absurdly Over-Named USB Digital Audio Interface Mk II".into(),
            ..stream(44_100, 44_100, 44_100)
        });
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let rows = render(w, h, &a);
            let row = &rows[rows.len() - 2];
            assert!(row.contains("● BIT-PERFECT"), "seal lost at {w}×{h}: {row}");
        }
        // At 100 columns the chain is the part that gets cut.
        assert!(seal_row(&a).contains('…'), "{}", seal_row(&a));
    }

    /// **The seal row's rule, applied to the two rows above it.**
    ///
    /// The right group is laid out first and the left is truncated into what is
    /// left, on every row of the footer — so no track title takes the spectrum
    /// with it, no album name takes the queue position, and nothing at all
    /// reaches the seal. The scrubber is fixed-width by construction and cannot
    /// be pushed by anything.
    ///
    /// Before this, the title and artist rows had no such rule: they were laid
    /// out at full length and clipped by the rect, so an overlong name ran into
    /// the frame with nothing to say it had been cut.
    #[test]
    fn nothing_a_track_is_called_can_push_anything_off_the_footer() {
        let mut a = sealed();
        {
            let now = a.now.as_mut().expect("the fixture must be playing");
            now.title = "A Title Of Quite Preposterous And Frankly Self-Indulgent Length".into();
            now.artist = "An Artist With An Equally Unreasonable Name Attached To Them".into();
            now.album_name = "On An Album Named At Greater Length Than Either Of Those".into();
        }
        a.stream = Some(StreamInfo {
            device: "Some Absurdly Over-Named USB Digital Audio Interface Mk II".into(),
            ..stream(44_100, 44_100, 44_100)
        });

        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let rows = render(w, h, &a);
            let n = rows.len();
            let (title, artist, scrubber, seal) =
                (&rows[n - 5], &rows[n - 4], &rows[n - 3], &rows[n - 2]);
            // Each row's right-hand group survived…
            assert!(title.contains('█'), "spectrum lost at {w}×{h}: {title}");
            assert!(
                artist.contains("2 of 3"),
                "position lost at {w}×{h}: {artist}"
            );
            assert!(
                scrubber.contains("09:09"),
                "clock lost at {w}×{h}: {scrubber}"
            );
            assert!(
                seal.contains("● BIT-PERFECT"),
                "seal lost at {w}×{h}: {seal}"
            );
            // …and the left of each gave way inside the pane, saying so.
            for row in [title, artist] {
                assert!(row.ends_with(" │"), "ran into the frame at {w}×{h}: {row}");
                // 240 columns is room for all of it; the other two are not, and
                // there the cut is marked.
                assert_eq!(
                    row.contains('…'),
                    w < 240,
                    "the cut was not marked at {w}×{h}: {row}"
                );
            }
        }
    }

    /// An error note and a live seal share the row; the note is what shortens.
    #[test]
    fn a_status_note_shortens_rather_than_hiding_the_seal() {
        let mut a = sealed();
        a.set_status(
            "a very long error message about a music folder that could not be read at all",
        );
        let row = seal_row(&a);
        assert!(row.contains("● BIT-PERFECT"), "{row}");
        assert!(row.contains("a very long error"), "{row}");
        assert!(row.contains('…'), "{row}");
    }

    // ── the seal, against a real file and a real output device ───────────

    /// Play a real track out of `~/Music` and print the seal the engine earns.
    ///
    /// `#[ignore]`d for the same reason as `tests/playback.rs`: it claims the
    /// machine's default output device, so it must not run in a normal
    /// `cargo test` or in CI. Run it deliberately:
    ///
    /// ```text
    /// cargo test -p eko-cli --bin eko-cli -- --ignored --nocapture real_track
    /// ```
    ///
    /// It is **silent** — the volume is dropped to zero before the second
    /// reading, and the first reading only needs the engine's rates, not its
    /// output. What it proves is the one thing a unit test with a hand-built
    /// `StreamInfo` cannot: that the rates a real decoder and a real CoreAudio
    /// device report reach `derive` intact, and that the frame on screen says
    /// what `derive` returned.
    #[test]
    #[ignore = "claims the default audio output device; run it deliberately"]
    fn the_seal_on_a_real_track_from_the_music_folder() {
        use crate::app::AppEvent;
        use std::time::{Duration, Instant};

        let root = dirs::home_dir().expect("a home directory").join("Music");
        let mut a = scanned();
        a.library = match crate::library::scan(&root, &|_| true) {
            crate::library::ScanEvent::Finished(lib) => *lib,
            other => panic!("could not scan {}: {other:?}", root.display()),
        };
        assert!(!a.library.is_empty(), "no music in {}", root.display());
        let item = a.library.albums[0].tracks[0].clone();
        println!("track: {}", item.path);

        a.view = View::Album(0);
        a.play(Source::Local, 0, 0);
        assert!(
            a.stream.is_none(),
            "a fresh session has nothing to seal yet"
        );

        // Poll exactly as the event loop does until the engine has described the
        // stream, or give up.
        let start = Instant::now();
        while a.stream.is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
            a.handle(AppEvent::Tick);
        }
        let info = a
            .stream
            .clone()
            .expect("the engine never described the stream");
        // Let it run a moment so the clock, the scrubber and the spectrum are
        // showing real playback rather than the first frame of it.
        let until = Instant::now() + Duration::from_millis(1_800);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(33));
            a.handle(AppEvent::Tick);
        }
        println!(
            "stream: codec={} src_rate={} rate={} dev_rate={} bits={} device={:?}",
            info.codec, info.src_rate, info.rate, info.dev_rate, info.bits, info.device
        );

        // ── 1. unity volume, flat EQ ─────────────────────────────────────
        let unity = a.seal();
        println!(
            "\n1. UNITY  active={} pure={} seal_label={:?} engine_label={:?}\n   flags={:?}\n   src={:?} output={:?} rg_label={:?}",
            unity.active,
            unity.pure,
            unity.seal_label,
            unity.engine_label,
            unity.flags,
            unity.src,
            unity.output,
            unity.rg_label
        );
        let frame = render(100, 30, &a);
        println!("\n{}\n", frame.join("\n"));
        assert!(unity.active, "the seal must be active with a live stream");
        let row = frame[28].clone();
        assert!(
            row.contains(&unity.seal_label),
            "the rendered row does not carry derive's label: {row}"
        );
        assert_eq!(
            row.contains("BIT-PERFECT"),
            unity.pure,
            "the rendered seal and `pure` disagree: {row}"
        );

        // There is no "── 2. below unity ──" section here any more. It pressed
        // `-` and watched the green lamp go, which was the correct rendering of a
        // feature that should not have existed: on a bit-perfect player the very
        // first press took the seal off bit-perfect.

        a.act(crate::keys::Action::PlayPause);
    }

    // ── streaming ────────────────────────────────────────────────────────

    /// [`remote_album_open`], with its first track streaming and the engine
    /// reporting an unmodified 44.1 kHz path.
    ///
    /// Started through [`App::play`] for the reason [`playing`] is: a hand-set
    /// `now` beside an empty queue is a state the app cannot reach, and a
    /// snapshot of one pins a frame nobody can see. The client points at port 1
    /// on the loopback — reserved and refused instantly — so the URL is minted
    /// and signed exactly as it would be, and the engine session dies before it
    /// asks for an output device.
    fn remote_playing() -> App {
        let mut a = remote_album_open();
        a.connection = Some(crate::server::Connection {
            name: "home".into(),
            username: "rod".into(),
            client: std::sync::Arc::new(
                eko_net::Client::new(eko_net::Config {
                    base_url: "http://127.0.0.1:1".into(),
                    username: "rod".into(),
                    password: "hunter2".into(),
                })
                .unwrap(),
            ),
        });
        a.play(Source::Remote, 0, 0);
        a.pos_ms = 124_000;
        a.bands = (0..32).map(|i| (i as f32) / 31.0).collect();
        a.stream = Some(stream(44_100, 44_100, 44_100));
        a
    }

    /// Snapshot: **a track streaming from the server, sealed.** The deliverable
    /// of Task 3.
    ///
    /// Identical chrome to a local track playing — one keymap, one footer, one
    /// seal — and the `▶` is on the server's track list, which it could not be
    /// before this task because nothing remote could play. The green lamp came
    /// from `derive`, out of the rates the engine reported for the stream; see
    /// [`the_rendered_seal_is_the_label_derive_returned`] and
    /// `tests/streaming.rs` for the same claim measured against a real HTTP
    /// server and a real output device.
    #[test]
    fn deck_at_100x30_streaming_a_remote_track() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────────────────── home · 3 albums ┐",
            "│ SOURCES        │ ‹ SPIRIT OF EDEN                                                                │",
            "│                │ Talk Talk · 3 tracks                                                            │",
            "│ ○ Local        │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ ● home       3 │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ○ Search       │ ▌  ▶  Talk Talk — Spirit of Eden     3  ▌  ▶  The Rainbow                  9:15 │",
            "│ ● Queue      3 │       Talk Talk — Laughing Stock     6     2  Eden                         6:26 │",
            "│                │       My Bloody Valentine — Lovel…  11     3  Desire                       6:58 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ The Rainbow                                         ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Talk Talk · Spirit of Eden                                                      1 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:15 │",
            "│ ░░░░░░░░  FLAC · 44.1 kHz · 24-bit → Topping E30 · 44.1 kHz   ● BIT-PERFECT    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",

        ];
        assert_eq!(render(100, 30, &remote_playing()), expected);
    }

    /// Snapshot: **the same stream, resampled.** The frame that must exist.
    ///
    /// Byte-for-byte the frame above except the chain and the seal. A streamed
    /// track the device resamples showing `BIT-PERFECT` is the single worst
    /// outcome available in this codebase, and the streaming path reaches the
    /// seal through exactly the same `stream_info` gate and the same `derive` the
    /// local path does — this pins that it does.
    #[test]
    fn deck_at_100x30_streaming_a_remote_track_that_is_resampled() {
        let mut a = remote_playing();
        a.stream = Some(stream(96_000, 48_000, 48_000));
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────────────────── home · 3 albums ┐",
            "│ SOURCES        │ ‹ SPIRIT OF EDEN                                                                │",
            "│                │ Talk Talk · 3 tracks                                                            │",
            "│ ○ Local        │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ ● home       3 │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ○ Search       │ ▌  ▶  Talk Talk — Spirit of Eden     3  ▌  ▶  The Rainbow                  9:15 │",
            "│ ● Queue      3 │       Talk Talk — Laughing Stock     6     2  Eden                         6:26 │",
            "│                │       My Bloody Valentine — Lovel…  11     3  Desire                       6:58 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ The Rainbow                                         ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Talk Talk · Spirit of Eden                                                      1 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:15 │",
            "│ ░░░░░░░░  FLAC · 96 kHz · 24-bit → Topping E30 · 48 kHz         ● RESAMPLED    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",

        ];
        assert_eq!(render(100, 30, &a), expected);
        // …and the lamp is red, not green — the thing a user reads at a glance.
        let theme = Theme::new(Accent::Orange, ColorDepth::TrueColor);
        assert_eq!(lamp(100, 30, &a), ("●".into(), theme.led_red));
    }

    /// **A stream reaches the seal through the same gate a file does.**
    ///
    /// Every combination the client can derive, asserted on a *remote* fixture,
    /// so nothing about the URL path can arrive at a different label than the
    /// file path would for identical rates.
    #[test]
    fn a_streamed_track_seals_exactly_as_a_local_one_with_the_same_rates() {
        for (src, rate, dev, what) in seal_cases() {
            let mut remote = remote_playing();
            remote.stream = Some(stream(src, rate, dev));

            let mut local = sealed();
            local.stream = Some(stream(src, rate, dev));

            let (r, l) = (remote.seal(), local.seal());
            assert_eq!((r.active, r.pure), (l.active, l.pure), "{what}");
            assert_eq!(r.seal_label, l.seal_label, "{what}");
            assert_eq!(r.src, l.src, "{what}");
            assert_eq!(r.output, l.output, "{what}");
            assert_eq!(
                seal_row(&remote).contains("BIT-PERFECT"),
                r.pure,
                "{what}: the streamed seal and `pure` disagree"
            );
        }
    }

    /// **A stream that never opened claims nothing at all.**
    ///
    /// The engine writes no rate when the open fails, so `stream_info` refuses
    /// the status and `derive` reports `active: false`. Note that `derive` also
    /// reports `pure: true` for that state — every flag is false because there is
    /// no stream to raise one — so a footer that read `pure` on its own would
    /// paint the green lamp over a 404. It reads `active` first; this is the
    /// rendered proof, and it is the desktop app's bug on this exact path.
    #[test]
    fn a_remote_stream_that_never_opened_renders_no_claim() {
        let theme = Theme::new(Accent::Orange, ColorDepth::TrueColor);
        let mut a = remote_playing();
        a.stream = None;
        a.playback = Playback::Stopped;
        a.set_status("could not play The Rainbow · the server sent nothing playable");

        assert!(a.seal().pure, "the premise: `pure` alone is not the guard");
        assert!(!a.seal().active);

        let rows = render(100, 30, &a);
        for row in &rows {
            assert!(
                !row.contains("BIT-PERFECT"),
                "a failed stream sealed: {row}"
            );
        }
        let seal = &rows[28];
        assert!(seal.contains("○ IDLE"), "{seal}");
        assert!(seal.contains("could not play The Rainbow"), "{seal}");
        assert_eq!(lamp(100, 30, &a), ("○".into(), theme.ink_faint));
        // The footer does not name a track it is not playing.
        assert!(rows[25].contains("Nothing playing"), "{}", rows[25]);
        // And nothing on screen is the signed URL that failed.
        for row in &rows {
            for secret in ["hunter2", "&t=", "&s=", "/rest/stream"] {
                assert!(
                    !row.contains(secret),
                    "{secret:?} reached the screen: {row}"
                );
            }
        }
    }

    /// **The `▶` marker is source-aware.**
    ///
    /// `album` and `track` are indices into one of two lists. A remote track
    /// playing at index 0 must not also light local album 0 — a marker on a row
    /// that is not playing is the same class of claim as a seal on audio that is
    /// not bit-perfect, in miniature.
    #[test]
    fn the_playing_marker_lands_only_in_the_library_the_track_came_from() {
        let mut a = remote_playing();
        a.library = scanned().library;
        a.scan = ScanState::Ready;

        // The server's open album has the marker.
        assert!(
            render(100, 30, &a)[FIRST_LIST_ROW].contains('▶'),
            "{}",
            render(100, 30, &a)[FIRST_LIST_ROW]
        );

        // The local album list, with a remote track playing, has none.
        a.selected = 0;
        let rows = render(100, 30, &a);
        assert!(rows[1].contains("LOCAL"), "{}", rows[1]);
        for row in rows.iter().take(24) {
            assert!(
                !row.contains('▶'),
                "a remote track marked a local row: {row}"
            );
        }

        // …and the same the other way round: a local track playing marks nothing
        // on the server's list.
        let mut a = remote_playing();
        a.now = Some(NowPlaying {
            source: Source::Local,
            album: 0,
            track: 0,
            title: "Xtal".into(),
            artist: "Aphex Twin".into(),
            album_name: "Selected Ambient Works 85-92".into(),
            dur_ms: 291_000,
        });
        let rows = render(100, 30, &a);
        for row in rows.iter().take(24) {
            assert!(
                !row.contains('▶'),
                "a local track marked a remote row: {row}"
            );
        }
    }

    /// `secs` of a `freq` Hz sine as a 16-bit stereo WAV at `rate`.
    ///
    /// A twin of the one in `tests/streaming.rs`, and deliberately not shared:
    /// `eko-cli` is a binary, so an integration test cannot reach into it and
    /// this module cannot reach out. Forty lines of WAV header is a cheaper
    /// price than a `lib.rs` that exists only for a fixture.
    #[cfg(test)]
    fn sine_wav(rate: u32, secs: f64, freq: f64) -> Vec<u8> {
        let (channels, bits) = (2u16, 16u16);
        let frames = (f64::from(rate) * secs) as u32;
        let block_align = channels * bits / 8;
        let data_len = frames * u32::from(block_align);

        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let t = f64::from(i) / f64::from(rate);
            let s =
                (0.5 * (2.0 * std::f64::consts::PI * freq * t).sin() * f64::from(i16::MAX)) as i16;
            for _ in 0..channels {
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
        out
    }

    /// **The whole chain, end to end: a mock server to a rendered seal.**
    ///
    /// Everything else in this file states its `StreamInfo`. This one earns it:
    /// a `mockito` server serving a generated WAV, a real `eko_net::Client`
    /// signing a real URL, `App::play` on the remote source, the real engine
    /// downloading and decoding it through a real output device, and the frame
    /// that comes out the other end.
    ///
    /// It is the only test that can catch a break anywhere along that chain —
    /// a `stream://` URL that will not resolve, a rate that never reaches
    /// `derive`, a seal rendered from something other than what `derive`
    /// returned.
    ///
    /// `#[ignore]`d for the same reason as `tests/playback.rs`: it claims the
    /// machine's default output device. Run it deliberately:
    ///
    /// ```text
    /// cargo test -p eko-cli --bin eko-cli -- --ignored --nocapture real_stream
    /// ```
    ///
    /// It is **silent** — the volume is dropped to zero before the engine is
    /// given anything, and the seal is read from the engine's reported rates
    /// rather than from anything audible.
    #[test]
    #[ignore = "claims the default audio output device; run it deliberately"]
    fn the_seal_on_a_real_stream_from_a_mock_server() {
        use crate::app::{AppEvent, ConnState, RemoteState};
        use std::time::{Duration, Instant};

        // ── a server with one streamable track on it ─────────────────────
        let mut server = mockito::Server::new();
        let _stream = server
            .mock("GET", "/rest/stream")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "audio/wav")
            .with_body(sine_wav(44_100, 8.0, 440.0))
            .expect_at_least(1)
            .create();
        let client = std::sync::Arc::new(
            eko_net::Client::new(eko_net::Config {
                base_url: server.url(),
                username: "rod".into(),
                password: "hunter2".into(),
            })
            .unwrap(),
        );

        // ── a Deck browsing it ───────────────────────────────────────────
        let mut a = connecting_to(ConnState::Connected {
            name: "home".into(),
            username: "rod".into(),
        });
        a.connection = Some(crate::server::Connection {
            name: "home".into(),
            username: "rod".into(),
            client,
        });
        a.remote = crate::remote::Library {
            albums: vec![crate::remote::Album {
                id: "al-1".into(),
                name: "Spirit of Eden".into(),
                artist: "Talk Talk".into(),
                song_count: Some(1),
                tracks: Some(vec![crate::remote::Track {
                    id: "tr-1".into(),
                    title: "The Rainbow".into(),
                    artist: "Talk Talk".into(),
                    album: "Spirit of Eden".into(),
                    track_no: Some(1),
                    duration: 8.0,
                }]),
            }],
        };
        a.remote_state = RemoteState::Ready;
        a.sources[1].badge = Some("1".into());
        a.selected = 1;
        a.remote_view = View::Album(0);
        a.context = "home · 1 albums".into();
        a.hush(); // silent

        // ── play it ──────────────────────────────────────────────────────
        a.play(Source::Remote, 0, 0);
        assert_eq!(a.playback, Playback::Playing);
        assert!(a.stream.is_none(), "a fresh stream has nothing to seal yet");

        // Poll exactly as the event loop does until the engine has described it.
        let start = Instant::now();
        while a.stream.is_none() && start.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(50));
            a.handle(AppEvent::Tick);
        }
        let info = a
            .stream
            .clone()
            .expect("the engine never described the stream");
        // Let it run so the clock, the scrubber and the spectrum are real.
        let until = Instant::now() + Duration::from_millis(1_800);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(33));
            a.handle(AppEvent::Tick);
        }
        println!(
            "stream: codec={} src_rate={} rate={} dev_rate={} bits={} device={:?} pos_ms={}",
            info.codec, info.src_rate, info.rate, info.dev_rate, info.bits, info.device, a.pos_ms
        );
        assert!(a.pos_ms > 0, "the playhead never moved on a stream");
        assert_eq!(a.playback, Playback::Playing, "the stream dropped out");

        // ── the seal, at unity ───────────────────────────────────────────
        a.unhush();
        let unity = a.seal();
        println!(
            "\n1. UNITY  active={} pure={} seal_label={:?}\n   flags={:?}\n   src={:?} output={:?}",
            unity.active, unity.pure, unity.seal_label, unity.flags, unity.src, unity.output
        );
        let frame = render(100, 30, &a);
        println!("\n{}\n", frame.join("\n"));
        assert!(unity.active, "a live stream derived no seal");
        let row = frame[28].clone();
        assert!(row.contains(&unity.seal_label), "{row}");
        assert_eq!(
            row.contains("BIT-PERFECT"),
            unity.pure,
            "the rendered seal and `pure` disagree: {row}"
        );
        // Nothing on screen is the signed URL that fetched it.
        for line in &frame {
            for secret in ["hunter2", "&t=", "&s=", "/rest/stream"] {
                assert!(!line.contains(secret), "{secret:?} on screen: {line}");
            }
        }

        // There is no "one step down" section here any more: pressing `-`
        // downgraded a *streamed* seal exactly as it downgraded a local one, and
        // both were the correct rendering of a control this player should never
        // have had.
        a.act(crate::keys::Action::PlayPause);
    }

    // ── the EQ panel ─────────────────────────────────────────────────────

    /// [`sealed`], with the EQ panel open, engaged, and 310 Hz pushed to +5 dB.
    ///
    /// Built by pressing keys rather than by setting fields, so the fixture is a
    /// state the application can actually be put in — the same rule [`playing`]
    /// follows.
    fn eq_engaged() -> App {
        let mut a = sealed();
        a.act(crate::keys::Action::EqPanel);
        for _ in 0..3 {
            a.act(crate::keys::Action::EqNext);
        }
        a.act(crate::keys::Action::EqToggle);
        for _ in 0..5 {
            a.act(crate::keys::Action::Up);
        }
        a
    }

    /// Snapshot: **the EQ panel open over a track whose seal has downgraded to
    /// `EQ`.** The deliverable of this task, and the frame that shows the two
    /// belong to each other — the sliders and the label in the footer are two
    /// renderings of one state.
    ///
    /// Note what is *not* here: `BIT-PERFECT`. The bar at 310 is up, the readout
    /// says `EQ on`, and the seal says `● EQ`. A frame in which the panel showed
    /// a curve and the footer still showed the green lamp is the bug this whole
    /// task was sequenced around.
    #[test]
    fn deck_at_100x30_with_the_eq_panel_open_and_the_seal_downgraded() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────── Music · 2 albums · 4 tracks ┐",
            "│ SOURCES        │ ‹ SELECTED AMBIENT WORKS 85-92                                                  │",
            "│                │ Aphex Twin · 3 tracks                                                           │",
            "│ ● Local      2 │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ + Add server   │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ● Queue      3 │ ▌  ▶  Aphex Twin — Selected Ambien…  3     1  Xtal                         4:51 │",
            "│                │       Boards o┌─ EQ ────────────────────────────────────────────┐          9:09 │",
            "│                │               │ +12  ·   ·   ·   ·   ·   ·   ·   ·   ·   ·   ·  │          3:52 │",
            "│                │               │      ·   ·   ·   ·   ·   ·   ·   ·   ·   ·   ·  │               │",
            "│                │               │      ·   ·   ·  ███  ·   ·   ·   ·   ·   ·   ·  │               │",
            "│                │               │   0 ███ ███ ███ ███ ███ ███ ███ ███ ███ ███ ███ │               │",
            "│                │               │      ·   ·   ·   ·   ·   ·   ·   ·   ·   ·   ·  │               │",
            "│                │               │      ·   ·   ·   ·   ·   ·   ·   ·   ·   ·   ·  │               │",
            "│                │               │ -12  ·   ·   ·   ·   ·   ·   ·   ·   ·   ·   ·  │               │",
            "│                │               │     pre  60 170 310 600  1k  3k  6k 12k 14k 16k │               │",
            "│                │               │ custom · EQ on                     310  +5.0 dB │               │",
            "│                │               │     h/l band  k/j gain  ,/. preset  E on/off    │               │",
            "│                │               └─────────────────────────────────────────────────┘               │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ Tha                                                 ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Aphex Twin · Selected Ambient Works 85-92                                       2 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:09 │",
            "│ ░░░░░░░░  FLAC · 44.1 kHz · 24-bit → Topping E30 · 44.1 kHz              ● EQ    eq on    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        let a = eq_engaged();
        assert_eq!(render(100, 30, &a), expected);
        // And the seal on that frame is `derive`'s, not this module's.
        let seal = a.seal();
        assert!(!seal.pure);
        assert_eq!(seal.seal_label, "EQ");
    }

    /// **The panel is never allowed to be the exception.** The seal is the last
    /// row of the footer at every usable size, panel or no panel.
    #[test]
    fn the_panel_never_covers_the_seal_at_any_usable_size() {
        let a = eq_engaged();
        for (w, h) in [(80, 20), (80, 60), (100, 30), (200, 24), (240, 80)] {
            let rows = render(w, h, &a);
            let seal = &rows[rows.len() - 2];
            assert!(
                seal.contains("EQ") && !seal.contains("BIT-PERFECT"),
                "the seal at {w}×{h} is wrong or hidden: {seal}"
            );
        }
    }

    /// A bypassed curve and an engaged one are different states, and they read
    /// as different states — the panel says which, and the footer agrees,
    /// because both are reading the same `enabled`.
    #[test]
    fn the_panel_says_whether_the_curve_is_routed_and_the_footer_agrees() {
        let mut a = eq_engaged();
        let on = render(100, 30, &a);
        assert!(
            on.iter().any(|r| r.contains("custom · EQ on")),
            "{}",
            on.join("\n")
        );

        a.act(crate::keys::Action::EqToggle);
        let off = render(100, 30, &a);
        assert!(
            off.iter().any(|r| r.contains("custom · EQ off")),
            "{}",
            off.join("\n")
        );
        assert!(seal_row(&a).contains("BIT-PERFECT"), "{}", seal_row(&a));
    }

    /// Presets reach the panel by name, out of `eko_core`'s table — this crate
    /// never writes one down.
    #[test]
    fn the_panel_names_the_preset_it_is_sitting_on() {
        let mut a = sealed();
        a.act(crate::keys::Action::EqPanel);
        for preset in eko_core::eq_presets::PRESETS.iter().skip(1) {
            a.act(crate::keys::Action::EqPresetNext);
            let rows = render(100, 30, &a);
            assert!(
                rows.iter().any(|r| r.contains(preset.name)),
                "{:?} never appeared in the panel:\n{}",
                preset.name,
                rows.join("\n")
            );
        }
    }

    /// Closed, the panel is not on screen at all — no border, no sliders.
    #[test]
    fn a_closed_panel_draws_nothing() {
        let rows = render(100, 30, &sealed());
        assert!(!rows.iter().any(|r| r.contains("─ EQ ")));
        assert!(!rows.iter().any(|r| r.contains("pre  60 170")));
    }

    /// Play a real track out of `~/Music` and read the seal at three EQ
    /// settings, printing each verbatim.
    ///
    /// This is the one thing a unit test cannot do: it puts a real decoder, a
    /// real CoreAudio device and a real `Engine::set_eq` in the path and shows
    /// that the seal still tracks the EQ. You cannot hear an EQ from a test
    /// runner, so all three readings are numeric.
    ///
    /// The middle reading is the whole point. `EQ` with `pure == false`, over a
    /// session that was really handed the gains, is what says the seventh
    /// false-`BIT-PERFECT` path is closed.
    ///
    /// `#[ignore]`d for the same reason as its neighbours: it claims the
    /// machine's default output device. Run it deliberately:
    ///
    /// ```text
    /// cargo test -p eko-cli --bin eko-cli -- --ignored --nocapture eq_engaged_on_a_real_track
    /// ```
    ///
    /// It is **silent**: [`crate::app::App::hush`] drops the engine to zero
    /// before anything is played, and [`crate::app::App::unhush`] puts the field
    /// the seal reads back to unity — so every reading here is the EQ's own
    /// contribution rather than `VOLUME`.
    #[test]
    #[ignore = "claims the default audio output device; run it deliberately"]
    fn eq_engaged_on_a_real_track_downgrades_a_real_seal() {
        use crate::app::AppEvent;
        use std::time::{Duration, Instant};

        let root = dirs::home_dir().expect("a home directory").join("Music");
        let mut a = scanned();
        a.library = match crate::library::scan(&root, &|_| true) {
            crate::library::ScanEvent::Finished(lib) => *lib,
            other => panic!("could not scan {}: {other:?}", root.display()),
        };
        assert!(!a.library.is_empty(), "no music in {}", root.display());
        println!("track: {}", a.library.albums[0].tracks[0].path);

        a.view = View::Album(0);
        // Silent: `started` hands the engine whatever `App::volume` is at the
        // moment the session opens, so `hush` mutes the output device; `unhush`
        // then puts the field the *seal* reads back to unity without telling the
        // engine. Without that split every reading below would be `VOLUME` and
        // the EQ's own contribution to the label would be invisible.
        a.hush();
        a.play(Source::Local, 0, 0);
        a.unhush();

        let start = Instant::now();
        while a.stream.is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
            a.handle(AppEvent::Tick);
        }
        let info = a
            .stream
            .clone()
            .expect("the engine never described the stream");
        let until = Instant::now() + Duration::from_millis(1_500);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(33));
            a.handle(AppEvent::Tick);
        }
        println!(
            "stream: codec={} src_rate={} rate={} dev_rate={} bits={} device={:?} pos_ms={}",
            info.codec, info.src_rate, info.rate, info.dev_rate, info.bits, info.device, a.pos_ms
        );
        assert!(a.pos_ms > 0, "the playhead never moved");

        let report = |label: &str, a: &App| {
            let seal = a.seal();
            println!(
                "\n{label}\n   active={} pure={} seal_label={:?} engine_label={:?}\n   flags={:?}\n   eq: enabled={} preamp={} gains={:?}  pushes={}",
                seal.active,
                seal.pure,
                seal.seal_label,
                seal.engine_label,
                seal.flags,
                a.eq().enabled(),
                a.seal_input().eq.preamp,
                a.seal_input().eq.gains,
                a.eq_applied,
            );
            seal
        };

        // ── 1. EQ off, flat ──────────────────────────────────────────────
        let flat = report("1. EQ OFF / FLAT", &a);
        assert!(flat.active, "a live track derived no seal");
        assert!(flat.pure, "an untouched path was not bit-perfect");

        // ── 2. EQ on, one band raised ────────────────────────────────────
        a.act(crate::keys::Action::EqPanel);
        for _ in 0..3 {
            a.act(crate::keys::Action::EqNext);
        }
        a.act(crate::keys::Action::EqToggle);
        for _ in 0..6 {
            a.act(crate::keys::Action::Up);
        }
        // Let the DSP thread actually pick the new snapshot up.
        let until = Instant::now() + Duration::from_millis(600);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(33));
            a.handle(AppEvent::Tick);
        }
        let eqd = report("2. EQ ON, 310 Hz +6 dB", &a);
        let frame = render(100, 30, &a);
        println!("\n{}\n", frame.join("\n"));
        assert!(!eqd.pure, "an EQ'd signal still claimed bit-perfect");
        assert!(eqd.flags.eq_active);
        assert_eq!(eqd.seal_label, "EQ");
        let row = frame[28].clone();
        assert!(row.contains(&eqd.seal_label), "{row}");
        assert!(!row.contains("BIT-PERFECT"), "{row}");

        // ── 3. flat again ────────────────────────────────────────────────
        for _ in 0..6 {
            a.act(crate::keys::Action::Down);
        }
        a.act(crate::keys::Action::EqToggle);
        let again = report("3. FLATTENED AGAIN", &a);
        assert_eq!(
            (again.pure, again.seal_label.clone()),
            (flat.pure, flat.seal_label.clone()),
            "flattening did not return the seal to where it started"
        );

        a.act(crate::keys::Action::PlayPause);
    }

    #[test]
    fn justified_drops_the_right_group_rather_than_overlapping() {
        let line = justified(vec![Span::raw("aaaaaaaa")], vec![Span::raw("bbbbbbbb")], 10);
        assert_eq!(line.width(), 8);
    }

    // ── the cover ────────────────────────────────────────────────────────
    //
    // These are the assertions that stand in for a pair of eyes. Nothing here
    // can look at a terminal, so the halfblock renderer — the universal
    // fallback, and the only one whose pixels are ratatui cells — is tested on
    // the **rendered buffer**, and the two out-of-band renderers are tested on
    // the **bytes they emit** and on the blank cells they leave behind.

    use crate::art;

    /// A 64×64 PNG, left half red and right half blue. Generated so the suite
    /// carries no binary fixture, and asymmetric so a mirrored or transposed
    /// grid would be visible.
    fn cover_png() -> Vec<u8> {
        let mut img = image::RgbImage::new(64, 64);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = if x < 32 {
                image::Rgb([220, 30, 30])
            } else {
                image::Rgb([30, 30, 220])
            };
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A playing Deck with a cover already decoded, through the **real** fold.
    ///
    /// No shortcut into `art::Pane`'s internals: the terminal size is reported,
    /// which is what makes the fold ask for a cover, and the answer is handed
    /// back through [`art::Pane::accept`] exactly as a worker's would be. A test
    /// that reached past that would not be exercising the staleness rule at all.
    fn with_cover(protocol: art::Protocol, w: u16, h: u16) -> App {
        let mut a = playing();
        a.art.protocol = protocol;
        a.set_term_size(w, h);
        let key = a.art.wants().cloned().expect("no cover was asked for");
        let outcome = art::encode(&cover_png(), &key);
        assert!(
            matches!(outcome, art::Outcome::Ready(_)),
            "the fixture did not decode"
        );
        let generation = a.art.generation();
        assert!(
            a.art.accept(art::Event {
                generation,
                key,
                outcome
            }),
            "the fold refused its own answer"
        );
        a
    }

    /// Every cell of the buffer carrying a placeholder glyph.
    fn placeholder_cells(width: u16, height: u16, app: &App) -> Vec<(u16, u16)> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        let mut found = Vec::new();
        for y in 0..height {
            for x in 0..width {
                if buf[(x, y)].symbol() == "░" {
                    found.push((x, y));
                }
            }
        }
        found
    }

    /// **The anti-drift assertion.** [`art_area`] duplicates [`render_deck`]'s
    /// layout arithmetic, because the fold and the out-of-band painter need the
    /// rect before a frame exists. This renders real frames and checks that the
    /// placeholder — which occupies exactly the block the pixels will — sits in
    /// precisely those cells and no others.
    #[test]
    fn the_art_area_is_where_the_art_is_actually_drawn() {
        for (w, h) in [(80, 20), (100, 30), (120, 45), (240, 80)] {
            let rect = art_area(Rect::new(0, 0, w, h)).expect("no art area");
            let mut expected: Vec<(u16, u16)> = (rect.y..rect.bottom())
                .flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y)))
                .collect();
            let mut drawn = placeholder_cells(w, h, &app());
            drawn.sort_unstable_by_key(|(x, y)| (*y, *x));
            expected.sort_unstable_by_key(|(x, y)| (*y, *x));
            assert_eq!(drawn, expected, "at {w}×{h}");
        }
    }

    /// Below the minimum there is no Deck, so there is no block to put a cover
    /// in — and the fold must be told that rather than left to guess a rect.
    #[test]
    fn a_terminal_too_small_for_the_deck_has_no_art_area() {
        for (w, h) in [(0, 0), (40, 10), (79, 30), (100, 19)] {
            assert_eq!(art_area(Rect::new(0, 0, w, h)), None, "at {w}×{h}");
        }
    }

    /// Halfblock: every cell of the block is a `▀` with its own two colours, and
    /// the picture is the right way round.
    #[test]
    fn the_halfblock_cover_fills_its_block_with_pixels() {
        let a = with_cover(art::Protocol::Halfblock, 100, 30);
        let rect = art_area(Rect::new(0, 0, 100, 30)).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();

        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let cell = &buf[(x, y)];
                assert_eq!(cell.symbol(), "▀", "cell ({x}, {y}) is not a half block");
                assert_ne!(cell.fg, Color::Reset, "cell ({x}, {y}) has no upper pixel");
                assert_ne!(cell.bg, Color::Reset, "cell ({x}, {y}) has no lower pixel");
            }
        }
        // Left half red, right half blue — the fixture, not a flat fill.
        let left = buf[(rect.x, rect.y)].fg;
        let right = buf[(rect.right() - 1, rect.y)].fg;
        assert_ne!(left, right, "the whole block is one colour");
        let Color::Rgb(lr, _, lb) = left else {
            panic!("{left:?}")
        };
        let Color::Rgb(rr, _, rb) = right else {
            panic!("{right:?}")
        };
        assert!(lr > lb, "the left edge should be red");
        assert!(rb > rr, "the right edge should be blue");
    }

    /// **The seal survives an art render.** The one assertion this whole task is
    /// not allowed to fail.
    ///
    /// The block shares the seal's *rows* — it is four cells tall and so is the
    /// footer — so the guarantee is entirely about columns. This renders each
    /// protocol with and without a cover at every size the Deck supports, and
    /// requires the seal row to be identical from the text column onwards.
    #[test]
    fn an_art_render_never_touches_the_seal_row() {
        for protocol in [
            art::Protocol::Halfblock,
            art::Protocol::Kitty,
            art::Protocol::Iterm2,
        ] {
            for (w, h) in [(80, 20), (100, 30), (240, 80)] {
                let mut bare = playing();
                bare.set_term_size(w, h);
                let covered = with_cover(protocol, w, h);

                let rect = art_area(Rect::new(0, 0, w, h)).unwrap();
                let seal_row = usize::from(h - 2);
                let before = render(w, h, &bare);
                let after = render(w, h, &covered);

                // The whole row from the text column onwards, cell for cell.
                // Counted in `char`s rather than bytes: the placeholder and the
                // half block are three bytes each, so a byte offset would cut
                // one of them in half and compare nothing useful.
                let from = usize::from(rect.right() + super::footer::ART_GAP);
                let tail = |row: &str| row.chars().skip(from).collect::<String>();
                assert_eq!(
                    tail(&before[seal_row]),
                    tail(&after[seal_row]),
                    "{protocol:?} at {w}×{h} changed the seal row"
                );
                // And the lamp is still a lamp.
                let (glyph, _) = lamp(w, h, &covered);
                assert!(matches!(glyph.as_str(), "●" | "○"), "{protocol:?}: {glyph}");
                assert!(
                    after[seal_row].contains("eq flat"),
                    "{protocol:?} at {w}×{h}: {}",
                    after[seal_row]
                );
            }
        }
    }

    /// A graphics protocol leaves the block **blank** in the buffer, so ratatui
    /// owns those cells: its diff leaves them alone while the image is up, and
    /// repaints them the instant the state changes back. Drawing the placeholder
    /// under the image instead would show through wherever detection was wrong.
    #[test]
    fn a_graphics_protocol_leaves_its_block_blank_for_the_out_of_band_layer() {
        for protocol in [art::Protocol::Kitty, art::Protocol::Iterm2] {
            let a = with_cover(protocol, 100, 30);
            let rect = art_area(Rect::new(0, 0, 100, 30)).unwrap();
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| draw(f, &a)).unwrap();
            let buf = terminal.backend().buffer();
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    let cell = &buf[(x, y)];
                    assert_eq!(cell.symbol(), " ", "{protocol:?}: ({x}, {y}) is not blank");
                    assert_eq!(
                        cell.bg,
                        Color::Reset,
                        "{protocol:?}: ({x}, {y}) has a ground"
                    );
                }
            }
            assert!(placeholder_cells(100, 30, &a).is_empty());
        }
    }

    /// The bytes that go to the terminal, for the two renderers no test can see.
    ///
    /// Three things about the placement, all of them geometry the seal depends
    /// on: it starts at the block's top-left cell, it is as many cells wide and
    /// tall as the block, and its bottom row is above the terminal's last row —
    /// an inline image drawn *on* the last row would scroll the alternate
    /// screen, and DECRC cannot undo a scroll.
    #[test]
    fn the_out_of_band_cover_is_placed_inside_its_own_block() {
        for protocol in [art::Protocol::Kitty, art::Protocol::Iterm2] {
            for (w, h) in [(80, 20), (100, 30), (240, 80)] {
                let a = with_cover(protocol, w, h);
                let rect = art_area(Rect::new(0, 0, w, h)).unwrap();
                let place = a.art_placement().expect("no placement");
                assert_eq!((place.x, place.y), (rect.x, rect.y), "{protocol:?}");
                assert_eq!((place.art.cols, place.art.rows), (rect.width, rect.height));
                // Two columns of gap before the footer's text column, so the
                // seal's own columns are not in this rect at all.
                assert!(place.x + place.art.cols < w - 2, "{protocol:?} at {w}×{h}");
                // The image's last row is above the border, so it cannot scroll.
                assert!(place.y + place.art.rows < h, "{protocol:?} at {w}×{h}");

                let mut out: Vec<u8> = Vec::new();
                let mut painter = art::Painter::new();
                painter.take_down(&mut out, Some(place)).unwrap();
                painter.place(&mut out, Some(place)).unwrap();
                let s = String::from_utf8_lossy(&out);
                assert!(s.starts_with('\u{1b}'), "{protocol:?}: no escape");
                assert!(
                    s.contains(&format!("\x1b[{};{}H", rect.y + 1, rect.x + 1)),
                    "{protocol:?} at {w}×{h}: not positioned at the block"
                );
            }
        }
    }

    /// The halfblock renderer paints a background, and it is the only thing in
    /// the Deck allowed to — that background *is* the lower pixel. Everything
    /// outside the block still obeys
    /// [`no_cell_in_the_deck_paints_its_own_background`].
    #[test]
    fn only_the_halfblock_cover_paints_a_ground_and_only_inside_its_block() {
        let a = with_cover(art::Protocol::Halfblock, 100, 30);
        let rect = art_area(Rect::new(0, 0, 100, 30)).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        for y in 0..30u16 {
            for x in 0..100u16 {
                let inside = rect.x <= x && x < rect.right() && rect.y <= y && y < rect.bottom();
                if inside {
                    continue;
                }
                assert_eq!(
                    buf[(x, y)].bg,
                    Color::Reset,
                    "cell ({x}, {y}) painted its own background"
                );
            }
        }
    }

    // ── the visualiser ───────────────────────────────────────────────────

    /// A Deck with the `z` view open, at a stated terminal size.
    fn visualising(w: u16, h: u16) -> App {
        let mut a = playing();
        a.set_term_size(w, h);
        a.act(crate::keys::Action::Visualiser);
        assert!(a.visualiser, "z did not open the view");
        a
    }

    /// The same, with the cover decoded **for the visualiser's own block**.
    ///
    /// It is [`with_cover`] with the `z` pressed before the encode, which is the
    /// point: the key the fold asks for is a different grid, and this asserts
    /// that it asked for one rather than reusing the footer's.
    fn visualising_with_cover(protocol: art::Protocol, w: u16, h: u16) -> App {
        let mut a = playing();
        a.art.protocol = protocol;
        a.set_term_size(w, h);
        a.act(crate::keys::Action::Visualiser);
        let key = a.art.wants().cloned().expect("no cover was asked for");
        assert!(
            key.cols > footer::ART_WIDTH,
            "the visualiser asked for the footer's block: {key:?}"
        );
        let outcome = art::encode(&cover_png(), &key);
        assert!(
            matches!(outcome, art::Outcome::Ready(_)),
            "the fixture did not decode"
        );
        let generation = a.art.generation();
        assert!(
            a.art.accept(art::Event {
                generation,
                key,
                outcome
            }),
            "the fold refused its own answer"
        );
        a
    }

    /// Give the fold an envelope, as a worker would.
    fn with_envelope(mut a: App) -> App {
        let key = a.wave.wants().cloned().expect("no envelope was asked for");
        let peaks: Vec<f32> = (0..crate::wave::BUCKETS)
            .map(|i| i as f32 / crate::wave::BUCKETS as f32)
            .collect();
        // The fill is half the outline everywhere, which is roughly the crest
        // factor of real music and is enough for the assertions about the two
        // to be about two different things.
        let rms: Vec<f32> = peaks.iter().map(|p| p * 0.5).collect();
        let generation = a.wave.generation_for_test();
        assert!(a.wave.accept(crate::wave::Event {
            generation,
            key,
            outcome: crate::wave::Outcome::Ready(crate::wave::Envelope::from_parts(peaks, rms)),
        }));
        a
    }

    /// **The footer does not move.** Not by a row, not by a column, not by a
    /// glyph.
    ///
    /// This is the single thing that makes the `z` view a *view* rather than an
    /// overlay, and it is the reason the seal survives it: the four rows that
    /// carry the signal path are rendered by the same module into the same rect
    /// whichever body is above them.
    ///
    /// The one cell that legitimately differs is the cover block, which is blank
    /// while the visualiser is up — the picture is on screen at forty times the
    /// size, four rows higher. So the comparison is of the **text column**,
    /// which is everything the footer says.
    #[test]
    fn the_visualiser_replaces_the_body_and_leaves_the_footer_exactly_where_it_was() {
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let mut a = playing();
            a.set_term_size(w, h);
            let before = render(w, h, &a);
            a.act(crate::keys::Action::Visualiser);
            let after = render(w, h, &a);

            let text_x = usize::from(1 + GUTTER + footer::ART_WIDTH + footer::ART_GAP);
            for row in (h - 1 - FOOTER_HEIGHT)..(h - 1) {
                let i = usize::from(row);
                let b: String = before[i].chars().skip(text_x).collect();
                let c: String = after[i].chars().skip(text_x).collect();
                assert_eq!(b, c, "the footer moved at {w}×{h}, row {row}");
            }
            // The border and the rule above the footer are where they were too.
            assert_eq!(before[0], after[0], "the title border moved at {w}×{h}");
            // The rule is the same rule, less the `┴` that used to join the
            // sidebar's — there is no sidebar to join. See [`draw`].
            assert_eq!(
                before[usize::from(h - 2 - FOOTER_HEIGHT)].replace('┴', "─"),
                after[usize::from(h - 2 - FOOTER_HEIGHT)],
                "the footer's rule moved at {w}×{h}"
            );
            assert_eq!(
                before[usize::from(h - 1)],
                after[usize::from(h - 1)],
                "the bottom border moved at {w}×{h}"
            );

            // And the body really was replaced: no sidebar, no list.
            assert!(
                !after
                    .iter()
                    .take(usize::from(h - 5))
                    .any(|r| r.contains("SOURCES")),
                "the sidebar survived at {w}×{h}"
            );
        }
    }

    /// **`z` and `esc` are the same door.** Both leave, and both leave the frame
    /// byte-for-byte as it was — the assertion
    /// [`closing_the_help_overlay_restores_the_frame_exactly`] makes about the
    /// overlay, made about the view.
    #[test]
    fn z_and_esc_both_leave_the_visualiser_and_restore_the_frame_exactly() {
        for leave in [crate::keys::Action::Visualiser, crate::keys::Action::Back] {
            for (w, h) in [(80, 20), (100, 30), (240, 80)] {
                let mut a = playing();
                a.set_term_size(w, h);
                let before = render(w, h, &a);
                a.act(crate::keys::Action::Visualiser);
                assert_ne!(render(w, h, &a), before, "z drew nothing at {w}×{h}");
                a.act(leave);
                assert!(!a.visualiser, "{leave:?} did not leave at {w}×{h}");
                assert_eq!(
                    render(w, h, &a),
                    before,
                    "{leave:?} did not restore the frame at {w}×{h}"
                );
            }
        }
    }

    /// The body the visualiser draws into is the body [`render_deck`] gives it,
    /// and it stops above the rule.
    ///
    /// [`visualiser::body`] reproduces that arithmetic so the fold can key the
    /// cover cache on it before a frame exists — the same duplication
    /// [`art_area`] carries, and the same reason it needs a test that renders a
    /// real frame.
    ///
    /// [`render_deck`]: draw
    #[test]
    fn the_visualiser_body_stops_above_the_footer_rule() {
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let body = visualiser::body(Rect::new(0, 0, w, h)).expect("no body");
            assert_eq!((body.x, body.y), (1, 1));
            assert_eq!(body.width, w - 2);
            // One rule row and FOOTER_HEIGHT rows below it, then the border.
            assert_eq!(body.bottom(), h - 1 - FOOTER_HEIGHT - 1);
            // Every panel is inside it, at every size — and inside the margins.
            let p = visualiser::panels(body);
            for r in [
                p.frame,
                p.cover,
                p.meta,
                p.analyser,
                p.analyser_foot,
                p.wave,
                p.readout,
            ] {
                if r.is_empty() {
                    continue;
                }
                assert!(r.bottom() <= body.bottom(), "{r:?} at {w}×{h}");
                assert!(r.x >= body.x + visualiser::MARGIN, "{r:?} at {w}×{h}");
                assert!(
                    r.right() <= body.right() - visualiser::MARGIN,
                    "{r:?} at {w}×{h}"
                );
            }
        }
        // Below the minimum there is no Deck and so no body.
        assert!(visualiser::body(Rect::new(0, 0, 79, 30)).is_none());
        assert!(visualiser::body(Rect::new(0, 0, 100, 19)).is_none());
    }

    /// **The big cover is the footer's renderer at a bigger rect.**
    ///
    /// Same halfblock painter, same cache, same `art::Key` shape — only `cols`
    /// and `rows` differ. The assertion is on the pixels: the cells inside the
    /// visualiser's cover block are `▀`, and there are far more of them than the
    /// footer's thirty-two.
    #[test]
    fn the_big_cover_is_the_same_renderer_at_a_different_rect() {
        let a = visualising_with_cover(art::Protocol::Halfblock, 100, 30);
        let rect = visualiser::cover_area(Rect::new(0, 0, 100, 30)).expect("no cover rect");
        // Square, and much larger than the footer's block.
        assert_eq!(rect.width, rect.height * 2);
        assert!(rect.width > footer::ART_WIDTH * 2);

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                assert_eq!(buf[(x, y)].symbol(), "▀", "cell ({x}, {y}) is not a pixel");
                assert_ne!(
                    buf[(x, y)].bg,
                    Color::Reset,
                    "({x}, {y}) has no lower pixel"
                );
            }
        }
        // And the footer's own block is blank rather than a stale placeholder.
        let footer_block = footer::art_rect(Rect {
            x: 1,
            y: 30 - 1 - FOOTER_HEIGHT,
            width: 98,
            height: FOOTER_HEIGHT,
        });
        for y in footer_block.y..footer_block.bottom() {
            for x in footer_block.x..footer_block.right() {
                assert_eq!(buf[(x, y)].symbol(), " ", "the footer redrew the cover");
            }
        }
    }

    /// An out-of-band cover is placed in the **visualiser's** block, and that
    /// block is still entirely above the footer.
    #[test]
    fn a_kitty_cover_is_placed_in_the_visualisers_block_and_never_over_the_seal() {
        for protocol in [art::Protocol::Kitty, art::Protocol::Iterm2] {
            for (w, h) in [(80, 20), (100, 30), (240, 80)] {
                let a = visualising_with_cover(protocol, w, h);
                let rect = visualiser::cover_area(Rect::new(0, 0, w, h)).unwrap();
                let place = a.art_placement().expect("no placement");
                assert_eq!((place.x, place.y), (rect.x, rect.y), "{protocol:?}");
                assert_eq!((place.art.cols, place.art.rows), (rect.width, rect.height));
                // The image's last row is above the footer's rule, so it cannot
                // reach the seal however tall it is.
                assert!(
                    place.y + place.art.rows < h - 1 - FOOTER_HEIGHT,
                    "{protocol:?} at {w}×{h}: the image reaches the footer"
                );
            }
        }
    }

    /// **Thirty-two bars, separated, and no thirty-third.** At the frame, not
    /// just in the arithmetic.
    ///
    /// Read off the buffer as lit column groups: a build that interpolated the
    /// bands up to one bar per column would draw one unbroken field, and a build
    /// that had lost the gap would draw thirty-two bars that fused into terrain
    /// — which is exactly what the previous draft did, and what a peak-hold cap
    /// needs the gap to be readable against.
    #[test]
    fn the_analyser_draws_thirty_two_separated_bars_and_not_one_more() {
        for (w, h) in [(100u16, 30u16), (140, 40), (240, 60)] {
            let mut a = visualising(w, h);
            // Every band at full scale, so nothing is silent and every bar is
            // lit for its whole width — the gaps are then the only blanks.
            a.bands = vec![1.0; 32];
            a.peaks = vec![1.0; 32];
            let body = visualiser::body(Rect::new(0, 0, w, h)).unwrap();
            let an = visualiser::panels(body).analyser;
            assert!(!an.is_empty(), "{w}×{h}: no analyser");
            let bw = visualiser::bar_width(an.width).expect("no bar width");

            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &a)).unwrap();
            let buf = terminal.backend().buffer();

            // Lit columns on the floor row, grouped into runs.
            let floor = an.bottom() - 1;
            let lit: Vec<bool> = (an.x..an.right())
                .map(|x| buf[(x, floor)].symbol() != " ")
                .collect();
            let mut runs: Vec<usize> = Vec::new();
            let mut gaps: Vec<usize> = Vec::new();
            let mut i = 0;
            while i < lit.len() {
                let start = i;
                while i < lit.len() && lit[i] == lit[start] {
                    i += 1;
                }
                if lit[start] {
                    runs.push(i - start);
                } else if start > 0 && i < lit.len() {
                    // Interior blanks only; the centring leaves air at each end.
                    gaps.push(i - start);
                }
            }
            assert_eq!(
                runs.len(),
                visualiser::BANDS,
                "{w}×{h}: the analyser drew {} bars; the engine reported {}",
                runs.len(),
                visualiser::BANDS
            );
            assert!(
                runs.iter().all(|r| *r == usize::from(bw)),
                "{w}×{h}: a bar was not {bw} columns wide: {runs:?}"
            );
            assert!(
                gaps.iter().all(|g| *g == 1),
                "{w}×{h}: the bars are not separated by one column: {gaps:?}"
            );
        }
    }

    /// **The analyser is amber on the screen, not only in the ramp function.**
    ///
    /// The seal's green is the smallest important thing on this view and the
    /// analyser is the largest decorative one; a green field beside a green lamp
    /// is the failure that motivated the redraw. So no lit analyser cell may be
    /// either of the two greens the seal's lamp is drawn in.
    #[test]
    fn no_cell_of_the_analyser_is_ever_the_seals_green() {
        let mut a = visualising(140, 40);
        a.bands = (0..32).map(|i| (i as f32 + 1.0) / 32.0).collect();
        a.peaks = vec![1.0; 32];
        let body = visualiser::body(Rect::new(0, 0, 140, 40)).unwrap();
        let an = visualiser::panels(body).analyser;
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        let mut lit = 0;
        for y in an.y..an.bottom() {
            for x in an.x..an.right() {
                if buf[(x, y)].symbol() == " " {
                    continue;
                }
                lit += 1;
                assert_ne!(buf[(x, y)].fg, a.theme.led_green, "({x}, {y}) is green");
                assert_ne!(buf[(x, y)].fg, a.theme.led_green_dim, "({x}, {y}) is green");
            }
        }
        assert!(lit > 100, "only {lit} cells were lit");
        // And the seal's lamp is still the green one, four rows below.
        let rows = render(140, 40, &a);
        assert!(rows[40 - 2].contains('○'), "the lamp moved");
    }

    /// **The signal readout, derived and never invented.**
    ///
    /// The three rows are on screen at a size that has room for them, they say
    /// what `signal_path::derive` said, and there is no fourth row: `BUFFER`
    /// was asked for and dropped, because `EngineStatus` reports no buffer
    /// depth. See [`visualiser::decode_terms`].
    #[test]
    fn the_readout_says_what_the_seal_says_and_nothing_it_cannot_measure() {
        let mut a = visualising(140, 40);
        a.stream = Some(StreamInfo {
            rate: 96_000,
            src_rate: 96_000,
            dev_rate: 96_000,
            bits: 24,
            codec: "flac".into(),
            device: "Topping E30".into(),
        });
        let seal = a.seal();
        assert!(seal.pure, "the fixture is not a clean path");

        let body = visualiser::body(Rect::new(0, 0, 140, 40)).unwrap();
        let readout = visualiser::panels(body).readout;
        assert_eq!(readout.height, 3, "the readout is not three rows");
        let rows = render(140, 40, &a);
        let text: Vec<&String> = (readout.y..readout.bottom())
            .map(|y| &rows[usize::from(y)])
            .collect();

        assert!(text[0].contains("SOURCE"), "{}", text[0]);
        assert!(text[0].contains(&seal.src), "{} != {}", text[0], seal.src);
        assert!(text[1].contains("DECODE"), "{}", text[1]);
        assert!(text[1].contains("no resampling"), "{}", text[1]);
        assert!(text[1].contains("no EQ"), "{}", text[1]);
        assert!(text[1].contains("no gain"), "{}", text[1]);
        assert!(text[2].contains("OUTPUT"), "{}", text[2]);
        assert!(
            text[2].contains(&seal.output),
            "{} != {}",
            text[2],
            seal.output
        );

        // Nothing anywhere on the view claims a stage nobody measured.
        let whole = rows.join("\n").to_lowercase();
        for word in ["dither", "buffer", "underrun", "kbps", "latency"] {
            assert!(!whole.contains(word), "the view claimed `{word}`");
        }

        // Break the seal and the readout breaks with it, in the same words.
        a.stream.as_mut().unwrap().src_rate = 44_100;
        let seal = a.seal();
        assert!(seal.seal_label.contains("RESAMPLED"));
        let rows = render(140, 40, &a);
        let decode = &rows[usize::from(readout.y + 1)];
        assert!(decode.contains("resampled"), "{decode}");
        assert!(!decode.contains("no resampling"), "{decode}");
    }

    /// **With nothing to report, the readout reports nothing** — not a
    /// plausible default.
    #[test]
    fn an_inactive_seal_leaves_the_readout_as_dashes() {
        let a = visualising(140, 40);
        assert!(!a.seal().active, "the fixture described a stream");
        let body = visualiser::body(Rect::new(0, 0, 140, 40)).unwrap();
        let readout = visualiser::panels(body).readout;
        let rows = render(140, 40, &a);
        for y in readout.y..readout.bottom() {
            let row = &rows[usize::from(y)];
            assert!(row.contains('—'), "{row}");
            assert!(!row.contains("kHz"), "a rate was invented: {row}");
            assert!(!row.contains("no resampling"), "a claim was made: {row}");
        }
    }

    /// The analyser can be switched off, and then the tick drops with it.
    #[test]
    fn an_analyser_that_is_configured_off_is_not_drawn_and_does_not_cost_a_tick() {
        let mut a = visualising(100, 30);
        a.analyser = false;
        let body = visualiser::body(Rect::new(0, 0, 100, 30)).unwrap();
        let p = visualiser::panels(body);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        // The bars **and** their caption: an axis with no axis under it is a
        // label for something that is not there.
        for r in [p.analyser, p.analyser_foot] {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    assert_eq!(buf[(x, y)].symbol(), " ", "drawn at ({x}, {y})");
                }
            }
        }
        assert_eq!(a.tick_interval(), Some(crate::app::CLOCK_INTERVAL));
        a.analyser = true;
        assert_eq!(a.tick_interval(), Some(crate::app::FRAME_INTERVAL));
    }

    /// **There is no block clock.** The footer says the position four rows
    /// below, the 3×5 font collided with the envelope on the size the owner ran,
    /// and a clock at that size is not what this view is for.
    ///
    /// Asserted as the font's own glyph: the clock was drawn in `█`, and the
    /// only two things on this view allowed to draw one are the analyser and the
    /// envelope, both of which grow theirs out of the floor.
    #[test]
    fn the_view_draws_no_clock_of_its_own() {
        for (w, h) in [(100u16, 30u16), (140, 40)] {
            let a = with_envelope(visualising(w, h));
            let body = visualiser::body(Rect::new(0, 0, w, h)).unwrap();
            let p = visualiser::panels(body);
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &a)).unwrap();
            let buf = terminal.backend().buffer();
            let inside =
                |r: Rect, x: u16, y: u16| r.x <= x && x < r.right() && r.y <= y && y < r.bottom();
            for y in body.y..body.bottom() {
                for x in body.x..body.right() {
                    if inside(p.analyser, x, y) || inside(p.wave, x, y) {
                        continue;
                    }
                    assert_ne!(
                        buf[(x, y)].symbol(),
                        "█",
                        "{w}×{h}: a block glyph survived at ({x}, {y})"
                    );
                }
            }
            // And the footer still says the position, where it always has.
            let rows = render(w, h, &a);
            let scrubber = &rows[usize::from(h) - 3];
            assert!(scrubber.contains("02:04"), "{scrubber}");
        }
    }

    // ── degrading ────────────────────────────────────────────────────────

    /// **Nothing outside the cover paints a background, at any colour depth.**
    ///
    /// This was `without_truecolor_the_visualiser_paints_no_backdrop_at_all`,
    /// which asserted that a 256-colour terminal was spared the ambient wash
    /// because quantising it produced blotches. There is no wash any more, so
    /// the assertion is the stronger one: neither depth paints anything.
    #[test]
    fn the_visualiser_paints_no_background_outside_the_cover_at_either_depth() {
        for depth in [ColorDepth::Ansi256, ColorDepth::TrueColor] {
            let mut a = App::new(
                &Config::default(),
                Theme::new(Accent::Orange, depth),
                MusicFolder::Unset { probed: None },
            );
            a.library = scanned().library;
            a.scan = ScanState::Ready;
            a.play(Source::Local, 0, 1);
            a.art.protocol = art::Protocol::Halfblock;
            a.set_term_size(100, 30);
            a.act(crate::keys::Action::Visualiser);
            let key = a.art.wants().cloned().unwrap();
            let outcome = art::encode(&cover_png(), &key);
            let generation = a.art.generation();
            assert!(a.art.accept(art::Event {
                generation,
                key,
                outcome
            }));

            let cover = visualiser::cover_area(Rect::new(0, 0, 100, 30)).unwrap();
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| draw(f, &a)).unwrap();
            let buf = terminal.backend().buffer();
            for y in 0..30u16 {
                for x in 0..100u16 {
                    let in_cover =
                        cover.x <= x && x < cover.right() && cover.y <= y && y < cover.bottom();
                    if in_cover {
                        continue;
                    }
                    assert_eq!(
                        buf[(x, y)].bg,
                        Color::Reset,
                        "{depth:?}: cell ({x}, {y}) painted a background"
                    );
                }
            }
            // The cover is real pixels, and it is the only thing that is.
            assert_eq!(buf[(cover.x, cover.y)].symbol(), "▀", "{depth:?}");
            assert_ne!(buf[(cover.x, cover.y)].bg, Color::Reset, "{depth:?}");
        }
    }

    /// **No cover, no backdrop, and a placeholder rather than a guess.**
    #[test]
    fn a_visualiser_with_no_cover_shows_the_placeholder_and_no_wash() {
        // `playing()` never answers a cover request, so this is the real
        // "still loading, or there is no picture" state.
        let a = visualising(100, 30);
        let cover = visualiser::cover_area(Rect::new(0, 0, 100, 30)).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        for y in cover.y..cover.bottom() {
            for x in cover.x..cover.right() {
                assert_eq!(
                    buf[(x, y)].symbol(),
                    "░",
                    "({x}, {y}) is not the placeholder"
                );
            }
        }
        for y in 0..30u16 {
            for x in 0..100u16 {
                assert_eq!(
                    buf[(x, y)].bg,
                    Color::Reset,
                    "no cover, and yet ({x}, {y}) was washed"
                );
            }
        }
    }

    /// **No envelope, no shape** — and nothing synthesised in its place. The
    /// clocks at each end are still drawn, because they are read from the
    /// transport rather than from a decode that may never land.
    #[test]
    fn a_track_with_no_envelope_draws_no_overview_and_invents_nothing() {
        let a = visualising(140, 40);
        assert!(a.wave.envelope().is_none());
        let body = visualiser::body(Rect::new(0, 0, 140, 40)).unwrap();
        let wave = visualiser::panels(body).wave;
        assert!(!wave.is_empty(), "no envelope block was reserved");
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();
        // The reserved rows are blank between the clocks' columns.
        for y in wave.y..wave.bottom() {
            for x in (wave.x + 6)..(wave.right() - 6) {
                assert_eq!(
                    buf[(x, y)].symbol(),
                    " ",
                    "an overview was drawn with no envelope at ({x}, {y})"
                );
            }
        }
        // The scrubber is still on the footer, and it is still the plain one.
        let rows = render(140, 40, &a);
        let scrubber = &rows[40 - 3];
        assert!(
            scrubber.contains('━') || scrubber.contains('─'),
            "{scrubber}"
        );
    }

    /// And with one, the block is a shape whose played half is the accent, with
    /// a clock at each end that never shares a column with it.
    #[test]
    fn an_envelope_draws_an_overview_that_doubles_as_a_scrubber() {
        let a = with_envelope(visualising(140, 40));
        let body = visualiser::body(Rect::new(0, 0, 140, 40)).unwrap();
        let wave = visualiser::panels(body).wave;
        // Six rows here, and **not a constant**: the envelope is where the
        // layout's residual rows go, so its height is a function of the body and
        // it ends on the body's own last row. See [`visualiser::panels`] and
        // `no_size_leaves_a_void_and_no_two_blocks_share_a_row`.
        assert_eq!(
            wave.height, 6,
            "the envelope is not the size the body gave it"
        );
        assert_eq!(
            wave.bottom(),
            body.bottom(),
            "the envelope left a void below"
        );
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|f| draw(f, &a)).unwrap();
        let buf = terminal.backend().buffer();

        let floor = wave.bottom() - 1;
        let glyphs: String = ((wave.x + 6)..(wave.right() - 6))
            .map(|x| buf[(x, floor)].symbol())
            .collect();
        assert!(glyphs.contains('█'), "{glyphs}");
        // The fixture's envelope ramps from silence, so the top row is empty at
        // the head and full at the tail — four rows of resolution, not one.
        let top: String = ((wave.x + 6)..(wave.right() - 6))
            .map(|x| buf[(x, wave.y)].symbol())
            .collect();
        assert!(top.starts_with(' '), "{top}");
        assert!(top.trim_end().ends_with('█'), "{top}");

        // The played portion is the accent; the rest is the scrubber's track.
        assert_eq!(buf[(wave.x + 6, floor)].fg, a.theme.accent);
        assert_eq!(buf[(wave.right() - 7, floor)].fg, a.theme.scrubber_track);

        // **Both readings are on the block.** The fixture's fill is half its
        // outline, so every column that has a shape at all has a solid bottom
        // and a dimmed top — the peak outline and the RMS fill of
        // [`crate::wave::Envelope`], which is the whole reason the block is not
        // a slab. A frame with only one of the two styles in it is the bug.
        let mut solid = 0;
        let mut outline = 0;
        for y in wave.y..wave.bottom() {
            for x in (wave.x + 6)..(wave.right() - 6) {
                if buf[(x, y)].symbol() == " " {
                    continue;
                }
                if buf[(x, y)].modifier.contains(ratatui::style::Modifier::DIM) {
                    outline += 1;
                } else {
                    solid += 1;
                }
            }
        }
        assert!(solid > 0 && outline > 0, "solid {solid}, outline {outline}");

        // The clocks own their columns, on the block's last row, and the shape
        // never reaches them.
        let row: String = (wave.x..wave.right())
            .map(|x| buf[(x, floor)].symbol())
            .collect();
        assert!(row.starts_with("02:04"), "{row}");
        // The duration is the fixture's own, read the way the footer reads it.
        let dur = format!("{:02}:{:02}", a.dur_ms() / 60_000, a.dur_ms() / 1000 % 60);
        assert!(row.trim_end().ends_with(&dur), "{row} does not end {dur}");
    }

    /// **A framed sleeve, and that is what replaced the wash.**
    ///
    /// The ambient backdrop is gone — it painted the whole body's cell
    /// backgrounds, cost three times what the analyser did, and could not be
    /// drawn at all on kitty or iTerm2, where the cover is a real photograph and
    /// most wanted an edge. What a light sleeve on a dark terminal actually
    /// needed was an edge, so it has one: a one-cell rule, in the same colour
    /// every other rule on the Deck is drawn in, and no background anywhere.
    #[test]
    fn the_cover_gets_a_rule_around_it_rather_than_a_wash_behind_it() {
        for protocol in [
            art::Protocol::Halfblock,
            art::Protocol::Kitty,
            art::Protocol::Iterm2,
        ] {
            let a = visualising_with_cover(protocol, 100, 30);
            let body = visualiser::body(Rect::new(0, 0, 100, 30)).unwrap();
            let p = visualiser::panels(body);
            let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
            terminal.draw(|f| draw(f, &a)).unwrap();
            let buf = terminal.backend().buffer();

            // The rule, on all four sides, in the rule colour.
            let f = p.frame;
            for (x, y, want) in [
                (f.x, f.y, "\u{256d}"),
                (f.right() - 1, f.y, "\u{256e}"),
                (f.x, f.bottom() - 1, "\u{2570}"),
                (f.right() - 1, f.bottom() - 1, "\u{256f}"),
                (f.x + 1, f.y, "\u{2500}"),
                (f.x, f.y + 1, "\u{2502}"),
            ] {
                assert_eq!(buf[(x, y)].symbol(), want, "{protocol:?} at ({x}, {y})");
                assert_eq!(buf[(x, y)].fg, a.theme.rule, "{protocol:?} at ({x}, {y})");
                assert_eq!(buf[(x, y)].bg, Color::Reset, "{protocol:?} at ({x}, {y})");
            }

            // And nothing outside the picture itself paints a background — the
            // assertion the whole body used to be exempt from.
            let c = p.cover;
            for y in 0..30u16 {
                for x in 0..100u16 {
                    if c.x <= x && x < c.right() && c.y <= y && y < c.bottom() {
                        continue;
                    }
                    assert_eq!(
                        buf[(x, y)].bg,
                        Color::Reset,
                        "{protocol:?}: ({x}, {y}) painted a background"
                    );
                }
            }
        }
    }

    /// The margins are real, and they are five columns wide.
    ///
    /// The previous draft laid the view out in `pane_body` — one column, the
    /// Deck's pane padding — so a white cover and a full-width analyser both ran
    /// to within a cell of the frame.
    #[test]
    fn the_view_keeps_five_clear_columns_at_each_edge() {
        for (w, h) in [(80u16, 20u16), (100, 30), (140, 40), (240, 60)] {
            let a = with_envelope(visualising_with_cover(art::Protocol::Halfblock, w, h));
            let rows = render(w, h, &a);
            let body = visualiser::body(Rect::new(0, 0, w, h)).unwrap();
            for y in body.y..body.bottom() {
                let row: Vec<char> = rows[usize::from(y)].chars().collect();
                for x in body.x..(body.x + visualiser::MARGIN) {
                    assert_eq!(row[usize::from(x)], ' ', "{w}\u{d7}{h}: ({x}, {y})");
                }
                for x in (body.right() - visualiser::MARGIN)..body.right() {
                    assert_eq!(row[usize::from(x)], ' ', "{w}\u{d7}{h}: ({x}, {y})");
                }
            }
        }
    }

    /// **What a visualiser frame costs.** Reported, not assumed.
    ///
    /// `#[ignore]`d: it is a measurement, not an assertion — a loaded CI runner
    /// would fail a wall-clock bound that says nothing about the code. Run it
    /// deliberately:
    ///
    /// ```text
    /// cargo test --release -p eko-cli visualiser_frame_cost -- --ignored --nocapture
    /// ```
    ///
    /// The number that matters is the *ratio* to a library frame, because the
    /// design claim is that the Deck pays nothing for the eye candy and the
    /// visualiser pays for it only while it is on screen. Both are drawn at the
    /// same size, from the same fixtures, in the same process.
    #[test]
    #[ignore = "a measurement, not an assertion; run it deliberately"]
    fn visualiser_frame_cost() {
        use std::time::Instant;
        const N: u32 = 2_000;

        let measure = |label: &str, a: &App, w: u16, h: u16| {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            // Warm the allocator and the buffer.
            for _ in 0..100 {
                terminal.draw(|f| draw(f, a)).unwrap();
            }
            let start = Instant::now();
            for _ in 0..N {
                terminal.draw(|f| draw(f, a)).unwrap();
            }
            let each = start.elapsed() / N;
            println!("{label:52} {w}×{h}  {:>9.1} µs", each.as_secs_f64() * 1e6);
            each
        };

        for (w, h) in [(100u16, 30u16), (240, 80)] {
            let deck = playing();
            let vis = with_envelope(visualising_with_cover(art::Protocol::Halfblock, w, h));
            let mut no_analyser =
                with_envelope(visualising_with_cover(art::Protocol::Halfblock, w, h));
            no_analyser.analyser = false;

            let d = measure("the Deck (library + footer spectrum)", &deck, w, h);
            let v = measure("the visualiser, everything on", &vis, w, h);
            measure("the visualiser, analyser off", &no_analyser, w, h);
            println!(
                "  ratio visualiser/Deck at {w}×{h}: {:.2}×,  budget at 30fps is 33_000 µs\n",
                v.as_secs_f64() / d.as_secs_f64()
            );
        }
    }

    /// The whole degradation matrix as frames: two sizes, with and without every
    /// optional part, and nothing anywhere that was invented to fill a gap.
    #[test]
    fn the_visualiser_degrades_without_inventing_anything() {
        for (w, h) in [(80u16, 20u16), (100, 30)] {
            for cover in [false, true] {
                for envelope in [false, true] {
                    let mut a = if cover {
                        visualising_with_cover(art::Protocol::Halfblock, w, h)
                    } else {
                        visualising(w, h)
                    };
                    if envelope {
                        a = with_envelope(a);
                    }
                    let rows = render(w, h, &a);
                    let label = format!("{w}×{h} cover={cover} envelope={envelope}");

                    // The frame is the right shape, whatever is missing.
                    assert_eq!(rows.len(), usize::from(h), "{label}");
                    assert!(
                        rows.iter().all(|r| r.chars().count() == usize::from(w)),
                        "{label}"
                    );
                    // The seal is on its row, saying what it always says.
                    let seal = &rows[usize::from(h - 2)];
                    assert!(
                        seal.contains("UNVERIFIED") || seal.contains("IDLE"),
                        "{label}: {seal}"
                    );
                    // Nothing claims a picture it has not got.
                    let body_rows = &rows[1..usize::from(h - 2 - FOOTER_HEIGHT)];
                    let has_pixels = body_rows.iter().any(|r| r.contains('▀'));
                    assert_eq!(has_pixels, cover, "{label}: pixels without a cover");
                    let has_placeholder = body_rows.iter().any(|r| r.contains('░'));
                    assert_eq!(has_placeholder, !cover, "{label}: no placeholder either");
                }
            }
        }
    }

    /// One thing owns the body. Opening any overlay leaves the visualiser, and
    /// opening the visualiser closes any overlay.
    #[test]
    fn the_visualiser_and_the_three_overlays_are_mutually_exclusive() {
        for open in [crate::keys::Action::Help, crate::keys::Action::EqPanel] {
            let mut a = visualising(100, 30);
            a.act(open);
            assert!(!a.visualiser, "{open:?} left the view open under it");
            a.act(crate::keys::Action::Visualiser);
            assert!(a.visualiser);
            assert!(!a.help_open && !a.eq_open && !a.device_open, "{open:?}");
        }
        // And the keymap is never drawn over the view.
        let mut a = visualising(100, 30);
        a.act(crate::keys::Action::Help);
        let rows = render(100, 30, &a);
        assert!(rows.iter().any(|r| r.contains("─ KEYS ")));
        assert!(
            rows.iter().any(|r| r.contains("SOURCES")),
            "the body did not come back"
        );
    }

    /// Every way a cover can fail to arrive ends in the placeholder, and none of
    /// them leaves a half-drawn or misleading cell.
    #[test]
    fn a_cover_that_never_arrives_leaves_the_placeholder() {
        let rect = art_area(Rect::new(0, 0, 100, 30)).unwrap();
        let block: Vec<(u16, u16)> = (rect.y..rect.bottom())
            .flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y)))
            .collect();

        // 1. Still loading — a worker was started and has not answered.
        let mut loading = playing();
        loading.set_term_size(100, 30);
        assert!(loading.art.wants().is_some());
        assert!(loading.art_placement().is_none());

        // 2. Answered, and there is no picture: a 404, a decode failure, a
        //    track with no art, a server that sent a login page.
        let mut none = playing();
        none.set_term_size(100, 30);
        let key = none.art.wants().cloned().unwrap();
        let generation = none.art.generation();
        assert!(none.art.accept(art::Event {
            generation,
            key,
            outcome: art::Outcome::Unavailable,
        }));

        // 3. Nothing playing at all.
        let mut stopped = app();
        stopped.set_term_size(100, 30);

        for (what, a) in [("loading", &loading), ("no art", &none), ("idle", &stopped)] {
            let mut cells = placeholder_cells(100, 30, a);
            cells.sort_unstable_by_key(|(x, y)| (*y, *x));
            let mut want = block.clone();
            want.sort_unstable_by_key(|(x, y)| (*y, *x));
            assert_eq!(cells, want, "{what}");
            assert!(a.art_placement().is_none(), "{what}");
        }
    }

    /// Snapshot of the halfblock renderer at 100×30, with the seal on screen.
    ///
    /// The cover is the eight-by-four block of `▀` on the left of the footer —
    /// square in device pixels, which is the whole reason it is eight; the
    /// colours a text dump cannot carry are asserted by
    /// [`the_halfblock_cover_fills_its_block_with_pixels`]. Everything to the
    /// right of it — including the whole seal row — is identical to
    /// [`deck_at_100x30_sealed_bit_perfect`], which is the point.
    #[test]
    fn deck_at_100x30_with_a_halfblock_cover() {
        let mut a = with_cover(art::Protocol::Halfblock, 100, 30);
        a.stream = Some(stream(44_100, 44_100, 44_100));
        let frame = render(100, 30, &a);
        assert_eq!(
            &frame[25..],
            &[

                "│ ▀▀▀▀▀▀▀▀  ▶ Tha                                                 ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
                "│ ▀▀▀▀▀▀▀▀  Aphex Twin · Selected Ambient Works 85-92                                       2 of 3 │",
                "│ ▀▀▀▀▀▀▀▀  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:09 │",
                "│ ▀▀▀▀▀▀▀▀  FLAC · 44.1 kHz · 24-bit → Topping E30 · 44.1 kHz   ● BIT-PERFECT    eq flat    rg off │",
                "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
            ]
        );
    }
    // ── the output-device picker ──────────────────────────────────────────

    /// [`sealed`], with the picker open over it and two devices to choose from.
    ///
    /// The device list is **stated**, not read off the host: `list_devices` would
    /// return whatever is plugged into the machine running the suite, and a
    /// snapshot of that is a snapshot of somebody's desk.
    fn picking() -> App {
        let mut a = sealed();
        a.devices = vec![
            "MacBook Pro Speakers".to_string(),
            "Topping E30".to_string(),
        ];
        a.device_open = true;
        a.device_cursor = 2;
        a
    }

    /// Snapshot: **the picker open, and the seal still in full view under it.**
    ///
    /// The device is exactly what the right-hand half of the seal row describes,
    /// so a picker that covered it would be hiding the claim it is about to
    /// change. It is drawn over the main pane and nothing else.
    ///
    /// Read the `▶` against the seal row at the bottom of this frame: both say
    /// `Topping E30`, and they agree because they are the same field — the
    /// engine's live stream — rendered twice. The *preference* in this fixture is
    /// the system default, and the system default here is `Topping E30`: no row
    /// claims to be the configured one, because none of them is. While `▶` came
    /// from the preference this same frame marked `System default` under a seal
    /// that said `Topping E30` — the two halves of one screen naming two DACs.
    #[test]
    fn deck_at_100x30_with_the_device_picker_open() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────── Music · 2 albums · 4 tracks ┐",
            "│ SOURCES        │ ‹ SELECTED AMBIENT WORKS 85-92                                                  │",
            "│                │ Aphex Twin · 3 tracks                                                           │",
            "│ ● Local      2 │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ + Add server   │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ● Queue      3 │ ▌  ▶  Aphex Twin — Selected Ambien…  3     1  Xtal                         4:51 │",
            "│                │       Boards of Canada — Music Has…  1  ▌  ▶  Tha                          9:09 │",
            "│                │                                            3  Pulsewidth                   3:52 │",
            "│                │                                                                                 │",
            "│                │                         ┌─ OUTPUT ───────────────────┐                          │",
            "│                │                         │    System default          │                          │",
            "│                │                         │    MacBook Pro Speakers    │                          │",
            "│                │                         │  ▶ Topping E30             │                          │",
            "│                │                         │                            │                          │",
            "│                │                         │    enter select  esc close │                          │",
            "│                │                         └────────────────────────────┘                          │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ Tha                                                 ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Aphex Twin · Selected Ambient Works 85-92                                       2 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:09 │",
            "│ ░░░░░░░░  FLAC · 44.1 kHz · 24-bit → Topping E30 · 44.1 kHz   ● BIT-PERFECT    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &picking()), expected);
    }

    /// **The picker is never allowed to be the exception either.** The seal is the
    /// last row of the footer at every usable size, picker or no picker.
    ///
    /// Both shapes of the panel are checked: the plain list, and the two rows
    /// taller one a pending choice adds. A sentence that made the panel too tall
    /// for an 80×20 terminal would make `d` draw nothing at all there, which is
    /// how a fix for one screen becomes a blank overlay on another.
    #[test]
    fn the_picker_never_covers_the_seal_at_any_usable_size() {
        let mut pending = picking();
        pending.playback = Playback::Paused;
        pending.stream = Some(StreamInfo {
            device: "MacBook Pro Speakers".into(),
            ..stream(44_100, 44_100, 44_100)
        });
        pending.device = crate::config::OutputDevice::Configured("Topping E30".into());

        for (w, h) in [(80, 20), (80, 60), (100, 30), (200, 24), (240, 80)] {
            let rows = render(w, h, &picking());
            let seal = &rows[rows.len() - 2];
            assert!(
                seal.contains("BIT-PERFECT"),
                "the seal at {w}×{h} is wrong or hidden: {seal}"
            );

            let rows = render(w, h, &pending);
            let seal = &rows[rows.len() - 2];
            assert!(
                seal.contains("BIT-PERFECT"),
                "with a pending device at {w}×{h} the seal is wrong or hidden: {seal}"
            );
            assert!(
                rows.iter()
                    .any(|r| r.contains("Topping E30 from the next track")),
                "the pending sentence did not fit at {w}×{h}:\n{}",
                rows.join("\n")
            );
        }
    }

    /// **The overlay cannot reach the cover art.**
    ///
    /// Art on kitty and iTerm2 is written outside ratatui's buffer by
    /// [`crate::art::Painter`], positioned by [`art_area`] — so an overlay drawn
    /// over those cells would have the image on top of it with nothing in the
    /// buffer model to notice. The reason it cannot happen is geometric: every
    /// overlay in this application is drawn inside the *body*, and `art_area` is
    /// inside the *footer*. This asserts that, rather than trusting it.
    #[test]
    fn no_overlay_can_reach_the_cover_art_or_the_seal() {
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let area = Rect::new(0, 0, w, h);
            let art = art_area(area).expect("an art block");
            // The body is everything above the footer's rule.
            let body_bottom = h - 1 - FOOTER_HEIGHT - 1;
            assert!(
                art.y >= body_bottom,
                "the art block at {w}×{h} reaches into the body: {art:?}"
            );
        }
    }

    /// Closed, the picker is not on screen at all.
    #[test]
    fn a_closed_picker_draws_nothing() {
        let rows = render(100, 30, &sealed());
        assert!(!rows.iter().any(|r| r.contains("─ OUTPUT ")));
        assert!(!rows.iter().any(|r| r.contains("System default")));
    }

    /// **A device that is not connected is named, and is not a row.**
    ///
    /// A selectable line for a DAC that is asleep would be the screen claiming it
    /// is there; the sentence says what happened instead, and the `▶` sits on the
    /// device the engine really opened instead of it.
    ///
    /// The stream is restated here rather than taken from [`picking`]: a session
    /// reporting `Topping E30` while `Topping E30` is missing is a state the world
    /// cannot be in, and a fixture that impossible would let the assertion below
    /// pass for the wrong reason. The engine fell back to the system default,
    /// which on this machine is the laptop speakers, and that is what it reports.
    ///
    /// **The device list is restated for the same reason, and this is what the
    /// test used to get wrong.** It kept [`picking`]'s list — which holds
    /// `Topping E30` — and then asserted *both* that the panel prints
    /// `Topping E30 is not connected` and that there is a `Topping E30` row to
    /// look for a marker on. That is the screen contradicting itself, and it is
    /// reachable: the picker re-read the host's list on every open but never
    /// re-resolved the choice against it, so a DAC plugged in after launch got a
    /// row while the enum went on saying it was absent. `App::set_devices` now
    /// resolves the two together, and a missing device is one the list does not
    /// have — so the fixture says so, and the assertion below is that the DAC is
    /// on **no** row at all rather than on an unmarked one.
    #[test]
    fn the_picker_names_a_configured_device_that_is_not_connected() {
        let mut a = picking();
        a.devices = vec!["MacBook Pro Speakers".to_string()];
        a.device = crate::config::OutputDevice::Missing("Topping E30".into());
        a.stream = Some(StreamInfo {
            device: "MacBook Pro Speakers".into(),
            ..stream(44_100, 44_100, 44_100)
        });
        a.device_cursor = 0;
        let rows = render(100, 30, &a);
        assert!(
            rows.iter()
                .any(|r| r.contains("Topping E30 is not connected")),
            "{}",
            rows.join("\n")
        );
        // The in-use marker is on the device the audio is really coming out of.
        let speakers = rows
            .iter()
            .find(|r| r.contains("MacBook Pro Speakers"))
            .expect("a row for the device in use");
        assert!(speakers.contains('▶'), "{speakers}");
        // A sentence *instead of* a row: the only line naming the absent DAC is
        // the sentence itself. A selectable line for it — marked or not — would
        // be the screen claiming it is there while the line under it says it is
        // not.
        assert!(
            !rows
                .iter()
                .any(|r| r.contains("Topping E30") && !r.contains("not connected")),
            "{}",
            rows.join("\n")
        );
        // Nor a "from the next track" line, which would be a promise about
        // hardware that is not plugged in.
        assert!(
            !rows.iter().any(|r| r.contains("from the next track")),
            "{}",
            rows.join("\n")
        );
        // And the seal names the same device the marker is on.
        assert!(
            seal_row(&a).contains("MacBook Pro Speakers"),
            "{}",
            seal_row(&a)
        );
    }

    /// **The picker and the seal can never name two different devices.**
    ///
    /// The general statement, over the three transport states rather than the one
    /// that was reported: whenever the seal's chain names a device — which it does
    /// exactly when the seal is active — the row carrying `▶` is that same device.
    /// The paused case is the one that used to fail: its session stays on the old
    /// device by design, so a `▶` read off the preference named the new one while
    /// the seal, correctly, went on naming the old.
    #[test]
    fn the_picker_never_names_a_different_device_than_the_seal() {
        for (what, playback) in [("playing", Playback::Playing), ("paused", Playback::Paused)] {
            let mut a = picking();
            a.playback = playback;
            // The engine is on the laptop speakers; the *preference* is the DAC,
            // as it is one keypress after choosing it on a paused Deck.
            a.stream = Some(StreamInfo {
                device: "MacBook Pro Speakers".into(),
                ..stream(44_100, 44_100, 44_100)
            });
            a.device = crate::config::OutputDevice::Configured("Topping E30".into());
            let rows = render(100, 30, &a);
            let seal = &rows[rows.len() - 2];
            assert!(
                seal.contains("MacBook Pro Speakers"),
                "{what}: the seal names something else: {seal}"
            );
            assert!(
                rows.iter().any(|r| r.contains("▶ MacBook Pro Speakers")),
                "{what}: ▶ is not on the device the engine opened:\n{}",
                rows.join("\n")
            );
            assert!(
                !rows.iter().any(|r| r.contains("▶ Topping E30")),
                "{what}: ▶ is on a device nothing is coming out of:\n{}",
                rows.join("\n")
            );
            // The choice is not lost — it is a sentence, not a marker.
            assert!(
                rows.iter()
                    .any(|r| r.contains("Topping E30 from the next track")),
                "{what}: the chosen device was taken with nothing to say so:\n{}",
                rows.join("\n")
            );
        }

        // Stopped: no session, so the seal names no device — `— → —` — and there
        // is nothing for the marker to contradict. It falls back to the choice,
        // which is then the only true thing on the screen.
        let mut a = picking();
        a.playback = Playback::Stopped;
        a.stream = None;
        a.device = crate::config::OutputDevice::Configured("Topping E30".into());
        let rows = render(100, 30, &a);
        let seal = &rows[rows.len() - 2];
        assert!(seal.contains("— → —"), "{seal}");
        assert!(!seal.contains("Topping E30"), "{seal}");
        assert!(
            rows.iter().any(|r| r.contains("▶ Topping E30")),
            "{}",
            rows.join("\n")
        );
        assert!(
            !rows.iter().any(|r| r.contains("from the next track")),
            "{}",
            rows.join("\n")
        );
    }

    /// **The green lamp cannot survive a device change.**
    ///
    /// `dev_rate` is one of the three rates `derive` reads, and a switch replaces
    /// the engine session — so between the keypress and the new device opening
    /// there is nothing to describe. The frame in that window must not still be
    /// showing `● BIT-PERFECT` about the device that has just been left.
    #[test]
    fn switching_the_output_device_never_leaves_a_green_lamp() {
        let mut a = picking();
        assert!(seal_row(&a).contains("BIT-PERFECT"), "{}", seal_row(&a));
        a.act(crate::keys::Action::Open);
        let row = seal_row(&a);
        assert!(
            !row.contains("BIT-PERFECT"),
            "the seal survived a device switch: {row}"
        );
        assert!(row.contains("UNVERIFIED"), "{row}");
        let (lamp, colour) = lamp(100, 30, &a);
        assert_eq!(lamp, "○", "the lamp stayed solid across a device switch");
        assert_ne!(
            colour, a.theme.led_green,
            "the lamp was still green with nothing to be green about"
        );
    }

    // ── the sleep timer ──────────────────────────────────────────────────

    /// The remaining time is on the **artist** row, in `mm:ss`, and it is the
    /// fold's own number — this module reads no clock. See [`crate::app::Sleep`].
    ///
    /// It used to sit on the transport row, where it took eleven columns off
    /// the one measurement on the screen that is *about* width. It replaces the
    /// queue position, which is the thing that row carries when no timer runs.
    #[test]
    fn a_running_sleep_timer_shows_its_remaining_time_on_the_artist_row() {
        let mut a = sealed();
        a.act(crate::keys::Action::Sleep);
        a.sleep.as_mut().unwrap().remaining = std::time::Duration::from_secs(29 * 60 + 41);
        let rows = render(100, 30, &a);
        let artist = &rows[26];
        assert!(artist.contains("sleep 29:41"), "{artist}");
        // In amber, which is what makes it a warning rather than a readout.
        let colour = colour_of(&a, 100, 30, 26, "29:41");
        assert_eq!(colour, Some(a.theme.led_amber), "{artist}");
        // It has taken nothing off the scrubber or the seal.
        let scrubber = &rows[27];
        assert!(!scrubber.contains("sleep"), "{scrubber}");
        assert!(scrubber.contains("02:04"), "{scrubber}");
        assert!(scrubber.contains("09:09"), "{scrubber}");
        assert!(rows[28].contains("BIT-PERFECT"), "{}", rows[28]);
    }

    #[test]
    fn no_timer_means_no_sleep_group_at_all() {
        let rows = render(100, 30, &sealed());
        assert!(!rows.iter().any(|r| r.contains("sleep")));
    }

    /// It fits the minimum terminal without taking the clocks or the seal with it.
    #[test]
    fn the_sleep_readout_fits_the_smallest_supported_terminal() {
        let mut a = sealed();
        a.act(crate::keys::Action::Sleep);
        a.sleep.as_mut().unwrap().remaining = std::time::Duration::from_secs(60 * 60);
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let rows = render(w, h, &a);
            let artist = &rows[rows.len() - 4];
            assert!(
                artist.contains("sleep 60:00"),
                "clipped at {w}×{h}: {artist}"
            );
            let scrubber = &rows[rows.len() - 3];
            assert!(scrubber.contains("09:09"), "at {w}×{h}: {scrubber}");
            let seal = &rows[rows.len() - 2];
            assert!(seal.contains("BIT-PERFECT"), "at {w}×{h}: {seal}");
        }
    }

    // ── the footer, corrected ────────────────────────────────────────────

    /// **The scrubber is the whole text width, with a clock at each end.**
    ///
    /// It used to share the row with the transport glyph and, while one ran,
    /// the sleep timer — thirteen columns taken out of the one measurement on
    /// the screen that is *about* width.
    #[test]
    fn the_scrubber_takes_the_whole_text_width_at_every_size() {
        let with_a_timer = || {
            let mut a = sealed();
            a.act(crate::keys::Action::Sleep);
            a
        };

        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            for (what, a) in [("plain", sealed()), ("with a timer", with_a_timer())] {
                let rows = render(w, h, &a);
                let row = &rows[rows.len() - 3];
                // The text column, less the two clocks and the space beside
                // each: `mm:ss ` and ` mm:ss`.
                let text_w = usize::from(w)
                    - 2
                    - 2 * usize::from(GUTTER)
                    - usize::from(footer::ART_WIDTH)
                    - usize::from(footer::ART_GAP);
                let text: String = row
                    .chars()
                    .skip(1 + usize::from(GUTTER + footer::ART_WIDTH + footer::ART_GAP))
                    .take(text_w)
                    .collect();
                // Two clocks and a bar. Nothing else is on the row at all.
                assert!(
                    text.chars()
                        .all(|c| matches!(c, '━' | '─' | ' ' | ':') || c.is_ascii_digit()),
                    "{what} at {w}×{h}: {row}"
                );
                let bar = text.chars().filter(|c| matches!(c, '━' | '─')).count();
                assert_eq!(bar, text_w - 12, "{what} at {w}×{h}: {row}");
            }
        }
    }

    /// The artist row's right-hand side: the queue position, and the sleep
    /// timer in its place while one runs.
    ///
    /// It was the one row in the footer with nothing on the right.
    #[test]
    fn the_artist_row_carries_the_queue_position_and_gives_it_up_to_a_timer() {
        let a = playing();
        let artist = &render(100, 30, &a)[26];
        assert!(artist.contains("Aphex Twin"), "{artist}");
        assert!(artist.contains("2 of 3"), "{artist}");
        assert_eq!(
            colour_of(&a, 100, 30, 26, "2 of 3"),
            Some(a.theme.ink_faint),
            "{artist}"
        );

        // A timer takes the slot, and the position steps aside rather than
        // being squeezed in beside it.
        let mut sleeping = playing();
        sleeping.act(crate::keys::Action::Sleep);
        let artist = &render(100, 30, &sleeping)[26];
        assert!(artist.contains("sleep"), "{artist}");
        assert!(!artist.contains("2 of 3"), "{artist}");

        // Nothing playing, nothing queued: no number invented to fill the slot.
        let cold = &render(100, 30, &scanned())[26];
        assert!(!cold.contains(" of "), "{cold}");
    }

    /// **The chips are lowercase. The seal's label is `derive`'s and is not
    /// touched.**
    ///
    /// `◆ EQ` beside a seal reading `EQ · VOLUME` shouts the same word twice at
    /// two different weights, and the one that matters loses. The chips report
    /// the state of a stage; the seal reports a verdict.
    #[test]
    fn the_modifier_chips_are_lowercase_and_the_seal_label_is_left_alone() {
        let row = seal_row(&sealed());
        assert!(row.contains("eq flat"), "{row}");
        assert!(row.contains("rg off"), "{row}");
        assert!(!row.contains("EQ"), "{row}");
        assert!(!row.contains("RG"), "{row}");
        // …and the label beside them is still `derive`'s, in `derive`'s case.
        assert!(row.contains("BIT-PERFECT"), "{row}");

        // Engaged, the chip's own word is still lowered and `rg_label` is not.
        let a = eq_engaged();
        let row = seal_row(&a);
        assert!(row.contains("eq on"), "{row}");
        assert_eq!(row.contains("eq on"), a.seal().flags.eq_active, "{row}");
        assert!(
            row.contains(&a.seal().seal_label),
            "the seal label was re-cased: {row}"
        );
    }

    /// **The scrubber's unplayed track is a control, and reads as one.**
    ///
    /// Step 4 of the plan asked whether `RULE` on the dark ground was genuinely
    /// too dim or whether it only looked empty in a downscaled screenshot. It
    /// was genuinely too dim: **2.03:1**, under the 3:1 WCAG 2.1 SC 1.4.11 asks
    /// of a non-text user-interface component. That is fine for a border, which
    /// SC 1.4.11 does not reach, and not fine for the half of a progress bar
    /// that says how much of the track is left.
    ///
    /// The ratio is computed here rather than written down, so the constant and
    /// the claim about it cannot drift apart.
    #[test]
    fn the_unplayed_scrubber_is_bright_enough_to_be_a_control() {
        use crate::ui::theme::{Rgb, GROUND, INK_3, RULE, SCRUBBER_TRACK};

        fn luminance(c: Rgb) -> f64 {
            let channel = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.040_45 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
        }
        fn contrast(a: Rgb, b: Rgb) -> f64 {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        }

        // The finding, stated as an assertion so it cannot quietly come back.
        let was = contrast(RULE, GROUND);
        assert!(
            was < 3.0,
            "RULE is {was:.2}:1 — the bug is gone on its own?"
        );

        let now = contrast(SCRUBBER_TRACK, GROUND);
        assert!(now >= 3.0, "the scrubber track is {now:.2}:1 on --bg");
        // …and still visibly a track rather than text: the clocks at each end
        // are `--ink-3`, and it is dimmer than they are.
        assert!(
            now < contrast(INK_3, GROUND),
            "the track is as loud as the clocks"
        );

        // And it is what the row is actually drawn in.
        let a = sealed();
        assert_eq!(
            colour_of(&a, 100, 30, 27, "─"),
            Some(a.theme.scrubber_track),
            "the unplayed track is not the colour that was measured"
        );
    }

    // ── the help overlay ──────────────────────────────────────────────────

    /// [`sealed`], with `?` pressed. Built by pressing the key, not by writing
    /// the flag, so the fixture is a state the application can be put in.
    fn helping() -> App {
        let mut a = sealed();
        a.act(crate::keys::Action::Help);
        a
    }

    /// Snapshot: **the whole keyboard on one screen, with the seal still under
    /// it.**
    ///
    /// Every row of this came out of `keys::BINDINGS` via the *same*
    /// `main_pane::key_hints` the empty pane uses — there is no second layout for
    /// the keymap and nothing in `help.rs` spells a key.
    ///
    /// Note the sidebar's `│` rule is **not** running through the middle of the
    /// table. That rule is painted straight into the buffer after the panes, so
    /// the overlay is drawn after it; an overlay drawn beside the EQ and device
    /// panels — which are over the main pane only and never meet that column —
    /// would come back with a stripe down it.
    #[test]
    fn deck_at_100x30_with_the_help_overlay_open() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────────────────────────── Music · 2 albums · 4 tracks ┐",
            "│ SOURCES        │ ‹ SELECTED AMBIENT WORKS 85-92                                                  │",
            "│                │ Aphex Twin · 3 tracks                                                           │",
            "│ ● Local      2 │       ALBUM                     TRACKS     #  TITLE                        TIME │",
            "│ + Add server   │ ─────────────────────────────────────────────────────────────────────────────── │",
            "│ ● Queue      3┌─ KEYS ───────────────────────────────────────────────────────────┐          4:51 │",
            "│               │       q  quit                         s  spectrum on / off       │          9:09 │",
            "│               │     tab  sources ⇄ library            z  now playing             │          3:52 │",
            "│               │   j / ↓  down                         r  rescan · reconnect      │               │",
            "│               │   k / ↑  up                           /  search the server       │               │",
            "│               │   enter  open album · play track      d  output device           │               │",
            "│               │     esc  back                         t  sleep timer             │               │",
            "│               │   space  play / pause                 ?  keys · this list        │               │",
            "│               │       n  next track                   e  EQ panel                │               │",
            "│               │       p  previous track               E  EQ on / off             │               │",
            "│               │       a  add to queue                 h  EQ band left            │               │",
            "│               │       x  remove from queue            l  EQ band right           │               │",
            "│               │   [ / ←  seek back 5s                 ,  EQ preset back          │               │",
            "│               │   ] / →  seek forward 5s              .  EQ preset forward       │               │",
            "│               └──────────────────────────────────────────────────────────────────┘               │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│                │                                                                                 │",
            "│ tab sources    │                                                                                 │",
            "├────────────────┴─────────────────────────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ Tha                                                 ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Aphex Twin · Selected Ambient Works 85-92                                       2 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━━━━━━───────────────────────────────────────────────────────── 09:09 │",
            "│ ░░░░░░░░  FLAC · 44.1 kHz · 24-bit → Topping E30 · 44.1 kHz   ● BIT-PERFECT    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(100, 30, &helping()), expected);
    }

    /// Snapshot: **the minimum supported terminal.**
    ///
    /// Fourteen rows of table into eleven rows of body, so it is cut — and the cut
    /// says so with a `…`, the same way the empty pane's copy does, because it is
    /// the same function doing it. Three columns would fit the whole table here
    /// only by truncating the descriptions, and a description you have to guess the
    /// end of is a worse answer than a row you can see is missing.
    ///
    /// Below this size there is no overlay because there is no Deck: `draw`
    /// renders the "needs 80 × 20" card and returns.
    #[test]
    fn help_overlay_at_the_minimum_terminal_is_cut_and_says_so() {
        let expected = vec![
            "┌─ EKO ─────────────────────────────────────────── Music · 2 albums · 4 tracks ┐",
            "│ SOUR┌─ KEYS ───────────────────────────────────────────────────────────┐     │",
            "│     │       q  quit                         s  spectrum on / off       │     │",
            "│ ● Lo│     tab  sources ⇄ library            z  now playing             │TIME │",
            "│ + Ad│   j / ↓  down                         r  rescan · reconnect      │──── │",
            "│ ● Qu│   k / ↑  up                           /  search the server       │4:51 │",
            "│     │   enter  open album · play track      d  output device           │9:09 │",
            "│     │     esc  back                         t  sleep timer             │3:52 │",
            "│     │   space  play / pause                 ?  keys · this list        │     │",
            "│     │       n  next track                   e  EQ panel                │     │",
            "│     │       p  previous track               E  EQ on / off             │     │",
            "│     │       a  add to queue                 h  EQ band left            │     │",
            "│     │          …                                                       │     │",
            "│ tab └──────────────────────────────────────────────────────────────────┘     │",
            "├────────────────┴─────────────────────────────────────────────────────────────┤",
            "│ ░░░░░░░░  ▶ Tha                             ▁▁▁▂▂▂▂▃▃▃▃▃▄▄▄▄▅▅▅▅▆▆▆▆▆▇▇▇▇███ │",
            "│ ░░░░░░░░  Aphex Twin · Selected Ambient Works 85-92                   2 of 3 │",
            "│ ░░░░░░░░  02:04 ━━━━━━━━━━━━────────────────────────────────────────── 09:09 │",
            "│ ░░░░░░░░  FLAC · 44.1 kHz · 24-bit → …    ● BIT-PERFECT    eq flat    rg off │",
            "└──────────────────────────────────────────────────────────────────────────────┘",
        ];
        assert_eq!(render(MIN_WIDTH, MIN_HEIGHT, &helping()), expected);
    }

    /// **Every binding is on the overlay at 100×30 — from the table.**
    ///
    /// This is the guarantee that moved here out of
    /// [`the_keymap_shrinks_to_fit_instead_of_disappearing`] when the empty pane's
    /// copy of the table ran out of room. A binding added without a description, or
    /// with one too long for two columns, fails here.
    #[test]
    fn the_help_overlay_shows_every_binding_at_a_hundred_by_thirty() {
        let rows = render(100, 30, &helping());
        for binding in crate::keys::BINDINGS {
            assert!(
                rows.iter().any(|r| r.contains(binding.description)),
                "{:?} is missing from the help overlay:\n{}",
                binding.description,
                rows.join("\n")
            );
            assert!(
                rows.iter().any(|r| r.contains(binding.label)),
                "{:?}'s key {:?} is missing from the help overlay:\n{}",
                binding.description,
                binding.label,
                rows.join("\n")
            );
        }
        // Complete, so it must not be claiming to have been cut.
        assert!(
            !rows.iter().any(|r| r.contains('…')),
            "the whole table fitted and the overlay still said it was cut:\n{}",
            rows.join("\n")
        );
    }

    /// **The overlay is never the exception either.** The seal is the last row of
    /// the footer at every usable size, overlay or no overlay — which is why this
    /// is drawn over the *body* rather than over the frame. A full-frame overlay
    /// would fit the entire table at 80×20; it would also cover the one thing this
    /// application guarantees is never covered.
    #[test]
    fn the_help_overlay_never_covers_the_seal_at_any_usable_size() {
        let a = helping();
        for (w, h) in [(80, 20), (80, 60), (100, 30), (200, 24), (240, 80)] {
            let rows = render(w, h, &a);
            let seal = &rows[rows.len() - 2];
            assert!(
                seal.contains("BIT-PERFECT"),
                "the seal at {w}×{h} is wrong or hidden: {seal}"
            );
            // And the seal it shows is the one it would show without the overlay.
            let without = render(w, h, &sealed());
            assert_eq!(
                seal,
                &without[without.len() - 2],
                "the overlay changed the seal row at {w}×{h}"
            );
        }
    }

    /// **It restores cleanly.** Closing it leaves the frame byte-identical to the
    /// one before it was opened — including the seal, on the very frame it closes.
    ///
    /// Nothing here has to be *restored* by hand, and that is the claim: the
    /// overlay only writes ratatui cells inside the body, so ratatui's own diff
    /// repaints them, and the cover art — which is written outside the buffer by
    /// `crate::art::Painter` into the footer's block — was never covered. There is
    /// no `Painter::invalidate` in this path because there is nothing to
    /// invalidate.
    #[test]
    fn closing_the_help_overlay_restores_the_frame_exactly() {
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let mut a = sealed();
            let before = render(w, h, &a);
            a.act(crate::keys::Action::Help);
            assert_ne!(
                render(w, h, &a),
                before,
                "the overlay drew nothing at {w}×{h}"
            );
            a.act(crate::keys::Action::Back);
            assert!(!a.help_open);
            assert_eq!(
                render(w, h, &a),
                before,
                "the frame did not come back at {w}×{h}"
            );
        }
    }

    /// The overlay paints no background of its own either — the rule the whole
    /// console follows. See [`no_cell_in_the_deck_paints_its_own_background`].
    #[test]
    fn the_help_overlay_paints_no_background_of_its_own() {
        let a = helping();
        for (w, h) in [(80, 20), (100, 30), (240, 80)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &a)).unwrap();
            let buf = terminal.backend().buffer();
            for y in 0..h {
                for x in 0..w {
                    assert_eq!(
                        buf[(x, y)].bg,
                        Color::Reset,
                        "at {w}×{h}: cell ({x}, {y}) painted its own background"
                    );
                }
            }
        }
    }

    #[test]
    fn a_closed_help_overlay_draws_nothing() {
        let rows = render(100, 30, &sealed());
        assert!(!rows.iter().any(|r| r.contains("─ KEYS ")));
    }

    /// One overlay at a time, all the way round.
    #[test]
    fn opening_the_help_closes_the_other_overlays() {
        let mut a = picking();
        a.act(crate::keys::Action::Help);
        assert!(a.help_open);
        assert!(!a.device_open);
        let rows = render(100, 30, &a);
        assert!(rows.iter().any(|r| r.contains("─ KEYS ")));
        assert!(
            !rows.iter().any(|r| r.contains("─ OUTPUT ")),
            "both overlays were drawn:\n{}",
            rows.join("\n")
        );

        a.act(crate::keys::Action::EqPanel);
        assert!(a.eq_open);
        assert!(!a.help_open);
        let rows = render(100, 30, &a);
        assert!(rows.iter().any(|r| r.contains("─ EQ ")));
        assert!(!rows.iter().any(|r| r.contains("─ KEYS ")));
    }

    /// While the keymap is up there is no list under it, so the list keys do
    /// nothing rather than moving a cursor nobody can see.
    #[test]
    fn the_overlay_swallows_the_list_keys_rather_than_moving_what_is_hidden() {
        let mut a = helping();
        let cursor = a.cursor();
        let now = a.now.clone();
        a.act(crate::keys::Action::Down);
        a.act(crate::keys::Action::Up);
        a.act(crate::keys::Action::Open);
        assert_eq!(a.cursor(), cursor, "a cursor moved behind the overlay");
        assert_eq!(a.now, now, "enter behind the overlay started a track");
        assert!(a.help_open, "the overlay closed itself");
    }
}
