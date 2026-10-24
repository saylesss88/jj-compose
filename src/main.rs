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
mod message;

use std::{error::Error, process::Command};

use app::App;
use diff::{DiffFormat, load_diff};

fn main() -> Result<(), Box<dyn Error>> {
    // `--git` falls back to a plain git-format diff colored by this tool;
    // the default uses jj's own colors and respects the user's jj config.
    let format = if std::env::args().skip(1).any(|a| a == "--git") {
        DiffFormat::Git
    } else {
        DiffFormat::Jj
    };
    // Fail before touching the terminal: if we can't diff, we can't describe either.
    let diff = load_diff(format)?;

    // `init` enters raw mode + the alternate screen and installs a panic hook
    // that restores the terminal, so a panic won't leave the shell garbled.
    let mut terminal = ratatui::init();
    let result = App::new(diff).run(&mut terminal);

    // Leave raw mode and the alternate screen.
    ratatui::restore();

    // Run `jj describe` only after the terminal is restored, so its output
    // (and any error) is visible to the user.
    if let Some(message) = result? {
        let status = Command::new("jj")
            .args(["describe", "@", "-m", &message])
            .status()?;
        if !status.success() {
            return Err(format!("jj describe failed ({status})").into());
        }
    }

    Ok(())
}
