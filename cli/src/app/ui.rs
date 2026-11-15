//! Turning application state into a frame.

use jj_compose_core::{Field, HEADER_HARD_LIMIT, HEADER_SOFT_LIMIT, Split, overview};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

use super::{App, Focus, Keymap, Status, View, plural};
use crate::vim;

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame<'_>) {
        let [main, status] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
        // Zoom lasts only while the diff has focus.
        self.zoomed &= self.focus == Focus::Diff;
        // Side by side when there's room; stacked on narrow terminals.
        let [diff_area, form_area] = if self.zoomed {
            [main, Rect::default()]
        } else if main.width >= 100 {
            Layout::horizontal([Constraint::Percentage(50); 2]).areas(main)
        } else {
            Layout::vertical([Constraint::Percentage(40), Constraint::Fill(1)]).areas(main)
        };

        let accent = self.accent();

        // While splitting, the pane is one hunk and a tally, not the diff.
        let splitting = match &self.split {
            Some(split) if self.choosing() => Some((
                split.at(),
                split.total(),
                split.path().to_owned(),
                split.count(),
                split.strip(),
                split.is_marked(),
            )),
            _ => None,
        };
        if let Some(cursor) = self.overview.filter(|_| self.choosing()) {
            let rows = self.overview_rows();
            let marked = self.split.as_ref().map_or(0, Split::count);
            let hunks = rows.iter().filter(|r| r.hunk.is_some()).count();
            let title = Line::from(vec![
                Span::raw(format!(" Hunks in {} ", self.change_id)),
                immutable_marker(self.target.immutable),
                Span::styled(format!("· {marked} of {hunks} marked "), Color::Green),
            ]);
            let block = pane(title, Some(accent));
            let inner = block.inner(diff_area);
            frame.render_widget(block, diff_area);
            let height = usize::from(inner.height).max(1);
            // Keep the cursor in view without a scrollbar's worth of state.
            let top = cursor
                .saturating_sub(height / 2)
                .min(rows.len().saturating_sub(height));
            let lines: Vec<Line<'static>> = rows
                .iter()
                .enumerate()
                .skip(top)
                .take(height)
                .map(|(i, row)| overview_line(row, i == cursor))
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
            frame.render_widget(self.status_line(), status);
            return;
        }

        if let Some((at, total, path, marked, strip, on)) = splitting {
            let mut title = vec![
                Span::raw(format!(" Split {} ", self.change_id)),
                immutable_marker(self.target.immutable),
                Span::raw(format!("· hunk {}/{total} ", at + 1)),
                Span::raw(format!("· {path} ")),
                // The strip is every hunk at once: `#` here and marked, `o`
                // here and not, `+` marked elsewhere, `·` not.
                Span::styled(format!("· [{strip}] "), Color::Cyan),
                Span::styled(
                    format!("· {marked} marked "),
                    if on { Color::Green } else { Color::DarkGray },
                ),
            ];
            title.retain(|span| !span.content.is_empty());
            let title = Line::from(title);
            let block = pane(title, Some(accent));
            self.diff.render_hunk(frame, diff_area, block, at);
            frame.render_widget(self.status_line(), status);
            return;
        }

        // The pane title is built up in pieces: change ID, then the file at the
        // top (commit 2), then the search (commit 4).
        let mut title = vec![
            Span::raw(format!(" Diff {} ", self.change_id)),
            immutable_marker(self.target.immutable),
        ];
        title.retain(|span| !span.content.is_empty());
        if let Some((n, file)) = self.diff.current_file() {
            let count = self.diff.file_count();
            title.push(Span::raw(format!("· {n}/{count} {} ", file.path)));
            title.push(Span::styled(format!("+{} ", file.added), Color::Green));
            title.push(Span::styled(format!("-{} ", file.removed), Color::Red));
        }
        if let Some((query, n, count)) = self.diff.search_status() {
            title.push(Span::raw(format!("· /{query} {n}/{count} ")));
        }

        let block = pane(
            Line::from(title),
            (self.focus == Focus::Diff).then_some(accent),
        );
        self.diff.render(frame, diff_area, block);

        if !self.zoomed {
            match self.view {
                View::Structured => self.draw_form(frame, form_area),
                View::Raw => self.draw_raw(frame, form_area),
            }
        }
        frame.render_widget(self.status_line(), status);
    }

    fn draw_form(&mut self, frame: &mut Frame<'_>, area: Rect) {
        // Trailers are usually a line or two; give the room to the prose.
        let trailer_rows = self.fields[Field::Trailers as usize].lines().len();
        let trailer_height = u16::try_from(trailer_rows.clamp(1, 4)).unwrap_or(4) + 2;
        let [top, subject, body, problem, solution, trailers] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Length(trailer_height),
        ])
        .areas(area);
        let [ty, scope] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Fill(1)]).areas(top);
        let areas = [ty, scope, subject, body, problem, solution, trailers];

        let header_len = self.draft().header().chars().count();
        let accent = self.accent();
        for (field, area) in Field::ALL.into_iter().zip(areas) {
            let focused = self.focus == Focus::Editor && self.field == field;
            let editor = &mut self.fields[field as usize];
            editor.set_block(pane(
                field_title(field, header_len),
                focused.then_some(accent),
            ));
            editor.set_cursor_style(cursor_style(focused));
            frame.render_widget(&*editor, area);
        }
    }

    fn draw_raw(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let header_len = self.raw.lines()[0].chars().count();
        let title = Line::from(vec![
            Span::raw(" Message (raw) "),
            header_counter(header_len),
        ]);
        let focused = self.focus == Focus::Editor;
        let accent = self.accent();
        self.raw.set_block(pane(title, focused.then_some(accent)));
        self.raw.set_cursor_style(cursor_style(focused));
        frame.render_widget(&self.raw, area);
    }

    /// Border color of the focused pane: green in Vim's Normal mode,
    /// yellow while typing.
    const fn accent(&self) -> Color {
        match (self.keymap, self.vim.mode) {
            (Keymap::Vim, vim::Mode::Insert) | (Keymap::Default, _) => Color::Yellow,
            (Keymap::Vim, _) => Color::Green,
        }
    }

    /// Bottom row: a pending message, or the keys that work right now.
    pub(super) fn status_line(&self) -> Line<'static> {
        if let Some(prompt) = &self.search_prompt {
            return Line::from(format!("/{prompt}▏"));
        }
        match &self.status {
            Some(Status::Error(err)) => return Line::from(format!(" ✗ {err}")).red().bold(),
            Some(Status::Info(info)) => return Line::from(format!(" {info}")).cyan(),
            Some(Status::ConfirmQuit) => {
                return Line::from(
                    " Unsaved changes. Quit again to discard them, or press any other key to keep editing.",
                )
                .yellow()
                .bold();
            }
            None => {}
        }
        if self.overview.is_some() && self.choosing() {
            let keys =
                " Space mark · j/k row · a/d file · A/C all · Enter read it · f close · Esc leave";
            return Line::from(vec![Span::raw(keys).dark_gray()]);
        }
        if self.choosing() {
            let keys = " Space mark · n/p hunk · a/d file · A/C all · Enter message · Esc leave";
            return Line::from(vec![Span::raw(keys).dark_gray()]);
        }
        let keys = match (self.focus, self.keymap, self.vim.mode) {
            // Name the tool, so `o` says what it will actually open.
            (Focus::Diff, ..) => {
                let open = self.diff_tool.as_deref().unwrap_or("open");
                format!(
                    " j/k scroll · [/] file · {{/}} hunk · f files · / search · z zoom · s split · o/O {open} · l back"
                )
            }
            // A pending split changes what ^s does, so it says so.
            (Focus::Editor, Keymap::Default, _) => self.split.as_ref().map(Split::count).map_or_else(|| " Tab next · Enter next/newline · Alt-← diff · Alt-s split · ^t raw/form · ^s save · Esc quit".to_owned(), |marked| format!(
                     " Tab next · Alt-← hunks · ^s commit {marked} {} · ^t raw/form · Esc quit",
                     plural(marked, "hunk")
                 )),
            (Focus::Editor, Keymap::Vim, vim::Mode::Insert) => {
                " Esc normal · Tab next · Enter next/newline · ^t raw/form · ^s save".to_owned()
            }
            (Focus::Editor, Keymap::Vim, vim::Mode::Visual) => {
                " d/c/y operate · Esc cancel".to_owned()
            }
            (Focus::Editor, Keymap::Vim, _) => {
                " i insert · hjkl past an edge: next box · ^d/^u scroll diff · ^t raw/form · ^s save · q quit".to_owned()
            }
        };
        let mut spans = Vec::new();
        if self.keymap == Keymap::Vim {
            let badge = Span::raw(self.vim.mode.label()).black();
            spans.push(badge.bg(self.accent()));
        }
        spans.push(Span::raw(keys).dark_gray());
        Line::from(spans)
    }
}

const fn cursor_style(focused: bool) -> Style {
    if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    }
}

/// The raw message editor, cursor at the start.
pub(super) fn raw_editor(text: &str) -> TextArea<'static> {
    let mut editor = TextArea::new(text.split('\n').map(str::to_owned).collect());
    editor.set_cursor_line_style(Style::new());
    editor.set_wrap_mode(WrapMode::WordOrGlyph);
    editor
}

/// An editor for `field`, pre-filled with `text`, cursor at the end.
pub(super) fn field_editor(field: Field, text: &str) -> TextArea<'static> {
    let mut editor = TextArea::new(text.split('\n').map(str::to_owned).collect());
    editor.set_placeholder_text(field.placeholder());
    // The default underlines the cursor line, which is noisy in a form.
    editor.set_cursor_line_style(Style::new());
    if field.is_multiline() {
        editor.set_wrap_mode(WrapMode::WordOrGlyph);
    }
    editor.move_cursor(CursorMove::Bottom);
    editor.move_cursor(CursorMove::End);
    editor
}

/// A bordered pane, drawn in `accent` when focused.
fn pane(title: Line<'static>, accent: Option<Color>) -> Block<'static> {
    let style = Style::new().fg(accent.unwrap_or(Color::DarkGray));
    Block::bordered()
        .title(title)
        .border_style(style)
        .title_style(style)
}

/// The field's name; Subject also shows a live length counter for the
/// whole header line.
fn field_title(field: Field, header_len: usize) -> Line<'static> {
    let title = format!(" {} ", field.title());
    if field != Field::Subject {
        return Line::from(title);
    }
    Line::from(vec![Span::raw(title), header_counter(header_len)])
}

fn header_counter(len: usize) -> Span<'static> {
    let color = match len {
        0..=HEADER_SOFT_LIMIT => Color::Green,
        n if n <= HEADER_HARD_LIMIT => Color::Yellow,
        _ => Color::Red,
    };
    Span::styled(
        format!("{len}/{HEADER_HARD_LIMIT} "),
        Style::new().fg(color),
    )
}

/// `n` with `word` pluralised the easy way.
/// One row of the overview: a file heading, or a hunk marked or not.
fn overview_line(row: &overview::Row, cursor: bool) -> Line<'static> {
    let color = match row.kind {
        overview::Kind::File => Color::Cyan,
        overview::Kind::Marked => Color::Green,
        overview::Kind::Unmarked => Color::Gray,
    };
    let mut style = Style::new().fg(color);
    if row.kind == overview::Kind::File {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cursor {
        style = style.add_modifier(Modifier::REVERSED);
    }
    Line::styled(format!(" {}", row.text), style)
}

/// A standing reminder that this session is rewriting protected history, so
/// it can't be forgotten three splits in. Empty when it isn't.
pub(super) fn immutable_marker(immutable: bool) -> Span<'static> {
    if immutable {
        Span::styled("· immutable ", Color::Red)
    } else {
        Span::raw("")
    }
}
#[cfg(test)]
mod tests {
    use super::immutable_marker;

    #[test]
    fn the_immutable_marker_shows_only_on_protected_history() {
        assert_eq!(immutable_marker(false).content, "");
        assert!(immutable_marker(true).content.contains("immutable"));
    }
}
