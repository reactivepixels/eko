//! The main list pane — the album list, and one album's tracks.
//!
//! Two rows of chrome (a heading and a summary), then either a list under its
//! own column headers or, when there is nothing to list, a sentence saying why.
//!
//! ## Albums beside tracks
//!
//! On a terminal of at least [`SPLIT_MIN_WIDTH`] × [`SPLIT_MIN_HEIGHT`] the pane
//! is **two columns**: the albums on the left, the selected album's tracks on
//! the right. Below that it is one column and `enter` / `esc` walk between the
//! two lists, exactly as they did before — and exactly as they still do in the
//! split, where they only decide which of the two columns has the cursor.
//!
//! The threshold is a constant with a test on each of its edges rather than
//! something that emerges from squeezing: a layout that collapses when the
//! arithmetic happens to run out is a layout nobody can predict.
//!
//! ## One row shape, everywhere
//!
//! Every list in this pane — albums, tracks, search results, the queue — is
//! built by [`row_spans`] from an [`Entry`], so all four have the same gutter,
//! the same number field, the same text column and the same right-hand edge.
//! Two things follow that were not true before:
//!
//! * **Two distinct markers.** `▌` in the gutter is *the cursor*; `▶` in the
//!   number field is *what is playing*. They used to both be orange glyphs in
//!   the same cell, which is a screen you have to already know how to read.
//! * **A fixed right edge.** The counts and durations right-align to
//!   [`COLUMN_MAX`] rather than to the frame, so an album's track count is a
//!   readable distance from its title on a 240-column terminal instead of a
//!   hundred and thirty cells away from it.
//!
//! ## Two sources, one shape
//!
//! The local folder and the server are drawn by the same functions in the same
//! rows, and navigated by the same keys — [`crate::keys`] is not forked. What
//! differs is only where the rows come from and what the chrome has to say when
//! there are none: a local library can be unconfigured or missing, a remote one
//! can be unauthenticated, mid-walk, or half-fetched. Each of those is its own
//! sentence, for the reason the empty states already exist — a blank pane that
//! does not say why reads as a broken program.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{
    App, ConnState, DetailState, Focus, RemoteState, ScanState, SearchState, Source, View,
};
use crate::config::{MusicFolder, CONFIG_PATH_HINT};
use crate::library::{format_duration, AUDIO_EXTS, MAX_DEPTH};
use crate::queue::Media;
use crate::ui::footer::fit;
use crate::ui::pane_body;
use crate::ui::theme::Theme;

/// Rows of chrome above an **empty** pane: heading, summary, blank.
const CHROME_ROWS: u16 = 3;

/// Rows of chrome above a list: heading, summary, column headers, rule.
///
/// One more than [`CHROME_ROWS`] because a list has columns to head and an
/// explanation does not. A header row over a sentence is furniture describing
/// nothing.
const LIST_CHROME_ROWS: u16 = 4;

/// The terminal width at and above which the library is two columns.
pub const SPLIT_MIN_WIDTH: u16 = 100;
/// The terminal height at and above which the library is two columns.
///
/// Height matters as much as width: two columns of four rows each is not a
/// library, it is two stubs. The Deck already refuses to draw at all below
/// [`crate::ui::MIN_WIDTH`] × [`crate::ui::MIN_HEIGHT`]; this is the second,
/// softer threshold, and it degrades to something that still works rather than
/// to a card.
pub const SPLIT_MIN_HEIGHT: u16 = 30;

/// Columns between the album column and the track column.
const SPLIT_GAP: u16 = 2;

/// The widest a list column's content may get.
///
/// The right-hand column — a track count, a duration — is right-aligned to
/// **this**, not to the pane's edge. Without it a 240-column terminal puts a
/// four-character number a hundred and thirty cells from the title it belongs
/// to, which is a table the eye cannot follow across.
///
/// Sixty-four leaves forty-two columns of text after [`ROW_PREFIX`], a
/// [`ROW_GAP`] and a six-column count — and it is wider than either half of the
/// split at the smallest terminal the split runs on, so nothing is capped there.
pub const COLUMN_MAX: u16 = 64;

/// `▌`, a space, the two-column number field, and two spaces.
const ROW_PREFIX: usize = 6;
/// The least space between a row's text and its right-aligned column.
const ROW_GAP: usize = 2;

/// Whether a terminal of `area` is big enough for the two-column library.
///
/// Stated as a function of the **terminal**, not of whatever rect the pane ends
/// up with, so the answer is one a user can predict from the size of their
/// window and a test can state without rendering anything.
#[must_use]
pub fn splits(area: Rect) -> bool {
    area.width >= SPLIT_MIN_WIDTH && area.height >= SPLIT_MIN_HEIGHT
}

/// `(album column width, track column x, track column width)` in a body
/// `width` columns wide.
///
/// The two columns divide the body evenly and each is then capped at
/// [`COLUMN_MAX`], so they stay adjacent — the pair hugs the left rather than
/// leaving a hole down the middle of a very wide pane.
fn split_columns(width: u16) -> (u16, u16, u16) {
    let left = (width.saturating_sub(SPLIT_GAP) / 2).min(COLUMN_MAX);
    let right_x = left + SPLIT_GAP;
    (left, right_x, width.saturating_sub(right_x).min(COLUMN_MAX))
}

/// One row of any list in this pane, before it is styled and padded.
///
/// The four lists differ only in what they put in these four fields. Having one
/// struct rather than four `format!`s is what makes the gutter, the number
/// field and the right edge line up across the split — and what makes them
/// impossible to line up differently by accident.
struct Entry {
    /// The two-column number field: a track number, a queue position, or blank.
    /// Replaced by `▶` when this row is the one playing.
    number: String,
    text: String,
    /// Right-aligned at the column's fixed edge.
    trailing: Vec<Span<'static>>,
    playing: bool,
}

/// Which slice of a list to draw so the cursor is on screen.
///
/// Stateless on purpose — there is no scroll offset to keep in sync with the
/// cursor, and therefore no way for the two to disagree. The window pins the
/// cursor to the middle once the list is longer than the pane, and to the ends
/// when it is near one.
#[must_use]
pub fn window(cursor: usize, len: usize, height: usize) -> (usize, usize) {
    if height == 0 || len == 0 {
        return (0, 0);
    }
    if len <= height {
        return (0, len);
    }
    let start = cursor.saturating_sub(height / 2).min(len - height);
    (start, start + height)
}

/// Render the main pane.
///
/// Everything below is written flush left and laid out in [`pane_body`], so the
/// pane's air comes from [`crate::ui::GUTTER`] once instead of from a leading
/// `" "` on each of two dozen `format!`s.
///
/// Nothing here is *clipped* any more. Every row of text — the heading, the
/// summary and each list row — goes through [`fit`], so a title too long for
/// its column ends in `…` rather than simply stopping at whatever cell the rect
/// ran out on. The summary row was the visible case: at 80×20 the unconfigured
/// pane's `no music folder · set music_folder in ~/.config/eko/config.toml` is
/// sixty-three columns in a fifty-nine-column body and was cut with nothing to
/// say it had been. It matters more now that there are two columns, because a
/// clip in the album column would have run into the track column.
pub fn render(frame: &mut Frame, outer: Rect, app: &App) {
    let area = pane_body(outer);
    if area.is_empty() {
        return;
    }
    let theme = &app.theme;
    let width = area.width;
    let source = app.source();

    let (heading, summary) = chrome(app);
    let mut lines = vec![
        Line::from(Span::styled(
            fit(&heading, width as usize),
            theme.accent_strong(),
        )),
        Line::from(Span::styled(
            fit(&summary, width as usize),
            Style::default().fg(theme.ink_faint),
        )),
    ];

    // The split needs an album list to put in its left column. A source that has
    // none — the queue, a search's flat result list — is one column whatever the
    // terminal is, and so is a library with no albums in it yet, because two
    // empty columns say strictly less than one sentence does.
    let split = splits(frame.area())
        && matches!(source, Source::Local | Source::Remote)
        && album_count(app, source) > 0;

    if split {
        let rows = area.height.saturating_sub(LIST_CHROME_ROWS) as usize;
        split_body(app, source, &mut lines, width, rows);
    } else if app.row_count() == 0 {
        lines.push(Line::default());
        let rows = area.height.saturating_sub(CHROME_ROWS) as usize;
        empty_state(app, &mut lines, rows, width as usize);
    } else {
        let rows = area.height.saturating_sub(LIST_CHROME_ROWS) as usize;
        single_body(app, source, &mut lines, width, rows);
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// The heading and the summary — the two rows above every list.
fn chrome(app: &App) -> (String, String) {
    let source = app.source();
    match (source, app.view()) {
        (Source::Local, View::Albums) => (albums_heading(app), summary(app)),
        (Source::Local, View::Album(i)) => match app.library.albums.get(i) {
            Some(album) => (
                format!("‹ {}", album.name.to_uppercase()),
                format!("{} · {} tracks", album.artist, album.tracks.len()),
            ),
            None => (albums_heading(app), summary(app)),
        },
        (Source::Remote, View::Albums) => {
            (app.selected_label().to_uppercase(), remote_summary(app))
        }
        (Source::Remote, View::Album(_)) => match app.open_remote_album() {
            Some(album) => (
                format!("‹ {}", album.name.to_uppercase()),
                remote_album_summary(app, album),
            ),
            None => (app.selected_label().to_uppercase(), remote_summary(app)),
        },
        (Source::Search, View::Albums) => ("SEARCH".to_string(), search_summary(app)),
        (Source::Search, View::Album(_)) => match app.search.open_album() {
            Some(album) => (
                format!("‹ {}", album.name.to_uppercase()),
                search_album_summary(app, album),
            ),
            None => ("SEARCH".to_string(), search_summary(app)),
        },
        (Source::Queue, _) => ("QUEUE".to_string(), queue_summary(app)),
    }
}

// ── the two layouts ──────────────────────────────────────────────────────────

/// One list, under its own headers. The shape the pane has always had.
fn single_body(app: &App, source: Source, lines: &mut Vec<Line<'static>>, width: u16, rows: usize) {
    let theme = &app.theme;
    let w = width.min(COLUMN_MAX);
    let (number, left, right) = headers(app, source);
    lines.push(Line::from(header_spans(theme, number, left, right, w)));
    lines.push(rule_row(theme, w));

    let live = app.focus == Focus::Main;
    let cursor = app.cursor();
    let (start, end) = window(cursor, app.row_count(), rows);
    for i in start..end {
        let Some(entry) = entry_at(app, source, i) else {
            return;
        };
        lines.push(Line::from(row_spans(theme, i == cursor, live, entry, w)));
    }
}

/// Albums on the left, the selected album's tracks on the right.
///
/// **One header row and one rule row across both columns**, so the eye reads
/// straight across the split instead of meeting two separate tables that happen
/// to be side by side.
///
/// Which column holds the cursor is [`App::view`] and nothing else: at
/// [`View::Albums`] the album column is live and the track column is a preview
/// of whatever the cursor is sitting on; at [`View::Album`] the track column is
/// live and the album column shows the open album as selected-but-not-focused.
/// `enter` and `esc` move between them, which is what they already did — they
/// just used to swap one list for another.
fn split_body(app: &App, source: Source, lines: &mut Vec<Line<'static>>, width: u16, rows: usize) {
    let theme = &app.theme;
    let (left_w, right_x, right_w) = split_columns(width);
    let gap = " ".repeat(SPLIT_GAP as usize);

    let mut header = header_spans(theme, "", "ALBUM", "TRACKS", left_w);
    header.push(Span::raw(gap.clone()));
    header.extend(header_spans(theme, "#", "TITLE", "TIME", right_w));
    lines.push(Line::from(header));
    lines.push(rule_row(theme, right_x + right_w));

    let open = match app.view() {
        View::Album(i) => Some(i),
        View::Albums => None,
    };
    let album = open.unwrap_or_else(|| app.cursor());
    let live = app.focus == Focus::Main;
    let (album_start, album_end) = window(album, album_count(app, source), rows);
    let track_cursor = open.map(|_| app.cursor());
    let (track_start, track_end) = window(
        track_cursor.unwrap_or(0),
        track_count(app, source, album),
        rows,
    );

    // A track column with nothing in it says which kind of nothing it is. A
    // blank half-pane beside a full one reads as a rendering fault.
    let note = if track_start == track_end {
        column_note(app, source, album, open.is_some())
    } else {
        Vec::new()
    };

    let used = (album_end - album_start)
        .max(track_end - track_start)
        .max(note.len())
        .min(rows);
    for row in 0..used {
        let mut spans = album_start
            .checked_add(row)
            .filter(|i| *i < album_end)
            .and_then(|i| {
                album_entry(app, source, i)
                    .map(|e| row_spans(theme, i == album, live && open.is_none(), e, left_w))
            })
            .unwrap_or_else(|| blank(left_w));
        spans.push(Span::raw(gap.clone()));
        spans.extend(match note.get(row) {
            Some((text, style)) => note_spans(text, *style, right_w),
            None => track_start
                .checked_add(row)
                .filter(|i| *i < track_end)
                .and_then(|i| {
                    track_entry(app, source, album, i).map(|e| {
                        let selected = track_cursor == Some(i);
                        row_spans(theme, selected, live && open.is_some(), e, right_w)
                    })
                })
                .unwrap_or_else(|| blank(right_w)),
        });
        lines.push(Line::from(spans));
    }
}

/// What to put in the track column when there is no track list to put there.
///
/// **An open album with nothing in it gets [`empty_message`] verbatim** — the
/// same sentences the single-column pane would have shown, at the column's
/// width. The copy for "the server is still fetching this", "this album is
/// genuinely empty" and "the fetch failed, here is why" is written once, and
/// the split does not get a second, terser vocabulary of its own that could
/// drift from it.
///
/// A column that is only a *preview* — the cursor is in the album column,
/// nothing has been opened — gets one dim line instead, because the reader has
/// not asked about this album yet and a four-line explanation of an album they
/// are merely passing over is noise.
fn column_note(app: &App, source: Source, album: usize, open: bool) -> Vec<(String, Style)> {
    if open && app.row_count() == 0 {
        return empty_message(app)
            .into_iter()
            .map(|line| {
                let style = line.spans.first().map_or_else(Style::default, |s| s.style);
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                (text, style)
            })
            .collect();
    }
    track_note(app, source, album)
        .map(|text| vec![(text, Style::default().fg(app.theme.ink_faint))])
        .unwrap_or_default()
}

// ── the row, and the two things that mark it ─────────────────────────────────

/// `▌ ▶  Xtal                                              04:51`
///
/// Exactly `width` columns wide, always, so a caller can lay two of these side
/// by side without either bleeding into the other.
///
/// The two markers are deliberately in different places and different shapes.
/// `▌` occupies the gutter and means *the cursor is here*; `▶` replaces the
/// number and means *this is what is coming out of the speakers*. Before this
/// they were the same colour in the same cell — `▶` for playing, `▸` for
/// selected — and telling a playing row from a selected one meant comparing two
/// glyphs that differ by a serif.
fn row_spans(
    theme: &Theme,
    selected: bool,
    live: bool,
    entry: Entry,
    width: u16,
) -> Vec<Span<'static>> {
    let w = width as usize;
    let trailing_w: usize = entry.trailing.iter().map(Span::width).sum();
    let room = w.saturating_sub(ROW_PREFIX + trailing_w + ROW_GAP);
    let field = if entry.playing {
        "▶"
    } else {
        entry.number.as_str()
    };
    let mut spans = vec![
        Span::styled(
            if selected { "▌" } else { " " },
            row_style(theme, true, live),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{field:>2}"),
            Style::default().fg(if entry.playing {
                theme.accent
            } else {
                theme.ink_faint
            }),
        ),
        Span::raw("  "),
        Span::styled(fit(&entry.text, room), row_style(theme, selected, live)),
    ];
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(" ".repeat(w.saturating_sub(used + trailing_w))));
    spans.extend(entry.trailing);
    spans
}

/// `width` columns of nothing — the shorter column's rows in the split.
fn blank(width: u16) -> Vec<Span<'static>> {
    vec![Span::raw(" ".repeat(width as usize))]
}

/// A sentence where a list would be, indented to the text column. See
/// [`column_note`].
fn note_spans(note: &str, style: Style, width: u16) -> Vec<Span<'static>> {
    let w = width as usize;
    let text = fit(note, w.saturating_sub(ROW_PREFIX));
    let mut spans = vec![
        Span::raw(" ".repeat(ROW_PREFIX.min(w))),
        Span::styled(text, style),
    ];
    let used: usize = spans.iter().map(Span::width).sum();
    spans.push(Span::raw(" ".repeat(w.saturating_sub(used))));
    spans
}

/// The column headers for one list column, `width` columns wide exactly.
fn header_spans(
    theme: &Theme,
    number: &str,
    left: &str,
    right: &str,
    width: u16,
) -> Vec<Span<'static>> {
    let w = width as usize;
    let style = Style::default()
        .fg(theme.ink_faint)
        .add_modifier(Modifier::BOLD);
    let head = fit(&format!("  {number:>2}  {left}"), w);
    let head_w = Span::raw(&head).width();
    // The right-hand header is dropped rather than overlapped when the column is
    // too narrow to hold both, for the reason `justified` drops one: half a
    // header is worse than none.
    let right = if head_w + Span::raw(right).width() <= w {
        right
    } else {
        ""
    };
    let mut spans = vec![Span::styled(head, style)];
    spans.push(Span::raw(
        " ".repeat(w.saturating_sub(head_w + Span::raw(right).width())),
    ));
    spans.push(Span::styled(right.to_string(), style));
    spans
}

/// The rule under the headers. One row, shared by both columns.
fn rule_row(theme: &Theme, width: u16) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width as usize),
        Style::default().fg(theme.rule),
    ))
}

/// `(number-field header, left header, right header)` for the list on screen.
///
/// A search's result list gets no right-hand header: its last column is a track
/// count on an album row and a duration on a song row, and there is no one word
/// that is true of both. An absent header is honest; a wrong one is not.
fn headers(app: &App, source: Source) -> (&'static str, &'static str, &'static str) {
    match (source, app.view()) {
        (Source::Local | Source::Remote, View::Albums) => ("", "ALBUM", "TRACKS"),
        (Source::Search, View::Albums) => ("", "RESULT", ""),
        (Source::Queue, _) | (_, View::Album(_)) => ("#", "TITLE", "TIME"),
    }
}

fn albums_heading(app: &App) -> String {
    match app.library.root_name() {
        Some(name) => format!("{} · {}", app.selected_label().to_uppercase(), name),
        None => app.selected_label().to_uppercase(),
    }
}

/// The line under the heading for the server — the one place its state is
/// spelled out.
///
/// The connection comes first, because a library that cannot load because nobody
/// has signed in is not the same problem as one that loaded nothing.
fn remote_summary(app: &App) -> String {
    match &app.conn {
        // Reachable since the sidebar grew an `+ Add server` row: this is the
        // heading someone sees on a first run, before anything is configured.
        ConnState::NotConfigured => "nothing configured · enter adds one".to_string(),
        ConnState::NeedsPassword { .. } => "no password stored · enter to sign in".to_string(),
        ConnState::Connecting { .. } => "connecting…".to_string(),
        ConnState::Failed { message, .. } => message.clone(),
        ConnState::Connected { username, .. } => match &app.remote_state {
            RemoteState::Idle => format!("connected as {username}"),
            RemoteState::Loading => {
                format!("loading albums · {} so far", app.remote.albums.len())
            }
            RemoteState::Ready if app.remote.is_empty() => {
                "no albums on the server · r to try again".to_string()
            }
            RemoteState::Ready => format!("{} albums", app.remote.albums.len()),
            // The count is still shown: the walk stopped early, and what it did
            // fetch is real.
            RemoteState::Failed(message) => {
                format!(
                    "{} albums · incomplete · {message}",
                    app.remote.albums.len()
                )
            }
        },
    }
}

/// The line under `QUEUE`.
///
/// Says how many, and which one is playing. "3 of 12" is the fact a queue is
/// actually consulted for.
fn queue_summary(app: &App) -> String {
    if app.queue.is_empty() {
        return "nothing queued".to_string();
    }
    let n = app.queue.len();
    let tracks = if n == 1 {
        "1 track".to_string()
    } else {
        format!("{n} tracks")
    };
    match app.queue.position() {
        Some(at) => format!("{tracks} · playing {} of {n}", at + 1),
        None => tracks.to_string(),
    }
}

/// The line under `SEARCH` — and, while the input is open, **the input itself**.
///
/// The query is typed into the summary row rather than into a popup. A modal over
/// the list would hide the results the query is about, and this Deck has no modals
/// at all: the footer's four rows belong to the transport and the seal, and the
/// one row that already describes the pane's state is the honest place for the
/// thing that changes it.
fn search_summary(app: &App) -> String {
    let name = app.source_label(Source::Remote);
    if let Some(input) = &app.search.input {
        // A block after the text, because a terminal cursor is hidden for the
        // life of the Deck (`tui::init` sends `Hide`) and a text field with no
        // caret does not look like one.
        return format!("/{input}▏");
    }
    match &app.search.state {
        SearchState::Idle => format!("press / to search {name}"),
        SearchState::Searching => format!("searching {name} for {}…", app.search.query),
        SearchState::Ready if app.search.results.is_empty() => {
            format!("nothing matched {}", app.search.query)
        }
        SearchState::Ready => format!(
            "{} · {} albums · {} songs",
            app.search.query,
            app.search.results.albums.len(),
            app.search.results.songs.len()
        ),
        SearchState::Failed(message) => message.clone(),
    }
}

/// The line under an open *result* album's name.
fn search_album_summary(app: &App, album: &crate::remote::Album) -> String {
    match (&app.search.detail, &album.tracks) {
        (DetailState::Loading, _) => format!("{} · loading tracks…", album.artist),
        (DetailState::Failed(message), _) => format!("{} · {message}", album.artist),
        (DetailState::Idle, Some(tracks)) => {
            format!("{} · {} tracks", album.artist, tracks.len())
        }
        (DetailState::Idle, None) => album.artist.clone(),
    }
}

/// The track id the transport is actually holding, when it is a remote one.
///
/// The search pane marks its rows by **id**, not by index. It has to: a result is
/// at no position in either library, so its queue entry carries
/// [`crate::queue::NO_ROW`] and the positional comparison every other pane uses
/// would never fire. Identity is the better test anyway — it cannot light the
/// wrong row even in principle.
fn playing_remote_id(app: &App) -> Option<&str> {
    app.now.as_ref()?;
    match &app.queue.current()?.media {
        Media::Remote(id) => Some(id.as_str()),
        Media::Local(_) => None,
    }
}

/// The results: every matched album, then every matched song, in one flat list.
///
/// **No section headings.** The cursor walks this list by index and every row has
/// to be a row you can land on; a heading in the middle is either a row that
/// refuses the cursor — the thing `NOT YET` exists to keep out of the sidebar — or
/// an off-by-one waiting to happen. So the kind is a *column* instead: `album` or
/// `song`, in the same dim right-hand position the Queue pane names its source in.
fn search_entry(app: &App, i: usize) -> Option<Entry> {
    let theme = &app.theme;
    let playing = playing_remote_id(app);
    let albums = app.search.results.albums.len();
    let (text, kind, trailing, is_playing) = if i < albums {
        let album = app.search.results.albums.get(i)?;
        // An album row can only claim to be playing once its tracks are in
        // hand — before that it genuinely does not know what is in it.
        let holds = album
            .tracks
            .as_ref()
            .is_some_and(|tracks| playing.is_some_and(|id| tracks.iter().any(|t| t.id == id)));
        (
            format!("{} — {}", album.artist, album.name),
            "album",
            album
                .track_count()
                .map_or_else(String::new, |n| n.to_string()),
            holds,
        )
    } else {
        let (_, song) = app.search.song_at(i)?;
        (
            format!("{} — {}", song.title, song.artist),
            "song",
            format_duration(song.duration),
            playing.is_some_and(|id| id == song.id),
        )
    };
    Some(Entry {
        number: String::new(),
        text,
        trailing: vec![
            Span::styled(kind, Style::default().fg(theme.ink_dim)),
            Span::styled(
                format!("  {trailing}"),
                Style::default().fg(theme.ink_faint),
            ),
        ],
        playing: is_playing,
    })
}

/// One *result* album's tracks. The shape of [`track_entry`], marked by id.
fn search_track_entry(app: &App, i: usize) -> Option<Entry> {
    let track = app
        .search
        .open_album()
        .and_then(|a| a.tracks.as_ref())?
        .get(i)?;
    Some(Entry {
        number: number(track.track_no),
        text: track.title.clone(),
        trailing: vec![Span::styled(
            format_duration(track.duration),
            Style::default().fg(app.theme.ink_faint),
        )],
        playing: playing_remote_id(app).is_some_and(|id| id == track.id),
    })
}

/// What to say when the results list is empty.
///
/// Six states again, and the same rule as everywhere else in this file: name the
/// fact, then the one key that changes it. "Nobody has searched yet" and "the
/// server matched nothing" are the two that would otherwise look identical, and
/// they are the two with completely different next moves.
fn search_empty_message(app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let fact = Style::default().fg(theme.ink);
    let detail = Style::default().fg(theme.ink_faint);
    let say = |text: String, style: Style| Line::from(Span::styled(text, style));
    let blank = || Line::default();
    let name = app.source_label(Source::Remote).to_string();

    // An opened result album with nothing in it is about the album, not the query.
    if let (View::Album(_), Some(album)) = (app.search.view, app.search.open_album()) {
        return match (&app.search.detail, &album.tracks) {
            (DetailState::Loading, _) => vec![say(format!("Loading {}.", album.name), detail)],
            (DetailState::Failed(message), _) => vec![
                say(format!("Could not load {}.", album.name), fact),
                blank(),
                say(message.clone(), detail),
                say("Press esc to go back.".to_string(), detail),
            ],
            (DetailState::Idle, Some(_)) => vec![
                say(format!("{} has no tracks.", album.name), fact),
                blank(),
                say(
                    format!("{name} matched it, but the album is empty on the server."),
                    detail,
                ),
            ],
            (DetailState::Idle, None) => vec![
                say(format!("{} has not been loaded.", album.name), fact),
                blank(),
                say(
                    "Press r to reconnect, then open it again.".to_string(),
                    detail,
                ),
            ],
        };
    }

    if app.search.input.is_some() {
        return vec![
            say(format!("Searching {name}."), fact),
            blank(),
            say("Type a query and press enter.".to_string(), detail),
            say("Press esc to leave it as it was.".to_string(), detail),
        ];
    }

    match &app.search.state {
        SearchState::Idle => vec![
            say(format!("Nothing asked of {name} yet."), fact),
            blank(),
            say(
                "Press / and type. Albums and songs come back".to_string(),
                detail,
            ),
            say(
                "in one list — enter opens an album, or plays a song.".to_string(),
                detail,
            ),
        ],
        SearchState::Searching => vec![say(
            format!("Asking {name} about {}.", app.search.query),
            detail,
        )],
        SearchState::Ready => vec![
            say(
                format!("{name} has nothing matching {}.", app.search.query),
                fact,
            ),
            blank(),
            say(
                "The server answered, so that is its whole answer.".to_string(),
                detail,
            ),
            say("Press / to ask something else.".to_string(), detail),
        ],
        SearchState::Failed(message) => vec![
            say(format!("Could not search {name}."), fact),
            blank(),
            say(message.clone(), detail),
            say(
                "Press r to reconnect, then / to try again.".to_string(),
                detail,
            ),
        ],
    }
}

/// The line under an open remote album's name.
fn remote_album_summary(app: &App, album: &crate::remote::Album) -> String {
    match (&app.remote_detail, &album.tracks) {
        (DetailState::Loading, _) => format!("{} · loading tracks…", album.artist),
        (DetailState::Failed(message), _) => format!("{} · {message}", album.artist),
        (DetailState::Idle, Some(tracks)) => {
            format!("{} · {} tracks", album.artist, tracks.len())
        }
        // Idle with nothing fetched: the album was opened while disconnected.
        (DetailState::Idle, None) => album.artist.clone(),
    }
}

/// The line under the heading — the one place the scan's state is spelled out.
fn summary(app: &App) -> String {
    match &app.scan {
        ScanState::Unconfigured => {
            format!("no music folder · set music_folder in {CONFIG_PATH_HINT}")
        }
        ScanState::Discovering { found } => format!("scanning · {found} files found"),
        ScanState::Reading { done, total } => format!("reading tags · {done} of {total}"),
        ScanState::Missing(path) => format!("music_folder is missing · {}", path.display()),
        ScanState::Failed(message) => message.clone(),
        // An empty library is a normal, nameable state — not an error, and not
        // a spinner that never stops.
        ScanState::Ready if app.library.is_empty() => "no audio files found · r to rescan".into(),
        ScanState::Ready => format!(
            "{} albums · {} tracks",
            app.library.albums.len(),
            app.library.track_count()
        ),
    }
}

/// The pane with nothing in it.
///
/// A blank list has a small, known set of causes and each has exactly one fix,
/// so the pane names the cause and the fix. It used to print the keymap and
/// nothing else — which is how a Deck that had never been pointed at a folder
/// came to look like a Deck that was ignoring the keyboard: fifteen bindings on
/// screen, every one of them inert, and not a word about why.
///
/// The message comes first and always fits. The keymap takes whatever is left.
///
/// # The rule this used to have, and why it had to go
///
/// It was all-or-nothing: `message + 1 + BINDINGS.len() <= height`, or no keymap
/// at all. That was defensible at fifteen bindings and stopped being defensible
/// the moment there were more, because *the number of keys is not something the
/// user changed*. Task 4 added two and a four-line message lost its keymap; this
/// task adds a third. The owner had already reported the keymap vanishing once.
/// A rule under which every new feature makes the help less likely to appear is
/// a rule that ends with no help.
///
/// So: [`key_hints`] lays the table out in **two columns** when the pane is wide
/// enough — which halves the rows it needs — and prints as many of those rows as
/// fit, ending with a `…` when it had to stop early. The keymap now shrinks
/// instead of disappearing, and the one thing it must never do is *lie*: a
/// truncated list says so on its last row rather than reading as the whole
/// keyboard.
fn empty_state(app: &App, lines: &mut Vec<Line<'static>>, height: usize, width: usize) {
    let message = empty_message(app);
    if message.is_empty() {
        key_hints(app, lines, height, width);
        return;
    }
    // One blank row between the message and the keys, and at least two rows of
    // keys — a single row of a seventeen-row table is noise, not help.
    let spare = height.saturating_sub(message.len() + 1);
    lines.extend(message);
    if spare >= 2 {
        lines.push(Line::default());
        key_hints(app, lines, spare, width);
    }
}

/// What to say, per state. Empty means "nothing worth saying" — the keymap
/// then stands in, as it did before.
///
/// Written as the product, not as an apology: state the fact, then the one
/// thing to do about it. Every path is named in full, because a path is the
/// only part of these messages the user can act on.
fn empty_message(app: &App) -> Vec<Line<'static>> {
    match app.source() {
        Source::Local => local_empty_message(app),
        Source::Remote => remote_empty_message(app),
        Source::Search => search_empty_message(app),
        Source::Queue => queue_empty_message(app),
    }
}

/// What to say when the queue is empty.
///
/// It has exactly one cause and two fixes, and both fixes are a key — so it names
/// them. An empty pane here would be the same mistake as the empty library: a
/// screen that looks like a broken list rather than one nobody has filled.
fn queue_empty_message(app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let fact = Style::default().fg(theme.ink);
    let detail = Style::default().fg(theme.ink_faint);
    let say = |text: &str, style: Style| Line::from(Span::styled(text.to_string(), style));
    vec![
        say("Nothing queued.", fact),
        Line::default(),
        say("Play a track and the rest of its album queues up", detail),
        say("behind it. Press a on an album or a track to add", detail),
        say("it to the end, from either library.", detail),
    ]
}

/// What to say when the *server's* list is empty.
///
/// Six states, six sentences, each naming the one thing to do about it. The
/// alternative — a blank pane, or the keymap — is what made an unconfigured local
/// library read as a broken program, and a server has strictly more ways to be
/// empty than a folder does.
fn remote_empty_message(app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let fact = Style::default().fg(theme.ink);
    let detail = Style::default().fg(theme.ink_faint);
    let literal = Style::default().fg(theme.accent);
    let say = |text: String, style: Style| Line::from(Span::styled(text, style));
    let blank = || Line::default();
    let name = app.selected_label().to_string();

    // An open album with nothing in it is about the album, not the server.
    if let (View::Album(_), Some(album)) = (app.remote_view, app.open_remote_album()) {
        return match (&app.remote_detail, &album.tracks) {
            (DetailState::Loading, _) => vec![say(format!("Loading {}.", album.name), detail)],
            (DetailState::Failed(message), _) => vec![
                say(format!("Could not load {}.", album.name), fact),
                blank(),
                say(message.clone(), detail),
                say(
                    "Press esc to go back, or r to reconnect.".to_string(),
                    detail,
                ),
            ],
            (DetailState::Idle, Some(_)) => vec![
                say(format!("{} has no tracks.", album.name), fact),
                blank(),
                say(
                    format!("{name} lists it, but the album is empty on the server."),
                    detail,
                ),
            ],
            (DetailState::Idle, None) => vec![
                say(format!("{} has not been loaded.", album.name), fact),
                blank(),
                say(
                    "Press r to reconnect, then open it again.".to_string(),
                    detail,
                ),
            ],
        };
    }

    match &app.conn {
        // **The first-run screen, and the one that used to be blank.** A Deck
        // with no `[[servers]]` in its config had no server row, no sentence and
        // nothing to press; the only way to connect to Navidrome was to quit,
        // hand-write a TOML file and run a second command. The fix is a
        // keystroke, and this is where it is named.
        ConnState::NotConfigured => vec![
            say("No server yet.".to_string(), fact),
            blank(),
            say(
                "Press enter on + Add server in the sources column.".to_string(),
                detail,
            ),
            blank(),
            say(
                "It asks for a name, an address and a user; the password comes".to_string(),
                detail,
            ),
            say(
                "last, on a plain terminal, and goes to the system keychain.".to_string(),
                detail,
            ),
        ],
        ConnState::NeedsPassword { .. } => vec![
            say(format!("No password stored for {name}."), fact),
            blank(),
            say(
                format!("Press enter on {name} in the sources column."),
                detail,
            ),
            blank(),
            say(
                "It is typed on a plain terminal, never echoed, and never".to_string(),
                detail,
            ),
            say(
                "written to a file. From a password manager, instead:".to_string(),
                detail,
            ),
            blank(),
            say(
                format!("    pass show music | eko-cli login {name}"),
                literal,
            ),
        ],
        ConnState::Connecting { .. } => vec![say(format!("Talking to {name}."), detail)],
        ConnState::Failed { message, .. } => vec![
            say(format!("{name} did not answer."), fact),
            blank(),
            say(message.clone(), detail),
            say(
                format!("Check {name} in {CONFIG_PATH_HINT}, then press r."),
                detail,
            ),
        ],
        ConnState::Connected { .. } => match &app.remote_state {
            RemoteState::Idle => vec![say(format!("Connected to {name}."), detail)],
            RemoteState::Loading => vec![say(format!("Reading {name}'s albums."), detail)],
            RemoteState::Ready => vec![
                say(format!("No albums on {name}."), fact),
                blank(),
                say(
                    "The connection worked, so this is what the server sent.".to_string(),
                    detail,
                ),
                say(
                    "Check the library has been scanned there, then press r.".to_string(),
                    detail,
                ),
            ],
            // Failed with nothing at all: the very first page never landed.
            RemoteState::Failed(message) => vec![
                say(format!("Could not read {name}'s albums."), fact),
                blank(),
                say(message.clone(), detail),
                say("Press r to try again.".to_string(), detail),
            ],
        },
    }
}

/// What to say when the *local* list is empty. Unchanged from the previous
/// phase; see [`empty_state`].
fn local_empty_message(app: &App) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let fact = Style::default().fg(theme.ink);
    let detail = Style::default().fg(theme.ink_faint);
    let literal = Style::default().fg(theme.accent);
    let say = |text: String, style: Style| Line::from(Span::styled(text, style));
    let blank = || Line::default();

    match &app.scan {
        // The scan speaks for itself in the summary row; this just names what
        // is being read, so a long scan is obviously a scan of *something*.
        ScanState::Discovering { .. } | ScanState::Reading { .. } => app
            .folder
            .path()
            .map(|root| vec![say(format!("Reading {}.", root.display()), detail)])
            .unwrap_or_default(),

        ScanState::Unconfigured => match &app.folder {
            MusicFolder::Unset { probed } => {
                let mut out = vec![say("No music folder.".to_string(), fact), blank()];
                if let Some(probed) = probed {
                    // Short on purpose: at the minimum 80 columns the pane
                    // body is 59 wide, and a sentence that clips is a sentence
                    // that has to be guessed at. See
                    // `the_empty_states_fit_the_smallest_supported_terminal`.
                    out.push(say(
                        format!("There is no {} to fall back to.", probed.display()),
                        detail,
                    ));
                }
                out.push(say(format!("Put this in {CONFIG_PATH_HINT}:"), detail));
                out.push(blank());
                out.push(say(
                    r#"    music_folder = "/path/to/music""#.to_string(),
                    literal,
                ));
                out.push(blank());
                out.push(say("Then press r to scan it.".to_string(), detail));
                out
            }
            // A folder was resolved but no scan has started: the frame between
            // `App::new` and `App::attach`, which the event loop never draws.
            MusicFolder::Configured(_) | MusicFolder::Platform(_) => Vec::new(),
        },

        ScanState::Missing(path) => vec![
            say(format!("{} does not exist.", path.display()), fact),
            blank(),
            say(
                format!("That path is music_folder in {CONFIG_PATH_HINT}."),
                detail,
            ),
            say(
                "Fix it there, or reconnect the drive, then press r.".to_string(),
                detail,
            ),
        ],

        // Scanned, and genuinely nothing in it. Naming the formats is the
        // difference between "my library is broken" and "ah, those are .dsf".
        ScanState::Ready if app.library.is_empty() => {
            let root = app.library.root.as_deref().or_else(|| app.folder.path());
            let mut out = vec![match root {
                Some(root) => say(format!("No audio in {}.", root.display()), fact),
                None => say("No audio found.".to_string(), fact),
            }];
            out.push(blank());
            out.push(say(
                format!("EKO reads these, up to {MAX_DEPTH} folders deep:"),
                detail,
            ));
            out.push(say(format!("    {}", audio_extensions()), literal));
            out.push(blank());
            out.push(say(
                "Add some there, or point music_folder somewhere else in".to_string(),
                detail,
            ));
            out.push(say(
                format!("{CONFIG_PATH_HINT}, then press r to rescan."),
                detail,
            ));
            out
        }

        // The summary row already carries the scanner's own words; repeating
        // them here would be volume, not information.
        ScanState::Failed(_) => vec![say(
            "The scan did not finish. Press r to try again.".to_string(),
            detail,
        )],

        // A library with albums in it, but an empty list — an album with no
        // tracks. Nothing to explain, so the keymap stands.
        ScanState::Ready => Vec::new(),
    }
}

/// The readable formats, straight from [`AUDIO_EXTS`], so the sentence cannot
/// promise a format the walk skips or omit one it accepts.
fn audio_extensions() -> String {
    AUDIO_EXTS
        .iter()
        .map(|ext| format!(".{ext}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Width of the label column in the key hints. The widest label is `j / ↓`.
const HINT_LABEL: usize = 7;
/// The label column and the two spaces after it, before the description starts.
///
/// It used to carry a leading space of its own as well. That space was the
/// pane's gutter wearing the table's clothes, and it is now
/// [`crate::ui::GUTTER`] — which is also why the help overlay is two columns
/// narrower than it was: it was padding twice and only saying so once.
const HINT_GUTTER: usize = HINT_LABEL + 2;

/// How wide one hint column has to be: the gutter plus the longest description.
///
/// Measured off [`crate::keys::BINDINGS`] rather than written down, so adding a
/// binding with a longer description cannot silently make the second column
/// overflow into the pane's right edge.
///
/// `pub(crate)` because [`crate::ui::help`] lays out the *same* table and must
/// choose its own width from the same measurement. Two column-width arithmetics
/// over one table is exactly the drift this function exists to remove.
pub(crate) fn hint_column_width() -> usize {
    let widest = crate::keys::BINDINGS
        .iter()
        .map(|b| b.description.chars().count())
        .max()
        .unwrap_or(0);
    HINT_GUTTER + widest
}

/// With no list to show, show the keyboard instead.
///
/// Read straight out of [`crate::keys::BINDINGS`] — the same table `on_key`
/// dispatches through, so this cannot document a key the app does not have.
///
/// Two columns when `width` has room for two, one otherwise; at most `height`
/// rows, and a `…` on the last one when there were more. See [`empty_state`].
///
/// `pub(crate)` because [`crate::ui::help`] renders the same table and uses this
/// to do it. **One layout for the keymap, in one place**: the overlay is not a
/// second, prettier rendering that could disagree with this one about how many
/// columns fit or whether a row was dropped. It only chooses a bigger `height`
/// and `width` to hand in.
pub(crate) fn key_hints(app: &App, lines: &mut Vec<Line<'static>>, height: usize, width: usize) {
    let theme = &app.theme;
    let bindings = crate::keys::BINDINGS;
    let columns = if width >= hint_column_width() * 2 {
        2
    } else {
        1
    };
    let rows = bindings.len().div_ceil(columns);
    let shown = rows.min(height);
    // Column-major, so a two-column table still reads top-to-bottom on the left
    // before it reads the right — the order [`crate::keys`] documents.
    let truncated = shown < rows;
    let label = Style::default().fg(theme.ink_dim);
    let text = Style::default().fg(theme.ink_faint);
    for row in 0..shown {
        if truncated && row + 1 == shown {
            lines.push(Line::from(Span::styled(
                format!("{}…", " ".repeat(HINT_GUTTER)),
                label,
            )));
            break;
        }
        let mut spans = Vec::new();
        for column in 0..columns {
            let Some(binding) = bindings.get(column * rows + row) else {
                continue;
            };
            if column > 0 {
                let used: usize = spans.iter().map(Span::width).sum();
                spans.push(Span::raw(
                    " ".repeat(hint_column_width().saturating_sub(used)),
                ));
            }
            spans.push(Span::styled(
                format!("{:>width$}  ", binding.label, width = HINT_LABEL),
                label,
            ));
            spans.push(Span::styled(binding.description, text));
        }
        lines.push(Line::from(spans));
    }
}

/// The index of the album playing **out of `source`'s library**, if any.
///
/// The source check is what stops the `▶` landing on the wrong row. `album` is
/// an index into one of two independently-ordered lists, so a remote track
/// playing at index 3 would otherwise put a `▶` on local album 3 as well — a
/// screen claiming to play something it is not.
fn playing_album(app: &App, source: Source) -> Option<usize> {
    app.now
        .as_ref()
        .filter(|n| n.source == source)
        .map(|n| n.album)
}

/// The index of the playing track when `album` of `source` is the open one.
fn playing_track(app: &App, source: Source, album: usize) -> Option<usize> {
    app.now
        .as_ref()
        .filter(|n| n.source == source && n.album == album)
        .map(|n| n.track)
}

/// The two-column number field's contents, or blank when there is no number.
fn number(track_no: Option<u32>) -> String {
    track_no.map_or_else(String::new, |n| n.to_string())
}

/// How many albums `source` has. Zero for the sources that are not album lists.
fn album_count(app: &App, source: Source) -> usize {
    match source {
        Source::Local => app.library.albums.len(),
        Source::Remote => app.remote.albums.len(),
        Source::Search | Source::Queue => 0,
    }
}

/// One album row, from either library.
///
/// The two libraries used to have a function each, identical but for the getter
/// and for the server's `None` track count. One function reading `source` is
/// what keeps the album column the same shape whichever source is open — and
/// `playing_album` is gated on `source`, so a local track at index 3 cannot
/// light remote album 3.
fn album_entry(app: &App, source: Source, i: usize) -> Option<Entry> {
    let faint = Style::default().fg(app.theme.ink_faint);
    let (artist, name, count) = match source {
        Source::Local => {
            let album = app.library.albums.get(i)?;
            (
                album.artist.clone(),
                album.name.clone(),
                album.tracks.len().to_string(),
            )
        }
        Source::Remote => {
            let album = app.remote.album(i)?;
            // Nothing is claimed when the server did not say how many tracks
            // there are — an invented `0` beside an album with music in it is
            // worse than a blank column.
            (
                album.artist.clone(),
                album.name.clone(),
                album
                    .track_count()
                    .map_or_else(String::new, |n| n.to_string()),
            )
        }
        Source::Search | Source::Queue => return None,
    };
    Some(Entry {
        number: String::new(),
        text: format!("{artist} — {name}"),
        trailing: vec![Span::styled(count, faint)],
        playing: playing_album(app, source) == Some(i),
    })
}

/// How many tracks album `album` of `source` has in hand.
fn track_count(app: &App, source: Source, album: usize) -> usize {
    match source {
        Source::Local => app.library.albums.get(album).map_or(0, |a| a.tracks.len()),
        Source::Remote => app
            .remote
            .album(album)
            .and_then(|a| a.tracks.as_ref())
            .map_or(0, Vec::len),
        Source::Search | Source::Queue => 0,
    }
}

/// One track row, from either library.
fn track_entry(app: &App, source: Source, album: usize, i: usize) -> Option<Entry> {
    let faint = Style::default().fg(app.theme.ink_faint);
    let (no, title, duration) = match source {
        Source::Local => {
            let track = app.library.albums.get(album)?.tracks.get(i)?;
            (track.track_no, track.title.clone(), track.duration)
        }
        Source::Remote => {
            let track = app.remote.album(album)?.tracks.as_ref()?.get(i)?;
            (track.track_no, track.title.clone(), track.duration)
        }
        Source::Search | Source::Queue => return None,
    };
    Some(Entry {
        number: number(no),
        text: title,
        trailing: vec![Span::styled(format_duration(duration), faint)],
        playing: playing_track(app, source, album) == Some(i),
    })
}

/// What to put in the track column when the selected album has no tracks in
/// hand. See [`split_body`].
fn track_note(app: &App, source: Source, album: usize) -> Option<String> {
    match source {
        Source::Local => app
            .library
            .albums
            .get(album)
            .map(|_| "no tracks".to_string()),
        Source::Remote => {
            let open = app.remote_view == View::Album(album);
            let fetched = app.remote.album(album)?.tracks.is_some();
            Some(match (&app.remote_detail, open, fetched) {
                (DetailState::Loading, true, _) => "loading tracks…".to_string(),
                (DetailState::Failed(message), true, _) => message.clone(),
                (_, _, true) => "no tracks".to_string(),
                (_, _, false) => "enter to load".to_string(),
            })
        }
        Source::Search | Source::Queue => None,
    }
}

/// The queue, in order.
///
/// The shape of a track list, plus the one column the other two panes do not
/// need: **where each entry came from**. This is the only list in the Deck whose
/// rows can be from different libraries, and a row that did not say so would be
/// a list you had to guess at — the id and the path behave very differently when
/// the network goes, and the user is entitled to know which one is next.
///
/// The label is [`App::source_label`], so a server is named here exactly as the
/// sidebar names it.
fn queue_entry(app: &App, i: usize) -> Option<Entry> {
    let theme = &app.theme;
    let entry = app.queue.entries().get(i)?;
    let from = app.source_label(entry.source()).to_lowercase();
    Some(Entry {
        number: (i + 1).to_string(),
        text: format!("{} — {}", entry.title, entry.artist),
        trailing: vec![
            Span::styled(from, Style::default().fg(theme.ink_dim)),
            Span::styled(
                format!("  {}", format_duration(entry.dur_ms as f64 / 1000.0)),
                Style::default().fg(theme.ink_faint),
            ),
        ],
        playing: app.queue.position() == Some(i),
    })
}

/// The row at `i` of whatever single list is on screen.
fn entry_at(app: &App, source: Source, i: usize) -> Option<Entry> {
    match (source, app.view()) {
        (Source::Local | Source::Remote, View::Albums) => album_entry(app, source, i),
        (Source::Local | Source::Remote, View::Album(album)) => track_entry(app, source, album, i),
        (Source::Search, View::Albums) => search_entry(app, i),
        (Source::Search, View::Album(_)) => search_track_entry(app, i),
        (Source::Queue, _) => queue_entry(app, i),
    }
}

/// A selected row is only *the* cursor when this pane has the keyboard.
fn row_style(theme: &Theme, selected: bool, focused: bool) -> Style {
    match (selected, focused) {
        (true, true) => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        (true, false) => Style::default().fg(theme.accent),
        (false, _) => Style::default().fg(theme.ink),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_that_fits_is_never_scrolled() {
        assert_eq!(window(0, 5, 10), (0, 5));
        assert_eq!(window(4, 5, 10), (0, 5));
    }

    #[test]
    fn a_long_list_keeps_the_cursor_on_screen() {
        for cursor in 0..100 {
            let (start, end) = window(cursor, 100, 10);
            assert!(
                (start..end).contains(&cursor),
                "cursor {cursor} fell outside {start}..{end}"
            );
            assert_eq!(end - start, 10);
        }
    }

    #[test]
    fn the_window_stops_at_both_ends_rather_than_running_past_them() {
        assert_eq!(window(0, 100, 10), (0, 10));
        assert_eq!(window(99, 100, 10), (90, 100));
    }

    #[test]
    fn an_empty_list_or_a_pane_with_no_room_yields_nothing() {
        assert_eq!(window(0, 0, 10), (0, 0));
        assert_eq!(window(0, 10, 0), (0, 0));
    }
}
