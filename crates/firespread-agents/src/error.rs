use std::path::PathBuf;
use std::time::Duration;

use firespread_ensemble::EnsembleError;

/// Errors raised by the forecast runtime.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AgentError {
    /// No mailbox carries the requested name.
    #[error("unknown agent `{name}`")]
    UnknownAgent { name: String },
    /// A message could not be delivered to an agent.
    #[error("agent `{name}` did not accept message `{message}`")]
    Undeliverable { name: String, message: String },
    /// A mailbox of this name is already registered on the bus.
    #[error("agent `{name}` is already registered on the bus")]
    DuplicateAgent { name: String },
    /// An agent did not signal readiness within `timeout`.
    #[error("agent `{agent}` did not signal ready within {timeout:?}")]
    ReadyTimeout { agent: String, timeout: Duration },
    /// An agent's task ended before it signalled readiness.
    #[error("agent `{agent}` dropped before its ready signal")]
    ListenLoopDropped { agent: String },
    /// The compute pool could not be built.
    #[error("building a compute pool of {threads} threads")]
    ExecutorBuild {
        threads: usize,
        #[source]
        source: tokio_rayon::rayon::ThreadPoolBuildError,
    },
    /// An executor setting is out of range.
    #[error("executor {field} is {value}, expected a value >= 1")]
    InvalidExecutor { field: &'static str, value: usize },
    /// The executor's admission semaphore is closed.
    #[error("executor admission closed during {stage}")]
    AdmissionClosed { stage: &'static str },
    /// A job on the compute pool panicked.
    #[error("{stage} panicked: {message}")]
    Panicked {
        stage: &'static str,
        message: String,
    },
    /// One ensemble member returned an error.
    #[error("ensemble member {member} failed")]
    MemberFailed {
        member: u32,
        #[source]
        source: EnsembleError,
    },
    /// One ensemble member panicked.
    #[error("ensemble member {member} panicked: {message}")]
    MemberPanicked { member: u32, message: String },
    /// An engine call failed.
    #[error("{stage} failed")]
    Ensemble {
        stage: &'static str,
        #[source]
        source: EnsembleError,
    },
    /// A raster or manifest read or write failed.
    #[error("{stage} failed")]
    Grid {
        stage: &'static str,
        #[source]
        source: firespread_grid::GridError,
    },
    /// A directory could not be created.
    #[error("creating {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The weather of the window ended before every hour arrived.
    #[error("weather ended after {got} of {expected} hours")]
    WeatherIncomplete { expected: usize, got: usize },
    /// A weather message arrived out of protocol.
    #[error("weather protocol: {reason}")]
    WeatherProtocol { reason: String },
    /// Every member failed or none ran, so there is no burn probability.
    #[error("no member of {requested} completed")]
    NoCompletedMembers { requested: u32 },
}

pub type Result<T> = core::result::Result<T, AgentError>;
