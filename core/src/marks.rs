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

use crate::HunkId;

/// The hunks of a change, which one is being looked at, and which commit
/// each is destined for.
#[derive(Debug, Clone)]
pub struct Split {
    /// Every hunk in the diff, in the order they appear.
    ids: Vec<HunkId>,
    /// The file each hunk belongs to, for assigning a whole file at once.
    paths: Vec<String>,
    /// The hunk being looked at.
    at: usize,
    /// Which commit each hunk is destined for, parallel to `ids`. `None`
    /// means it stays behind in what is left of the change.
    boxes: Vec<Option<u8>>,
    /// The highest box number ever used, so "commit 2 of 3" can still say
    /// "of 3" once commit 1 has been made.
    planned: u8,
}

impl Split {
    /// Every hunk of the change, as its id and the path of the file it is in.
    ///
    /// `None` when there is nothing to split: with a single hunk, one of the
    /// two commits would come out empty whichever way it went.
    pub fn new(hunks: impl IntoIterator<Item = (HunkId, String)>) -> Option<Self> {
        let (ids, paths): (Vec<_>, Vec<_>) = hunks.into_iter().unzip();
        (ids.len() > 1).then(|| Self {
            boxes: vec![None; ids.len()],
            ids,
            paths,
            at: 0,
            planned: 0,
        })
    }

    /// The hunk being looked at, counted from zero.
    #[must_use]
    pub const fn at(&self) -> usize {
        self.at
    }

    /// How many hunks the change has. Never fewer than two.
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

    /// The commit the hunk being looked at is destined for.
    #[must_use]
    pub fn box_of(&self) -> Option<u8> {
        self.boxes.get(self.at).copied().flatten()
    }

    /// How many boxes were ever planned, which outlives committing them.
    #[must_use]
    pub const fn planned(&self) -> u8 {
        self.planned
    }

    /// The boxes that still hold hunks, lowest first.
    #[must_use]
    pub fn pending(&self) -> Vec<u8> {
        let mut used: Vec<u8> = self.boxes.iter().flatten().copied().collect();
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

    /// The hunks in one box, for [`crate::Patch::subset`].
    #[must_use]
    pub fn picked(&self, commit: u8) -> Vec<HunkId> {
        self.ids
            .iter()
            .zip(&self.boxes)
            .filter(|&(_, slot)| *slot == Some(commit))
            .map(|(&id, _)| id)
            .collect()
    }

    /// How many hunks are in one box.
    #[must_use]
    pub fn count(&self, commit: u8) -> usize {
        self.boxes
            .iter()
            .filter(|slot| **slot == Some(commit))
            .count()
    }

    /// How many hunks are in any box at all.
    #[must_use]
    pub fn assigned(&self) -> usize {
        self.boxes.iter().flatten().count()
    }

    /// Whether every hunk is in the same single box, which is a description
    /// rather than a split: nothing would be left behind.
    #[must_use]
    pub fn is_everything(&self) -> bool {
        self.pending().len() == 1 && self.assigned() == self.ids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two files, three hunks: enough to plan more than one commit.
    fn split() -> Split {
        Split::new([
            (HunkId { file: 0, hunk: 0 }, "a.rs".to_owned()),
            (HunkId { file: 0, hunk: 1 }, "a.rs".to_owned()),
            (HunkId { file: 1, hunk: 0 }, "b.rs".to_owned()),
        ])
        .expect("three hunks can be split")
    }

    #[test]
    fn a_single_hunk_cannot_be_split() {
        assert!(Split::new([(HunkId { file: 0, hunk: 0 }, "a.rs".to_owned())]).is_none());
        assert!(Split::new([]).is_none());
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
