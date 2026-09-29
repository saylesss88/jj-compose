use std::{error::Error, io};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend};

// Track where the user's cursor currently is
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
                    // TODO: Add H/J/K/L navigation here
                    _ => {}
                },
                InputMode::Insert => match key.code {
                    KeyCode::Esc => app.input_mode = InputMode::Normal,
                    // TODO: Route typing to the active text buffer
                    _ => {}
                },
            }
        }
    }
    Ok(())
}

// The Rendering Function
fn ui(f: &mut Frame, app: &App) {
    // TODO: Split the frame and draw widgets based on app.active_pane
}
