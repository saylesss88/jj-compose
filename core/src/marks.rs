//! Planning which hunks of a change go into which commit.
//!
//! Unlike `git add -p`, where each hunk is decided once as it goes past, the
//! position and the plan here are independent: moving decides nothing, and a
//! hunk can be moved between commits until the split is made. Deciding a
//! hunk well usually means having seen the others first.
//!
//! Hunks are put in numbered boxes, one per commit to be made, and the boxes
//! are committed lowest first — so the numbering is also the order the
//! commits will end up in.

use std::collections::BTreeMap;

use crate::{HunkId, Take};

/// The hunks of a change, which one is being looked at, and which commit
/// each is destined for.
#[derive(Debug, Clone)]
pub struct Split {
    /// Every hunk in the diff, in the order they appear.
    ids: Vec<HunkId>,
    /// The file each hunk belongs to, for assigning a whole file at once.
    paths: Vec<String>,
    /// How many `+`/`-` lines each hunk has, parallel to `ids`: how many
    /// choices it holds, and so whether it can be divided at all.
    changes: Vec<usize>,
    /// The hunk being looked at.
    at: usize,
    /// Which commit each hunk is destined for, parallel to `ids`. `None`
    /// means it stays behind in what is left of the change.
    boxes: Vec<Option<u8>>,
    /// The highest box number ever used, so "commit 2 of 3" can still say
    /// "of 3" once commit 1 has been made.
    planned: u8,
    /// Hunks opened up so their lines can go to different commits, keyed by
    /// position in `ids`. Sparse on purpose: a hunk not in here is decided
    /// whole, which is nearly all of them.
    divided: BTreeMap<usize, Lines>,
    /// The line being looked at inside a divided hunk.
    line: usize,
}

/// One hunk's `+`/`-` lines and where each is going.
#[derive(Debug, Clone)]
struct Lines {
    /// The lines themselves, as indices into the patch, in the order
    /// [`crate::Patch::changed_lines`] gives them.
    at: Vec<usize>,
    /// Which commit each is destined for, parallel to `at`.
    boxes: Vec<Option<u8>>,
}

impl Split {
    /// Every hunk of the change: its id, the path of the file it is in, and
    /// how many `+`/`-` lines it has (see [`crate::Patch::changed_lines`]).
    ///
    /// `None` when there is nothing to split, which means one hunk holding
    /// one change: either commit would come out empty whichever way it
    /// went. A single hunk with two changes in it can be divided, so it is
    /// splittable.
    pub fn new(hunks: impl IntoIterator<Item = (HunkId, String, usize)>) -> Option<Self> {
        let mut ids = Vec::new();
        let mut paths = Vec::new();
        let mut changes = Vec::new();
        for (id, path, n) in hunks {
            ids.push(id);
            paths.push(path);
            changes.push(n);
        }
        let divisible = changes.first().is_some_and(|&n| n > 1);
        (ids.len() > 1 || (ids.len() == 1 && divisible)).then(|| Self {
            boxes: vec![None; ids.len()],
            ids,
            paths,
            changes,
            at: 0,
            planned: 0,
            divided: BTreeMap::new(),
            line: 0,
        })
    }

    /// How many `+`/`-` lines the hunk being looked at has: how many
    /// choices dividing it would offer.
    #[must_use]
    pub fn changes(&self) -> usize {
        self.changes.get(self.at).copied().unwrap_or_default()
    }

    /// The hunk being looked at, counted from zero.
    #[must_use]
    pub const fn at(&self) -> usize {
        self.at
    }

    /// How many hunks the change has.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.ids.len()
    }

    /// The file the hunk being looked at belongs to.
    #[must_use]
    pub fn path(&self) -> &str {
        self.paths.get(self.at).map_or("", String::as_str)
    }

    /// The hunk being looked at, by id.
    #[must_use]
    pub fn current(&self) -> Option<HunkId> {
        self.ids.get(self.at).copied()
    }

    /// Which commit each hunk is destined for, in the order they appear.
    #[must_use]
    pub fn plan(&self) -> &[Option<u8>] {
        &self.boxes
    }

    /// The commit the hunk being looked at is destined for, or `None` if it
    /// stays behind. A divided hunk has one only if every line agrees.
    #[must_use]
    pub fn box_of(&self) -> Option<u8> {
        self.divided
            .get(&self.at)
            .map_or_else(|| self.boxes.get(self.at).copied().flatten(), Lines::sole)
    }

    /// How many boxes were ever planned, which outlives committing them.
    #[must_use]
    pub const fn planned(&self) -> u8 {
        self.planned
    }

    /// The boxes that still hold hunks, lowest first.
    #[must_use]
    pub fn pending(&self) -> Vec<u8> {
        let mut used: Vec<u8> = (0..self.ids.len())
            .flat_map(|i| {
                self.divided
                    .get(&i)
                    .map_or_else(|| vec![self.boxes[i]], |div| div.boxes.clone())
            })
            .flatten()
            .collect();
        used.sort_unstable();
        used.dedup();
        used
    }

    /// The box to commit next: the lowest one still holding hunks.
    #[must_use]
    pub fn next_box(&self) -> Option<u8> {
        self.pending().first().copied()
    }

    /// Move without deciding anything, stopping at either end.
    pub const fn next(&mut self) {
        if self.at + 1 < self.ids.len() {
            self.at += 1;
        }
    }

    pub const fn prev(&mut self) {
        self.at = self.at.saturating_sub(1);
    }

    pub const fn first(&mut self) {
        self.at = 0;
    }

    pub const fn last(&mut self) {
        self.at = self.ids.len().saturating_sub(1);
    }

    /// Move to a particular hunk, if the change still has it.
    ///
    /// This is what lets an overview share one cursor with the hunk view:
    /// moving in the list moves the hunk on screen, so assigning always acts
    /// on the thing you are looking at.
    pub fn go_to(&mut self, id: HunkId) {
        if let Some(i) = self.ids.iter().position(|&h| h == id) {
            self.at = i;
        }
    }

    /// Put the hunk being looked at in a box, or take it out with `None`.
    pub fn assign(&mut self, commit: Option<u8>) {
        self.set(self.at, commit);
    }

    /// Put it in `commit`, or take it out if it is already there, so the
    /// same key both assigns and undoes.
    pub fn toggle(&mut self, commit: u8) {
        let next = if self.box_of() == Some(commit) {
            None
        } else {
            Some(commit)
        };
        self.assign(next);
    }

    /// Assign every hunk in the file being looked at, wherever those hunks
    /// sit in the change.
    pub fn assign_file(&mut self, commit: Option<u8>) {
        let Some(path) = self.paths.get(self.at).cloned() else {
            return;
        };
        for i in 0..self.paths.len() {
            if self.paths[i] == path {
                self.set(i, commit);
            }
        }
    }

    /// Assign every hunk between `from` and `to`, inclusive and in either
    /// order, as a visual selection in the overview does. Out-of-range ends
    /// are clamped to the last hunk.
    pub fn assign_range(&mut self, from: usize, to: usize, commit: Option<u8>) {
        let last = self.ids.len().saturating_sub(1);
        let (lo, hi) = (from.min(to).min(last), from.max(to).min(last));
        for i in lo..=hi {
            self.set(i, commit);
        }
    }

    /// [`Self::toggle`] for a range: if every hunk in it is already in
    /// `commit` they all come out, otherwise they all go in. A selection
    /// that is half in box 1 is made whole rather than flipped hunk by hunk,
    /// so pressing the digit twice always ends where it started.
    pub fn toggle_range(&mut self, from: usize, to: usize, commit: u8) {
        let last = self.ids.len().saturating_sub(1);
        let (lo, hi) = (from.min(to).min(last), from.max(to).min(last));
        let all_in = self.boxes[lo..=hi].iter().all(|b| *b == Some(commit));
        self.assign_range(lo, hi, (!all_in).then_some(commit));
    }

    pub fn assign_all(&mut self, commit: Option<u8>) {
        for i in 0..self.boxes.len() {
            self.set(i, commit);
        }
    }

    fn set(&mut self, i: usize, commit: Option<u8>) {
        // Deciding the hunk as a whole discards how its lines were divided.
        self.divided.remove(&i);
        if let Some(slot) = self.boxes.get_mut(i) {
            *slot = commit;
        }
        if let Some(n) = commit {
            self.planned = self.planned.max(n);
        }
    }

    /// Restore a box number worked out elsewhere, as when a plan is carried
    /// across a split; see [`crate::plan::Plan`].
    pub fn restore(&mut self, id: HunkId, commit: u8, planned: u8) {
        if let Some(i) = self.ids.iter().position(|&h| h == id) {
            self.boxes[i] = Some(commit);
        }
        self.planned = self.planned.max(planned);
    }

    /// What one box holds, for [`crate::Patch::subset_with`]: whole hunks,
    /// and the chosen lines of any hunk that was divided.
    #[must_use]
    pub fn taken(&self, commit: u8) -> Vec<(HunkId, Take)> {
        self.ids
            .iter()
            .enumerate()
            .filter_map(|(i, &id)| {
                self.divided.get(&i).map_or_else(
                    || (self.boxes[i] == Some(commit)).then_some((id, Take::Whole)),
                    |div| {
                        let lines = div.in_box(commit);
                        (!lines.is_empty()).then_some((id, Take::Lines(lines)))
                    },
                )
            })
            .collect()
    }

    /// Where each line of hunk `i` is going, if it has been divided: the
    /// patch lines, and the boxes parallel to them.
    ///
    /// Unlike [`Self::line_plan`] this reaches any hunk, not just the one
    /// being looked at, so a plan can be captured across the whole change.
    #[must_use]
    pub fn division(&self, i: usize) -> Option<(&[usize], &[Option<u8>])> {
        self.divided
            .get(&i)
            .map(|d| (d.at.as_slice(), d.boxes.as_slice()))
    }

    /// Restore a division worked out elsewhere, as when a plan is carried
    /// across a split; see [`crate::plan::Plan`]. `boxes` is parallel to
    /// `lines`, which are the hunk's `+`/`-` lines in this diff.
    pub fn restore_division(
        &mut self,
        id: HunkId,
        lines: Vec<usize>,
        boxes: Vec<Option<u8>>,
        planned: u8,
    ) {
        let Some(i) = self.ids.iter().position(|&h| h == id) else {
            return;
        };
        if lines.len() != boxes.len() || boxes.iter().all(Option::is_none) {
            return;
        }
        self.planned = self.planned.max(planned);
        let restored = Lines { at: lines, boxes };
        // Every line headed for the same commit is that hunk going there
        // whole, which is what it will look like next time round. Keeping it
        // divided would mark it `*` in the list and open the line view for a
        // decision already made.
        if let Some(sole) = restored.sole() {
            self.boxes[i] = Some(sole);
            self.divided.remove(&i);
            return;
        }
        self.boxes[i] = None;
        self.divided.insert(i, restored);
    }

    /// The hunks in one box, whole. A divided hunk is not one of them.
    #[must_use]
    pub fn picked(&self, commit: u8) -> Vec<HunkId> {
        self.ids
            .iter()
            .zip(&self.boxes)
            .filter(|&(_, slot)| *slot == Some(commit))
            .map(|(&id, _)| id)
            .collect()
    }

    /// How many hunks put something in one box. A divided hunk counts once
    /// however many of its lines are in there.
    #[must_use]
    pub fn count(&self, commit: u8) -> usize {
        (0..self.ids.len())
            .filter(|i| {
                self.divided.get(i).map_or_else(
                    || self.boxes[*i] == Some(commit),
                    |div| !div.in_box(commit).is_empty(),
                )
            })
            .count()
    }

    /// How many hunks put something in any box at all.
    #[must_use]
    pub fn assigned(&self) -> usize {
        (0..self.ids.len())
            .filter(|i| {
                self.divided.get(i).map_or_else(
                    || self.boxes[*i].is_some(),
                    |div| div.boxes.iter().any(Option::is_some),
                )
            })
            .count()
    }

    /// Whether the whole change is in one box, which is a description rather
    /// than a split: nothing would be left behind. A divided hunk with a
    /// line left out means something is, so this is false.
    #[must_use]
    pub fn is_everything(&self) -> bool {
        self.pending().len() == 1
            && (0..self.ids.len()).all(|i| {
                self.divided.get(&i).map_or_else(
                    || self.boxes[i].is_some(),
                    |div| div.boxes.iter().all(Option::is_some),
                )
            })
    }

    // --- Dividing a hunk by line ---------------------------------------
    //
    // A hunk is the unit of a split, but one hunk can hold two unrelated
    // edits on neighbouring lines. Dividing opens the hunk up so each of
    // its `+`/`-` lines can be decided on its own; everything above goes on
    // treating the hunk as one thing, because nearly every hunk is.

    /// Open the hunk being looked at, with `lines` from
    /// [`crate::Patch::changed_lines`]. Its lines start where the hunk as a
    /// whole was going, so dividing an assigned hunk decides nothing by
    /// itself.
    ///
    /// A hunk with fewer than two choices in it cannot be divided.
    pub fn divide(&mut self, lines: Vec<usize>) {
        if lines.len() < 2 || self.divided.contains_key(&self.at) {
            return;
        }
        let was = self.boxes.get(self.at).copied().flatten();
        self.boxes[self.at] = None;
        self.divided.insert(
            self.at,
            Lines {
                boxes: vec![was; lines.len()],
                at: lines,
            },
        );
        self.line = 0;
    }

    /// Close the hunk being looked at. Lines that all agree fold back into
    /// a whole-hunk assignment, so a division the user undid leaves no
    /// trace.
    pub fn undivide(&mut self) {
        let Some(div) = self.divided.remove(&self.at) else {
            return;
        };
        let sole = div.sole();
        if sole.is_some() || div.boxes.iter().all(Option::is_none) {
            self.boxes[self.at] = sole;
        } else {
            // A real division being closed: keep it rather than lose it.
            self.divided.insert(self.at, div);
        }
    }

    /// Whether the hunk being looked at is open for line-level assignment.
    #[must_use]
    pub fn is_divided(&self) -> bool {
        self.divided.contains_key(&self.at)
    }

    /// Where each line of the hunk being looked at is going, parallel to
    /// the lines it was divided with. `None` if it is not divided.
    #[must_use]
    pub fn line_plan(&self) -> Option<&[Option<u8>]> {
        self.divided.get(&self.at).map(|d| d.boxes.as_slice())
    }

    /// The line being looked at inside a divided hunk, counted from zero.
    #[must_use]
    pub const fn line_at(&self) -> usize {
        self.line
    }

    /// The commit that line is destined for.
    #[must_use]
    pub fn box_of_line(&self) -> Option<u8> {
        self.divided
            .get(&self.at)
            .and_then(|d| d.boxes.get(self.line).copied().flatten())
    }

    pub fn next_line(&mut self) {
        if let Some(div) = self.divided.get(&self.at) {
            self.line = (self.line + 1).min(div.boxes.len().saturating_sub(1));
        }
    }

    pub const fn prev_line(&mut self) {
        self.line = self.line.saturating_sub(1);
    }

    /// Put the line being looked at in a box, or take it out with `None`.
    pub fn assign_line(&mut self, commit: Option<u8>) {
        let line = self.line;
        if let Some(div) = self.divided.get_mut(&self.at)
            && let Some(slot) = div.boxes.get_mut(line)
        {
            *slot = commit;
        }
        if let Some(n) = commit {
            self.planned = self.planned.max(n);
        }
    }

    /// [`Self::toggle`] for one line.
    pub fn toggle_line(&mut self, commit: u8) {
        let next = if self.box_of_line() == Some(commit) {
            None
        } else {
            Some(commit)
        };
        self.assign_line(next);
    }

    /// How many lines the division of the hunk being looked at has, which
    /// is how many choices it holds.
    #[must_use]
    pub fn lines(&self) -> usize {
        self.divided.get(&self.at).map_or(0, |d| d.boxes.len())
    }

    /// Move the line cursor, clamped to the division.
    pub fn go_to_line(&mut self, line: usize) {
        self.line = line.min(self.lines().saturating_sub(1));
    }

    pub fn first_line(&mut self) {
        self.line = 0;
    }

    pub fn last_line(&mut self) {
        self.line = self.lines().saturating_sub(1);
    }

    /// Assign every line between `from` and `to`, inclusive and in either
    /// order, as a visual selection in the divided view does. Out-of-range
    /// ends are clamped.
    pub fn assign_line_range(&mut self, from: usize, to: usize, commit: Option<u8>) {
        let last = self.lines().saturating_sub(1);
        let (lo, hi) = (from.min(to).min(last), from.max(to).min(last));
        let Some(div) = self.divided.get_mut(&self.at) else {
            return;
        };
        for slot in &mut div.boxes[lo..=hi] {
            *slot = commit;
        }
        if let Some(n) = commit {
            self.planned = self.planned.max(n);
        }
    }

    /// [`Self::toggle_range`] for lines: if every line in the selection is
    /// already in `commit` they all come out, otherwise they all go in, so
    /// pressing the digit twice ends where it started.
    pub fn toggle_line_range(&mut self, from: usize, to: usize, commit: u8) {
        let last = self.lines().saturating_sub(1);
        let (lo, hi) = (from.min(to).min(last), from.max(to).min(last));
        let all_in = self
            .divided
            .get(&self.at)
            .is_some_and(|d| d.boxes[lo..=hi].iter().all(|b| *b == Some(commit)));
        self.assign_line_range(lo, hi, (!all_in).then_some(commit));
    }
}

impl Lines {
    /// The one box every line is in, if they all agree on a box.
    fn sole(&self) -> Option<u8> {
        let first = self.boxes.first().copied().flatten()?;
        self.boxes
            .iter()
            .all(|b| *b == Some(first))
            .then_some(first)
    }

    /// The lines in one box, as patch line indices.
    fn in_box(&self, commit: u8) -> Vec<usize> {
        self.at
            .iter()
            .zip(&self.boxes)
            .filter(|(_, b)| **b == Some(commit))
            .map(|(&l, _)| l)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::patch;

    /// Two files, three hunks: enough to plan more than one commit.
    fn split() -> Split {
        Split::new([
            (HunkId { file: 0, hunk: 0 }, "a.rs".to_owned(), 3),
            (HunkId { file: 0, hunk: 1 }, "a.rs".to_owned(), 2),
            (HunkId { file: 1, hunk: 0 }, "b.rs".to_owned(), 2),
        ])
        .expect("three hunks can be split")
    }

    /// Patch line indices; what they point at doesn't matter here.
    fn divided() -> Split {
        let mut s = split();
        s.divide(vec![4, 5, 6]);
        s
    }

    #[test]
    fn dividing_a_hunk_decides_nothing_by_itself() {
        let s = divided();
        assert!(s.is_divided());
        assert_eq!(s.line_plan(), Some([None, None, None].as_slice()));
        assert_eq!(s.assigned(), 0, "no line is in a box yet");
        assert_eq!(s.box_of(), None);
    }

    /// The lines inherit the hunk's box, so opening an assigned hunk to move
    /// one line out of it doesn't lose the other lines' decision.
    #[test]
    fn a_divided_hunk_keeps_where_it_was_going() {
        let mut s = split();
        s.assign(Some(2));
        s.divide(vec![4, 5, 6]);
        assert_eq!(s.line_plan(), Some([Some(2); 3].as_slice()));
        assert_eq!(s.box_of(), Some(2), "it is still all going to 2");
    }

    #[test]
    fn a_hunk_with_one_choice_in_it_cannot_be_divided() {
        let mut s = split();
        s.divide(vec![4]);
        assert!(!s.is_divided());
    }

    #[test]
    fn lines_go_to_boxes_one_at_a_time() {
        let mut s = divided();
        s.assign_line(Some(1));
        s.next_line();
        s.assign_line(Some(2));
        assert_eq!(s.line_plan(), Some([Some(1), Some(2), None].as_slice()));
        assert_eq!(s.pending(), [1, 2], "one hunk, two commits");
        assert_eq!(s.box_of(), None, "the hunk as a whole is going nowhere");
    }

    #[test]
    fn a_run_of_lines_goes_to_a_commit_at_once() {
        let mut s = divided();
        s.assign_line_range(0, 1, Some(2));
        assert_eq!(s.line_plan(), Some([Some(2), Some(2), None].as_slice()));
        assert_eq!(s.planned(), 2);
    }

    /// Either order, and either end out of range, still means the run
    /// between them.
    #[test]
    fn a_line_range_is_the_same_both_ways_round_and_is_clamped() {
        let mut s = divided();
        s.assign_line_range(2, 0, Some(1));
        assert_eq!(s.line_plan(), Some([Some(1); 3].as_slice()));
        let mut s = divided();
        s.assign_line_range(1, 99, Some(1));
        assert_eq!(s.line_plan(), Some([None, Some(1), Some(1)].as_slice()));
    }

    /// A selection half in a box is made whole rather than flipped line by
    /// line, so the digit twice always ends where it started.
    #[test]
    fn toggling_a_line_range_fills_it_before_emptying_it() {
        let mut s = divided();
        s.assign_line(Some(1));
        s.toggle_line_range(0, 2, 1);
        assert_eq!(
            s.line_plan(),
            Some([Some(1); 3].as_slice()),
            "half in means all in"
        );
        s.toggle_line_range(0, 2, 1);
        assert_eq!(s.line_plan(), Some([None; 3].as_slice()), "now all out");
    }

    #[test]
    fn g_and_shift_g_reach_either_end_of_a_division() {
        let mut s = divided();
        s.last_line();
        assert_eq!(s.line_at(), 2);
        s.first_line();
        assert_eq!(s.line_at(), 0);
        s.go_to_line(99);
        assert_eq!(s.line_at(), 2, "clamped to the last line");
    }

    /// Ranges are meaningless on a hunk that is not divided, and must not
    /// panic or reach for the whole-hunk boxes.
    #[test]
    fn line_ranges_do_nothing_on_an_undivided_hunk() {
        let mut s = split();
        s.assign_line_range(0, 2, Some(1));
        s.toggle_line_range(0, 2, 1);
        assert_eq!(s.plan(), [None, None, None]);
        assert_eq!(s.lines(), 0);
    }

    #[test]
    fn the_same_digit_takes_a_line_back_out() {
        let mut s = divided();
        s.toggle_line(1);
        assert_eq!(s.box_of_line(), Some(1));
        s.toggle_line(1);
        assert_eq!(s.box_of_line(), None);
    }

    #[test]
    fn the_line_cursor_stops_at_either_end() {
        let mut s = divided();
        s.prev_line();
        assert_eq!(s.line_at(), 0);
        for _ in 0..9 {
            s.next_line();
        }
        assert_eq!(s.line_at(), 2);
    }

    /// Only the chosen lines travel, and the other hunks still go whole.
    #[test]
    fn a_box_holds_whole_hunks_and_parts_of_divided_ones() {
        let mut s = divided();
        s.assign_line(Some(1));
        s.next_line();
        s.next_line();
        s.assign_line(Some(1));
        s.last();
        s.assign(Some(1));

        let taken = s.taken(1);
        assert_eq!(
            taken,
            [
                (HunkId { file: 0, hunk: 0 }, Take::Lines(vec![4, 6])),
                (HunkId { file: 1, hunk: 0 }, Take::Whole),
            ]
        );
        assert_eq!(s.count(1), 2, "a divided hunk counts once");
    }

    #[test]
    fn a_divided_hunk_puts_nothing_in_a_box_it_has_no_line_in() {
        let mut s = divided();
        s.assign_line(Some(1));
        assert_eq!(s.taken(2), [] as [(patch::HunkId, patch::Take); 0]);
    }

    /// `picked` is the whole-hunk view, and a divided hunk is not whole.
    #[test]
    fn picked_leaves_divided_hunks_out() {
        let mut s = divided();
        s.assign_line(Some(1));
        assert_eq!(s.picked(1), [] as [patch::HunkId; 0]);
        assert_eq!(s.taken(1).len(), 1);
    }

    /// The shortcut that treats one box holding everything as a description
    /// must not fire while a line is still unspoken for.
    #[test]
    fn a_line_left_out_means_it_is_not_the_whole_change() {
        let mut s = divided();
        s.assign_all(Some(1));
        assert!(s.is_everything(), "assigning all undoes the division");

        let mut s = divided();
        s.first();
        s.assign_line(Some(1));
        s.last();
        s.assign(Some(1));
        s.prev();
        s.assign(Some(1));
        assert_eq!(s.pending(), [1]);
        assert!(
            !s.is_everything(),
            "two lines of the divided hunk stay behind"
        );
    }

    #[test]
    fn deciding_the_hunk_as_a_whole_discards_the_division() {
        let mut s = divided();
        s.assign_line(Some(1));
        s.assign(Some(3));
        assert!(!s.is_divided());
        assert_eq!(s.taken(3), [(HunkId { file: 0, hunk: 0 }, Take::Whole)]);
    }

    /// Closing a division where every line agrees leaves no trace of it.
    #[test]
    fn a_division_the_user_undid_folds_back_up() {
        let mut s = divided();
        s.assign_line(Some(1));
        s.next_line();
        s.assign_line(Some(1));
        s.next_line();
        s.assign_line(Some(1));
        s.undivide();
        assert!(!s.is_divided());
        assert_eq!(s.box_of(), Some(1));

        let mut s = divided();
        s.undivide();
        assert!(!s.is_divided(), "nothing assigned folds up too");
        assert_eq!(s.box_of(), None);
    }

    /// But a real division is not thrown away by closing the view.
    #[test]
    fn closing_a_real_division_keeps_it() {
        let mut s = divided();
        s.assign_line(Some(1));
        s.undivide();
        assert!(
            s.is_divided(),
            "lines disagree, so it still means something"
        );
    }

    #[test]
    fn dividing_remembers_the_highest_box_for_commit_n_of_m() {
        let mut s = divided();
        s.assign_line(Some(3));
        assert_eq!(s.planned(), 3);
    }

    /// One hunk holding one change really cannot be split: whichever way
    /// it went, one of the two commits would be empty.
    #[test]
    fn one_hunk_with_one_change_cannot_be_split() {
        assert!(Split::new([(HunkId { file: 0, hunk: 0 }, "a.rs".to_owned(), 1)]).is_none());
        assert!(Split::new([]).is_none());
    }

    /// But one hunk holding two changes can be, by dividing it.
    #[test]
    fn one_hunk_with_two_changes_can_be_divided() {
        let mut s = Split::new([(HunkId { file: 0, hunk: 0 }, "a.rs".to_owned(), 2)])
            .expect("two changes are two choices");
        assert_eq!(s.total(), 1);
        assert_eq!(s.changes(), 2);
        s.divide(vec![4, 5]);
        assert!(s.is_divided());
        s.assign_line(Some(1));
        assert!(!s.is_everything(), "one line stays behind");
    }

    #[test]
    fn moving_decides_nothing() {
        let mut s = split();
        s.next();
        s.next();
        assert_eq!(s.at(), 2);
        assert_eq!(s.assigned(), 0);
    }

    #[test]
    fn moving_stops_at_either_end() {
        let mut s = split();
        s.prev();
        assert_eq!(s.at(), 0, "already at the first hunk");
        s.last();
        s.next();
        assert_eq!(s.at(), 2, "already at the last hunk");
    }

    #[test]
    fn go_to_moves_to_a_named_hunk_and_ignores_an_unknown_one() {
        let mut s = split();
        s.go_to(HunkId { file: 1, hunk: 0 });
        assert_eq!(s.at(), 2);
        s.go_to(HunkId { file: 9, hunk: 9 });
        assert_eq!(s.at(), 2, "an unknown hunk leaves the cursor alone");
    }

    #[test]
    fn hunks_go_into_numbered_boxes() {
        let mut s = split();
        s.assign(Some(1));
        s.next();
        s.assign(Some(2));
        s.next();
        s.assign(Some(2));
        assert_eq!(s.plan(), [Some(1), Some(2), Some(2)]);
        assert_eq!(s.count(1), 1);
        assert_eq!(s.count(2), 2);
        assert_eq!(s.pending(), [1, 2]);
        assert_eq!(s.next_box(), Some(1), "the lowest box is committed first");
    }

    /// The point of the whole design: a hunk can be moved to another commit,
    /// or taken out again, until the split is actually made.
    #[test]
    fn the_same_key_assigns_and_then_takes_back() {
        let mut s = split();
        s.toggle(1);
        assert_eq!(s.box_of(), Some(1));
        s.toggle(2);
        assert_eq!(s.box_of(), Some(2), "moved to another commit");
        s.toggle(2);
        assert_eq!(s.box_of(), None, "the same box again takes it out");
        assert_eq!(s.assigned(), 0);
    }

    #[test]
    fn a_plan_survives_looking_elsewhere() {
        let mut s = split();
        s.toggle(3);
        s.last();
        s.first();
        assert_eq!(s.box_of(), Some(3));
        assert_eq!(s.count(3), 1);
    }

    #[test]
    fn a_whole_file_can_go_in_one_commit() {
        let mut s = split();
        s.assign_file(Some(1));
        // Both of a.rs's hunks, and not b.rs's.
        assert_eq!(s.plan(), [Some(1), Some(1), None]);
        assert_eq!(s.at(), 0, "assigning does not move");
        s.assign_file(None);
        assert_eq!(s.assigned(), 0);
    }

    #[test]
    fn a_range_is_assigned_in_either_direction_and_clamped() {
        let mut s = split();
        s.assign_range(2, 1, Some(4));
        assert_eq!(s.plan(), [None, Some(4), Some(4)]);
        assert_eq!(s.planned(), 4);
        s.assign_range(0, 99, None);
        assert_eq!(s.assigned(), 0, "an end past the last hunk is clamped");
        assert_eq!(s.at(), 0, "assigning a range does not move");
    }

    /// A range spans files: that is what it adds over `assign_file`.
    #[test]
    fn toggling_a_range_fills_it_then_empties_it() {
        let mut s = split();
        s.assign(Some(1)); // hunk 0 already in box 1
        s.toggle_range(0, 2, 1);
        assert_eq!(s.plan(), [Some(1); 3], "a partly filled range is completed");
        s.toggle_range(0, 2, 1);
        assert_eq!(s.assigned(), 0, "a full range is emptied");

        s.assign(Some(2));
        s.toggle_range(0, 1, 1);
        assert_eq!(s.plan(), [Some(1), Some(1), None], "moved out of box 2");
    }

    #[test]
    fn everything_can_be_assigned_and_cleared() {
        let mut s = split();
        s.assign_all(Some(1));
        assert!(s.is_everything());
        s.assign_all(None);
        assert_eq!(s.assigned(), 0);
        assert!(!s.is_everything());
    }

    /// Everything in one box leaves nothing behind, so it is a description.
    /// Spread across two boxes it is a real split.
    #[test]
    fn one_box_holding_everything_is_not_a_split() {
        let mut s = split();
        s.assign_all(Some(1));
        assert!(s.is_everything());
        s.last();
        s.assign(Some(2));
        assert!(!s.is_everything(), "two boxes is a split");
    }

    #[test]
    fn picked_gives_one_box_in_order() {
        let mut s = split();
        s.last();
        s.assign(Some(1));
        s.first();
        s.assign(Some(1));
        s.next();
        s.assign(Some(2));
        assert_eq!(
            s.picked(1),
            [HunkId { file: 0, hunk: 0 }, HunkId { file: 1, hunk: 0 }]
        );
        assert_eq!(s.picked(2), [HunkId { file: 0, hunk: 1 }]);
        assert_eq!(s.picked(9), [], "an unused box holds nothing");
    }

    /// `planned` outlives committing a box, so "commit 2 of 3" can still say
    /// how many there were.
    #[test]
    fn the_number_planned_only_ever_grows() {
        let mut s = split();
        s.assign(Some(3));
        assert_eq!(s.planned(), 3);
        s.assign(None);
        assert_eq!(s.planned(), 3, "taking it out again does not forget");
        assert_eq!(s.pending(), [], "but nothing is pending");
    }

    #[test]
    fn the_plan_is_parallel_to_the_hunks() {
        let mut s = split();
        s.assign(Some(1));
        s.next();
        assert_eq!(s.plan(), [Some(1), None, None]);
        s.assign(Some(2));
        s.last();
        s.assign(Some(2));
        assert_eq!(s.plan(), [Some(1), Some(2), Some(2)]);
    }

    #[test]
    fn the_path_follows_the_hunk_on_screen() {
        let mut s = split();
        assert_eq!(s.path(), "a.rs");
        s.last();
        assert_eq!(s.path(), "b.rs");
    }

    #[test]
    fn restore_puts_a_hunk_back_in_its_box() {
        let mut s = split();
        s.restore(HunkId { file: 1, hunk: 0 }, 2, 4);
        assert_eq!(s.plan(), [None, None, Some(2)]);
        assert_eq!(s.planned(), 4);
        s.restore(HunkId { file: 9, hunk: 9 }, 1, 4);
        assert_eq!(s.assigned(), 1, "an unknown hunk is ignored");
    }
}
