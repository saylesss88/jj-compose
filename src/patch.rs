//! A git-format diff parsed into files and hunks.

use std::ops::Range;

/// One file's part of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Path after the change (the old path for a deletion).
    pub path: String,
    /// The `diff --git` line and the header lines after it.
    pub header: Range<usize>,
    pub hunks: Vec<Hunk>,
    /// Binary files have no hunks, only a "Binary files differ" line.
    pub binary: bool,
    pub added: usize,
    pub removed: usize,
}

/// One `@@` section of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Index of the `@@ -a,b +c,d @@` line.
    pub header: usize,
    /// The context, `+` and `-` lines that follow it.
    pub lines: Range<usize>,
}

/// Split `lines` (one diff, without colors) into files and hunks.
pub fn parse<S: AsRef<str>>(lines: &[S]) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let line = line.as_ref();
        // Content lines always start with ` `, `+`, `-` or `\`, so these
        // markers can't be confused with a line of code.
        if let Some(paths) = line.strip_prefix("diff --git ") {
            files.push(FileDiff {
                path: new_path(paths),
                header: i..i + 1,
                hunks: Vec::new(),
                binary: false,
                added: 0,
                removed: 0,
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            file.hunks.push(Hunk {
                header: i,
                lines: i + 1..i + 1,
            });
        } else if let Some(hunk) = file.hunks.last_mut() {
            hunk.lines.end = i + 1;
            match line.as_bytes().first() {
                Some(b'+') => file.added += 1,
                Some(b'-') => file.removed += 1,
                _ => {}
            }
        } else {
            file.header.end = i + 1;
            if line.starts_with("Binary files") || line == "GIT binary patch" {
                file.binary = true;
            }
        }
    }
    files
}

/// The `b/...` path from `a/old b/new`.
fn new_path(paths: &str) -> String {
    paths
        .rsplit_once(" b/")
        .map_or(paths, |(_, new)| new)
        .to_owned()
}

/// Remove ANSI escape sequences (`ESC [ ... final-byte`), leaving the text.
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.next() == Some('[') {
                // Parameters and intermediates, then one final byte @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
