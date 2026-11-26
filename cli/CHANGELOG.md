# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-11-25

### 🛠️ Fixed

- cargo-binstall metadata

## [0.2.0] - 2026-11-25

### Added

- **(split)** select a run of lines in a divided hunk
- **(split)** divide a hunk by line in the TUI
- **(ui)** add color-coded buckets to make it more clear which hunk is going to which commit
- **(vim)** add visual select mode for the `f` board

### Fixed

- line-level splitting
- set rust-version in manifests and remove nightly method calls
- clippy lints
- final fixes

### Documentation

- generate per-member changelogs with `jj-release`

## [0.1.0] - 2026-11-15

### Added

- add commit boxes feature, press 1 to have the hunk go to commit #1, etc.
- add release.yml to release binaries for cargo-binstall
- add board overview of hunks/changes with `f`
- implement ability to break up an immutable change
- scrollable hunks
- **(app)** attempt to change ui for adding hunks

### Changed

- **(cli)** slim down `app.rs`
- move Split to the lib
- **BREAKING** create core and cli workspaces

### Fixed

- metadata
- metadata
- **(app)** add is_vertical field and check
- use statements and re-exports

### Bug

- **(app)** adding hunks, a message, and saving left app in a broken state
