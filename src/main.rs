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

use std::{error::Error, path::PathBuf};

use clap::Parser;

use app::App;
use diff::{DiffFormat, load_diff};
use jj::Jj;

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

    // `init` enters raw mode + the alternate screen and installs a panic hook
    // that restores the terminal, so a panic won't leave the shell garbled.
    let mut terminal = ratatui::init();
    let result = App::new(diff).run(&mut terminal);

    // Leave raw mode and the alternate screen.
    ratatui::restore();

    // Run `jj describe` only after the terminal is restored, so its output
    // (and any error) is visible to the user.
    if let Some(message) = result? {
        jj.describe(&target, &message)?;
    }

    Ok(())
}
