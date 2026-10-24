//! The commit message model: one text buffer per form field, rendered to a
//! Conventional Commit message.

/// Header length (`type(scope): subject`) most style guides aim for.
pub const HEADER_SOFT_LIMIT: usize = 50;
/// Header length past which forges and `git log --oneline` start truncating.
pub const HEADER_HARD_LIMIT: usize = 72;

/// One input box in the form, in on-screen order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Type,
    Scope,
    Subject,
    Problem,
    Solution,
}

impl Field {
    pub const ALL: [Self; 5] = [
        Self::Type,
        Self::Scope,
        Self::Subject,
        Self::Problem,
        Self::Solution,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::Scope => "Scope",
            Self::Subject => "Subject",
            Self::Problem => "Problem",
            Self::Solution => "Solution",
        }
    }

    /// Hint shown in an empty field.
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::Type => "feat, fix, docs, refactor, …",
            Self::Scope => "optional",
            Self::Subject => "imperative summary, no period",
            Self::Problem => "What is wrong or missing? (optional)",
            Self::Solution => "How does this change address it? (optional)",
        }
    }

    pub const fn is_multiline(self) -> bool {
        matches!(self, Self::Problem | Self::Solution)
    }

    /// The field below this one (stays put on the last field).
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1).min(Self::ALL.len() - 1)]
    }

    /// The field above this one (stays put on the first field).
    pub const fn prev(self) -> Self {
        Self::ALL[(self as usize).saturating_sub(1)]
    }
}

/// The text the user has typed so far, one buffer per [`Field`].
#[derive(Debug, Default)]
pub struct Draft {
    inputs: [String; Field::ALL.len()],
}

impl Draft {
    pub fn get(&self, field: Field) -> &str {
        &self.inputs[field as usize]
    }

    pub fn set(&mut self, field: Field, value: String) {
        self.inputs[field as usize] = value;
    }

    /// True if every field is empty.
    pub fn is_empty(&self) -> bool {
        self.inputs.iter().all(|s| s.trim().is_empty())
    }

    /// `type(scope)!: subject`. A trailing `!` on the type marks a breaking
    /// change and is moved after the scope, per the Conventional Commits spec.
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

    /// The header plus the optional Problem/Solution body, or the first
    /// reason the header isn't a valid Conventional Commit
    pub fn build(&self) -> Result<String, &'static str> {
        let ty = self.get(Field::Type).trim();
        // Only one `!` is meaningful; `header()` strips exactly one.
        let ty = ty.strip_suffix('!').map_or(ty, str::trim_end);
        if ty.is_empty() {
            return Err("type is required (feat, fix, refactor, ...)");
        }
        if !ty.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("type must be a single word, e.g. feat or fix");
        }

        let scope = self.get(Field::Scope).trim();
        if scope.contains(|c: char| c.is_whitespace() || c == '(' || c == ')') {
            return Err("scope must not contain spaces or parentheses");
        }

        if self.get(Field::Subject).trim().is_empty() {
            return Err("subject is required");
        }

        Ok(self.render())
    }

    /// The message as it would be written, without validating it. An
    /// all-empty draft renders as an empty string.
    pub fn render(&self) -> String {
        let mut message = if self.is_empty() {
            String::new()
        } else {
            self.header()
        };
        for field in [Field::Problem, Field::Solution] {
            let body = self.get(field).trim();
            if !body.is_empty() {
                message.push_str("\n\n");
                message.push_str(field.title());
                message.push_str(":\n");
                message.push_str(body);
            }
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(ty: &str, scope: &str, subject: &str, problem: &str, solution: &str) -> Draft {
        Draft {
            inputs: [ty, scope, subject, problem, solution].map(str::to_owned),
        }
    }

    #[test]
    fn header_only() {
        let d = draft("fix", "", "handle empty repo", "", "");
        assert_eq!(d.build().unwrap(), "fix: handle empty repo");
    }

    #[test]
    fn full_message() {
        let d = draft(
            " feat ",
            "ui",
            "add diff pane",
            "No context.\n",
            "Show `jj diff`.",
        );
        assert_eq!(
            d.build().unwrap(),
            "feat(ui): add diff pane\n\nProblem:\nNo context.\n\nSolution:\nShow `jj diff`."
        );
    }

    #[test]
    fn breaking_change_bang_goes_after_scope() {
        let d = draft("feat!", "api", "drop v1 endpoints", "", "");
        assert_eq!(d.build().unwrap(), "feat(api)!: drop v1 endpoints");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(draft("", "", "x", "", "").build().is_err());
        assert!(draft("feat", "", " ", "", "").build().is_err());
        assert!(draft("new feature", "", "x", "", "").build().is_err());
        assert!(draft("feat", "a b", "x", "", "").build().is_err());
        assert!(draft("feat!!", "", "x", "", "").build().is_err());
        assert!(draft("!", "", "x", "", "").build().is_err());
    }
}
