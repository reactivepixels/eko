//! `eko-cli` — EKO's terminal client.
//!
//! A bit-perfect music player in 80 columns. The audio engine lives in
//! `eko-core` and the OpenSubsonic client in `eko-net`; this binary is the
//! Deck — the terminal frontend that drives them.
//!
//! This crate is FREE. It has no `pro` feature and no `src/pro/` module; the
//! terminal client ships whole in the public MIT repo.

#![forbid(unsafe_code)]

mod app;
mod art;
mod config;
mod eq;
mod keys;
mod library;
mod line;
mod lyrics;
mod queue;
mod remote;
mod server;
mod tui;
mod ui;
mod wave;

use std::io;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;

use app::{App, AppEvent, Prompt, PromptOutcome};
use crossterm::event::Event;
use tui::Tui;
use ui::theme::{Accent, Theme};

/// What `eko-cli --help` prints, and the whole command surface.
///
/// **The binary is `eko-cli`, and every line here has to say so.** This crate is
/// named `eko-cli` and has no `[[bin]]` section renaming it, so `cargo install
/// --path` puts `eko-cli` on the PATH — and it cannot become `eko`, because
/// `crates/eko-tauri` already has an `eko` bin target in this workspace and two
/// of them do not build. So the usage text is the side that moves. It said `eko`
/// once, which made every command in it wrong for the binary printing it, and
/// `eko login` — the first thing a new user is told to run, here and in the
/// sidebar and in the docs — fail before it started.
const USAGE: &str = "\
eko-cli — a bit-perfect music player in 80 columns

  eko-cli                    open the Deck
  eko-cli login [server]     store a server password in the OS keychain
  eko-cli logout [server]    forget it again
  eko-cli --help             this

`server` is the `name` of a `[[servers]]` entry in ~/.config/eko/config.toml.
Omit it when there is only one.

`login` asks for the password on the terminal and never echoes it. It is not a
flag and takes no value, so it cannot end up in your shell history; piping works
too, for a password manager:

  pass show music/navidrome | eko-cli login home";

fn main() -> ExitCode {
    // Subcommands are handled **before** the terminal takeover, on purpose: a
    // password prompt inside raw mode, over a half-drawn Deck, is a much harder
    // thing to get right than a prompt on a plain terminal — and this one has to
    // be got right.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("login") => return login(args.get(1).map(String::as_str)),
        Some("logout") => return logout(args.get(1).map(String::as_str)),
        Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("eko-cli: unknown command {other:?}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    }

    deck()
}

/// Look up the server a subcommand was aimed at.
///
/// With no name and exactly one server there is no ambiguity, so naming it is
/// not required. With several, it is — guessing which of someone's servers a
/// password belongs to is not a guess worth making.
fn pick_server(
    config: &config::Config,
    name: Option<&str>,
) -> Result<server::ServerConfig, String> {
    if config.servers.is_empty() {
        // The Deck can do this now, so the Deck is offered first: hand-writing
        // TOML is the fallback rather than the instruction. `login` itself is
        // still worth having — it is the one that takes a pipe from a password
        // manager, which a panel cannot.
        return Err(format!(
            "no servers configured. Run `eko-cli` and press enter on `+ Add server` \
             in the sources column — or put this in {}:\n\n\
             [[servers]]\n\
             name = \"home\"\n\
             base_url = \"https://music.example.com\"\n\
             username = \"you\"",
            config::CONFIG_PATH_HINT
        ));
    }
    match name {
        Some(name) => config
            .servers
            .iter()
            .find(|s| s.name == name)
            .cloned()
            .ok_or_else(|| {
                let known: Vec<&str> = config.servers.iter().map(|s| s.name.as_str()).collect();
                format!("no server named {name:?}. Configured: {}", known.join(", "))
            }),
        None if config.servers.len() == 1 => Ok(config.servers[0].clone()),
        None => {
            let known: Vec<&str> = config.servers.iter().map(|s| s.name.as_str()).collect();
            Err(format!(
                "several servers are configured — name one: {}",
                known.join(", ")
            ))
        }
    }
}

/// `eko-cli login [server]` — read a password without echoing it, and store it.
///
/// The password reaches this process through a **prompt or a pipe**, never
/// through `argv`: an argument would be in the shell's history file before the
/// program started, and no amount of care afterwards can take it out again.
fn login(name: Option<&str>) -> ExitCode {
    let (config, note) = config::load();
    if let Some(note) = note {
        eprintln!("eko-cli: {note}");
    }
    let server = match pick_server(&config, name) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("eko-cli: {e}");
            return ExitCode::FAILURE;
        }
    };

    let prompt = format!("Password for {}@{}: ", server.username, server.name);
    let password = match server::read_password(&prompt) {
        Ok(Some(password)) => password,
        Ok(None) => {
            eprintln!("eko-cli: cancelled — nothing was stored.");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("eko-cli: could not read the password: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = server::set_password(&server.name, &password) {
        eprintln!("eko-cli: {e}");
        return ExitCode::FAILURE;
    }
    // Names the store, so "where did that go?" has an answer that does not
    // require reading this source.
    println!(
        "eko-cli: stored the password for {:?} in the {} keychain.",
        server.name,
        server::KEYCHAIN_SERVICE
    );
    ExitCode::SUCCESS
}

/// `eko-cli logout [server]` — forget a stored password. Idempotent.
fn logout(name: Option<&str>) -> ExitCode {
    let (config, _) = config::load();
    let server = match pick_server(&config, name) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("eko-cli: {e}");
            return ExitCode::FAILURE;
        }
    };
    match server::delete_password(&server.name) {
        Ok(()) => {
            println!("eko-cli: forgot the password for {:?}.", server.name);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("eko-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Open the Deck.
fn deck() -> ExitCode {
    let (config, note) = config::load();
    let theme = Theme::new(Accent::from_name(&config.accent), ui::theme::detect_depth());
    // Resolved here, once, and never written back — a first run with no config
    // file at all still finds `~/Music`. See `config::MusicFolder`.
    let folder = config::resolve_music_folder(&config);
    let mut app = App::new(&config, theme, folder);
    // Read once, here, so the fold and every test state the protocol rather than
    // inheriting whatever terminal the process happened to start in. See
    // [`art::protocol_from_env`] for how little an environment variable proves.
    app.art.protocol = art::detect();
    if let Some(note) = note {
        app.set_status(note);
    }

    // The hook goes in before the takeover, so even a failure inside `init`
    // cannot leave the terminal half-claimed.
    tui::install_panic_hook();

    let mut terminal = match tui::init() {
        Ok(terminal) => terminal,
        Err(e) => {
            let _ = tui::restore();
            eprintln!("eko-cli: could not start the terminal UI: {e}");
            return ExitCode::FAILURE;
        }
    };

    let result = run(&mut terminal, &mut app);

    // Restore before reporting, always — including on the error path, where the
    // message has to land on a terminal that can show it.
    let restored = tui::restore();

    if let Err(e) = result {
        eprintln!("eko-cli: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = restored {
        eprintln!("eko-cli: the terminal may need `reset`: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// The event loop.
///
/// One fold on the main thread. Input arrives on its own channel so the loop
/// can wait on *either* an event or the tick, and — crucially — can wait
/// indefinitely when there is no tick to wait for. See [`app::tick_interval`].
///
/// The library scan shares that channel. It runs on a worker thread and pushes
/// its own progress in, so a scan of a very large or very slow folder never
/// blocks a redraw and never needs a timer to animate: the loop wakes exactly
/// when the scanner has something new to say.
fn run(terminal: &mut Tui, app: &mut App) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    // The suspend handshake. See [`spawn_input_reader`] and [`serve_prompt`].
    let suspended = Arc::new(AtomicBool::new(false));
    let (keys_tx, keys_rx) = mpsc::channel();
    spawn_input_reader(tx.clone(), keys_tx, Arc::clone(&suspended));
    app.attach(tx);

    // The first size comes from an `ioctl`; every later one arrives as a
    // `Resize` event. Without this the cover cache would have no grid to key on
    // until the window was first resized.
    let size = terminal.size()?;
    app.set_term_size(size.width, size.height);

    // The one thing in this application that writes to the terminal outside
    // ratatui's buffer. See [`art::Painter`] for the sequencing, and `tui.rs`
    // for the hazard it is deliberately joining.
    let mut painter = art::Painter::new();

    loop {
        if app.take_dirty() {
            // ── ORDER MATTERS ────────────────────────────────────────────
            // The out-of-band layer brackets the frame, and both halves are
            // where they are for a reason.
            //
            // Taking the old cover *down* comes first, because on iTerm2 that
            // means overwriting cells — the image is painted into them, and
            // that is the only way it comes off. Those cells are ratatui's, and
            // ratatui's previous buffer says they are blank (the footer blanks
            // the block whenever an out-of-band protocol is in use), so erasing
            // them here leaves screen and buffer in agreement, and the frame
            // below repaints whatever should be there instead. Erasing *after*
            // the frame would rub out what it had just drawn — the `░`
            // placeholder, or the border under a block that has moved — and
            // ratatui, which never learns those cells changed, would never
            // repaint them again.
            //
            // Putting the new cover *up* comes last, because ratatui owns every
            // cell including the art block: drawing the image before the frame
            // would let that same blanking erase it in the same breath.
            //
            // The placement is read once and handed to both steps, so they
            // cannot disagree about what this frame is for.
            let want = app.art_placement();
            painter.take_down(&mut io::stdout(), want)?;
            terminal.draw(|frame| ui::draw(frame, app))?;
            painter.place(&mut io::stdout(), want)?;
        }

        let event = match app.tick_interval() {
            // Playing: wake for the tick if no input beats it to it.
            Some(interval) => match rx.recv_timeout(interval) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => AppEvent::Tick,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            // Idle: block. No timer, no polling, no spinning — the process uses
            // nothing at all until the user touches a key.
            None => match rx.recv() {
                Ok(event) => event,
                Err(_) => break,
            },
        };

        // A resize makes ratatui redraw the whole frame, and the painter cannot
        // see that from its own bookkeeping: the cover's position may be
        // unchanged (the art block is anchored to the bottom-left corner, so a
        // width-only resize does not move it) while the terminal has either
        // scrolled the image away or kept it. Removing and redrawing is correct
        // whichever happened; leaving it alone is correct in neither.
        if matches!(event, AppEvent::Input(crossterm::event::Event::Resize(..))) {
            painter.invalidate();
        }

        app.handle(event);
        // Asked for by the fold, done out here: the terminal is this function's,
        // and a password prompt has to run where raw mode and a half-drawn Deck
        // are not. See [`serve_prompt`].
        if let Some(prompt) = app.take_prompt() {
            serve_prompt(terminal, app, &mut painter, prompt, &suspended, &keys_rx)?;
        }
        if app.should_quit() {
            break;
        }
    }

    // Take the cover down before the alternate screen goes. A kitty image is
    // held by the terminal, not by the screen buffer, so leaving the alternate
    // screen is not on its own enough to be sure it is gone.
    let _ = painter.clear(&mut io::stdout());
    Ok(())
}

/// Read input on a dedicated thread and forward it — to the fold normally, and
/// to the password prompt while the Deck is suspended.
///
/// Detached on purpose: it is blocked inside `event::read` when the loop exits,
/// and the only way to unblock it is a keystroke that will never come. Nothing
/// it owns needs dropping, and the send fails harmlessly once the receiver is
/// gone.
///
/// # Why the prompt does not just call `event::read` itself
///
/// **Two threads reading one stdin is a race for every keystroke.** This thread
/// spends its life parked inside `event::read`, and it is parked there at the
/// exact moment the fold decides it wants a password — so a prompt that read the
/// terminal directly would win some characters and lose others, and the ones it
/// lost would arrive at the fold as key bindings. Half a password would be typed
/// into a Deck that was quitting on the `q` in it.
///
/// So this stays the only caller of `event::read` for the life of the process,
/// and `suspended` decides where what it reads goes. The flag is checked after
/// the read rather than before, which is the only ordering that works: the
/// request to suspend is *caused* by a keystroke this thread has already
/// delivered, so at the moment it is set this thread is always blocked, and the
/// next event it sees is the first one typed at the prompt.
fn spawn_input_reader(tx: Sender<AppEvent>, keys: Sender<Event>, suspended: Arc<AtomicBool>) {
    thread::spawn(move || {
        // Stops on the first read error, or as soon as the fold has hung up.
        while let Ok(event) = crossterm::event::read() {
            if suspended.load(Ordering::Acquire) {
                if keys.send(event).is_err() {
                    break;
                }
                continue;
            }
            if tx.send(AppEvent::Input(event)).is_err() {
                break;
            }
        }
    });
}

/// Give the terminal back, ask for a password on it, and take the terminal
/// again.
///
/// # The rule this keeps rather than works around
///
/// `main` handles `login` before the takeover on purpose, and says why: *a
/// password prompt inside raw mode, over a half-drawn Deck, is a much harder
/// thing to get right than a prompt on a plain terminal.* That reasoning is
/// still right, so the prompt still happens on a plain terminal — the Deck is
/// torn down first and rebuilt afterwards. It is the same
/// [`server::prompt_password_from`], with the same raw-mode no-echo guarantee
/// and the same `RawGuard`; the only thing that changed is that the user did not
/// have to quit to reach it.
///
/// # What the password touches
///
/// The prompt's buffer, [`server::set_password`], and the keychain. It is a
/// local in this function, it is not passed to `App`, it is not returned, and
/// the value that goes back to the fold is a [`PromptOutcome`], which has
/// nowhere to hold one. It is never in a rendered frame because no frame is
/// drawn while it exists — the Deck is not on screen — and never in the config
/// file because [`server::ServerConfig`] has no field for it.
///
/// # Panics and interrupts
///
/// The panic hook is [`tui::install_panic_hook`], installed before the takeover
/// and still installed here. If anything in this function panics, the hook calls
/// [`tui::restore`] — which is idempotent, so restoring a terminal this function
/// has already restored is harmless — and then the original hook prints on a
/// terminal that can show it. `RawGuard` inside the prompt undoes its own raw
/// mode on the way out of the unwind. The shell is never left without echo.
///
/// A `Ctrl-C` at the prompt is *not* a signal: the prompt is in raw mode, so it
/// arrives as a key and [`server::apply_key`] answers `Cancel` — the buffer is
/// dropped, nothing is stored, and the Deck comes back. A `Ctrl-C` in the sliver
/// between the teardown and the prompt's `enable_raw_mode` *is* a signal, and it
/// kills the process on a terminal that has already been fully restored, which
/// is the best possible outcome for it.
///
/// # Errors
/// Only a terminal that could not be taken back. The prompt's own failures —
/// cancelled, keychain refused, config unwritable — are outcomes rather than
/// errors, because none of them is a reason to stop the music.
fn serve_prompt(
    terminal: &mut Tui,
    app: &mut App,
    painter: &mut art::Painter,
    prompt: Prompt,
    suspended: &AtomicBool,
    keys: &Receiver<Event>,
) -> io::Result<()> {
    // Take the cover down before the alternate screen goes: a kitty image is
    // held by the terminal rather than by the screen buffer, so leaving the
    // alternate screen is not on its own enough to be sure it is gone.
    let _ = painter.clear(&mut io::stdout());
    // Set *before* the teardown, so there is no window in which a keystroke
    // could reach the fold and be acted on by a Deck that is no longer drawn.
    suspended.store(true, Ordering::Release);
    let torn_down = tui::restore();

    let outcome = match torn_down {
        Ok(()) => ask(&prompt, app.config_path(), keys),
        // The Deck is still up and raw mode is still on, so prompting would be
        // exactly the thing this function exists not to do.
        Err(e) => PromptOutcome::Failed(format!("could not free the terminal: {e}")),
    };

    suspended.store(false, Ordering::Release);
    // Anything typed after the prompt returned and before the flag cleared is a
    // keystroke aimed at a prompt that is gone. Dropped rather than replayed
    // into a Deck that has been off screen for however long the password took.
    while keys.try_recv().is_ok() {}

    *terminal = tui::init()?;
    painter.invalidate();
    // The window may have been resized while the Deck was off screen, and that
    // `Resize` event went to the prompt's channel and was dropped with the rest.
    // Nothing else in this loop reads the size except on a `Resize`, so without
    // this the Deck comes back laying itself out for the terminal it left.
    let size = terminal.size()?;
    app.set_term_size(size.width, size.height);
    app.finish_prompt(outcome);
    Ok(())
}

/// The prompt itself: ask, store, and — for a new server — write the config.
///
/// Split out from [`serve_prompt`] so the terminal is restored on **every** exit
/// path here, including the early returns, without a guard that would have to
/// own a `&mut Tui` to do its job.
///
/// The keychain is written **before** the config file, on purpose: a cancelled
/// or refused prompt leaves `config.toml` exactly as it was, rather than adding
/// a `[[servers]]` entry with no password behind it — which is precisely the
/// half-configured state this whole feature exists to get people out of.
fn ask(
    prompt: &Prompt,
    config_path: Option<&std::path::Path>,
    keys: &Receiver<Event>,
) -> PromptOutcome {
    let server = prompt.server();
    let text = format!("Password for {}@{}: ", server.username, server.name);
    // Reading from the channel the input thread forwards to, rather than from
    // crossterm directly — see [`spawn_input_reader`].
    let answer = server::prompt_password_from(&text, || {
        keys.recv()
            .map_err(|_| io::Error::other("the input thread stopped"))
    });
    let password = match answer {
        Ok(Some(password)) => password,
        Ok(None) => return PromptOutcome::Cancelled,
        Err(e) => return PromptOutcome::Failed(format!("could not read the password: {e}")),
    };
    if let Err(e) = server::set_password(&server.name, &password) {
        return PromptOutcome::Failed(e.to_string());
    }
    // The password is out of this function's hands from here. Dropped
    // explicitly, so the last statement about it in this file is that it is
    // gone.
    drop(password);

    let mut notes = Vec::new();
    if matches!(prompt, Prompt::Add(_)) {
        match config_path {
            Some(path) => {
                if let Err(e) = config::write_server(path, server) {
                    // The keychain has it, so the server *works* this session —
                    // it just will not come back. That is worth a sentence, not
                    // a failure.
                    notes.push(format!("not saved to the config: {e}"));
                }
            }
            None => notes.push("not saved: no config file".to_string()),
        }
    }
    PromptOutcome::Stored {
        server: server.clone(),
        notes,
    }
}
