//! Application state, input handling and rendering.

use std::io;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph},
};

use crate::message::{Draft, Field, HEADER_HARD_LIMIT, HEADER_SOFT_LIMIT};

/// Where keyboard focus is: the diff pane, or one of the form fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Diff,
    Field(Field),
}

/// Navigating (Normal) or typing (Insert).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Insert,
}

/// How the event loop ended.
enum Exit {
    Abort,
    Commit(String),
}

pub struct App {
    diff: Text<'static>,
    diff_len: u16,
    diff_scroll: u16,
    /// Rows visible inside the diff pane, updated on every draw.
    diff_height: u16,
    draft: Draft,
    focus: Focus,
    /// Form field to return to when leaving the diff pane.
    last_field: Field,
    mode: Mode,
    /// Shown in the status bar until the next key press
    error: Option<&'static str>,
}

impl App {
    pub fn new(diff: Text<'static>) -> Self {
        let diff_len = u16::try_from(diff.lines.len()).unwrap_or(u16::MAX);
        Self {
            diff,
            diff_len,
            diff_scroll: 0,
            diff_height: 0,
            draft: Draft::default(),
            focus: Focus::Field(Field::Type),
            last_field: Field::Type,
            mode: Mode::Normal,
            error: None,
        }
    }
    /// Returns the finished commit message, or `None` if the user quit.
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<Option<String>> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

            // Ignore key *release* events (reported on Windows and with the
            // kitty keyboard protocol), or every keystroke registers twice.
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && let Some(exit) = self.handle_key(key)
            {
                return Ok(match exit {
                    Exit::Abort => None,
                    Exit::Commit(message) => Some(message),
                });
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Exit> {
        self.error = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Global bindings, active in both modes.
        match key.code {
            KeyCode::Char('s') if ctrl => return self.submit(),
            KeyCode::Char('c') if ctrl => return Some(Exit::Abort),
            KeyCode::Tab => {
                self.focus_field(self.current_field().next());
                return None;
            }
            KeyCode::BackTab => {
                self.focus_field(self.current_field().prev());
                return None;
            }
            _ => {}
        }

        match self.mode {
            Mode::Normal => return self.normal_key(key.code, ctrl),
            Mode::Insert => self.insert_key(key.code, ctrl),
        }
        None
    }

    fn normal_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Exit> {
        match (code, self.focus) {
            (KeyCode::Char('q'), _) => return Some(Exit::Abort),

            (KeyCode::Char('i' | 'a') | KeyCode::Enter, Focus::Field(_)) => {
                self.mode = Mode::Insert;
            }

            (KeyCode::Char('h') | KeyCode::Left, _) => self.focus = Focus::Diff,
            (KeyCode::Char('l') | KeyCode::Right, _) => self.focus_field(self.last_field),

            (KeyCode::Char('j') | KeyCode::Down, Focus::Diff) => self.scroll_down(1),
            (KeyCode::Char('k') | KeyCode::Up, Focus::Diff) => self.scroll_up(1),
            (KeyCode::Char('j') | KeyCode::Down, Focus::Field(f)) => self.focus_field(f.next()),
            (KeyCode::Char('k') | KeyCode::Up, Focus::Field(f)) => self.focus_field(f.prev()),

            // Diff scrolling works from anywhere so you can read while writing.
            (KeyCode::Char('d'), _) if ctrl => self.scroll_down(self.diff_height / 2),
            (KeyCode::Char('u'), _) if ctrl => self.scroll_up(self.diff_height / 2),
            (KeyCode::Char('g'), _) => self.diff_scroll = 0,
            (KeyCode::Char('G'), _) => self.diff_scroll = self.max_scroll(),
            _ => {}
        }
        None
    }

    fn insert_key(&mut self, code: KeyCode, ctrl: bool) {
        // You can't type into the diff pane; Esc still leaves Insert.
        let Focus::Field(field) = self.focus else {
            self.mode = Mode::Normal;
            return;
        };
        let buf = self.draft.get_mut(field);

        match code {
            KeyCode::Esc => self.mode = Mode::Normal,
            // Ctrl+<letter> is a command, not text.
            KeyCode::Char(c) if !ctrl => buf.push(c),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Enter if field.is_multiline() => buf.push('\n'),
            // Enter on a one-line field moves on, like a form.
            KeyCode::Enter => self.focus_field(field.next()),
            _ => {}
        }
    }

    /// Finish with a valid message, or stay open and show why not.
    fn submit(&mut self) -> Option<Exit> {
        match self.draft.build() {
            Ok(message) => Some(Exit::Commit(message)),
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// The field that Tab moves from, or that `l` returns to.
    const fn current_field(&self) -> Field {
        match self.focus {
            Focus::Field(f) => f,
            Focus::Diff => self.last_field,
        }
    }

    const fn focus_field(&mut self, field: Field) {
        self.focus = Focus::Field(field);
        self.last_field = field;
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
        let [diff_area, form_area] =
            Layout::horizontal([Constraint::Percentage(50); 2]).areas(main);

        // Remember the viewport height for half-page scrolling and clamping.
        self.diff_height = diff_area.height.saturating_sub(2);

        let diff = Paragraph::new(self.diff.clone())
            .block(self.pane(" Diff (@) ", Focus::Diff))
            .scroll((self.diff_scroll, 0));
        frame.render_widget(diff, diff_area);

        let field_areas: [Rect; 5] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ])
        .areas(form_area);

        for (field, area) in Field::ALL.into_iter().zip(field_areas) {
            let text = self.draft.get(field);
            let input =
                Paragraph::new(text).block(self.pane(self.field_title(field), Focus::Field(field)));
            frame.render_widget(input, area);

            // ratatui hides the cursor unless a frame places it.
            if self.mode == Mode::Insert && self.focus == Focus::Field(field) {
                frame.set_cursor_position(cursor_position(area, text));
            }
        }
        frame.render_widget(self.status_line(), status);
    }

    fn pane<'a>(&self, title: impl Into<Line<'a>>, target: Focus) -> Block<'a> {
        let style = match (self.focus == target, self.mode) {
            (false, _) => Style::new().fg(Color::DarkGray),
            (true, Mode::Normal) => Style::new().fg(Color::Green),
            (true, Mode::Insert) => Style::new().fg(Color::Yellow),
        };
        Block::bordered()
            .title(title)
            .border_style(style)
            .title_style(style)
    }

    /// The Subject title shows a live length counter for the whole header line.
    fn field_title(&self, field: Field) -> Line<'static> {
        let title = format!(" {} ", field.title());
        if field != Field::Subject {
            return Line::from(title);
        }

        let len = self.draft.header().chars().count();
        let color = match len {
            0..=HEADER_SOFT_LIMIT => Color::Green,
            n if n <= HEADER_HARD_LIMIT => Color::Yellow,
            _ => Color::Red,
        };
        Line::from(vec![
            Span::raw(title),
            Span::styled(
                format!("{len}/{HEADER_HARD_LIMIT} "),
                Style::new().fg(color),
            ),
        ])
    }

    /// Bottom row: the current mode and the keys that work in it.
    fn status_line(&self) -> Line<'static> {
        if let Some(err) = self.error {
            return Line::from(format!(" ✗ {err}")).red().bold();
        }
        let (mode, keys) = match self.mode {
            Mode::Normal => (
                Span::raw(" NORMAL ").black().on_green(),
                " i edit · j/k move · h/l diff/form · ^d/^u g/G scroll · Tab next · ^s commit · q quit",
            ),
            Mode::Insert => (
                Span::raw(" INSERT ").black().on_yellow(),
                " Esc normal · Tab next field · Enter newline/next · ^s commit",
            ),
        };
        Line::from(vec![mode, Span::raw(keys).dark_gray()])
    }
}

/// Cursor at the end of `text` inside a bordered `area`, clamped to the box.
fn cursor_position(area: Rect, text: &str) -> Position {
    let row = text.matches('\n').count();
    let last_line = text.rsplit('\n').next().unwrap_or_default();
    let col = Line::raw(last_line).width();

    // +1 skips the left/top border; the min() keeps it inside the box.
    let clamp = |offset: usize, start: u16, end: u16| {
        let offset = u16::try_from(offset).unwrap_or(u16::MAX);
        start
            .saturating_add(1)
            .saturating_add(offset)
            .min(end.saturating_sub(2))
    };
    Position::new(
        clamp(col, area.x, area.right()),
        clamp(row, area.y, area.bottom()),
    )
}
