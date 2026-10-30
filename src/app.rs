//! Application state, input handling and rendering.

use std::io;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::Block,
};
use ratatui_textarea::{CursorMove, Input, TextArea, WrapMode};

use crate::{
    diff::DiffView,
    message::{Draft, Field, HEADER_HARD_LIMIT, HEADER_SOFT_LIMIT},
    vim::{self, Dir, Vim},
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

/// How the message is being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// One box per part of a Conventional Commit.
    Structured,
    /// The exact message text, for anything the form can't represent.
    Raw,
}

/// What a session starts from.
pub struct Session {
    /// The revision's current description.
    pub description: String,
    /// Short change ID, for the diff pane's title.
    pub change_id: String,
    pub keymap: Keymap,
    /// Open the raw editor even if the description fits the form.
    pub raw: bool,
}

/// Somewhere focus can move to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Diff,
    Field(Field),
}

/// Where keyboard focus is: the diff pane, or the editor (the current form
/// field, or the raw message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Diff,
    Editor,
}

/// A one-shot message for the status bar, cleared by the next key press.
enum Status {
    Error(String),
    Info(String),
    /// Quit was requested with unsaved edits; a second quit confirms.
    ConfirmQuit,
}

/// How the event loop ended.
pub enum Exit {
    Abort,
    Commit(String),
}

pub struct App {
    diff: DiffView,
    change_id: String,
    /// One editor per [`Field`], indexed by `field as usize`.
    fields: [TextArea<'static>; Field::ALL.len()],
    /// The form field being edited, or to return to from the diff pane.
    field: Field,
    /// Type or Scope, whichever was left last: where Up from Subject goes.
    top_field: Field,
    /// The whole message, used in [`View::Raw`].
    raw: TextArea<'static>,
    view: View,
    focus: Focus,
    /// The message the session started with, to detect unsaved edits.
    original: String,
    status: Option<Status>,
    keymap: Keymap,
    /// Modal state, used only with [`Keymap::Vim`].
    vim: Vim,
    /// The search being typed after `/` in the diff pane.
    search_prompt: Option<String>,
    zoomed: bool,
}

impl App {
    #[must_use]
    pub fn new(diff: DiffView, session: Session) -> Self {
        let original = session.description.trim_end().to_owned();

        // Open the form only if it can represent the description exactly;
        // otherwise show the text as-is rather than guess.
        let (draft, view, status) = match Draft::parse(&original) {
            Ok(draft) if !session.raw => (draft, View::Structured, None),
            Ok(draft) => (draft, View::Raw, None),
            Err(e) => (
                Draft::default(),
                View::Raw,
                Some(Status::Info(format!(
                    "Opened as raw text because {e}. ^t switches to the form."
                ))),
            ),
        };
        // Vim starts in Insert for a new message, Normal for a revision.
        let mode = if original.is_empty() {
            vim::Mode::Insert
        } else {
            vim::Mode::Normal
        };

        Self {
            diff,
            change_id: session.change_id,
            fields: Field::ALL.map(|field| field_editor(field, draft.get(field))),
            field: Field::Type,
            top_field: Field::Type,
            raw: raw_editor(&original),
            view,
            focus: Focus::Editor,
            original,
            status,
            keymap: session.keymap,
            vim: Vim::new(mode),
            search_prompt: None,
            zoomed: false,
        }
    }

    /// Runs until the user commits or quits.
    /// # Errors
    ///
    /// Returns an [`io::Error`] if drawing the UI frame to the terminal fails, or
    /// if reading the underlying terminal events (such as keystrokes) fails.
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

    /// The message as it stands, valid or not.
    fn message(&self) -> String {
        match self.view {
            View::Structured => self.draft().render(),
            View::Raw => self.raw.lines().join("\n").trim_end().to_owned(),
        }
    }

    fn is_dirty(&self) -> bool {
        self.message() != self.original
    }

    const fn editor(&mut self) -> &mut TextArea<'static> {
        match self.view {
            View::Structured => &mut self.fields[self.field as usize],
            View::Raw => &mut self.raw,
        }
    }

    /// Enter inserts a newline here (rather than moving to the next field).
    const fn editor_is_multiline(&self) -> bool {
        matches!(self.view, View::Raw) || self.field.is_multiline()
    }

    // -----------------------------------------------------------------------
    // Input
    // -----------------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) -> Option<Exit> {
        let confirming = matches!(self.status, Some(Status::ConfirmQuit));
        self.status = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        if self.search_prompt.is_some() {
            return self.search_key(key);
        }

        // Global bindings, wherever focus is.
        match key.code {
            KeyCode::Char('s') if ctrl => return self.submit(),
            KeyCode::Char('c') if ctrl => return self.quit(confirming),
            KeyCode::Char('t') if ctrl => self.toggle_view(),
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            // Read the diff without leaving the field you're typing in.
            KeyCode::PageDown => self.diff.scroll_down(self.diff.half_page()),
            KeyCode::PageUp => self.diff.scroll_up(self.diff.half_page()),
            KeyCode::Down if alt => self.diff.scroll_down(1),
            KeyCode::Up if alt => self.diff.scroll_up(1),
            // Alt-←/→ jump between the diff and the editor from anywhere.
            KeyCode::Left if alt => self.focus_diff(),
            KeyCode::Right if alt => self.focus = Focus::Editor,
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
            KeyCode::Char('j') | KeyCode::Down => self.diff.scroll_down(1),
            KeyCode::Char('k') | KeyCode::Up => self.diff.scroll_up(1),
            KeyCode::Char('d') if ctrl => self.diff.scroll_down(self.diff.half_page()),
            KeyCode::Char('u') if ctrl => self.diff.scroll_up(self.diff.half_page()),
            KeyCode::Char(' ') => self.diff.scroll_down(self.diff.half_page()),
            KeyCode::Char('g') | KeyCode::Home => self.diff.scroll_to_top(),
            KeyCode::Char('G') | KeyCode::End => self.diff.scroll_to_bottom(),
            // h/l move between panes, as before; Shift or the arrows pan.
            KeyCode::Char('L') | KeyCode::Right => self.diff.pan_right(8),
            KeyCode::Char('H') | KeyCode::Left => self.diff.pan_left(8),
            KeyCode::Char('l' | 'i') | KeyCode::Esc | KeyCode::Enter => {
                self.focus = Focus::Editor;
            }
            KeyCode::Char(']') => self.diff.next_file(),
            KeyCode::Char('[') => self.diff.prev_file(),
            KeyCode::Char('}') => self.diff.next_hunk(),
            KeyCode::Char('{') => self.diff.prev_hunk(),
            KeyCode::Char('f') => self.diff.toggle_file_list(),
            KeyCode::Char('/') => self.search_prompt = Some(String::new()),
            KeyCode::Char('n') => self.diff.next_match(),
            KeyCode::Char('N') => self.diff.prev_match(),
            KeyCode::Char('z') => self.zoomed = !self.zoomed,
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
            KeyCode::Char('d') if ctrl => self.diff.scroll_down(self.diff.half_page()),
            KeyCode::Char('u') if ctrl => self.diff.scroll_up(self.diff.half_page()),
            KeyCode::Char('e') if ctrl => self.diff.scroll_down(1),
            KeyCode::Char('y') if ctrl => self.diff.scroll_up(1),
            _ => {
                let form = self.view == View::Structured;
                let editor = match self.view {
                    View::Structured => &mut self.fields[self.field as usize],
                    View::Raw => &mut self.raw,
                };
                match self.vim.handle(&Input::from(key), editor) {
                    vim::Action::Quit => return self.quit(confirming),
                    vim::Action::NextField if form => self.focus_field(self.field.next()),
                    vim::Action::Move(dir) => {
                        self.go(dir);
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn editor_key(&mut self, key: KeyEvent, confirming: bool) -> Option<Exit> {
        let field = self.field;
        let form = self.view == View::Structured;
        let multiline = self.editor_is_multiline();
        // Shift+arrows select text, so only bare arrows leave the field.
        let plain = key.modifiers.is_empty();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let editor = self.editor();
        let (row, col) = (editor.cursor().0, editor.cursor().1);
        let last_row = editor.lines().len() - 1;
        let line_len = editor.lines()[row].chars().count();
        // Arrowing off the edge of a field moves to the field next to it on
        // screen; sideways only between Type and Scope, so holding an arrow
        // key never lands in the diff.
        let field_beyond =
            |app: &Self, dir| form && plain && matches!(app.neighbor(dir), Some(Stop::Field(_)));

        match key.code {
            KeyCode::Esc => return self.quit(confirming),
            // Enter on a one-line field moves on, like a form.
            KeyCode::Enter if !multiline => self.focus_field(field.next()),
            KeyCode::Up if row == 0 && field_beyond(self, Dir::Up) => self.go(Dir::Up),
            KeyCode::Down if row == last_row && field_beyond(self, Dir::Down) => {
                self.go(Dir::Down);
            }
            KeyCode::Left if col == 0 && field_beyond(self, Dir::Left) => self.go(Dir::Left),
            KeyCode::Right
                if row == last_row && col == line_len && field_beyond(self, Dir::Right) =>
            {
                self.go(Dir::Right);
            }
            KeyCode::Char('z') if ctrl => {
                self.editor().undo();
            }
            KeyCode::Char('w') if alt => self.editor().copy(),
            _ => {
                self.editor().input(Input::from(key));
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
        let text = if self.editor_is_multiline() {
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
        if self.view == View::Raw {
            // Raw text is the escape hatch: written exactly as typed.
            return Some(Exit::Commit(self.message()));
        }
        match self.draft().build() {
            Ok(message) => Some(Exit::Commit(message)),
            Err(e) => {
                self.status = Some(Status::Error(e.into()));
                None
            }
        }
    }

    fn focus_diff(&mut self) {
        self.editor().cancel_selection();
        if matches!(self.vim.mode, vim::Mode::Visual | vim::Mode::Operator(_)) {
            self.vim.mode = vim::Mode::Normal;
        }
        self.focus = Focus::Diff;
    }

    /// Switch between the form and the raw text. Going back to the form only
    /// works if it can hold the text exactly.
    fn toggle_view(&mut self) {
        match self.view {
            View::Structured => {
                self.raw = raw_editor(&self.draft().render());
                self.view = View::Raw;
            }
            View::Raw => match Draft::parse(&self.message()) {
                Ok(draft) => {
                    self.fields = Field::ALL.map(|field| field_editor(field, draft.get(field)));
                    self.view = View::Structured;
                }
                Err(e) => {
                    self.status = Some(Status::Error(format!("can't switch to the form: {e}")));
                    return;
                }
            },
        }
        self.focus = Focus::Editor;
        if matches!(self.vim.mode, vim::Mode::Visual | vim::Mode::Operator(_)) {
            self.vim.mode = vim::Mode::Normal;
        }
    }

    /// Tab order: every form field (or the raw editor), then the diff pane,
    /// then around again.
    fn cycle_focus(&mut self, forward: bool) {
        if self.view == View::Raw {
            self.focus = match self.focus {
                Focus::Diff => Focus::Editor,
                Focus::Editor => Focus::Diff,
            };
            return;
        }
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

    /// What lies in `dir` from the current field, as laid out on screen:
    ///
    /// ```text
    ///  diff | Type | Scope
    ///       | Subject
    ///       | Body ... Trailers
    /// ```
    fn neighbor(&self, dir: Dir) -> Option<Stop> {
        use Field::{Scope, Subject, Trailers, Type};
        if self.view == View::Raw {
            return (dir == Dir::Left).then_some(Stop::Diff);
        }
        let field = match (dir, self.field) {
            (Dir::Up, Type | Scope) | (Dir::Down, Trailers) => {
                return None;
            }
            (Dir::Right, _) if self.field != Type => return None,
            (Dir::Left, f) if f != Scope => return Some(Stop::Diff),
            (Dir::Up, Subject) => self.top_field,
            (Dir::Down, Type | Scope) => Subject,
            (Dir::Left, _) => Type,
            (Dir::Right, _) => Scope,
            (Dir::Up, f) => f.prev(),
            (Dir::Down, f) => f.next(),
        };
        Some(Stop::Field(field))
    }

    /// Move focus in `dir`, if there's anything there.
    fn go(&mut self, dir: Dir) {
        match self.neighbor(dir) {
            Some(Stop::Field(field)) => {
                self.focus_field(field);
                // Arrive at the side you came from, so l then h is a no-op.
                let editor = self.editor();
                match dir {
                    Dir::Right => {
                        editor.move_cursor(CursorMove::Top);
                        editor.move_cursor(CursorMove::Head);
                    }
                    Dir::Left => {
                        editor.move_cursor(CursorMove::Bottom);
                        editor.move_cursor(CursorMove::End);
                    }
                    Dir::Up | Dir::Down => {}
                }
            }
            Some(Stop::Diff) => self.focus_diff(),
            None => {}
        }
    }

    fn focus_field(&mut self, field: Field) {
        if matches!(self.field, Field::Type | Field::Scope) {
            self.top_field = self.field;
        }
        // Leave no half-finished selection or operator behind.
        self.editor().cancel_selection();
        if matches!(self.vim.mode, vim::Mode::Visual | vim::Mode::Operator(_)) {
            self.vim.mode = vim::Mode::Normal;
        }
        self.focus = Focus::Editor;
        self.field = field;
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn draw(&mut self, frame: &mut Frame) {
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

        // The pane title is built up in pieces: change ID, then the file at the
        // top (commit 2), then the search (commit 4).
        let mut title = vec![Span::raw(format!(" Diff {} ", self.change_id))];
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
            editor.set_cursor_style(cursor_style(focused));
            frame.render_widget(&*editor, area);
        }
    }

    fn draw_raw(&mut self, frame: &mut Frame, area: Rect) {
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
    fn status_line(&self) -> Line<'static> {
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
        let keys = match (self.focus, self.keymap, self.vim.mode) {
            (Focus::Diff, ..) => {
                " j/k scroll · [/] file · {/} hunk · f files · / search · n/N match · l back"
            }
            (Focus::Editor, Keymap::Default, _) => {
                " Tab next · Enter next/newline · Alt-← diff · PgUp/PgDn scroll · ^t raw/form · ^s save · Esc quit"
            }
            (Focus::Editor, Keymap::Vim, vim::Mode::Insert) => {
                " Esc normal · Tab next · Enter next/newline · ^t raw/form · ^s save"
            }
            (Focus::Editor, Keymap::Vim, vim::Mode::Visual) => " d/c/y operate · Esc cancel",
            (Focus::Editor, Keymap::Vim, _) => {
                " i insert · hjkl past an edge: next box · ^d/^u scroll diff · ^t raw/form · ^s save · q quit"
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

    /// Typing a search: Enter runs it, Esc cancels, Ctrl-c still quits.
    fn search_key(&mut self, key: KeyEvent) -> Option<Exit> {
        let prompt = self.search_prompt.as_mut()?;
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_prompt = None;
                return self.quit(false);
            }
            KeyCode::Char(c) => prompt.push(c),
            KeyCode::Backspace => {
                prompt.pop();
            }
            KeyCode::Enter => {
                let query = self.search_prompt.take().unwrap_or_default();
                if self.diff.search(&query) == 0 && !query.is_empty() {
                    self.status = Some(Status::Error(format!("no match for `{query}`")));
                }
            }
            KeyCode::Esc => self.search_prompt = None,
            _ => {}
        }
        None
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
fn raw_editor(text: &str) -> TextArea<'static> {
    let mut editor = TextArea::new(text.split('\n').map(str::to_owned).collect());
    editor.set_cursor_line_style(Style::new());
    editor.set_wrap_mode(WrapMode::WordOrGlyph);
    editor
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const EXAMPLE: &str = "fix(parser): handle empty input\n\n\
                           This also corrects an unrelated diagnostic.\n\n\
                           Fixes #42\nReviewed-by: Someone\n";

    fn app(description: &str, keymap: Keymap) -> App {
        App::new(
            DiffView::from_output(b"", "diff").unwrap(),
            Session {
                description: description.into(),
                change_id: "abc".into(),
                keymap,
                raw: false,
            },
        )
    }

    fn press(app: &mut App, code: KeyCode) -> Option<Exit> {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(app: &mut App, c: char) -> Option<Exit> {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn committed(exit: Option<Exit>) -> String {
        match exit {
            Some(Exit::Commit(message)) => message,
            _ => panic!("expected a commit"),
        }
    }

    #[test]
    fn types_immediately_and_enter_moves_on() {
        let mut app = app("", Keymap::Default);
        type_str(&mut app, "feat");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "add a thing");
        assert_eq!(committed(ctrl(&mut app, 's')), "feat: add a thing");
    }

    #[test]
    fn opens_existing_description_in_the_form_and_saves_it_unchanged() {
        let mut app = app(EXAMPLE, Keymap::Default);
        assert_eq!(app.view, View::Structured);
        assert!(!app.is_dirty());
        assert_eq!(committed(ctrl(&mut app, 's')), EXAMPLE.trim_end());
    }

    #[test]
    fn edits_mid_line() {
        let mut app = app(EXAMPLE, Keymap::Default);
        app.focus_field(Field::Subject);
        press(&mut app, KeyCode::Home);
        type_str(&mut app, "gracefully ");
        let message = committed(ctrl(&mut app, 's'));
        assert!(message.starts_with("fix(parser): gracefully handle empty input\n"));
        assert!(message.ends_with("Fixes #42\nReviewed-by: Someone"));
    }

    #[test]
    fn unparseable_description_opens_raw_and_saves_verbatim() {
        let mut app = app("WIP\nstuff", Keymap::Default);
        assert_eq!(app.view, View::Raw);
        assert!(matches!(app.status, Some(Status::Info(_))));
        assert_eq!(committed(ctrl(&mut app, 's')), "WIP\nstuff");
    }

    #[test]
    fn raw_view_round_trips_through_the_form() {
        let mut app = app(EXAMPLE, Keymap::Default);
        ctrl(&mut app, 't');
        assert_eq!(app.view, View::Raw);
        assert_eq!(app.message(), EXAMPLE.trim_end());
        ctrl(&mut app, 't');
        assert_eq!(app.view, View::Structured);
        assert!(!app.is_dirty());
    }

    #[test]
    fn refuses_to_switch_to_the_form_when_it_would_lose_text() {
        let mut app = app("WIP", Keymap::Default);
        ctrl(&mut app, 't');
        assert_eq!(app.view, View::Raw);
        assert!(matches!(app.status, Some(Status::Error(_))));
    }

    #[test]
    fn invalid_form_stays_open_with_an_error() {
        let mut app = app("", Keymap::Default);
        type_str(&mut app, "two words");
        assert!(ctrl(&mut app, 's').is_none());
        assert!(matches!(app.status, Some(Status::Error(_))));
    }

    #[test]
    fn quitting_with_edits_asks_first() {
        let mut clean = app(EXAMPLE, Keymap::Default);
        assert!(matches!(press(&mut clean, KeyCode::Esc), Some(Exit::Abort)));

        let mut dirty = app("", Keymap::Default);
        type_str(&mut dirty, "x");
        assert!(press(&mut dirty, KeyCode::Esc).is_none());
        assert!(matches!(press(&mut dirty, KeyCode::Esc), Some(Exit::Abort)));

        // Any other key cancels the prompt.
        assert!(press(&mut dirty, KeyCode::Esc).is_none());
        type_str(&mut dirty, "y");
        assert!(press(&mut dirty, KeyCode::Esc).is_none());
    }

    #[test]
    fn paste_into_a_one_line_field_collapses_newlines() {
        let mut app = app("", Keymap::Default);
        app.focus_field(Field::Subject);
        app.paste("add\r\nthe   thing\n");
        assert_eq!(app.draft().get(Field::Subject), "add the thing");

        app.focus_field(Field::Body);
        app.paste("one\r\ntwo");
        assert_eq!(app.draft().get(Field::Body), "one\ntwo");
    }

    #[test]
    fn tab_cycles_through_fields_and_diff() {
        let mut app = app("", Keymap::Default);
        for _ in Field::ALL {
            press(&mut app, KeyCode::Tab);
        }
        assert_eq!(app.focus, Focus::Diff);
        press(&mut app, KeyCode::Tab);
        assert_eq!((app.focus, app.field), (Focus::Editor, Field::Type));
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.focus, Focus::Diff);
    }

    #[test]
    fn h_and_l_move_between_the_form_and_the_diff() {
        let mut app = app(EXAMPLE, Keymap::Vim);
        // `h` moves the cursor until it reaches the start of the line...
        app.editor().move_cursor(CursorMove::End);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.focus, Focus::Editor);
        app.editor().move_cursor(CursorMove::Head);
        // ...then carries on into the diff; `l` comes back.
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.focus, Focus::Diff);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!((app.focus, app.field), (Focus::Editor, Field::Type));
    }

    #[test]
    fn vim_hjkl_follow_the_layout() {
        let mut app = app("", Keymap::Vim);
        press(&mut app, KeyCode::Esc);
        let mut go = |c| {
            press(&mut app, KeyCode::Char(c));
            (app.focus, app.field)
        };
        let field = |f| (Focus::Editor, f);
        // Type and Scope sit side by side above Subject.
        assert_eq!(go('l'), field(Field::Scope));
        assert_eq!(go('j'), field(Field::Subject));
        assert_eq!(go('k'), field(Field::Scope));
        assert_eq!(go('h'), field(Field::Type));
        assert_eq!(go('j'), field(Field::Subject));
        assert_eq!(go('k'), field(Field::Type));
        assert_eq!(go('k'), field(Field::Type));
        // Left of Type, and of everything below, is the diff.
        assert_eq!(go('h').0, Focus::Diff);
        assert_eq!(go('l'), field(Field::Type));

        // With text in both: l lands at the start of Scope, so h goes
        // straight back.
        let mut app = super::tests::app(EXAMPLE, Keymap::Vim);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.field, Field::Scope);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.field, Field::Type);
        go('j');
        assert_eq!(go('j'), field(Field::Body));
        assert_eq!(go('h').0, Focus::Diff);
    }

    #[test]
    fn arrows_at_field_edges_follow_the_layout() {
        let mut app = app("", Keymap::Default);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.field, Field::Scope);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.field, Field::Subject);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.field, Field::Scope);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.field, Field::Type);
        // Holding Left never wanders into the diff.
        press(&mut app, KeyCode::Left);
        assert_eq!(app.focus, Focus::Editor);
    }

    #[test]
    fn alt_arrows_jump_to_the_diff_and_back() {
        let mut app = app("", Keymap::Default);
        let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
        app.handle_key(alt(KeyCode::Left));
        assert_eq!(app.focus, Focus::Diff);
        app.handle_key(alt(KeyCode::Right));
        assert_eq!(app.focus, Focus::Editor);
    }

    #[test]
    fn vim_starts_in_insert_for_new_and_normal_for_existing() {
        assert_eq!(app("", Keymap::Vim).vim.mode, vim::Mode::Insert);
        assert_eq!(app(EXAMPLE, Keymap::Vim).vim.mode, vim::Mode::Normal);
    }

    #[test]
    fn vim_j_walks_fields_and_q_quits() {
        let mut app = app(EXAMPLE, Keymap::Vim);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.field, Field::Subject);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.field, Field::Body);
        assert!(matches!(
            press(&mut app, KeyCode::Char('q')),
            Some(Exit::Abort)
        ));
    }

    #[test]
    fn search_prompt_types_runs_and_cancels() {
        let mut app = app(EXAMPLE, Keymap::Default);
        app.focus_diff();
        press(&mut app, KeyCode::Char('/'));
        type_str(&mut app, "zzz");
        // While typing, keys go to the prompt, not to Tab or the form.
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.search_prompt.as_deref(), Some("zzz"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.search_prompt, None);
        assert!(matches!(app.status, Some(Status::Error(_))));

        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.search_prompt, None);
        assert_eq!(app.focus, Focus::Diff);
    }

    #[test]
    fn zoom_ends_when_focus_leaves_the_diff() {
        let mut app = app("", Keymap::Default);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
        app.focus_diff();
        press(&mut app, KeyCode::Char('z'));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(app.zoomed);
        press(&mut app, KeyCode::Char('l'));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(!app.zoomed);
    }
}
