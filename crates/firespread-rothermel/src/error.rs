/// Errors raised by the spread model.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RothermelError {
    /// A fuel bed or environment scalar is outside its admissible range.
    #[error("{field} must be finite and {bound}, got {value}")]
    InvalidInput {
        field: &'static str,
        bound: &'static str,
        value: f64,
    },
    /// A field of the class at `index` is outside its admissible range.
    #[error("class {index}: {field} must be finite and {bound}, got {value}")]
    InvalidClass {
        index: usize,
        field: &'static str,
        bound: &'static str,
        value: f64,
    },
    /// Every class carries zero load, so the weighting factors are undefined.
    #[error("fuel bed has no load over {classes} classes")]
    NoFuel { classes: usize },
    /// The heat sink is not positive, so the spread rate is undefined.
    #[error("heat sink is {heat_sink_btu_ft3} Btu/ft3; spread rate undefined")]
    ZeroHeatSink { heat_sink_btu_ft3: f64 },
}

pub type Result<T> = core::result::Result<T, RothermelError>;
