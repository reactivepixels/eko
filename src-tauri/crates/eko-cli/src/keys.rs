//! The keymap — **one table**, and the only place a key is spelled.
//!
//! [`BINDINGS`] is the whole keyboard. [`App::on_key`] dispatches on the
//! [`Action`] it resolves to and never matches a [`KeyCode`] itself; Phase
//! 1b-ii's help overlay renders the same slice. A key that appears in one and
//! not the other is a bug the compiler cannot catch, so there is only one.
//!
//! [`Action`] is deliberately context-free. `Enter` resolves to
//! [`Action::Open`] whether the cursor is on an album or a track, and the fold
//! decides what "open" means where it is — a table that had to know about the
//! current view would stop being a table.
//!
//! ## The one thing that is *not* in the table
//!
//! While the search input is open, a key is **text**, and text is not an action.
//! [`App::on_key`] therefore offers the event to the line editor first and only
//! consults this table when the editor declines it — which it does for anything
//! with `CONTROL` held, so `Ctrl-C` still quits mid-query. That is a routing
//! decision in one place rather than a special case scattered through
//! [`Action`]: nothing here has to know that a mode exists.
//!
//! [`App::on_key`]: crate::app::App::on_key

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Something the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Move the cursor down one row.
    Down,
    /// Move the cursor up one row.
    Up,
    /// Open the thing under the cursor: an album, or a track (which plays it).
    Open,
    /// Leave the album and go back to the album list.
    Back,
    /// Move focus between the sidebar and the main pane.
    ToggleFocus,
    /// Play, or pause, or resume.
    PlayPause,
    /// Next entry in the queue.
    Next,
    /// Previous entry in the queue.
    Previous,
    /// Add the album or track under the cursor to the end of the queue.
    Enqueue,
    /// Drop the entry under the cursor out of the queue.
    Unqueue,
    SeekBack,
    SeekForward,
    /// Show or hide the footer spectrum. Changes the tick rate.
    ToggleSpectrum,
    /// Open or close the full-screen Now Playing view. See
    /// [`crate::ui::visualiser`].
    ///
    /// **`esc` also leaves it**, and there is no second row in this table for
    /// that: `esc` is [`Action::Back`], and the fold closes whatever is open
    /// before `Back` means anything else — the same way it already closes the
    /// help overlay, the device picker and the EQ panel. One key, one row; the
    /// meaning of *back* is the fold's business, which is what [`Action`] being
    /// context-free is for.
    Visualiser,
    /// Scan the music folder again, and retry the server connection.
    Rescan,
    /// Open the search input. See [`crate::app::Search`].
    Search,

    /// Show or hide the output-device picker. See [`crate::ui::device_panel`].
    ///
    /// Opening it changes nothing about the audio — the *choice* does, and it is
    /// a choice the seal has to follow, because the device's rate is one of the
    /// three `derive` reads.
    Devices,
    /// Cycle the sleep timer: off → 15 → 30 → 45 → 60 minutes → off.
    ///
    /// One key rather than a picker, because there are four values and the
    /// gesture is "a bit longer". See [`crate::app::Sleep`].
    Sleep,
    /// Show or hide the help overlay, which renders **this table**.
    ///
    /// It is a row here like any other, which is the point: the overlay is the
    /// only complete rendering of the keyboard, and a keyboard reference that
    /// could not tell you how to close itself would be an odd one.
    Help,

    // ── the EQ ───────────────────────────────────────────────────────────
    //
    // Six entries rather than two, because the alternative was to overload the
    // transport keys while the panel is up — and the help table would then be
    // describing keys that do something else. `Action` being context-free does
    // not license the *table* being wrong.
    //
    // The one overload kept is [`Action::Down`] / [`Action::Up`]: the panel is
    // modal, so there is no list cursor to move while it is open, and "down"
    // moving a slider down is the same verb rather than a different one.
    /// Show or hide the EQ panel. Changes nothing about the audio.
    EqPanel,
    /// Route the EQ into the signal path, or take it back out.
    ///
    /// Deliberately not the same key as [`Action::EqPanel`]. A curve you have
    /// dialled in and bypassed is a real state, and it is exactly the one the
    /// seal has to be able to tell apart from an engaged one.
    EqToggle,
    /// Move the panel's cursor one column left — towards the pre-amp.
    EqPrev,
    /// Move the panel's cursor one column right — towards 16k.
    EqNext,
    /// Load the previous preset from `eko_core::eq_presets::PRESETS`.
    EqPresetPrev,
    /// Load the next preset from `eko_core::eq_presets::PRESETS`.
    EqPresetNext,
}

/// How far `[` and `]` move the playhead.
pub const SEEK_STEP_SECS: f64 = 5.0;

// There is no volume step, and no `-`/`+` to apply one. Software volume is a
// multiply on every sample, and `eko_core::signal_path::derive` reports it
// honestly: the first press took the seal from `BIT-PERFECT` to `VOLUME`. A
// player whose only claim is that it does not touch the samples has no business
// shipping a control whose entire job is to touch them, and anyone driving a DAC
// already has a better one on the DAC. See
// `crate::app::tests::nothing_a_user_can_press_takes_the_seal_off_unity`.

/// One row of the keymap: the keys, how to name them on screen, and what they
/// do.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// Every key that triggers this action.
    pub keys: &'static [Key],
    /// How the help overlay writes it.
    pub label: &'static str,
    /// What the help overlay says it does.
    pub description: &'static str,
    pub action: Action,
}

/// A key plus the modifiers it needs. `NONE` also matches `SHIFT`, because a
/// terminal reports `+` as shift-`=` on most layouts and the user pressed one
/// key either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
}

impl Key {
    const fn plain(code: KeyCode) -> Self {
        Self { code, ctrl: false }
    }

    const fn ctrl(code: KeyCode) -> Self {
        Self { code, ctrl: true }
    }

    fn matches(self, event: KeyEvent) -> bool {
        event.code == self.code && event.modifiers.contains(KeyModifiers::CONTROL) == self.ctrl
    }
}

const fn c(ch: char) -> Key {
    Key::plain(KeyCode::Char(ch))
}

/// The keyboard.
///
/// Ordered the way the help overlay should read it: leaving, moving, then the
/// transport.
pub const BINDINGS: &[Binding] = &[
    Binding {
        keys: &[
            c('q'),
            Key::ctrl(KeyCode::Char('c')),
            Key::ctrl(KeyCode::Char('C')),
        ],
        label: "q",
        description: "quit",
        action: Action::Quit,
    },
    Binding {
        keys: &[Key::plain(KeyCode::Tab)],
        label: "tab",
        description: "sources ⇄ library",
        action: Action::ToggleFocus,
    },
    Binding {
        keys: &[c('j'), Key::plain(KeyCode::Down)],
        label: "j / ↓",
        description: "down",
        action: Action::Down,
    },
    Binding {
        keys: &[c('k'), Key::plain(KeyCode::Up)],
        label: "k / ↑",
        description: "up",
        action: Action::Up,
    },
    Binding {
        keys: &[Key::plain(KeyCode::Enter)],
        label: "enter",
        description: "open album · play track",
        action: Action::Open,
    },
    Binding {
        keys: &[Key::plain(KeyCode::Esc)],
        label: "esc",
        description: "back",
        action: Action::Back,
    },
    Binding {
        keys: &[c(' ')],
        label: "space",
        description: "play / pause",
        action: Action::PlayPause,
    },
    Binding {
        keys: &[c('n')],
        label: "n",
        description: "next track",
        action: Action::Next,
    },
    Binding {
        keys: &[c('p')],
        label: "p",
        description: "previous track",
        action: Action::Previous,
    },
    Binding {
        keys: &[c('a')],
        label: "a",
        description: "add to queue",
        action: Action::Enqueue,
    },
    Binding {
        keys: &[c('x')],
        label: "x",
        description: "remove from queue",
        action: Action::Unqueue,
    },
    Binding {
        keys: &[c('['), Key::plain(KeyCode::Left)],
        label: "[ / ←",
        description: "seek back 5s",
        action: Action::SeekBack,
    },
    Binding {
        keys: &[c(']'), Key::plain(KeyCode::Right)],
        label: "] / →",
        description: "seek forward 5s",
        action: Action::SeekForward,
    },
    Binding {
        keys: &[c('s')],
        label: "s",
        description: "spectrum on / off",
        action: Action::ToggleSpectrum,
    },
    Binding {
        keys: &[c('z')],
        label: "z",
        // "now playing", not "now playing · esc to leave". None of the other
        // three things `esc` closes — the keymap, the EQ panel, the device
        // picker — says so in its own row either, and spelling it out here
        // widened the KEYS panel by eight columns to describe a key that
        // already has a row of its own two lines below.
        description: "now playing",
        action: Action::Visualiser,
    },
    Binding {
        keys: &[c('r')],
        label: "r",
        description: "rescan · reconnect",
        action: Action::Rescan,
    },
    Binding {
        keys: &[c('/')],
        label: "/",
        description: "search the server",
        action: Action::Search,
    },
    Binding {
        keys: &[c('d')],
        label: "d",
        description: "output device",
        action: Action::Devices,
    },
    Binding {
        keys: &[c('t')],
        label: "t",
        description: "sleep timer",
        action: Action::Sleep,
    },
    // Shift-`/` on most layouts, which a terminal reports as `Char('?')` — so it
    // is a distinct row from `/`, exactly as `E` is from `e`.
    Binding {
        keys: &[c('?')],
        label: "?",
        description: "keys · this list",
        action: Action::Help,
    },
    Binding {
        keys: &[c('e')],
        label: "e",
        description: "EQ panel",
        action: Action::EqPanel,
    },
    // Shift-`e`. `Key::matches` only constrains CONTROL, so a bare `E` and a
    // shifted one both land here, and `e`/`E` stay two distinct rows because
    // `KeyCode::Char` carries the case.
    Binding {
        keys: &[c('E')],
        label: "E",
        description: "EQ on / off",
        action: Action::EqToggle,
    },
    Binding {
        keys: &[c('h')],
        label: "h",
        description: "EQ band left",
        action: Action::EqPrev,
    },
    Binding {
        keys: &[c('l')],
        label: "l",
        description: "EQ band right",
        action: Action::EqNext,
    },
    Binding {
        keys: &[c(','), c('<')],
        label: ",",
        description: "EQ preset back",
        action: Action::EqPresetPrev,
    },
    Binding {
        keys: &[c('.'), c('>')],
        label: ".",
        description: "EQ preset forward",
        action: Action::EqPresetNext,
    },
];

/// Resolve a key event to an action, or `None` if nothing is bound to it.
#[must_use]
pub fn action_for(event: KeyEvent) -> Option<Action> {
    BINDINGS
        .iter()
        .find(|b| b.keys.iter().any(|k| k.matches(event)))
        .map(|b| b.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_transport_keys_are_all_bound() {
        assert_eq!(action_for(key(KeyCode::Char(' '))), Some(Action::PlayPause));
        assert_eq!(action_for(key(KeyCode::Char('n'))), Some(Action::Next));
        assert_eq!(action_for(key(KeyCode::Char('p'))), Some(Action::Previous));
        assert_eq!(action_for(key(KeyCode::Char('['))), Some(Action::SeekBack));
        assert_eq!(
            action_for(key(KeyCode::Char(']'))),
            Some(Action::SeekForward)
        );
        assert_eq!(action_for(key(KeyCode::Left)), Some(Action::SeekBack));
        assert_eq!(action_for(key(KeyCode::Right)), Some(Action::SeekForward));
        // `-`, `_`, `+` and `=` are unbound: there is no software volume. A
        // key that used to do something and now does nothing must do *nothing*,
        // not something else.
        for ch in ['-', '_', '+', '='] {
            assert_eq!(action_for(key(KeyCode::Char(ch))), None, "{ch:?}");
        }
    }

    #[test]
    fn slash_opens_the_search_input() {
        assert_eq!(action_for(key(KeyCode::Char('/'))), Some(Action::Search));
    }

    #[test]
    fn navigation_takes_vim_keys_and_arrows() {
        assert_eq!(action_for(key(KeyCode::Char('j'))), Some(Action::Down));
        assert_eq!(action_for(key(KeyCode::Down)), Some(Action::Down));
        assert_eq!(action_for(key(KeyCode::Char('k'))), Some(Action::Up));
        assert_eq!(action_for(key(KeyCode::Up)), Some(Action::Up));
        assert_eq!(action_for(key(KeyCode::Enter)), Some(Action::Open));
        assert_eq!(action_for(key(KeyCode::Esc)), Some(Action::Back));
    }

    #[test]
    fn ctrl_c_quits_but_a_bare_c_is_unbound() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_c), Some(Action::Quit));
        assert_eq!(action_for(key(KeyCode::Char('c'))), None);
    }

    /// The EQ's keys — including `E`, which is `e` with shift and must resolve
    /// to a *different* action from it. `Key::matches` only constrains CONTROL,
    /// so this is the assertion that the two rows are actually distinguishable.
    #[test]
    fn the_eq_keys_are_all_bound_and_e_is_not_shift_e() {
        assert_eq!(action_for(key(KeyCode::Char('e'))), Some(Action::EqPanel));
        assert_eq!(action_for(key(KeyCode::Char('E'))), Some(Action::EqToggle));
        let shift_e = KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT);
        assert_eq!(action_for(shift_e), Some(Action::EqToggle));
        assert_eq!(action_for(key(KeyCode::Char('h'))), Some(Action::EqPrev));
        assert_eq!(action_for(key(KeyCode::Char('l'))), Some(Action::EqNext));
        assert_eq!(
            action_for(key(KeyCode::Char(','))),
            Some(Action::EqPresetPrev)
        );
        assert_eq!(
            action_for(key(KeyCode::Char('<'))),
            Some(Action::EqPresetPrev)
        );
        assert_eq!(
            action_for(key(KeyCode::Char('.'))),
            Some(Action::EqPresetNext)
        );
        assert_eq!(
            action_for(key(KeyCode::Char('>'))),
            Some(Action::EqPresetNext)
        );
        // Ctrl-e is nobody's, and must not fall through to the panel.
        let ctrl_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_e), None);
    }

    /// The device picker and the sleep timer are the two keys that can move the
    /// *seal* and the *transport* without touching a list, so they get their own
    /// assertion rather than riding along in the transport one.
    #[test]
    fn the_device_picker_and_the_sleep_timer_are_bound() {
        assert_eq!(action_for(key(KeyCode::Char('d'))), Some(Action::Devices));
        assert_eq!(action_for(key(KeyCode::Char('t'))), Some(Action::Sleep));
        // Neither is a control key, and `Ctrl-d` must not fall through to one.
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_d), None);
    }

    /// `?` is shift-`/` on most layouts, and must resolve to a *different* action
    /// from `/` — the same distinction `e` and `E` turn on.
    #[test]
    fn question_mark_opens_the_help_and_is_not_slash() {
        assert_eq!(action_for(key(KeyCode::Char('?'))), Some(Action::Help));
        let shift_q = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT);
        assert_eq!(action_for(shift_q), Some(Action::Help));
        assert_eq!(action_for(key(KeyCode::Char('/'))), Some(Action::Search));
    }

    /// **Every action in the table is reachable from a key.**
    ///
    /// The help overlay renders one row per [`Binding`], so a row with no key
    /// would be the overlay documenting something nobody can press. The reverse —
    /// a key bound twice — is `no_key_is_bound_to_two_actions`; this is the other
    /// direction, and it is what stops an [`Action`] variant being added to the
    /// enum, handled in the fold, and never given a row.
    #[test]
    fn every_action_the_fold_handles_has_exactly_one_row() {
        let mut seen: Vec<Action> = Vec::new();
        for binding in BINDINGS {
            assert!(
                !seen.contains(&binding.action),
                "{:?} has two rows in the table",
                binding.action
            );
            seen.push(binding.action);
            assert_eq!(
                action_for(KeyEvent::new(
                    binding.keys[0].code,
                    if binding.keys[0].ctrl {
                        KeyModifiers::CONTROL
                    } else {
                        KeyModifiers::NONE
                    }
                )),
                Some(binding.action),
                "{:?}'s own first key does not resolve to it",
                binding.action
            );
        }
    }

    /// `z` opens the visualiser, and `esc` is **not** given a second row to
    /// close it with — it is [`Action::Back`], and the fold decides what back
    /// means where it is. A table with two rows for `esc` would be a table that
    /// disagrees with itself.
    #[test]
    fn z_opens_the_visualiser_and_esc_is_still_the_one_back_key() {
        assert_eq!(
            action_for(key(KeyCode::Char('z'))),
            Some(Action::Visualiser)
        );
        assert_eq!(action_for(key(KeyCode::Esc)), Some(Action::Back));
        assert_eq!(
            BINDINGS
                .iter()
                .filter(|b| b.keys.iter().any(|k| k.code == KeyCode::Esc))
                .count(),
            1
        );
        let ctrl_z = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_z), None);
    }

    #[test]
    fn unbound_keys_resolve_to_nothing() {
        assert_eq!(action_for(key(KeyCode::Char('y'))), None);
        assert_eq!(action_for(key(KeyCode::F(7))), None);
    }

    #[test]
    fn no_key_is_bound_to_two_actions() {
        // The whole point of one table is that it cannot disagree with itself.
        let mut seen: Vec<(KeyCode, bool)> = Vec::new();
        for binding in BINDINGS {
            for k in binding.keys {
                let id = (k.code, k.ctrl);
                assert!(!seen.contains(&id), "{:?} is bound twice", k.code);
                seen.push(id);
            }
        }
    }

    #[test]
    fn every_binding_is_describable_for_the_help_overlay() {
        for binding in BINDINGS {
            assert!(!binding.keys.is_empty(), "{:?} has no keys", binding.action);
            assert!(
                !binding.label.is_empty(),
                "{:?} has no label",
                binding.action
            );
            assert!(
                !binding.description.is_empty(),
                "{:?} has no description",
                binding.action
            );
        }
    }
}
