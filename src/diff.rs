//! Loading the diff shown next to the form.

use std::error::Error;

use ansi_to_tui::IntoText;
use ratatui::text::Text;

use crate::jj::{Jj, Target};

/// Columns a tab advances to. Terminal tab stops are every 8 columns, but
/// diffs are already indented by line numbers, so a narrower stop keeps
/// tab-indented code readable in a half-width pane.
const TAB_WIDTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffFormat {
    /// Whatever format the user configured in jj (color-words by default).
    Jj,
    /// Unified git-format diff.
    Git,
}

/// Load the diff of `target`, styled with jj's own colors.
pub fn load_diff(
    jj: &Jj,
    target: &Target,
    format: DiffFormat,
) -> Result<Text<'static>, Box<dyn Error>> {
    let rev = target.commit_id.as_str();
    let args: &[&str] = match format {
        DiffFormat::Jj => &["diff", "-r", rev, "--color=always"],
        DiffFormat::Git => &["diff", "-r", rev, "--git", "--color=always"],
    };
    let stdout = jj.output(args)?;

    if stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Text::raw(format!("(no changes in {})", target.change_id)));
    }

    // Turn the ANSI escape codes into ratatui styles instead of printing
    // them as literal `[38;5;2m` garbage.
    Ok(expand_tabs(&stdout).into_text()?)
}

/// Replace tabs with spaces up to the next tab stop. Terminals would do
/// this, but ratatui gives a tab zero width, so tab-indented code would
/// otherwise lose its indentation. Escape sequences take no columns.
fn expand_tabs(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut column = 0;
    let mut in_escape = false;
    for &b in bytes {
        match b {
            b'\t' => {
                let spaces = TAB_WIDTH - column % TAB_WIDTH;
                out.extend(std::iter::repeat_n(b' ', spaces));
                column += spaces;
                continue;
            }
            b'\n' => column = 0,
            0x1b => in_escape = true,
            // An SGR sequence (the only kind jj emits) ends with `m`.
            b'm' if in_escape => in_escape = false,
            // Count characters, not UTF-8 continuation bytes.
            _ if !in_escape && b & 0xC0 != 0x80 => column += 1,
            _ => {}
        }
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tabs_to_tab_stops() {
        assert_eq!(expand_tabs(b"\tx"), b"    x");
        assert_eq!(expand_tabs(b"ab\tx\n\ty"), b"ab  x\n    y");
    }

    #[test]
    fn escapes_and_multibyte_chars_take_no_extra_columns() {
        assert_eq!(expand_tabs(b"\x1b[32m+\x1b[0m\tx"), b"\x1b[32m+\x1b[0m   x");
        assert_eq!(expand_tabs("é\tx".as_bytes()), "é   x".as_bytes());
    }
}
