//! Application state, input handling and rendering.

mod effects;
mod focus;
mod split;
mod ui;

use std::io;

use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
};
use ratatui_textarea::{CursorMove, Input, TextArea};

use crate::{
    app::focus::Stop,
    diff::DiffView,
    vim::{self, Dir, Vim},
};

use jj_compose_core::{Draft, Field, Jj, Split, TaggedCommit, Target};

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
    /// Tool `o`/`O` opens the diff in; `None` disables them, with a hint.
    pub diff_tool: Option<String>,
    /// Tags a rewrite of this revision would strand. Empty unless the
    /// revision is immutable and `--ignore-immutable` was given.
    pub at_risk: Vec<TaggedCommit>,
}

/// What a session ended with.
pub struct Outcome {
    pub exit: Exit,
    /// The revision the message is for, which a split moves on.
    pub target: Target,
    /// Commands to run because rewriting immutable history stranded tags.
    pub recovery: Vec<String>,
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

/// Something a key press asks for that needs the real terminal and `jj`,
/// so the event loop does it rather than the key handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Show the diff outside the TUI: one file, or the whole change.
    OpenDiff { path: Option<String> },
    /// Commit the hunks chosen in split mode, described by `message`.
    Split { message: String },
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
    diff_tool: Option<String>,
    /// Set by a key press, carried out by [`App::run`] before the next draw.
    pending: Option<Request>,
    /// The revision being described, which a split replaces.
    target: Target,
    /// The split being composed, if any.
    split: Option<Split>,
    /// Tags that a rewrite would strand, and the commands to put them back.
    at_risk: Vec<TaggedCommit>,
    recovery: Vec<String>,
    is_vertical: bool,
}

impl App {
    #[must_use]
    pub fn new(diff: DiffView, session: Session, target: Target) -> Self {
        let original = session.description.trim_end().to_owned();

        // Open the form only if it can represent the description exactly;
        // otherwise show the text as-is rather than guess.
        let (draft, view, status) = match Draft::parse(&original) {
            Ok(draft) if !session.raw => (draft, View::Structured, None),
            Ok(draft) => (draft, View::Raw, None), // ← this one
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
            fields: Field::ALL.map(|field| ui::field_editor(field, draft.get(field))),
            field: Field::Type,
            top_field: Field::Type,
            raw: ui::raw_editor(&original),
            view,
            focus: Focus::Editor,
            original,
            status,
            keymap: session.keymap,
            vim: Vim::new(mode),
            search_prompt: None,
            zoomed: false,
            diff_tool: session.diff_tool,
            pending: None,
            target,
            split: None,
            at_risk: session.at_risk,
            recovery: Vec::new(),
            is_vertical: false,
        }
    }

    /// Runs until the user commits or quits.
    /// # Errors
    ///
    /// Returns an [`io::Error`] if drawing the UI frame to the terminal fails, or
    /// if reading the underlying terminal events (such as keystrokes) fails.
    pub fn run(mut self, terminal: &mut DefaultTerminal, jj: &Jj) -> io::Result<Outcome> {
        const VERTICAL_BREAKPOINT: u16 = 120;

        // Said before any work is done, not after it is wasted.
        self.status = self.immutable_warning().or_else(|| self.status.take());
        loop {
            let size = terminal.size()?;
            self.is_vertical = size.width < VERTICAL_BREAKPOINT;

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
                // The target changes when a split happens, so the caller is
                // told which revision the message is for.
                return Ok(Outcome {
                    exit,
                    target: self.target,
                    recovery: self.recovery,
                });
            }
            if let Some(request) = self.pending.take() {
                self.perform(terminal, jj, &request)?;
            }
        }
    }

    /// Whether hunks are being chosen right now: a split is under way, it
    ///
    /// The pane, the status hint and the key routing all ask this, so none of
    /// them can show one mode while another acts on a different one.
    fn choosing(&self) -> bool {
        self.focus == Focus::Diff && self.split.is_some()
    }

    /// Show `split`'s current hunk and hand it the keys.
    fn enter_split(&mut self, split: Split) {
        self.diff.show_hunk(split.at());
        self.split = Some(split);
        self.focus = Focus::Diff;
        self.zoomed = true;
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
            KeyCode::Char('s') if alt => self.start_split(),
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
                    // Split mode takes over the diff pane's keys.
                    Focus::Diff if self.choosing() => self.split_key(key.code, ctrl),
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
            // The file at the top of the pane, or the whole change.
            KeyCode::Char('o') => {
                let path = self.diff.current_file().map(|(_, file)| file.path.clone());
                self.pending = Some(Request::OpenDiff { path });
            }
            KeyCode::Char('O') => self.pending = Some(Request::OpenDiff { path: None }),
            KeyCode::Char('s') => self.start_split(),
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
        // Raw text is the escape hatch: written exactly as typed.
        let message = if self.view == View::Raw {
            self.message()
        } else {
            match self.draft().build() {
                Ok(message) => message,
                Err(e) => {
                    self.status = Some(Status::Error(e.into()));
                    return None;
                }
            }
        };
        if let Some(split) = &self.split {
            if split.count() == 0 {
                self.status = Some(Status::Error(
                    "no hunks added: y adds the one on screen, Esc leaves split mode".into(),
                ));
                return None;
            }
            // Adding every hunk describes the change instead of splitting
            // it, which is what jj would do anyway with nothing left over.
            if !split.is_everything() {
                self.pending = Some(Request::Split { message });
                return None;
            }
        }
        Some(Exit::Commit(message))
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

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

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_owned()
    } else {
        format!("{word}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;

    use crate::app::ui;

    const EXAMPLE: &str = "fix(parser): handle empty input\n\n\
                           This also corrects an unrelated diagnostic.\n\n\
                           Fixes #42\nReviewed-by: Someone\n";

    fn app(description: &str, keymap: Keymap) -> App {
        app_with_tool(description, keymap, None)
    }

    fn app_with_tool(description: &str, keymap: Keymap, diff_tool: Option<&str>) -> App {
        App::new(
            DiffView::from_output(b"", "diff").unwrap(),
            Session {
                description: description.into(),
                change_id: "abc".into(),
                keymap,
                raw: false,
                diff_tool: diff_tool.map(str::to_owned),
                at_risk: Vec::new(),
            },
            Target {
                commit_id: "0123456789abcdef".into(),
                change: "zzzzzzzzzzzzzzzz".into(),
                change_id: "zzzzzzzz".into(),
                immutable: false,
            },
        )
    }

    fn press(app: &mut App, code: KeyCode) -> Option<Exit> {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn alt(app: &mut App, c: char) -> Option<Exit> {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT))
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
    fn o_asks_for_the_file_on_top_and_shift_o_for_the_whole_change() {
        const DIFF: &[u8] = b"diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let mut app = app("", Keymap::Default);
        app.diff = DiffView::from_output(DIFF, "").unwrap();

        // In a field, `o` is just a letter.
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(app.pending, None);

        app.focus_diff();
        press(&mut app, KeyCode::Char('o'));
        let path = Some("a.rs".to_owned());
        assert_eq!(app.pending.take(), Some(Request::OpenDiff { path }));
        press(&mut app, KeyCode::Char('O'));
        assert_eq!(app.pending, Some(Request::OpenDiff { path: None }));
    }

    /// Two files, three hunks: enough to split more than one way.
    const THREE_HUNKS: &[u8] = b"\
diff --git a/a.rs b/a.rs
index 1111111..2222222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-one
+ONE
@@ -5,1 +5,1 @@
-five
+FIVE
diff --git a/b.rs b/b.rs
index 3333333..4444444 100644
--- a/b.rs
+++ b/b.rs
@@ -1,1 +1,1 @@
-x
+X
";

    fn splitting_app() -> App {
        let mut app = app("", Keymap::Default);
        app.diff = DiffView::from_output(THREE_HUNKS, "").unwrap();
        app
    }

    #[test]
    fn alt_s_starts_choosing_hunks_from_the_first() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        let split = app.split.as_ref().expect("split mode");
        assert_eq!(split.total(), 3);
        assert_eq!(split.at(), 0);
        assert_eq!(app.focus, Focus::Diff);
        assert!(app.zoomed, "one hunk fills the pane");
    }

    /// `Ctrl-x` is the text editor's cut; splitting must not steal it.
    #[test]
    fn ctrl_x_still_belongs_to_the_editor() {
        let mut app = splitting_app();
        ctrl(&mut app, 'x');
        assert!(app.split.is_none());
    }

    #[test]
    fn a_change_with_one_hunk_cannot_be_split() {
        let one = b"diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let mut app = app("", Keymap::Default);
        app.diff = DiffView::from_output(one, "").unwrap();
        alt(&mut app, 's');
        assert!(app.split.is_none());
        assert!(matches!(app.status, Some(Status::Error(_))));
    }

    /// A split left mid-decision must keep the keys with the diff pane; the
    /// reload after a commit used to hand them back to the form while the
    /// pane still showed a hunk, so y/n went into the Type field as text.
    #[test]
    fn a_split_in_progress_always_has_the_keys() {
        let mut app = splitting_app();
        app.enter_split(
            Split::new(app.diff.hunks().iter().map(|h| (h.id, h.path.clone()))).unwrap(),
        );
        assert_eq!(app.focus, Focus::Diff);
        assert!(app.choosing());

        press(&mut app, KeyCode::Char('y'));
        assert_eq!(
            app.split.as_ref().unwrap().at(),
            1,
            "y decided a hunk rather than being typed"
        );
        assert_eq!(app.fields[Field::Type as usize].lines()[0], "");
    }

    /// Tabbing out to the form mid-decision is allowed, and then the pane and
    /// the keys agree that the form has them.
    #[test]
    fn leaving_the_pane_mid_split_gives_the_form_the_keys() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Editor);
        assert!(!app.choosing(), "the hunk view must not be on screen");

        // Back in the pane, the same split carries on where it left off.
        app.focus_diff();
        assert!(app.choosing());
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.split.as_ref().unwrap().at(), 1);
    }

    #[test]
    fn enter_goes_to_the_message_without_seeing_every_hunk() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Editor);
        assert!(!app.zoomed);
        assert!(!app.choosing());
        assert_eq!(
            app.split.as_ref().unwrap().count(),
            1,
            "the marks are still held"
        );
    }

    #[test]
    fn esc_leaves_split_mode_without_choosing() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        press(&mut app, KeyCode::Esc);
        assert!(app.split.is_none());
        assert_eq!(app.focus, Focus::Editor);
    }

    #[test]
    fn saving_with_no_hunks_marked_explains_itself_instead() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "feat");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_str(&mut app, "something");
        assert!(ctrl(&mut app, 's').is_none());
        assert!(app.pending.is_none(), "no split was requested");
        assert!(matches!(app.status, Some(Status::Error(_))));
    }

    #[test]
    fn saving_some_marked_hunks_asks_for_a_split() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "feat");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_str(&mut app, "only the first hunk");
        assert!(ctrl(&mut app, 's').is_none(), "the app stays open");
        let Some(Request::Split { message }) = app.pending.take() else {
            panic!("expected a split request");
        };
        assert!(
            message.starts_with("feat: only the first hunk"),
            "{message}"
        );
    }

    /// Marking everything leaves nothing behind, so it is a description.
    #[test]
    fn marking_every_hunk_just_describes_the_change() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        press(&mut app, KeyCode::Char('A'));
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "feat");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        type_str(&mut app, "all of it");
        let exit = ctrl(&mut app, 's');
        assert!(matches!(exit, Some(Exit::Commit(_))));
        assert!(
            app.pending.is_none(),
            "splitting off everything is a describe"
        );
    }

    #[test]
    fn the_marked_hunks_make_a_patch_of_only_themselves() {
        let mut app = splitting_app();
        alt(&mut app, 's');
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Char(' '));
        let picked = app.split.as_ref().unwrap().picked();
        let text = app.diff.patch().subset(&picked);
        assert!(text.contains("+FIVE"), "{text}");
        assert!(!text.contains("+ONE"), "{text}");
        assert!(!text.contains("b.rs"), "{text}");
    }

    /// Rewriting protected history is flagged before any work is done, and
    /// the flag names the tags that would be stranded.
    #[test]
    fn an_immutable_revision_warns_and_names_the_tags_at_risk() {
        let mut app = splitting_app();
        assert!(app.immutable_warning().is_none(), "mutable: nothing to say");

        app.target.immutable = true;
        assert!(matches!(app.immutable_warning(), Some(Status::Error(_))));

        app.at_risk = vec![TaggedCommit {
            tag: "v0.1.0".to_owned(),
            commit: "aca8d2e2".to_owned(),
            summary: "feat: three things at once".to_owned(),
        }];
        let Some(Status::Error(text)) = app.immutable_warning() else {
            panic!("expected an error status");
        };
        assert!(text.contains("v0.1.0"), "{text}");
        assert!(text.contains("1 tag"), "{text}");
    }

    #[test]
    fn the_immutable_marker_shows_only_on_protected_history() {
        assert_eq!(ui::immutable_marker(false).content, "");
        assert!(ui::immutable_marker(true).content.contains("immutable"));
    }

    #[test]
    fn the_diff_hint_names_the_configured_tool() {
        let hint = |tool| {
            let mut app = app_with_tool("", Keymap::Default, tool);
            app.focus_diff();
            app.status_line()
                .spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert!(
            hint(Some("meld")).contains("o/O meld"),
            "{}",
            hint(Some("meld"))
        );
        assert!(hint(None).contains("o/O open"));
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
