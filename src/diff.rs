//! Loading the diff shown next to the form.

use std::error::Error;

use crate::jj::{Jj, Target};
use ansi_to_tui::IntoText;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Text},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffFormat {
    /// `jj diff --color=always`: whatever format the user configured in jj
    /// (color-words by default), ANSI-parsed into styled text.
    Jj,
    /// `jj diff --git`: plain unified diff, colored line-by-line here.
    Git,
}

pub fn load_diff(
    jj: &Jj,
    target: &Target,
    format: DiffFormat,
) -> Result<Text<'static>, Box<dyn Error>> {
    let rev = target.commit_id.as_str();
    let args: &[&str] = match format {
        DiffFormat::Jj => &["diff", "-r", rev, "--color=always"],
        DiffFormat::Git => &["diff", "-r", rev, "--git", "--color=never"],
    };
    let stdout = jj.output(args)?;

    if stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Text::raw(format!("(no changes in {})", target.change_id)));
    }

    Ok(match format {
        // Turn the ANSI escape codes into ratatui styles instead of
        // printing them as literal `[38;5;2m` garbage.
        DiffFormat::Jj => stdout.into_text()?,
        DiffFormat::Git => String::from_utf8_lossy(&stdout)
            .lines()
            .map(diff_line)
            .collect(),
    })
}

/// Color one line of a git-format diff by what kind of line it is.
fn diff_line(line: &str) -> Line<'static> {
    let style = if line.starts_with("diff --git") {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if line.starts_with("+++") || line.starts_with("---") {
        Style::new().add_modifier(Modifier::BOLD)
    } else if line.starts_with("@@") {
        Style::new().fg(Color::Cyan)
    } else if line.starts_with('+') {
        Style::new().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::new().fg(Color::Red)
    } else if line.starts_with("index ") || line.contains(" mode ") {
        Style::new().fg(Color::DarkGray)
    } else {
        Style::new()
    };
    Line::styled(line.to_owned(), style)
}
