//! The commit message model: one text buffer per form field, rendered to
//! (and parsed from) a Conventional Commit message:
//!
//! ```text
//! type(scope)!: subject
//!
//! Free-form body text.
//!
//! Problem:
//! ...
//!
//! Solution:
//! ...
//!
//! Token: value
//! Token #value
//! ```
//!
//! Every part after the header is optional. Parsing is deliberately
//! conservative: a message is only split into fields if rendering those
//! fields reproduces it byte for byte, so opening and saving a message
//! never rewrites it behind the user's back.

use std::fmt;

/// Header length (`type(scope): subject`) most style guides aim for.
pub const HEADER_SOFT_LIMIT: usize = 50;
/// Header length past which forges and `git log --oneline` start truncating.
pub const HEADER_HARD_LIMIT: usize = 72;

/// One input box in the form, in on-screen (and message) order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Type,
    Scope,
    Subject,
    Body,
    Problem,
    Solution,
    Trailers,
}

impl Field {
    pub const ALL: [Self; 7] = [
        Self::Type,
        Self::Scope,
        Self::Subject,
        Self::Body,
        Self::Problem,
        Self::Solution,
        Self::Trailers,
    ];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::Scope => "Scope",
            Self::Subject => "Subject",
            Self::Body => "Body",
            Self::Problem => "Problem",
            Self::Solution => "Solution",
            Self::Trailers => "Trailers",
        }
    }

    /// Hint shown in an empty field.
    #[must_use]
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::Type => "feat, fix, docs, refactor, …",
            Self::Scope => "optional",
            Self::Subject => "imperative summary, no period",
            Self::Body => "Free-form context (optional)",
            Self::Problem => "What is wrong or missing? (optional)",
            Self::Solution => "How does this change address it? (optional)",
            Self::Trailers => "Fixes #42, Reviewed-by: Name <email> (optional)",
        }
    }

    #[must_use]
    pub const fn is_multiline(self) -> bool {
        !matches!(self, Self::Type | Self::Scope | Self::Subject)
    }

    /// The section heading this field is written under, if any.
    const fn heading(self) -> Option<&'static str> {
        match self {
            Self::Problem => Some("Problem:"),
            Self::Solution => Some("Solution:"),
            _ => None,
        }
    }

    /// The field below this one (stays put on the last field).
    #[must_use]
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1).min(Self::ALL.len() - 1)]
    }

    /// The field above this one (stays put on the first field).
    #[must_use]
    pub const fn prev(self) -> Self {
        Self::ALL[(self as usize).saturating_sub(1)]
    }
}

/// Why a message couldn't be split into fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The first line isn't `type(scope)!: subject`.
    Header,
    /// The header and body aren't separated by a blank line.
    NoBlankLine,
    /// The fields can't reproduce the message exactly (unusual spacing,
    /// sections out of order, ...).
    NotLossless,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Header => "the first line isn't `type(scope): subject`",
            Self::NoBlankLine => "there's no blank line after the first line",
            Self::NotLossless => "the form can't reproduce its exact layout",
        })
    }
}

/// The text of each field.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Draft {
    inputs: [String; Field::ALL.len()],
}

impl Draft {
    #[must_use]
    pub fn get(&self, field: Field) -> &str {
        &self.inputs[field as usize]
    }

    pub fn set(&mut self, field: Field, value: impl Into<String>) {
        self.inputs[field as usize] = value.into();
    }

    /// True if every field is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inputs.iter().all(|s| s.trim().is_empty())
    }

    /// `type(scope)!: subject`. A trailing `!` on the type marks a breaking
    /// change and is moved after the scope, per the Conventional Commits spec.
    #[must_use]
    pub fn header(&self) -> String {
        let ty = self.get(Field::Type).trim();
        let (ty, bang) = ty
            .strip_suffix('!')
            .map_or((ty, ""), |ty| (ty.trim_end(), "!"));
        let scope = self.get(Field::Scope).trim();
        let subject = self.get(Field::Subject).trim();

        if scope.is_empty() {
            format!("{ty}{bang}: {subject}")
        } else {
            format!("{ty}({scope}){bang}: {subject}")
        }
    }

    /// The rendered message, or the first reason it isn't a valid
    /// Conventional Commit.
    ///
    /// # Errors
    ///
    /// Returns a static string describing the validation failure if:
    /// * The commit type is empty or contains multiple words/invalid characters.
    /// * The scope contains whitespace or parentheses.
    /// * The subject line is empty.
    /// * The trailers section contains text that does not conform to standard trailer formatting (e.g., `Token: value` or `Token #value`).
    pub fn build(&self) -> Result<String, &'static str> {
        let ty = self.get(Field::Type).trim();
        // Only one `!` is meaningful; `header()` strips exactly one.
        let ty = ty.strip_suffix('!').map_or(ty, str::trim_end);
        if ty.is_empty() {
            return Err("type is required (feat, fix, refactor, ...)");
        }
        if !is_token(ty) {
            return Err("type must be a single word, e.g. feat or fix");
        }

        let scope = self.get(Field::Scope).trim();
        if scope.contains(|c: char| c.is_whitespace() || c == '(' || c == ')') {
            return Err("scope must not contain spaces or parentheses");
        }

        if self.get(Field::Subject).trim().is_empty() {
            return Err("subject is required");
        }

        let trailers = tidy(self.get(Field::Trailers));
        if !trailers.is_empty() && !is_trailer_block(trailers) {
            return Err("each trailer must look like `Token: value` or `Token #value`");
        }

        Ok(self.render())
    }

    /// The message as it would be written, without validating it. An
    /// all-empty draft renders as an empty string.
    #[must_use]
    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        if !self.is_empty() {
            parts.push(self.header());
        }
        for field in [
            Field::Body,
            Field::Problem,
            Field::Solution,
            Field::Trailers,
        ] {
            let text = tidy(self.get(field));
            if text.is_empty() {
                continue;
            }
            parts.push(
                field
                    .heading()
                    .map_or_else(|| text.to_owned(), |heading| format!("{heading}\n{text}")),
            );
        }
        parts.join("\n\n")
    }

    /// Split an existing message into fields, if that can be done without
    /// changing it: `parse(m)?.render()` is always `m` minus trailing
    /// whitespace.
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] if the message cannot be safely parsed:
    /// * [`ParseError::Header`]: The first line does not conform to the `type(scope): subject` format.
    /// * [`ParseError::NoBlankLine`]: A body is present, but it is not separated from the header by a blank line.
    /// * [`ParseError::NotLossless`]: The parsed representation, when re-rendered, does not match the original input string (meaning data or formatting would be lost).
    pub fn parse(message: &str) -> Result<Self, ParseError> {
        let message = message.trim_end();
        let mut draft = Self::default();
        if message.is_empty() {
            return Ok(draft);
        }

        let (header, rest) = message.split_once('\n').unwrap_or((message, ""));
        let (ty, scope, subject) = parse_header(header).ok_or(ParseError::Header)?;
        draft.set(Field::Type, ty);
        draft.set(Field::Scope, scope);
        draft.set(Field::Subject, subject);

        let body = match rest.strip_prefix('\n') {
            Some(body) => body,
            None if rest.is_empty() => "",
            None => return Err(ParseError::NoBlankLine),
        };

        // Trailers: the last paragraph, if it's made only of trailer lines.
        let (body, trailers) = match body.rsplit_once("\n\n") {
            Some((before, last)) if is_trailer_block(last) => (before, last),
            None if is_trailer_block(body) => ("", body),
            _ => (body, ""),
        };
        draft.set(Field::Trailers, trailers);

        // Sections: a heading line at the start of a paragraph.
        let lines: Vec<&str> = body.lines().collect();
        let is_heading =
            |i: usize, heading: &str| lines[i] == heading && (i == 0 || lines[i - 1].is_empty());
        let problem = (0..lines.len()).find(|&i| is_heading(i, "Problem:"));
        let solution =
            (problem.map_or(0, |p| p + 1)..lines.len()).find(|&i| is_heading(i, "Solution:"));

        // Text between `from` and the next heading (exclusive), without the
        // blank line that separates them.
        let span = |from: usize, to: Option<usize>| {
            let to = to.map_or(lines.len(), |to| to.saturating_sub(1));
            lines.get(from..to).unwrap_or_default().join("\n")
        };
        draft.set(Field::Body, span(0, problem.or(solution)));
        if let Some(p) = problem {
            draft.set(Field::Problem, span(p + 1, solution));
        }
        if let Some(s) = solution {
            draft.set(Field::Solution, span(s + 1, None));
        }

        if draft.render() == message {
            Ok(draft)
        } else {
            Err(ParseError::NotLossless)
        }
    }
}

/// A field's text as written into the message: no leading blank lines, no
/// trailing whitespace. Leading indentation on the first line is kept.
fn tidy(text: &str) -> &str {
    let text = text.trim_end();
    let start = text
        .split_inclusive('\n')
        .take_while(|line| line.trim().is_empty())
        .map(str::len)
        .sum();
    &text[start..]
}

/// A Conventional Commit type or trailer token: letters, digits, `-`.
fn is_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Split `type(scope)!: subject` into the Type field (which keeps the `!`),
/// the scope and the subject.
fn parse_header(header: &str) -> Option<(String, &str, &str)> {
    let (prefix, subject) = header.split_once(": ")?;
    let (prefix, bang) = prefix.strip_suffix('!').map_or((prefix, ""), |p| (p, "!"));
    let (ty, scope) = match prefix.strip_suffix(')') {
        Some(p) => p.split_once('(')?,
        None => (prefix, ""),
    };
    let scope_ok = !scope.contains(|c: char| c.is_whitespace() || c == '(' || c == ')');
    (is_token(ty) && scope_ok && !subject.trim().is_empty())
        .then(|| (format!("{ty}{bang}"), scope, subject))
}

/// A paragraph of git trailers / Conventional Commit footers: every line is
/// `Token: value` or `Token #value`, or an indented continuation line.
fn is_trailer_block(paragraph: &str) -> bool {
    let mut lines = paragraph.lines();
    lines.next().is_some_and(is_trailer)
        && lines.all(|line| is_trailer(line) || line.starts_with([' ', '\t']))
}

fn is_trailer(line: &str) -> bool {
    let rest = line
        .strip_prefix("BREAKING CHANGE")
        .or_else(|| {
            let end = line.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))?;
            is_token(&line[..end]).then(|| &line[end..])
        })
        .unwrap_or_default();
    rest.strip_prefix(": ")
        .or_else(|| rest.strip_prefix(" #"))
        .is_some_and(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(fields: &[(Field, &str)]) -> Draft {
        let mut d = Draft::default();
        for &(field, text) in fields {
            d.set(field, text);
        }
        d
    }

    fn header(ty: &str, scope: &str, subject: &str) -> Draft {
        draft(&[
            (Field::Type, ty),
            (Field::Scope, scope),
            (Field::Subject, subject),
        ])
    }

    /// Parses, and checks that rendering gives the message back.
    fn round_trip(message: &str) -> Draft {
        let d = Draft::parse(message).unwrap_or_else(|e| panic!("{e}: {message:?}"));
        assert_eq!(d.render(), message.trim_end());
        d
    }

    #[test]
    fn header_only() {
        let d = header("fix", "", "handle empty repo");
        assert_eq!(d.build().unwrap(), "fix: handle empty repo");
    }

    #[test]
    fn full_message() {
        let mut d = header(" feat ", "ui", "add diff pane");
        d.set(Field::Problem, "No context.\n");
        d.set(Field::Solution, "Show `jj diff`.");
        assert_eq!(
            d.build().unwrap(),
            "feat(ui): add diff pane\n\nProblem:\nNo context.\n\nSolution:\nShow `jj diff`."
        );
    }

    #[test]
    fn breaking_change_bang_goes_after_scope() {
        let d = header("feat!", "api", "drop v1 endpoints");
        assert_eq!(d.build().unwrap(), "feat(api)!: drop v1 endpoints");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(header("", "", "x").build().is_err());
        assert!(header("feat", "", " ").build().is_err());
        assert!(header("new feature", "", "x").build().is_err());
        assert!(header("feat", "a b", "x").build().is_err());
        assert!(header("feat!!", "", "x").build().is_err());
        assert!(header("!", "", "x").build().is_err());

        let mut d = header("fix", "", "x");
        d.set(Field::Trailers, "fixes the bug");
        assert!(d.build().is_err());
    }

    #[test]
    fn empty_draft_renders_empty() {
        assert_eq!(Draft::default().render(), "");
        assert_eq!(Draft::parse("\n").unwrap(), Draft::default());
    }

    #[test]
    fn keeps_first_line_indentation() {
        let mut d = header("docs", "", "add example");
        d.set(Field::Body, "\n\n    let x = 1;\nmore\n\n");
        assert_eq!(d.render(), "docs: add example\n\n    let x = 1;\nmore");
    }

    #[test]
    fn parses_unknown_body_and_trailers() {
        let d = round_trip(
            "fix(parser): handle empty input\n\n\
             This also corrects an unrelated diagnostic.\n\n\
             Fixes #42\n\
             Reviewed-by: Someone\n",
        );
        assert_eq!(d.get(Field::Type), "fix");
        assert_eq!(d.get(Field::Scope), "parser");
        assert_eq!(d.get(Field::Subject), "handle empty input");
        assert_eq!(
            d.get(Field::Body),
            "This also corrects an unrelated diagnostic."
        );
        assert_eq!(d.get(Field::Trailers), "Fixes #42\nReviewed-by: Someone");
    }

    #[test]
    fn parses_sections() {
        let d = round_trip(
            "feat(ui)!: add diff pane\n\n\
             Context first.\n\n\
             Problem:\nNo context.\n\nSecond paragraph.\n\n\
             Solution:\nShow `jj diff`.\n\n\
             BREAKING CHANGE: the old pane is gone",
        );
        assert_eq!(d.get(Field::Type), "feat!");
        assert_eq!(d.get(Field::Body), "Context first.");
        assert_eq!(d.get(Field::Problem), "No context.\n\nSecond paragraph.");
        assert_eq!(d.get(Field::Solution), "Show `jj diff`.");
        assert_eq!(
            d.get(Field::Trailers),
            "BREAKING CHANGE: the old pane is gone"
        );
    }

    #[test]
    fn parses_trailers_without_body() {
        let d = round_trip("fix: x\n\nSigned-off-by: A <a@example.com>\n  continued");
        assert_eq!(d.get(Field::Body), "");
        assert!(d.get(Field::Trailers).starts_with("Signed-off-by"));
    }

    #[test]
    fn solution_without_problem() {
        let d = round_trip("fix: x\n\nSolution:\nDo it.");
        assert_eq!(d.get(Field::Solution), "Do it.");
    }

    #[test]
    fn heading_mid_paragraph_is_body_text() {
        let d = round_trip("fix: x\n\nSee below\nProblem:\nnot a heading");
        assert_eq!(d.get(Field::Problem), "");
    }

    #[test]
    fn rejects_what_it_cannot_reproduce() {
        let cases = [
            ("WIP", ParseError::Header),
            ("fix(a b): x", ParseError::Header),
            ("fix:  x", ParseError::NotLossless),
            ("fix: x\nbody", ParseError::NoBlankLine),
            ("fix: x\n\n\nbody", ParseError::NotLossless),
            ("fix: x\n\n  \nbody", ParseError::NotLossless),
            (
                "fix: x\n\nProblem:\n\nSolution:\ny",
                ParseError::NotLossless,
            ),
        ];
        for (message, error) in cases {
            assert_eq!(Draft::parse(message), Err(error), "{message:?}");
        }
    }

    #[test]
    fn trailer_lines() {
        assert!(is_trailer("Fixes #42"));
        assert!(is_trailer("Reviewed-by: Someone"));
        assert!(is_trailer("BREAKING CHANGE: gone"));
        assert!(!is_trailer("Fixes 42"));
        assert!(!is_trailer("Two words: no"));
        assert!(!is_trailer("Token:"));
    }
}
