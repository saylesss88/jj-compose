//! Thin wrapper around the `jj` binary.
//!
//! Every command runs against a single resolved commit ID rather than the
//! user's revset, so `@-` or a bookmark can't move between loading the
//! description and writing it back.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{Error, Result};

/// How to invoke `jj`: which repository to operate on, and whether it may
/// rewrite history jj considers immutable.
#[derive(Debug, Clone, Default)]
pub struct Jj {
    repository: Option<PathBuf>,
    /// Pass `--ignore-immutable` to every command, letting released commits
    /// be rewritten. See [`Jj::orphaned_tags`] for what that costs.
    ignore_immutable: bool,
}

/// A tag that a rewrite would leave pointing at a hidden commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedCommit {
    /// The tag's name, such as `v0.1.0`.
    pub tag: String,
    /// Short commit ID it currently points at.
    pub commit: String,
    /// First line of that commit's description.
    pub summary: String,
}

/// The one revision a session edits.
#[derive(Debug, Clone)]
pub struct Target {
    /// Full commit ID: unambiguous even if the change ID is divergent.
    pub commit_id: String,
    /// Full change ID: stays the same when the commit is rewritten.
    pub change: String,
    /// Shortest unique change ID, for display.
    pub change_id: String,
    /// Whether jj protects this commit as immutable. Only ever true when
    /// [`Jj::ignores_immutable`] is set, since otherwise resolving refuses.
    pub immutable: bool,
}

impl Jj {
    #[must_use]
    pub const fn new(repository: Option<PathBuf>) -> Self {
        Self {
            repository,
            ignore_immutable: false,
        }
    }

    /// Allow rewriting commits jj protects as immutable, such as tagged
    /// releases and anything on trunk.
    #[must_use]
    pub const fn ignoring_immutable(mut self, yes: bool) -> Self {
        self.ignore_immutable = yes;
        self
    }

    /// Whether this is allowed to rewrite immutable history.
    #[must_use]
    pub const fn ignores_immutable(&self) -> bool {
        self.ignore_immutable
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new("jj");
        if let Some(repo) = &self.repository {
            cmd.arg("-R").arg(repo);
        }
        // One place, so describe, split and diff all agree about it.
        if self.ignore_immutable {
            cmd.arg("--ignore-immutable");
        }
        cmd
    }

    /// Run a read-only `jj` command and return its stdout.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The `jj` executable cannot be found or spawned.
    /// * The `jj` command exits with a non-zero status code (the error will contain the command's stderr).
    pub fn output(&self, args: &[&str]) -> Result<Vec<u8>> {
        let output = self
            .command()
            .args(args)
            .arg("--no-pager")
            .stdin(Stdio::null())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            let clean_stderr = stderr.strip_prefix("Error: ").unwrap_or(stderr);

            return Err(Error::CommandFailed {
                args: args.join(" "),
                stderr: clean_stderr.to_owned(),
            });
        }
        Ok(output.stdout)
    }

    /// Resolve `revset` to exactly one mutable revision.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The underlying `jj log` command fails to execute.
    /// * The stdout from `jj` is not valid UTF-8.
    /// * The provided `revset` resolves to zero or multiple revisions instead of exactly one.
    /// * The tab-separated output from `jj log` does not match the expected template structure.
    /// * The resolved revision is marked as immutable.
    pub fn resolve(&self, revset: &str) -> Result<Target> {
        const TEMPLATE: &str = r#"commit_id ++ "\t" ++ change_id ++ "\t" ++ change_id.shortest(8) ++ "\t" ++ if(immutable, "immutable", "mutable") ++ "\n""#;
        let stdout = self.output(&[
            "log",
            "--no-graph",
            "--color=never",
            "--limit=2",
            "-r",
            revset,
            "-T",
            TEMPLATE,
        ])?;
        let stdout = String::from_utf8(stdout)?;
        let mut lines = stdout.lines();
        let (Some(line), None) = (lines.next(), lines.next()) else {
            return Err(Error::AmbiguousRevset(revset.to_owned()));
        };

        let mut parts = line.split('\t');
        let (Some(commit_id), Some(change), Some(change_id), Some(mutability)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(Error::UnexpectedOutput(line.to_owned()));
        };
        if mutability == "immutable" && !self.ignore_immutable {
            return Err(Error::ImmutableRevision(change_id.to_owned()));
        }

        Ok(Target {
            commit_id: commit_id.to_owned(),
            change: change.to_owned(),
            change_id: change_id.to_owned(),
            immutable: mutability == "immutable",
        })
    }

    /// A jj config value, or `None` if it isn't set.
    ///
    /// `jj config get` exits non-zero when the key is missing, so an unset
    /// key and a broken repo are both simply "no value" here.
    #[must_use]
    pub fn config_get(&self, key: &str) -> Option<String> {
        let stdout = self.output(&["config", "get", key]).ok()?;
        let value = String::from_utf8(stdout).ok()?.trim().to_owned();
        (!value.is_empty()).then_some(value)
    }

    /// The current description of `target`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The underlying `jj log` command fails to execute or returns a non-zero status.
    /// * The stdout returned by `jj` contains invalid UTF-8.
    pub fn description(&self, target: &Target) -> Result<String> {
        let stdout = self.output(&[
            "log",
            "--no-graph",
            "--color=never",
            "-r",
            &target.commit_id,
            "-T",
            "description",
        ])?;
        Ok(String::from_utf8(stdout)?)
    }

    /// Show the diff of `target` in the external `tool`, on the real
    /// terminal. `path` (relative to the repository root) limits it to one
    /// file.
    ///
    /// Unlike [`Jj::output`], nothing is captured: the child inherits the
    /// terminal, so a pager or a full-screen tool can run. The caller must
    /// have suspended the TUI first.
    ///
    /// # Errors
    ///
    /// Returns an error if `jj` can't be spawned or exits with a failure.
    pub fn show_diff(&self, target: &Target, tool: &str, path: Option<&str>) -> Result<()> {
        let mut cmd = self.command();
        // No `--no-pager`: a terminal tool's output goes through jj's pager,
        // which is what holds the screen until the user quits it.
        cmd.args(["diff", "-r", &target.commit_id, "--tool", tool]);
        if let Some(path) = path {
            // A bare path would be resolved against the current directory;
            // `root-file:` anchors it at the repository root, which is what
            // the paths in the diff are relative to.
            cmd.arg("--").arg(format!("root-file:{path:?}"));
        }
        let status = cmd.status()?;

        if !status.success() {
            return Err(Error::CommandFailed {
                args: format!("diff -r {} --tool {}", target.commit_id, tool),
                stderr: format!("Process exited with {status}"),
            });
        }
        Ok(())
    }

    /// Split `target` in two: the hunks in the patch at `patch` become a
    /// commit described by `message`, and everything else stays behind in a
    /// new change.
    ///
    /// jj-compose at `exe` runs as jj's diff editor, so what the user chose
    /// in the TUI is what jj sees. The tool is defined on the command line
    /// rather than in the user's config, which this must not touch.
    ///
    /// # Errors
    ///
    /// Returns an error if `jj split` fails, including when the choice would
    /// leave one of the two commits empty.
    pub fn split(&self, target: &Target, message: &str, patch: &Path, exe: &Path) -> Result<()> {
        let (program, edit_args) = diff_editor_config(exe, patch);
        self.output(&[
            "--config",
            &program,
            "--config",
            &edit_args,
            "split",
            "-i",
            "-r",
            &target.change,
            "-m",
            message,
            "--tool",
            "jj-compose",
        ])?;
        Ok(())
    }

    /// Tags on `target` or anything descended from it: exactly the tags a
    /// rewrite of `target` would strand, left pointing at commits that are
    /// about to become hidden.
    ///
    /// Rewriting `target` replaces it and rebases its descendants onto the
    /// result, so every commit from here on gets a new hash while the tags
    /// stay where they were. Re-anchor each with
    /// `jj tag set <tag> --allow-move -r <new revision>`. Tags on ancestors
    /// are untouched and are not reported.
    ///
    /// # Errors
    ///
    /// Returns an error if the `jj log` fails or its output can't be parsed.
    pub fn orphaned_tags(&self, target: &Target) -> Result<Vec<TaggedCommit>> {
        const TEMPLATE: &str =
            r#"tags ++ "\t" ++ commit_id.shortest(8) ++ "\t" ++ description.first_line() ++ "\n""#;
        let revset = format!("tags() & {}::", target.change);
        let stdout = self.output(&["log", "--no-graph", "-r", &revset, "-T", TEMPLATE])?;
        String::from_utf8(stdout)?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let mut parts = line.split('\t');
                let (Some(tag), Some(commit), Some(summary)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    return Err(Error::UnexpectedOutput(line.to_owned()));
                };
                Ok(TaggedCommit {
                    tag: tag.trim().to_owned(),
                    commit: commit.to_owned(),
                    summary: summary.to_owned(),
                })
            })
            .collect()
    }

    /// Set the description of `target`.
    ///
    /// The message goes over stdin rather than `-m`, so a message starting
    /// with `-` can't be mistaken for a flag.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * The `jj describe` command fails to spawn.
    /// * The child process's standard input cannot be captured.
    /// * Writing the commit message to the process's standard input fails.
    /// * Waiting for the process to complete fails.
    /// * The `jj describe` command exits with a non-zero status code.
    pub fn describe(&self, target: &Target, message: &str) -> Result<()> {
        let mut child = self
            .command()
            .args(["describe", &target.change, "--stdin"])
            .stdin(Stdio::piped())
            .spawn()?;
        // .map_err(|e| format!("failed to run jj: {e}"))?;
        child
            .stdin
            .take()
            .ok_or(Error::StdinUnavailable)?
            .write_all(message.as_bytes())?;
        let status = child.wait()?;
        if !status.success() {
            // return Err(format!("jj describe failed ({status})").into());
            return Err(Error::CommandFailed {
                args: format!("describe {} --stdin", target.change),
                stderr: format!("Process exited with {status}"),
            });
        }
        Ok(())
    }

    /// Print the diff in colored git format
    ///
    /// # Errors
    ///
    /// If there is no jj repo found at target
    /// If the underlying jj command fails
    pub fn git_diff(&self, target: &Target) -> Result<Vec<u8>> {
        self.output(&["diff", "-r", &target.commit_id, "--git", "--color=always"])
    }
}

/// A path written as a TOML string: quoted, with backslashes and quotes
/// escaped.
///
/// The quotes are not decoration. `jj --config` parses its value as TOML, so
/// a bare path inside an array makes jj reject the whole value with a parse
/// error rather than treating it as a string.
fn toml_string(path: &Path) -> String {
    let escaped = path
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The two `--config` values that make jj-compose jj's diff editor: the
/// program to run, and the arguments to run it with.
fn diff_editor_config(exe: &Path, patch: &Path) -> (String, String) {
    (
        format!("merge-tools.jj-compose.program={}", toml_string(exe)),
        format!(
            "merge-tools.jj-compose.edit-args=[\"--apply-hunks\",{},\"$left\",\"$right\"]",
            toml_string(patch)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unquoted path makes jj reject the whole `--config` value, which
    /// showed up only as a failed split at the end of a long session.
    #[test]
    fn the_diff_editor_config_quotes_its_paths() {
        let (program, edit_args) = diff_editor_config(
            Path::new("/usr/local/bin/jj-compose"),
            Path::new("/tmp/a patch.patch"),
        );
        assert_eq!(
            program,
            "merge-tools.jj-compose.program=\"/usr/local/bin/jj-compose\""
        );
        assert_eq!(
            edit_args,
            "merge-tools.jj-compose.edit-args=[\"--apply-hunks\",\"/tmp/a patch.patch\",\"$left\",\"$right\"]"
        );
    }
}
