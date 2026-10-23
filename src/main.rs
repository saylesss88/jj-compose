use std::{error::Error, io, process::Command};

use ansi_to_tui::IntoText;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph},
};

/// Header length (`type(scope): subject`) most style guides aim for.
const HEADER_SOFT_LIMIT: usize = 50;
/// Header length past which forges and `git log --oneline` start truncating.
const HEADER_HARD_LIMIT: usize = 72;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffFormat {
    /// `jj diff --color=always`: whatever format the user configured in jj
    /// (color-words by default), ANSI-parsed into styled text.
    Jj,
    /// `jj diff --git`: plain unified diff, colored line-by-line here.
    Git,
}

fn load_diff(format: DiffFormat) -> Result<Text<'static>, Box<dyn Error>> {
    let args: &[&str] = match format {
        DiffFormat::Jj => &["diff", "-r", "@", "--color=always"],
        DiffFormat::Git => &["diff", "-r", "@", "--git", "--color=never"],
    };
    let output = Command::new("jj")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run jj: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }

    if output.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Text::raw("(no changes in @)"));
    }

    Ok(match format {
        // Turn the ANSI escape codes into ratatui styles instead of
        // printing them as literal `[38;5;2m` garbage.
        DiffFormat::Jj => output.stdout.into_text()?,
        DiffFormat::Git => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(diff_line)
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Commit message model
// ---------------------------------------------------------------------------

/// One input box in the form, in on-screen order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Type,
    Scope,
    Subject,
    Problem,
    Solution,
}

impl Field {
    const ALL: [Self; 5] = [
        Self::Type,
        Self::Scope,
        Self::Subject,
        Self::Problem,
        Self::Solution,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::Scope => "Scope",
            Self::Subject => "Subject",
            Self::Problem => "Problem",
            Self::Solution => "Solution",
        }
    }

    const fn is_multiline(self) -> bool {
        matches!(self, Self::Problem | Self::Solution)
    }

    /// The field below this one (stays put on the last field).
    fn next(self) -> Self {
        Self::ALL[(self as usize + 1).min(Self::ALL.len() - 1)]
    }

    /// The field above this one (stays put on the first field).
    const fn prev(self) -> Self {
        Self::ALL[(self as usize).saturating_sub(1)]
    }
}

/// The text the user has typed so far, one buffer per [`Field`].
#[derive(Debug, Default)]
struct Draft {
    inputs: [String; Field::ALL.len()],
}

impl Draft {
    fn get(&self, field: Field) -> &str {
        &self.inputs[field as usize]
    }

    const fn get_mut(&mut self, field: Field) -> &mut String {
        &mut self.inputs[field as usize]
    }

    /// `type(scope)!: subject`. A trailing `!` on the type marks a breaking
    /// change and is moved after the scope, per the Conventional Commits spec.
    fn header(&self) -> String {
        let ty = self.get(Field::Type).trim();
        let (ty, bang) = ty
            .strip_suffix('!')
            .map_or((ty, ""), |ty| (ty.trim_end(), "!"));
        let scope = self.get(Field::Scope).trim();
        let subject = self.get(Field::Subject).trim();

        if scope.is_empty() {
            format!("{ty}{bang}: {subject}")
        } else {
            format!("{ty}({scope}){bang}: {subject}")
        }
    }

    /// The header plus the optional Problem/Solution body, or the first
    /// reason the header isn't a valid Conventional Commit
    fn build(&self) -> Result<String, &'static str> {
        let ty = self.get(Field::Type).trim().trim_end_matches('!');
        if ty.is_empty() {
            return Err("type is required (feat, fix, refactor, ...)");
        }
        if !ty.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("type must be a single word, e.g. feat or fix");
        }

        let scope = self.get(Field::Scope).trim();
        if scope.contains(|c: char| c.is_whitespace() || c == '(' || c == ')') {
            return Err("scope must not contain spaces or parentheses");
        }

        if self.get(Field::Subject).trim().is_empty() {
            return Err("subject is required");
        }

        let mut message = self.header();
        for field in [Field::Problem, Field::Solution] {
            let body = self.get(field).trim();
            if !body.is_empty() {
                message.push_str("\n\n");
                message.push_str(field.title());
                message.push_str(":\n");
                message.push_str(body);
            }
        }
        Ok(message)
    }
}

// ---------------------------------------------------------------------------
// Application state + input handling
// ---------------------------------------------------------------------------

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

struct App {
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
    fn new(diff: Text<'static>) -> Self {
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
    fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<Option<String>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(ty: &str, scope: &str, subject: &str, problem: &str, solution: &str) -> Draft {
        Draft {
            inputs: [ty, scope, subject, problem, solution].map(str::to_owned),
        }
    }

    #[test]
    fn header_only() {
        let d = draft("fix", "", "handle empty repo", "", "");
        assert_eq!(d.build().unwrap(), "fix: handle empty repo");
    }

    #[test]
    fn full_message() {
        let d = draft(
            " feat ",
            "ui",
            "add diff pane",
            "No context.\n",
            "Show `jj diff`.",
        );
        assert_eq!(
            d.build().unwrap(),
            "feat(ui): add diff pane\n\nProblem:\nNo context.\n\nSolution:\nShow `jj diff`."
        );
    }

    #[test]
    fn breaking_change_bang_goes_after_scope() {
        let d = draft("feat!", "api", "drop v1 endpoints", "", "");
        assert_eq!(d.build().unwrap(), "feat(api)!: drop v1 endpoints");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(draft("", "", "x", "", "").build().is_err());
        assert!(draft("feat", "", " ", "", "").build().is_err());
        assert!(draft("new feature", "", "x", "", "").build().is_err());
        assert!(draft("feat", "a b", "x", "", "").build().is_err());
    }
}
