//! Thin wrapper around the `jj` binary.
//!
//! Every command runs against a single resolved commit ID rather than the
//! user's revset, so `@-` or a bookmark can't move between loading the
//! description and writing it back.

use std::{
    error::Error,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// How to invoke `jj`: which repository to operate on.
#[derive(Debug, Clone, Default)]
pub struct Jj {
    repository: Option<PathBuf>,
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
}

impl Jj {
    #[must_use]
    pub const fn new(repository: Option<PathBuf>) -> Self {
        Self { repository }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new("jj");
        if let Some(repo) = &self.repository {
            cmd.arg("-R").arg(repo);
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
            .output()
            .map_err(|e| format!("failed to run jj: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            // We add our own prefix; don't print `jj-compose: Error: ...`.
            return Err(stderr.strip_prefix("Error: ").unwrap_or(stderr).into());
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
            return Err(format!("revset `{revset}` must resolve to exactly one revision").into());
        };

        let mut parts = line.split('\t');
        let (Some(commit_id), Some(change), Some(change_id), Some(mutability)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(format!("unexpected output from jj log: {line:?}").into());
        };
        if mutability == "immutable" {
            return Err(format!(
                "change {change_id} is immutable; jj describe would refuse to rewrite it"
            )
            .into());
        }

        Ok(Target {
            commit_id: commit_id.to_owned(),
            change: change.to_owned(),
            change_id: change_id.to_owned(),
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
        let status = cmd.status().map_err(|e| format!("failed to run jj: {e}"))?;
        if !status.success() {
            return Err(format!("jj diff failed ({status})").into());
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
        let program = format!("merge-tools.jj-compose.program={}", exe.display());
        let edit_args = format!(
            "merge-tools.jj-compose.edit-args=[\"--apply-hunks\",{},\"$left\",\"$right\"]",
            patch.display()
        );
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
            .spawn()
            .map_err(|e| format!("failed to run jj: {e}"))?;
        child
            .stdin
            .take()
            .ok_or("jj stdin unavailable")?
            .write_all(message.as_bytes())?;
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("jj describe failed ({status})").into());
        }
        Ok(())
    }
}
