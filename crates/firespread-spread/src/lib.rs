#![forbid(unsafe_code)]
//! Fire front propagation over a grid driven by the Rothermel surface model.

/// Errors raised while propagating a fire front.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SpreadError {
    /// The time step is zero, negative or not finite.
    #[error("time step must be positive and finite, got {step_s} s")]
    InvalidTimeStep { step_s: f64 },
    /// An ignition cell lies outside the grid.
    #[error("ignition cell ({row}, {col}) is outside a {rows}x{cols} grid")]
    IgnitionOutOfBounds {
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    },
}

pub type Result<T> = core::result::Result<T, SpreadError>;
