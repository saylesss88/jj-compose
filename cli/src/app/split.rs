use crate::{
    Exit,
    app::{self, App, Focus, KeyCode, Split, Status},
};

impl App {
    /// Walking the list: the cursor skips file headings, and moving it moves
    /// the hunk on screen, so marking always acts on what you are looking at.
    ///
    /// `v` or `V` starts a visual selection at the cursor; moving then
    /// stretches it, and a digit puts every hunk in it in that commit at
    /// once (`V G 1`, `V ] 2`). The selection is a run of hunks in diff
    /// order, so unlike `a` it can span files.
    fn overview_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Exit> {
        let rows = self.overview_rows();
        let at = self.overview?;
        // Only hunk rows can hold the cursor; work in hunk numbers, which
        // are also the split's, and map back to rows at the end.
        let hunk_rows: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.hunk.is_some())
            .map(|(i, _)| i)
            .collect();
        let here = hunk_rows.iter().position(|&r| r == at)?;
        let last = hunk_rows.len().saturating_sub(1);
        let files: Vec<usize> = hunk_rows
            .iter()
            .filter_map(|&r| rows[r].hunk.map(|h| h.file))
            .collect();
        let half = self.diff.half_page();
        let mut to = here;

        match code {
            // Motions, which also stretch a selection.
            KeyCode::Char('d') if ctrl => to = (here + half).min(last),
            KeyCode::Char('u') if ctrl => to = here.saturating_sub(half),
            KeyCode::Char('j') | KeyCode::Down => to = (here + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => to = here.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => to = 0,
            KeyCode::Char('G') | KeyCode::End => to = last,
            KeyCode::Char(']') => to = file_jump(&files, here, true),
            KeyCode::Char('[') => to = file_jump(&files, here, false),

            // Selecting.
            KeyCode::Char('v' | 'V') => {
                self.visual = if self.visual.is_some() {
                    None
                } else {
                    Some(here)
                };
            }
            // Vim's `o`: jump to the other end of the selection.
            KeyCode::Char('o') if self.visual.is_some() => {
                to = self.visual.replace(here).unwrap_or(here);
            }
            KeyCode::Esc if self.visual.is_some() => self.visual = None,

            // `Enter` goes back to reading the hunk under the cursor; the
            // others just close the list. Same effect, since the cursor has
            // been moving the hunk on screen all along.
            KeyCode::Char('f' | 'q') | KeyCode::Esc | KeyCode::Enter => {
                self.overview = None;
                self.visual = None;
            }

            // Assigning works exactly as it does on the hunk itself, or on
            // the whole selection when there is one. Either way a selection
            // ends once it has been used, as in Vim.
            KeyCode::Char(c @ ('1'..='9' | ' ')) => {
                let commit = if c == ' ' { 1 } else { digit(c) };
                let split = self.split.as_mut()?;
                match self.visual.take() {
                    Some(from) => split.toggle_range(from, here, commit),
                    None => split.toggle(commit),
                }
            }
            KeyCode::Char('0') => {
                let split = self.split.as_mut()?;
                match self.visual.take() {
                    Some(from) => split.assign_range(from, here, None),
                    None => split.assign(None),
                }
            }
            // `d`/`x` on a selection take it out, as Vim's would delete it.
            KeyCode::Char('d' | 'x') if self.visual.is_some() => {
                let from = self.visual.take()?;
                self.split.as_mut()?.assign_range(from, here, None);
            }
            KeyCode::Char('a') => {
                self.visual = None;
                let commit = self.split.as_ref()?.box_of().unwrap_or(1);
                self.split.as_mut()?.assign_file(Some(commit));
            }
            KeyCode::Char('d') => self.split.as_mut()?.assign_file(None),
            KeyCode::Char('A') => {
                self.visual = None;
                self.split.as_mut()?.assign_all(Some(1));
            }
            KeyCode::Char('C') => {
                self.visual = None;
                self.split.as_mut()?.assign_all(None);
            }
            _ => return None,
        }
        // Keep the hunk on screen in step with the row under the cursor.
        if self.overview.is_some() {
            self.overview = hunk_rows.get(to).copied();
        }
        if let (Some(at), Some(split)) = (self.overview, self.split.as_mut()) {
            if let Some(id) = rows.get(at).and_then(|r| r.hunk) {
                split.go_to(id);
            }
            let hunk = split.at();
            self.diff.show_hunk(hunk);
        }
        None
    }

    /// Open the list with the cursor on the hunk on screen, optionally
    /// starting a visual selection there.
    fn open_overview(&mut self, select: bool) {
        let rows = self.overview_rows();
        let at = self.split.as_ref().map_or(0, Split::at);
        self.overview = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.hunk.is_some())
            .nth(at)
            .map(|(i, _)| i);
        self.visual = (select && self.overview.is_some()).then_some(at);
        // The symbols cost a subprocess each, so only now are they wanted.
        if self.symbols.is_empty() {
            self.pending = Some(app::Request::Symbols);
        }
    }

    /// Deciding one hunk at a time: add it, skip it, or move around.
    pub(super) fn split_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Exit> {
        if self.overview.is_some() {
            return self.overview_key(code, ctrl);
        }
        // A divided hunk takes the keys over: the cursor is a line now, and
        // a digit decides that line rather than the whole hunk.
        if self.split.as_ref().is_some_and(Split::is_divided) {
            return self.divided_key(code, ctrl);
        }
        match code {
            KeyCode::Char('f') => {
                self.open_overview(false);
                return None;
            }
            // Selecting happens in the list, so `V` opens it already
            // selecting from the hunk on screen.
            KeyCode::Char('v' | 'V') => {
                self.open_overview(true);
                return None;
            }
            // Open this hunk up so its lines can go to different commits.
            KeyCode::Char('e') => {
                let at = self.split.as_ref()?.at();
                let lines = self.hunk_lines(at);
                let count = lines.len();
                self.split.as_mut()?.divide(lines);
                if self.split.as_ref().is_some_and(Split::is_divided) {
                    self.diff.show_hunk(at);
                } else {
                    self.status = Some(Status::Info(format!(
                        "nothing to divide: this hunk changes {count} line{}",
                        if count == 1 { "" } else { "s" }
                    )));
                }
                return None;
            }
            _ => {}
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

    /// The `+`/`-` lines of one hunk, as [`Split::divide`] wants them.
    fn hunk_lines(&self, at: usize) -> Vec<usize> {
        let Some(id) = self.diff.hunks().get(at).map(|h| h.id) else {
            return Vec::new();
        };
        let patch = self.diff.patch();
        patch
            .files()
            .get(id.file)
            .and_then(|f| f.hunks.get(id.hunk))
            .map_or_else(Vec::new, |hunk| patch.changed_lines(hunk))
    }

    /// Deciding one line at a time, inside a hunk that has been opened up.
    ///
    /// Only the keys that mean something here are handled: `j`/`k` move
    /// between the hunk's own lines, so the pane no longer scrolls, and
    /// `Ctrl-d`/`Ctrl-u` are what a hunk taller than the pane is read with.
    fn divided_key(&mut self, code: KeyCode, ctrl: bool) -> Option<Exit> {
        match code {
            // Close it. Lines that all agree fold back into the hunk.
            KeyCode::Char('e') => {
                self.split.as_mut()?.undivide();
                if self.split.as_ref().is_some_and(Split::is_divided) {
                    self.status = Some(Status::Info(
                        "the lines of this hunk go to different commits, so it stays divided"
                            .into(),
                    ));
                }
            }
            KeyCode::Char('d') if ctrl => self.diff.scroll_down(self.diff.half_page()),
            KeyCode::Char('u') if ctrl => self.diff.scroll_up(self.diff.half_page()),
            KeyCode::Char('j' | 'n') | KeyCode::Down => self.split.as_mut()?.next_line(),
            KeyCode::Char('k' | 'p') | KeyCode::Up => self.split.as_mut()?.prev_line(),
            KeyCode::Char(c @ '1'..='9') => self.split.as_mut()?.toggle_line(digit(c)),
            KeyCode::Char(' ') => self.split.as_mut()?.toggle_line(1),
            KeyCode::Char('0') => self.split.as_mut()?.assign_line(None),
            // As in the hunk view: not final, just quicker.
            KeyCode::Char('y') => {
                let split = self.split.as_mut()?;
                split.assign_line(Some(1));
                split.next_line();
            }
            // Leaving split mode entirely still works from in here.
            KeyCode::Esc | KeyCode::Char('q') => self.cancel_split(),
            KeyCode::Enter => self.write_message(),
            _ => {}
        }
        None
    }

    pub(super) fn start_split(&mut self) {
        match Split::new(self.splittable()) {
            Some(split) => self.enter_split(split),
            None => {
                self.status = Some(Status::Error(
                    "nothing to split: this change has one hunk with one change in it".into(),
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

/// Where `]` (forward) or `[` lands from hunk `here`, given each hunk's
/// file: the next file's first hunk, or this file's first hunk (the
/// previous file's, if already there). Past either end it stops on the
/// first or last hunk, so `V ]` on the last file selects to the end.
fn file_jump(files: &[usize], here: usize, forward: bool) -> usize {
    let last = files.len().saturating_sub(1);
    let start_of = |k: usize| {
        (0..=k)
            .rev()
            .take_while(|&i| files[i] == files[k])
            .last()
            .unwrap_or(k)
    };
    if forward {
        (here..=last)
            .find(|&k| files[k] != files[here])
            .unwrap_or(last)
    } else {
        let first = start_of(here);
        if first < here {
            first
        } else {
            first.checked_sub(1).map_or(0, start_of)
        }
    }
}

/// A digit key as the commit number it names.
fn digit(c: char) -> u8 {
    u8::try_from(c.to_digit(10).unwrap_or(1)).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::file_jump;

    #[test]
    fn file_jumps_land_on_file_boundaries() {
        // Hunks 0-2 in file 0, 3-4 in file 1, 5 in file 2.
        let files = [0, 0, 0, 1, 1, 2];
        assert_eq!(file_jump(&files, 1, true), 3);
        assert_eq!(file_jump(&files, 3, true), 5);
        assert_eq!(
            file_jump(&files, 5, true),
            5,
            "the last file stops at its end"
        );
        assert_eq!(file_jump(&files, 4, false), 3, "this file's start");
        assert_eq!(file_jump(&files, 3, false), 0, "then the previous file's");
        assert_eq!(file_jump(&files, 0, false), 0);
    }
}
