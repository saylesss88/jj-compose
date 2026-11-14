use jj_compose_core::Draft;
use ratatui_textarea::CursorMove;

use crate::{
    App,
    app::{Field, Focus, Status, View, ui},
    vim::{self, Dir},
};

/// Somewhere focus can move to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Diff,
    Field(Field),
}

impl App {
    pub(super) fn focus_diff(&mut self) {
        self.editor().cancel_selection();
        if matches!(self.vim.mode, vim::Mode::Visual | vim::Mode::Operator(_)) {
            self.vim.mode = vim::Mode::Normal;
        }
        self.focus = Focus::Diff;
    }

    /// Switch between the form and the raw text. Going back to the form only
    /// works if it can hold the text exactly.
    pub(super) fn toggle_view(&mut self) {
        match self.view {
            View::Structured => {
                self.raw = ui::raw_editor(&self.draft().render());
                self.view = View::Raw;
            }
            View::Raw => match Draft::parse(&self.message()) {
                Ok(draft) => {
                    self.fields = Field::ALL.map(|field| ui::field_editor(field, draft.get(field)));
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
    pub(super) fn cycle_focus(&mut self, forward: bool) {
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
    pub(super) fn neighbor(&self, dir: Dir) -> Option<Stop> {
        use Field::{Scope, Subject, Trailers, Type};
        if self.view == View::Raw {
            return if self.is_vertical {
                (dir == Dir::Up).then_some(Stop::Diff)
            } else {
                (dir == Dir::Left).then_some(Stop::Diff)
            };
        }

        let field = match (dir, self.field) {
            (Dir::Up, Type | Scope) => {
                return if self.is_vertical {
                    Some(Stop::Diff)
                } else {
                    None
                };
            }
            (Dir::Down, Trailers) => return None,

            (Dir::Right, _) if self.field != Type => return None,

            (Dir::Left, f) if f != Scope => {
                return if self.is_vertical {
                    None
                } else {
                    Some(Stop::Diff)
                };
            }

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
    pub(super) fn go(&mut self, dir: Dir) {
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

    pub(super) fn focus_field(&mut self, field: Field) {
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
}
