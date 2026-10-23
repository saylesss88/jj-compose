use std::{error::Error, io, process::Command};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Paragraph},
};

fn main() -> Result<(), Box<dyn Error>> {
    // Fail before touching the terminal: if we can't diff, we can't describe either.
    let diff = load_diff()?;

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

fn load_diff() -> Result<String, Box<dyn Error>> {
    let output = Command::new("jj")
        .args(["diff", "-r", "@", "--git"])
        .output()
        .map_err(|e| format!("failed to run jj: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }

    let diff = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(if diff.trim().is_empty() {
        "(no changes in @)".to_owned()
    } else {
        diff
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

    /// `type(scope): subject`
    fn header(&self) -> String {
        let ty = self.get(Field::Type).trim();
        let scope = self.get(Field::Scope).trim();
        let subject = self.get(Field::Subject).trim();

        if scope.is_empty() {
            format!("{ty}: {subject}")
        } else {
            format!("{ty}({scope}): {subject}")
        }
    }

    /// The header plus the optional Problem/Solution body.
    fn build(&self) -> String {
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
        message
    }
}

// ---------------------------------------------------------------------------
// Application state
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
    diff: String,
    diff_scroll: u16,
    draft: Draft,
    focus: Focus,
    mode: Mode,
}

impl App {
    fn new(diff: String) -> Self {
        Self {
            diff,
            diff_scroll: 0,
            draft: Draft::default(),
            focus: Focus::Field(Field::Type),
            mode: Mode::Normal,
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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('s') {
            return Some(Exit::Commit(self.draft.build()));
        }

        match self.mode {
            Mode::Normal => return self.normal_key(key.code),
            Mode::Insert => self.insert_key(key.code),
        }
        None
    }

    fn normal_key(&mut self, code: KeyCode) -> Option<Exit> {
        match (code, self.focus) {
            (KeyCode::Char('q'), _) => return Some(Exit::Abort),
            (KeyCode::Char('i'), _) => self.mode = Mode::Insert,

            (KeyCode::Char('h'), _) => self.focus = Focus::Diff,
            (KeyCode::Char('l'), _) => self.focus = Focus::Field(Field::Type),

            (KeyCode::Char('j') | KeyCode::Down, Focus::Diff) => {
                self.diff_scroll = self.diff_scroll.saturating_add(1);
            }
            (KeyCode::Char('k') | KeyCode::Up, Focus::Diff) => {
                self.diff_scroll = self.diff_scroll.saturating_sub(1);
            }
            (KeyCode::Char('j') | KeyCode::Down, Focus::Field(f)) => {
                self.focus = Focus::Field(f.next());
            }
            (KeyCode::Char('k') | KeyCode::Up, Focus::Field(f)) => {
                self.focus = Focus::Field(f.prev());
            }
            _ => {}
        }
        None
    }

    fn insert_key(&mut self, code: KeyCode) {
        // You can't type into the diff pane; Esc still leaves Insert.
        let Focus::Field(field) = self.focus else {
            if code == KeyCode::Esc {
                self.mode = Mode::Normal;
            }
            return;
        };
        let buf = self.draft.get_mut(field);

        match code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char(c) => buf.push(c),
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Enter if field.is_multiline() => buf.push('\n'),
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn draw(&self, frame: &mut Frame) {
        let [diff_area, form_area] =
            Layout::horizontal([Constraint::Percentage(50); 2]).areas(frame.area());

        let diff = Paragraph::new(self.diff.as_str())
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
            let title = format!(" {} ", field.title());
            let input =
                Paragraph::new(self.draft.get(field)).block(self.pane(title, Focus::Field(field)));
            frame.render_widget(input, area);
        }
    }

    /// A bordered box, green when `target` has focus and gray otherwise.
    fn pane<'a>(&self, title: impl Into<Line<'a>>, target: Focus) -> Block<'a> {
        let color = if self.focus == target {
            Color::Green
        } else {
            Color::DarkGray
        };
        Block::bordered().title(title).style(Style::new().fg(color))
    }
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
    fn formats_header_and_body() {
        let d = draft(" feat ", "ui", "add diff pane", "No context.\n", "");
        assert_eq!(
            d.build(),
            "feat(ui): add diff pane\n\nProblem:\nNo context."
        );
    }

    #[test]
    fn omits_empty_scope() {
        let d = draft("fix", "", "handle empty repo", "", "");
        assert_eq!(d.build(), "fix: handle empty repo");
    }
}
