//! The Now Playing view behind `z` — **an instrument, not a poster**.
//!
//! ```text
//! ┌─ EKO ──────────────────────────────────────────────────────────────────┐
//! │     ╭──────────────────────╮                                           │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│  ▸ NOW PLAYING                   z  close │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│                                           │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│  W H E N   Y O U   W E R E   Y O U N G    │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│                                           │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│  The Innocence Mission                    │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│  Befriended                               │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│         ▄▄  ██  █▄  ▄█  ██  █▄  ▄▄        │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│  ██ ██  ██  ██  ██  ██  ██  ██  ██        │
//! │     │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│  low   32 bands · the engine's own · dis… │
//! │     ╰──────────────────────╯                                           │
//! │                                                                        │
//! │     SOURCE   FLAC · 96 kHz · 24-bit                                    │
//! │     DECODE   no resampling · no EQ · no gain                           │
//! │     OUTPUT   Topping E30 · 96 kHz                                      │
//! │                                                                        │
//! │                        ▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇▇▅▃▂        │
//! │     00:00  ▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂   04:51  │
//! ├────────────────────────────────────────────────────────────────────────┤
//! │ …the footer, exactly where it was…                                     │
//! └────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Two columns, and everything takes a share
//!
//! **The hero sits at the top margin at every size, and the surplus goes into
//! the picture, the bars and the envelope rather than above them.** The version
//! before this one grew nothing: [`COVER_MAX_ROWS`] was a single number applied
//! as a `min`, every block had a floor and a design size, and on a body taller
//! than that stack the leftover rows were drawn as blank rows *above* the
//! content. On the terminal the owner ran — 230×56 — two thirds of the screen
//! was empty at the top, the view sank to the bottom, and the analyser and the
//! envelope never appeared. It looked right at 172 columns only because at 172
//! columns there was no surplus to mishandle.
//!
//! The structural half of the fix is that **the analyser is beside the cover, in
//! the same column as the metadata, not stacked under it.** Stacked, the column
//! to the right of the picture is dead for the picture's whole height, and the
//! taller the terminal the larger that hole is. Filling it means the hero grows
//! in both directions at once, and there is nothing left to pool anywhere. The
//! arithmetic half is [`fit`], which hands the residual rows to the gaps and
//! then to the envelope, and [`panels`], whose first `y` is `content.y` with
//! nothing added to it.
//!
//! ## What this view is *for*
//!
//! The footer already carries the cover, the title, the artist, the album, the
//! scrubber, both clocks, the chain and the seal. A view that repeats all of it
//! at four times the size earns nothing — it is the same screen, louder. So the
//! only things here that are not on the footer are the ones the footer cannot
//! fit: **the picture at a size worth looking at**, and **the signal readout**.
//! Everything else on this view is a caption for those two.
//!
//! ## Every element reserves its rows, and no element knows its own row number
//!
//! [`panels`] computes a rect for each part from the available rect and hands
//! back a struct; nothing is placed by absolute row arithmetic and nothing
//! decides its own height at draw time. Every `y` below the hero is the previous
//! block's `bottom()`. An earlier draft placed the block clock and the envelope
//! by adding constants to `content.y`, and on the size the owner ran they landed
//! on the same rows.
//!
//! ## The four things this module refuses to do
//!
//! 1. **It does not interpolate the analyser.** `eko_core::engine` computes
//!    `N_BANDS = 32` log-spaced bands. This draws exactly thirty-two. When the
//!    terminal is too narrow for thirty-two bars the bars get *narrower*
//!    ([`bar_width`]), and when even one column each is too many there is **no
//!    analyser at all** — never a subset. Dropping bands would misrepresent the
//!    spectrum; dropping the whole display is honest. The cover gives up columns
//!    to keep thirty-two bars on screen ([`wanted_cover_rows`]), and never the
//!    other way round.
//!    It does not *overclaim* them either: the caption says `32 bands · the
//!    engine's own · display curve` and not `exactly what the engine reports`,
//!    because [`display_level`] shapes the heights before they are drawn.
//! 2. **It does not invent an envelope.** No local file, no shape — and the
//!    footer's plain scrubber is what says where you are. See [`crate::wave`].
//! 3. **It does not invent a readout.** Every value in [`render_readout`] comes
//!    from `eko_core::signal_path::SignalPath`, and the `DECODE` row is the
//!    seal's own `flags` restated — so the readout and the seal cannot disagree.
//!    Fields the engine does not report are **not shown**; see [`decode_terms`].
//! 4. **It paints no background.** Not one cell, anywhere, except the halfblock
//!    cover's own `▀`s — whose background *is* the lower of the cell's two
//!    pixels. `crate::ui::tests::no_cell_in_the_deck_paints_its_own_background`
//!    is the assertion, and its exemption for this view is now the cover block
//!    alone rather than the whole body.
//!
//! ## Green belongs to the seal
//!
//! The analyser runs on an **amber** ramp ([`RAMP`]). It used to run green
//! through most of its travel, which put a field of green forty times the size
//! of `● BIT-PERFECT` on the same screen as it — the one green thing in this
//! product that means something, outshouted by decoration.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use eko_core::signal_path::{SignalFlags, SignalPath};

use crate::app::App;
use crate::art;
use crate::ui::theme::Rgb;
use crate::ui::{footer, justified};

/// Bands the engine reports, and therefore bands this draws. **Not a display
/// resolution** — it is `eko_core::engine::N_BANDS`, and the two must agree.
pub const BANDS: usize = 32;

/// The eighth-block ramp: 8 sub-steps inside one cell, so `n` rows is `8n`
/// levels of vertical resolution.
const EIGHTHS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

// ── the measurements the layout is made of ───────────────────────────────────

/// Clear columns at each edge of the body.
///
/// Five, not one. The Deck's [`crate::ui::GUTTER`] is a pane's padding — it
/// keeps a row off a border. This is a *margin*, and it is the difference
/// between a picture hung on a wall and a picture taped to the edge of one.
pub const MARGIN: u16 = 5;

/// The sleeve rule around the cover, in cells, on every side.
const FRAME: u16 = 1;

/// The cover's height in rows; its width is twice that, because a cell is 1:2.
///
/// **A range, not a cap.** The previous draft had one number and applied it as
/// `min`, so on anything larger than the terminal it was drawn on the cover sat
/// at its design size and the leftover rows collected in a single void above the
/// content. The cover now takes a *share* of the body ([`wanted_cover_rows`])
/// and is clamped into this range at the ends.
///
/// Twenty-eight is deliberate, and it is the one place a cap is still right:
/// past it the sleeve stops being the hero of the view and becomes the view.
/// Eight is the floor it shrinks to before any block below it is dropped —
/// sixteen columns by eight rows is still a sleeve, and a cover barely wider
/// than the one already on the footer is the thing this whole view exists not to
/// be.
const COVER_MAX_ROWS: u16 = 28;
const COVER_MIN_ROWS: u16 = 8;

/// The cover's share of the body's height, its rule included: five eighths.
///
/// Five eighths of the body, less the rule, is what the cover asks for before
/// the blocks below it have taken theirs — so a taller terminal buys a bigger
/// picture *and* a taller analyser beside it, rather than a bigger gap. It is
/// only ever an ask: [`fit`] hands out what is left, and both ends of
/// `COVER_MIN_ROWS ..= COVER_MAX_ROWS` still bind.
const HERO_NUM: u32 = 5;
const HERO_DEN: u32 = 8;

/// Columns between the framed cover and the column beside it.
const META_GAP: u16 = 2;

/// Rows the metadata takes at the top of that column.
///
/// `▸ NOW PLAYING`, a blank, the title, a blank, the artist, the album. The two
/// blanks are the reason it is six and not four: the title is set in
/// [`spaced_caps`] and a letter-spaced line with a normal one hard against it
/// reads as one paragraph rather than as a heading over a credit.
const META_ROWS: u16 = 6;

/// Blank rows between two blocks, and the ceiling they grow to.
///
/// The gap is the *second* place the body's slack goes — see [`fit`]. One row
/// is what a small terminal can afford; three is where more air stops reading as
/// composition and starts reading as a layout that ran out of things to say.
const BLOCK_GAP: u16 = 1;
const GAP_MAX: u16 = 3;
/// Body rows per row of gap.
const GAP_STEP: u16 = 16;

/// The floor the analyser shrinks to before it is dropped.
///
/// Three is where eighth-blocks still read as a bar rather than a flicker. There
/// is no design height any more: the analyser fills the column beside the cover,
/// so its height is the cover's height less the metadata and the foot row.
const ANALYSER_MIN_ROWS: u16 = 3;
/// The row under the bars: the axis label, and the caption.
const ANALYSER_FOOT: u16 = 1;

// ── the lyrics layout's own measurements ─────────────────────────────────────

/// Rows the analyser strip grows through when the lyrics have taken the column.
///
/// **Three to five, and it reads at three because the bars have gaps.** A field
/// of fused bars three rows tall is terrain; thirty-two bars with a blank column
/// between each is thirty-two separable heights, and the eighth-block ramp gives
/// each of them `8 × rows` levels — twenty-four at the floor. That gap
/// ([`BAR_GAP`]) is the whole reason this is legible at a height the analyser
/// beside the cover would never be asked to work at, and it is why the strip
/// re-uses [`render_analyser`] rather than getting a compressed renderer of its
/// own.
///
/// Five is the ceiling because past it the strip stops being a strip and starts
/// competing with the sleeve for the eye — and because the rows are worth more
/// to the envelope, which draws a real measurement in eighths.
const STRIP_MIN_ROWS: u16 = 3;
const STRIP_MAX_ROWS: u16 = 5;
/// Body rows per row of strip. See [`strip_rows`].
const STRIP_STEP: u16 = 12;

/// One band's bar in the **strip**, at each width it degrades through.
///
/// A second ladder rather than a wider [`BAR_WIDTHS`], because the two analysers
/// are answering different questions about width. The one beside the cover is in
/// a column it *shares with the picture*: every column it takes is a column the
/// sleeve gives up ([`wanted_cover_rows`]), so it is capped at four and the
/// surplus goes to the cover. The strip is under the hero with the whole body to
/// itself and nothing to compete with, so it takes the widest bar that fits and
/// reads as a band across the view rather than a small field adrift in it.
///
/// The rule is [`BAR_WIDTHS`]' rule and is not relaxed: **width degrades, the
/// count never does.** Thirty-two seven-cell bars with a gap need `32 × 7 + 31 =
/// 255` columns, six need 223, five need 191, and below one each there is no
/// strip at all — never a subset.
const STRIP_BAR_WIDTHS: [u16; 7] = [7, 6, 5, 4, 3, 2, 1];

/// The floor the lyric column shrinks to before the sheet is not worth showing.
///
/// Two: the line that is sounding, and one neighbour. Below that there is no
/// "scrolling with the transport" left to see, and the honest answer is the
/// layout that has no lyrics in it.
const LYRICS_MIN_ROWS: u16 = 2;

/// One band's bar, at each width the analyser degrades through.
///
/// **Width, never count.** Thirty-two bars four columns wide with a one-column
/// gap need `32 × 4 + 31 = 159` columns; three need 127; two need 95; one needs
/// 63. Below 63 there is no analyser.
///
/// Four is the top step and it is new with the growing layout: the column beside
/// the cover is a hundred and ninety cells wide on a terminal the size the owner
/// runs, and thirty-two three-cell bars in it left a third of the column blank.
const BAR_WIDTHS: [u16; 4] = [4, 3, 2, 1];
/// Blank columns between two bars.
///
/// The previous draft had none, so six-cell bars fused into terrain and a
/// peak-hold cap read as a slab floating over a hillside rather than as the
/// cap of the bar under it.
const BAR_GAP: u16 = 1;

/// The narrowest column thirty-two bars can stand in: one each, with a gap.
///
/// **Derived from [`BANDS`] and [`BAR_GAP`], never written down.** It is what
/// bounds the cover's width in [`wanted_cover_rows`]: the cover and the analyser
/// share one row of columns now, so a cover free to grow to its height share on
/// a hundred-column terminal would squeeze the analyser out of a view whose
/// whole point is that the analyser is on it.
const COLUMN_MIN: u16 = BANDS as u16 + (BANDS as u16 - 1) * BAR_GAP;

/// Rows the whole-track envelope gets before the body's slack reaches it.
///
/// Two: one row of shape, and the row the clocks are printed on. Everything
/// above this is slack — see [`fit`] — because extra rows here are the one place
/// on this view where more cells really are more information: the envelope is
/// drawn in eighths, so `n` rows is `8n` levels of a real measurement.
const WAVE_MIN_ROWS: u16 = 2;
/// Columns reserved at each end of the envelope for `00:00`, plus a space.
const TIME_W: u16 = 6;

/// Rows in the signal readout: `SOURCE`, `DECODE`, `OUTPUT`.
///
/// **Three, not four.** See [`render_readout`] for the fourth.
const READOUT_ROWS: u16 = 3;
/// Columns the readout's label column occupies, gap included.
const LABEL_W: u16 = 9;

/// The analyser's colour ramp, floor to ceiling. **Amber, never green.**
///
/// Four stops: a dark ember, a burnt amber, the accent itself, and a hot
/// near-white top. Keyed to height rather than to a band's level, so one tall
/// bar is a gradient and the eye reads the whole field at once.
pub const RAMP: [Rgb; 4] = [
    Rgb::hex(0x5e3a18),
    Rgb::hex(0xc87828),
    Rgb::hex(0xef6a1e),
    Rgb::hex(0xfad0a0),
];

/// How much of full scale the loudest possible band is drawn at.
///
/// **Headroom is the point.** With no headroom every band on a loud passage
/// sits pinned against the ceiling, and a row of bars that are all exactly as
/// tall as each other and as tall as they can get shows nothing at all. See
/// [`display_level`].
const HEADROOM: f32 = 0.86;

/// The bottom of the analyser's scale, in dBFS. See [`display_level`].
///
/// **−96, and it is not a taste.** It is the quantisation floor of a 16-bit
/// master (6.02 dB a bit, sixteen bits), so a band with nothing in it but the
/// noise of the format draws nothing, and everything above the format's own
/// floor has somewhere on the block to be.
const FLOOR_DBFS: f32 = -96.0;

/// How much shorter the top band is drawn than the bottom one, at equal level.
///
/// Music has less energy at 16 kHz than at 60 Hz and always will. The dead
/// range that used to leave was reclaimed by the decibel map in
/// [`display_level`], which is the honest fix and does the larger half of this
/// job; what the tilt still buys is the *silhouette* — a field that reads left
/// to right as a spectrum rather than as a wall. **It is a display curve and
/// nothing else** — it is monotonic, it never reorders two bands and it never
/// reaches full scale, so a bar's height still answers "louder or quieter than
/// a moment ago" for its own band, which is the only question a spectrum
/// display can answer honestly anyway.
const TILT: f32 = 0.45;

/// Where each part of the view goes.
///
/// A **plain struct of rects, computed before anything is drawn**, for the same
/// reason [`crate::ui::art_area`] exists: [`crate::app::App`] keys the cover
/// cache on the block's size and [`crate::art::Painter`] positions an
/// out-of-band image by absolute cursor address, and neither can wait for a
/// frame to be rendered to find out where the cover is.
///
/// An empty rect means *this is not shown at this size* — never *this is shown
/// at zero height*. Every consumer checks `is_empty`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panels {
    /// The sleeve rule. [`Panels::cover`] is the cell inside it.
    pub frame: Rect,
    pub cover: Rect,
    pub meta: Rect,
    /// The bars only. **Beside the cover without lyrics, a strip under the hero
    /// with them** — see [`panels_for`].
    pub analyser: Rect,
    /// The row under the bars: `low`, and the caption.
    pub analyser_foot: Rect,
    /// The words, in the column beside the cover. Empty in the layout that has
    /// no lyrics, which is every layout the analyser is beside the cover in.
    pub lyrics: Rect,
    pub wave: Rect,
    pub readout: Rect,
}

#[cfg(test)]
impl Panels {
    /// Every rect, for the assertions that are about all of them at once.
    fn all(&self) -> [Rect; 8] {
        [
            self.frame,
            self.cover,
            self.meta,
            self.analyser,
            self.analyser_foot,
            self.lyrics,
            self.wave,
            self.readout,
        ]
    }
}

/// The body rect the visualiser draws into, given the whole terminal.
///
/// Reproduces [`crate::ui::draw`]'s vertical arithmetic — the outer border,
/// then everything above the rule row and the footer — and is pinned against a
/// real frame by `the_visualiser_body_stops_above_the_footer_rule`.
#[must_use]
pub fn body(area: Rect) -> Option<Rect> {
    if area.width < crate::ui::MIN_WIDTH || area.height < crate::ui::MIN_HEIGHT {
        return None;
    }
    Some(Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width - 2,
        height: area.height - 2 - 1 - crate::ui::FOOTER_HEIGHT,
    })
}

/// The body, less its margins. Empty when the body cannot hold both of them.
#[must_use]
fn content(body: Rect) -> Rect {
    Rect {
        x: body.x + MARGIN,
        y: body.y,
        width: body.width.saturating_sub(MARGIN * 2),
        height: body.height,
    }
}

/// Where the big cover goes on a terminal of `area`, or `None` when there is no
/// visualiser to draw.
///
/// This is what [`crate::app::App`] swaps in for [`crate::ui::art_area`] while
/// the view is open: **same protocols, same cache, same key — only the rect
/// changes.** The renderer is not forked, and neither is the placement.
#[must_use]
pub fn cover_area(area: Rect) -> Option<Rect> {
    let rect = panels(body(area)?).cover;
    (!rect.is_empty()).then_some(rect)
}

/// The cover's height the layout **asks for** at a body of `h` rows and `w`
/// content columns, before anything below it has taken its rows.
///
/// Three bounds, and the smallest wins:
///
/// * **A share of the height** — [`HERO_NUM`]`/`[`HERO_DEN`], less the rule.
///   This is the whole correction. The previous draft had a single design size
///   applied as a `min`, which meant the cover was the same on a 172-column
///   terminal and on a 260-column one and the difference collected as a void.
/// * **A share of the width**, so that the column beside the cover keeps at
///   least [`COLUMN_MIN`] columns. The cover and the analyser are side by side
///   now, so a cover that took its height share on a hundred-column terminal
///   would push the analyser off the view. The picture yields; the bands do not.
/// * **Square, and inside the content.** `w` columns hold a picture of at most
///   `(w - 2×FRAME) / 2` rows, whatever the other two say.
///
/// Clamped into `COVER_MIN_ROWS ..= COVER_MAX_ROWS` between the second bound and
/// the third, so the floor can lift a cover the width bound crushed — at eighty
/// columns there is no analyser at any cover size, and shrinking the picture to
/// nothing to make room for one would buy nothing.
fn wanted_cover_rows(h: u16, w: u16) -> u16 {
    let share = u32::from(h) * HERO_NUM / HERO_DEN;
    let by_height = u16::try_from(share)
        .unwrap_or(u16::MAX)
        .saturating_sub(FRAME * 2);
    let by_width = w.saturating_sub(FRAME * 2 + META_GAP + COLUMN_MIN) / 2;
    let square = w.saturating_sub(FRAME * 2) / 2;
    by_height
        .min(by_width)
        .clamp(COVER_MIN_ROWS, COVER_MAX_ROWS)
        .min(square)
}

/// The rows of bars the column beside a cover of `cover_rows` has left over.
///
/// `0` means **no analyser**, never an analyser of no height. The metadata takes
/// [`META_ROWS`] off the top of the column and the caption takes
/// [`ANALYSER_FOOT`] off the bottom; what is between them is the analyser, and
/// below [`ANALYSER_MIN_ROWS`] there is nothing worth drawing.
fn analyser_rows(cover_rows: u16) -> u16 {
    let rows = cover_rows.saturating_sub(META_ROWS + ANALYSER_FOOT);
    if rows >= ANALYSER_MIN_ROWS {
        rows
    } else {
        0
    }
}

/// Which parts of the view a body of `h` rows and `w` columns can hold, and how
/// tall the cover and the envelope are once they have taken theirs.
///
/// # Everything takes a share, and the slack has somewhere to be
///
/// **This is the bug this rewrite exists to make impossible.** The old `fit`
/// grew nothing: every block had a minimum and a design size, so on a body
/// taller than the design stack the surplus went into a `top_pad` field and was
/// drawn as twenty-five blank rows above the picture. On a 230×56 terminal two
/// thirds of the screen was empty and the content sank to the bottom.
///
/// There is no `top_pad` any more, and there is nowhere for one to go: the head
/// of the layout is `content.y` with nothing added to it ([`panels`]), and the
/// residual rows are handed out in this order.
///
/// 1. **The cover**, to its share of the height and no further than
///    [`COVER_MAX_ROWS`] — and with it the analyser beside it, whose height is
///    the cover's less the metadata and the caption.
/// 2. **The gaps between the blocks**, one row per [`GAP_STEP`] rows of body, to
///    [`GAP_MAX`].
/// 3. **The envelope**, which takes everything still unspent. Rows here are the
///    one thing on this view that extra cells genuinely buy: the shape is drawn
///    in eighths, so `n` rows is `8n` levels of a real measurement rather than
///    the same picture larger.
///
/// Between them those three spend the body exactly whenever there is an
/// envelope, which is what `no_size_leaves_a_void_and_no_two_blocks_share_a_row`
/// checks at every size from 80×20 up.
///
/// # The order things give way in is a statement about what this view is for
///
/// * **The readout goes first.** It is the only block here whose content is
///   also somewhere else on the same screen — the footer's fourth row carries
///   `FLAC · 96 kHz · 24-bit → Topping E30 · 96 kHz  ● BIT-PERFECT`, which is
///   the same three facts compressed onto one line. Losing it costs detail, not
///   information.
/// * **Then the envelope**, whose job the footer's plain scrubber also does.
/// * **Then everything but the cover.**
///
/// The analyser is not in that list, because it is no longer a block: it is the
/// bottom of the cover's own column, and it is present exactly when that column
/// is tall enough ([`analyser_rows`]) and wide enough ([`bar_width`]) for
/// thirty-two bars. The cover is not in the list either — it **shrinks**, down
/// to [`COVER_MIN_ROWS`], before any block is dropped. Shrinking a picture is not
/// the same as removing a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fit {
    cover_rows: u16,
    /// `0` means no envelope at this size.
    wave_rows: u16,
    readout: bool,
    /// Rows of **bars** in the full-width strip; `0` means no strip. Always `0`
    /// without lyrics, where the analyser is beside the cover instead.
    strip_rows: u16,
    /// Blank rows between two blocks, at this size.
    gap: u16,
}

/// Rows of bars the strip gets on a body of `h` rows. See [`STRIP_MIN_ROWS`].
fn strip_rows(h: u16) -> u16 {
    (h / STRIP_STEP).clamp(STRIP_MIN_ROWS, STRIP_MAX_ROWS)
}

/// The layout with lyrics, laid over the one without. **The cover is the same
/// picture, at the same size, in both.**
///
/// This is not a nicety. [`crate::art::Key`] carries the cover's cell grid, so a
/// cover that changed size when the words landed would change the key, refetch
/// and re-encode the picture, and flash the placeholder — a few hundred
/// milliseconds into every track, because that is when a lyric fetch answers.
/// The module docs already refuse to let the *envelope's* arrival move the
/// layout; the words are the same hazard with a longer fuse.
///
/// So [`base_fit`] decides the cover and nothing here may revisit it. What is
/// left below the sleeve is spent on the blocks, and they give way in this
/// order:
///
/// 1. **The readout**, first, for the reason it goes first without lyrics: the
///    footer's fourth row already carries the same three facts.
/// 2. **The strip.** Three rows of spectrum beside a lyric sheet is the block
///    with the least to say; the footer's own spectrum says it too.
/// 3. **The envelope**, last, because it is the only block below the hero that
///    is also a *control* — it is the scrubber, shaped like the music.
///
/// The envelope is also where the residual rows go, exactly as it is without
/// lyrics, which is what makes this spend the body to its last row.
fn fit_with_lyrics(base: Fit, h: u16, w: u16) -> Fit {
    let cover_rows = base.cover_rows;
    let gap = base.gap;
    let below = h.saturating_sub(cover_rows + FRAME * 2);
    let want_strip = strip_rows(h);
    // Too narrow for thirty-two bars even one column wide: no strip at any
    // height. **Never a subset** — see [`STRIP_BAR_WIDTHS`].
    let can_strip = strip_bar_width(w).is_some();

    for (readout, strip, wave) in [
        (true, true, true),
        (false, true, true),
        (false, false, true),
        (false, false, false),
    ] {
        let strip = strip && can_strip;
        let cost = if readout { gap + READOUT_ROWS } else { 0 }
            + if strip {
                gap + want_strip + ANALYSER_FOOT
            } else {
                0
            }
            + if wave { gap + WAVE_MIN_ROWS } else { 0 };
        if cost > below {
            continue;
        }
        return Fit {
            cover_rows,
            // **Every row the blocks did not claim ends up here**, which is why
            // there is none left to pool above the content.
            wave_rows: if wave {
                WAVE_MIN_ROWS + (below - cost)
            } else {
                0
            },
            readout,
            strip_rows: if strip { want_strip } else { 0 },
            gap,
        };
    }
    // Unreachable above the Deck's own minimum — the last rung costs nothing.
    Fit {
        cover_rows,
        wave_rows: 0,
        readout: false,
        strip_rows: 0,
        gap,
    }
}

fn fit(h: u16, w: u16, lyrics: bool) -> Option<Fit> {
    let base = base_fit(h, w)?;
    Some(if lyrics {
        fit_with_lyrics(base, h, w)
    } else {
        base
    })
}

fn base_fit(h: u16, w: u16) -> Option<Fit> {
    if h <= FRAME * 2 || w <= FRAME * 2 {
        return None;
    }
    let gap = (h / GAP_STEP).clamp(BLOCK_GAP, GAP_MAX);
    let want = wanted_cover_rows(h, w);
    // The floor is the floor, unless the content is too narrow to reach it —
    // in which case the width bound in `wanted_cover_rows` is already the floor.
    let floor = COVER_MIN_ROWS.min(want);

    for (readout, wave) in [(true, true), (false, true), (false, false)] {
        let below = if readout { gap + READOUT_ROWS } else { 0 }
            + if wave { gap + WAVE_MIN_ROWS } else { 0 };
        // The cover box: the picture plus a rule on every side.
        let room = h.saturating_sub(below + FRAME * 2);
        if room < floor {
            continue;
        }
        let cover_rows = room.min(want);
        return Some(Fit {
            cover_rows,
            // **Every row the caps left over ends up here**, which is why there
            // is no room left to put above the content.
            wave_rows: if wave {
                WAVE_MIN_ROWS + (room - cover_rows)
            } else {
                0
            },
            readout,
            strip_rows: 0,
            gap,
        });
    }
    // Nothing left to drop. A cover alone, if there is room for one.
    let cover_rows = (h - FRAME * 2).min(want);
    (cover_rows >= 1).then_some(Fit {
        cover_rows,
        wave_rows: 0,
        readout: false,
        strip_rows: 0,
        gap,
    })
}

/// Split the **body** into the panels.
///
/// Takes the body rather than an already-inset rect so that [`MARGIN`] is
/// applied in exactly one place, and so [`cover_area`] and [`render`] cannot
/// answer differently.
///
/// # The shape of it
///
/// ```text
///  ┌──────────┐  ▸ NOW PLAYING                z  close   ┐
///  │          │                                          │
///  │  cover   │  T H E   T I T L E                       │ the hero, at the
///  │ (square) │                                          │ top margin
///  │          │  the artist                              │
///  └──────────┘  the album                               │
///                 ███ ██  █  ██ ██▄ █▄  ▄  ▄             │ the analyser fills
///                 low          32 bands · … · display …  ┘ the rest of the column
///
///  SOURCE   FLAC · 96 kHz · 24-bit                         the readout,
///  DECODE   no resampling · no EQ · no gain                full width
///  OUTPUT   Topping E30 · 96 kHz
///
///  00:00  ▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃  04:51   the envelope, and
/// ```                                                      every leftover row
///
/// **The analyser is beside the cover, not under it**, and that is the change
/// that closed the void. Stacked down a tall frame — cover, then analyser, then
/// envelope — the column to the right of the cover is empty for the whole height
/// of the picture, and the taller the terminal the larger that hole gets.
/// Filling the column instead means the hero grows in both directions at once
/// and there is nothing left over to pool anywhere.
///
/// # It is a function of the rect and nothing else
///
/// In particular it does **not** take "is there an envelope", "is there a
/// cover" or "is the engine reporting". Every row is reserved whenever the
/// terminal is tall enough for one and left blank when there is nothing to put
/// in it. The alternative — giving a block's rows away when its content is
/// missing — makes the cover's height depend on an answer that arrives from a
/// worker thread a few hundred milliseconds into the track: the cover's block
/// would change size mid-track, which changes [`crate::art::Key`], which
/// refetches and re-encodes the picture and flashes the placeholder while it
/// does.
///
/// [`panels_for`] is the *one* exception, and it is an exception that proves the
/// rule: there are two arrangements, chosen by whether this track has words, and
/// **the cover is the same rect in both**. See its docs.
#[must_use]
pub fn panels(body: Rect) -> Panels {
    panels_for(body, false)
}

/// The panels, in the arrangement `lyrics` calls for.
///
/// # Two layouts, and neither has a dead column
///
/// **Without lyrics** the shape above stands unchanged: the analyser fills the
/// column beside the sleeve, under the metadata.
///
/// **With lyrics** the words take that column — they are the thing worth the
/// tall shape, and the analyser is not — and the analyser moves to a strip
/// across the body, above the envelope:
///
/// ```text
///  ┌──────────┐  ▸ NOW PLAYING                z  close   ┐
///  │          │                                          │
///  │  cover   │  T H E   T I T L E                       │ the hero
///  │ (square) │                                          │
///  │          │  the artist                              │
///  └──────────┘  the album                               │
///                 …a line before                         │ the words fill
///                 THE LINE THAT IS SOUNDING              │ the rest of the
///                 …a line after                          ┘ column
///
///  SOURCE   FLAC · 96 kHz · 24-bit                         the readout
///  DECODE   no resampling · no EQ · no gain
///  OUTPUT   Topping E30 · 96 kHz
///
///     ▄▄  ██  █▄  ▄█  ██  █▄  ▄▄  ██  █▄  ▄█  ██  █▄      the strip: 3–5 rows,
///     low                  32 bands · … · display curve    thirty-two bars
///
///  00:00  ▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃▂▁▂▃▅▇█▇▅▃  04:51   the envelope
/// ```
///
/// Neither arrangement leaves a column with nothing in it, which is the whole
/// reason the analyser had to move rather than the lyrics being squeezed in
/// beside it: a lyric sheet in half a column is unreadable, and a spectrum with
/// a lyric sheet under it in the other half is two things fighting over one
/// column's width.
///
/// # The cover is identical in both, by construction
///
/// `panels_for(b, true).cover == panels_for(b, false).cover` at every size, and
/// the sweep asserts it. Lyrics arrive from a worker a few hundred milliseconds
/// into a track; a cover that resized when they landed would change
/// [`crate::art::Key`] and flash the placeholder mid-song. [`fit_with_lyrics`]
/// takes [`base_fit`]'s cover as given and spends only what is below it — which
/// is also why [`cover_area`] does not need to know about lyrics at all.
#[must_use]
pub fn panels_for(body: Rect, lyrics: bool) -> Panels {
    let c = content(body);
    let empty = Rect {
        x: c.x,
        y: c.y,
        width: 0,
        height: 0,
    };
    let nothing = Panels {
        frame: empty,
        cover: empty,
        meta: empty,
        analyser: empty,
        analyser_foot: empty,
        lyrics: empty,
        wave: empty,
        readout: empty,
    };
    if c.is_empty() {
        return nothing;
    }
    let Some(f) = fit(c.height, c.width, lyrics) else {
        return nothing;
    };

    // ── the hero: a framed cover, and a column beside it ──────────────────
    //
    // **`c.y`, with nothing added to it.** There is no `top_pad` term and no
    // centring term, at any size, which is what makes a leading void something
    // this function cannot express rather than something it happens not to do.
    // `wanted_cover_rows` has already bounded the picture by the content's own
    // width, so the frame fits across without a `min` that would unsquare it.
    let frame = Rect {
        x: c.x,
        y: c.y,
        width: f.cover_rows * 2 + FRAME * 2,
        height: f.cover_rows + FRAME * 2,
    };
    let cover = Rect {
        x: frame.x + FRAME,
        y: frame.y + FRAME,
        width: f.cover_rows * 2,
        height: f.cover_rows,
    };

    // The column beside it, aligned with the picture rather than with the rule:
    // the metadata reads as a caption for the sleeve, and the analyser under it
    // ends on the sleeve's own last row.
    let column_x = frame.right() + META_GAP;
    let column_w = c.right().saturating_sub(column_x);
    let column_x = column_x.min(c.right());
    let meta = Rect {
        x: column_x,
        y: cover.y,
        width: column_w,
        height: META_ROWS.min(f.cover_rows),
    };

    // What is under the metadata in that column: the analyser, or the words.
    // Never both, and never neither while there is room for one — that is the
    // "no dead column" rule, and it is why these two are decided together.
    let column_rest = Rect {
        x: column_x,
        y: cover.y + META_ROWS,
        width: column_w,
        height: f.cover_rows.saturating_sub(META_ROWS),
    };
    let no_rect = Rect {
        x: column_x,
        y: cover.y + meta.height,
        width: 0,
        height: 0,
    };

    // The analyser is narrowed to the bars' own span and hung on the **column's
    // left edge**, and the foot row takes the same columns — so `low` sits under
    // the first bar, the caption ends under the last one, and the first bar
    // starts in the same column as `▸ NOW PLAYING` and the title above it.
    //
    // It was centred while it was a full-width block under a centred stack, and
    // centring it inside the column instead put its left edge fifteen cells
    // right of everything else in that column at the size the owner runs. One
    // left edge for the whole column is worth more than a balanced bar field.
    //
    // Too narrow for thirty-two bars even one column wide, or too short for bars
    // that read as bars: **no analyser at all**, never a subset. See the module
    // docs. And never here at all when the words have the column.
    let rows = analyser_rows(f.cover_rows);
    let column_bars = (!lyrics)
        .then(|| bar_width(column_w))
        .flatten()
        .filter(|_| rows > 0)
        .map(|bw| {
            let span = BANDS as u16 * bw + (BANDS as u16 - 1) * BAR_GAP;
            let a = Rect {
                x: column_x,
                y: cover.y + META_ROWS,
                width: span,
                height: rows,
            };
            let foot = Rect {
                x: a.x,
                y: a.bottom(),
                width: a.width,
                height: ANALYSER_FOOT,
            };
            (a, foot)
        });

    // The words take the whole of the rest of the column — full width, down to
    // the sleeve's own last row. Below LYRICS_MIN_ROWS there is nothing left to
    // read and the rect is empty.
    let lyrics_rect = if lyrics && column_rest.width > 0 && column_rest.height >= LYRICS_MIN_ROWS {
        column_rest
    } else {
        no_rect
    };

    // ── the blocks below it, each one under the last ──────────────────────
    //
    // **Nothing here is placed at an absolute row.** Every `y` is the previous
    // block's `bottom()`, which is how the block clock came to be drawn on top
    // of the envelope in an earlier draft: it was placed by adding a constant to
    // `content.y`, and on the size the owner ran the constant was wrong.
    let mut y = frame.bottom();
    let mut block = |rows: u16| -> Rect {
        if rows == 0 {
            return Rect {
                x: c.x,
                y,
                width: 0,
                height: 0,
            };
        }
        let r = Rect {
            x: c.x,
            y: y + f.gap,
            width: c.width,
            height: rows,
        };
        y = r.bottom();
        r
    };
    let readout = block(if f.readout { READOUT_ROWS } else { 0 });
    // The strip — the analyser, when the words have taken the column. It sits
    // between the readout and the envelope, so the eye reads the sleeve, then
    // the chain, then the spectrum, then the shape of the whole track.
    // `strip_rows` is `0` in the layout without lyrics, so this costs that
    // layout nothing at all.
    let strip = block(if f.strip_rows > 0 {
        f.strip_rows + ANALYSER_FOOT
    } else {
        0
    });
    let wave = block(f.wave_rows);

    // The bars are centred in the strip and the foot row takes their exact
    // columns, so `low` still sits under the first bar. Centred rather than hung
    // on the left edge because the strip is a band across the body rather than
    // the bottom of a column — there is no left edge above it to line up with.
    let (analyser, analyser_foot) = match column_bars {
        Some(pair) => pair,
        None if !strip.is_empty() => match strip_bar_width(strip.width) {
            Some(bw) => {
                let span = BANDS as u16 * bw + (BANDS as u16 - 1) * BAR_GAP;
                let x = strip.x + (strip.width - span) / 2;
                let a = Rect {
                    x,
                    y: strip.y,
                    width: span,
                    height: f.strip_rows,
                };
                let foot = Rect {
                    x,
                    y: a.bottom(),
                    width: span,
                    height: ANALYSER_FOOT,
                };
                (a, foot)
            }
            // `fit_with_lyrics` only reserves the rows when the bars fit, so
            // this is unreachable; it is here so rows cannot be reserved and
            // then silently left blank.
            None => (no_rect, no_rect),
        },
        None => (no_rect, no_rect),
    };

    Panels {
        frame,
        cover,
        meta,
        analyser,
        analyser_foot,
        lyrics: lyrics_rect,
        wave,
        readout,
    }
}

/// Render the whole view into `area`, which is the Deck's body.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() {
        return;
    }
    let p = panels(area);
    render_frame(frame, p.frame, app);
    render_cover(frame, p.cover, app);
    render_meta(frame, p.meta, app);
    render_analyser(frame, p.analyser, app);
    render_analyser_foot(frame, p.analyser_foot, app);
    render_wave(frame, p.wave, app);
    render_readout(frame, p.readout, app);
}

// ── the sleeve ───────────────────────────────────────────────────────────────

/// The rule around the cover.
///
/// **A white sleeve on a dark terminal has no edge.** Without this the picture
/// bleeds into the ground on one album and stops dead on the next, and neither
/// looks deliberate. One cell of rule, in the same colour every other rule on
/// the Deck is drawn in.
fn render_frame(f: &mut Frame, area: Rect, app: &App) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let style = app.theme.on_console(app.theme.rule);
    let inner = usize::from(area.width - 2);
    let top = format!("╭{}╮", "─".repeat(inner));
    let bottom = format!("╰{}╯", "─".repeat(inner));
    let mut rows: Vec<Line> = Vec::with_capacity(usize::from(area.height));
    rows.push(Line::from(Span::styled(top, style)));
    for _ in 1..area.height - 1 {
        rows.push(Line::from(vec![
            Span::styled("│", style),
            // Not a background: the cover is drawn over these cells straight
            // after, and where there is no cover this is simply blank.
            Span::raw(" ".repeat(inner)),
            Span::styled("│", style),
        ]));
    }
    rows.push(Line::from(Span::styled(bottom, style)));
    f.render_widget(Paragraph::new(rows), area);
}

// ── the cover ────────────────────────────────────────────────────────────────

/// The big cover — [`crate::ui::footer`]'s renderer, at a different rect.
///
/// The three cases are the footer's three cases and are deliberately written
/// the same way round: real halfblock pixels; blank cells that an out-of-band
/// image will be written over after the frame is flushed; or the placeholder,
/// at exactly the size the picture would have been.
fn render_cover(f: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() {
        return;
    }
    match app.art.rendered() {
        Some(art::Rendered::Cells(cells))
            if cells.cols == area.width && cells.rows == area.height =>
        {
            footer::paint_cells(f, area, cells, app.theme.depth);
        }
        Some(art::Rendered::Escape(_)) => f.render_widget(Clear, area),
        _ => {
            let style = app.theme.on_console(app.theme.rule);
            let block = "░".repeat(area.width as usize);
            let rows: Vec<Line> = (0..area.height)
                .map(|_| Line::from(Span::styled(block.clone(), style)))
                .collect();
            f.render_widget(Paragraph::new(rows), area);
        }
    }
}

// ── the metadata column ──────────────────────────────────────────────────────

/// `WHEN YOU WERE YOUNG` → `W H E N   Y O U   W E R E   Y O U N G`.
///
/// Letter-spaced caps, which is the one typographic move a terminal can make:
/// there is one font, one weight and one size, so emphasis has to come from
/// *rhythm*. A space becomes three columns because the space itself is spaced
/// like every other character, which is what keeps words separable.
#[must_use]
pub fn spaced_caps(text: &str) -> String {
    let mut out = String::new();
    for ch in text.to_uppercase().chars() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// The column beside the cover: what is playing, and how to leave.
///
/// **No year**, because there is no year. [`crate::app::NowPlaying`] carries a
/// title, an artist, an album name and a duration, and `eko-cli`'s scanner
/// never reads a date tag — so `album · 1997` would be a number this program
/// made up.
fn render_meta(f: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() || area.height < 1 {
        return;
    }
    let theme = &app.theme;
    let w = area.width;
    let playing = app.playback != crate::app::Playback::Stopped;

    let head = justified(
        vec![Span::styled("▸ NOW PLAYING", theme.accent_strong())],
        vec![Span::styled("z  close", theme.on_console(theme.ink_faint))],
        w,
    );

    let (title, artist, album) = match &app.now {
        Some(now) if playing => (
            spaced_caps(&now.title),
            now.artist.clone(),
            now.album_name.clone(),
        ),
        _ => (
            "N O T H I N G   P L A Y I N G".to_string(),
            String::new(),
            String::new(),
        ),
    };

    let ink = if playing { theme.ink } else { theme.ink_dim };
    // Six rows, and [`META_ROWS`] is six because of the two blanks: the title is
    // letter-spaced and the credit under it is not, and with the two hard
    // against each other they read as one paragraph rather than as a heading
    // over a credit.
    let rows = vec![
        head,
        Line::default(),
        Line::from(Span::styled(
            footer::fit(&title, usize::from(w)),
            Style::default().fg(ink).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(Span::styled(
            footer::fit(&artist, usize::from(w)),
            theme.on_console(theme.ink_dim),
        )),
        Line::from(Span::styled(
            footer::fit(&album, usize::from(w)),
            theme.on_console(theme.ink_faint),
        )),
    ];
    // The rect clips whatever a short frame cannot hold; nothing is moved to
    // make room, because moving it is what put two things on one row before.
    f.render_widget(Paragraph::new(rows), area);
}

// ── the analyser ─────────────────────────────────────────────────────────────

/// The widest bar `width` columns can show thirty-two of, or `None`.
///
/// **Width degrades; the count never does.** See [`BAR_WIDTHS`].
#[must_use]
pub fn bar_width(width: u16) -> Option<u16> {
    let bands = BANDS as u16;
    BAR_WIDTHS
        .into_iter()
        .find(|w| bands * w + (bands - 1) * BAR_GAP <= width)
}

/// The widest **strip** bar `width` columns can show thirty-two of, or `None`.
///
/// [`bar_width`]'s rule over [`STRIP_BAR_WIDTHS`]' wider ladder: width degrades,
/// the count never does, and below one column each there is no strip rather than
/// a subset of the bands. The two ladders exist because the two analysers are
/// answering different questions about width — see [`STRIP_BAR_WIDTHS`].
#[must_use]
pub fn strip_bar_width(width: u16) -> Option<u16> {
    let bands = BANDS as u16;
    STRIP_BAR_WIDTHS
        .into_iter()
        .find(|w| bands * w + (bands - 1) * BAR_GAP <= width)
}

/// Where each of the [`BANDS`] bars starts, and how wide it is, inside `width`.
///
/// Centred inside whatever it is given — which, since [`panels`] hands it a rect
/// exactly [`bar_width`] wide, is a no-op there. It matters for a caller that
/// measures a wider span: a field of bars adrift in it reads as a mistake.
#[must_use]
pub fn bands_across(width: u16) -> Vec<(u16, u16)> {
    let Some(w) = bar_width(width) else {
        return Vec::new();
    };
    let bands = BANDS as u16;
    let span = bands * w + (bands - 1) * BAR_GAP;
    let left = (width - span) / 2;
    (0..bands).map(|i| (left + i * (w + BAR_GAP), w)).collect()
}

/// The height band `i` is drawn at, for the level the engine reported.
///
/// See [`FLOOR_DBFS`], [`HEADROOM`] and [`TILT`]. Pure, monotonic in `level`,
/// and strictly decreasing in `i` — so it can never reorder two bands.
///
/// # The scale is decibels, and this is the arithmetic
///
/// `eko_core::engine` reports `(m / (FFT_N × 0.25)).sqrt()`, where `m` is the
/// mean bin magnitude in the band and `FFT_N × 0.25` is the magnitude a
/// full-scale sine produces under the engine's Hann window. So the quantity
/// under that square root is an **amplitude ratio referenced to full scale**,
/// and the value handed to this function is its square root — a compression,
/// but still a linear-ish one.
///
/// A spectrum is read logarithmically or it is not read at all, and the
/// measurements say so. Across four hundred windows of a real mastered track
/// the median reported value runs from 0.19 in the bass to 0.014 at the top,
/// which drawn linearly is a bass end at a sixth of the block and a top half
/// lying flat on the floor at under one percent of it. That is the whole
/// complaint: not a curve anyone dislikes, a top octave with nowhere to move.
///
/// So the reported value is squared back into the amplitude ratio the engine
/// divided down — hence `40 × log10`, which is `20 × log10(level²)` — and that
/// dBFS reading is mapped onto the block from [`FLOOR_DBFS`] to 0. The same
/// four hundred windows then run 0.70 to 0.13, and the median band's own
/// travel over time goes from 0.03 of the block to 0.20 at band 28. `level` of
/// zero is `-inf` dB, which clamps to the floor: **silence still draws
/// nothing.**
///
/// **This is a display transform and it is the only one.** The band count is
/// the engine's, unchanged; no value is synthesised; the map is monotonic, so
/// a bar still answers "louder or quieter than a moment ago" for its own band
/// and two bands are never reordered. The caption under the bars says exactly
/// that: `32 bands · the engine's own · display curve`.
#[must_use]
pub fn display_level(band: usize, level: f32) -> f32 {
    let t = if BANDS <= 1 {
        0.0
    } else {
        band.min(BANDS - 1) as f32 / (BANDS - 1) as f32
    };
    let dbfs = 40.0 * level.clamp(0.0, 1.0).log10();
    let scaled = ((dbfs - FLOOR_DBFS) / -FLOOR_DBFS).clamp(0.0, 1.0);
    scaled * HEADROOM * (1.0 - TILT * t)
}

/// The bar colour at `frac` of the analyser's full height.
///
/// A continuous amber ramp keyed to **height, not to the band's level**. The
/// stops are lerped and then quantised by [`Rgb::resolve`], so a 256-colour
/// terminal gets four or five ambers rather than a truecolor escape it would
/// print as text — and, crucially, **no green at either depth**.
#[must_use]
pub fn ramp(theme: &crate::ui::theme::Theme, frac: f32) -> ratatui::style::Color {
    let frac = frac.clamp(0.0, 1.0);
    let segments = (RAMP.len() - 1) as f32;
    let scaled = frac * segments;
    let i = (scaled.floor() as usize).min(RAMP.len() - 2);
    let t = scaled - i as f32;
    let (a, b) = (RAMP[i], RAMP[i + 1]);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2)).resolve(theme.depth)
}

/// One band's glyph for one row, given the bar's fill and the peak's, both in
/// eighths from the floor.
///
/// Returns the glyph and whether it came from the **peak** rather than the bar,
/// which is what decides the colour.
///
/// # The cap is composited into the bar's own cell
///
/// A peak-hold drawn as a separate row above the bar leaves a one-cell hole
/// whenever the peak is only a fraction of a cell above it, and the hole
/// flickers at 30fps. So the peak and the bar are resolved *per cell*: where
/// they land in the same cell the taller of the two glyphs is drawn once, and
/// it belongs to the bar, because a bar with a cap inside it is still a bar.
#[must_use]
fn bar_glyph(bar: u16, peak: u16, row_from_floor: u16) -> Option<(&'static str, bool)> {
    let floor = row_from_floor * 8;
    let bar_in_cell = bar.saturating_sub(floor).min(8);
    // `peak > 0` first: a held peak of zero is *no peak*, and without the guard
    // every silent band would grow a floor glyph in its bottom cell — a picture
    // of a signal, drawn for the absence of one.
    let peak_in_cell = if peak > 0 && peak >= floor && peak < floor + 8 {
        (peak - floor).max(1)
    } else {
        0
    };
    if bar_in_cell == 0 && peak_in_cell == 0 {
        return None;
    }
    let level = bar_in_cell.max(peak_in_cell);
    Some((EIGHTHS[usize::from(level - 1)], bar_in_cell == 0))
}

/// The 32-band analyser: thirty-two bars, separated, with held caps.
///
/// Bands come from `Engine::bands` by way of [`crate::app::App::bands`]; peaks
/// come from [`crate::app::App::peaks`], which is state rather than a
/// render-time decay so that a frame is a function of [`App`] and a snapshot
/// test does not depend on how long the test took to reach its assertion.
///
/// When the engine has reported nothing — not playing, or a session that has
/// not filled its first FFT — every bar sits on the floor. A floor is honest;
/// an idle animation would not be.
fn render_analyser(f: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() || !app.analyser {
        return;
    }
    let spans = bands_across(area.width);
    if spans.is_empty() {
        return;
    }
    let theme = &app.theme;
    let rows = area.height;
    let ceiling = rows * 8;
    let buf = f.buffer_mut();

    for (i, (x0, bw)) in spans.iter().enumerate() {
        let level = display_level(i, app.bands.get(i).copied().unwrap_or(0.0));
        let peak_level = display_level(i, app.peaks.get(i).copied().unwrap_or(0.0));
        let bar = (level * f32::from(ceiling)).round() as u16;
        let peak = (peak_level * f32::from(ceiling)).round() as u16;
        for row in 0..rows {
            // Row 0 of the rect is the top; the bar grows from the bottom.
            let from_floor = rows - 1 - row;
            let Some((glyph, is_peak)) = bar_glyph(bar.min(ceiling), peak.min(ceiling), from_floor)
            else {
                continue;
            };
            let frac = if rows <= 1 {
                0.0
            } else {
                f32::from(from_floor) / f32::from(rows - 1)
            };
            let colour = ramp(theme, frac);
            let style = if is_peak {
                Style::default().fg(colour).add_modifier(Modifier::DIM)
            } else {
                Style::default().fg(colour)
            };
            for col in 0..*bw {
                if let Some(cell) = buf.cell_mut((area.x + x0 + col, area.y + row)) {
                    cell.set_symbol(glyph).set_style(style);
                }
            }
        }
    }
}

/// The row under the bars.
///
/// # There is no numeric frequency scale, and this is why
///
/// A band's frequency is `bin × rate / FFT_N`, over bins geometrically spaced
/// from 1 to `FFT_N / 2`. `FFT_N` and that spacing are **private to
/// `eko_core::engine`**, and the rate is whatever the current stream is running
/// at — so band 0 is ~43 Hz at 44.1 kHz and ~94 Hz at 96 kHz. A `31 · 125 · 1k`
/// axis would therefore be wrong at every rate but one, and reproducing the
/// engine's window length in this crate to compute a right one would be a
/// second copy of a private constant, free to drift, printed as a measurement.
///
/// So the axis says the one thing about it that is true at every rate and needs
/// nothing private to know: which end is which.
///
/// # And the caption names the curve, because the caption is a claim
///
/// It read `32 bands · exactly what the engine reports`, which was true of the
/// **count** and false of the picture: [`display_level`] applies [`HEADROOM`] and
/// [`TILT`] before a bar is drawn, so a bar's height is the engine's level
/// through a display curve and not the level itself. On the one view in this
/// product whose argument is that its readouts are falsifiable, a caption that
/// overclaims by one word is the worst kind of wrong. It now says what is
/// actually on screen: the engine's own bands, drawn through a display curve.
fn render_analyser_foot(f: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() || !app.analyser {
        return;
    }
    let theme = &app.theme;
    let faint = theme.on_console(theme.ink_faint);
    f.render_widget(
        Paragraph::new(justified(
            vec![Span::styled("low", faint)],
            vec![Span::styled(
                "32 bands · the engine's own · display curve",
                faint,
            )],
            area.width,
        )),
        area,
    );
}

// ── the envelope ─────────────────────────────────────────────────────────────

/// The whole-track overview, with a clock at each end.
///
/// # Two heights per column, because peak alone is a slab
///
/// The **peak** is drawn as a dimmed outline and the **RMS** as a solid fill
/// inside it — the DAW and SoundCloud rendering, and see [`crate::wave::Envelope`]
/// for the measurements. The correction is not cosmetic: on a mastered track
/// nearly every half-second bucket peaks at full scale, so an outline-only
/// block is one flat rectangle with a fade at each end, and every column of it
/// says the same thing. The fill is where the shape of the music actually is.
///
/// Both readings come from the same decode and neither is invented; what the
/// dim outline shows is the transient the fill does not reach, which is a fact
/// about the audio and not a halo.
///
/// Played in the accent, unplayed in the scrubber's track colour, so the block
/// is a scrubber that happens to be shaped like the music. When there is no
/// envelope this draws **nothing but the clocks** and the footer's plain
/// scrubber is the only positional control on screen — see [`crate::wave`] for
/// why a remote track cannot have one and why nothing is drawn in its place.
///
/// The clocks own [`TIME_W`] columns at each end on **every** row of the block,
/// and are printed on the last of them. The envelope is laid out between those
/// reserved columns rather than under them, so there is no width at which a
/// clock and a waveform want the same cell.
fn render_wave(f: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() || area.width <= TIME_W * 2 {
        return;
    }
    let theme = &app.theme;
    let faint = theme.on_console(theme.ink_faint);
    let dur = app.dur_ms();

    // The clocks, on the block's last row.
    f.render_widget(
        Paragraph::new(justified(
            vec![Span::styled(clock(app.pos_ms), faint)],
            vec![Span::styled(clock(dur), faint)],
            area.width,
        )),
        Rect {
            x: area.x,
            y: area.bottom() - 1,
            width: area.width,
            height: 1,
        },
    );

    let Some(env) = app.wave.envelope() else {
        return;
    };
    let inner = Rect {
        x: area.x + TIME_W,
        y: area.y,
        width: area.width - TIME_W * 2,
        height: area.height,
    };
    let played = if dur == 0 {
        0
    } else {
        ((app.pos_ms.min(dur) as f64 / dur as f64) * f64::from(inner.width)).round() as u16
    };
    let ceiling = inner.height * 8;
    let buf = f.buffer_mut();
    for col in 0..inner.width {
        let peak = env.peak_column(col, inner.width).clamp(0.0, 1.0);
        let level = env.rms_column(col, inner.width).clamp(0.0, 1.0);
        // The outline's floor is one eighth rather than a blank: a bucket that
        // really is near-silent is part of the shape, and a gap in the row
        // would read as the end of the track. **The fill has no floor**, and
        // that is deliberate — a quiet passage is a bare outline, which is the
        // one thing a peak-only envelope could never draw.
        let outline = ((peak * f32::from(ceiling)).round() as u16)
            .max(1)
            .min(ceiling);
        let fill = ((level * f32::from(ceiling)).round() as u16).min(outline);
        let colour = if col < played {
            theme.accent
        } else {
            theme.scrubber_track
        };
        let solid = Style::default().fg(colour);
        let dim = Style::default().fg(colour).add_modifier(Modifier::DIM);
        for row in 0..inner.height {
            let from_floor = inner.height - 1 - row;
            let Some((glyph, is_outline)) = wave_glyph(fill, outline, from_floor) else {
                continue;
            };
            if let Some(cell) = buf.cell_mut((inner.x + col, inner.y + row)) {
                cell.set_symbol(glyph)
                    .set_style(if is_outline { dim } else { solid });
            }
        }
    }
}

/// One column's glyph for one row, given the RMS fill and the peak outline,
/// both in eighths from the floor.
///
/// Returns the glyph and whether the cell is **outline only** — which is what
/// decides whether it is drawn solid or dimmed.
///
/// # Why this is not [`bar_glyph`]
///
/// The analyser's peak-hold is a *cap*: a mark at one height, drawn in the one
/// cell it lands in, with nothing between it and the bar. A waveform's peak is
/// an *outline*: the whole silhouette from the floor up, with the RMS body
/// solid inside it. So the outline fills every cell below it here, where the
/// cap fills only its own — and the two cannot share a function without one of
/// them lying about what it is drawing.
///
/// # The fill wins the cell it ends in, and this is the whole resolution
///
/// A cell carries one glyph and one colour, and this view paints no cell
/// backgrounds (`crate::ui::tests::no_cell_in_the_deck_paints_its_own_background`),
/// so a cell the fill ends partway up and the outline passes through cannot
/// show both. **The fill takes it**, and the outline's share of that one cell is
/// given up.
///
/// The alternative was tried against the real file and is why this is not
/// [`bar_glyph`]: letting the taller of the two win the glyph quantises the
/// fill to whole cells, and the fill's whole range on a mastered track is about
/// one and a half cells of a six-row block. Rounding it to cells rounds it to a
/// constant, which is the fault this change exists to remove — the eighths
/// *are* the picture. What it costs is a hairline of blank between the dim mass
/// above and the bright mass below, in the one cell where they meet, which is
/// how the fill's top edge is legible at all.
#[must_use]
fn wave_glyph(fill: u16, outline: u16, row_from_floor: u16) -> Option<(&'static str, bool)> {
    let floor = row_from_floor * 8;
    let fill_in_cell = fill.saturating_sub(floor).min(8);
    if fill_in_cell > 0 {
        return Some((EIGHTHS[usize::from(fill_in_cell - 1)], false));
    }
    let outline_in_cell = outline.saturating_sub(floor).min(8);
    (outline_in_cell > 0).then(|| (EIGHTHS[usize::from(outline_in_cell - 1)], true))
}

/// `mm:ss`, saturating at 99:59 — the footer's clock, at the same saturation.
#[must_use]
fn clock(ms: u64) -> String {
    let secs = (ms / 1000).min(99 * 60 + 59);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

// ── the signal readout ───────────────────────────────────────────────────────

/// The `DECODE` row: **the seal's own flags, restated as measurements.**
///
/// Each term is a stage of the chain and each one is read from
/// [`SignalFlags`] — the same booleans `signal_path::derive` built the seal
/// label from. They therefore cannot disagree: if the seal says `RESAMPLED`,
/// the first term here says `resampled`, because both are
/// `flags.resampled || flags.os_resampled`. `decode_agrees_with_the_seal_for_
/// every_combination_of_flags` is the assertion.
///
/// # `no dither` is not here, and this is why
///
/// There is no dither flag. `grep -rni dither` over `crates/` finds one thing:
/// nothing. `SignalFlags` reports EQ, attenuation, ReplayGain, engine
/// resampling and OS resampling, and `eko-core`'s output path neither dithers
/// nor reports that it does not. A `no dither` on this row would be a
/// measurement of a stage this program has never looked at — in the one
/// application whose whole claim is that its readouts are falsifiable.
///
/// Returns `(term, engaged)` pairs; `engaged` is what colours the term.
#[must_use]
pub fn decode_terms(flags: &SignalFlags) -> [(&'static str, bool); 3] {
    let resampled = flags.resampled || flags.os_resampled;
    let gain = flags.attenuated || flags.rg_active;
    [
        (
            if resampled {
                "resampled"
            } else {
                "no resampling"
            },
            resampled,
        ),
        (
            if flags.eq_active { "EQ" } else { "no EQ" },
            flags.eq_active,
        ),
        (if gain { "gain applied" } else { "no gain" }, gain),
    ]
}

/// `SOURCE` / `DECODE` / `OUTPUT` — the reason this view exists.
///
/// Every value is [`SignalPath`]'s, verbatim or derived from its flags. This
/// function constructs no measurement of its own, and when the seal is not
/// `active` — no engine, or an engine that has not described a whole stream yet
/// — every value is an em-dash rather than a plausible default.
///
/// # There is no `BUFFER` row
///
/// The brief asked for one. `EngineStatus` has no buffer depth, no underrun
/// count and no latency: the nearest field is `buffered_ms`, which is *decode
/// progress within the track*, and which this client itself only trusts for a
/// remote source — `App::seek_limit` reads it for `Source::Remote` and returns
/// `None` for a local file, because for a local file it is a restatement of the
/// duration. Printing decode progress under the word
/// `BUFFER` would be read as buffer depth by everyone who read it. So the row
/// is not shown, and no number was invented to fill it.
fn render_readout(f: &mut Frame, area: Rect, app: &App) {
    if area.is_empty() {
        return;
    }
    let theme = &app.theme;
    let seal = app.seal();
    let label = theme.on_console(theme.ink_faint);
    let value = theme.on_console(theme.ink_dim);
    let pad = usize::from(LABEL_W);
    let room = usize::from(area.width.saturating_sub(LABEL_W));

    let row = |name: &str, spans: Vec<Span<'static>>| -> Line<'static> {
        let mut out = vec![Span::styled(format!("{name:pad$}"), label)];
        out.extend(spans);
        Line::from(out)
    };

    let plain = |text: &str| vec![Span::styled(footer::fit(text, room), value)];
    let dash = || vec![Span::styled("—", value)];

    let (source, decode, output) = if seal.active {
        (
            plain(&seal.src),
            decode_spans(&seal, theme, room),
            plain(&seal.output),
        )
    } else {
        (dash(), dash(), dash())
    };

    f.render_widget(
        Paragraph::new(vec![
            row("SOURCE", source),
            row("DECODE", decode),
            row("OUTPUT", output),
        ]),
        area,
    );
}

/// The `DECODE` terms, joined by the chain separator and coloured per term.
///
/// An engaged stage is **amber**, never red and never green: red is the seal's
/// own emphasis for a rate change, and green is the seal's and nothing else's.
fn decode_spans(
    seal: &SignalPath,
    theme: &crate::ui::theme::Theme,
    room: usize,
) -> Vec<Span<'static>> {
    let terms = decode_terms(&seal.flags);
    let joined: String = terms
        .iter()
        .map(|(t, _)| *t)
        .collect::<Vec<_>>()
        .join(" · ");
    // Too narrow to show the terms apart: one truncated string, uncoloured, is
    // better than three that have each lost their ending.
    if joined.chars().count() > room {
        return vec![Span::styled(
            footer::fit(&joined, room),
            theme.on_console(theme.ink_dim),
        )];
    }
    let mut spans = Vec::with_capacity(terms.len() * 2);
    for (i, (text, engaged)) in terms.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", theme.on_console(theme.ink_faint)));
        }
        let style = if engaged {
            theme.on_console(theme.led_amber)
        } else {
            theme.on_console(theme.ink_dim)
        };
        spans.push(Span::styled(text, style));
    }
    spans
}

// ── the peak-hold decay ──────────────────────────────────────────────────────

/// How far a held peak falls per tick, as a fraction of full scale.
///
/// The tick is ~30fps while the analyser is up (see
/// [`crate::app::tick_interval`]), so this is a fall of full scale to the floor
/// in about a second and a half — slow enough to see where a transient reached,
/// fast enough not to leave a ceiling of stale caps over quiet music.
pub const PEAK_FALL: f32 = 0.022;

/// Bring the held peaks up to the new bands and let the rest fall.
///
/// Here rather than in `app.rs` because it is the analyser's own rule and the
/// constant above is the only thing that sets it; `App` calls it once per tick.
/// It holds the **engine's** levels; [`display_level`] is applied at draw time,
/// so a cap and its bar cannot be scaled differently.
pub fn hold_peaks(peaks: &mut Vec<f32>, bands: &[f32]) {
    if peaks.len() != BANDS {
        peaks.resize(BANDS, 0.0);
    }
    for (i, peak) in peaks.iter_mut().enumerate() {
        let level = bands.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        *peak = if level >= *peak {
            level
        } else {
            (*peak - PEAK_FALL).max(level).max(0.0)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{Accent, ColorDepth, Theme};

    /// A body rect of `w × h`, as [`body`] would have produced.
    fn body_of(w: u16, h: u16) -> Rect {
        Rect {
            x: 1,
            y: 1,
            width: w,
            height: h,
        }
    }

    /// **Exactly thirty-two bars, at every width, with a gap between each.**
    ///
    /// The one assertion in this file that is about the product's values rather
    /// than its layout: the count is the engine's `N_BANDS` and cannot become a
    /// function of the terminal.
    #[test]
    fn the_analyser_narrows_its_bars_and_never_drops_a_band() {
        for width in [63u16, 64, 94, 95, 96, 126, 127, 200, 512] {
            let spans = bands_across(width);
            assert_eq!(spans.len(), BANDS, "at {width}");
            let bw = bar_width(width).expect("no bar width");
            let mut previous: Option<(u16, u16)> = None;
            for (start, w) in &spans {
                assert_eq!(*w, bw, "at {width}: a bar of another width");
                if let Some((px, pw)) = previous {
                    assert_eq!(
                        *start,
                        px + pw + BAR_GAP,
                        "at {width}: the bars are not separated"
                    );
                }
                previous = Some((*start, *w));
            }
            let (last_x, last_w) = spans[BANDS - 1];
            assert!(last_x + last_w <= width, "at {width}: a bar ran over");
        }
        // Width degrades in the stated steps and never the count.
        assert_eq!(bar_width(159), Some(4));
        assert_eq!(bar_width(158), Some(3));
        assert_eq!(bar_width(127), Some(3));
        assert_eq!(bar_width(126), Some(2));
        assert_eq!(bar_width(95), Some(2));
        assert_eq!(bar_width(94), Some(1));
        assert_eq!(bar_width(63), Some(1));
        // Below one column each, **no analyser at all** — never a subset.
        assert_eq!(bar_width(62), None);
        assert!(bands_across(62).is_empty());
        assert!(bands_across(0).is_empty());
    }

    /// Nothing is ever drawn pinned at full scale, and the tilt cannot reorder
    /// two bands.
    #[test]
    fn the_display_curve_leaves_headroom_and_tilts_without_reordering() {
        for band in 0..BANDS {
            assert!(
                display_level(band, 1.0) < 1.0,
                "band {band} reaches the ceiling"
            );
            assert_eq!(display_level(band, 0.0), 0.0, "band {band} floats");
            // Monotonic in the engine's level.
            assert!(display_level(band, 0.5) < display_level(band, 0.6));
        }
        // Strictly tilted downward toward the highs, at equal level.
        for band in 1..BANDS {
            assert!(
                display_level(band, 1.0) < display_level(band - 1, 1.0),
                "band {band} is not below band {}",
                band - 1
            );
        }
        // The loudest band is drawn at the headroom, exactly.
        assert!((display_level(0, 1.0) - HEADROOM).abs() < 1e-6);
        // A level out of range is clamped rather than extrapolated.
        assert_eq!(display_level(0, 5.0), display_level(0, 1.0));
        assert_eq!(display_level(0, -1.0), 0.0);
        // Out-of-range band indices do not panic.
        let _ = display_level(BANDS + 9, 1.0);
    }

    /// **The decibel map is what makes the top of the spectrum visible.**
    ///
    /// The levels below are the medians the engine reports across four hundred
    /// windows of a real mastered track (*Emergency On Planet Earth*, 44.1 kHz
    /// FLAC) at bands 0, 8, 16, 24 and 31. Drawn linearly they are a bass end
    /// at a sixth of the block and a top end at seven thousandths of it — three
    /// of the five land inside one eighth-block of each other on a twenty-row
    /// analyser, which is to say they are the same bar.
    ///
    /// The assertion is that they are not the same bar any more: each is at
    /// least one whole cell taller than the band above it, and the top band has
    /// somewhere to be that is not the floor.
    #[test]
    fn real_band_levels_land_on_visibly_different_bars_under_the_decibel_curve() {
        // (band, the engine's median reported level for it)
        let measured = [
            (0usize, 0.1877f32),
            (8, 0.2043),
            (16, 0.1387),
            (24, 0.0650),
            (31, 0.0140),
        ];
        // A twenty-row analyser: 160 eighths, which is what `render_analyser`
        // quantises a level to.
        let rows = 20u16;
        let eighths =
            |band, level| (display_level(band, level) * f32::from(rows * 8)).round() as i32;

        // Before: a linear scale put three of the five within one cell.
        let linear = |band: usize, level: f32| {
            let t = band as f32 / (BANDS - 1) as f32;
            ((level.clamp(0.0, 1.0) * HEADROOM * (1.0 - TILT * t)) * f32::from(rows * 8)).round()
                as i32
        };
        let flat = measured
            .windows(2)
            .filter(|p| (linear(p[0].0, p[0].1) - linear(p[1].0, p[1].1)).abs() < 8)
            .count();
        assert!(
            flat >= 2,
            "the fixture does not reproduce the fault it is here to prove fixed"
        );
        assert!(
            linear(31, 0.0140) < 8,
            "the top band was already off the floor"
        );

        // After: every step is at least a whole cell, and the top band is up.
        for pair in measured.windows(2) {
            let (hi, lo) = (eighths(pair[0].0, pair[0].1), eighths(pair[1].0, pair[1].1));
            assert!(
                hi - lo >= 8,
                "bands {} and {} are the same bar: {hi} and {lo} eighths",
                pair[0].0,
                pair[1].0
            );
        }
        assert!(
            eighths(31, 0.0140) >= 16,
            "the top band is still on the floor: {} eighths",
            eighths(31, 0.0140)
        );
        // And it is still a scale with a bottom: the format's own noise floor
        // and anything under it draws nothing.
        assert_eq!(display_level(0, 0.0), 0.0);
        let at_floor = 10f32.powf(FLOOR_DBFS / 40.0);
        assert!(
            display_level(0, at_floor) < 1e-6,
            "the floor is not a floor"
        );
    }

    /// Eighth-blocks give eight sub-steps per cell, so `n` rows is `8n` levels
    /// and the bar reaches the top only at full scale.
    #[test]
    fn a_bar_has_eight_levels_per_row_and_fills_exactly_at_full_scale() {
        for row in 0..4u16 {
            assert_eq!(bar_glyph(32, 0, row), Some(("█", false)));
        }
        assert_eq!(bar_glyph(16, 0, 0), Some(("█", false)));
        assert_eq!(bar_glyph(16, 0, 1), Some(("█", false)));
        assert_eq!(bar_glyph(16, 0, 2), None);
        assert_eq!(bar_glyph(1, 0, 0), Some(("▁", false)));
        assert_eq!(bar_glyph(1, 0, 1), None);
        // Silence draws nothing at all — not a floor of glyphs.
        assert_eq!(bar_glyph(0, 0, 0), None);
    }

    /// The cap is composited into the bar's cell when they meet, and is its own
    /// glyph only above it.
    #[test]
    fn the_peak_cap_lands_in_the_bars_own_cell_rather_than_a_row_of_its_own() {
        assert_eq!(bar_glyph(12, 14, 0), Some(("█", false)));
        assert_eq!(bar_glyph(12, 14, 1), Some(("▆", false)));
        assert_eq!(bar_glyph(4, 20, 0), Some(("▄", false)));
        assert_eq!(bar_glyph(4, 20, 1), None);
        assert_eq!(bar_glyph(4, 20, 2), Some(("▄", true)));
        assert_eq!(bar_glyph(0, 8, 1), Some(("▁", true)));
    }

    /// The envelope's two heights, resolved per cell: the outline fills every
    /// cell below it, the fill takes the cell it ends in, and neither invents a
    /// glyph where there is nothing.
    #[test]
    fn the_envelope_fills_to_the_rms_and_outlines_to_the_peak() {
        // A full-height outline with a fill ten eighths up — the shape of very
        // nearly every column of a mastered track.
        assert_eq!(wave_glyph(10, 48, 0), Some(("█", false)));
        assert_eq!(wave_glyph(10, 48, 1), Some(("▂", false)));
        assert_eq!(wave_glyph(10, 48, 2), Some(("█", true)));
        assert_eq!(wave_glyph(10, 48, 5), Some(("█", true)));
        assert_eq!(wave_glyph(10, 48, 6), None);
        // **The eighths are the picture.** One eighth of level is one step of
        // glyph, where rounding the fill to whole cells would have drawn these
        // four columns identically.
        for (fill, glyph) in [(9u16, "▁"), (10, "▂"), (11, "▃"), (12, "▄")] {
            assert_eq!(wave_glyph(fill, 48, 1), Some((glyph, false)));
        }
        // A near-silent bucket is a bare outline, which is the reading a
        // peak-only envelope could not produce.
        assert_eq!(wave_glyph(0, 48, 0), Some(("█", true)));
        assert_eq!(wave_glyph(0, 1, 0), Some(("▁", true)));
        // And nothing at all above the outline.
        assert_eq!(wave_glyph(0, 0, 0), None);
    }

    /// Peaks rise instantly, fall slowly, and never sit under the bar.
    #[test]
    fn a_held_peak_follows_a_rise_at_once_and_a_fall_by_one_step() {
        let mut peaks = Vec::new();
        hold_peaks(&mut peaks, &[1.0; BANDS]);
        assert_eq!(peaks.len(), BANDS);
        assert!(peaks.iter().all(|&p| (p - 1.0).abs() < 1e-6));
        hold_peaks(&mut peaks, &[0.0; BANDS]);
        assert!((peaks[0] - (1.0 - PEAK_FALL)).abs() < 1e-6, "{}", peaks[0]);
        for _ in 0..100 {
            hold_peaks(&mut peaks, &[0.0; BANDS]);
        }
        assert!(peaks.iter().all(|&p| p == 0.0));
        hold_peaks(&mut peaks, &[0.5; BANDS]);
        assert!(peaks.iter().all(|&p| (p - 0.5).abs() < 1e-6));
        hold_peaks(&mut peaks, &[]);
        assert_eq!(peaks.len(), BANDS);
    }

    /// **The analyser is amber, and green belongs to the seal.**
    ///
    /// Asserted as colour rather than as a constant: every stop and every point
    /// between them has more red than green, at both colour depths, and none of
    /// them is the LED green the seal's lamp is drawn in.
    #[test]
    fn the_ramp_is_amber_at_every_height_and_never_the_seals_green() {
        use ratatui::style::Color;
        let full = Theme::new(Accent::Orange, ColorDepth::TrueColor);
        let flat = Theme::new(Accent::Orange, ColorDepth::Ansi256);
        for i in 0..=200 {
            let frac = i as f32 / 200.0;
            let c = ramp(&full, frac);
            let Color::Rgb(r, g, b) = c else {
                panic!("truecolor gave {c:?}");
            };
            assert!(r > g && g >= b, "{frac}: {r},{g},{b} is not an amber");
            assert_ne!(c, full.led_green, "{frac} landed on the seal's green");
            assert_ne!(c, full.led_green_dim, "{frac} landed on the seal's green");
            assert_ne!(ramp(&flat, frac), flat.led_green);
            assert_ne!(ramp(&flat, frac), flat.led_green_dim);
        }
        // The ends are the stated stops, and there really is a gradient between.
        assert_eq!(ramp(&full, 0.0), RAMP[0].resolve(full.depth));
        assert_eq!(ramp(&full, 1.0), RAMP[3].resolve(full.depth));
        let seen: std::collections::HashSet<String> = (0..=100)
            .map(|i| format!("{:?}", ramp(&full, i as f32 / 100.0)))
            .collect();
        assert!(seen.len() > 20, "the ramp had {} steps", seen.len());
    }

    /// **The readout cannot contradict the seal.** All thirty-two combinations.
    #[test]
    fn decode_agrees_with_the_seal_for_every_combination_of_flags() {
        use eko_core::signal_path::{derive, EqState, RgMode, SealInput, StreamInfo};

        for bits in 0..32u8 {
            let mut input = SealInput {
                engine_active: true,
                info: Some(StreamInfo {
                    rate: 96_000,
                    src_rate: 96_000,
                    dev_rate: 96_000,
                    bits: 24,
                    codec: "flac".into(),
                    device: "Topping E30".into(),
                }),
                eq: EqState {
                    gains: vec![0.0; 10],
                    ..Default::default()
                },
                volume: 1.0,
                replaygain_db: eko_core::signal_path::applied_replaygain_db(None),
                replaygain_mode: RgMode::Off,
            };
            // Reach each flag the way a real state would.
            if bits & 1 != 0 {
                input.info.as_mut().unwrap().src_rate = 44_100;
            }
            if bits & 2 != 0 {
                input.info.as_mut().unwrap().dev_rate = 48_000;
            }
            if bits & 4 != 0 {
                input.eq.enabled = true;
                input.eq.preamp = -3.0;
            }
            if bits & 8 != 0 {
                input.volume = 0.5;
            }
            if bits & 16 != 0 {
                input.replaygain_db = eko_core::signal_path::applied_replaygain_db(Some(-6.5));
                input.replaygain_mode = RgMode::Track;
            }
            let seal = derive(&input);
            let terms = decode_terms(&seal.flags);

            // Every term's engaged bit is the seal's own boolean.
            assert_eq!(
                terms[0].1,
                seal.seal_label.contains("RESAMPLED"),
                "{bits:05b}: resampling disagrees with {}",
                seal.seal_label
            );
            assert_eq!(
                terms[1].1,
                seal.seal_label.contains("EQ"),
                "{bits:05b}: EQ disagrees with {}",
                seal.seal_label
            );
            assert_eq!(
                terms[2].1,
                seal.seal_label.contains("VOLUME") || seal.seal_label.contains("REPLAYGAIN"),
                "{bits:05b}: gain disagrees with {}",
                seal.seal_label
            );
            // And a clean path says so in every term.
            if seal.pure {
                assert_eq!(
                    terms.map(|(t, _)| t),
                    ["no resampling", "no EQ", "no gain"],
                    "{bits:05b}: a bit-perfect path was not reported clean"
                );
            } else {
                assert!(
                    terms.iter().any(|(_, on)| *on),
                    "{bits:05b}: a broken seal reported nothing engaged"
                );
            }
        }
    }

    /// **Nothing about dither is reported, because nothing measures it.**
    ///
    /// A grep is not a test, so this is: the readout's own vocabulary, checked
    /// against the words it is allowed to use.
    #[test]
    fn the_readout_claims_nothing_the_engine_does_not_report() {
        for bits in 0..32u8 {
            let flags = SignalFlags {
                resampled: bits & 1 != 0,
                os_resampled: bits & 2 != 0,
                eq_active: bits & 4 != 0,
                attenuated: bits & 8 != 0,
                rg_active: bits & 16 != 0,
            };
            for (term, _) in decode_terms(&flags) {
                let t = term.to_lowercase();
                assert!(!t.contains("dither"), "{term}: there is no dither flag");
                assert!(!t.contains("buffer"), "{term}: there is no buffer depth");
                assert!(!t.contains("underrun"), "{term}: nothing counts underruns");
                assert!(
                    !t.contains("bit"),
                    "{term}: nothing measures bit depth here"
                );
            }
        }
    }

    /// The three sizes the owner runs, and the floor, **pinned to the cell**.
    ///
    /// A property test says the layout is never wrong; this says what it
    /// actually is. It is the test that would have failed on the bug that
    /// prompted this rewrite: at 230×56 the old layout drew a fourteen-row
    /// cover and left twenty-five rows of nothing above it.
    ///
    /// Read down the column: the cover, the analyser beside it and the envelope
    /// under both **all grow with the terminal**, and at every one of the four
    /// the last block ends on the body's own last row.
    #[test]
    fn the_layout_grows_with_the_terminal_at_the_pinned_sizes() {
        // terminal, cover rows, analyser (rows, bar width), envelope rows
        for (w, h, cover_rows, analyser, wave_rows) in [
            (230u16, 56u16, 28u16, Some((21u16, 3u16)), 10u16),
            (172, 40, 18, Some((11, 2)), 6),
            (100, 30, 10, Some((3, 1)), 6),
            // The floor. Sixty-eight content columns cannot hold a cover *and*
            // thirty-two bars beside it, so there is no analyser — and the
            // readout, whose three facts the footer also carries, is what the
            // thirteen rows were spent on instead.
            (80, 20, 8, None, 2),
        ] {
            let b = body(Rect::new(0, 0, w, h)).expect("no body");
            let c = content(b);
            let p = panels(b);
            let at = format!("{w}×{h}");

            // The hero, at the top margin, square, framed.
            assert_eq!(p.frame.y, c.y, "{at}: the hero did not start at the margin");
            assert_eq!(p.cover.height, cover_rows, "{at}: cover rows");
            assert_eq!(p.cover.width, cover_rows * 2, "{at}: cover columns");
            assert_eq!(p.frame.height, cover_rows + FRAME * 2, "{at}: frame");

            // The metadata beside it, and the analyser under the metadata.
            assert_eq!(p.meta.x, p.frame.right() + META_GAP, "{at}: meta column");
            assert_eq!(p.meta.y, p.cover.y, "{at}: meta row");
            assert_eq!(p.meta.height, META_ROWS, "{at}: meta rows");
            match analyser {
                Some((rows, bw)) => {
                    assert_eq!(p.analyser.height, rows, "{at}: analyser rows");
                    assert_eq!(bar_width(p.analyser.width), Some(bw), "{at}: bar width");
                    assert_eq!(bands_across(p.analyser.width).len(), BANDS, "{at}: bands");
                    // It ends on the sleeve's own last row: the column is full.
                    assert_eq!(p.analyser.y, p.cover.y + META_ROWS, "{at}: analyser row");
                    assert_eq!(p.analyser_foot.bottom(), p.cover.bottom(), "{at}: foot");
                }
                None => {
                    assert!(p.analyser.is_empty(), "{at}: an analyser appeared");
                    assert!(p.analyser_foot.is_empty(), "{at}: a caption appeared");
                }
            }

            // The readout, then the envelope, then the body's last row.
            assert_eq!(p.readout.is_empty(), h == 20, "{at}: readout");
            if !p.readout.is_empty() {
                assert_eq!(p.readout.height, READOUT_ROWS, "{at}: readout rows");
                assert_eq!(p.readout.width, c.width, "{at}: the readout is inset");
            }
            assert_eq!(p.wave.height, wave_rows, "{at}: envelope rows");
            assert_eq!(p.wave.width, c.width, "{at}: the envelope is inset");
            assert_eq!(
                p.wave.bottom(),
                c.bottom(),
                "{at}: the layout did not reach the bottom of the body"
            );
        }
    }

    /// **No size leaves a void, and no two blocks share a row.** The sweep.
    ///
    /// Every terminal from the Deck's own minimum up past anything anyone runs,
    /// asserting the five things the layout is not allowed to get wrong at any
    /// of them. (b) and the last clause of (a) are the two that were actually
    /// broken: the leading void the owner reported, and the block clock an
    /// earlier draft placed on top of the envelope by absolute row arithmetic.
    #[test]
    fn no_size_leaves_a_void_and_no_two_blocks_share_a_row() {
        for w in crate::ui::MIN_WIDTH..=260u16 {
            for h in crate::ui::MIN_HEIGHT..=70u16 {
                let b = body(Rect::new(0, 0, w, h)).expect("no body");
                let c = content(b);
                let p = panels(b);
                let at = format!("{w}×{h}");
                let drawn: Vec<Rect> = p.all().into_iter().filter(|r| !r.is_empty()).collect();
                assert!(!drawn.is_empty(), "{at}: nothing was laid out");

                // (a) **No leading void.** The topmost drawn row is the margin,
                //     and the bottom-most is the body's last row: the blocks
                //     spend the body rather than pooling anywhere.
                let top = drawn.iter().map(|r| r.y).min().unwrap();
                assert_eq!(top, c.y, "{at}: {} blank rows above the content", top - c.y);
                assert_eq!(p.frame.y, c.y, "{at}: the hero is not at the margin");
                assert_eq!(
                    drawn.iter().map(|r| r.bottom()).max().unwrap(),
                    c.bottom(),
                    "{at}: the layout stopped short of the body"
                );

                // (b) **No two blocks on one row.** The three stacked blocks
                //     are row-disjoint, and no two rects overlap at all — bar
                //     the cover, which is inside its own rule on purpose.
                for (i, a) in drawn.iter().enumerate() {
                    for other in drawn.iter().skip(i + 1) {
                        if (*a == p.frame && *other == p.cover)
                            || (*a == p.cover && *other == p.frame)
                        {
                            continue;
                        }
                        let disjoint = a.bottom() <= other.y
                            || other.bottom() <= a.y
                            || a.right() <= other.x
                            || other.right() <= a.x;
                        assert!(disjoint, "{at}: {a:?} and {other:?} overlap");
                    }
                }
                let mut stacked = [p.frame, p.readout, p.wave]
                    .into_iter()
                    .filter(|r| !r.is_empty())
                    .collect::<Vec<_>>();
                stacked.sort_by_key(|r| r.y);
                for pair in stacked.windows(2) {
                    assert!(
                        pair[0].bottom() <= pair[1].y,
                        "{at}: {:?} and {:?} share a row",
                        pair[0],
                        pair[1]
                    );
                }

                // (c) Everything inside the body, and inside its margins.
                for r in &drawn {
                    assert!(r.y >= c.y && r.bottom() <= c.bottom(), "{at}: {r:?}");
                    assert!(r.x >= c.x && r.right() <= c.right(), "{at}: {r:?}");
                    assert!(r.x >= b.x + MARGIN, "{at}: {r:?} is in the margin");
                    assert!(r.right() <= b.right() - MARGIN, "{at}: {r:?}");
                }

                // (d) The cover is square, framed, and inside its range.
                assert!(!p.cover.is_empty(), "{at}: no cover");
                assert_eq!(p.cover.width, p.cover.height * 2, "{at}: not square");
                assert!(
                    (COVER_MIN_ROWS..=COVER_MAX_ROWS).contains(&p.cover.height),
                    "{at}: a cover of {} rows",
                    p.cover.height
                );
                assert_eq!(p.frame.width, p.cover.width + FRAME * 2, "{at}: rule");
                assert_eq!(p.frame.height, p.cover.height + FRAME * 2, "{at}: rule");

                // (e) Thirty-two bands, or no analyser. Never a subset.
                if p.analyser.is_empty() {
                    assert!(p.analyser_foot.is_empty(), "{at}: a caption with no bars");
                } else {
                    assert_eq!(bands_across(p.analyser.width).len(), BANDS, "{at}: bands");
                    assert!(p.analyser.height >= ANALYSER_MIN_ROWS, "{at}: a flicker");
                    assert_eq!(p.analyser_foot.width, p.analyser.width, "{at}: caption");
                }
            }
        }
    }

    /// The layout gives way in the stated order, and the cover shrinks before
    /// anything is dropped.
    #[test]
    fn the_panels_give_way_in_the_stated_order() {
        // A body wide enough that only the height ever binds.
        let wide = 218u16;

        // Tall: the cover at its cap, and the slack in the gaps and the
        // envelope rather than above the content.
        let big = fit(60, wide, false).expect("no fit at 60 rows");
        assert_eq!(big.cover_rows, COVER_MAX_ROWS, "the cover did not reach it");
        assert_eq!(big.gap, GAP_MAX, "the gaps did not grow");
        assert!(big.readout);
        assert!(
            big.wave_rows > WAVE_MIN_ROWS * 4,
            "the envelope did not take the slack: {} rows",
            big.wave_rows
        );

        // **It grows.** Every extra row of body buys a row somewhere, and the
        // cover is strictly larger on a larger body until it reaches the cap.
        let mut previous = fit(13, wide, false).unwrap();
        for h in 14..=80u16 {
            let f = fit(h, wide, false).unwrap();
            assert!(
                f.cover_rows >= previous.cover_rows,
                "the cover shrank at {h}"
            );
            assert!(f.cover_rows <= COVER_MAX_ROWS, "past the cap at {h}");
            previous = f;
        }
        assert!(
            fit(56, wide, false).unwrap().cover_rows > fit(33, wide, false).unwrap().cover_rows,
            "a taller terminal did not buy a bigger picture"
        );

        // The exact height at which everything first fits: the cover at its
        // floor, every block at its own floor, gaps at their base.
        let full =
            COVER_MIN_ROWS + FRAME * 2 + BLOCK_GAP + READOUT_ROWS + BLOCK_GAP + WAVE_MIN_ROWS;
        let f = fit(full, wide, false).unwrap();
        assert!(f.readout, "the readout did not fit where it should");
        assert_eq!(f.cover_rows, COVER_MIN_ROWS);
        assert_eq!(f.wave_rows, WAVE_MIN_ROWS);
        assert_eq!(f.gap, BLOCK_GAP);

        // One row short: the **readout** goes, and nothing else.
        let f = fit(full - 1, wide, false).unwrap();
        assert!(!f.readout, "the readout did not go first");
        assert!(f.wave_rows >= WAVE_MIN_ROWS, "the envelope went first");
        assert_eq!(f.cover_rows, COVER_MIN_ROWS);

        // Then the envelope, and the cover is the last thing standing.
        let no_readout = COVER_MIN_ROWS + FRAME * 2 + BLOCK_GAP + WAVE_MIN_ROWS;
        let f = fit(no_readout - 1, wide, false).unwrap();
        assert_eq!(f.wave_rows, 0, "the envelope did not go next");
        assert!(!f.readout);
        assert_eq!(f.cover_rows, COVER_MIN_ROWS);

        // And below everything there is still a cover, or nothing at all.
        assert!(fit(3, wide, false).unwrap().cover_rows >= 1);
        assert_eq!(fit(2, wide, false), None);
        assert_eq!(fit(0, wide, false), None);
        assert_eq!(fit(60, 2, false), None);
    }

    /// **The picture yields columns to the bands, and never the other way.**
    ///
    /// The cover and the analyser share one row of columns now, so a cover free
    /// to take its height share at every width would push the analyser off a
    /// hundred-column terminal — which is a size the owner runs. The width bound
    /// in [`wanted_cover_rows`] is what stops it, and this is that bound:
    /// wherever the content is wide enough for a cover *and* thirty-two bars
    /// beside it, there are thirty-two bars.
    #[test]
    fn the_cover_gives_up_columns_before_the_analyser_gives_up_bands() {
        // The narrowest content that can hold the smallest cover, its rule, the
        // gap and thirty-two single-column bars.
        let tight = COVER_MIN_ROWS * 2 + FRAME * 2 + META_GAP + COLUMN_MIN;
        for w in tight..=(tight + 60) {
            // Tall enough that only the width can bind.
            let f = fit(60, w, false).unwrap();
            let column = w - (f.cover_rows * 2 + FRAME * 2) - META_GAP;
            assert!(
                bar_width(column).is_some(),
                "{w} columns: the cover took the analyser's, leaving {column}"
            );
            assert!(f.cover_rows >= COVER_MIN_ROWS, "{w}: the cover was crushed");
        }
        // One column narrower and the honest answer is no analyser at all —
        // never fewer than thirty-two bands.
        let f = fit(60, tight - 1, false).unwrap();
        assert_eq!(
            f.cover_rows, COVER_MIN_ROWS,
            "the cover shrank past its floor"
        );
        assert!(bar_width(tight - 1 - (f.cover_rows * 2 + FRAME * 2) - META_GAP).is_none());
    }

    /// **The strip's width ladder degrades, and never drops a band.**
    ///
    /// The arithmetic [`STRIP_BAR_WIDTHS`] states, asserted rather than trusted:
    /// thirty-two seven-cell bars with a gap need 255 columns, six need 223,
    /// five need 191, one needs 63 — and below 63 there is no strip, rather than
    /// a strip with fewer than thirty-two bands in it.
    #[test]
    fn the_strip_degrades_in_width_and_never_in_count() {
        let bands = BANDS as u16;
        for (columns, want) in [(255u16, 7u16), (223, 6), (191, 5), (159, 4), (63, 1)] {
            assert_eq!(
                strip_bar_width(columns),
                Some(want),
                "{columns} columns is exactly {want} cells a bar"
            );
            // One column short of the exact fit takes the next rung down, never
            // the same bar with a band missing.
            let narrower = strip_bar_width(columns - 1);
            assert!(
                narrower < Some(want),
                "{} columns did not step down from {want}",
                columns - 1
            );
        }
        // Below one column each there is no strip at all.
        assert_eq!(strip_bar_width(bands + bands - 1 - 1), None);
        assert_eq!(strip_bar_width(0), None);
        // Whatever it returns, thirty-two of them fit with a gap between each.
        for w in 0..=400u16 {
            if let Some(bw) = strip_bar_width(w) {
                assert!(
                    bands * bw + (bands - 1) * BAR_GAP <= w,
                    "{w}: {bw}-cell bars do not fit thirty-two"
                );
            }
        }
    }

    /// **The cover is the same rect with the words and without them.**
    ///
    /// The invariant [`panels_for`] is built around, and the reason
    /// [`fit_with_lyrics`] takes [`base_fit`]'s cover as given. Lyrics arrive
    /// from a worker a few hundred milliseconds into a track; a cover that
    /// resized when they landed would change [`crate::art::Key`], refetch and
    /// re-encode the picture, and flash the placeholder mid-song.
    #[test]
    fn the_words_arriving_cannot_move_the_cover() {
        for w in (24u16..=240).step_by(7) {
            for h in 3u16..=80 {
                let b = body_of(w, h);
                let without = panels_for(b, false);
                let with = panels_for(b, true);
                assert_eq!(with.cover, without.cover, "{w}×{h}: the cover moved");
                assert_eq!(with.frame, without.frame, "{w}×{h}: the rule moved");
                assert_eq!(with.meta, without.meta, "{w}×{h}: the metadata moved");
            }
        }
    }

    /// **With the words, the blocks give way in the order the docs state:**
    /// the readout, then the strip, then the envelope.
    ///
    /// Asserted as the implication chain rather than at three magic heights, so
    /// it holds however the constants move: wherever the readout survives the
    /// strip does, and wherever the strip survives the envelope does.
    #[test]
    fn with_the_words_the_blocks_give_way_in_the_stated_order() {
        // Wide enough that thirty-two bars always fit, so only height binds.
        let wide = 218u16;
        let mut seen_strip = false;
        let mut seen_readout = false;
        for h in 3u16..=90 {
            let Some(f) = fit(h, wide, true) else {
                continue;
            };
            if f.readout {
                assert!(f.strip_rows > 0, "{h}: the readout outlived the strip");
                seen_readout = true;
            }
            if f.strip_rows > 0 {
                assert!(f.wave_rows > 0, "{h}: the strip outlived the envelope");
                assert!(
                    (STRIP_MIN_ROWS..=STRIP_MAX_ROWS).contains(&f.strip_rows),
                    "{h}: {} rows of strip is outside the band",
                    f.strip_rows
                );
                seen_strip = true;
            }
            // Without the words there is no strip at any height — the analyser
            // is beside the cover instead.
            assert_eq!(
                fit(h, wide, false).unwrap().strip_rows,
                0,
                "{h}: a strip appeared without lyrics"
            );
        }
        assert!(seen_strip, "the strip never appeared at any height");
        assert!(seen_readout, "the readout never appeared at any height");
    }

    /// Too narrow for thirty-two bars means **no strip at any height**, rather
    /// than rows reserved for a band that cannot be drawn.
    #[test]
    fn a_body_too_narrow_for_the_bands_has_no_strip() {
        for h in 3u16..=90 {
            for w in [24u16, 40, 55] {
                let b = body_of(w, h);
                if strip_bar_width(content(b).width).is_some() {
                    continue; // wide enough after all — not this test's case
                }
                let p = panels_for(b, true);
                assert!(
                    p.analyser.is_empty() || !p.lyrics.is_empty(),
                    "{w}×{h}: a strip was drawn where no band fits"
                );
                if let Some(f) = fit(h, w, true) {
                    assert_eq!(f.strip_rows, 0, "{w}×{h}: rows reserved for no strip");
                }
            }
        }
    }

    /// **The layout cannot move when an envelope or a cover arrives**, because
    /// it never asked whether there was one.
    #[test]
    fn the_layout_is_a_function_of_the_rect_and_nothing_else() {
        for (w, h) in [(78u16, 13u16), (98, 23), (118, 33), (238, 73)] {
            let p = panels(body_of(w, h));
            assert_eq!(p, panels(body_of(w, h)));
        }
    }

    /// Nothing the layout produces overlaps anything else, or leaves the body
    /// less its margins — which is what keeps the readout off the envelope and
    /// both of them off the footer.
    ///
    /// The sizes here are the ones the sweep cannot reach: bodies far below the
    /// Deck's own minimum, where [`fit`] is degrading rather than growing.
    #[test]
    fn no_panel_overlaps_another_or_escapes_the_margins() {
        for (w, h) in [
            (78u16, 13u16),
            (98, 23),
            (108, 29),
            (118, 33),
            (238, 73),
            (40, 8),
            (24, 30),
            (12, 4),
            (10, 3),
            (0, 0),
        ] {
            let b = body_of(w, h);
            let c = content(b);
            let p = panels(b);
            let rects = p.all();
            // Square even here: a body too narrow for the cover's floor shrinks
            // the picture rather than letting the rule crop one side of it.
            if !p.cover.is_empty() {
                assert_eq!(p.cover.width, p.cover.height * 2, "{w}×{h}: not square");
                assert_eq!(p.frame.width, p.cover.width + FRAME * 2, "{w}×{h}: rule");
            }
            for r in rects {
                if r.is_empty() {
                    continue;
                }
                assert!(r.x >= c.x && r.right() <= c.right(), "{w}×{h}: {r:?}");
                assert!(r.y >= c.y && r.bottom() <= c.bottom(), "{w}×{h}: {r:?}");
                // And inside the margins, which is the whole point of them.
                assert!(r.x >= b.x + MARGIN, "{w}×{h}: {r:?} is in the margin");
                assert!(
                    r.right() <= b.right() - MARGIN,
                    "{w}×{h}: {r:?} is in the margin"
                );
            }
            // The cover is inside the frame on purpose; every other pair is
            // disjoint.
            for (i, a) in rects.iter().enumerate() {
                for b2 in rects.iter().skip(i + 1) {
                    if a.is_empty() || b2.is_empty() {
                        continue;
                    }
                    if (*a == p.frame && *b2 == p.cover) || (*a == p.cover && *b2 == p.frame) {
                        continue;
                    }
                    let disjoint = a.bottom() <= b2.y
                        || b2.bottom() <= a.y
                        || a.right() <= b2.x
                        || b2.right() <= a.x;
                    assert!(disjoint, "{w}×{h}: {a:?} and {b2:?} overlap");
                }
            }
        }
    }

    /// Letter-spaced caps, including what happens to the spaces.
    #[test]
    fn spaced_caps_spaces_the_spaces_too() {
        assert_eq!(spaced_caps("when you"), "W H E N   Y O U");
        assert_eq!(spaced_caps("a"), "A");
        assert_eq!(spaced_caps(""), "");
        // Non-ASCII survives, and is upper-cased the way Rust does it.
        assert_eq!(spaced_caps("café"), "C A F É");
    }

    #[test]
    fn the_clock_saturates_where_the_footers_clock_does() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(291_400), "04:51");
        assert_eq!(clock(u64::MAX), "99:59");
    }
}
