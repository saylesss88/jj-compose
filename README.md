# jj-compose

An interactive terminal UI for Jujutsu ([jj](https://jj-vcs.github.io/jj/)) that
helps you split working-copy changes into atomic, structured
[Conventional Commits](https://www.conventionalcommits.org/) while viewing the
diff.

## Features

- **Strict Conventional Commits:** Draft structured messages with automatic
  validation for types, scopes, header lengths (50/72 chars), and Git trailers.
- **Interactive Hunk Splitting:** Press `Alt-s` to review your change hunk by
  hunk. Mark the pieces that belong together and commit them instantly—zero
  external `ui.diff-editor` configuration required.
- **Live, Responsive Diffing:** View the full `jj` diff alongside your commit
  form. The layout automatically stacks on narrow terminals and flags stale
  diffs if you edit files in another window.
- **Rewriting released history:** `--ignore-immutable` splits even a tagged
  release, naming the tags it strands and printing the commands to re-anchor
  them.
- **First-Class Vim Keymap:** Use intuitive UI defaults, or launch with
  `--keymap vim` for full Normal/Insert mode modal editing (supports standard
  motions, operators, and visual selections).
- **Lossless Parsing:** Safely revises existing descriptions. If an old commit
  message doesn't perfectly match the Conventional format, it opens in a raw
  text editor to prevent accidental mangling.
- **External Tool Integration:** Press `o` to instantly open the current file or
  change in your configured external diff viewer (like Meld or Difftastic).

```text
┌ Diff xuurlzoz · 1/2 README.md +2 -0 ───────────┐┌ Type ────────────┐┌ Scope ─────────────────────┐
│README.md  +2 -0                                ││fix               ││parser                      │
│@@ -1,1 +1,3 @@                                 │└──────────────────┘└────────────────────────────┘
│ # demo                                         │┌ Subject 31/72 ─────────────────────────────────┐
│+                                               ││handle empty input                              │
│+Empty input now parses to no tokens.           │└────────────────────────────────────────────────┘
│                                                │┌ Body ──────────────────────────────────────────┐
│src/parser.rs  +1 -1                            ││This also corrects an unrelated diagnostic.     │
│@@ -1,7 +1,7 @@                                 ││                                                │
│ pub fn parse(input: &str) -> Vec<Token> {      ││                                                │
│     let len = input.len();                     │└────────────────────────────────────────────────┘
│     if len == 0 {                              │┌ Problem ───────────────────────────────────────┐
│-        panic!("empty input");                 ││ What is wrong or missing? (optional)           │
│+        return Vec::new();                     ││                                                │
│     }                                          │└────────────────────────────────────────────────┘
│     lex(input)                                 │┌ Solution ──────────────────────────────────────┐
│ }                                              ││ How does this change address it? (optional)    │
│                                                ││                                                │
│                                                │└────────────────────────────────────────────────┘
│                                                │┌ Trailers ──────────────────────────────────────┐
│                                                ││Fixes #42                                       │
└────────────────────────────────────────────────┘└────────────────────────────────────────────────┘
 Tab next · Enter next/newline · Alt-← diff · Alt-s split · ^t raw/form · ^s save · Esc quit
```

## Install

```sh
cargo install jj-compose
```

Install the pre-compiled binary with `cargo-binstall`:

```sh
cargo binstall jj-compose
```

Add an alias so it runs as `jj compose`:

```toml
# ~/.config/jj/config.toml
[aliases]
compose = ["util", "exec", "--", "jj-compose"]
```

To always use the Vim keymap, add it to the alias:

```toml
compose = ["util", "exec", "--", "jj-compose", "--keymap", "vim"]
```

---

## Quick start

You've been working for an hour and `@` now holds a bug fix, a refactor, and a
docs tweak. That's one change with three commits' worth of work in it. Run:

```sh
jj compose
```

1. **Press `Alt-s`.** The diff pane switches to split mode and shows one hunk at
   a time.
2. **Press `f`** to see every hunk in the change at once, with the function each
   one sits in.
3. **Assign hunks with digits.** Move with `j`/`k` and press `1` on each hunk
   that belongs to the fix and `2` on each one that belongs to the refactor.
   Leave the docs hunks unassigned so they stay in the remainder. Lower numbers
   are committed first, so `1` becomes the earliest commit. Nothing is final
   yet: press a digit again or `0` to take a hunk back out.
4. **Press `Enter`** to go to the form. The status bar shows
   `Describing commit 1 of 3`.
5. **Write the message and press `Ctrl-s`.** jj-compose commits those hunks,
   reopens on what's left, and moves on to commit 2. Do the same for that one.
6. **Describe the remainder last.** The docs tweak is all that's left, and its
   message goes on `@`.

The result:

```text
@  docs: clarify empty-input behavior
○  refactor(lexer): extract token helpers
○  fix(parser): return no tokens for empty input
```

You get three atomic commits, ordered so each one builds, and you never left the
terminal or touched `ui.diff-editor`.

If you change your mind partway through, press `Alt-←` to go back to the hunks
and change the marks. To make a single commit with no split, skip `Alt-s`, fill
in the form, and press `Ctrl-s`.

## Usage

```sh
jj-compose              # describe @
jj-compose -r @-        # revise the parent's description
jj-compose <revset>     # any single mutable revision
```

| Option                    | Effect                                                       |
| ------------------------- | ------------------------------------------------------------ |
| `-r, --revision <REVSET>` | Revision to describe (or pass it positionally). Default `@`. |
| `-R, --repository <PATH>` | Repository to operate on, as with `jj -R`.                   |
| `--keymap <default\|vim>` | Key bindings. Also set by `JJ_COMPOSE_KEYMAP`.               |
| `--raw`                   | Start in the raw text editor instead of the form.            |
| `--diff-tool <NAME>`      | Tool `o`/`O` open the diff in. Also `JJ_COMPOSE_DIFF_TOOL`.  |
| `--ignore-immutable`      | Allow rewriting protected history. See below.                |

The revset must resolve to exactly one mutable revision; immutable ones are
rejected before the TUI opens. It's resolved once, at launch, so `@-` or a
bookmark can't drift to a different change while you're writing.

Saving runs `jj describe` on that change by its change ID, so it still finds the
right change if you edit files while jj-compose is open (jj rewrites the commit
when it snapshots your edits). Saving a message identical to the current one
does nothing.

## The message

```text
type(scope)!: subject

Free-form body text.

Problem:
What is wrong or missing.

Solution:
How this change addresses it.

Fixes #42
Reviewed-by: Name <email>
```

Only the header is required. A `!` at the end of the Type field marks a breaking
change and is written after the scope. Trailers are the final paragraph of
`Token: value` or `Token #value` lines (`BREAKING CHANGE:` included); they are
checked when you save.

The Subject title shows the header's length: green up to 50 characters, yellow
up to 72, red beyond.

### Revising existing descriptions

On launch the revision's current description is loaded. It opens in the form
**only if the form can reproduce it byte for byte**; otherwise it opens in the
raw editor, unchanged, and the status bar says why (for example, the first line
isn't `type(scope): subject`). Opening and saving a message never reshapes it
behind your back.

`Ctrl-t` switches between the form and the raw text at any time. Switching to
the form from raw text follows the same rule, so it's refused (with the reason)
until the text fits. Saving from the raw view writes the text exactly as typed,
without Conventional Commit validation.

## The diff pane

The diff is shown in git format with jj's colors, so your jj color settings
apply, but `ui.diff-formatter` does not. Each file starts with a title row
giving its path and how many lines were added and removed. The pane's title
shows which file is at the top, how many there are, and the current search.

- **Files and hunks:** `]` / `[` jump to the next / previous file, `}` / `{` to
  the next / previous hunk. `f` shows a list of every file with its counts down
  the side of the pane.
- **Search:** `/` opens a prompt in the status bar; `Enter` jumps to the first
  match, `n` / `N` move between matches. Searches ignore case unless they
  contain a capital letter.
- **Zoom:** `z` gives the diff the whole window, until you move back to the
  form.
- **Splitting:** `s` (or `Alt-s` from the form) reads the change one hunk at a
  time and marks the ones to commit together, so each piece gets its own
  message. See [Splitting a change](#splitting-a-change).
- **Stale diff:** when you're describing `@`, editing a file that's in the diff
  (in another terminal or your editor) flags the pane `stale: r to reload`
  within a second. `r` reloads it and keeps your place and search. New files
  that weren't in the diff show up on the next reload.

On terminals narrower than 100 columns the diff sits above the form instead of
beside it, and moving between the two follows the layout on screen.

## Keys

### Everywhere

| Key                 | Action                                |
| ------------------- | ------------------------------------- |
| `Ctrl-s`            | Save and run `jj describe`            |
| `Ctrl-c`            | Quit (asks again if there are edits)  |
| `Ctrl-t`            | Switch between form and raw text      |
| `Alt-s`             | Start splitting the change            |
| `Tab` / `Shift-Tab` | Next / previous field, then the diff  |
| `Alt-←` / `Alt-→`   | Jump to the diff / back to the editor |
| `PgDn` / `PgUp`     | Scroll the diff half a page           |
| `Alt-↓` / `Alt-↑`   | Scroll the diff one line              |

### Moving between boxes

The form is laid out with Type and Scope side by side above Subject, and the
other fields below. Moving past the edge of a field's text goes to whatever is
next to it on screen:

```text
 diff  │  Type  ⇄  Scope          diff (narrow terminals)
       │      Subject              ──────────────────────
       │      Body                   Type  ⇄  Scope
       │      ...                        Subject
       │      Trailers                   ...
```

- From Type or Scope, down goes to Subject; up from Subject returns to whichever
  of the two you left last.
- The diff is reached by going left from the start of a field, or up from Type
  or Scope on narrow terminals.
- Entering a field from the side puts the cursor on the side you came from.

Tab and `Enter` always follow the plain top-to-bottom order.

### Default keymap

The form opens ready to type in Type.

| Key                                    | Action                                                        |
| -------------------------------------- | ------------------------------------------------------------- |
| `Enter`                                | Next field (one-line fields) or newline                       |
| Arrows at a field's edge               | The field in that direction (←/→ only between Type and Scope) |
| `Esc`                                  | Quit (asks again if there are edits)                          |
| `Shift`+arrows                         | Select                                                        |
| `Ctrl-x` / `Alt-w` / `Ctrl-y`          | Cut / copy / paste (internal clipboard)                       |
| `Ctrl-z` or `Ctrl-u` / `Ctrl-r`        | Undo / redo                                                   |
| `Ctrl-←` / `Ctrl-→`, `Alt-b` / `Alt-f` | Word left / right                                             |
| `Ctrl-w`, `Alt-Backspace` / `Alt-d`    | Delete word before / after                                    |
| `Home` / `End`, `Ctrl-a` / `Ctrl-e`    | Line start / end                                              |
| `Ctrl-k` / `Ctrl-j`                    | Delete to end / start of line                                 |

The arrow keys never move you into the diff, so holding one down can't leave the
form; use `Alt-←` or Tab for that.

Pasting from the terminal inserts text as-is; in one-line fields newlines become
spaces.

### Vim keymap

`--keymap vim` or `JJ_COMPOSE_KEYMAP=vim`. Starts in Insert mode for a new
description and Normal mode when revising one; the status bar shows the mode.
Insert mode uses the default keymap's editing keys.

| Key                                                 | Action                                |
| --------------------------------------------------- | ------------------------------------- |
| `h` `j` `k` `l`, `w` `b` `e`, `0` `^` `$`, `gg` `G` | Motions                               |
| `h` `j` `k` `l` past the edge of a field's text     | The box in that direction (see above) |
| `Enter`                                             | Next field                            |
| `i` `a` `I` `A` `o` `O`                             | Insert                                |
| `x` `D` `C` `S` `J`                                 | Edit                                  |
| `d` `c` `y` + motion, `dd` `cc` `yy`, `p` `P`       | Operators and paste                   |
| `v` / `V`                                           | Visual / visual line                  |
| `u` / `Ctrl-r`                                      | Undo / redo                           |
| `Ctrl-d` / `Ctrl-u`, `Ctrl-e` / `Ctrl-y`            | Scroll the diff                       |
| `q`                                                 | Quit (asks again if there are edits)  |

### Diff pane

| Key                          | Action                                                       |
| ---------------------------- | ------------------------------------------------------------ |
| `j` / `k`, `↓` / `↑`         | Scroll a line                                                |
| `Ctrl-d` / `Ctrl-u`, `Space` | Half page                                                    |
| `g` / `G`, `Home` / `End`    | Top / bottom                                                 |
| `]` / `[`                    | Next / previous file                                         |
| `}` / `{`                    | Next / previous hunk                                         |
| `f`                          | Show / hide the file list                                    |
| `/`, then `n` / `N`          | Search, next / previous match                                |
| `z`                          | Zoom the diff to the whole window                            |
| `r`                          | Reload the diff                                              |
| `H` / `L`, `←` / `→`         | Pan sideways                                                 |
| `l`, `Esc`, `Enter`, `i`     | Back to the editor                                           |
| `s`                          | Start splitting the change                                   |
| `o` / `O`                    | Open the current file / the whole change in an external tool |
| `q` (Vim keymap)             | Quit (asks again if there are edits)                         |

---

## Splitting a change

A change that does three things needs three messages. `Alt-s` (or `s` in the
diff pane) fills the pane with one hunk at a time so each can be read on its
own, and you mark the ones that belong together.

Moving decides nothing, and nothing is final until you commit. Hunks go into
numbered boxes, one per commit you mean to make, and you can move a hunk between
them or take it out again right up to the split:

| Key                             | Action                                                     |
| ------------------------------- | ---------------------------------------------------------- |
| `1`–`9`                         | Put this hunk in that commit                               |
| the same digit again, or `0`    | Take it back out                                           |
| `n` / `p`, `}` / `{`, `→` / `←` | Next / previous hunk                                       |
| `g` / `G`                       | First / last hunk                                          |
| `a` / `d`                       | Put the rest of this file in the same commit / take it out |
| `f`                             | Show the whole change as a list                            |
| `Enter`                         | Done planning; go write the messages                       |
| `Esc`, `q`                      | Leave split mode, discarding the plan                      |
| `j` / `k`, `Ctrl-d` / `Ctrl-u`  | Scroll inside a hunk taller than the pane                  |

**The numbering is the order.** Boxes are committed lowest first, so putting a
hunk in commit 1 makes it the earliest commit regardless of where it sits in the
diff. That is how you order a decomposition so each commit builds.

The title carries the whole plan while only one hunk is on screen:

```text
 Split xuurlzoz · hunk 2/5 · src/app.rs · 1[2]·2· · commit 2 · 4/5 planned
```

A digit per hunk, `·` for one staying behind, and brackets round the hunk you
are looking at.

`f` swaps the single hunk for the whole change at once, which is usually how you
work out what belongs together:

```text
┌ Hunks in wvkqrwwn · 3 of 6 assigned · 2 commits ┐
│ README.md  +1 -1  0 of 1 assigned               │
│     8    +1   -1  line CHANGED                  │
│ cli/src/app.rs  +2 -2  1 of 2 assigned          │
│ 2   1    +1   -1  impl App                      │
│    18    +1   -1  fn plural()                   │
│ core/src/jj.rs  +3 -3  2 of 3 assigned          │
│ 1   1    +1   -1  fn resolve()                  │
│ 1  14    +1   -1  fn describe()                 │
│    28    +1   -1  fn split_change()             │
└─────────────────────────────────────────────────┘
```

Each file heading carries its totals and how many of its hunks are spoken for —
`1 of 3 assigned` is what tells you a file you thought you had finished still
has something left in it. Each hunk row gives its commit, its line number, its
size, and the definition it sits in.

The cursor in the list and the hunk on screen are the same cursor, so `j`/`k`
move both and every assigning key works the same in either view. `Enter`, `f` or
`Esc` goes back to reading whichever hunk the cursor was left on.

The enclosing definition is worked out from the file rather than read out of the
diff, because `jj diff --git` omits it where `git diff` includes it. It uses
git's own language-agnostic rule, the nearest line starting in the first column
— so in prose files it reports whatever line happens to sit above, which is
noise rather than a signal.

`Enter` hands the keys back to the form, which then describes one commit at a
time: write the message for commit 1 and press `Ctrl-s`, and jj-compose makes
it, reopens on what is left with an empty form, and moves on to commit 2. The
status bar says which one you are on, `Describing commit 2 of 3: 2 hunks`, and
so does the form's key hint.

Boxes survive being committed even though the hunks' positions do not: the plan
is keyed by each hunk's text, because committing part of a change shifts every
later hunk's line numbers. Whatever you left unassigned stays behind in the
final change, which keeps the original description and which you describe last.

While a split is pending the form's status bar says so. `^s commit 2 hunks`
rather than `^s save`, and `Alt-←` goes back to the hunks to change the marks.

This needs no configuration. jj-compose runs itself as jj's diff editor, passing
the tool on `jj split`'s own command line, so `ui.diff-editor` is neither read
nor needed: a `jj split --tool` setup of your own is left alone and goes on
working from the command line as before.

The description carries over to the remainder, as it does with `jj split`, so
the form starts out holding the original message. Clear the parts that belong to
the piece you just committed.

Putting every hunk in one box leaves nothing behind, so that is a description
rather than a split and `Ctrl-s` behaves exactly as it does outside split mode.

Hunks are the smallest unit: there is no splitting a hunk by line, and a binary
file has no hunks at all, so its change always stays with the remainder. A
change with only one hunk can't be split, which is also why the last box needs
no split of its own — once everything else is committed, what is left _is_ that
commit.

## Rewriting released history

<!--prettier-ignore-->
> [!WARNING]
> **Do not rewrite commits that others rely on.** Since jj marks pushed commits
> as immutable, this flag exists to let you rewrite them. Only do this if you
> are absolutely certain no one else has pulled these commits, and be prepared
> to forcefully push the rewritten tags.

By default a revision jj protects as immutable: a tagged release, anything on
trunk, is refused before the TUI opens. `--ignore-immutable` passes jj's own
flag of that name through to every command, so such a revision can be split like
any other.

Two things happen when you do, and jj-compose can reverse **neither**:

- **Every tag on that revision or after it is stranded**, left pointing at a
  commit that is now hidden. Splitting is worse than describing here: one commit
  becomes two, so there is no mechanical answer to where the tag belongs.
  jj-compose suggests the remainder, since that is the commit holding the whole
  of the original content. Tags on _ancestors_ are untouched.
- **Every later commit gets a new hash**, because jj rebases the descendants.

So the mode is loud about it. The pane title carries `· immutable` for the whole
session, the status bar names the tags at risk before any work is done, and on
exit jj-compose prints what to run:

```text
Rewriting immutable history left these tags behind:

  jj tag set v0.1.0 --allow-move -r oxuonltl    # was 0688276c (feat: three things at once)
  git push origin v0.1.0 --force
```

The `git push` is there because `jj git push` cannot force-push a tag; that one
step has to go through git.

Think twice before using this on anything already published. If a release is on
crates.io, rewriting its commit makes your history disagree with the tarball,
and you then maintain that disagreement through force-pushed tags and a
regenerated changelog. The honest use is a release made minutes ago that nobody
has pulled, and for that, `jj op restore` is usually cleaner still, since it
rolls the tag back too instead of leaving you to re-anchor it.

## External diff tools

`o` opens the file at the top of the pane in another program; `O` opens the
whole change. Both run `jj diff --tool`, so the tool is whatever `--diff-tool`
says, and otherwise jj's own `ui.diff-editor`: a working `jj split` setup needs
no extra configuration here. The status bar names the tool that `o` will use.

`jj diff --tool` takes its arguments from `diff-args`, while `jj split` uses
`edit-args`, so a tool configured only for splitting needs the one line:

```toml
# ~/.config/jj/config.toml
[ui]
diff-editor = "meld"

[merge-tools.meld]
program = "meld"
edit-args = ["--newtab", "$left", "$right"]
diff-args = ["--newtab", "$left", "$right"]   # for `o`/`O`
```

A GUI tool returns to the form as soon as you close its window. A terminal tool
such as `difft` is paged by jj, so quit the pager (`q`) to come back. With no
tool configured at all, `o` says so rather than opening jj's internal diff,
which is the one already on screen.

`ui.diff-editor` is sometimes set to something that only makes sense for
splitting, such as a script wrapping `git add -p`. That isn't a viewer, so `o`
would pick it up and do nothing useful. Set `--diff-tool` or
`JJ_COMPOSE_DIFF_TOOL` to a real diff viewer in that case.

## License

[Apache-2.0](https://github.com/saylesss88/jj-compose/blob/main/LICENSE)
