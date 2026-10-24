//! A TUI for writing and revising Conventional Commit descriptions of
//! changes in a `jj` repository, with the change's diff alongside.
//!
//! See `message` for the message format and how existing descriptions are
//! parsed without being rewritten.

pub mod app;
pub mod diff;
pub mod jj;
pub mod message;
pub mod patch;
pub mod vim;
