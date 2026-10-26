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
    /// A canopy or weather value at a cell is out of range.
    #[error("{layer} cell {index} holds {value}, expected {bound}")]
    InvalidCell {
        layer: &'static str,
        bound: &'static str,
        index: usize,
        value: f64,
    },
    /// A canopy or fuel input of the wind adjustment factor is out of range.
    #[error("wind adjustment input {field} is {value}, expected {bound}")]
    InvalidCanopy {
        field: &'static str,
        bound: &'static str,
        value: f64,
    },
    /// A weather input of the dead fuel moisture model is out of range.
    #[error("dead fuel moisture input {field} is {value}, expected {bound}")]
    InvalidWeather {
        field: &'static str,
        bound: &'static str,
        value: f64,
    },
    /// The sky-cover edges are not finite and strictly increasing.
    #[error("sky cover edges {edges:?} are not finite and strictly increasing")]
    InvalidSkyEdges { edges: [f64; 3] },
    /// A spread multiplier is negative or not finite.
    #[error("spread multiplier of {model} is {value}, expected a finite value >= 0")]
    InvalidSpreadMultiplier { model: &'static str, value: f64 },
    /// A field of the night moisture setting is out of range.
    #[error("night moisture {field} is {value}, expected {bound}")]
    InvalidNightMoisture {
        field: &'static str,
        bound: &'static str,
        value: f64,
    },
    /// An hour label is not `YYYY-MM-DDTHH:MM:SSZ`.
    #[error("hour {index} label {label:?} is not YYYY-MM-DDTHH:MM:SSZ")]
    InvalidHourLabel { index: usize, label: String },
    /// The weather does not hold one UTC label per hour.
    #[error("weather holds {labels} hour labels for {hours} hours")]
    HourLabels { hours: usize, labels: usize },
    /// The weather index could not be read or parsed.
    #[error("weather index {path}: {reason}")]
    WeatherIndex {
        path: std::path::PathBuf,
        reason: String,
    },
    /// The weather index names a unit other than the one the loader reads.
    #[error("weather index unit of {field} is {found:?}, expected {expected:?}")]
    WeatherUnit {
        field: &'static str,
        found: Option<String>,
        expected: &'static str,
    },
    /// The weather index has no window of this name.
    #[error("weather index has no window {window:?}")]
    MissingWindow { window: String },
    /// The weather index lacks a file of a window or hour.
    #[error("weather window {window:?} lacks {layer} for {at}")]
    MissingWeatherFile {
        window: String,
        layer: &'static str,
        at: String,
    },
    /// A window holds fewer hours of weather than a run needs.
    #[error("weather holds {available} hours, fewer than the {horizon_h} h asked for")]
    TooFewHours { available: usize, horizon_h: u32 },
    /// A weather raster's geometry differs from the fuel grid's.
    #[error("weather raster {path} is not on the FBFM40 grid")]
    WeatherGrid { path: std::path::PathBuf },
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
