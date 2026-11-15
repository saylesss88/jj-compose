//! Core library of `jj-compose`: a `jj` wrapper, the Conventional Commit
//! message model, and splitting a change into hunks.
//!
//! See [`message`] for the message format and how existing descriptions are
//! parsed without being rewritten, [`patch`] and [`marks`] for choosing part
//! of a change, and [`split`] for writing that choice back out.

pub mod error;
pub mod jj;
pub mod marks;
pub mod message;
pub mod overview;
pub mod patch;
pub mod split;
pub mod symbols;

pub use error::{Error, Result};
pub use jj::{Jj, TaggedCommit, Target};
pub use marks::Split;
pub use message::{Draft, Field, HEADER_HARD_LIMIT, HEADER_SOFT_LIMIT, ParseError};
pub use patch::{FileDiff, Hunk, HunkId, Patch};
