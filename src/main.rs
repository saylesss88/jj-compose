use std::{error::Error, io};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

// Track where the user's cursor currently is
#[derive(Clone)]
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

pub struct App {
    // Jujutsu Data
    pub diff_content: String,

    // User Input Buffers
    pub commit_type: String,
    pub scope: String,
    pub subject: String,
    pub problem: String,
    pub solution: String,

    // UI State
    pub active_pane: ActivePane,
    pub input_mode: InputMode,
    pub diff_scroll_position: u16,
    pub should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            diff_content: String::new(),
            commit_type: String::new(),
            scope: String::new(),
            subject: String::new(),
            problem: String::new(),
            solution: String::new(),
            active_pane: ActivePane::Type, // Start cursor here
            input_mode: InputMode::Normal, // Start in Normal mode
            diff_scroll_position: 0,
            should_quit: false,
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen)?; // Swaps to a clean screen
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;

    // Initialize state
    let mut app = App::default();

    // Run application loop
    let res = run_app(&mut terminal, &mut app);

    // Teardown terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{err:?}");
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    app: &mut App,
) -> Result<(), Box<dyn Error>> {
    while !app.should_quit {
        // Draw the UI
        terminal.draw(|f| ui(f, app))?;

        // Handle events (blocking wait for next keystroke)
        if let Event::Key(key) = event::read()? {
            match app.input_mode {
                InputMode::Normal => match key.code {
                    KeyCode::Char('q') => app.should_quit = true,

                    KeyCode::Char('i') => app.input_mode = InputMode::Insert,

                    // Left/Right navigation
                    KeyCode::Char('h') => app.active_pane = ActivePane::DiffViewer,
                    KeyCode::Char('l') => app.active_pane = ActivePane::Type, // Jump back to forms

                    // Up/Down navigation through the right-side forms
                    KeyCode::Char('j') => {
                        app.active_pane = match app.active_pane {
                            ActivePane::Type => ActivePane::Scope,
                            ActivePane::Scope => ActivePane::Subject,
                            ActivePane::Subject => ActivePane::Problem,
                            ActivePane::Problem => ActivePane::Solution,
                            _ => app.active_pane.clone(), // Do nothing if in Solution or DiffViewer
                        }
                    }
                    KeyCode::Char('k') => {
                        app.active_pane = match app.active_pane {
                            ActivePane::Scope => ActivePane::Type,
                            ActivePane::Subject => ActivePane::Scope,
                            ActivePane::Problem => ActivePane::Subject,
                            ActivePane::Solution => ActivePane::Problem,
                            _ => app.active_pane.clone(),
                        }
                    }
                    _ => {}
                },
                InputMode::Insert => {
                    // 1. Get a mutable reference to the currently active text buffer
                    let active_buffer = match app.active_pane {
                        ActivePane::Type => Some(&mut app.commit_type),
                        ActivePane::Scope => Some(&mut app.scope),
                        ActivePane::Subject => Some(&mut app.subject),
                        ActivePane::Problem => Some(&mut app.problem),
                        ActivePane::Solution => Some(&mut app.solution),
                        ActivePane::DiffViewer => None, // Cannot type in the diff viewer
                    };

                    // 2. Handle the keystrokes
                    match key.code {
                        KeyCode::Esc => app.input_mode = InputMode::Normal,
                        KeyCode::Char(c) => {
                            if let Some(buf) = active_buffer {
                                buf.push(c);
                            }
                        }
                        KeyCode::Backspace => {
                            if let Some(buf) = active_buffer {
                                buf.pop(); // Removes the last character
                            }
                        }
                        KeyCode::Enter => {
                            // Only allow multi-line text in the Problem and Solution boxes
                            if matches!(app.active_pane, ActivePane::Problem | ActivePane::Solution)
                                && let Some(buf) = active_buffer
                            {
                                buf.push('\n');
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

// The Rendering Function
fn ui(f: &mut Frame, app: &App) {
    // 1. Split the entire screen in half horizontally
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(f.area());

    // 2. Draw the Left Side (Diff Viewer)
    let diff_color = if matches!(app.active_pane, ActivePane::DiffViewer) {
        Color::Green
    } else {
        Color::DarkGray
    };
    let diff_block = Paragraph::new(app.diff_content.as_str()).block(
        Block::default()
            .title(" Diff (@) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(diff_color)),
    );
    f.render_widget(diff_block, main_chunks[0]);

    // 3. Split the Right Side vertically for the input forms
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Type
            Constraint::Length(3), // Scope
            Constraint::Length(3), // Subject
            Constraint::Min(5),    // Problem (expands to fill space)
            Constraint::Min(5),    // Solution (expands to fill space)
        ])
        .split(main_chunks[1]);

    // Helper macro/function to draw input boxes with the correct highlight color
    let get_color = |target_pane: &ActivePane| -> Color {
        // If the app's active pane matches this box, highlight it Green
        if std::mem::discriminant(&app.active_pane) == std::mem::discriminant(target_pane) {
            Color::Green
        } else {
            Color::DarkGray
        }
    };

    // 4. Draw each input box on the right side
    let p_type = Paragraph::new(app.commit_type.as_str()).block(
        Block::default()
            .title(" Type ")
            .borders(Borders::ALL)
            .style(Style::default().fg(get_color(&ActivePane::Type))),
    );
    f.render_widget(p_type, right_chunks[0]);

    let p_scope = Paragraph::new(app.scope.as_str()).block(
        Block::default()
            .title(" Scope ")
            .borders(Borders::ALL)
            .style(Style::default().fg(get_color(&ActivePane::Scope))),
    );
    f.render_widget(p_scope, right_chunks[1]);

    let p_subject = Paragraph::new(app.subject.as_str()).block(
        Block::default()
            .title(" Subject ")
            .borders(Borders::ALL)
            .style(Style::default().fg(get_color(&ActivePane::Subject))),
    );
    f.render_widget(p_subject, right_chunks[2]);

    let p_problem = Paragraph::new(app.problem.as_str()).block(
        Block::default()
            .title(" Problem ")
            .borders(Borders::ALL)
            .style(Style::default().fg(get_color(&ActivePane::Problem))),
    );
    f.render_widget(p_problem, right_chunks[3]);

    let p_solution = Paragraph::new(app.solution.as_str()).block(
        Block::default()
            .title(" Solution ")
            .borders(Borders::ALL)
            .style(Style::default().fg(get_color(&ActivePane::Solution))),
    );
    f.render_widget(p_solution, right_chunks[4]);
}
