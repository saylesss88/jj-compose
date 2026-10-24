# jj-compose

A terminal UI for writing and revising
[Conventional Commit](https://www.conventionalcommits.org/) descriptions in
[jj](https://jj-vcs.github.io/jj/), with the change's diff alongside.

```text
┌ Diff kxqvmzpo ──────────────────┐┌ Type ──────┐┌ Scope ──────────────┐
│Modified regular file src/app.rs:││fix         ││parser               │
│  41   41:     let len = ...     │└────────────┘└─────────────────────┘
│  42     : -   if len == 0 {     │┌ Subject 31/72 ────────────────────┐
│       42: +   if len < 2 {      ││handle empty input                 │
│                                 │└───────────────────────────────────┘
│                                 │┌ Body ─────────────────────────────┐
│                                 ││This also corrects a diagnostic.   │
│                                 │└───────────────────────────────────┘
│                                 │┌ Problem ──────────────────────────┐
│                                 ││What is wrong or missing?          │
│                                 │└───────────────────────────────────┘
│                                 │┌ Solution ─────────────────────────┐
│                                 ││How does this change address it?   │
│                                 │└───────────────────────────────────┘
│                                 │┌ Trailers ─────────────────────────┐
│                                 ││Fixes #42                          │
└─────────────────────────────────┘└───────────────────────────────────┘
 Tab next · Enter next/newline · PgUp/PgDn diff · ^t raw/form · ^s save · Esc quit
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

## Usage

```sh
jj-compose              # describe @
jj-compose -r @-        # revise the parent's description
jj-compose <revset>     # any single mutable revision
```

| Option                   | Effect                                                        |
| ------------------------ | ------------------------------------------------------------- |
| `-r, --revision <REVSET>` | Revision to describe (or pass it positionally). Default `@`. |
| `-R, --repository <PATH>` | Repository to operate on, as with `jj -R`.                   |
| `--git`                  | Show the diff in git format instead of jj's configured one.   |
| `--keymap <default\|vim>` | Key bindings. Also set by `JJ_COMPOSE_KEYMAP`.               |
| `--raw`                  | Start in the raw text editor instead of the form.             |

The revset must resolve to exactly one mutable revision. Saving runs
`jj describe` on that exact commit; saving a message identical to the
current one does nothing.

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

Only the header is required. A `!` at the end of the Type field marks a
breaking change and is written after the scope. Trailers are the final
paragraph of `Token: value` or `Token #value` lines (`BREAKING CHANGE:`
included); they are checked when you save.

The Subject title shows the header's length: green up to 50 characters,
yellow up to 72, red beyond.

### Revising existing descriptions

On launch the revision's current description is loaded. It opens in the form
**only if the form can reproduce it byte for byte**; otherwise it opens in the
raw editor, unchanged, and the status bar says why (for example, the first
line isn't `type(scope): subject`). Opening and saving a message never
reshapes it behind your back.

`Ctrl-t` switches between the form and the raw text at any time. Switching to
the form from raw text follows the same rule, so it's refused (with the
reason) until the text fits. Saving from the raw view writes the text exactly
as typed, without Conventional Commit validation.

## Keys

### Everywhere

| Key                  | Action                                 |
| -------------------- | -------------------------------------- |
| `Ctrl-s`             | Save and run `jj describe`             |
| `Ctrl-c`             | Quit (asks again if there are edits)   |
| `Ctrl-t`             | Switch between form and raw text       |
| `Tab` / `Shift-Tab`  | Next / previous field, then the diff   |
| `PgDn` / `PgUp`      | Scroll the diff half a page            |
| `Alt-↓` / `Alt-↑`    | Scroll the diff one line               |

### Default keymap

The form opens ready to type in Type.

| Key                         | Action                                        |
| --------------------------- | --------------------------------------------- |
| `Enter`                     | Next field (one-line fields) or newline       |
| `↑` / `↓` at a field's edge | Previous / next field                         |
| `Esc`                       | Quit (asks again if there are edits)          |
| `Shift`+arrows              | Select                                        |
| `Ctrl-x` / `Alt-w` / `Ctrl-y` | Cut / copy / paste (internal clipboard)     |
| `Ctrl-z` or `Ctrl-u` / `Ctrl-r` | Undo / redo                               |
| `Ctrl-←` / `Ctrl-→`, `Alt-b` / `Alt-f` | Word left / right                  |
| `Ctrl-w`, `Alt-Backspace` / `Alt-d` | Delete word before / after            |
| `Home` / `End`, `Ctrl-a` / `Ctrl-e` | Line start / end                      |
| `Ctrl-k` / `Ctrl-j`         | Delete to end / start of line                 |

Pasting from the terminal inserts text as-is; in one-line fields newlines
become spaces.

### Vim keymap

`--keymap vim` or `JJ_COMPOSE_KEYMAP=vim`. Starts in Insert mode for a new
description, Normal mode when revising one. Insert mode uses the default
keymap's editing keys.

| Key                               | Action                                     |
| --------------------------------- | ------------------------------------------ |
| `h` `j` `k` `l`, `w` `b` `e`, `0` `^` `$`, `gg` `G` | Motions                |
| `j` / `k` on a field's last / first line, `Enter` | Next / previous field      |
| `i` `a` `I` `A` `o` `O`            | Insert                                     |
| `x` `D` `C` `S` `J`                | Edit                                       |
| `d` `c` `y` + motion, `dd` `cc` `yy`, `p` `P` | Operators and paste             |
| `v` / `V`                          | Visual / visual line                       |
| `u` / `Ctrl-r`                     | Undo / redo                                |
| `Ctrl-d` / `Ctrl-u`, `Ctrl-e` / `Ctrl-y` | Scroll the diff                      |
| `q`                                | Quit (asks again if there are edits)       |

### Diff pane

| Key                         | Action             |
| --------------------------- | ------------------ |
| `j` / `k`, `↓` / `↑`        | Scroll a line      |
| `Ctrl-d` / `Ctrl-u`, `Space` | Half page         |
| `g` / `G`, `Home` / `End`   | Top / bottom       |
| `h` / `l`, `←` / `→`        | Pan sideways       |
| `Esc`, `Enter`, `i`         | Back to the editor |

On terminals narrower than 100 columns the diff sits above the form.

## License

Apache-2.0
