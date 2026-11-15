use crate::{
    Exit,
    app::{self, App, Focus, KeyCode, Split, Status},
};

impl App {
    /// Walking the list: the cursor skips file headings, and moving it moves
    /// the hunk on screen, so marking always acts on what you are looking at.
    fn overview_key(&mut self, code: KeyCode) -> Option<Exit> {
        let rows = self.overview_rows();
        let at = self.overview?;
        let step = |from: usize, down: bool| {
            let mut i = from;
            loop {
                let next = if down { i + 1 } else { i.checked_sub(1)? };
                if next >= rows.len() {
                    return None;
                }
                i = next;
                if rows[i].hunk.is_some() {
                    return Some(i);
                }
            }
        };
        match code {
            // `Enter` goes back to reading the hunk under the cursor; the
            // others just close the list. Same effect, since the cursor has
            // been moving the hunk on screen all along.
            KeyCode::Char('f' | 'q') | KeyCode::Esc | KeyCode::Enter => self.overview = None,
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(i) = step(at, true) {
                    self.overview = Some(i);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(i) = step(at, false) {
                    self.overview = Some(i);
                }
            }
            KeyCode::Char('g') => self.overview = rows.iter().position(|r| r.hunk.is_some()),
            KeyCode::Char('G') => self.overview = rows.iter().rposition(|r| r.hunk.is_some()),
            // Marking works exactly as it does on the hunk itself.
            KeyCode::Char(' ') => self.split.as_mut()?.toggle(),
            KeyCode::Char('y') => self.split.as_mut()?.mark(true),
            KeyCode::Char('n') => self.split.as_mut()?.mark(false),
            KeyCode::Char('a') => self.split.as_mut()?.mark_file(true),
            KeyCode::Char('d') => self.split.as_mut()?.mark_file(false),
            KeyCode::Char('A') => self.split.as_mut()?.mark_all(true),
            KeyCode::Char('C') => self.split.as_mut()?.mark_all(false),
            _ => return None,
        }
        // Keep the hunk on screen in step with the row under the cursor.
        if let (Some(at), Some(split)) = (self.overview, self.split.as_mut()) {
            if let Some(id) = rows.get(at).and_then(|r| r.hunk) {
                split.go_to(id);
            }
            let hunk = split.at();
            self.diff.show_hunk(hunk);
        }
        None
    }

    /// Deciding one hunk at a time: add it, skip it, or move around.
    pub(super) fn split_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Exit> {
        if self.overview.is_some() {
            return self.overview_key(code);
        }
        // Opening the list needs the symbols, which cost a subprocess each.
        if code == KeyCode::Char('f') {
            let rows = self.overview_rows();
            let at = self.split.as_ref().map_or(0, Split::at);
            self.overview = rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.hunk.is_some())
                .nth(at)
                .map(|(i, _)| i);
            if self.symbols.is_empty() {
                self.pending = Some(app::Request::Symbols);
            }
            return None;
        }
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
