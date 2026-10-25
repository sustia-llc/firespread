#![forbid(unsafe_code)]
//! Agent coalitions that steer and interpret spread runs.

/// Errors raised by agent coordination.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AgentError {
    /// No agent carries the requested name.
    #[error("unknown agent `{name}`")]
    UnknownAgent { name: String },
    /// A message could not be delivered to an agent.
    #[error("agent `{name}` did not accept message `{message}`")]
    Undeliverable { name: String, message: String },
}

pub type Result<T> = core::result::Result<T, AgentError>;
