# jj-compose-core

The library behind [`jj-compose`](https://github.com/saylesss88/jj-compose): a
wrapper around the `jj` binary, a model of
[Conventional Commit](https://www.conventionalcommits.org/) messages, and the
machinery for splitting a change into hunks.

It has no user interface and no UI dependencies. A terminal frontend is one
consumer, but nothing here assumes one.

## Requirements

The `jj` binary must be on `PATH`. Everything in `jj` shells out to it rather
than linking against `jj-lib`, so this crate stays small and tracks whatever
`jj` the user already has.

## What's in it

| Module     | What it does                                                                                          |
| ---------- | ----------------------------------------------------------------------------------------------------- |
| `jj`       | Run `jj` commands against one resolved revision: read its description and diff, describe it, split it |
| `message`  | Parse and render `type(scope): subject` messages with `Problem:`/`Solution:` sections and trailers    |
| `patch`    | Parse a git-format diff into files and hunks, and build a patch of a chosen subset                    |
| `marks`    | Track which hunks the user picked, independently of where they are in the diff                        |
| `plan`     | Carry the remaining boxes across a split, keyed by hunk content                                       |
| `split`    | Write a chosen subset into the two trees `jj split -i` hands a diff editor                            |
| `overview` | Lay out every hunk in a change as aligned rows, with its box and enclosing definition                 |
| `symbols`  | Find the function or type each hunk sits inside, which `jj diff --git` leaves out                     |
| `error`    | One `Error` enum, and a `Result` alias                                                                |

## Describing a change

A revision is resolved once, to a commit ID and a change ID, so a revset like
`@-` can't drift between reading the description and writing it back:

```rust
use jj_compose_core::{Draft, Field, Jj, Result};

fn retitle() -> Result<()> {
    let jj = Jj::new(None);
    let target = jj.resolve("@")?;

    let mut draft = match Draft::parse(&jj.description(&target)?) {
        Ok(draft) => draft,
        // The message doesn't fit the form; leave it alone rather than
        // reshaping it behind the author's back.
        Err(_) => return Ok(()),
    };
    draft.set(Field::Type, "fix");
    draft.set(Field::Subject, "handle empty input");

    let message = draft.build().map_err(|e| {
        jj_compose_core::Error::UnexpectedOutput(e.to_owned())
    })?;
    jj.describe(&target, &message)
}
```

`Draft::parse` is deliberately strict. It returns `Err` unless the form can
reproduce the message byte for byte, so round-tripping an existing description
never silently rewrites it. `build()` validates the result and returns the
message text.

`resolve` refuses an immutable revision unless the `Jj` was built with
`ignoring_immutable(true)`; see `orphaned_tags` for what rewriting one costs.

## Choosing hunks

`Patch` parses a diff and `Split` holds the plan: which of several commits each
hunk is destined for. The two are separate on purpose — position and plan are
independent, so a caller can read the whole change before deciding any of it,
and move a hunk between commits until the split is made.

```rust
use jj_compose_core::{Jj, Patch, Result, Split, patch};

fn first_commits_patch() -> Result<String> {
    let jj = Jj::new(None);
    let target = jj.resolve("@")?;

    let diff = String::from_utf8_lossy(&jj.git_diff(&target)?).into_owned();
    let parsed = Patch::from_text(&patch::strip_ansi(&diff));

    let hunks = parsed.files().iter().enumerate().flat_map(|(f, file)| {
        (0..file.hunks.len()).map(move |h| {
            (jj_compose_core::HunkId { file: f, hunk: h }, file.path.clone())
        })
    });
    // `None` when there are fewer than two hunks: nothing to split.
    let Some(mut split) = Split::new(hunks) else {
        return Ok(String::new());
    };

    // Boxes are numbered by the commit they will become, and committed
    // lowest first, so the numbering is also the order.
    split.assign(Some(1));
    split.next();
    split.assign_file(Some(2));

    // A git-format patch of just commit 1's hunks, to apply to the tree as
    // it was before the change.
    Ok(parsed.subset(&split.picked(1)))
}
```

`strip_ansi` is there because `git_diff` asks `jj` for color, so the caller gets
the user's own diff colors; the escape codes have to come off before parsing.

Splitting a change into more than two commits means committing one box, then
re-finding the rest: a `HunkId` only means anything within one diff, and
committing part of a change shifts every later hunk's line numbers. Use `Plan`
to carry the remaining boxes across, it keys them by each hunk's text, which a
split leaves untouched:

```rust
use jj_compose_core::{Patch, Plan, Split};

/// `before` and `split` describe the change as it was planned; `after` is
/// the remainder's diff once the lowest box has been committed, and `fresh`
/// a `Split` built from it.
fn carry_over(before: &Patch, split: &Split, after: &Patch, fresh: &mut Split) {
    let plan = Plan::capture(before, split);
    plan.apply(after, fresh);
    // Box numbers are kept, not renumbered: commit `fresh.pending()` lowest
    // first, as before. Hunks that aren't found were the ones just committed.
}
```

Capture the plan _before_ committing, since afterwards the old diff is gone.

## Seeing the whole change

`overview::build` lays out every hunk at once: a heading per file, then a row
per hunk with its box, size, and the definition it sits in. With the column
alignment done here so it can be tested without a terminal. The caller only
picks colors. The definitions come from `symbols::for_patch`, which reads each
file back through `jj` because `jj diff --git` doesn't put them in the hunk
headers the way `git diff` does.

## Splitting

`jj split -i` runs a diff editor with two directories: `left` holds the files as
they were, `right` as they are. Whatever the editor leaves in `right` becomes
the first of two commits.

`Jj::split` drives that, pointing `jj` at an executable you supply as the diff
editor. The usual arrangement is for a binary to re-run itself in a mode that
calls `apply_selection`:

```rust
use std::{env, fs, path::Path};
use jj_compose_core::{Jj, Result, Target, split};

/// The caller's own binary, invoked by `jj` as the diff editor.
fn as_diff_editor(patch: &Path, left: &Path, right: &Path) -> Result<()> {
    split::apply_selection(&fs::read_to_string(patch)?, left, right)
}

fn commit_marked(jj: &Jj, target: &Target, message: &str, subset: &str) -> Result<()> {
    let patch = env::temp_dir().join("chosen.patch");
    fs::write(&patch, subset)?;
    let exe = env::current_exe()?;
    let result = jj.split(target, message, &patch, &exe);
    let _ = fs::remove_file(&patch);
    result
}
```

Two things worth knowing about the result:

- The commit that was split off **keeps the original change ID**; the remainder
  gets a new one and inherits the description. The remainder is reachable as
  `<change>+`, which holds even when the change already had descendants, since
  they get rebased onto it.
- Files the patch doesn't mention are reset to their before-state in `right`,
  which is how `jj` is told those changes belong in the other commit.
  `apply_selection` handles that, along with added files, deleted files and
  nested paths.

## Errors

Every fallible call returns `Result<T, Error>`. `Error` is `#[non_exhaustive]`
and carries the detail rather than flattening it: `CommandFailed` holds the
arguments and `jj`'s own stderr, and `Io` and `Utf8` keep their sources.

Binaries are expected to add their own context on top — `anyhow` or similar —
rather than this crate guessing at presentation.

## What it doesn't do

No rendering, no key handling, no terminal. Those live in the `jj-compose`
binary. Hunks are also the smallest unit here: there's no splitting a hunk by
line, and a binary file has no hunks, so its change can't be separated from the
rest.

## License

[Apache-2.0](https://github.com/saylesss88/jj-compose/blob/main/LICENSE)
