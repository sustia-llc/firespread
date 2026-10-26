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
    /// More members than a `u16` burn count can hold.
    #[error("{members} members exceed the maximum of {max}")]
    TooManyMembers { members: u32, max: u32 },
    /// No forecast horizon was given.
    #[error("no forecast horizons")]
    NoHorizons,
    /// A forecast horizon is negative or not finite.
    #[error("horizon {index} is {value} s, expected a finite value >= 0")]
    InvalidHorizon { index: usize, value: f32 },
    /// A perturbation parameter is out of range.
    #[error("perturbation {field} is {value}, expected {bound}")]
    InvalidPerturbation {
        field: &'static str,
        bound: &'static str,
        value: f64,
    },
    /// A layer does not hold one value per cell.
    #[error("layer {layer} has {got} cells, expected {expected}")]
    LayerLength {
        layer: &'static str,
        expected: usize,
        got: usize,
    },
    /// A layer's geometry differs from the FBFM40 layer's.
    #[error("layer {layer} is not on the FBFM40 grid")]
    LayerMismatch { layer: &'static str },
    /// The cells are not square.
    #[error("cells are {dx} x {dy}, expected square cells")]
    NonSquareCells { dx: f64, dy: f64 },
    /// An FBFM40 cell holds a code that is neither one of the 40 Scott and
    /// Burgan models nor non-burnable (91-99).
    #[error("FBFM40 cell {index} holds unknown code {code}")]
    UnknownFuelCode { index: usize, code: f32 },
    /// A slope or aspect value at a burnable cell is out of range.
    #[error("{layer} cell {index} holds {value}, expected {bound}")]
    InvalidTerrain {
        layer: &'static str,
        bound: &'static str,
        index: usize,
        value: f32,
    },
    /// A thread panicked while holding the member buffer pool.
    #[error("member buffer pool poisoned by a panicked thread")]
    BufferPoolPoisoned,
    #[error(transparent)]
    Fuel(#[from] firespread_fuel::FuelError),
    #[error(transparent)]
    Grid(#[from] firespread_grid::GridError),
    #[error(transparent)]
    Rothermel(#[from] firespread_rothermel::RothermelError),
    #[error(transparent)]
    Spread(#[from] firespread_spread::SpreadError),
}

pub type Result<T> = core::result::Result<T, EnsembleError>;
