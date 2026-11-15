# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-11-15

### Added

- add commit boxes feature, press 1 to have the hunk go to commit #1, etc.
- add board overview of hunks/changes with `f`
- implement ability to break up an immutable change
- scrollable hunks
- **(core)** add custom Error enum with thiserror

### Changed

- move Split to the lib
- **BREAKING** create core and cli workspaces

### Fixed

- metadata
- metadata
- re-exports
- use statements and re-exports

### Documentation

- **(core)** add an example to split the first hunk of a revision off into its own commit
- **(core)** add an overview example
- **(core)** update core README
- add link to LICENSE file
- **(core)** add include_str! for the README in lib.rs
- **(core)** add a README to the lib
- fix doc comments
