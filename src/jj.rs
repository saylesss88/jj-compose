//! Thin wrapper around the `jj` binary.
//!
//! Every command runs against a single resolved commit ID rather than the
//! user's revset, so `@-` or a bookmark can't move between loading the
//! description and writing it back.

use std::{
    error::Error,
    io::Write,
    path::PathBuf,
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
    /// Shortest unique change ID, for display.
    pub change_id: String,
}

impl Jj {
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
    pub fn resolve(&self, revset: &str) -> Result<Target> {
        const TEMPLATE: &str = r#"commit_id ++ "\t" ++ change_id.shortest(8) ++ "\t" ++ if(immutable, "immutable", "mutable") ++ "\n""#;
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
        let (Some(commit_id), Some(change_id), Some(mutability)) =
            (parts.next(), parts.next(), parts.next())
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
            change_id: change_id.to_owned(),
        })
    }

    /// The current description of `target`.
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

    /// Set the description of `target`.
    ///
    /// The message goes over stdin rather than `-m`, so a message starting
    /// with `-` can't be mistaken for a flag.
    pub fn describe(&self, target: &Target, message: &str) -> Result<()> {
        let mut child = self
            .command()
            .args(["describe", &target.commit_id, "--stdin"])
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
