//! Terminal lifecycle.
//!
//! Raw mode plus the alternate screen, restored on **every** exit path: a clean
//! quit, `Ctrl-C`, an I/O error, or a panic. A panic that leaves raw mode on
//! leaves the user's shell with no echo and no line editing — they have to type
//! `reset` blind to get it back — so the panic hook restores first and re-panics
//! second.
//!
//! # The alternate screen does not protect against stderr
//!
//! Everything here takes over **fd 1 only**. `enable_raw_mode`, `EnterAlternateScreen`
//! and the ratatui backend all go through `io::stdout()`; fd 2 is never redirected,
//! duplicated, or silenced. On a normal terminal both descriptors point at the same tty,
//! so *any* write to stderr — from this crate, a dependency, the panic hook, or a future
//! `eprintln!` — lands on the alternate screen mid-frame and paints over the UI. Nothing
//! repairs it until the next full redraw.
//!
//! This is a class of bug, not one bug. Removing a single stray `eprintln!` (as was done
//! for the signed-URL leak at `eko-core`'s `Source::Url` fetch site) closes that instance
//! and leaves the class open. A real fix redirects fd 2 for the lifetime of the takeover
//! — to a log file, or to `/dev/null` with anything worth seeing routed through the TUI
//! instead — and restores it alongside raw mode on every exit path above.

use std::io::{self, Stdout, Write};
use std::panic;
use std::sync::Once;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// The terminal this application draws to.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Take over the terminal.
///
/// Installs the panic hook *first*, so a panic between here and the first draw
/// is still caught.
///
/// # Errors
/// Propagates any failure to enter raw mode or the alternate screen. On failure
/// the terminal is restored before returning, so a partial takeover never
/// escapes.
pub fn init() -> io::Result<Tui> {
    install_panic_hook();
    enable_raw_mode()?;
    // Bracketed paste, for the search input. Without it a pasted query arrives
    // as a burst of synthetic keystrokes — indistinguishable from typing, so a
    // pasted newline would *submit* mid-paste and the rest would land as
    // commands. With it the whole thing arrives as one `Event::Paste` that the
    // line editor can sanitise in one go. It is one escape sequence in and one
    // out, which is as cheap as this gets.
    if let Err(e) = execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        Hide
    ) {
        let _ = restore();
        return Err(e);
    }
    match Terminal::new(CrosstermBackend::new(io::stdout())) {
        Ok(mut terminal) => {
            terminal.clear()?;
            Ok(terminal)
        }
        Err(e) => {
            let _ = restore();
            Err(e)
        }
    }
}

/// Give the terminal back.
///
/// Every step is attempted even if an earlier one failed — a half-restored
/// terminal is the failure mode this whole module exists to prevent — and the
/// first error is returned afterwards.
///
/// Idempotent: calling it twice, or on a terminal that was never taken over, is
/// harmless. Both the normal exit path and the panic hook call it.
///
/// # Errors
/// Returns the first error encountered while restoring.
pub fn restore() -> io::Result<()> {
    let mut first_error = None;
    let mut record = |r: io::Result<()>| {
        if let Err(e) = r {
            first_error.get_or_insert(e);
        }
    };

    record(disable_raw_mode());
    record(execute!(
        io::stdout(),
        DisableBracketedPaste,
        LeaveAlternateScreen,
        Show
    ));
    record(io::stdout().flush());

    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Restore the terminal on panic, then let the original hook print the message.
///
/// Ordering matters twice over: the restore has to happen *before* the default
/// hook writes, or the backtrace is painted into the alternate screen and
/// vanishes with it; and the original hook has to still run, or a panic becomes
/// a silent exit.
///
/// Safe to call more than once — only the first call installs.
pub fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = restore();
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_is_safe_on_a_terminal_that_was_never_taken_over() {
        // Under `cargo test` stdout is a pipe, so this exercises the
        // never-initialised path. It must not panic and must not hang.
        let _ = restore();
        let _ = restore();
    }

    #[test]
    fn installing_the_panic_hook_twice_is_harmless() {
        install_panic_hook();
        install_panic_hook();
    }
}
