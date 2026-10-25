#![forbid(unsafe_code)]
//! Ensembles of spread runs over perturbed inputs.

/// Errors raised while building or running an ensemble.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnsembleError {
    /// The ensemble has no members.
    #[error("ensemble has no members")]
    Empty,
    /// A member failed; `member` is its zero-based index.
    #[error("ensemble member {member} of {members} failed: {reason}")]
    MemberFailed {
        member: usize,
        members: usize,
        reason: String,
    },
}

pub type Result<T> = core::result::Result<T, EnsembleError>;
