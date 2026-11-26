# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-11-25

### 🛠️ Fixed

- cargo-binstall metadata

### 📚 Documentation

- increase leading png size
- slim down README

## [0.2.0] - 2026-11-25

### Added

- **(split)** select a run of lines in a divided hunk
- **(overview)** mark a divided hunk in the list
- **(plan)** carry a division across a split
- **(split)** divide a hunk by line in the TUI
- **(marks)** divide a hunk's lines between commits
- **(patch)** take a hunk by line, not just whole
- **(ui)** add color-coded buckets to make it more clear which hunk is going to which commit
- **(vim)** add visual select mode for the `f` board

### Fixed

- stop excluding tests
- screenshot size
- syntax error in README path
- line-level splitting
- set rust-version in manifests and remove nightly method calls
- clippy lints
- final fixes
- **(examples)** update examples to use new syntax
- **(marks)** let a single hunk be split when it holds two changes
- **(patch)** prevent integer underflow panic on malformed diff headers

### Documentation

- add a few more screenshots
- add screenshot to README
- update README
- update documentation for both cli and core
- **(core)** add link to examples
- generate per-member changelogs with `jj-release`
- add quick start to README

## [0.1.0] - 2026-11-15

### Added

- add commit boxes feature, press 1 to have the hunk go to commit #1, etc.
- add release.yml to release binaries for cargo-binstall
- add board overview of hunks/changes with `f`
- implement ability to break up an immutable change
- scrollable hunks
- **(app)** attempt to change ui for adding hunks
- **(core)** add custom Error enum with thiserror
- tie in split and patch code to the binary
- tie in patch and split code
- add Patch struct, a diff's text together with its parse
- add split method to split big working copy changes
- **(app)** add `Split` struct and methods to make hunk splitting possible
- **(app)** add custom diff tool support
- **(jj)** read a config value
- **(diff)** zoom the diff to the full window with z
- **(diff)** search the diff with / and n/N
- **(diff)** jump between files and hunks, and show a file list
- **(keys)** navigate the form by its on-screen layout
- **(keys)** move between the form and the diff with h/l and Alt-arrows
- edit existing descriptions, with a raw view as fallback
- **(message)** add Body and Trailers fields and a lossless parser
- **(keys)** add an opt-in Vim keymap
- **BREAKING** **(editor)** replace append-only input with ratatui-textarea
- **(cli)** describe any revision with -r/--revision and -R/--repository
- **(diff)** render jj's own colored diff output
- **(diff)** collor added and removed lines
- **(input)** add Tab navigation, Enter-to-advance and diff paging
- **(ui)** show a header length counter on the Subject box
- support the breaking-change marker
- validate the commit header before describing
- **(ui)** show the mode, key hints and a cursor while typing
- **(input)** implement the commit functionality
- **(input)** route typing to the active text buffer in insert mode
- implement `ui` function
- **(input)** add app keybinds for insert/normal mode and movement
- **(core)** set up the terminal initialization and event loop
- layout desired application state
- initialize project

### Changed

- **(cli)** slim down `app.rs`
- move Split to the lib
- **BREAKING** create core and cli workspaces
- **BREAKING** **(diff)** parse the diff into files and hunks
- split message model, diff loading and app into modules
- move event loop and drawing into App methods
- model form fields as a Field enum
- extract commit message formatting into a function

### Performance

- **(diff)** draw only the visible lines and add horizontal panning

### Fixed

- typo
- workflow, enable manual runs
- metadata
- metadata
- re-exports
- **(app)** add is_vertical field and check
- use statements and re-exports
- **(keys)** reach the diff with k when it sits above the form
- use statements
- **(jj)** describe the change, not the commit it was at on launch
- **(cli)** print errors as plain text
- **(diff)** keep tab indentation and let jj color --git diffs
- **(message)** reject a type with more than one trailing '!'
- **(diff)** stop scrolling past the end of the diff
- **(input)** don't type characters for Ctrl key combos
- **(ui)** keep typed text readable in unfocused boxes
- exit before starting the TUI when `jj diff` fails
- run jj describe after terminal is restored
- remove crossterm dep and use ratatui to accomplish the same thing

### Bug

- **(app)** adding hunks, a message, and saving left app in a broken state
- add crossterm::KeyEventKind and ignore key release events

### Documentation

- **(core)** add an example to split the first hunk of a revision off into its own commit
- **(core)** add an overview example
- **(core)** update core README
- add link to LICENSE file
- **(core)** add include_str! for the README in lib.rs
- **(core)** add a README to the lib
- add warning to README about rewriting immutable history
- update description and add a features section
- fix doc comments
- update README
- README edits
- update README
- update README
- update README
- describe the tool in the crate docs
- describe the message format in the crate docs
