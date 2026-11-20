//! Core library of `jj-compose`: a `jj` wrapper, the Conventional Commit
//! message model, and splitting a change into hunks.
//!
//! See [`message`] for the message format and how existing descriptions are
//! parsed without being rewritten, [`patch`] and [`marks`] for choosing part
//! of a change, and [`split`] for writing that choice back out.
/// Compiles the examples in `README.md` as doctests, so they can't drift
/// from the API. Not part of the public surface.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct Readme;

pub mod error;
pub mod jj;
pub mod marks;
pub mod message;
pub mod overview;
pub mod patch;
pub mod plan;
pub mod split;
pub mod symbols;

pub use error::{Error, Result};
pub use jj::{Jj, TaggedCommit, Target};
pub use marks::Split;
pub use message::{Draft, Field, HEADER_HARD_LIMIT, HEADER_SOFT_LIMIT, ParseError};
pub use patch::{FileDiff, Hunk, HunkId, Patch, Take};
pub use plan::Plan;
