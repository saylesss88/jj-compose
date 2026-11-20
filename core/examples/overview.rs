//! Print the board for a revision: every hunk, the file it's in and the
//! definition it sits inside. Read-only.
//!
//! Run inside a jj repository:
//!
//! ```sh
//! cargo run -p jj_compose_core --example overview          # @
//! cargo run -p jj_compose_core --example overview -- @-    # any revset
//! ```

use jj_compose_core::{Jj, Patch, Result, overview, patch, symbols};

fn main() -> Result<()> {
    let revset = std::env::args().nth(1).unwrap_or_else(|| "@".into());

    let jj = Jj::new(None);
    let target = jj.resolve(&revset)?;

    // `git_diff` asks jj for color; strip it before parsing.
    let diff = String::from_utf8_lossy(&jj.git_diff(&target)?).into_owned();
    let parsed = Patch::from_text(&patch::strip_ansi(&diff));

    if parsed.ids().is_empty() {
        println!("{} has no hunks", target.change_id);
        return Ok(());
    }

    // Nothing is planned yet, so every hunk is loose.
    let plan = vec![overview::Where::Nowhere; parsed.ids().len()];
    let symbols = symbols::for_patch(&jj, &target, &parsed)?;

    println!("{}", target.change_id);
    for row in overview::build(&parsed, &plan, &symbols) {
        println!("{}", row.text);
    }
    Ok(())
}
