use crate::{
    Exit,
    app::{self, App, Focus, KeyCode, Split, Status},
};

impl App {
    /// Deciding one hunk at a time: add it, skip it, or move around.
    pub(super) fn split_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Exit> {
        // Scrolling, for a hunk taller than the pane.
        match code {
            KeyCode::Char('d') if ctrl => {
                self.diff.scroll_down(self.diff.half_page());
                return None;
            }
            KeyCode::Char('u') if ctrl => {
                self.diff.scroll_up(self.diff.half_page());
                return None;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.diff.scroll_down(1);
                return None;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.diff.scroll_up(1);
                return None;
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.cancel_split();
                return None;
            }
            // Done marking, whether or not every hunk has been seen.
            KeyCode::Enter => {
                self.write_message();
                return None;
            }
            _ => {}
        }
        let split = self.split.as_mut()?;
        match code {
            KeyCode::Char(' ') => split.toggle(),
            // `y` and `n` read as they do in `git add -p`, except that
            // neither is final: an unmarked hunk can be marked later.
            KeyCode::Char('y') => {
                split.mark(true);
                split.next();
            }
            KeyCode::Char('n' | '}') | KeyCode::Right => split.next(),
            KeyCode::Char('p' | '{') | KeyCode::Left => split.prev(),
            KeyCode::Char('a') => split.mark_file(true),
            KeyCode::Char('d') => split.mark_file(false),
            KeyCode::Char('A') => split.mark_all(true),
            KeyCode::Char('C') => split.mark_all(false),
            KeyCode::Char('g') => split.first(),
            KeyCode::Char('G') => split.last(),
            _ => return None,
        }

        if let Some(at) = self.split.as_ref().map(Split::at) {
            self.diff.show_hunk(at);
        }
        None
    }

    pub(super) fn start_split(&mut self) {
        match Split::new(self.diff.hunks().iter().map(|h| (h.id, h.path.clone()))) {
            Some(split) => self.enter_split(split),
            None => {
                self.status = Some(Status::Error(
                    "nothing to split: this change has fewer than two hunks".into(),
                ));
            }
        }
    }

    fn cancel_split(&mut self) {
        self.split = None;
        self.zoomed = false;
        self.focus = Focus::Editor;
        self.status = Some(Status::Info("Left split mode.".into()));
    }

    /// Leave the hunks marked as they are and go write the message.
    fn write_message(&mut self) {
        let marked = self.split.as_ref().map_or(0, Split::count);
        self.zoomed = false;
        self.focus = Focus::Editor;
        self.status = Some(Status::Info(format!(
            "{marked} {} marked. Write the message, then ^s to commit them.",
            app::plural(marked, "hunk")
        )));
    }

    /// Start choosing hunks for a commit, showing the first on its own.
    /// The warning to open with when this revision is protected history.
    pub(super) fn immutable_warning(&self) -> Option<Status> {
        if !self.target.immutable {
            return None;
        }
        let tags: Vec<&str> = self.at_risk.iter().map(|t| t.tag.as_str()).collect();
        Some(Status::Error(if tags.is_empty() {
            "immutable revision: rewriting it gives every later commit a new hash".to_owned()
        } else {
            format!(
                "immutable revision: a rewrite strands {} {} ({})",
                tags.len(),
                app::plural(tags.len(), "tag"),
                tags.join(", ")
            )
        }))
    }
}
