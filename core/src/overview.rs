//! A list of every hunk in a change, so its shape and plan can be seen at
//! once.
//!
//! Split mode shows one hunk at a time, which is right for reading one but
//! useless for deciding which belong together. This assembles the whole
//! change as rows: a heading per file with its totals and how many of its
//! hunks are spoken for, then a row per hunk with the commit it is destined
//! for, its size, and the definition it sits in.
//!
//! The text is built here rather than in the UI so the column alignment can
//! be tested without a terminal; the caller only picks colors.

use crate::{HunkId, Patch};

/// What a row is, so the caller knows how to draw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A file heading.
    File,
    /// A hunk destined for the given commit.
    InBox(u8),
    /// A hunk whose lines go to more than one place.
    Divided,
    /// A hunk that would stay behind.
    Loose,
}

/// What the plan says about one hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    /// Nothing takes it.
    Nowhere,
    /// All of it goes to one commit.
    Whole(u8),
    /// It has been divided: this many of its `+`/`-` lines are taken, of
    /// this many. A division whose lines all agree folds back into
    /// [`Where::Whole`], so a divided row always has something spread about
    /// or left behind.
    Divided { taken: usize, total: usize },
}

impl Where {
    /// Whether anything at all takes this hunk.
    const fn is_taken(self) -> bool {
        !matches!(self, Self::Nowhere | Self::Divided { taken: 0, .. })
    }

    /// The column saying where the hunk is going: its box number, or `*`
    /// for one that is divided.
    fn marker(self) -> String {
        match self {
            Self::Whole(n) if n < 10 => char::from(b'0' + n).to_string(),
            Self::Whole(_) => "+".to_owned(),
            Self::Divided { taken: 0, .. } | Self::Nowhere => " ".to_owned(),
            Self::Divided { .. } => "*".to_owned(),
        }
    }
}

/// One line of the overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: Kind,
    /// The line as it should be drawn, already aligned.
    pub text: String,
    /// The hunk this row selects, or `None` for a file heading.
    pub hunk: Option<HunkId>,
}

/// Build the overview of `patch`, with `plan` parallel to [`Patch::ids`]
/// (see [`crate::Split::plan`]) and `symbols` likewise (see
/// [`crate::symbols::for_patch`]).
///
/// Rows come in diff order, each file's heading followed by its hunks, so a
/// cursor can walk the list and land on something meaningful.
#[must_use]
pub fn build(patch: &Patch, plan: &[Where], symbols: &[Option<String>]) -> Vec<Row> {
    let width = patch
        .files()
        .iter()
        .flat_map(|f| f.hunks.iter().map(|h| h.old.start + 1))
        .map(digits)
        .max()
        .unwrap_or(1);

    let mut rows = Vec::new();
    let mut at = 0;
    for (index, file) in patch.files().iter().enumerate() {
        let span = at..at + file.hunks.len();
        let here = plan.get(span.clone()).unwrap_or(&[]);
        let spoken_for = here.iter().filter(|w| w.is_taken()).count();
        let (added, removed) = file
            .hunks
            .iter()
            .map(|hunk| counts(patch, hunk))
            .fold((0, 0), |(a, r), (x, y)| (a + x, r + y));

        rows.push(Row {
            kind: Kind::File,
            text: format!(
                "{}  +{added} -{removed}  {spoken_for} of {} assigned",
                file.path,
                file.hunks.len()
            ),
            hunk: None,
        });
        for (i, hunk) in file.hunks.iter().enumerate() {
            let (added, removed) = counts(patch, hunk);
            let going = here.get(i).copied().unwrap_or(Where::Nowhere);
            let symbol = symbols.get(at + i).and_then(Option::as_deref).unwrap_or("");
            // A divided hunk says how much of it is spoken for, since the
            // marker alone cannot.
            let note = match going {
                Where::Divided { taken, total } if taken > 0 => {
                    format!("  {taken} of {total} lines")
                }
                _ => String::new(),
            };
            rows.push(Row {
                kind: match going {
                    Where::Whole(n) => Kind::InBox(n),
                    Where::Divided { taken, .. } if taken > 0 => Kind::Divided,
                    _ => Kind::Loose,
                },
                text: format!(
                    "{} {:>width$}  {:>4} {:>4}  {symbol}{note}",
                    going.marker(),
                    hunk.old.start + 1,
                    format!("+{added}"),
                    format!("-{removed}"),
                )
                .trim_end()
                .to_owned(),
                hunk: Some(HunkId {
                    file: index,
                    hunk: i,
                }),
            });
        }
        at = span.end;
    }
    rows
}

/// How many lines a hunk adds and removes.
fn counts(patch: &Patch, hunk: &crate::patch::Hunk) -> (usize, usize) {
    hunk.lines.clone().fold((0, 0), |(added, removed), i| {
        match patch.line(i).as_bytes().first() {
            Some(b'+') => (added + 1, removed),
            Some(b'-') => (added, removed + 1),
            _ => (added, removed),
        }
    })
}

const fn digits(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two files, three hunks, with line numbers of different widths.
    const DIFF: &str = "\
diff --git a/core/src/jj.rs b/core/src/jj.rs
--- a/core/src/jj.rs
+++ b/core/src/jj.rs
@@ -52,3 +52,5 @@
 context
-gone
+new
+also new
@@ -164,2 +166,2 @@
-old
+fresh
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -7,1 +7,3 @@
+a
+b
";

    fn rows(plan: &[Where]) -> Vec<Row> {
        let patch = Patch::from_text(DIFF);
        let symbols = vec![
            Some("fn new".to_owned()),
            Some("fn config_get".to_owned()),
            None,
        ];
        build(&patch, plan, &symbols)
    }

    #[test]
    fn every_file_gets_a_heading_and_every_hunk_a_row() {
        let rows = rows(&[Where::Nowhere; 3]);
        assert_eq!(rows.len(), 5, "2 headings + 3 hunks");
        assert_eq!(rows[0].kind, Kind::File);
        assert_eq!(rows[1].kind, Kind::Loose);
        assert_eq!(rows[3].kind, Kind::File);
    }

    #[test]
    fn a_file_heading_totals_its_hunks_and_counts_the_assigned_ones() {
        let rows = rows(&[Where::Whole(1), Where::Nowhere, Where::Nowhere]);
        assert_eq!(rows[0].text, "core/src/jj.rs  +3 -2  1 of 2 assigned");
        assert_eq!(rows[3].text, "README.md  +2 -0  0 of 1 assigned");
    }

    /// The "n of m" is the point: it says when a file has been half done.
    #[test]
    fn assigning_a_whole_file_shows_in_its_heading() {
        let rows = rows(&[Where::Whole(1), Where::Whole(2), Where::Nowhere]);
        assert!(
            rows[0].text.ends_with("2 of 2 assigned"),
            "{}",
            rows[0].text
        );
    }

    #[test]
    fn a_hunk_row_carries_its_commit_line_size_and_symbol() {
        let rows = rows(&[Where::Whole(1), Where::Whole(2), Where::Nowhere]);
        assert_eq!(rows[1].text, "1  52    +2   -1  fn new");
        assert_eq!(rows[2].text, "2 164    +1   -1  fn config_get");
        assert_eq!(rows[4].text, "    7    +2   -0", "unassigned has no digit");
    }

    #[test]
    fn a_row_says_which_commit_it_is_for() {
        let rows = rows(&[Where::Whole(1), Where::Whole(3), Where::Nowhere]);
        assert_eq!(rows[1].kind, Kind::InBox(1));
        assert_eq!(rows[2].kind, Kind::InBox(3));
        assert_eq!(rows[4].kind, Kind::Loose);
    }

    /// Right-aligned in one field sized for the widest number in the whole
    /// change, so the sizes beside them line up too.
    #[test]
    fn line_numbers_line_up_across_files() {
        let rows = rows(&[Where::Nowhere; 3]);
        assert_eq!(&rows[2].text[2..5], "164");
        assert_eq!(&rows[4].text[2..5], "  7");
        let sizes = |text: &str| text.find('+').unwrap();
        assert_eq!(sizes(&rows[2].text), sizes(&rows[4].text));
    }

    #[test]
    fn a_hunk_without_a_symbol_leaves_no_trailing_space() {
        let rows = rows(&[Where::Nowhere; 3]);
        assert_eq!(rows[4].text, rows[4].text.trim_end());
    }

    #[test]
    fn every_hunk_row_points_at_its_hunk() {
        let rows = rows(&[Where::Nowhere; 3]);
        let ids: Vec<_> = rows.iter().filter_map(|r| r.hunk).collect();
        assert_eq!(
            ids,
            [
                HunkId { file: 0, hunk: 0 },
                HunkId { file: 0, hunk: 1 },
                HunkId { file: 1, hunk: 0 },
            ]
        );
        assert!(rows[0].hunk.is_none(), "a heading selects nothing");
    }

    /// A divided hunk is in no one box, so the marker is `*` and the row
    /// says how much of it is spoken for. Showing it as unassigned would
    /// hide work already done.
    #[test]
    fn a_divided_hunk_says_so_rather_than_looking_untouched() {
        let rows = rows(&[
            Where::Divided { taken: 1, total: 3 },
            Where::Whole(2),
            Where::Nowhere,
        ]);
        assert_eq!(rows[1].kind, Kind::Divided);
        assert!(rows[1].text.starts_with('*'), "{}", rows[1].text);
        assert!(rows[1].text.ends_with("1 of 3 lines"), "{}", rows[1].text);
        assert!(
            rows[0].text.ends_with("2 of 2 assigned"),
            "a divided hunk counts toward its file: {}",
            rows[0].text
        );
    }

    /// Divided but nothing taken yet is still untouched, and says nothing.
    #[test]
    fn a_division_with_no_line_taken_reads_as_loose() {
        let rows = rows(&[
            Where::Divided { taken: 0, total: 3 },
            Where::Nowhere,
            Where::Nowhere,
        ]);
        assert_eq!(rows[1].kind, Kind::Loose);
        assert!(
            rows[0].text.ends_with("0 of 2 assigned"),
            "{}",
            rows[0].text
        );
        assert_eq!(rows[1].text, rows[1].text.trim_end(), "no dangling note");
    }

    #[test]
    fn an_empty_change_has_no_rows() {
        assert_eq!(build(&Patch::from_text(""), &[], &[]), vec![]);
    }
}
