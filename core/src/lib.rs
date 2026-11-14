//! A TUI for writing and revising Conventional Commit descriptions of
//! changes in a `jj` repository, with the change's diff alongside.
//!
//! See `message` for the message format and how existing descriptions are
//! parsed without being rewritten.

pub mod error;
pub mod jj;
pub mod marks;
pub mod message;
pub mod patch;
pub mod split;

pub use error::{Error, Result};
pub use jj::{Jj, Target};
pub use marks::Split;
pub use message::{Draft, Field, HEADER_HARD_LIMIT, HEADER_SOFT_LIMIT, ParseError};
pub use patch::{FileDiff, Hunk, HunkId, Patch};
