use std::string::FromUtf8Error;
use thiserror::Error;

/// The central error type for the jj-compose-core library.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum Error {
    // The `#[from]` attribute automatically converts standard I/O errors into this variant.
    // This lets you use `?` directly on things like `Command::output()`.
    #[error("I/O error during execution")]
    Io(#[from] std::io::Error),

    // Automatically converts UTF-8 parsing errors.
    #[error("Output from jj command contained invalid UTF-8")]
    Utf8(#[from] FromUtf8Error),

    // A structured error for when `jj` runs but exits with a failure code.
    #[error("`jj {args}` failed: {stderr}")]
    CommandFailed { args: String, stderr: String },

    // Specific domain errors for your application logic
    #[error("Unexpected output from jj log: {0}")]
    UnexpectedOutput(String),

    #[error("Revset `{0}` must resolve to exactly one revision")]
    AmbiguousRevset(String),

    #[error("Change {0} is immutable; jj describe will refuse to rewrite it")]
    ImmutableRevision(String),

    #[error("Failed to open standard input for the jj process")]
    StdinUnavailable,

    #[error("Failed to parse commit message")]
    Parse(#[from] crate::message::ParseError),
}

/// A convenient Result alias specific to this library.
pub type Result<T, E = Error> = std::result::Result<T, E>;
