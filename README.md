# jj-compose

A terminal UI for writing and revising
[Conventional Commit](https://www.conventionalcommits.org/) descriptions in
[jj](https://jj-vcs.github.io/jj/), with the change's diff alongside.

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
- **Splitting:** `s` (or `Alt-s` from the form) walks the change one hunk at a
  time, so each piece can be committed with its own message. See
  [Splitting a change](#splitting-a-change).
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
diff pane) walks the change one hunk at a time, filling the pane with just that
hunk and asking whether it belongs in the commit you are about to write:

| Key                            | Action                                     |
| ------------------------------ | ------------------------------------------ |
| `y`, `Space`                   | Add this hunk to the commit                |
| `n`                            | Leave it for what remains of the change    |
| `a`                            | Add this hunk and the rest of the file     |
| `d`                            | Skip this hunk and the rest of the file    |
| `u`, `Backspace`               | Back up and decide the previous hunk again |
| `Enter`                        | Stop here and describe what has been added |
| `Esc`, `q`                     | Leave split mode, deciding nothing         |
| `j` / `k`, `Ctrl-d` / `Ctrl-u` | Scroll inside a hunk taller than the pane  |

The pane's title tracks where you are: `hunk 3/7 · src/app.rs · 2 added`.

Once every hunk is decided the form takes focus again. Write the message for the
hunks you added and press `Ctrl-s`: those hunks become their own commit, and
jj-compose reopens on what is left of the change, ready for the next message.
Repeat until nothing is left.

This needs no configuration. jj-compose runs itself as jj's diff editor, passing
the tool on `jj split`'s own command line, so `ui.diff-editor` is neither read
nor needed: a `jj split --tool` setup of your own is left alone and goes on
working from the command line as before.

The description carries over to the remainder, as it does with `jj split`, so
the form starts out holding the original message. Clear the parts that belong to
the piece you just committed.

Adding every hunk is a description rather than a split, so `Ctrl-s` then behaves
exactly as it does outside split mode.

Hunks are the smallest unit: there is no splitting a hunk by line, and a binary
file has no hunks at all, so its change always stays with the remainder. A
change with only one hunk can't be split.

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

Apache-2.0
