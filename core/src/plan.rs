//! Carrying a split plan across a split.
//!
//! A [`crate::HunkId`] says where a hunk sits in one particular diff, so it does not
//! survive committing part of the change: the remainder is diffed against a
//! new parent, and every later hunk's line numbers shift. What does survive
//! is the hunk's own text — the context and `+`/`-` lines are byte for byte
//! what they were.
//!
//! So a plan spanning several commits is keyed by content. Before each split
//! it is [captured](Plan::capture) from the current diff, and after the
//! reload it is [applied](Plan::apply) to the new one, which re-finds every
//! hunk that has not been committed yet.

use crate::{Patch, Split};

/// One hunk's place in the plan, as something that outlives a split.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: String,
    /// The hunk's lines, without its `@@` header: the header is the one part
    /// a split does change.
    content: Vec<String>,
    commit: u8,
}

/// One line of a divided hunk, as something that outlives a split.
///
/// A divided hunk's content changes when part of it is committed, so the
/// hunk itself cannot be the key. The line's own text can be: committing a
/// line removes it from what is left, and the lines that remain are still in
/// the order they were.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LineEntry {
    path: String,
    /// The line, sign column and all.
    text: String,
    commit: u8,
}

/// Which commit each still-uncommitted hunk belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    entries: Vec<Entry>,
    /// The lines of divided hunks, in the order they appear in the change.
    lines: Vec<LineEntry>,
    planned: u8,
}

impl Plan {
    /// Record where every assigned hunk is headed.
    #[must_use]
    pub fn capture(patch: &Patch, split: &Split) -> Self {
        let mut entries = Vec::new();
        let mut lines = Vec::new();
        // The box about to be committed is the lowest pending one. Its lines
        // will not be in the change afterwards, so recording them would
        // leave the matching in `apply_lines` looking for lines that are
        // gone -- and, where two lines read the same, matching the survivor
        // against the wrong box.
        let committing = split.next_box();
        for (i, id) in patch.ids().into_iter().enumerate() {
            let Some(file) = patch.files().get(id.file) else {
                continue;
            };
            let Some(hunk) = file.hunks.get(id.hunk) else {
                continue;
            };
            // A divided hunk is recorded line by line; its own content will
            // not survive its lines being committed separately.
            if let Some((at, boxes)) = split.division(i) {
                for (&l, commit) in at.iter().zip(boxes) {
                    match *commit {
                        Some(c) if Some(c) != committing => lines.push(LineEntry {
                            path: file.path.clone(),
                            text: patch.line(l).to_owned(),
                            commit: c,
                        }),
                        _ => {}
                    }
                }
                continue;
            }
            if let Some(commit) = split.plan().get(i).copied().flatten() {
                entries.push(Entry {
                    path: file.path.clone(),
                    content: patch.hunk_content(hunk),
                    commit,
                });
            }
        }
        Self {
            entries,
            lines,
            planned: split.planned(),
        }
    }

    /// Whether anything is still waiting to be committed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.lines.is_empty()
    }

    /// How many boxes were planned, including ones already committed.
    #[must_use]
    pub const fn planned(&self) -> u8 {
        self.planned
    }

    /// Re-find each planned hunk in a freshly loaded diff and put it back in
    /// its box. Hunks that are gone were the ones just committed.
    pub fn apply(&self, patch: &Patch, split: &mut Split) {
        let mut whole = Vec::new();
        for (id, file) in patch
            .ids()
            .into_iter()
            .filter_map(|id| patch.files().get(id.file).map(|f| (id, f)))
        {
            let Some(hunk) = file.hunks.get(id.hunk) else {
                continue;
            };
            let content = patch.hunk_content(hunk);
            if let Some(entry) = self
                .entries
                .iter()
                .find(|e| e.path == file.path && e.content == content)
            {
                split.restore(id, entry.commit, self.planned);
                whole.push(id);
            }
        }
        self.apply_lines(patch, split, &whole);
    }

    /// Re-find the lines of each divided hunk.
    ///
    /// The stored lines are matched against the file's `+`/`-` lines in
    /// order, taking each as it comes. Order is what makes this exact:
    /// splitting never reorders a file's lines, and the lines just
    /// committed are gone from the change, so whatever is left lines up
    /// with what was recorded even when two of them read the same.
    fn apply_lines(&self, patch: &Patch, split: &mut Split, whole: &[crate::HunkId]) {
        for (index, file) in patch.files().iter().enumerate() {
            let mut stored = self.lines.iter().filter(|e| e.path == file.path).peekable();
            for (h, hunk) in file.hunks.iter().enumerate() {
                let id = crate::HunkId {
                    file: index,
                    hunk: h,
                };
                let changed = patch.changed_lines(hunk);
                let mut boxes = vec![None; changed.len()];
                for (n, &l) in changed.iter().enumerate() {
                    if stored.peek().is_some_and(|e| e.text == patch.line(l)) {
                        // `peek` just confirmed there is one.
                        if let Some(e) = stored.next() {
                            boxes[n] = Some(e.commit);
                        }
                    }
                }
                // A hunk already restored whole is not divided after all.
                if !whole.contains(&id) {
                    split.restore_division(id, changed, boxes, self.planned);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HunkId;

    /// Three hunks in one file, far enough apart to stay separate.
    const BEFORE: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,3 +1,5 @@
-one
+ONE
+EXTRA
+MORE
 two
 three
@@ -20,3 +22,3 @@
-twenty
+TWENTY
 x
 y
@@ -30,3 +32,3 @@
-thirty
+THIRTY
 p
 q
";

    /// The same change with the first hunk committed: it is gone, and the
    /// others have shifted on the old side by the two lines it added.
    const AFTER: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -22,3 +22,3 @@
-twenty
+TWENTY
 x
 y
@@ -32,3 +32,3 @@
-thirty
+THIRTY
 p
 q
";

    fn split_of(patch: &Patch) -> Split {
        Split::new(patch.ids().into_iter().map(|id| {
            let file = &patch.files()[id.file];
            let changes = patch.changed_lines(&file.hunks[id.hunk]).len();
            (id, file.path.clone(), changes)
        }))
        .expect("more than one hunk")
    }

    /// A hunk divided across two commits: after commit 1 is made, the
    /// hunk's content has changed, so only its lines can find it again.
    /// A one-hunk change, splittable because the hunk holds several changes.
    fn split_of_one(patch: &Patch) -> Split {
        Split::new(patch.ids().into_iter().map(|id| {
            let file = &patch.files()[id.file];
            let changes = patch.changed_lines(&file.hunks[id.hunk]).len();
            (id, file.path.clone(), changes)
        }))
        .expect("divisible")
    }

    /// A division is not limited to two commits, nor to one split. Three
    /// boxes in one hunk, committed one after another: boxes 2 and 3 are
    /// both re-found after commit 1, and box 3 again after commit 2.
    #[test]
    fn a_division_can_span_every_box_and_outlive_several_splits() {
        const R0: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,7 +1,7 @@
 head
-aaa
+AAA
-bbb
+BBB
-ccc
+CCC
 tail
";
        // After commit 1: aaa is renamed, bbb and ccc are not.
        let r1 = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,7 +1,7 @@
 head
 AAA
-bbb
+BBB
-ccc
+CCC
 tail
";
        let r2 = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,7 +1,7 @@
 head
 AAA
 BBB
-ccc
+CCC
 tail
";
        let p0 = Patch::from_text(R0);
        let mut s0 = split_of_one(&p0);
        let h = &p0.files()[0].hunks[0];
        s0.divide(p0.changed_lines(h));
        // Paired order: -aaa +AAA -bbb +BBB -ccc +CCC.
        for commit in [1u8, 1, 2, 2, 3, 3] {
            s0.assign_line(Some(commit));
            s0.next_line();
        }
        assert_eq!(s0.pending(), [1, 2, 3]);
        let plan0 = Plan::capture(&p0, &s0);

        let p1 = Patch::from_text(r1);
        let mut s1 = split_of_one(&p1);
        plan0.apply(&p1, &mut s1);
        s1.first();
        assert_eq!(
            s1.line_plan(),
            Some([Some(2), Some(2), Some(3), Some(3)].as_slice()),
            "boxes 2 and 3 both survived the first split"
        );
        assert_eq!(s1.next_box(), Some(2));
        assert_eq!(s1.planned(), 3);

        let plan1 = Plan::capture(&p1, &s1);
        let p2 = Patch::from_text(r2);
        let mut s2 = split_of_one(&p2);
        plan1.apply(&p2, &mut s2);
        s2.first();
        assert_eq!(s2.next_box(), Some(3), "box 3 survived two splits");
        assert_eq!(s2.box_of(), Some(3));
        assert!(
            !s2.is_divided(),
            "all that is left agrees, so it is the hunk going whole"
        );
    }

    #[test]
    fn a_division_survives_its_hunk_being_part_committed() {
        const DIVIDED: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,4 +1,4 @@
 fn run() {
-    let cnt = 0;
-    if cnt > 1 {
+    let count = 0;
+    if count >= 1 {
 }
@@ -20,1 +20,1 @@
-x
+X
";
        // The remainder after the rename is committed: the same file, one
        // hunk whose content is nothing like what was captured.
        const REST: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,4 +1,4 @@
 fn run() {
     let count = 0;
-    if cnt > 1 {
+    if count >= 1 {
 }
@@ -20,1 +20,1 @@
-x
+X
";
        let before = Patch::from_text(DIVIDED);
        let mut split = split_of(&before);
        let hunk = &before.files()[0].hunks[0];
        split.divide(before.changed_lines(hunk));
        // The rename pair to commit 1, the logic pair to commit 2.
        split.assign_line(Some(1));
        split.next_line();
        split.assign_line(Some(1));
        split.next_line();
        split.assign_line(Some(2));
        split.next_line();
        split.assign_line(Some(2));
        split.last();
        split.assign(Some(3));

        let plan = Plan::capture(&before, &split);
        assert!(!plan.is_empty());

        let rest = Patch::from_text(REST);
        let mut next = split_of(&rest);
        plan.apply(&rest, &mut next);

        next.first();
        assert_eq!(
            next.box_of(),
            Some(2),
            "the lines commit 1 took are gone; commit 2's were found again, \
             and agreeing on one box makes it the hunk going whole"
        );
        assert!(!next.is_divided(), "nothing left to decide line by line");
        next.last();
        assert_eq!(next.box_of(), Some(3), "the whole hunk too");
        assert_eq!(next.next_box(), Some(2));
        assert_eq!(next.planned(), 3);
    }

    /// Two identical lines, one taken and one not. The one left behind is
    /// matched in order rather than by counting, so it lands in its own box
    /// instead of the taken one's.
    #[test]
    fn identical_lines_are_matched_in_order() {
        const DUPES: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,4 +1,6 @@
 a
+}
 b
+}
 c
@@ -20,1 +20,1 @@
-x
+X
";
        const REST: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,5 +1,6 @@
 a
 }
 b
+}
 c
@@ -20,1 +20,1 @@
-x
+X
";
        let before = Patch::from_text(DUPES);
        let mut split = split_of(&before);
        let hunk = &before.files()[0].hunks[0];
        split.divide(before.changed_lines(hunk));
        split.assign_line(Some(1)); // the first `+}`
        split.next_line();
        split.assign_line(Some(2)); // the second `+}`

        let plan = Plan::capture(&before, &split);
        let rest = Patch::from_text(REST);
        let mut next = split_of(&rest);
        plan.apply(&rest, &mut next);

        next.first();
        assert_eq!(
            next.box_of(),
            Some(2),
            "the surviving `+}}` is commit 2's, not commit 1's"
        );
    }

    #[test]
    fn a_plan_survives_the_hunk_numbers_changing() {
        let before = Patch::from_text(BEFORE);
        let mut split = split_of(&before);
        split.assign(Some(1)); // hunk 0 → commit 1
        split.next();
        split.assign(Some(2)); // hunk 1 → commit 2
        split.next();
        split.assign(Some(3)); // hunk 2 → commit 3
        let plan = Plan::capture(&before, &split);
        assert_eq!(plan.planned(), 3);

        // Commit 1 has been made; the diff is now the other two hunks, whose
        // line numbers have moved.
        let after = Patch::from_text(AFTER);
        let mut next = split_of(&after);
        assert_eq!(next.plan(), [None, None], "a fresh split knows nothing");
        plan.apply(&after, &mut next);

        assert_eq!(next.plan(), [Some(2), Some(3)], "both found again");
        assert_eq!(next.planned(), 3, "still three commits were planned");
        assert_eq!(next.next_box(), Some(2));
    }

    #[test]
    fn unassigned_hunks_are_not_recorded() {
        let before = Patch::from_text(BEFORE);
        let mut split = split_of(&before);
        split.assign(Some(1));
        let plan = Plan::capture(&before, &split);
        assert_eq!(plan.entries.len(), 1);

        let after = Patch::from_text(AFTER);
        let mut next = split_of(&after);
        plan.apply(&after, &mut next);
        assert_eq!(next.assigned(), 0, "nothing left to restore");
    }

    #[test]
    fn an_empty_plan_changes_nothing() {
        let patch = Patch::from_text(AFTER);
        let mut split = split_of(&patch);
        let plan = Plan::default();
        assert!(plan.is_empty());
        plan.apply(&patch, &mut split);
        assert_eq!(split.assigned(), 0);
    }

    /// Two files can hold textually identical hunks; the path keeps them
    /// apart so a plan never puts one in the other's commit.
    #[test]
    fn identical_hunks_in_different_files_stay_distinct() {
        let text = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-x
+y
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1,1 +1,1 @@
-x
+y
";
        let patch = Patch::from_text(text);
        let mut split = split_of(&patch);
        split.assign(Some(1));
        let plan = Plan::capture(&patch, &split);

        let mut next = split_of(&patch);
        plan.apply(&patch, &mut next);
        assert_eq!(next.plan(), [Some(1), None], "only a.rs's hunk was planned");
        assert_eq!(next.picked(1), [HunkId { file: 0, hunk: 0 }]);
    }
}
