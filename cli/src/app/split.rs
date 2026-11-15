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
            // Assigning works exactly as it does on the hunk itself.
            KeyCode::Char(c @ '1'..='9') => self.split.as_mut()?.toggle(digit(c)),
            KeyCode::Char(' ') => self.split.as_mut()?.toggle(1),
            KeyCode::Char('0') => self.split.as_mut()?.assign(None),
            KeyCode::Char('a') => {
                let commit = self.split.as_ref()?.box_of().unwrap_or(1);
                self.split.as_mut()?.assign_file(Some(commit));
            }
            KeyCode::Char('d') => self.split.as_mut()?.assign_file(None),
            KeyCode::Char('A') => self.split.as_mut()?.assign_all(Some(1)),
            KeyCode::Char('C') => self.split.as_mut()?.assign_all(None),
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
            // A digit is the box; the same digit again takes it back out.
            KeyCode::Char(c @ '1'..='9') => split.toggle(digit(c)),
            KeyCode::Char(' ') => split.toggle(1),
            KeyCode::Char('0') => split.assign(None),
            // `y` reads as it does in `git add -p`, except that it is not
            // final: a hunk can be moved or taken out later.
            KeyCode::Char('y') => {
                split.assign(Some(1));
                split.next();
            }
            KeyCode::Char('n' | '}') | KeyCode::Right => split.next(),
            KeyCode::Char('p' | '{') | KeyCode::Left => split.prev(),
            // The file joins whichever commit this hunk is already in.
            KeyCode::Char('a') => {
                let commit = split.box_of().unwrap_or(1);
                split.assign_file(Some(commit));
            }
            KeyCode::Char('d') => split.assign_file(None),
            KeyCode::Char('A') => split.assign_all(Some(1)),
            KeyCode::Char('C') => split.assign_all(None),
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

    /// Leave the plan as it is and go write the next commit's message.
    fn write_message(&mut self) {
        let (commit, held, planned) = self.split.as_ref().map_or((0, 0, 0), |split| {
            (
                split.next_box().unwrap_or(0),
                split.next_box().map_or(0, |n| split.count(n)),
                split.pending().len(),
            )
        });
        self.zoomed = false;
        self.focus = Focus::Editor;
        self.status = Some(Status::Info(if planned == 0 {
            "Nothing assigned yet. 1-9 puts the hunk on screen in a commit.".to_owned()
        } else {
            format!(
                "Describing commit {commit} of {planned}: {held} {}. ^s commits it.",
                app::plural(held, "hunk")
            )
        }));
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

/// A digit key as the commit number it names.
fn digit(c: char) -> u8 {
    u8::try_from(c.to_digit(10).unwrap_or(1)).unwrap_or(1)
}
