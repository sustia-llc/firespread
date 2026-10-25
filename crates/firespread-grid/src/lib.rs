#![forbid(unsafe_code)]
//! Raster grid of fuel, terrain and weather cells for the spread engine.

/// Errors raised by grid construction and indexing.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GridError {
    /// A cell index lies outside the grid.
    #[error("cell ({row}, {col}) is outside a {rows}x{cols} grid")]
    OutOfBounds {
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    },
    /// A grid dimension or cell size is zero or not finite.
    #[error("grid {field} must be positive, got {value}")]
    InvalidDimension { field: &'static str, value: f64 },
}

pub type Result<T> = core::result::Result<T, GridError>;
