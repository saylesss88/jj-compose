//! Choosing which hunks of a change go into the next commit.
//!
//! Unlike `git add -p`, where each hunk is decided once as it goes past, the
//! position and the marks here are independent: moving decides nothing, and
//! a mark can be changed until the commit is made. Deciding a hunk well
//! usually means having seen the others first.

use crate::HunkId;

/// The hunks of a change, which one is being looked at, and which are marked
/// to go in the commit being written.
#[derive(Debug, Clone)]
pub struct Split {
    /// Every hunk in the diff, in the order they appear.
    ids: Vec<HunkId>,
    /// The file each hunk belongs to, for marking a whole file at once.
    paths: Vec<String>,
    /// The hunk being looked at.
    at: usize,
    /// Whether each hunk is marked, parallel to `ids`.
    marked: Vec<bool>,
}

impl Split {
    /// Every hunk of the change, as its id and the path of the file it is in.
    ///
    /// `None` when there is nothing to split: with a single hunk, one of the
    /// two commits would come out empty whichever way it went.
    pub fn new(hunks: impl IntoIterator<Item = (HunkId, String)>) -> Option<Self> {
        let (ids, paths): (Vec<_>, Vec<_>) = hunks.into_iter().unzip();
        (ids.len() > 1).then(|| Self {
            marked: vec![false; ids.len()],
            ids,
            paths,
            at: 0,
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
    pub fn path(&self) -> &str {
        self.paths.get(self.at).map_or("", String::as_str)
    }

    /// Whether each hunk is marked, in the order the hunks appear.
    #[must_use]
    pub fn marks(&self) -> &[bool] {
        &self.marked
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
    /// moving in the list moves the hunk on screen, so marking always acts
    /// on the thing you are looking at.
    pub fn go_to(&mut self, id: HunkId) {
        if let Some(i) = self.ids.iter().position(|&h| h == id) {
            self.at = i;
        }
    }

    pub fn mark(&mut self, on: bool) {
        if let Some(slot) = self.marked.get_mut(self.at) {
            *slot = on;
        }
    }

    pub fn toggle(&mut self) {
        self.mark(!self.is_marked());
    }

    /// Mark or unmark every hunk in the file being looked at, wherever in
    /// the change those hunks sit.
    pub fn mark_file(&mut self, on: bool) {
        let Some(path) = self.paths.get(self.at).cloned() else {
            return;
        };
        for i in 0..self.paths.len() {
            if self.paths[i] == path {
                self.marked[i] = on;
            }
        }
    }

    pub fn mark_all(&mut self, on: bool) {
        self.marked.fill(on);
    }

    #[must_use]
    pub fn is_marked(&self) -> bool {
        self.marked.get(self.at).copied().unwrap_or(false)
    }

    /// One character per hunk, so the shape of the whole change is visible
    /// at a glance while only one hunk is on screen: `#` the hunk being
    /// looked at and marked, `o` it and unmarked, `+` marked elsewhere, `·`
    /// untouched.
    #[must_use]
    pub fn strip(&self) -> String {
        self.marked
            .iter()
            .enumerate()
            .map(|(i, &on)| match (i == self.at, on) {
                (true, true) => '#',
                (true, false) => 'o',
                (false, true) => '+',
                (false, false) => '·',
            })
            .collect()
    }

    /// The marked hunks, for [`Patch::subset`](crate::Patch::subset).
    #[must_use]
    pub fn picked(&self) -> Vec<HunkId> {
        self.ids
            .iter()
            .zip(&self.marked)
            .filter(|&(_, on)| *on)
            .map(|(&id, _)| id)
            .collect()
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.marked.iter().filter(|&&on| on).count()
    }

    /// Whether every hunk is marked, which is a description, not a split:
    /// nothing would be left behind.
    #[must_use]
    pub fn is_everything(&self) -> bool {
        self.count() == self.ids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two files, three hunks: enough to split more than one way.
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
    fn go_to_moves_to_a_named_hunk_and_ignores_an_unknown_one() {
        let mut s = split();
        s.go_to(HunkId { file: 1, hunk: 0 });
        assert_eq!(s.at(), 2);
        s.go_to(HunkId { file: 9, hunk: 9 });
        assert_eq!(s.at(), 2, "an unknown hunk leaves the cursor alone");
    }

    #[test]
    fn moving_decides_nothing() {
        let mut s = split();
        s.next();
        s.next();
        assert_eq!(s.at(), 2);
        assert_eq!(s.count(), 0);
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

    /// The point of the whole design: a hunk passed over can be marked once
    /// the rest of the change has been read.
    #[test]
    fn a_mark_can_be_changed_after_looking_elsewhere() {
        let mut s = split();
        s.next();
        s.next();
        s.first();
        s.toggle();
        assert_eq!(s.marks(), [true, false, false]);
        s.toggle();
        assert_eq!(s.marks(), [false, false, false]);
    }

    #[test]
    fn marks_survive_moving_around() {
        let mut s = split();
        s.toggle();
        s.last();
        s.first();
        assert!(s.is_marked());
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn a_file_is_marked_wherever_its_hunks_sit() {
        let mut s = split();
        s.mark_file(true);
        // Both of a.rs's hunks, and not b.rs's.
        assert_eq!(s.marks(), [true, true, false]);
        assert_eq!(s.at(), 0, "marking does not move");
        s.mark_file(false);
        assert_eq!(s.marks(), [false, false, false]);
    }

    #[test]
    fn everything_can_be_marked_and_cleared() {
        let mut s = split();
        s.mark_all(true);
        assert!(s.is_everything());
        s.mark_all(false);
        assert_eq!(s.count(), 0);
        assert!(!s.is_everything());
    }

    #[test]
    fn picked_gives_the_marked_hunks_in_order() {
        let mut s = split();
        s.last();
        s.mark(true);
        s.first();
        s.mark(true);
        assert_eq!(
            s.picked(),
            [HunkId { file: 0, hunk: 0 }, HunkId { file: 1, hunk: 0 }]
        );
    }

    #[test]
    fn the_strip_shows_every_hunk_and_where_you_are() {
        let mut s = split();
        s.toggle();
        s.next();
        assert_eq!(s.strip(), "+o·");
        s.toggle();
        assert_eq!(s.strip(), "+#·");
    }

    #[test]
    fn the_path_follows_the_hunk_on_screen() {
        let mut s = split();
        assert_eq!(s.path(), "a.rs");
        s.last();
        assert_eq!(s.path(), "b.rs");
    }
}
