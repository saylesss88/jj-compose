//! Application state, input handling and rendering.

use std::io;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph},
};
use ratatui_textarea::{CursorMove, Input, TextArea, WrapMode};

use crate::{
    message::{Draft, Field, HEADER_HARD_LIMIT, HEADER_SOFT_LIMIT},
    vim::{self, Vim},
};

/// Which key bindings the editor uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Keymap {
    /// Type immediately; standard editing keys (the default)
    #[default]
    Default,
    /// Modal editing: Normal/Insert/Visual with Vim motions
    Vim,
}

/// Where keyboard focus is: the diff pane, or the current form field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Diff,
    Editor,
}

/// A one-shot message for the status bar, cleared by the next key press.
enum Status {
    Error(String),
    /// Quit was requested with unsaved edits; a second quit confirms.
    ConfirmQuit,
}

/// How the event loop ended.
pub enum Exit {
    Abort,
    Commit(String),
}

pub struct App {
    diff: Text<'static>,
    diff_len: u16,
    diff_scroll: u16,
    /// Rows visible inside the diff pane, updated on every draw.
    diff_height: u16,
    /// One editor per [`Field`], indexed by `field as usize`.
    fields: [TextArea<'static>; Field::ALL.len()],
    /// The form field being edited, or to return to from the diff pane.
    field: Field,
    focus: Focus,
    /// The message the session started with, to detect unsaved edits.
    original: String,
    status: Option<Status>,
    keymap: Keymap,
    /// Modal state, used only with [`Keymap::Vim`].
    vim: Vim,
}

impl App {
    pub fn new(diff: Text<'static>, keymap: Keymap) -> Self {
        let diff_len = u16::try_from(diff.lines.len()).unwrap_or(u16::MAX);
        Self {
            diff,
            diff_len,
            diff_scroll: 0,
            diff_height: 0,
            fields: Field::ALL.map(|field| field_editor(field, "")),
            field: Field::Type,
            focus: Focus::Editor,
            original: String::new(),
            status: None,
            keymap,
            vim: Vim::new(vim::Mode::Insert),
        }
    }

    /// Runs until the user commits or quits.
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<Exit> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

            let exit = match event::read()? {
                // Ignore key *release* events (reported on Windows and with the
                // kitty keyboard protocol), or every keystroke registers twice.
                Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
                Event::Paste(text) => {
                    self.paste(&text);
                    None
                }
                _ => None,
            };
            if let Some(exit) = exit {
                return Ok(exit);
            }
        }
    }

    fn draft(&self) -> Draft {
        let mut draft = Draft::default();
        for field in Field::ALL {
            draft.set(field, self.fields[field as usize].lines().join("\n"));
        }
        draft
    }

    fn is_dirty(&self) -> bool {
        self.draft().render() != self.original
    }

    const fn editor(&mut self) -> &mut TextArea<'static> {
        &mut self.fields[self.field as usize]
    }

    // -----------------------------------------------------------------------
    // Input
    // -----------------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) -> Option<Exit> {
        let confirming = matches!(self.status, Some(Status::ConfirmQuit));
        self.status = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        // Global bindings, wherever focus is.
        match key.code {
            KeyCode::Char('s') if ctrl => return self.submit(),
            KeyCode::Char('c') if ctrl => return self.quit(confirming),
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            // Read the diff without leaving the field you're typing in.
            KeyCode::PageDown => self.scroll_down(self.diff_height / 2),
            KeyCode::PageUp => self.scroll_up(self.diff_height / 2),
            KeyCode::Down if alt => self.scroll_down(1),
            KeyCode::Up if alt => self.scroll_up(1),
            _ => {
                return match self.focus {
                    Focus::Diff => self.diff_key(key.code, ctrl, confirming),
                    Focus::Editor if self.keymap == Keymap::Vim => self.vim_key(key, confirming),
                    Focus::Editor => self.editor_key(key, confirming),
                };
            }
        }
        None
    }

    fn diff_key(&mut self, code: KeyCode, ctrl: bool, confirming: bool) -> Option<Exit> {
        match code {
            KeyCode::Char('q') if self.keymap == Keymap::Vim => return self.quit(confirming),
            KeyCode::Char('j') | KeyCode::Down => self.scroll_down(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_up(1),
            KeyCode::Char('d') if ctrl => self.scroll_down(self.diff_height / 2),
            KeyCode::Char('u') if ctrl => self.scroll_up(self.diff_height / 2),
            KeyCode::Char(' ') => self.scroll_down(self.diff_height / 2),
            KeyCode::Char('g') | KeyCode::Home => self.diff_scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.diff_scroll = self.max_scroll(),
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i') => self.focus = Focus::Editor,
            _ => {}
        }
        None
    }

    fn vim_key(&mut self, key: KeyEvent, confirming: bool) -> Option<Exit> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.vim.mode == vim::Mode::Insert {
            if key.code != KeyCode::Esc {
                // Insert mode types like the default keymap.
                return self.editor_key(key, confirming);
            }
            self.vim.mode = vim::Mode::Normal;
            let editor = self.editor();
            if editor.cursor().1 > 0 {
                editor.move_cursor(CursorMove::Back);
            }
            return None;
        }

        // Normal mode: the scroll keys that would move a long buffer move
        // the diff instead, since fields are short.
        match key.code {
            KeyCode::Char('d') if ctrl => self.scroll_down(self.diff_height / 2),
            KeyCode::Char('u') if ctrl => self.scroll_up(self.diff_height / 2),
            KeyCode::Char('e') if ctrl => self.scroll_down(1),
            KeyCode::Char('y') if ctrl => self.scroll_up(1),
            _ => {
                let editor = &mut self.fields[self.field as usize];
                match self.vim.handle(&Input::from(key), editor) {
                    vim::Action::None => {}
                    vim::Action::Quit => return self.quit(confirming),
                    vim::Action::NextField => self.focus_field(self.field.next()),
                    vim::Action::PrevField => self.focus_field(self.field.prev()),
                }
            }
        }
        None
    }

    fn editor_key(&mut self, key: KeyEvent, confirming: bool) -> Option<Exit> {
        let field = self.field;
        // Shift+arrows select text, so only bare arrows leave the field.
        let plain = key.modifiers.is_empty();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let editor = self.editor();
        let row = editor.cursor().0;
        let last_row = editor.lines().len() - 1;

        match key.code {
            KeyCode::Esc => return self.quit(confirming),
            // Enter on a one-line field moves on, like a form.
            KeyCode::Enter if !field.is_multiline() => self.focus_field(field.next()),
            // Arrowing off the top or bottom of a field moves between fields.
            KeyCode::Up if plain && row == 0 && field != Field::Type => {
                self.focus_field(field.prev());
            }
            KeyCode::Down if plain && row == last_row && field.next() != field => {
                self.focus_field(field.next());
            }
            KeyCode::Char('z') if ctrl => {
                editor.undo();
            }
            KeyCode::Char('w') if alt => editor.copy(),
            _ => {
                editor.input(Input::from(key));
            }
        }
        None
    }

    /// Bracketed paste: insert the text as-is, instead of replaying it as
    /// key presses (where every newline would act as Enter).
    fn paste(&mut self, text: &str) {
        if self.focus != Focus::Editor {
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let text = if self.field.is_multiline() {
            text
        } else {
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        self.editor().insert_str(text);
    }

    /// Quit now if nothing would be lost (or this is the second request).
    fn quit(&mut self, confirmed: bool) -> Option<Exit> {
        if confirmed || !self.is_dirty() {
            return Some(Exit::Abort);
        }
        self.status = Some(Status::ConfirmQuit);
        None
    }

    /// Finish with a valid message, or stay open and show why not.
    fn submit(&mut self) -> Option<Exit> {
        match self.draft().build() {
            Ok(message) => Some(Exit::Commit(message)),
            Err(e) => {
                self.status = Some(Status::Error(e.into()));
                None
            }
        }
    }

    /// Tab order: every form field, then the diff pane, then around again.
    fn cycle_focus(&mut self, forward: bool) {
        let stops = Field::ALL.len() + 1;
        let current = match self.focus {
            Focus::Editor => self.field as usize,
            Focus::Diff => Field::ALL.len(),
        };
        let next = if forward {
            (current + 1) % stops
        } else {
            (current + stops - 1) % stops
        };
        if let Some(&field) = Field::ALL.get(next) {
            self.focus_field(field);
        } else {
            self.editor().cancel_selection();
            self.focus = Focus::Diff;
        }
    }

    fn focus_field(&mut self, field: Field) {
        // Leave no half-finished selection or operator behind.
        self.editor().cancel_selection();
        if matches!(self.vim.mode, vim::Mode::Visual | vim::Mode::Operator(_)) {
            self.vim.mode = vim::Mode::Normal;
        }
        self.focus = Focus::Editor;
        self.field = field;
    }

    /// The last scroll offset that still fills the pane.
    const fn max_scroll(&self) -> u16 {
        self.diff_len.saturating_sub(self.diff_height)
    }

    fn scroll_down(&mut self, n: u16) {
        self.diff_scroll = self
            .diff_scroll
            .saturating_add(n.max(1))
            .min(self.max_scroll());
    }

    fn scroll_up(&mut self, n: u16) {
        self.diff_scroll = self.diff_scroll.saturating_sub(n.max(1));
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn draw(&mut self, frame: &mut Frame) {
        let [main, status] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
        // Side by side when there's room; stacked on narrow terminals.
        let [diff_area, form_area] = if main.width >= 100 {
            Layout::horizontal([Constraint::Percentage(50); 2]).areas(main)
        } else {
            Layout::vertical([Constraint::Percentage(40), Constraint::Fill(1)]).areas(main)
        };

        // Remember the viewport height for half-page scrolling and clamping.
        self.diff_height = diff_area.height.saturating_sub(2);
        self.diff_scroll = self.diff_scroll.min(self.max_scroll());

        let accent = self.accent();
        let diff = Paragraph::new(self.diff.clone())
            .block(pane(
                " Diff ".into(),
                (self.focus == Focus::Diff).then_some(accent),
            ))
            .scroll((self.diff_scroll, 0));
        frame.render_widget(diff, diff_area);

        self.draw_form(frame, form_area);
        frame.render_widget(self.status_line(), status);
    }

    fn draw_form(&mut self, frame: &mut Frame, area: Rect) {
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
            editor.set_cursor_style(if focused {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new()
            });
            frame.render_widget(&*editor, area);
        }
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
    fn status_line(&self) -> Line<'static> {
        match &self.status {
            Some(Status::Error(err)) => return Line::from(format!(" ✗ {err}")).red().bold(),
            Some(Status::ConfirmQuit) => {
                return Line::from(
                    " Unsaved changes. Quit again to discard them, or press any other key to keep editing.",
                )
                .yellow()
                .bold();
            }
            None => {}
        }
        let keys = match (self.focus, self.keymap, self.vim.mode) {
            (Focus::Diff, ..) => {
                " j/k scroll · ^d/^u half page · g/G top/bottom · Esc back · ^s save"
            }
            (Focus::Editor, Keymap::Default, _) => {
                " Tab next field · Enter next/newline · PgUp/PgDn diff · ^z undo · ^s save · Esc quit"
            }
            (Focus::Editor, Keymap::Vim, vim::Mode::Insert) => {
                " Esc normal · Tab next field · Enter next/newline · ^s save"
            }
            (Focus::Editor, Keymap::Vim, vim::Mode::Visual) => " d/c/y operate · Esc cancel",
            (Focus::Editor, Keymap::Vim, _) => {
                " i insert · j/k/Tab fields · ^d/^u diff · u undo · ^s save · q quit"
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

/// An editor for `field`, pre-filled with `text`, cursor at the end.
fn field_editor(field: Field, text: &str) -> TextArea<'static> {
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
