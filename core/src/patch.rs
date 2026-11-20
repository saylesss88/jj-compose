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

/// How much of a hunk a split takes.
///
/// A hunk is the usual unit, but one that bundles unrelated edits on
/// neighbouring lines can be divided: see [`Patch::subset_with`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Take {
    /// Every change in the hunk.
    Whole,
    /// Only these lines, as indices into the patch's own lines, as
    /// [`Patch::changed_lines`] gives them.
    Lines(Vec<usize>),
}

/// A `@@ -a,b +c,d @@` header with the new side's count replaced, the rest
/// of the line (jj writes no funcname, but a patch from elsewhere might)
/// left as it was.
fn recount(header: &str, new_count: usize) -> String {
    let mut parts = header.splitn(5, ' ');
    let (Some(at), Some(old), Some(new), Some(close)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return header.to_owned();
    };
    let start = new
        .strip_prefix('+')
        .and_then(|f| f.split_once(',').map_or(Some(f), |(s, _)| Some(s)))
        .unwrap_or("1");
    let rest = parts.next().map_or(String::new(), |r| format!(" {r}"));
    format!("{at} {old} +{start},{new_count} {close}{rest}")
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
    let start = if count == 0 {
        start
    } else {
        start.saturating_sub(1)
    };

    start..start.saturating_add(count)
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

    /// A hunk's lines without its `@@` header.
    ///
    /// This is the part of a hunk a split leaves alone, so it is what a plan
    /// spanning several commits is keyed by; see [`crate::plan::Plan`].
    #[must_use]
    pub fn hunk_content(&self, hunk: &Hunk) -> Vec<String> {
        hunk.lines
            .clone()
            .map(|i| self.line(i).to_owned())
            .collect()
    }

    /// A hunk's lines in the order a line-level split needs them: every run
    /// of `-` lines followed by a run of `+` lines is interleaved into
    /// pairs, so a removal sits next to the addition that replaces it.
    ///
    /// git groups the two sides (`-a -b +A +B`), which reads well but puts
    /// every removal before every addition. That matters as soon as only
    /// some of the lines are taken: a `-` left behind becomes context, and
    /// in the grouped order it would land *above* a `+` that was taken, so
    /// the committed file would hold the lines in the wrong order.
    ///
    /// Interleaving cannot change what a hunk means when the whole of it is
    /// taken, because it reorders the two sides only with respect to each
    /// other and never within a side. Runs of unequal length pair as far as
    /// they go, and the remainder follows.
    #[must_use]
    pub fn ordered_lines(&self, hunk: &Hunk) -> Vec<usize> {
        // A changed line and the `\ No newline` marker describing it, if any,
        // which has to travel with it.
        let mut out: Vec<usize> = Vec::new();
        let mut rest = hunk.lines.clone().peekable();
        while let Some(l) = rest.next() {
            let sign = self.line(l).as_bytes().first().copied();
            if sign != Some(b'-') {
                out.push(l);
                continue;
            }
            // Collect this run of removals, then the additions after it.
            let mut minus = vec![self.with_marker(l, &mut rest)];
            while rest.peek().is_some_and(|&n| self.line(n).starts_with('-')) {
                let n = rest.next().unwrap_or(l);
                minus.push(self.with_marker(n, &mut rest));
            }
            let mut plus: Vec<Vec<usize>> = Vec::new();
            while rest.peek().is_some_and(|&n| self.line(n).starts_with('+')) {
                let n = rest.next().unwrap_or(l);
                plus.push(self.with_marker(n, &mut rest));
            }
            for i in 0..minus.len().max(plus.len()) {
                out.extend(minus.get(i).into_iter().flatten());
                out.extend(plus.get(i).into_iter().flatten());
            }
        }
        out
    }

    /// `l`, plus the `\ No newline at end of file` marker that follows it if
    /// there is one, consumed from `rest` so it stays attached.
    fn with_marker(
        &self,
        l: usize,
        rest: &mut std::iter::Peekable<std::ops::Range<usize>>,
    ) -> Vec<usize> {
        if rest.peek().is_some_and(|&n| self.line(n).starts_with('\\'))
            && let Some(n) = rest.next()
        {
            return vec![l, n];
        }
        vec![l]
    }

    /// The `+` and `-` lines of a hunk, in the order
    /// [`Patch::ordered_lines`] gives them: the lines a split can decide
    /// about one at a time. Context and `\ No newline` markers are not
    /// choices, so they are left out.
    #[must_use]
    pub fn changed_lines(&self, hunk: &Hunk) -> Vec<usize> {
        self.ordered_lines(hunk)
            .into_iter()
            .filter(|&l| matches!(self.line(l).as_bytes().first(), Some(b'+' | b'-')))
            .collect()
    }

    /// A git-format patch of just `chosen`, to apply to the tree as it was
    /// before the change.
    ///
    /// Files with no chosen hunk are left out entirely rather than written
    /// as empty entries, so applying this patch touches nothing else.
    #[must_use]
    pub fn subset(&self, chosen: &[HunkId]) -> String {
        let whole: Vec<(HunkId, Take)> = chosen.iter().map(|&id| (id, Take::Whole)).collect();
        self.subset_with(&whole)
    }

    /// As [`Patch::subset`], but each hunk may be taken whole or by line.
    ///
    /// A hunk taken by line is rewritten rather than copied, because the
    /// lines left behind still have to describe the same before-state:
    ///
    /// - a `+` line not taken is dropped, so the commit doesn't add it;
    /// - a `-` line not taken becomes context, so the commit doesn't remove
    ///   it and the line survives into the remainder;
    /// - a `\ No newline at end of file` marker is kept only if the line it
    ///   describes survived, so the hunk stays self-consistent.
    ///
    /// The `-a,b` half of the header is left alone: neither rewrite changes
    /// the before-state, so the old side can't move. Only `+c,d` is
    /// recounted.
    #[must_use]
    pub fn subset_with(&self, chosen: &[(HunkId, Take)]) -> String {
        let mut out = String::new();
        for (i, file) in self.files.iter().enumerate() {
            let picked: Vec<(&Hunk, &Take)> = file
                .hunks
                .iter()
                .enumerate()
                .filter_map(|(h, hunk)| {
                    let id = HunkId { file: i, hunk: h };
                    chosen
                        .iter()
                        .find(|(other, _)| *other == id)
                        .map(|(_, take)| (hunk, take))
                })
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
            for (hunk, take) in picked {
                let body = self.rewrite(hunk, take);
                let new_count = body
                    .iter()
                    .filter(|l| matches!(l.as_bytes().first(), Some(b' ' | b'+') | None))
                    .count();
                out.push_str(&recount(self.line(hunk.header), new_count));
                out.push('\n');
                for line in body {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// A hunk's lines as the patch should carry them, given what is taken.
    fn rewrite(&self, hunk: &Hunk, take: &Take) -> Vec<String> {
        let Take::Lines(taken) = take else {
            return self.hunk_content(hunk);
        };
        let mut out: Vec<String> = Vec::new();
        // Whether the line a `\ No newline` marker describes is still there.
        let mut described = false;
        for l in self.ordered_lines(hunk) {
            let line = self.line(l);
            match line.as_bytes().first() {
                Some(b'\\') => {
                    if described {
                        out.push(line.to_owned());
                    }
                    continue;
                }
                Some(sign @ (b'+' | b'-')) if !taken.contains(&l) => {
                    // Not taken: an addition never happens, a removal turns
                    // into the context line it was removed from.
                    described = *sign == b'-';
                    if described {
                        out.push(format!(" {}", &line[1..]));
                    }
                    continue;
                }
                _ => {}
            }
            described = true;
            out.push(line.to_owned());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A rename and a logic change on neighbouring lines: one hunk, two
    /// unrelated edits. This is the case line-level splitting is for.
    const MIXED: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
index 1111111..2222222 100644
@@ -10,4 +10,4 @@
 fn run() {
-    let cnt = 0;
+    let count = 0;
-    if cnt > 1 {
+    if count >= 1 {
 }
";

    fn only(patch: &Patch, wanted: &[&str]) -> Take {
        let hunk = &patch.files()[0].hunks[0];
        Take::Lines(
            patch
                .changed_lines(hunk)
                .into_iter()
                .filter(|&l| wanted.contains(&patch.line(l)))
                .collect(),
        )
    }

    fn body(text: &str) -> Vec<String> {
        text.lines()
            .skip_while(|l| !l.starts_with("@@"))
            .skip(1)
            .map(str::to_owned)
            .collect()
    }

    /// git groups the sides, so the raw hunk is `- - + +`. Pairing them is
    /// what makes a line-level selection come out in the right order.
    #[test]
    fn runs_of_removals_and_additions_are_paired_up() {
        let patch = Patch::from_text(MIXED);
        let hunk = &patch.files()[0].hunks[0];
        let signs: Vec<&str> = patch
            .ordered_lines(hunk)
            .iter()
            .map(|&l| &patch.line(l)[..1])
            .collect();
        assert_eq!(signs, [" ", "-", "+", "-", "+", " "]);
    }

    /// Pairing reorders the two sides only against each other, so a hunk
    /// taken whole still produces the same file.
    #[test]
    fn pairing_cannot_change_a_whole_hunk() {
        let patch = Patch::from_text(MIXED);
        let hunk = &patch.files()[0].hunks[0];
        let before: Vec<String> = ["fn run() {", "    let cnt = 0;", "    if cnt > 1 {", "}"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let shifted = Patch::from_text(&MIXED.replace("@@ -10,4 +10,4 @@", "@@ -1,4 +1,4 @@"));
        let every: Vec<usize> = patch.changed_lines(hunk);
        let paired = shifted.subset_with(&[(HunkId { file: 0, hunk: 0 }, Take::Lines(every))]);
        let paired = Patch::from_text(&paired);
        assert_eq!(
            paired.apply(&paired.files()[0], &before),
            shifted.apply(&shifted.files()[0], &before)
        );
    }

    /// Three removals replaced by one addition: pair what pairs, and let
    /// the rest follow rather than dropping it.
    #[test]
    fn uneven_runs_pair_as_far_as_they_go() {
        const UNEVEN: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,4 +1,2 @@
 keep
-one
-two
-three
+only
";
        let patch = Patch::from_text(UNEVEN);
        let hunk = &patch.files()[0].hunks[0];
        let lines: Vec<&str> = patch
            .ordered_lines(hunk)
            .iter()
            .map(|&l| patch.line(l))
            .collect();
        assert_eq!(
            lines,
            [" keep", "-one", "+only", "-two", "-three"],
            "nothing is lost, and the counts still add up"
        );
        assert_eq!(patch.changed_lines(hunk).len(), 4);
    }

    #[test]
    fn changed_lines_are_the_choices_and_nothing_else() {
        let patch = Patch::from_text(MIXED);
        let hunk = &patch.files()[0].hunks[0];
        let signs: Vec<&str> = patch
            .changed_lines(hunk)
            .iter()
            .map(|&l| &patch.line(l)[..1])
            .collect();
        assert_eq!(signs, ["-", "+", "-", "+"], "context is not a choice");
    }

    #[test]
    fn taking_a_hunk_whole_is_what_subset_already_did() {
        let patch = Patch::from_text(MIXED);
        let id = HunkId { file: 0, hunk: 0 };
        assert_eq!(patch.subset(&[id]), patch.subset_with(&[(id, Take::Whole)]));
    }

    /// The `-` that wasn't taken becomes context, so this commit leaves the
    /// line alone and the remainder still has it to remove.
    #[test]
    fn a_removal_left_behind_turns_into_context() {
        let patch = Patch::from_text(MIXED);
        let take = only(&patch, &["-    let cnt = 0;", "+    let count = 0;"]);
        let out = patch.subset_with(&[(HunkId { file: 0, hunk: 0 }, take)]);
        assert_eq!(
            body(&out),
            [
                " fn run() {",
                "-    let cnt = 0;",
                "+    let count = 0;",
                "     if cnt > 1 {",
                " }",
            ]
        );
    }

    /// An addition left behind is simply absent: nothing adds it here.
    #[test]
    fn an_addition_left_behind_is_dropped() {
        let patch = Patch::from_text(MIXED);
        let take = only(&patch, &["-    if cnt > 1 {"]);
        let out = patch.subset_with(&[(HunkId { file: 0, hunk: 0 }, take)]);
        assert_eq!(
            body(&out),
            [
                " fn run() {",
                "     let cnt = 0;",
                "-    if cnt > 1 {",
                " }",
            ],
            "the + is gone and the untaken - is context"
        );
    }

    /// The before-state can't move, so `-a,b` is untouched however the lines
    /// are divided; only the new side is recounted.
    #[test]
    fn the_old_side_of_the_header_never_moves() {
        let patch = Patch::from_text(MIXED);
        let take = only(&patch, &["-    let cnt = 0;"]);
        let out = patch.subset_with(&[(HunkId { file: 0, hunk: 0 }, take)]);
        let header = out.lines().find(|l| l.starts_with("@@")).unwrap();
        assert_eq!(
            header, "@@ -10,4 +10,3 @@",
            "the old side is as it was; one line fewer on the new"
        );
    }

    #[test]
    fn a_funcname_on_the_header_survives_recounting() {
        assert_eq!(
            recount("@@ -1,3 +1,3 @@ fn run() {", 2),
            "@@ -1,3 +1,2 @@ fn run() {"
        );
    }

    /// The point of all of it: the file jj is handed must be right.
    #[test]
    fn the_file_written_holds_only_the_lines_taken() {
        let patch = Patch::from_text(MIXED);
        let before: Vec<String> = [
            "a",
            "b",
            "c",
            "d",
            "e",
            "f",
            "g",
            "h",
            "i",
            "fn run() {",
            "    let cnt = 0;",
            "    if cnt > 1 {",
            "}",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

        let take = only(&patch, &["-    let cnt = 0;", "+    let count = 0;"]);
        let out = patch.subset_with(&[(HunkId { file: 0, hunk: 0 }, take)]);
        let half = Patch::from_text(&out);
        let after = half.apply(&half.files()[0], &before);

        assert_eq!(
            &after[9..],
            ["fn run() {", "    let count = 0;", "    if cnt > 1 {", "}"],
            "the rename landed, the logic change did not"
        );
        assert_eq!(&after[..9], &before[..9], "nothing above the hunk moved");
    }

    /// Both halves together must rebuild the original file, or a split
    /// would lose work.
    #[test]
    fn the_two_halves_of_a_hunk_compose_back_to_the_whole() {
        let before: Vec<String> = ["fn run() {", "    let cnt = 0;", "    if cnt > 1 {", "}"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        // The hunk starts at line 10, so line it up with a short file.
        let shifted = MIXED.replace("@@ -10,4 +10,4 @@", "@@ -1,4 +1,4 @@");
        let patch = Patch::from_text(&shifted);
        let first = only(&patch, &["-    let cnt = 0;", "+    let count = 0;"]);
        let out = patch.subset_with(&[(HunkId { file: 0, hunk: 0 }, first)]);
        let half = Patch::from_text(&out);
        let middle = half.apply(&half.files()[0], &before);

        let whole = patch.apply(&patch.files()[0], &before);
        assert_ne!(middle, whole, "the first half is not the whole change");

        // The remainder, diffed against the halfway file, is the rest.
        let rest = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,4 @@
 fn run() {
     let count = 0;
-    if cnt > 1 {
+    if count >= 1 {
 }
";
        let rest = Patch::from_text(rest);
        assert_eq!(
            rest.apply(&rest.files()[0], &middle),
            whole,
            "half then the rest is the original change"
        );
    }

    #[test]
    fn a_no_newline_marker_follows_the_line_it_describes() {
        const EOF: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
 keep
-old
+new
\\ No newline at end of file
";
        let patch = Patch::from_text(EOF);
        let id = HunkId { file: 0, hunk: 0 };

        let kept = only(&patch, &["+new", "-old"]);
        assert!(
            patch.subset_with(&[(id, kept)]).contains("\\ No newline"),
            "the + it describes is still there"
        );

        let dropped = only(&patch, &["-old"]);
        let out = patch.subset_with(&[(id, dropped)]);
        assert!(
            !out.contains("\\ No newline"),
            "the + it described is gone, so the marker would be a lie: {out}"
        );
    }

    #[test]
    fn taking_no_lines_of_a_hunk_leaves_the_file_untouched() {
        let patch = Patch::from_text(MIXED);
        let out = patch.subset_with(&[(HunkId { file: 0, hunk: 0 }, Take::Lines(vec![]))]);
        assert_eq!(
            body(&out),
            [
                " fn run() {",
                "     let cnt = 0;",
                "     if cnt > 1 {",
                " }",
            ],
            "every line is context"
        );
    }

    #[test]
    fn test_fuzz_crash_c0cd0140() {
        // The exact byte sequence that panicked the fuzzer
        let data = [
            10, 11, 10, 10, 100, 105, 102, 102, 32, 45, 45, 103, 105, 116, 32, 10, 10, 64, 64, 61,
            61, 100, 32, 60, 0, 103, 105, 116, 32, 10, 10, 64, 64, 61, 61, 100, 105, 102, 51, 32,
            45, 48, 10, 10, 10, 10, 100, 105, 102, 102, 32, 45, 45, 103, 105, 116, 32, 10, 10, 64,
            64, 0, 0, 0, 0, 0, 0, 0, 10, 64, 64, 0, 103, 105,
        ];
        let diff_text = std::str::from_utf8(&data).unwrap();

        // This will trigger the standard Rust panic with a clear line number
        let _ = Patch::from_text(diff_text);
    }
}
