//! The diff pane: loading `jj diff` output and scrolling through it.

use std::{error::Error, iter};

use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Text},
    widgets::{Block, Paragraph},
};

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
                out.extend(iter::repeat_n(b' ', spaces));
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

/// The diff text plus its scroll position.
pub struct DiffView {
    lines: Vec<Line<'static>>,
    /// First visible line.
    scroll: usize,
    /// Columns scrolled to the right, for long lines.
    pan: u16,
    /// Rows visible inside the pane, updated on every draw.
    height: usize,
}

impl DiffView {
    pub fn new(text: Text<'static>) -> Self {
        Self {
            lines: text.lines,
            scroll: 0,
            pan: 0,
            height: 0,
        }
    }

    /// Half the visible height (at least one line).
    pub const fn half_page(&self) -> usize {
        if self.height > 2 { self.height / 2 } else { 1 }
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_add(n).min(self.max_scroll());
    }

    pub const fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub const fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    pub const fn pan_right(&mut self, n: u16) {
        self.pan = self.pan.saturating_add(n);
    }

    pub const fn pan_left(&mut self, n: u16) {
        self.pan = self.pan.saturating_sub(n);
    }

    /// The last scroll offset that still fills the pane.
    const fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.height)
    }

    /// Draw only the visible lines, so a large diff costs the same per
    /// frame as a small one.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, block: Block<'static>) {
        self.height = usize::from(block.inner(area).height);
        self.scroll = self.scroll.min(self.max_scroll());

        let end = (self.scroll + self.height).min(self.lines.len());
        let visible = self.lines[self.scroll..end].to_vec();
        let paragraph = Paragraph::new(visible).block(block).scroll((0, self.pan));
        frame.render_widget(paragraph, area);
    }
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

    #[test]
    fn scrolling_is_clamped() {
        let mut view = DiffView::new(Text::from("1\n2\n3\n4\n5"));
        view.height = 2;
        view.scroll_down(100);
        assert_eq!(view.scroll, 3);
        view.scroll_up(100);
        assert_eq!(view.scroll, 0);
    }
}
