//! The Deck's one line editor.
//!
//! Two things in this application take typing: the search query in the main
//! pane's summary row, and the three fields of the add-server panel. They are
//! the *same* editor, and this module is why — the alternative was a second
//! `match key.code` with its own idea of what backspace does, its own length
//! cap, and its own answer to whether a pasted escape sequence is text.
//!
//! ## Everything typed goes through the same gate the server's text does
//!
//! That is not belt-and-braces. A paste arrives as one `Event::Paste` carrying
//! whatever was on the clipboard, and every one of these fields is **echoed** —
//! the query in the summary row, the server fields in the panel. A control
//! character in an echoed field is a control character written to the terminal.
//! [`crate::server::is_unsafe_in_a_row`] is the predicate
//! [`crate::server::tidy`] enforces, shared rather than copied, so there is one
//! charset to audit and not two that can drift. What is *not* shared is `tidy`'s
//! whitespace squeezing — see [`type_into`] for the bug that came of it.
//!
//! ## What is deliberately *not* here
//!
//! No password. The password prompt is [`crate::server::apply_key`], it lives
//! next to the keychain call that consumes it, and it is a different editor on
//! purpose: it must not echo, must not cap, and must not tidy — a passphrase is
//! not display text and every one of this module's kindnesses would be a
//! mutation of somebody's secret.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Span;

/// How many terminal **columns** `text` occupies.
///
/// # Why every length in this crate's text handling is this and not `chars()`
///
/// A character is not a column. `音` is one `char`, two columns; a combining
/// mark is one `char` and none. Everything downstream of a field — the panel
/// that sizes itself around it, [`crate::ui::footer::fit`] that trims it, the
/// `ratatui` `Line` that lays it out — measures columns, because that is what a
/// terminal has. A cap counted in characters therefore lets a field hold **more
/// than twice** what the layout was told to expect.
///
/// That is not a cosmetic mismatch. The add-server panel used to size itself
/// from `Line::width` (columns) while capping and windowing in characters: a
/// name of 38 CJK characters made a 76-column value in a 38-column field, the
/// panel asked for more room than the pane had, and the `if` that guarded
/// against that returned without drawing — so the panel *vanished*, while
/// `App::add` stayed `Some` and went on swallowing every keystroke. One paste
/// and the Deck looked frozen.
///
/// It is `ratatui`'s own measure rather than a second call into
/// `unicode-width`, so a field cannot be capped by one table and laid out by
/// another.
#[must_use]
pub fn columns(text: &str) -> usize {
    Span::raw(text).width()
}

/// The columns one character occupies. See [`columns`].
#[must_use]
fn char_columns(c: char) -> usize {
    let mut buf = [0u8; 4];
    columns(c.encode_utf8(&mut buf))
}

/// What a keystroke did to the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStep {
    /// The buffer changed, or a key was swallowed. Keep editing.
    Continue,
    /// Enter.
    Submit,
    /// Esc.
    Cancel,
    /// Not text and not a verb this editor owns — the caller decides.
    Unhandled,
}

/// Fold one key into `buffer`, capped at `max` **columns** — see [`columns`].
///
/// Pure, so the whole editor is a table of cases rather than something to be
/// found by typing into a terminal. Modifiers are the caller's business: the
/// search input declines anything with `CONTROL` held so `Ctrl-C` still quits,
/// and the add-server panel does the same.
pub fn edit(buffer: &mut String, key: KeyEvent, max: usize) -> LineStep {
    match key.code {
        KeyCode::Char(c) => {
            type_into(buffer, &c.to_string(), max);
            LineStep::Continue
        }
        // Character-wise, not byte-wise: a server name can be typed in any
        // script even though only a subset of them will validate.
        KeyCode::Backspace => {
            buffer.pop();
            LineStep::Continue
        }
        KeyCode::Enter => LineStep::Submit,
        KeyCode::Esc => LineStep::Cancel,
        _ => LineStep::Unhandled,
    }
}

/// Append `text`, sanitised and capped at `max` **columns**. The paste path,
/// and the single-character path, are the same path.
///
/// Truncated rather than refused when it would overflow `max`: a paste that
/// silently did nothing reads as a broken terminal.
///
/// # The cap is in columns, and it is the same measure the layout uses
///
/// A wide character that would *straddle* the cap is left out rather than
/// allowed to push one column past it, so `columns(buffer) <= max` holds for
/// every input — which is the invariant the add-server panel's width is
/// computed from. See [`columns`] for the failure this replaced.
///
/// # Why this is [`crate::server::is_unsafe_in_a_row`] and not [`crate::server::tidy`]
///
/// `tidy` is two rules in one function: neutralise what must never reach a
/// terminal, and squeeze whitespace so a remote message reads as one row. A
/// field wants the first and must not have the second — this used to call `tidy`
/// on each keystroke, and `tidy(" ")` is the empty string, so **a space could
/// never be typed at all**. The search input has had that bug since it was
/// written: two-word queries lost their space and went to the server as one
/// word. The gate is shared; the whitespace policy is not, because a sentence
/// and a text field want different ones.
pub fn type_into(buffer: &mut String, text: &str, max: usize) {
    let mut used = columns(buffer);
    for c in text.chars() {
        // The one gate, same as every server-written string passes. Applied
        // *before* the width is taken, because it is the replacement that ends
        // up in the buffer and a control character is not one column.
        let c = if crate::server::is_unsafe_in_a_row(c) {
            ' '
        } else {
            c
        };
        let width = char_columns(c);
        if used + width > max {
            break;
        }
        buffer.push(c);
        used += width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_accumulates_and_enter_submits_and_esc_cancels() {
        let mut buffer = String::new();
        for c in "home".chars() {
            assert_eq!(
                edit(&mut buffer, press(KeyCode::Char(c)), 32),
                LineStep::Continue
            );
        }
        assert_eq!(buffer, "home");
        assert_eq!(
            edit(&mut buffer, press(KeyCode::Enter), 32),
            LineStep::Submit
        );
        assert_eq!(edit(&mut buffer, press(KeyCode::Esc), 32), LineStep::Cancel);
    }

    #[test]
    fn backspace_removes_a_character_not_a_byte_and_is_inert_when_empty() {
        let mut buffer = "pä".to_string();
        edit(&mut buffer, press(KeyCode::Backspace), 32);
        assert_eq!(buffer, "p");
        for _ in 0..3 {
            assert_eq!(
                edit(&mut buffer, press(KeyCode::Backspace), 32),
                LineStep::Continue
            );
        }
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_key_that_is_not_text_is_the_callers_problem() {
        let mut buffer = String::new();
        for code in [KeyCode::Left, KeyCode::F(5), KeyCode::Tab] {
            assert_eq!(edit(&mut buffer, press(code), 32), LineStep::Unhandled);
        }
        assert!(buffer.is_empty());
    }

    /// **The gate.** These fields are echoed, so a pasted escape sequence would
    /// be written to the terminal.
    #[test]
    fn a_pasted_control_sequence_is_tidied_rather_than_typed() {
        let mut buffer = String::new();
        type_into(&mut buffer, "https://h\u{1b}[2Jex\nample", 96);
        assert!(!buffer.contains(['\u{1b}', '\n']), "{buffer:?}");
        assert_eq!(buffer, "https://h [2Jex ample");
        // The bidi overrides `tidy` removes are removed here too — the gate is
        // the same predicate, not a second copy of the charset.
        let mut bidi = String::new();
        type_into(&mut bidi, "Zaireeka\u{202e}drawkcab", 96);
        assert_eq!(bidi, "Zaireeka drawkcab");
    }

    /// **A space is a character.** `tidy` squeezes leading spaces, which is
    /// right for a footer sentence and wrong for a field: this editor used to
    /// run each keystroke through `tidy`, and `tidy(" ")` is the empty string —
    /// so the search input could not take a two-word query and never could.
    #[test]
    fn a_space_can_be_typed_because_a_field_is_not_a_sentence() {
        let mut buffer = String::new();
        for c in "two words".chars() {
            edit(&mut buffer, press(KeyCode::Char(c)), 32);
        }
        assert_eq!(buffer, "two words");
        // Including a leading one, and a trailing one — both are things a person
        // typed, and neither is this function's to undo.
        let mut edges = String::new();
        type_into(&mut edges, " a ", 32);
        assert_eq!(edges, " a ");
    }

    #[test]
    fn the_cap_truncates_rather_than_refusing() {
        let mut buffer = String::new();
        type_into(&mut buffer, &"ä".repeat(20), 8);
        assert_eq!(buffer.chars().count(), 8);
        type_into(&mut buffer, "more", 8);
        assert_eq!(buffer.chars().count(), 8, "the cap stopped holding");
    }

    /// **The cap is columns, not characters** — the whole of fix 1.
    ///
    /// Every consumer of these fields measures columns: the add-server panel
    /// sizes itself from `Line::width`, `fit` trims to a column budget, and a
    /// terminal cell is a column. A character-counted cap let 38 CJK characters
    /// become 76 columns in a 38-column field, which made the panel ask for more
    /// room than the pane had — and the guard against *that* returned without
    /// drawing anything at all.
    #[test]
    fn the_cap_is_columns_so_wide_characters_cannot_overrun_the_layout() {
        // Four ideographs are eight columns. A char-counted cap of 8 would take
        // all eight of them — sixteen columns.
        let mut wide = String::new();
        type_into(&mut wide, &"音".repeat(20), 8);
        assert_eq!(columns(&wide), 8, "{wide:?}");
        assert_eq!(wide.chars().count(), 4);

        // A wide character that would straddle the cap is left out rather than
        // allowed one column past it, so the invariant is `<=` and not `<= max + 1`.
        let mut odd = String::new();
        type_into(&mut odd, "x音音音", 4);
        assert_eq!(
            odd, "x音",
            "a wide character was allowed to straddle the cap"
        );
        assert!(columns(&odd) <= 4);

        // And the cap holds across a paste that arrives one keystroke at a time,
        // which is the path a person types on.
        let mut typed = String::new();
        for c in "音楽再生音楽再生".chars() {
            edit(&mut typed, press(KeyCode::Char(c)), 6);
        }
        assert_eq!(columns(&typed), 6, "{typed:?}");
    }
}
