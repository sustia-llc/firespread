#![forbid(unsafe_code)]
//! Scores of simulated fire perimeters against observed ones.

/// Errors raised while scoring perimeters.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MetricsError {
    /// The simulated and observed rasters differ in shape.
    #[error(
        "shape mismatch: simulated {simulated_rows}x{simulated_cols}, observed {observed_rows}x{observed_cols}"
    )]
    ShapeMismatch {
        simulated_rows: usize,
        simulated_cols: usize,
        observed_rows: usize,
        observed_cols: usize,
    },
    /// Both perimeters are empty, so the score is undefined.
    #[error("{metric} is undefined when both perimeters are empty")]
    EmptyPerimeters { metric: &'static str },
}

pub type Result<T> = core::result::Result<T, MetricsError>;
