//! Carrying a split plan across a split.
//!
//! A [`HunkId`] says where a hunk sits in one particular diff, so it does not
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

/// Which commit each still-uncommitted hunk belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    entries: Vec<Entry>,
    planned: u8,
}

impl Plan {
    /// Record where every assigned hunk is headed.
    #[must_use]
    pub fn capture(patch: &Patch, split: &Split) -> Self {
        let mut entries = Vec::new();
        for (id, commit) in patch.ids().into_iter().zip(split.plan()) {
            let Some(commit) = *commit else { continue };
            let Some(file) = patch.files().get(id.file) else {
                continue;
            };
            let Some(hunk) = file.hunks.get(id.hunk) else {
                continue;
            };
            entries.push(Entry {
                path: file.path.clone(),
                content: patch.hunk_content(hunk),
                commit,
            });
        }
        Self {
            entries,
            planned: split.planned(),
        }
    }

    /// Whether anything is still waiting to be committed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many boxes were planned, including ones already committed.
    #[must_use]
    pub const fn planned(&self) -> u8 {
        self.planned
    }

    /// Re-find each planned hunk in a freshly loaded diff and put it back in
    /// its box. Hunks that are gone were the ones just committed.
    pub fn apply(&self, patch: &Patch, split: &mut Split) {
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
        Split::new(
            patch
                .ids()
                .into_iter()
                .map(|id| (id, patch.files()[id.file].path.clone())),
        )
        .expect("more than one hunk")
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
