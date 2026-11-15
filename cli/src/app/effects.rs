//! The operations that leave the TUI: running `jj`, and handing the
//! terminal over to another program.

use std::{
    fs,
    io::{self, Write},
    process,
};

use jj_compose_core::{Draft, Field, Jj, Plan, Split, symbols};
use ratatui::{
    DefaultTerminal,
    crossterm::{
        event::{DisableBracketedPaste, EnableBracketedPaste},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

use super::{App, Focus, Request, Status, View, ui};
use crate::{app, diff};

impl App {
    /// Carry out a [`Request`] with the TUI out of the way.
    pub(super) fn perform(
        &mut self,
        terminal: &mut DefaultTerminal,
        jj: &Jj,
        request: &Request,
    ) -> io::Result<()> {
        match request {
            // One `jj file show` per file, so it is done once per diff and
            // only when the overview is actually wanted.
            Request::Symbols => {
                self.symbols =
                    symbols::for_patch(jj, &self.target, self.diff.patch()).unwrap_or_default();
            }
            Request::Split { message } => {
                if let Err(e) = self.run_split(jj, message) {
                    self.status = Some(Status::Error(e));
                }
            }
            Request::OpenDiff { path } => {
                // With no tool there is nothing to open but jj's own diff,
                // which is the one already on screen; say so instead.
                let Some(tool) = self.diff_tool.clone() else {
                    self.status = Some(Status::Error(
                        "no diff tool: set ui.diff-editor in your jj config, or pass --diff-tool"
                            .into(),
                    ));
                    return Ok(());
                };
                let target = self.target.clone();
                let result = suspend(terminal, || {
                    // A GUI tool can take a moment to appear, and the screen
                    // is blank until it does.
                    println!("Opening {tool}...");
                    let result = jj.show_diff(&target, &tool, path.as_deref());
                    // jj pages a terminal tool's output and a GUI holds its
                    // own window, so only a failure needs the screen held:
                    // its message would otherwise vanish with the redraw.
                    if result.is_err() {
                        wait_for_key("Press Enter to return to jj-compose ");
                    }
                    result.map_err(|e| e.to_string())
                })?;
                if let Err(e) = result {
                    self.status = Some(Status::Error(e));
                }
            }
        }
        Ok(())
    }

    /// Commit the chosen hunks and carry on with what is left.
    ///
    /// Nothing is taken from the UI until `jj split` has succeeded, so a
    /// failure leaves the choice intact to try again.
    fn run_split(&mut self, jj: &Jj, message: &str) -> Result<(), String> {
        let Some(split) = self.split.as_ref() else {
            return Ok(());
        };
        // Lowest box first, so the numbering is the order the commits end up
        // in.
        let Some(commit) = split.next_box() else {
            return Ok(());
        };
        let added = split.count(commit);
        let text = self.diff.patch().subset(&split.picked(commit));

        // A HunkId only means anything within one diff, so whatever is still
        // planned has to be recorded by content before the split moves it.
        self.plan = Plan::capture(self.diff.patch(), split);

        // jj runs jj-compose as its diff editor and passes it this patch.
        let path = std::env::temp_dir().join(format!("jj-compose-{}.patch", process::id()));
        fs::write(&path, text).map_err(|e| format!("couldn't write the patch: {e}"))?;
        let exe = std::env::current_exe().map_err(|e| format!("couldn't find jj-compose: {e}"))?;
        let result = jj.split(&self.target, message, &path, &exe);
        let _ = fs::remove_file(&path);
        result.map_err(|e| e.to_string())?;

        let left = self.reload(jj)?;
        let waiting = self.split.as_ref().map_or(0, Split::assigned);
        // Rebuilt rather than appended: each tag needs re-anchoring once, to
        // whatever the remainder is after the last split of the session.
        self.recovery = self
            .at_risk
            .iter()
            .flat_map(|t| {
                [
                    format!(
                        "jj tag set {} --allow-move -r {}    # was {} ({})",
                        t.tag, self.target.change_id, t.commit, t.summary
                    ),
                    format!("git push origin {} --force", t.tag),
                ]
            })
            .collect();
        self.status = Some(Status::Info(if waiting > 0 {
            let next = self.split.as_ref().and_then(Split::next_box).unwrap_or(0);
            let planned = self.split.as_ref().map_or(0, Split::planned);
            format!(
                "Committed {added} {} as commit {commit} of {planned}. Now commit {next}: {waiting} {}.",
                app::plural(added, "hunk"),
                app::plural(waiting, "hunk"),
            )
        } else {
            format!(
                "Committed {added} {}. {left} {} left to describe.",
                app::plural(added, "hunk"),
                app::plural(left, "hunk"),
            )
        }));
        Ok(())
    }

    /// Move to what is left of the change after a split, and start again on
    /// it: new diff, the description jj carried over, and a fresh split.
    fn reload(&mut self, jj: &Jj) -> Result<usize, String> {
        // The hunks that were split off keep the old change ID; the
        // remainder is a new change, and the only child of that commit.
        let remainder = format!("{}+", self.target.change);
        self.target = jj.resolve(&remainder).map_err(|e| e.to_string())?;
        self.change_id = self.target.change_id.clone();
        self.diff = diff::load(jj, &self.target).map_err(|e| e.to_string())?;
        // A new diff means the old symbols describe the wrong lines.
        self.symbols.clear();
        self.overview = None;

        let description = jj.description(&self.target).map_err(|e| e.to_string())?;
        let original = description.trim_end().to_owned();
        let (draft, view) = Draft::parse(&original).map_or_else(
            |_| (Draft::default(), View::Raw),
            |draft| (draft, View::Structured),
        );
        self.fields = Field::ALL.map(|field| ui::field_editor(field, draft.get(field)));
        self.raw = ui::raw_editor(&original);
        self.view = view;
        self.original = original;
        self.field = Field::Type;
        if let Some(mut split) =
            Split::new(self.diff.hunks().iter().map(|h| (h.id, h.path.clone())))
        {
            // Put the still-uncommitted hunks back in the boxes they were
            // planned into, which their new HunkIds cannot say.
            self.plan.apply(self.diff.patch(), &mut split);
            let waiting = split.assigned() > 0;
            self.enter_split(split);
            if waiting {
                // The next commit needs a message of its own; the
                // description jj carried over belongs to whatever is left
                // at the very end.
                self.fields = Field::ALL.map(|field| ui::field_editor(field, ""));
                self.raw = ui::raw_editor("");
                self.view = View::Structured;
                self.original = String::new();
                self.focus = Focus::Editor;
                self.zoomed = false;
            }
        } else {
            self.split = None;
            self.focus = Focus::Editor;
            self.zoomed = false;
        }
        Ok(self.diff.hunks().len())
    }
}

/// Run `f` with the terminal handed back to the shell (cooked mode, main
/// screen), then take it over again and redraw from scratch.
fn suspend<T>(terminal: &mut DefaultTerminal, f: impl FnOnce() -> T) -> io::Result<T> {
    disable_raw_mode()?;
    execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen)?;
    let out = f();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    // ratatui only redraws cells it thinks changed; after another program
    // has used the screen, everything has.
    terminal.clear()?;
    Ok(out)
}

fn wait_for_key(prompt: &str) {
    print!("\n{prompt}");
    let _ = io::stdout().flush();
    let _ = io::stdin().read_line(&mut String::new());
}
