#![forbid(unsafe_code)]
//! Command-line entry point for the firespread engine.

/// Errors raised by the command-line entry point.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    /// A command-line argument is missing or malformed.
    #[error("argument `{name}`: {reason}")]
    Argument { name: String, reason: String },
    /// A run directory could not be created or written.
    #[error("run directory `{path}`: {source}")]
    RunDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = core::result::Result<T, CliError>;

fn main() {}
