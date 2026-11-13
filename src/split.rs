//! Writing a chosen part of a change into jj's diff-editor trees.
//!
//! `jj split -i` runs a diff editor with two directories: `left` holds the
//! files as they were before the change, `right` as they are after. Whatever
//! the editor leaves in `right` becomes the first of the two commits, and
//! everything else stays in the second.
//!
//! So jj-compose re-runs itself as that editor ([`apply_selection`]) with a
//! patch of the hunks the user chose, and rewrites `right` to hold the
//! before-state plus only those hunks. A file the patch doesn't mention is
//! reset to its before-state, which is how jj is told the change to it
//! belongs in the other commit.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::patch::{FileDiff, Patch};

/// jj leaves this in the right-hand tree to explain the editor's job; it
/// isn't part of the change.
const INSTRUCTIONS: &str = "JJ-INSTRUCTIONS";

/// Rewrite `right` to hold `left` plus only the changes in `patch_text`.
///
/// # Errors
///
/// If the trees can't be read or written.
pub fn apply_selection(patch_text: &str, left: &Path, right: &Path) -> io::Result<()> {
    let _ = fs::remove_file(right.join(INSTRUCTIONS));
    let patch = Patch::from_text(patch_text);

    let mut chosen: Vec<&str> = Vec::new();
    for file in patch.files() {
        write_chosen(&patch, file, left, right)?;
        chosen.push(&file.path);
    }
    reset_rest(left, right, &chosen)?;
    Ok(())
}

/// Write one file's before-state plus the patch's hunks for it.
fn write_chosen(patch: &Patch, file: &FileDiff, left: &Path, right: &Path) -> io::Result<()> {
    let target = right.join(&file.path);
    // A patch that deletes the file has nothing to write.
    if (file.header.clone()).any(|l| patch.line(l) == "+++ /dev/null") {
        return remove(&target);
    }
    // Absent from `left` means the change adds it, so it starts from nothing.
    let before = read_lines(&left.join(&file.path))?;
    let after = patch.apply(file, &before);

    let mut text = after.join("\n");
    if !after.is_empty() && ends_with_newline(patch, file) {
        text.push('\n');
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, text)
}

/// Whether the file ends in a newline once the patch is applied: it does
/// unless the diff says otherwise with a `\ No newline at end of file`
/// marker on the line after the last one it keeps.
fn ends_with_newline(patch: &Patch, file: &FileDiff) -> bool {
    let Some(hunk) = file.hunks.last() else {
        return true;
    };
    let mut kept = None;
    for l in hunk.lines.clone() {
        match patch.line(l).as_bytes().first() {
            Some(b'\\') if kept == Some(l - 1) => return false,
            Some(b'-') => {}
            _ => kept = Some(l),
        }
    }
    true
}

/// Make every file the patch didn't mention match its before-state, so jj
/// sees no change to it.
fn reset_rest(left: &Path, right: &Path, chosen: &[&str]) -> io::Result<()> {
    for path in relative_files(left)? {
        if chosen.iter().any(|c| Path::new(c) == path) {
            continue;
        }
        let target = right.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(left.join(&path), target)?;
    }
    // A file only in `right` is one the change adds; without it the added
    // file isn't part of this commit.
    for path in relative_files(right)? {
        if chosen.iter().any(|c| Path::new(c) == path) || left.join(&path).exists() {
            continue;
        }
        remove(&right.join(&path))?;
    }
    Ok(())
}

/// Every file under `root`, as paths relative to it.
fn relative_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(rel) = path.strip_prefix(root)
                && rel != Path::new(INSTRUCTIONS)
            {
                out.push(rel.to_path_buf());
            }
        }
    }
    Ok(out)
}

/// The file's lines, or none at all if it doesn't exist.
fn read_lines(path: &Path) -> io::Result<Vec<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().map(str::to_owned).collect()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A diff of two separate changes to one file.
    const TWO_HUNKS: &str = "\
diff --git a/f.txt b/f.txt
index 1111111..2222222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,1 +1,1 @@
-one
+ONE
@@ -7,1 +7,1 @@
-seven
+SEVEN
";

    fn before() -> Vec<String> {
        (1..=7)
            .map(|n| ["one", "two", "three", "four", "five", "six", "seven"][n - 1].to_owned())
            .collect()
    }

    #[test]
    fn one_chosen_hunk_leaves_the_other_alone() {
        let patch = Patch::from_text(TWO_HUNKS);
        let ids = patch.ids();
        assert_eq!(ids.len(), 2);

        let first = Patch::from_text(&patch.subset(&ids[..1]));
        let after = first.apply(&first.files()[0], &before());
        assert_eq!(after[0], "ONE");
        assert_eq!(after[6], "seven", "the second hunk must not be applied");

        let second = Patch::from_text(&patch.subset(&ids[1..]));
        let after = second.apply(&second.files()[0], &before());
        assert_eq!(after[0], "one");
        assert_eq!(after[6], "SEVEN");
    }

    #[test]
    fn both_hunks_rebuild_the_whole_change() {
        let patch = Patch::from_text(TWO_HUNKS);
        let after = patch.apply(&patch.files()[0], &before());
        assert_eq!(after[0], "ONE");
        assert_eq!(after[6], "SEVEN");
        assert_eq!(after.len(), 7);
    }

    #[test]
    fn choosing_nothing_from_a_file_leaves_it_out_of_the_patch() {
        let patch = Patch::from_text(TWO_HUNKS);
        assert_eq!(patch.subset(&[]), "");
    }

    #[test]
    fn an_added_file_applies_against_nothing() {
        let added = "\
diff --git a/new.txt b/new.txt
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world
";
        let patch = Patch::from_text(added);
        let after = patch.apply(&patch.files()[0], &[]);
        assert_eq!(after, ["hello", "world"]);
    }

    #[test]
    fn the_subset_patch_keeps_the_file_header_but_drops_blob_hashes() {
        let patch = Patch::from_text(TWO_HUNKS);
        let text = patch.subset(&patch.ids()[..1]);
        assert!(text.starts_with("diff --git a/f.txt b/f.txt\n"));
        assert!(text.contains("--- a/f.txt\n+++ b/f.txt\n"));
        assert!(!text.contains("index "), "{text}");
        assert!(!text.contains("seven"), "{text}");
    }
}

