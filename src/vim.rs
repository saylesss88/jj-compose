//! Opt-in Vim-style editing for the form's text fields.
//!
//! Adapted from ratatui-textarea's `vim` example (MIT). It covers the
//! motions and operators that matter for short prose, not all of Vim.

use ratatui_textarea::{CursorMove, Input, Key, TextArea};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    /// `d`, `c` or `y` typed, waiting for a motion.
    Operator(char),
}

impl Mode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal | Self::Operator(_) => " NORMAL ",
            Self::Insert => " INSERT ",
            Self::Visual => " VISUAL ",
        }
    }
}

/// A direction to move focus in, as the panes are laid out on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// What the form should do after a key, beyond editing the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    /// `Enter`: the next field in Tab order.
    NextField,
    /// `h`/`j`/`k`/`l` ran into the edge of the text.
    Move(Dir),
}

#[derive(Debug)]
pub struct Vim {
    pub mode: Mode,
    /// First key of a two-key sequence like `gg`.
    pending: Option<char>,
    /// Set when the pending operator applies to whole lines (`dd`, `yy`,
    /// `cc`): the row it was typed on.
    linewise: Option<usize>,
}

impl Vim {
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            pending: None,
            linewise: None,
        }
    }

    /// Handle a key in Normal, Visual or Operator-pending mode. Insert mode
    /// is left to the caller, which uses the default editing keys there.
    pub fn handle(&mut self, input: &Input, editor: &mut TextArea<'_>) -> Action {
        let pending = self.pending.take();
        let row = editor.cursor().0;
        let last_row = editor.lines().len() - 1;

        if input.ctrl || input.alt {
            if input.ctrl && input.key == Key::Char('r') {
                editor.redo();
            }
            return Action::None;
        }

        match input.key {
            Key::Esc => {
                editor.cancel_selection();
                self.mode = Mode::Normal;
                return Action::None;
            }
            Key::Char('q') if self.mode == Mode::Normal => return Action::Quit,
            Key::Enter if self.mode == Mode::Normal => return Action::NextField,

            // At the edge of the text, h/j/k/l carry on into whatever is
            // next to this field on screen.
            Key::Char('j') | Key::Down if self.mode == Mode::Normal && row == last_row => {
                return Action::Move(Dir::Down);
            }
            Key::Char('k') | Key::Up if self.mode == Mode::Normal && row == 0 => {
                return Action::Move(Dir::Up);
            }
            Key::Char('h') | Key::Left if self.mode == Mode::Normal && editor.cursor().1 == 0 => {
                return Action::Move(Dir::Left);
            }
            Key::Char('l') | Key::Right if self.mode == Mode::Normal && at_line_end(editor) => {
                return Action::Move(Dir::Right);
            }

            // Motions (also the targets of a pending operator).
            Key::Char('h') | Key::Left => editor.move_cursor(CursorMove::Back),
            Key::Char('j') | Key::Down => editor.move_cursor(CursorMove::Down),
            Key::Char('k') | Key::Up => editor.move_cursor(CursorMove::Up),
            Key::Char('l') | Key::Right => editor.move_cursor(CursorMove::Forward),
            // Like Vim, `cw` changes to the end of the word, not up to the
            // next one, so the space after it survives.
            Key::Char('w') if self.mode == Mode::Operator('c') => {
                editor.move_cursor(CursorMove::WordEnd);
                editor.move_cursor(CursorMove::Forward);
            }
            Key::Char('w') => editor.move_cursor(CursorMove::WordForward),
            Key::Char('b') => editor.move_cursor(CursorMove::WordBack),
            Key::Char('e') => {
                editor.move_cursor(CursorMove::WordEnd);
                if matches!(self.mode, Mode::Operator(_)) {
                    // Include the character under the cursor.
                    editor.move_cursor(CursorMove::Forward);
                }
            }
            Key::Char('0' | '^') | Key::Home => editor.move_cursor(CursorMove::Head),
            Key::Char('$') | Key::End => editor.move_cursor(CursorMove::End),
            Key::Char('g') if pending == Some('g') => editor.move_cursor(CursorMove::Top),
            Key::Char('g') => {
                self.pending = Some('g');
                return Action::None;
            }
            Key::Char('G') => editor.move_cursor(CursorMove::Bottom),

            // `dd`, `cc`, `yy`: operate on the whole line.
            Key::Char(c) if self.mode == Mode::Operator(c) => {
                self.linewise = Some(row);
                editor.cancel_selection();
                if c == 'c' || row == 0 && row == last_row {
                    // Just the text; the (only) line itself stays.
                    editor.move_cursor(CursorMove::Head);
                    editor.start_selection();
                    editor.move_cursor(CursorMove::End);
                } else if row < last_row {
                    // The line and its newline.
                    editor.move_cursor(CursorMove::Head);
                    editor.start_selection();
                    editor.move_cursor(CursorMove::Down);
                    editor.move_cursor(CursorMove::Head);
                } else {
                    // The last line: take the newline before it instead.
                    editor.move_cursor(CursorMove::Up);
                    editor.move_cursor(CursorMove::End);
                    editor.start_selection();
                    editor.move_cursor(CursorMove::Down);
                    editor.move_cursor(CursorMove::End);
                }
            }

            _ if self.mode == Mode::Normal => {
                self.normal_command(input.key, editor);
                return Action::None;
            }
            _ if self.mode == Mode::Visual => {
                self.visual_command(input.key, editor);
                return Action::None;
            }
            // Anything else cancels a pending operator.
            _ => {
                editor.cancel_selection();
                self.mode = Mode::Normal;
                return Action::None;
            }
        }

        // A motion completed a pending operator.
        if let Mode::Operator(op) = self.mode {
            self.apply(op, editor);
        }
        Action::None
    }

    fn normal_command(&mut self, key: Key, editor: &mut TextArea<'_>) {
        match key {
            Key::Char('i') => self.mode = Mode::Insert,
            Key::Char('a') => {
                if !at_line_end(editor) {
                    editor.move_cursor(CursorMove::Forward);
                }
                self.mode = Mode::Insert;
            }
            Key::Char('I') => {
                editor.move_cursor(CursorMove::Head);
                self.mode = Mode::Insert;
            }
            Key::Char('A') => {
                editor.move_cursor(CursorMove::End);
                self.mode = Mode::Insert;
            }
            Key::Char('o') => {
                editor.move_cursor(CursorMove::End);
                editor.insert_newline();
                self.mode = Mode::Insert;
            }
            Key::Char('O') => {
                editor.move_cursor(CursorMove::Head);
                editor.insert_newline();
                editor.move_cursor(CursorMove::Up);
                self.mode = Mode::Insert;
            }
            Key::Char('x') => {
                // Don't join lines when the cursor is past the last character.
                let (row, col) = (editor.cursor().0, editor.cursor().1);
                if col < editor.lines()[row].chars().count() {
                    editor.delete_next_char();
                }
            }
            Key::Char('D') => {
                editor.delete_line_by_end();
            }
            Key::Char('C') => {
                editor.delete_line_by_end();
                self.mode = Mode::Insert;
            }
            Key::Char('S') => {
                editor.move_cursor(CursorMove::Head);
                editor.delete_line_by_end();
                self.mode = Mode::Insert;
            }
            Key::Char('J') => {
                if editor.cursor().0 + 1 < editor.lines().len() {
                    editor.move_cursor(CursorMove::End);
                    editor.delete_next_char();
                    editor.insert_char(' ');
                }
            }
            Key::Char('p') => paste(editor, true),
            Key::Char('P') => paste(editor, false),
            Key::Char('u') => {
                editor.undo();
            }
            Key::Char('v') => {
                editor.start_selection();
                self.mode = Mode::Visual;
            }
            Key::Char('V') => {
                editor.move_cursor(CursorMove::Head);
                editor.start_selection();
                editor.move_cursor(CursorMove::End);
                self.mode = Mode::Visual;
            }
            Key::Char(op @ ('d' | 'c' | 'y')) => {
                editor.start_selection();
                self.mode = Mode::Operator(op);
            }
            _ => {}
        }
    }

    fn visual_command(&mut self, key: Key, editor: &mut TextArea<'_>) {
        match key {
            Key::Char(op @ ('d' | 'c' | 'y' | 'x')) => {
                // Vim's selection includes the character under the cursor.
                editor.move_cursor(CursorMove::Forward);
                self.apply(if op == 'x' { 'd' } else { op }, editor);
            }
            Key::Char('v') => {
                editor.cancel_selection();
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn apply(&mut self, op: char, editor: &mut TextArea<'_>) {
        self.mode = Mode::Normal;
        let linewise = self.linewise.take();
        let start = match linewise {
            Some(row) => Some((row, 0)),
            None => editor.selection_range().map(|(start, _)| start),
        };
        match op {
            'y' => {
                editor.copy();
                editor.cancel_selection();
                // Yanking leaves the cursor where the text starts.
                if let Some((row, col)) = start {
                    editor.move_cursor(jump(row, col));
                }
            }
            'c' => {
                editor.cut();
                self.mode = Mode::Insert;
            }
            _ => {
                editor.cut();
            }
        }

        if linewise.is_some() {
            // Store whole lines as `text\n`, which `p` puts on its own line.
            let yank = editor.yank_text();
            let line = yank
                .strip_suffix('\n')
                .or_else(|| yank.strip_prefix('\n'))
                .unwrap_or(&yank);
            editor.set_yank_text(format!("{line}\n"));
            if op == 'd' {
                editor.move_cursor(CursorMove::Head);
            }
        }
    }
}

fn jump(row: usize, col: usize) -> CursorMove {
    CursorMove::Jump(
        u16::try_from(row).unwrap_or(u16::MAX),
        u16::try_from(col).unwrap_or(u16::MAX),
    )
}

/// Vim-style `p`/`P`: a yank ending in a newline (from `dd`, `yy`) goes on
/// its own line below/above; anything else goes after/at the cursor.
fn paste(editor: &mut TextArea<'_>, after: bool) {
    let yank = editor.yank_text();
    if let Some(line) = yank.strip_suffix('\n') {
        if after {
            editor.move_cursor(CursorMove::End);
            editor.insert_newline();
        } else {
            editor.move_cursor(CursorMove::Head);
            editor.insert_newline();
            editor.move_cursor(CursorMove::Up);
        }
        editor.insert_str(line);
        editor.move_cursor(CursorMove::Head);
    } else {
        if after && !editor.lines()[editor.cursor().0].is_empty() {
            editor.move_cursor(CursorMove::Forward);
        }
        editor.paste();
    }
}

/// True if the cursor is on (or past) the last character of its line.
fn at_line_end(editor: &TextArea<'_>) -> bool {
    let cursor = editor.cursor();
    let len = editor.lines()[cursor.0].chars().count();
    cursor.1 + 1 >= len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> Input {
        Input {
            key: Key::Char(c),
            ..Input::default()
        }
    }

    fn run(text: &str, keys: &str) -> (Vim, TextArea<'static>) {
        let mut editor = TextArea::new(text.split('\n').map(str::to_owned).collect());
        let mut vim = Vim::new(Mode::Normal);
        for c in keys.chars() {
            vim.handle(&key(c), &mut editor);
        }
        (vim, editor)
    }

    #[test]
    fn delete_word() {
        let (vim, editor) = run("fix the parser", "dw");
        assert_eq!(editor.lines(), ["the parser"]);
        assert_eq!(vim.mode, Mode::Normal);
    }

    #[test]
    fn change_word_enters_insert() {
        let (vim, editor) = run("fix the parser", "wcw");
        assert_eq!(editor.lines(), ["fix  parser"]);
        assert_eq!(vim.mode, Mode::Insert);
    }

    #[test]
    fn delete_line_and_paste() {
        let (_, editor) = run("one\ntwo\nthree", "ddp");
        assert_eq!(editor.lines(), ["two", "one", "three"]);
    }

    #[test]
    fn yank_line_and_paste_above() {
        let (_, editor) = run("one\ntwo", "jyyP");
        assert_eq!(editor.lines(), ["one", "two", "two"]);
    }

    #[test]
    fn paste_word_after_cursor() {
        let (_, editor) = run("ab", "ywp");
        assert_eq!(editor.lines(), ["aabb"]);
    }

    #[test]
    fn delete_last_line_removes_it() {
        let (_, editor) = run("one\ntwo", "jdd");
        assert_eq!(editor.lines(), ["one"]);
    }

    #[test]
    fn gg_goes_to_top() {
        let (_, editor) = run("one\ntwo", "Ggg");
        assert_eq!(editor.cursor().0, 0);
    }

    #[test]
    fn j_on_last_line_leaves_field() {
        let mut editor = TextArea::new(vec!["only".into()]);
        let mut vim = Vim::new(Mode::Normal);
        assert_eq!(vim.handle(&key('j'), &mut editor), Action::Move(Dir::Down));
        assert_eq!(vim.handle(&key('k'), &mut editor), Action::Move(Dir::Up));
        editor.move_cursor(CursorMove::End);
        assert_eq!(vim.handle(&key('l'), &mut editor), Action::Move(Dir::Right));
        editor.move_cursor(CursorMove::Head);
        assert_eq!(vim.handle(&key('h'), &mut editor), Action::Move(Dir::Left));
    }
}
