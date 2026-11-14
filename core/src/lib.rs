//! A TUI for writing and revising Conventional Commit descriptions of
//! changes in a `jj` repository, with the change's diff alongside.
//!
//! See `message` for the message format and how existing descriptions are
//! parsed without being rewritten.

pub mod error;
pub mod jj;
pub mod message;
pub mod patch;
pub mod split;

pub use error::{Error, Result};
pub use jj::Jj;
pub use message::{Field, ParseError};
pub use patch::{FileDiff, Hunk, HunkId, Patch};
