/// Errors raised while propagating a fire front.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SpreadError {
    /// An ignition cell lies outside the grid.
    #[error("ignition cell ({row}, {col}) is outside a {rows}x{cols} grid")]
    IgnitionOutOfBounds {
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    },
    /// The grid has no cells.
    #[error("grid must have at least one cell, got {nx}x{ny}")]
    EmptyGrid { nx: usize, ny: usize },
    /// The grid has more cells than a `u32` index addresses.
    #[error("grid {nx}x{ny} exceeds {max} cells")]
    GridTooLarge { nx: usize, ny: usize, max: u64 },
    /// The cell size is zero, negative or not finite.
    #[error("cell size must be positive and finite, got {dx_m} m")]
    InvalidCellSize { dx_m: f64 },
    /// A per-cell slice does not hold one value per grid cell.
    #[error("{field} has {got} values, expected {expected}")]
    FieldLength {
        field: &'static str,
        expected: usize,
        got: usize,
    },
    /// The window start is not finite or the window end precedes it.
    #[error("window must satisfy finite start <= end, got [{t_start_s}, {t_end_s}] s")]
    InvalidWindow { t_start_s: f64, t_end_s: f64 },
    /// An ignition time is not finite.
    #[error("ignition time must be finite, got {t_s} s")]
    InvalidIgnitionTime { t_s: f64 },
    /// The arrival field holds a NaN.
    #[error("arrival time at cell {index} is NaN")]
    NanArrival { index: usize },
}

pub type Result<T> = core::result::Result<T, SpreadError>;
