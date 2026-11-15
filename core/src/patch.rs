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
    /// The lines this hunk replaces in the file as it was before the change:
    /// `a` and `b` of `@@ -a,b +c,d @@`, as a 0-based range.
    pub old: Range<usize>,
    /// The lines it occupies in the file as it is after: `c` and `d`.
    pub new: Range<usize>,
}

/// Which hunk, in a particular [`Patch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HunkId {
    pub file: usize,
    pub hunk: usize,
}

/// One side's `a,b` from a `@@ -a,b +c,d @@` line, as a 0-based range. A
/// missing count means one line, and a count of zero (a file being created
/// or deleted) means an empty range at that point.
fn side_range(header: &str, sign: char) -> Range<usize> {
    let field = header
        .split_whitespace()
        .nth(if sign == '-' { 1 } else { 2 })
        .and_then(|f| f.strip_prefix(sign))
        .unwrap_or("1");
    let (start, count) = field.split_once(',').unwrap_or((field, "1"));
    let start: usize = start.parse().unwrap_or(1);
    let count: usize = count.parse().unwrap_or(1);
    // Line numbers are 1-based, except that a count of zero points at the
    // gap *before* the given line rather than at a line.
    let start = if count == 0 { start } else { start - 1 };
    start..start + count
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
                old: side_range(line, '-'),
                new: side_range(line, '+'),
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

/// A diff's text together with its parse, which is what splitting a change
/// needs: the hunks say what to choose between, and the text is what gets
/// written back out as a patch of only the chosen ones.
#[derive(Debug, Clone)]
pub struct Patch {
    lines: Vec<String>,
    files: Vec<FileDiff>,
}

impl Patch {
    #[must_use]
    pub fn new(lines: Vec<String>) -> Self {
        let files = parse(&lines);
        Self { lines, files }
    }

    /// Parse a patch from its text.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self::new(text.lines().map(str::to_owned).collect())
    }

    #[must_use]
    pub fn files(&self) -> &[FileDiff] {
        &self.files
    }

    #[must_use]
    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    /// Every hunk, in the order they appear.
    #[must_use]
    pub fn ids(&self) -> Vec<HunkId> {
        self.files
            .iter()
            .enumerate()
            .flat_map(|(file, f)| (0..f.hunks.len()).map(move |hunk| HunkId { file, hunk }))
            .collect()
    }

    /// A git-format patch of just `chosen`, to apply to the tree as it was
    /// before the change.
    ///
    /// Files with no chosen hunk are left out entirely rather than written
    /// as empty entries, so applying this patch touches nothing else.
    #[must_use]
    pub fn subset(&self, chosen: &[HunkId]) -> String {
        let mut out = String::new();
        for (i, file) in self.files.iter().enumerate() {
            let picked: Vec<&Hunk> = file
                .hunks
                .iter()
                .enumerate()
                .filter(|(h, _)| chosen.contains(&HunkId { file: i, hunk: *h }))
                .map(|(_, hunk)| hunk)
                .collect();
            if picked.is_empty() {
                continue;
            }
            for l in file.header.clone() {
                // `index` names jj's blob hashes, which mean nothing to
                // anything reading this patch back.
                if !self.line(l).starts_with("index ") {
                    out.push_str(self.line(l));
                    out.push('\n');
                }
            }
            for hunk in picked {
                out.push_str(self.line(hunk.header));
                out.push('\n');
                for l in hunk.lines.clone() {
                    out.push_str(self.line(l));
                    out.push('\n');
                }
            }
        }
        out
    }

    /// The file as it is after applying every hunk the patch holds for it to
    /// `before`, which must be the file as it was before the change.
    ///
    /// The hunks came from a diff of this exact file, so their line numbers
    /// are exact and nothing has to be searched for.
    #[must_use]
    pub fn apply(&self, file: &FileDiff, before: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut pos = 0;
        for hunk in &file.hunks {
            let start = hunk.old.start.min(before.len());
            out.extend_from_slice(&before[pos.min(start)..start]);
            for l in hunk.lines.clone() {
                let line = self.line(l);
                match line.as_bytes().first() {
                    // A context or added line, without its marker column.
                    Some(b' ' | b'+') => out.push(line[1..].to_owned()),
                    // A removed line, or "\ No newline at end of file".
                    Some(b'-' | b'\\') => {}
                    // A context line that was empty, marker and all.
                    _ => out.push(String::new()),
                }
            }
            pos = hunk.old.end.min(before.len());
        }
        out.extend_from_slice(&before[pos.min(before.len())..]);
        out
    }
}
