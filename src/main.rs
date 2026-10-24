//! A small TUI for writing Conventional Commit messages for the working-copy
//! change in a `jj` repository.
//!
//! Message format:
//!
//! ```text
//! type(scope): subject
//!
//! Problem:
//! ...
//!
//! Solution:
//! ...
//! ```

mod app;
mod diff;
mod jj;
mod message;

use std::{error::Error, io, path::PathBuf};

use clap::Parser;

use app::{App, Exit};
use diff::{DiffFormat, load_diff};
use jj::Jj;
use ratatui::crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
};

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

    /// Show the diff in git format instead of jj's configured format
    #[arg(long)]
    git: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let revset = args.revset.or(args.revision).unwrap_or_else(|| "@".into());
    let jj = Jj::new(args.repository);

    // Fail before touching the terminal: resolve the revision once and use
    // that exact commit for the diff and for `jj describe`.
    let target = jj.resolve(&revset)?;
    let format = if args.git {
        DiffFormat::Git
    } else {
        DiffFormat::Jj
    };
    let diff = load_diff(&jj, &target, format)?;

    let mut terminal = init_terminal()?;
    let result = App::new(diff).run(&mut terminal);
    restore_terminal();

    // Run `jj describe` only after the terminal is restored, so its output
    // (and any error) is visible to the user.
    if let Exit::Commit(message) = result? {
        jj.describe(&target, &message)?;
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
