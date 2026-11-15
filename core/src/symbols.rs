//! Working out which function or type a hunk sits inside.
//!
//! `git diff` writes the enclosing definition into the hunk header —
//! `@@ -4,7 +4,7 @@ fn alpha() {` — but `jj diff --git` does not, so it has
//! to be recovered from the file itself. Knowing that three hunks are all in
//! `fn resolve` is usually what tells you they belong in one commit.

use crate::{Jj, Patch, Result, Target, patch::Hunk};

/// The definition a line sits inside, or `None` at the top level.
///
/// This is git's own heuristic: the nearest line at or above `at` that starts
/// in the first column with something name-like. It knows nothing about any
/// particular language, which is why it works for most of them.
#[must_use]
pub fn enclosing(lines: &[String], at: usize) -> Option<String> {
    lines
        .iter()
        .take(at.saturating_add(1).min(lines.len()))
        .rev()
        .find(|line| line.starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '$'))
        .map(|line| tidy(line))
}

/// Where the hunk's first real change sits in the file as it is now.
///
/// A hunk starts three lines of context early, which can be in the previous
/// function entirely; anchoring on the context start would then report the
/// wrong one. The first `+` or `-` line is what the hunk is actually about.
fn first_change(patch: &Patch, hunk: &Hunk) -> usize {
    let mut at = hunk.new.start;
    for i in hunk.lines.clone() {
        match patch.line(i).as_bytes().first() {
            // A removed line is still the start of the change, even though
            // it takes up no room in the new file.
            Some(b'+' | b'-') => return at,
            // Context advances; a "\ No newline" marker is not a line.
            Some(b'\\') => {}
            _ => at += 1,
        }
    }
    hunk.new.start
}

/// A definition line as a label: no trailing brace, no runaway length.
fn tidy(line: &str) -> String {
    const MAX: usize = 40;
    let text = line.trim_end().trim_end_matches(['{', ' ']).trim();
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let cut: String = text.chars().take(MAX - 1).collect();
    format!("{cut}…")
}

/// The enclosing definition of every hunk in `patch`, in the order
/// [`Patch::ids`] gives them.
///
/// One `jj file show` per file, so a diff of twenty files costs twenty
/// cheap calls. A file the change deletes has no "after" content and gets
/// `None`, as does anything that can't be read.
///
/// # Errors
///
/// Never fails as a whole: a file that can't be read yields `None` for its
/// hunks rather than aborting the lot. The `Result` is for future use.
pub fn for_patch(jj: &Jj, target: &Target, patch: &Patch) -> Result<Vec<Option<String>>> {
    let mut out = Vec::new();
    for file in patch.files() {
        // The hunk's `new` side indexes into the file as it is now, which is
        // the revision itself; no need to work out its parent.
        let lines = jj.file_lines(target, &file.path).unwrap_or_default();
        for hunk in &file.hunks {
            out.push(if lines.is_empty() {
                None
            } else {
                enclosing(&lines, first_change(patch, hunk))
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    const CODE: &str = "\
fn alpha() {
    let a = 1;
    let b = 2;
}

impl Thing {
    fn gamma(&self) {
        self.count += 1;
    }
}
";

    #[test]
    fn a_line_inside_a_function_names_that_function() {
        let code = lines(CODE);
        assert_eq!(enclosing(&code, 1).as_deref(), Some("fn alpha()"));
        assert_eq!(enclosing(&code, 2).as_deref(), Some("fn alpha()"));
    }

    /// Indented definitions don't start in the first column, so the nearest
    /// one above is the `impl` — which is what git reports too.
    #[test]
    fn an_indented_definition_reports_its_container() {
        let code = lines(CODE);
        assert_eq!(enclosing(&code, 7).as_deref(), Some("impl Thing"));
    }

    #[test]
    fn a_definition_line_names_itself() {
        assert_eq!(enclosing(&lines(CODE), 0).as_deref(), Some("fn alpha()"));
    }

    #[test]
    fn nothing_above_means_no_symbol() {
        let code = lines("    indented\n    more\n");
        assert_eq!(enclosing(&code, 1), None);
    }

    #[test]
    fn out_of_range_is_not_a_panic() {
        assert_eq!(
            enclosing(&lines(CODE), 9_999).as_deref(),
            Some("impl Thing")
        );
        assert_eq!(enclosing(&[], 0), None);
    }

    /// A hunk's three lines of leading context can start inside the
    /// previous function; the symbol must follow the change, not the context.
    #[test]
    fn the_symbol_follows_the_changed_line_not_the_hunk_start() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -12,7 +12,7 @@
 }
 
 fn beta() {
-    let a = 1;
+    let a = 2;
     let b = 3;
 }
";
        let patch = Patch::from_text(diff);
        let hunk = &patch.files()[0].hunks[0];
        // The hunk starts at line 12 (0-based 11), three context lines above
        // the change, which is inside `fn beta`.
        assert_eq!(hunk.new.start, 11);
        assert_eq!(first_change(&patch, hunk), 14);
    }

    #[test]
    fn a_long_signature_is_cut_short() {
        let code = lines(
            "pub fn a_very_long_function_name_that_goes_on_and_on(x: usize) -> bool {\n    x > 0\n",
        );
        let label = enclosing(&code, 1).unwrap();
        assert!(label.ends_with('…'), "{label}");
        assert!(label.chars().count() <= 40, "{label}");
    }

    #[test]
    fn the_trailing_brace_is_dropped() {
        assert_eq!(
            enclosing(&lines("struct Thing {\n    a: u8,\n"), 1).as_deref(),
            Some("struct Thing")
        );
    }
}
