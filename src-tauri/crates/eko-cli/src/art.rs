//! Album art, as real pixels.
//!
//! Three renderers, one of which is always available:
//!
//! | protocol | terminals | where the pixels go |
//! |---|---|---|
//! | [`Protocol::Kitty`] | kitty, Ghostty, WezTerm | an APC escape, **outside** ratatui's buffer |
//! | [`Protocol::Iterm2`] | iTerm2 | an OSC 1337 escape, **outside** ratatui's buffer |
//! | [`Protocol::Halfblock`] | everything else | `▀` cells, **inside** ratatui's buffer |
//!
//! No sixel. It is a fourth code path with its own quirks, and its terminals
//! skew towards Linux desktops, which is not this phase.
//!
//! # Detection claims nothing it cannot support
//!
//! [`protocol_from_env`] is a pure function over a handful of environment
//! variables and is the only place the decision is made. Every rule in it is a
//! *guess* — an environment variable says which program wrote it, not what the
//! program on the other end of fd 1 can draw. So the guesses are ordered to fail
//! **downwards**: anything not positively recognised is [`Protocol::Halfblock`],
//! which needs nothing from the terminal but a `▀` and colour, and which ratatui
//! itself is responsible for painting and repainting.
//!
//! Two negative rules matter more than the positive ones:
//!
//! * **A multiplexer wins.** `tmux` and GNU `screen` re-emit their pane contents
//!   from their own cell model and do not forward APC or OSC image payloads; a
//!   kitty escape sent through one is at best swallowed and at worst printed. If
//!   `TMUX` is set, or `TERM` starts with `tmux` or `screen`, the answer is
//!   halfblock no matter what else is set — including `TERM=xterm-kitty`, which
//!   a tmux session started from kitty inherits.
//! * **`TERM_PROGRAM` does not survive `ssh`.** Neither does `KITTY_WINDOW_ID`.
//!   `TERM` does, and `LC_TERMINAL` does (iTerm2 sets it precisely so it can be
//!   forwarded by `SendEnv`). So a remote session usually degrades to halfblock,
//!   which is the safe direction to be wrong in: both graphics protocols are
//!   in-band and would in fact have worked.
//!
//! # The bytes are hostile until proven otherwise
//!
//! Cover art for a remote track is whatever the server chose to answer with. It
//! is never handed to the terminal as it arrived — it is size-capped on the way
//! in ([`MAX_FETCH_BYTES`]), decoded under explicit [`image::Limits`], and
//! **re-encoded by this process** before a single byte reaches an escape
//! sequence. A terminal emulator's image decoder is a far more interesting
//! attack surface than this one, and forwarding an untrusted payload straight
//! into it would be handing it over.
//!
//! # Nothing here runs on the render thread
//!
//! [`spawn`] does the fetch, the decode and the encode on a worker and posts one
//! [`Event`] back to the fold, stamped with the generation it was started under
//! — the same drop-the-stale-answer rule [`crate::remote::spawn_albums`] uses.
//! [`load`] is that same work as a plain blocking function, so a test can drive
//! it without a thread.

use std::fmt;
use std::io::{self, Read, Write};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader, Limits};

use crate::ui::theme::Rgb;

// ── Bounds on untrusted input ────────────────────────────────────────────────

/// Ceiling on the compressed bytes accepted for one cover, from anywhere.
///
/// A cover that needs more than this is not a cover. The read is capped rather
/// than trusted: `Content-Length` is a claim, and a server that lies about it —
/// or omits it and streams forever — must not be able to grow this process
/// without bound. The body is read through a [`Read::take`] set one byte past
/// the limit, so "exactly at the limit" and "over it" are distinguishable and
/// the over case is refused rather than silently truncated into a decode
/// failure.
pub const MAX_FETCH_BYTES: usize = 8 * 1024 * 1024;

/// Ceiling on either edge of the *decoded* image.
///
/// Independent of [`MAX_FETCH_BYTES`], and the one that matters: a few dozen
/// bytes of PNG header can declare a 100,000 × 100,000 canvas, and a decoder
/// that believes it allocates tens of gigabytes before it has read a pixel.
/// `image` enforces this from the header, so the allocation never happens.
pub const MAX_DECODE_EDGE: u32 = 8_192;

/// Ceiling on what the decoder may allocate for one cover. Belt to
/// [`MAX_DECODE_EDGE`]'s braces, and the limit that catches formats whose
/// dimensions are not knowable from the header alone.
pub const MAX_DECODE_ALLOC: u64 = 64 * 1024 * 1024;

/// How long a cover fetch may take before it is abandoned.
///
/// Generous, because it is off the render thread and nothing waits on it: the
/// footer shows the placeholder until an answer arrives, and no answer at all is
/// simply a placeholder that stays.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Edge, in pixels, asked of the server.
///
/// Small on purpose. The largest grid this ever paints is a handful of cells, so
/// a 300-pixel thumbnail is already several times more detail than survives the
/// resize — and it is the size [`eko_net::urls::DEFAULT_COVER_SIZE`] asks for
/// elsewhere, so a server's thumbnail cache is shared rather than doubled.
pub const REQUEST_EDGE: u32 = 300;

// ── Assumed cell geometry ────────────────────────────────────────────────────

/// Assumed width of one terminal cell, in pixels, for the graphics protocols.
///
/// A guess, and it does not have to be right. Both protocols are told the
/// **cell** extent (`c`/`r` for kitty, `width`/`height` for iTerm2) and scale
/// the image into it themselves; this only decides how many source pixels they
/// are given to scale from. Ten by twenty is a common 1:2 cell at a typical font
/// size, and being wrong costs sharpness, never layout.
const CELL_PX_W: u32 = 10;
/// Assumed height of one terminal cell, in pixels. See [`CELL_PX_W`].
const CELL_PX_H: u32 = 20;

/// The kitty image id this module owns.
///
/// One id, reused for every cover, because there is only ever one cover on
/// screen. Transmitting under the same id replaces the previous image's data
/// rather than accumulating a new one per track — a four-thousand-track session
/// would otherwise leave four thousand images in the terminal's cache — and it
/// gives [`Painter`] a precise thing to delete instead of `d=A`, which would
/// also destroy images placed by anything else sharing the terminal.
const KITTY_IMAGE_ID: u32 = 6_804;

/// Base64 characters per kitty transmission chunk. The protocol's own maximum.
const KITTY_CHUNK: usize = 4_096;

// ── Capability detection ─────────────────────────────────────────────────────

/// Which renderer this terminal gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    /// `▀` with a foreground and a background colour — two vertical pixels per
    /// cell, drawn by ratatui like any other text. The universal fallback, and
    /// the only one that cannot corrupt a frame.
    #[default]
    Halfblock,
    /// The kitty graphics protocol (APC `_G`). kitty, Ghostty, WezTerm.
    Kitty,
    /// iTerm2 inline images (OSC 1337 `File=`).
    Iterm2,
}

/// The environment [`protocol_from_env`] reads, as data.
///
/// A struct rather than direct `std::env::var` calls, so the decision is a pure
/// function with a table of cases. Process-global environment variables are
/// shared mutable state that `cargo test`'s threads would fight over; nothing in
/// this module reads them except [`Env::from_process`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `TERM`.
    pub term: Option<String>,
    /// `TERM_PROGRAM`.
    pub term_program: Option<String>,
    /// `KITTY_WINDOW_ID`.
    pub kitty_window_id: Option<String>,
    /// `LC_TERMINAL` — iTerm2 sets this so it can be forwarded over `ssh`.
    pub lc_terminal: Option<String>,
    /// `TMUX`.
    pub tmux: Option<String>,
}

impl Env {
    /// Read the real process environment. Empty values read as absent, because
    /// `TERM_PROGRAM=` is not a terminal.
    #[must_use]
    pub fn from_process() -> Self {
        fn var(name: &str) -> Option<String> {
            std::env::var(name).ok().filter(|v| !v.is_empty())
        }
        Self {
            term: var("TERM"),
            term_program: var("TERM_PROGRAM"),
            kitty_window_id: var("KITTY_WINDOW_ID"),
            lc_terminal: var("LC_TERMINAL"),
            tmux: var("TMUX"),
        }
    }
}

/// Decide which renderer to use. **The only place the decision is made.**
///
/// Read the module docs before adding a rule: every one of these is a guess, and
/// the cost of guessing wrong upwards is a frame full of garbage on somebody
/// else's terminal.
#[must_use]
pub fn protocol_from_env(env: &Env) -> Protocol {
    let term = env.term.as_deref().unwrap_or_default().to_ascii_lowercase();

    // ── the negative rules, first ────────────────────────────────────────
    // A multiplexer re-renders its panes from its own cell model. Neither
    // protocol's payload survives that, and `TERM=xterm-kitty` is inherited
    // straight through a tmux session started from kitty — so the positive rules
    // below would all fire on exactly the terminal that cannot draw it.
    if env.tmux.is_some() || term.starts_with("tmux") || term.starts_with("screen") {
        return Protocol::Halfblock;
    }

    // ── kitty graphics ───────────────────────────────────────────────────
    // `KITTY_WINDOW_ID` is set by kitty itself. `TERM` is the terminfo entry
    // kitty and Ghostty ship and, unlike `TERM_PROGRAM`, survives `ssh`.
    if env.kitty_window_id.is_some() || term == "xterm-kitty" || term == "xterm-ghostty" {
        return Protocol::Kitty;
    }
    match env.term_program.as_deref() {
        // Ghostty writes `ghostty`; WezTerm writes `WezTerm`.
        Some(p) if p.eq_ignore_ascii_case("ghostty") || p.eq_ignore_ascii_case("wezterm") => {
            return Protocol::Kitty
        }
        // ── iTerm2 inline images ─────────────────────────────────────────
        Some(p) if p.eq_ignore_ascii_case("iterm.app") => return Protocol::Iterm2,
        _ => {}
    }
    if env
        .lc_terminal
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("iterm2"))
    {
        return Protocol::Iterm2;
    }

    Protocol::Halfblock
}

/// [`protocol_from_env`] against the real environment.
#[must_use]
pub fn detect() -> Protocol {
    protocol_from_env(&Env::from_process())
}

// ── What is being asked for ──────────────────────────────────────────────────

/// Everything that decides whether a rendered cover is still the right one.
///
/// **This is the cache key**, and it is compared by value on every fold. A
/// change in any field means the pixels on screen are wrong and a new fetch is
/// started; no change means nothing happens at all — which is the whole point,
/// because a four-thousand-track library at thirty frames a second must not
/// refetch.
///
/// `cols`/`rows` are in it because a resize changes what has to be drawn even
/// though the track did not, and `protocol` is in it because the encoded form is
/// protocol-specific: the cache stores the answer *already encoded for this
/// terminal*, not a decoded image that would have to be re-encoded per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// What identifies the art itself — a local path or a remote track id,
    /// prefixed so the two namespaces cannot collide.
    pub token: String,
    pub cols: u16,
    pub rows: u16,
    pub protocol: Protocol,
}

/// Where one cover's bytes come from.
///
/// # Redacting `Debug`
///
/// [`Fetch::Remote`] holds a **pre-signed** `getCoverArt` URL: `u`, `t` and `s`,
/// which together are a replayable credential for as long as the password
/// stands. It reaches a worker thread inside a value that would otherwise be
/// printable by any `{:?}` — a panic message, a failing assertion, a future log
/// line. Same reason [`crate::server::Connection`] hand-writes one.
#[derive(Clone)]
pub enum Fetch {
    /// A file on disk: its embedded picture, or a sidecar image beside it.
    Local(String),
    /// A signed, direct `getCoverArt` URL. See the type docs.
    Remote(String),
}

impl fmt::Debug for Fetch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fetch::Local(path) => f.debug_tuple("Local").field(path).finish(),
            Fetch::Remote(_) => f.debug_tuple("Remote").field(&"<signed url>").finish(),
        }
    }
}

// ── What comes back ──────────────────────────────────────────────────────────

/// A halfblock grid: `cols × rows` cells, two vertical pixels each.
#[derive(Clone, PartialEq, Eq)]
pub struct Cells {
    pub cols: u16,
    pub rows: u16,
    /// `cols * rows` pairs in row-major order: the upper half-pixel of the cell
    /// and the lower one. The upper becomes the `▀`'s foreground and the lower
    /// its background.
    pub pixels: Vec<(Rgb, Rgb)>,
}

/// `Debug` prints the size, not twenty-four colour triples.
impl fmt::Debug for Cells {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cells")
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("pixels", &self.pixels.len())
            .finish()
    }
}

impl Cells {
    /// The pair at `(col, row)`, or `None` outside the grid.
    #[must_use]
    pub fn at(&self, col: u16, row: u16) -> Option<(Rgb, Rgb)> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.pixels
            .get(usize::from(row) * usize::from(self.cols) + usize::from(col))
            .copied()
    }
}

/// A ready-to-write escape sequence, and the block it will occupy.
///
/// The sequence positions **nothing**: [`Painter`] moves the cursor to the top
/// left of the block, writes this, and puts the cursor back. Keeping the two
/// apart is what lets a test assert on the payload without a terminal, and what
/// keeps the one piece of cursor bookkeeping in one place.
#[derive(Clone, PartialEq, Eq)]
pub struct Escape {
    pub protocol: Protocol,
    pub cols: u16,
    pub rows: u16,
    payload: String,
}

impl Escape {
    /// The raw sequence. Meaningless without [`Painter`]'s positioning.
    #[must_use]
    pub fn payload(&self) -> &str {
        &self.payload
    }
}

/// `Debug` prints the length, never the payload — it is tens of kilobytes of
/// base64 wrapped in control characters, and a `{:?}` that dumped it into a
/// panic message would take the terminal with it.
impl fmt::Debug for Escape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Escape")
            .field("protocol", &self.protocol)
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("payload", &format_args!("{} bytes", self.payload.len()))
            .finish()
    }
}

/// A cover, encoded for the terminal that asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendered {
    /// Halfblock. ratatui owns these cells and repaints them like any others.
    Cells(Cells),
    /// kitty or iTerm2. Written outside the buffer by [`Painter`].
    Escape(Escape),
}

/// How a cover request ended.
///
/// There is deliberately no error *message*. A track with no art, a 404, a
/// decode failure and a server that answered a login page all end in the same
/// place — the footer's placeholder — because on a four-row footer they are the
/// same fact ("no picture"), and a note about any of them would push the chain
/// off the seal row for something nobody can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ready(Rendered),
    Unavailable,
}

/// One cover request's answer, stamped with the generation it was started under.
///
/// The stamp is the whole of the staleness rule: a fold that has moved on has
/// incremented its counter, and an answer whose generation no longer matches is
/// dropped rather than drawn. Without it, a slow fetch for track A landing after
/// track B started would paint A's cover under B's title.
#[derive(Debug, Clone)]
pub struct Event {
    pub generation: u64,
    pub key: Key,
    pub outcome: Outcome,
}

// ── The work ─────────────────────────────────────────────────────────────────

/// Fetch, decode and encode one cover. **Blocking**; see [`spawn`].
///
/// Separated from the spawn so a test can drive it synchronously against a mock
/// server, or against a generated image, with no thread in the way.
#[must_use]
pub fn load(fetch: &Fetch, key: &Key) -> Outcome {
    let Some(bytes) = read_bytes(fetch) else {
        return Outcome::Unavailable;
    };
    encode(&bytes, key)
}

/// Decode `bytes` and encode them for `key`'s protocol and grid.
///
/// Split out from [`load`] so the untrusted-input path is testable with no I/O
/// at all: hand it a hostile buffer and it must answer [`Outcome::Unavailable`]
/// rather than panicking or allocating.
#[must_use]
pub fn encode(bytes: &[u8], key: &Key) -> Outcome {
    if key.cols == 0 || key.rows == 0 {
        return Outcome::Unavailable;
    }
    let (px_w, px_h) = match key.protocol {
        // Two vertical pixels per cell, one horizontal. That is the whole
        // resolution of the halfblock renderer.
        Protocol::Halfblock => (u32::from(key.cols), u32::from(key.rows) * 2),
        Protocol::Kitty | Protocol::Iterm2 => (
            u32::from(key.cols) * CELL_PX_W,
            u32::from(key.rows) * CELL_PX_H,
        ),
    };
    let Some(image) = decode_within_limits(bytes) else {
        return Outcome::Unavailable;
    };
    // `resize_to_fill` centre-crops to the target aspect and *then* scales.
    // Nothing is stretched — and that is not the interesting part. The
    // interesting part is that whatever does not match the target aspect is
    // **thrown away**, from the middle outwards.
    //
    // So the block's aspect ratio is a decision about how much of the cover the
    // listener gets to see. `crate::ui::footer::ART_WIDTH` is 8 against
    // `FOOTER_HEIGHT`'s 4 precisely so that `cols × 2 == rows × 4`, making the
    // block square in device pixels and the crop a no-op for the square covers
    // that all album art is. At the old width of 6 the target was 3:4 and a
    // quarter of every cover's width was cut off before it was ever drawn.
    let fitted = image.resize_to_fill(px_w, px_h, FilterType::Lanczos3);

    match key.protocol {
        Protocol::Halfblock => Outcome::Ready(Rendered::Cells(halfblock(
            &fitted.to_rgb8(),
            key.cols,
            key.rows,
        ))),
        Protocol::Kitty => Outcome::Ready(Rendered::Escape(Escape {
            protocol: Protocol::Kitty,
            cols: key.cols,
            rows: key.rows,
            payload: kitty_payload(fitted.to_rgba8().as_raw(), px_w, px_h, key.cols, key.rows),
        })),
        Protocol::Iterm2 => {
            // Re-encoded by this process — the server's bytes never reach the
            // terminal's decoder. See the module docs.
            let mut png = Vec::new();
            if fitted
                .write_to(&mut io::Cursor::new(&mut png), ImageFormat::Png)
                .is_err()
            {
                return Outcome::Unavailable;
            }
            Outcome::Ready(Rendered::Escape(Escape {
                protocol: Protocol::Iterm2,
                cols: key.cols,
                rows: key.rows,
                payload: iterm2_payload(&png, key.cols, key.rows),
            }))
        }
    }
}

/// Run [`load`] on a worker thread and post the answer back.
///
/// `emit` returns `false` once the fold has hung up; nothing here acts on that,
/// because there is exactly one message and it is the last thing the thread
/// does.
pub fn spawn<F>(fetch: Fetch, key: Key, generation: u64, emit: F)
where
    F: FnOnce(Event) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        let outcome = load(&fetch, &key);
        emit(Event {
            generation,
            key,
            outcome,
        });
    });
}

/// The compressed bytes of one cover, or `None` for anything that did not work.
fn read_bytes(fetch: &Fetch) -> Option<Vec<u8>> {
    match fetch {
        Fetch::Local(path) => local_bytes(path),
        Fetch::Remote(url) => remote_bytes(url),
    }
}

/// A local track's embedded picture, or a sidecar image beside it.
///
/// Goes through `eko_core::metadata::read_cover` rather than reading tags again
/// here: it is the same reader the desktop app uses, it already prefers an
/// embedded picture and falls back to `cover.jpg` / `folder.jpg` / … in the same
/// folder, and duplicating that search order in a second crate is how two front
/// ends come to disagree about which file is the cover.
///
/// The cost is the shape of its answer: a `data:image/jpeg;base64,…` URI built
/// for an `<img src>` in a webview, so it is unwrapped back into bytes here.
/// That is one base64 round trip per track change, on a worker thread, over a
/// payload `read_cover` has already thumbnailed to ~320px. Measured against
/// growing a second tag reader in this crate, it is the cheaper mistake.
///
/// # One bound this cannot apply, stated rather than implied
///
/// `read_cover` decodes with a bare `image::load_from_memory`, under no
/// [`Limits`] at all — so a *local* file with a hostile picture in it is decoded
/// unbounded before any of this crate's ceilings are reached. The cap below and
/// [`decode_within_limits`] both run on what comes back, which is a 320-pixel
/// JPEG this project produced.
///
/// It is left alone on purpose. The fix is in `eko-core`, whose `metadata`
/// module the desktop app shares, and this task's blast radius is
/// `crates/eko-cli`. The exposure is also different in kind from the remote
/// path: these bytes are a file on the user's own disk that they pointed the
/// player at, not something a server chose to send. Worth fixing at the source;
/// not worth a second, divergent tag reader here.
fn local_bytes(path: &str) -> Option<Vec<u8>> {
    let uri = eko_core::metadata::read_cover(path.to_string()).ok()??;
    let b64 = uri.split_once(";base64,")?.1;
    // Bounded like every other input. `read_cover` is ours, but the *file* it
    // read is not, and a 200 MB embedded picture is a legal FLAC.
    if b64.len() > MAX_FETCH_BYTES / 3 * 4 + 4 {
        return None;
    }
    B64.decode(b64).ok()
}

/// One `getCoverArt` response body, capped at [`MAX_FETCH_BYTES`].
///
/// A client is built per request rather than shared. This runs at most once per
/// track change, and the connection pool a shared one would keep is worth less
/// than the alternative: `eko-net` growing a public raw-bytes `GET` — a change
/// to a crate the desktop app also depends on, for one caller in this one.
fn remote_bytes(url: &str) -> Option<Vec<u8>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .ok()?;
    let response = client.get(url).send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    // `Content-Length` is a claim, so it is used only to refuse early — never to
    // size a buffer. The `take` below is what actually bounds the read.
    if response
        .content_length()
        .is_some_and(|n| n > MAX_FETCH_BYTES as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    // One byte past the limit, so "at the limit" and "over it" are
    // distinguishable and the over case is refused rather than truncated into a
    // corrupt image.
    response
        .take(MAX_FETCH_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .ok()?;
    if body.len() > MAX_FETCH_BYTES {
        return None;
    }
    Some(body)
}

/// Decode under explicit limits, or `None`.
///
/// The format is **guessed from the content**, never from a file extension or a
/// `Content-Type` header: both are attacker-controlled on the remote path. Only
/// the four codecs `eko-core` enabled are compiled in, so anything else fails
/// here rather than reaching a decoder.
fn decode_within_limits(bytes: &[u8]) -> Option<image::DynamicImage> {
    let mut reader = ImageReader::new(io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    // `Limits` is `#[non_exhaustive]`, so it is built from the default and
    // narrowed. That is the right way round anyway: a future field arrives with
    // whatever `image` considers safe rather than silently unset.
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DECODE_EDGE);
    limits.max_image_height = Some(MAX_DECODE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    reader.decode().ok()
}

/// Fold an RGB image `cols × (rows * 2)` into `cols × rows` halfblock cells.
///
/// The colours stay as [`Rgb`] rather than resolved `ratatui` colours: the
/// footer runs them through [`Rgb::resolve`] at paint time, which is the same
/// quantiser the rest of the palette uses and the reason a 256-colour terminal
/// gets the nearest cube-or-grey slot instead of a truecolor escape it would
/// print literally.
fn halfblock(image: &image::RgbImage, cols: u16, rows: u16) -> Cells {
    let (w, h) = (image.width(), image.height());
    let mut pixels = Vec::with_capacity(usize::from(cols) * usize::from(rows));
    for row in 0..u32::from(rows) {
        for col in 0..u32::from(cols) {
            // Clamped rather than trusted: `resize_to_fill` gives back exactly
            // the requested size, but an off-by-one here would be a panic in a
            // render path rather than a wrong pixel.
            let sample = |y: u32| -> Rgb {
                let px = image.get_pixel(col.min(w - 1), y.min(h - 1));
                Rgb(px[0], px[1], px[2])
            };
            pixels.push((sample(row * 2), sample(row * 2 + 1)));
        }
    }
    Cells { cols, rows, pixels }
}

/// The kitty transmit-and-display sequence for a raw RGBA buffer.
///
/// `q=2` is not optional. Without it the terminal answers every chunk with its
/// own APC response, and this process is in raw mode with a reader blocked
/// inside `crossterm::event::read` — those answers would arrive as input events.
/// `C=1` asks kitty not to move the cursor; [`Painter`] saves and restores it
/// anyway, because "asks" is the strongest word available for anything here.
fn kitty_payload(rgba: &[u8], px_w: u32, px_h: u32, cols: u16, rows: u16) -> String {
    use fmt::Write as _;

    let b64 = B64.encode(rgba);
    // Base64 is ASCII, so a byte chunk is always a whole `str`.
    let chunks: Vec<&str> = b64
        .as_bytes()
        .chunks(KITTY_CHUNK)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    let last = chunks.len().saturating_sub(1);

    let mut out = String::with_capacity(b64.len() + chunks.len() * 16 + 64);
    for (i, chunk) in chunks.iter().enumerate() {
        out.push_str("\x1b_G");
        if i == 0 {
            // `f=32` is 32-bit RGBA; `s`/`v` are the *pixel* extent of that
            // buffer, `c`/`r` the *cell* extent it must be scaled into. `c`/`r`
            // are what keep the image inside the block the footer reserved.
            let _ = write!(
                out,
                "a=T,f=32,s={px_w},v={px_h},i={KITTY_IMAGE_ID},c={cols},r={rows},C=1,q=2,"
            );
        }
        let more = u8::from(i < last);
        let _ = write!(out, "m={more};{chunk}\x1b\\");
    }
    out
}

/// The iTerm2 inline-image sequence for a PNG.
///
/// `width`/`height` are cell counts (iTerm2 reads a bare integer as cells) and
/// `preserveAspectRatio=1` letterboxes rather than stretches — the image was
/// already cropped to the block's aspect, so in practice it fits exactly.
/// `doNotMoveCursor=1` is a request, not a guarantee; [`Painter`] restores the
/// cursor regardless.
fn iterm2_payload(png: &[u8], cols: u16, rows: u16) -> String {
    format!(
        "\x1b]1337;File=inline=1;size={};width={cols};height={rows};\
         preserveAspectRatio=1;doNotMoveCursor=1:{}\x07",
        png.len(),
        B64.encode(png)
    )
}

// ── The fold's state ─────────────────────────────────────────────────────────

/// Where a cover request has got to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum State {
    /// Nothing is playing, or nothing has been asked for yet.
    #[default]
    Idle,
    /// A worker is fetching. The footer shows the placeholder meanwhile — there
    /// is no spinner, because a cover that takes a moment is not an event.
    Loading,
    Ready(Rendered),
    /// Asked, and there is no picture: no art on the track, a 404, a decode
    /// failure, a server that answered something that was not an image. All four
    /// look the same on a four-row footer, and all four are the placeholder.
    Unavailable,
}

/// The cover-art half of [`crate::app::App`]: what is wanted, what has arrived,
/// and the counter that drops stale answers.
///
/// # The cache, in three lines
///
/// [`Pane::wants`] holds the [`Key`] the fold last asked for. Every fold
/// recomputes the key it *should* want; if the two are equal — which is the case
/// on all but a handful of frames in a session — nothing happens at all. When
/// they differ the generation is bumped, the state drops back to
/// [`State::Loading`], and one worker is started. An answer stamped with an
/// older generation, or carrying a key that is no longer wanted, is discarded by
/// [`Pane::accept`] rather than drawn.
///
/// That is what keeps a four-thousand-track library from refetching: the key
/// changes when the track changes or the pane is resized, and at no other time.
#[derive(Debug, Default)]
pub struct Pane {
    /// Which renderer this terminal gets. Set once, from `main`, so that
    /// `App::new` stays free of process-global environment reads and every test
    /// states the protocol rather than inheriting the machine's.
    pub protocol: Protocol,
    want: Option<Key>,
    state: State,
    generation: u64,
}

impl Pane {
    /// The key currently being asked for, if any.
    #[must_use]
    pub fn wants(&self) -> Option<&Key> {
        self.want.as_ref()
    }

    /// The generation answers must carry to be believed. Also the
    /// [`Placement::id`], so a redraw is forced when the cover changes to one
    /// that happens to encode identically.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The pixels, if any have arrived for what is currently wanted.
    #[must_use]
    pub fn rendered(&self) -> Option<&Rendered> {
        match &self.state {
            State::Ready(rendered) => Some(rendered),
            _ => None,
        }
    }

    /// The out-of-band sequence, if this terminal uses one and it is ready.
    #[must_use]
    pub fn escape(&self) -> Option<&Escape> {
        match self.rendered() {
            Some(Rendered::Escape(esc)) => Some(esc),
            _ => None,
        }
    }

    /// Ask for `want`, abandoning whatever was in flight.
    ///
    /// Returns the generation the new request must be stamped with. The caller
    /// starts the worker: only it knows whether the bytes are a file or a signed
    /// URL, and only it holds the channel to answer on.
    pub fn begin(&mut self, want: Option<Key>) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.state = if want.is_some() {
            State::Loading
        } else {
            State::Idle
        };
        self.want = want;
        self.generation
    }

    /// Give up on the current request without one having been made — no client
    /// to sign a URL with, for instance.
    pub fn give_up(&mut self) {
        if self.want.is_some() {
            self.state = State::Unavailable;
        }
    }

    /// Fold one worker's answer in. `false` when it was stale and dropped.
    pub fn accept(&mut self, event: Event) -> bool {
        if event.generation != self.generation || self.want.as_ref() != Some(&event.key) {
            return false;
        }
        self.state = match event.outcome {
            Outcome::Ready(rendered) => State::Ready(rendered),
            Outcome::Unavailable => State::Unavailable,
        };
        true
    }
}

// ── The out-of-band writer ───────────────────────────────────────────────────

/// Where an [`Escape`] goes, and which cover it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement<'a> {
    /// The art generation this was rendered for. Two different covers can encode
    /// to the same bytes — a compilation whose tracks share one picture — and
    /// this is what keeps "the same image again" from being mistaken for
    /// "nothing changed" when it does in fact have to be redrawn.
    pub id: u64,
    /// Zero-based column of the block's top-left cell.
    pub x: u16,
    /// Zero-based row of the block's top-left cell.
    pub y: u16,
    pub art: &'a Escape,
}

/// What is on the screen outside ratatui's model, and the one thing that changes
/// it.
///
/// # Why this exists at all
///
/// kitty and iTerm2 sequences are written straight to fd 1. ratatui does not
/// know those cells are occupied: its buffer says they hold spaces, so its diff
/// emits nothing for them, so it will neither repaint over the image (good — the
/// image survives) nor erase it when it should be gone (bad, and the entire
/// reason for this type). Something has to remember, and it cannot be the
/// buffer.
///
/// # The sequencing, and why it is safe
///
/// A frame is **three** steps, exactly one call site, in `main::run`, and always
/// in this order:
///
/// 1. [`Painter::take_down`] — takes the old image off the screen, *before* the
///    frame.
/// 2. `terminal.draw(…)` — ratatui writes the whole frame, including the art
///    block, which [`crate::ui::footer`] fills with **blank cells** whenever an
///    out-of-band protocol is in use. Nothing else in the application writes to
///    fd 1.
/// 3. [`Painter::place`] — puts the new image up, *after* the frame, so the
///    footer's blanking cannot erase it in the same breath.
///
/// # Why the removal is in step 1 and not step 3
///
/// iTerm2 has no removal sequence: an inline image is painted *into cells*, and
/// is erased the way any other cell content is — by overwriting them. Those
/// cells are ratatui's. Overwriting them after `draw` has flushed puts the
/// screen out of step with ratatui's previous buffer, which never learns they
/// changed, so its next diff emits nothing for them and the damage is permanent:
/// the `░` placeholder drawn in the very frame the cover came down is erased and
/// never redrawn, and a resize leaves a blank block at the image's *old* row.
///
/// Erasing *before* the frame is the repair, and it needs no help from ratatui:
/// while an out-of-band image is up, the footer's model of those cells is
/// **blank** (see [`crate::ui::footer`]'s `render_art`), so blanking them makes
/// the screen agree with the buffer ratatui is about to diff against. Whatever
/// the new frame wants there — the placeholder, a border, another pane — is a
/// difference from that buffer, and ratatui emits it in the ordinary way.
///
/// The alternative was to tell ratatui those cells are dirty. It has no API for
/// that below the whole viewport: `Terminal::clear` resets the back buffer but
/// first *queries the cursor*, a blocking read of a DSR reply that would race
/// this application's dedicated input thread; `Terminal::resize` would do it
/// without the query but clears and repaints the entire screen for a 6×4 block.
/// Ordering costs nothing and repairs both symptoms.
///
/// Five properties follow, and each is pinned by a test:
///
/// * **The art block is the footer's own.** Its rect comes from
///   [`crate::ui::art_area`], the same geometry the footer's text column is laid
///   out around, so it ends two columns before the text and cannot reach the
///   seal. The seal's columns are in no rect this type ever writes to.
/// * **A stale image is deleted, never left.** `take_down` with a different
///   placement — or with `None` — emits the protocol's removal. For kitty that
///   is a targeted `a=d,d=I,i=…`; for iTerm2 it is the block overwritten with
///   spaces, exactly as wide as the block.
/// * **Nothing is written over the frame that repairs the block.** `place`
///   writes the new image and nothing else — never a blank, never at the old
///   position — so the cells ratatui just repainted stay repainted.
/// * **The cursor is put back.** Every write is bracketed by `ESC 7` / `ESC 8`
///   (DECSC/DECRC), so wherever ratatui left the cursor is where it is
///   afterwards, whatever the terminal did with `C=1` or `doNotMoveCursor`.
/// * **Idempotence.** A frame whose placement is already on screen writes *zero*
///   bytes, in both steps. At thirty frames a second beside an audio thread, a
///   renderer that re-transmitted a cover every frame would be worse than no
///   cover at all.
///
/// [`Painter::invalidate`] is the escape hatch for the one thing this cannot
/// observe: a resize, after which ratatui repaints everything and the terminal
/// may or may not have kept the image. It forces the next frame to remove and
/// redraw, which is correct whichever happened.
#[derive(Debug, Default)]
pub struct Painter {
    on_screen: Option<OnScreen>,
    /// Set by [`Painter::invalidate`]; cleared by the next [`Painter::take_down`].
    stale: bool,
    /// Set by [`Painter::take_down`] when the screen and the want disagree;
    /// cleared by the [`Painter::place`] that completes the same frame. It is
    /// what carries "there is an image to put up" across `terminal.draw`, and
    /// what makes an unchanged frame write nothing in *either* step.
    pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OnScreen {
    id: u64,
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
    protocol: Protocol,
}

impl OnScreen {
    /// What a [`Placement`] looks like once it is on the screen. The one place
    /// the two are compared, so the frame's two steps cannot disagree about
    /// whether anything changed.
    fn of(p: Placement<'_>) -> Self {
        Self {
            id: p.id,
            x: p.x,
            y: p.y,
            cols: p.art.cols,
            rows: p.art.rows,
            protocol: p.art.protocol,
        }
    }
}

impl Painter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare that the terminal may have repainted underneath us — a resize, or
    /// anything else that makes ratatui redraw the whole frame.
    ///
    /// Does not write. The next frame removes and redraws.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// **Step one of a frame, before `terminal.draw`.** Take the image off the
    /// screen when it is not the one `want` asks for.
    ///
    /// Writes nothing — and leaves the following [`Painter::place`] with nothing
    /// to write either — when the screen already shows `want`.
    ///
    /// `want` must be the same value the [`Painter::place`] that completes this
    /// frame is given. [`Placement`] is `Copy` so the call site can read it once
    /// and hand the same one to both.
    ///
    /// # Errors
    /// Propagates any write or flush failure from `out`.
    pub fn take_down<W: Write>(
        &mut self,
        out: &mut W,
        want: Option<Placement<'_>>,
    ) -> io::Result<()> {
        self.pending = self.stale || self.on_screen != want.map(OnScreen::of);
        if !self.pending {
            return Ok(());
        }
        self.stale = false;
        let Some(old) = self.on_screen.take() else {
            return Ok(());
        };

        let mut bytes = String::new();
        // DECSC. Everything between here and the DECRC below may move the
        // cursor; nothing after it may notice.
        bytes.push_str("\x1b7");
        remove(&mut bytes, old);
        // DECRC.
        bytes.push_str("\x1b8");

        out.write_all(bytes.as_bytes())?;
        out.flush()
    }

    /// **Step three of a frame, after `terminal.draw`.** Put `want` on the
    /// screen, if the [`Painter::take_down`] that opened this frame found
    /// anything to do.
    ///
    /// The only thing this writes is the image itself: never a blank, and never
    /// at a position the previous image occupied. Everything ratatui drew in
    /// step two survives it.
    ///
    /// # Errors
    /// Propagates any write or flush failure from `out`.
    pub fn place<W: Write>(&mut self, out: &mut W, want: Option<Placement<'_>>) -> io::Result<()> {
        if !self.pending {
            return Ok(());
        }
        self.pending = false;
        let Some(p) = want else {
            return Ok(());
        };

        let mut bytes = String::new();
        bytes.push_str("\x1b7");
        move_to(&mut bytes, p.x, p.y);
        bytes.push_str(p.art.payload());
        bytes.push_str("\x1b8");

        out.write_all(bytes.as_bytes())?;
        out.flush()?;
        self.on_screen = Some(OnScreen::of(p));
        Ok(())
    }

    /// Take everything down. For the exit path, before the alternate screen is
    /// given back.
    ///
    /// Both steps at once, with no frame between them: there is no frame left to
    /// draw, and nothing to put up.
    ///
    /// # Errors
    /// Propagates any write or flush failure from `out`.
    pub fn clear<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        self.stale = false;
        self.take_down(out, None)?;
        self.place(out, None)
    }

    /// What the painter believes is on screen. A test seam, and only that — the
    /// application never asks, because asking would be a second source of truth
    /// about a screen only this type writes to.
    #[cfg(test)]
    #[must_use]
    pub fn placed(&self) -> Option<(u64, u16, u16)> {
        self.on_screen.map(|p| (p.id, p.x, p.y))
    }
}

/// `CUP` to a zero-based cell. The wire protocol is one-based.
fn move_to(out: &mut String, x: u16, y: u16) {
    use fmt::Write as _;
    let _ = write!(out, "\x1b[{};{}H", y + 1, x + 1);
}

/// Take one placement off the screen.
fn remove(out: &mut String, old: OnScreen) {
    use fmt::Write as _;
    match old.protocol {
        // Halfblock is never out of band, so it is never in `on_screen`.
        Protocol::Halfblock => {}
        // Targeted: delete the image with *this* id and free its data. `d=A`
        // would take down anything else sharing the terminal.
        Protocol::Kitty => {
            let _ = write!(out, "\x1b_Ga=d,d=I,i={KITTY_IMAGE_ID},q=2\x1b\\");
        }
        // iTerm2 has no removal sequence: an inline image is painted into the
        // cells, and is erased the way any other cell content is.
        Protocol::Iterm2 => {
            let blank = " ".repeat(usize::from(old.cols));
            for row in 0..old.rows {
                move_to(out, old.x, old.y + row);
                out.push_str(&blank);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── detection ────────────────────────────────────────────────────────

    fn env(pairs: &[(&str, &str)]) -> Env {
        let mut e = Env::default();
        for (k, v) in pairs {
            let slot = match *k {
                "TERM" => &mut e.term,
                "TERM_PROGRAM" => &mut e.term_program,
                "KITTY_WINDOW_ID" => &mut e.kitty_window_id,
                "LC_TERMINAL" => &mut e.lc_terminal,
                "TMUX" => &mut e.tmux,
                other => panic!("unknown variable {other}"),
            };
            *slot = Some((*v).to_string());
        }
        e
    }

    #[test]
    fn an_empty_environment_gets_the_universal_fallback() {
        assert_eq!(protocol_from_env(&Env::default()), Protocol::Halfblock);
    }

    #[test]
    fn kitty_ghostty_and_wezterm_get_the_kitty_protocol() {
        for pairs in [
            vec![("TERM", "xterm-kitty")],
            vec![("KITTY_WINDOW_ID", "1")],
            vec![("TERM", "xterm-ghostty")],
            vec![("TERM", "xterm-256color"), ("TERM_PROGRAM", "ghostty")],
            vec![("TERM", "xterm-256color"), ("TERM_PROGRAM", "WezTerm")],
        ] {
            assert_eq!(
                protocol_from_env(&env(&pairs)),
                Protocol::Kitty,
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn iterm2_gets_inline_images_including_over_ssh() {
        assert_eq!(
            protocol_from_env(&env(&[("TERM_PROGRAM", "iTerm.app")])),
            Protocol::Iterm2
        );
        // `LC_TERMINAL` is the one iTerm2 forwards; `TERM_PROGRAM` is not.
        assert_eq!(
            protocol_from_env(&env(&[
                ("TERM", "xterm-256color"),
                ("LC_TERMINAL", "iTerm2")
            ])),
            Protocol::Iterm2
        );
    }

    /// **The rule that matters most.** A tmux session started from kitty
    /// inherits `TERM=xterm-kitty` and `KITTY_WINDOW_ID`, and tmux does not
    /// forward the graphics payload. Every positive rule must lose to this.
    #[test]
    fn a_multiplexer_beats_every_positive_rule() {
        for pairs in [
            vec![
                ("TMUX", "/tmp/tmux-501/default,1,0"),
                ("TERM", "xterm-kitty"),
            ],
            vec![("TMUX", "/tmp/x"), ("KITTY_WINDOW_ID", "1")],
            vec![("TERM", "tmux-256color"), ("KITTY_WINDOW_ID", "1")],
            vec![
                ("TERM", "screen.xterm-256color"),
                ("TERM_PROGRAM", "ghostty"),
            ],
            vec![("TMUX", "/tmp/x"), ("TERM_PROGRAM", "iTerm.app")],
        ] {
            assert_eq!(
                protocol_from_env(&env(&pairs)),
                Protocol::Halfblock,
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn an_unrecognised_terminal_is_never_promoted() {
        for pairs in [
            vec![("TERM", "xterm-256color")],
            vec![("TERM", "linux")],
            vec![("TERM_PROGRAM", "Apple_Terminal")],
            vec![("TERM_PROGRAM", "vscode")],
            vec![("LC_TERMINAL", "something-else")],
            vec![("TERM", "dumb")],
        ] {
            assert_eq!(
                protocol_from_env(&env(&pairs)),
                Protocol::Halfblock,
                "{pairs:?}"
            );
        }
    }

    // ── decode bounds ────────────────────────────────────────────────────

    fn key(protocol: Protocol) -> Key {
        Key {
            token: "test".into(),
            cols: 6,
            rows: 4,
            protocol,
        }
    }

    /// A real image, generated rather than checked in: 64×64, left half red and
    /// right half blue, so a renderer that mirrored or transposed the grid would
    /// show up in the assertions below.
    fn test_png() -> Vec<u8> {
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
            .write_to(&mut io::Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn garbage_bytes_are_unavailable_rather_than_a_panic() {
        for bytes in [
            b"".to_vec(),
            b"not an image at all".to_vec(),
            // A PNG magic number and nothing behind it.
            b"\x89PNG\r\n\x1a\n".to_vec(),
            // A JPEG SOI and truncated garbage.
            b"\xff\xd8\xff\xe0\x00\x10JFIF\x00".to_vec(),
            vec![0xff; 4096],
        ] {
            let n = bytes.len();
            assert_eq!(
                encode(&bytes, &key(Protocol::Halfblock)),
                Outcome::Unavailable,
                "{n} bytes"
            );
        }
    }

    /// A PNG header that *declares* a canvas far past [`MAX_DECODE_EDGE`] must
    /// be refused from the header, before anything is allocated: 33 bytes of
    /// file claiming a 100,000 × 100,000 image, which is 30 GB decoded.
    #[test]
    fn an_enormous_declared_canvas_is_refused_from_the_header() {
        let mut png: Vec<u8> = b"\x89PNG\r\n\x1a\n".to_vec();
        // IHDR: width 100000, height 100000, 8-bit truecolour.
        let mut ihdr: Vec<u8> = b"IHDR".to_vec();
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(&ihdr);
        png.extend_from_slice(&crc32(&ihdr).to_be_bytes());
        assert_eq!(
            encode(&png, &key(Protocol::Halfblock)),
            Outcome::Unavailable
        );
    }

    /// Bitwise CRC-32, so the fixture above is a real PNG chunk rather than one
    /// the decoder rejects for the wrong reason.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for b in bytes {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn a_zero_sized_grid_is_unavailable_rather_than_a_panic() {
        for (cols, rows) in [(0u16, 4u16), (6, 0), (0, 0)] {
            let k = Key {
                cols,
                rows,
                ..key(Protocol::Halfblock)
            };
            assert_eq!(
                encode(&test_png(), &k),
                Outcome::Unavailable,
                "{cols}×{rows}"
            );
        }
    }

    // ── halfblock ────────────────────────────────────────────────────────

    #[test]
    fn halfblock_fills_exactly_the_grid_it_was_asked_for() {
        let Outcome::Ready(Rendered::Cells(cells)) = encode(&test_png(), &key(Protocol::Halfblock))
        else {
            panic!("no cells");
        };
        assert_eq!((cells.cols, cells.rows), (6, 4));
        assert_eq!(cells.pixels.len(), 24);
        assert!(cells.at(5, 3).is_some());
        assert!(cells.at(6, 3).is_none(), "read past the last column");
        assert!(cells.at(5, 4).is_none(), "read past the last row");
    }

    /// The left half of the fixture is red and the right half blue. A renderer
    /// that mirrored, transposed, or sampled one pixel for the whole grid would
    /// fail this.
    #[test]
    fn halfblock_keeps_the_picture_the_right_way_round() {
        let Outcome::Ready(Rendered::Cells(cells)) = encode(&test_png(), &key(Protocol::Halfblock))
        else {
            panic!("no cells");
        };
        let (top_left, _) = cells.at(0, 0).unwrap();
        let (top_right, _) = cells.at(5, 0).unwrap();
        assert!(top_left.0 > top_left.2, "left should be red: {top_left:?}");
        assert!(
            top_right.2 > top_right.0,
            "right should be blue: {top_right:?}"
        );
    }

    // ── the escape payloads ──────────────────────────────────────────────

    #[test]
    fn the_kitty_payload_is_well_formed_and_bounded_to_its_cells() {
        let Outcome::Ready(Rendered::Escape(esc)) = encode(&test_png(), &key(Protocol::Kitty))
        else {
            panic!("no escape");
        };
        let p = esc.payload();
        assert!(p.starts_with("\x1b_G"), "APC introducer");
        assert!(p.ends_with("\x1b\\"), "string terminator");
        // The cell extent is what keeps the image out of the seal's columns.
        assert!(p.contains(",c=6,r=4,"), "no cell extent");
        // Six columns of 10px and four rows of 20px.
        assert!(p.contains("s=60,v=80,"), "no pixel extent");
        assert!(p.contains("f=32,"), "32-bit RGBA");
        assert!(
            p.contains("q=2"),
            "responses must be suppressed in raw mode"
        );
        assert!(p.contains("C=1"), "the cursor must not be moved");
        // Every chunk but the last says more-is-coming, and the last says stop.
        let chunks = p.matches("\x1b_G").count();
        assert!(chunks > 1, "the fixture should need several chunks");
        assert_eq!(p.matches("m=1;").count(), chunks - 1);
        assert_eq!(p.matches("m=0;").count(), 1);
    }

    #[test]
    fn the_iterm2_payload_is_well_formed_and_bounded_to_its_cells() {
        let Outcome::Ready(Rendered::Escape(esc)) = encode(&test_png(), &key(Protocol::Iterm2))
        else {
            panic!("no escape");
        };
        let p = esc.payload();
        assert!(p.starts_with("\x1b]1337;File=inline=1;"));
        assert!(p.ends_with('\x07'), "BEL terminator");
        assert!(p.contains(";width=6;height=4;"), "no cell extent");
        assert!(p.contains("preserveAspectRatio=1"));
        // The declared size must be the real decoded length, or iTerm2 waits for
        // bytes that never come and eats whatever is typed next.
        let declared: usize = p
            .split(";size=")
            .nth(1)
            .and_then(|s| s.split(';').next())
            .and_then(|s| s.parse().ok())
            .expect("a size");
        let payload = p.split_once(':').unwrap().1.trim_end_matches('\x07');
        assert_eq!(B64.decode(payload).unwrap().len(), declared);
    }

    /// The payload is re-encoded by this process, so the server's bytes are not
    /// what reaches the terminal. A JPEG in must not be a JPEG out.
    #[test]
    fn the_iterm2_payload_is_our_png_not_the_servers_bytes() {
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            32,
            32,
            image::Rgb([10, 200, 90]),
        ))
        .write_to(&mut io::Cursor::new(&mut jpeg), ImageFormat::Jpeg)
        .unwrap();
        let Outcome::Ready(Rendered::Escape(esc)) = encode(&jpeg, &key(Protocol::Iterm2)) else {
            panic!("no escape");
        };
        let payload = esc
            .payload()
            .split_once(':')
            .unwrap()
            .1
            .trim_end_matches('\x07');
        let out = B64.decode(payload).unwrap();
        assert_eq!(&out[..8], b"\x89PNG\r\n\x1a\n", "not re-encoded as PNG");
    }

    /// Neither `Debug` may print a payload or a signed URL.
    #[test]
    fn debug_prints_no_payload_and_no_credential() {
        let Outcome::Ready(Rendered::Escape(esc)) = encode(&test_png(), &key(Protocol::Kitty))
        else {
            panic!("no escape");
        };
        let shown = format!("{esc:?}");
        assert!(shown.contains("bytes"), "{shown}");
        assert!(!shown.contains('\x1b'), "the payload leaked into Debug");

        let shown = format!(
            "{:?}",
            Fetch::Remote("https://x/rest?u=rod&t=deadbeef&s=abc".into())
        );
        assert!(!shown.contains("deadbeef"), "{shown}");
        assert!(!shown.contains("rod"), "{shown}");
    }

    // ── the painter ──────────────────────────────────────────────────────
    fn escape_of(protocol: Protocol) -> Escape {
        let Outcome::Ready(Rendered::Escape(esc)) = encode(&test_png(), &key(protocol)) else {
            panic!("no escape");
        };
        esc
    }

    /// One whole frame's worth of painter output, in order: the take-down, then
    /// the frame ratatui would draw in between — which writes nothing *here*,
    /// and is the whole point of the two steps — then the place.
    ///
    /// Concatenated, because the tests that use this care only that the bytes
    /// came out in this order. The ones that care *where the frame goes* keep
    /// their own two buffers.
    fn frame(painter: &mut Painter, want: Option<Placement<'_>>) -> String {
        let mut out: Vec<u8> = Vec::new();
        painter.take_down(&mut out, want).unwrap();
        painter.place(&mut out, want).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_painter_positions_the_block_and_puts_the_cursor_back() {
        let art = escape_of(Protocol::Kitty);
        let mut painter = Painter::new();
        let s = frame(
            &mut painter,
            Some(Placement {
                id: 1,
                x: 2,
                y: 25,
                art: &art,
            }),
        );
        assert!(s.starts_with("\x1b7"), "DECSC first");
        assert!(s.ends_with("\x1b8"), "DECRC last");
        // CUP is one-based: cell (2, 25) is row 26, column 3.
        assert!(s.contains("\x1b[26;3H"), "wrong position");
        assert_eq!(painter.placed(), Some((1, 2, 25)));
    }

    /// **Idempotence.** The loop calls this on every redraw; at thirty frames a
    /// second a painter that re-transmitted would spend more bandwidth on the
    /// cover than on the spectrum. Both steps have to stay silent: a take-down
    /// that wrote blanks over an unchanged cover would erase it.
    #[test]
    fn the_painter_writes_nothing_when_nothing_changed() {
        for protocol in [Protocol::Kitty, Protocol::Iterm2] {
            let art = escape_of(protocol);
            let place = Placement {
                id: 1,
                x: 2,
                y: 25,
                art: &art,
            };
            let mut painter = Painter::new();
            assert!(!frame(&mut painter, Some(place)).is_empty());
            for _ in 0..100 {
                let mut before: Vec<u8> = Vec::new();
                painter.take_down(&mut before, Some(place)).unwrap();
                assert!(before.is_empty(), "{protocol:?}: erased an unchanged cover");
                let mut after: Vec<u8> = Vec::new();
                painter.place(&mut after, Some(place)).unwrap();
                assert!(after.is_empty(), "{protocol:?}: re-transmitted it");
            }
        }
    }

    /// The delete does not merely precede the transmit — it precedes the whole
    /// frame, and the transmit follows it.
    #[test]
    fn a_new_cover_deletes_the_old_kitty_image_before_drawing() {
        let art = escape_of(Protocol::Kitty);
        let mut painter = Painter::new();
        frame(
            &mut painter,
            Some(Placement {
                id: 1,
                x: 2,
                y: 25,
                art: &art,
            }),
        );

        let next = Placement {
            id: 2,
            x: 2,
            y: 25,
            art: &art,
        };
        let mut before: Vec<u8> = Vec::new();
        painter.take_down(&mut before, Some(next)).unwrap();
        let before = String::from_utf8(before).unwrap();
        assert!(before.contains("a=d,d=I"), "no delete before the frame");
        assert!(!before.contains("a=T,"), "transmitted before the frame");

        let mut after: Vec<u8> = Vec::new();
        painter.place(&mut after, Some(next)).unwrap();
        let after = String::from_utf8(after).unwrap();
        assert!(after.contains("a=T,"), "no transmit after the frame");
        assert!(!after.contains("a=d,d=I"), "deleted after the frame");
    }

    #[test]
    fn taking_the_cover_down_deletes_it_and_writes_nothing_more() {
        let art = escape_of(Protocol::Kitty);
        let mut painter = Painter::new();
        frame(
            &mut painter,
            Some(Placement {
                id: 1,
                x: 2,
                y: 25,
                art: &art,
            }),
        );

        let s = frame(&mut painter, None);
        assert!(s.contains("a=d,d=I"), "no delete");
        assert!(!s.contains("a=T,"), "redrew while taking it down");
        assert_eq!(painter.placed(), None);

        assert!(
            frame(&mut painter, None).is_empty(),
            "kept deleting nothing"
        );
    }

    /// **The repaired property, and the reason for the two steps.**
    ///
    /// iTerm2 has no removal sequence: the image is painted into cells, and the
    /// only way it comes off is those cells being overwritten. They are
    /// ratatui's cells, and ratatui's previous buffer — which says *blank*,
    /// because that is what the footer draws in the block while an out-of-band
    /// image is up — never learns that anything happened to them. So the erase
    /// has to land **before** the frame, where it puts the screen back in
    /// agreement with that buffer and lets the frame's own diff draw whatever
    /// belongs there now; and **nothing** may be written after the frame except
    /// a new image.
    ///
    /// The old test asserted the opposite — that the blanking is what a
    /// take-down emits *after* `terminal.draw` — and pinned two real bugs as
    /// correct: the `░` placeholder drawn in the very frame the cover came down
    /// was erased and, the next diff being clean, stayed erased for the rest of
    /// the session; and a height-changing resize left a blank 6×4 block at the
    /// image's old row.
    ///
    /// The width assertion survives from it: a run one cell too wide would reach
    /// towards the text column, and the seal is in that direction.
    #[test]
    fn an_iterm2_cover_comes_down_before_the_frame_that_repaints_its_cells() {
        let art = escape_of(Protocol::Iterm2);
        let mut painter = Painter::new();
        frame(
            &mut painter,
            Some(Placement {
                id: 1,
                x: 2,
                y: 25,
                art: &art,
            }),
        );

        // Step one, before `terminal.draw`: the block, and exactly the block.
        let mut before: Vec<u8> = Vec::new();
        painter.take_down(&mut before, None).unwrap();
        let before = String::from_utf8(before).unwrap();
        for row in 0..4u16 {
            let cup = format!("\x1b[{};3H", 26 + row);
            let at = before
                .find(&cup)
                .unwrap_or_else(|| panic!("no CUP for row {row}"));
            let after = &before[at + cup.len()..];
            let run: String = after.chars().take_while(|c| *c == ' ').collect();
            assert_eq!(run.len(), 6, "row {row} blanked {} cells", run.len());
        }
        assert_eq!(painter.placed(), None);

        // ratatui draws here, and the `░` placeholder goes into the cells just
        // erased. Step two must not touch them.
        let mut after: Vec<u8> = Vec::new();
        painter.place(&mut after, None).unwrap();
        assert!(
            after.is_empty(),
            "wrote over the frame that repaints the block: {:?}",
            String::from_utf8_lossy(&after)
        );
    }

    /// The other half of the same bug: a height-changing resize moves the block,
    /// and the old rows are repainted by the frame — the sidebar or the main
    /// pane after a grow, the border after a shrink. Nothing may be written at
    /// the *old* position after that frame, or the repair is rubbed out and
    /// never re-emitted.
    #[test]
    fn a_moved_iterm2_cover_is_erased_at_the_old_row_before_the_frame_only() {
        let art = escape_of(Protocol::Iterm2);
        let mut painter = Painter::new();
        frame(
            &mut painter,
            Some(Placement {
                id: 1,
                x: 2,
                y: 25,
                art: &art,
            }),
        );

        // The same cover, four rows higher: the terminal grew.
        let moved = Placement {
            id: 1,
            x: 2,
            y: 21,
            art: &art,
        };
        painter.invalidate();
        let mut before: Vec<u8> = Vec::new();
        painter.take_down(&mut before, Some(moved)).unwrap();
        let before = String::from_utf8(before).unwrap();
        assert!(before.contains("\x1b[26;3H"), "the old row was not erased");

        let mut after: Vec<u8> = Vec::new();
        painter.place(&mut after, Some(moved)).unwrap();
        let after = String::from_utf8(after).unwrap();
        assert!(
            !after.contains("\x1b[26;3H"),
            "touched the old row after the frame that repainted it"
        );
        assert!(after.contains("\x1b[22;3H"), "not drawn at the new row");
        assert_eq!(painter.placed(), Some((1, 2, 21)));
    }

    /// A resize makes ratatui repaint everything, and this cannot see that. The
    /// invalidation is what forces a redraw of an image whose placement is
    /// otherwise unchanged.
    #[test]
    fn invalidating_forces_a_redraw_of_an_identical_placement() {
        let art = escape_of(Protocol::Kitty);
        let place = Placement {
            id: 1,
            x: 2,
            y: 25,
            art: &art,
        };
        let mut painter = Painter::new();
        frame(&mut painter, Some(place));
        assert!(frame(&mut painter, Some(place)).is_empty());

        painter.invalidate();
        let s = frame(&mut painter, Some(place));
        assert!(s.contains("a=d,d=I"), "no delete after invalidation");
        assert!(s.contains("a=T,"), "no redraw after invalidation");
    }

    /// Every byte the painter can emit is bracketed by the cursor save and
    /// restore, whatever the transition — and each step is bracketed on its own,
    /// because ratatui draws between them and moves the cursor as it likes.
    #[test]
    fn every_write_is_bracketed_by_the_cursor_save_and_restore() {
        let kitty = escape_of(Protocol::Kitty);
        let iterm = escape_of(Protocol::Iterm2);
        let mut painter = Painter::new();
        for want in [
            Some(Placement {
                id: 1,
                x: 2,
                y: 25,
                art: &kitty,
            }),
            None,
            Some(Placement {
                id: 2,
                x: 2,
                y: 15,
                art: &iterm,
            }),
            Some(Placement {
                id: 3,
                x: 4,
                y: 15,
                art: &iterm,
            }),
            None,
        ] {
            let mut down: Vec<u8> = Vec::new();
            painter.take_down(&mut down, want).unwrap();
            let mut up: Vec<u8> = Vec::new();
            painter.place(&mut up, want).unwrap();
            for step in [down, up] {
                if step.is_empty() {
                    continue;
                }
                let s = String::from_utf8(step).unwrap();
                assert!(
                    s.starts_with("\x1b7") && s.ends_with("\x1b8"),
                    "unbracketed write"
                );
            }
        }
    }

    // ── local art ────────────────────────────────────────────────────────

    #[test]
    fn a_file_that_is_not_there_is_unavailable() {
        assert_eq!(
            load(
                &Fetch::Local("/eko-cli-test/nothing-here.flac".into()),
                &key(Protocol::Halfblock),
            ),
            Outcome::Unavailable
        );
    }
}
