use std::{error::Error, process::Command};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Paragraph},
};

// Track where the user's cursor currently is
#[derive(Clone, Copy)]
pub enum ActivePane {
    Type,       // e.g., feat, fix, chore
    Scope,      // e.g., ui, backend
    Subject,    // The short summary
    Problem,    // The "Problem:" text block
    Solution,   // The "Solution:" text block
    DiffViewer, // Scrolling through the diff
}

// Track if they are navigating or typing
pub enum InputMode {
    Normal, // H/J/K/L to move between panes or scroll diff
    Insert, // Typing actual commit text
}

fn main() -> Result<(), Box<dyn Error>> {
    // Fail before touching the terminal: if we can't diff, we can't describe either.
    let diff = load_diff()?;

    let mut terminal = ratatui::init();

    // Initialize state
    let mut app = App::new(diff);
    // Run application loop
    let result = run_app(&mut terminal, &mut app);

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
}

fn run_app(
    terminal: &mut DefaultTerminal,
    app: &mut App,
) -> Result<Option<String>, Box<dyn Error>> {
    loop {
        // Draw the UI
        terminal.draw(|f| ui(f, app))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // Submit Block
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('s') {
            // Hand the message back to main; it runs jj describe.
            return Ok(Some(app.draft.build()));
        }

        match app.mode {
            Mode::Normal => match (key.code, app.focus) {
                (KeyCode::Char('q'), _) => return Ok(None),
                (KeyCode::Char('i'), _) => app.mode = Mode::Insert,

                (KeyCode::Char('h'), _) => app.focus = Focus::Diff,
                (KeyCode::Char('l'), _) => app.focus = Focus::Field(Field::Type),

                (KeyCode::Char('j') | KeyCode::Down, Focus::Diff) => {
                    app.diff_scroll = app.diff_scroll.saturating_add(1);
                }
                (KeyCode::Char('k') | KeyCode::Up, Focus::Diff) => {
                    app.diff_scroll = app.diff_scroll.saturating_sub(1);
                }
                (KeyCode::Char('j') | KeyCode::Down, Focus::Field(f)) => {
                    app.focus = Focus::Field(f.next());
                }
                (KeyCode::Char('k') | KeyCode::Up, Focus::Field(f)) => {
                    app.focus = Focus::Field(f.prev());
                }
                _ => {}
            },
            Mode::Insert => {
                // You can't type into the diff pane; Esc still leaves Insert.
                let Focus::Field(field) = app.focus else {
                    if key.code == KeyCode::Esc {
                        app.mode = Mode::Normal;
                    }
                    continue;
                };
                let buf = app.draft.get_mut(field);

                match key.code {
                    KeyCode::Esc => app.mode = Mode::Normal,
                    KeyCode::Char(c) => buf.push(c),
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Enter if field.is_multiline() => buf.push('\n'),
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

// The Rendering Function
fn ui(f: &mut Frame, app: &App) {
    let [diff_area, form_area] =
        Layout::horizontal([Constraint::Percentage(50); 2]).areas(f.area());

    let diff = Paragraph::new(app.diff.as_str())
        .block(pane(" Diff (@) ", app.focus == Focus::Diff))
        .scroll((app.diff_scroll, 0));
    f.render_widget(diff, diff_area);

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
        let input = Paragraph::new(app.draft.get(field))
            .block(pane(title, app.focus == Focus::Field(field)));
        f.render_widget(input, area);
    }
}

/// A bordered box, green when focused and gray otherwise.
fn pane<'a>(title: impl Into<Line<'a>>, focused: bool) -> Block<'a> {
    let color = if focused {
        Color::Green
    } else {
        Color::DarkGray
    };
    Block::bordered().title(title).style(Style::new().fg(color))
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
