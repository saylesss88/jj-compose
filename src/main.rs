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

fn main() {}
