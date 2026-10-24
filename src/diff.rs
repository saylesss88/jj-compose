//! The diff pane: loading `jj diff` output and scrolling through it.

use std::{error::Error, iter};

use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::{
    jj::{Jj, Target},
    patch::{self, FileDiff},
};

/// Columns a tab advances to. Terminal tab stops are every 8 columns, but
/// diffs are already indented by line numbers, so a narrower stop keeps
/// tab-indented code readable in a half-width pane.
const TAB_WIDTH: usize = 4;

/// Load and parse the diff of `target`.
///
/// # Errors
///
/// Returns an error if:
/// * Executing the underlying `jj diff` command fails or returns a non-zero exit status.
/// * Parsing the generated git diff output and its ANSI escape sequences into a `DiffView` fails.
pub fn load(jj: &Jj, target: &Target) -> Result<DiffView, Box<dyn Error>> {
    let rev = target.commit_id.as_str();
    let stdout = jj.output(&["diff", "-r", rev, "--git", "--color=always"])?;
    let empty = format!("(no changes in {})", target.change_id);
    DiffView::from_output(&stdout, &empty)
}

/// A file's position in the rendered diff.
pub struct FileEntry {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    /// Row of the file's title line.
    pub row: usize,
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
    rows: Vec<Line<'static>>,
    files: Vec<FileEntry>,
    /// Row of every hunk header, in order.
    hunk_rows: Vec<usize>,
    scroll: usize,
    /// Columns scrolled to the right, for long lines.
    pan: u16,
    /// Rows visible inside the pane, updated on every draw.
    height: usize,
}

impl DiffView {
    /// Build the view from `jj diff --git --color=always` output. `empty` is
    /// shown when there are no changes.
    ///
    /// # Errors
    ///
    /// If the colors in `output` can't be parsed.
    pub fn from_output(output: &[u8], empty: &str) -> Result<Self, Box<dyn Error>> {
        let expanded = expand_tabs(output);
        let styled = expanded.into_text()?.lines;
        let plain: Vec<String> = patch::strip_ansi(&String::from_utf8_lossy(&expanded))
            .lines()
            .map(str::to_owned)
            .collect();
        let files = patch::parse(&plain);

        let mut view = Self {
            rows: Vec::new(),
            files: Vec::new(),
            hunk_rows: Vec::new(),
            scroll: 0,
            pan: 0,
            height: 0,
        };
        if files.is_empty() {
            view.rows
                .push(Line::raw(empty.to_owned()).style(Color::DarkGray));
        }
        for file in &files {
            view.push_file(file, &styled, &plain);
        }
        Ok(view)
    }

    /// A title row with the path and counts, any header lines that say
    /// something (new, deleted, renamed, binary), then the hunks.
    fn push_file(&mut self, file: &FileDiff, styled: &[Line<'static>], plain: &[String]) {
        if !self.rows.is_empty() {
            self.rows.push(Line::default()); // blank line between files
        }
        self.files.push(FileEntry {
            path: file.path.clone(),
            added: file.added,
            removed: file.removed,
            row: self.rows.len(),
        });
        self.rows.push(file_title(file));

        let line = |i: usize| styled.get(i).cloned().unwrap_or_default();
        for i in file.header.clone() {
            // The path and counts are already in the title.
            let noise = ["diff --git ", "index ", "--- ", "+++ "];
            if !noise.iter().any(|p| plain[i].starts_with(p)) {
                self.rows.push(line(i).style(Color::DarkGray));
            }
        }
        for hunk in &file.hunks {
            self.hunk_rows.push(self.rows.len());
            self.rows.push(line(hunk.header));
            self.rows.extend(hunk.lines.clone().map(line));
        }
    }

    /// The file at the top of the pane, numbered from 1.
    #[must_use]
    pub fn current_file(&self) -> Option<(usize, &FileEntry)> {
        // The last file whose title is at or above the first visible row.
        let i = self.files.partition_point(|f| f.row <= self.scroll);
        let i = i.checked_sub(1)?;
        Some((i + 1, &self.files[i]))
    }

    #[must_use]
    pub const fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Half the visible height (at least one line).
    #[must_use]
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

    pub const fn scroll_to_bottom(&mut self) {
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
        self.rows.len().saturating_sub(self.height)
    }

    /// Draw only the visible lines, so a large diff costs the same per
    /// frame as a small one.
    pub fn render(&mut self, frame: &mut Frame, area: Rect, block: Block<'static>) {
        self.height = usize::from(block.inner(area).height);
        self.scroll = self.scroll.min(self.max_scroll());

        let end = (self.scroll + self.height).min(self.rows.len());
        let visible = self.rows[self.scroll..end].to_vec();
        let paragraph = Paragraph::new(visible).block(block).scroll((0, self.pan));
        frame.render_widget(paragraph, area);
    }
}

/// `path  +12 -3` in bold, with the counts in green and red.
fn file_title(file: &FileDiff) -> Line<'static> {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(file.path.clone(), bold.fg(Color::Yellow))];
    if file.binary {
        spans.push(Span::styled("  binary", Style::new().fg(Color::DarkGray)));
    } else {
        spans.push(Span::styled(
            format!("  +{}", file.added),
            bold.fg(Color::Green),
        ));
        spans.push(Span::styled(
            format!(" -{}", file.removed),
            bold.fg(Color::Red),
        ));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `jj diff --git --color=always` prints for a two-file change.
    const OUTPUT: &[u8] = b"\
\x1b[1mdiff --git a/a.rs b/a.rs\x1b[0m
\x1b[1mindex 1111..2222 100644\x1b[0m
\x1b[1m--- a/a.rs\x1b[0m
\x1b[1m+++ b/a.rs\x1b[0m
\x1b[38;5;6m@@ -1,2 +1,2 @@\x1b[39m
 keep
\x1b[38;5;1m-\told\x1b[39m
\x1b[38;5;2m+\tnew\x1b[39m
\x1b[1mdiff --git a/b.txt b/b.txt\x1b[0m
\x1b[1mnew file mode 100644\x1b[0m
\x1b[38;5;6m@@ -0,0 +1,1 @@\x1b[39m
\x1b[38;5;2m+hi\x1b[39m
";

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

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
    fn rows_have_titles_hunks_and_no_header_noise() {
        let view = DiffView::from_output(OUTPUT, "").unwrap();
        let rows: Vec<String> = view.rows.iter().map(text).collect();
        assert_eq!(
            rows,
            [
                "a.rs  +1 -1",
                "@@ -1,2 +1,2 @@",
                " keep",
                "-   old",
                "+   new",
                "",
                "b.txt  +1 -0",
                "new file mode 100644",
                "@@ -0,0 +1,1 @@",
                "+hi",
            ]
        );
        assert_eq!(view.files.iter().map(|f| f.row).collect::<Vec<_>>(), [0, 6]);
        assert_eq!(view.hunk_rows, [1, 8]);
    }

    #[test]
    fn current_file_follows_the_scroll() {
        let mut view = DiffView::from_output(OUTPUT, "").unwrap();
        view.height = 2;
        assert_eq!(view.current_file().unwrap().0, 1);
        view.scroll_down(5);
        assert_eq!(view.current_file().unwrap().0, 1);
        view.scroll_down(1);
        let (n, file) = view.current_file().unwrap();
        assert_eq!((n, file.path.as_str(), file.added), (2, "b.txt", 1));
    }

    #[test]
    fn empty_diff_says_so() {
        let view = DiffView::from_output(b"", "(no changes)").unwrap();
        assert_eq!(text(&view.rows[0]), "(no changes)");
        assert!(view.files.is_empty());
    }

    #[test]
    fn scrolling_is_clamped() {
        let mut view = DiffView::from_output(OUTPUT, "").unwrap();
        view.height = 4;
        view.scroll_down(100);
        assert_eq!(view.scroll, 6);
        view.scroll_up(100);
        assert_eq!(view.scroll, 0);
    }
}
