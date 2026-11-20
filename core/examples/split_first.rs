//! Split the first hunk of a revision off into its own commit.
//!
//! This shows the one non-obvious part of the library: `Jj::split` needs an
//! executable for `jj` to run as its diff editor, and the simplest one is
//! this same binary, called back with `--apply-hunks <patch> <left> <right>`.
//! `main` checks for that first, before doing anything else.
//!
//! **Warning! This rewrites history.** Run it in a throwaway repo:
//!
//! ```sh
//! # Run from the jj-compose repo root:
//! REPO=$(pwd)
//! jj git init /tmp/demo && cd /tmp/demo
//! printf 'a\n' > one.txt && printf 'b\n' > two.txt
//! cargo run --manifest-path "$REPO/Cargo.toml" \
//!     -p jj_compose_core --example split_first -- "feat: add one.txt"
//! jj log
//! ```

use std::{env, fs, path::Path};

use jj_compose_core::{Error, Jj, Patch, Result, Split, patch, split};

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();

    // Called back by jj mid-split, as the diff editor. Not a session at all:
    // write the chosen hunks into `right` and exit.
    if let [flag, patch_file, left, right] = args.as_slice()
        && flag == "--apply-hunks"
    {
        let text = fs::read_to_string(patch_file)?;
        return split::apply_selection(&text, Path::new(left), Path::new(right));
    }

    let message = args
        .first()
        .cloned()
        .unwrap_or_else(|| "chore: first hunk".into());

    let jj = Jj::new(None);
    let target = jj.resolve("@")?;

    let diff = String::from_utf8_lossy(&jj.git_diff(&target)?).into_owned();
    let parsed = Patch::from_text(&patch::strip_ansi(&diff));

    let hunks = parsed.ids().into_iter().map(|id| {
        let file = &parsed.files()[id.file];
        let changes = parsed.changed_lines(&file.hunks[id.hunk]).len();
        (id, file.path.clone(), changes)
    });
    let Some(mut plan) = Split::new(hunks) else {
        return Err(Error::UnexpectedOutput(
            "nothing to split: one hunk with one change in it".into(),
        ));
    };

    // The cursor starts on the first hunk; put it in box 1.
    plan.assign(Some(1));
    let subset = parsed.subset(&plan.picked(1));

    // jj reads the patch from a file, then runs this binary as its diff editor.
    let patch_file = env::temp_dir().join(format!("split-first-{}.patch", std::process::id()));
    fs::write(&patch_file, subset)?;
    let result = jj.split(&target, &message, &patch_file, &env::current_exe()?);
    let _ = fs::remove_file(&patch_file);
    result?;

    // The split-off commit keeps the change ID; the rest is now its child.
    println!("split {} into {0} and {0}+", target.change_id);
    Ok(())
}
