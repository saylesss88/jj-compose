//! A TUI for writing and revising Conventional Commit descriptions of
//! changes in a `jj` repository, with the change's diff alongside.
//!
//! See `message` for the message format and how existing descriptions are
//! parsed without being rewritten.

use std::{error::Error, io, path::PathBuf, process::ExitCode};

use clap::Parser;
use ratatui::crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
};

use jj_compose::app::{App, Exit, Keymap, Session};
use jj_compose::diff;
use jj_compose::jj::Jj;

/// Write a Conventional Commit description for a jj change.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// The revision to describe [default: @]
    #[arg(value_name = "REVSET", conflicts_with = "revision")]
    revset: Option<String>,

    /// The revision to describe (same as the positional argument)
    #[arg(short, long, value_name = "REVSET")]
    revision: Option<String>,

    /// Path to the repository (passed to jj as `-R`)
    #[arg(short = 'R', long, value_name = "PATH")]
    repository: Option<PathBuf>,

    /// Key bindings for the editor
    #[arg(long, value_enum, env = "JJ_COMPOSE_KEYMAP", default_value = "default")]
    keymap: Keymap,

    /// Start in the raw message editor instead of the form
    #[arg(long)]
    raw: bool,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `Display`, not the `Debug` output `main` would print.
            eprintln!("jj-compose: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), Box<dyn Error>> {
    let revset = args.revset.or(args.revision).unwrap_or_else(|| "@".into());
    let jj = Jj::new(args.repository);

    // Fail before touching the terminal: resolve the revision once. The diff
    // and description are read from that commit; `jj describe` targets its
    // change ID, which survives jj snapshotting edits made meanwhile.
    let target = jj.resolve(&revset)?;
    let diff = diff::load(&jj, &target)?;
    let description = jj.description(&target)?;
    let session = Session {
        description: description.clone(),
        change_id: target.change_id.clone(),
        keymap: args.keymap,
        raw: args.raw,
    };

    let mut terminal = init_terminal()?;
    let result = App::new(diff, session).run(&mut terminal);
    restore_terminal();

    // Run `jj describe` only after the terminal is restored, so its output
    // (and any error) is visible to the user.
    if let Exit::Commit(message) = result? {
        if message == description.trim_end() {
            eprintln!("Description of {} unchanged.", target.change_id);
        } else {
            jj.describe(&target, &message)?;
        }
    }

    Ok(())
}

/// Raw mode, the alternate screen, and bracketed paste (so a pasted
/// paragraph arrives as text, not as a stream of Enter presses).
fn init_terminal() -> io::Result<ratatui::DefaultTerminal> {
    // `init` installs a panic hook that restores the terminal; chain ours in
    // front of it so a panic also turns bracketed paste back off.
    let terminal = ratatui::init();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        hook(info);
    }));
    execute!(io::stdout(), EnableBracketedPaste)?;
    Ok(terminal)
}

fn restore_terminal() {
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
}
