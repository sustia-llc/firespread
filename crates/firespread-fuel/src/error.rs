/// Errors raised while parsing the fuel model table or building fuel classes.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FuelError {
    /// The embedded CSV table failed to parse.
    #[error("fuel model table: {source}")]
    Csv {
        #[from]
        source: csv::Error,
    },
    /// A `fuel_model_type` cell was neither `dynamic` nor `N/A`.
    #[error(
        "fuel model table row {row}: unknown fuel_model_type `{value}` (expected `dynamic` or `N/A`)"
    )]
    UnknownModelType { row: usize, value: String },
    /// No row carries the requested fuel model code.
    #[error("unknown fuel model code `{code}`")]
    UnknownCode { code: String },
    /// A class carries load but its surface-area-to-volume ratio is absent.
    #[error(
        "fuel model `{code}`: {class} load {load_kg_m2} kg/m2 has no surface-area-to-volume ratio"
    )]
    MissingSav {
        code: String,
        class: &'static str,
        load_kg_m2: f64,
    },
    /// A table cell is outside its admissible range.
    #[error("fuel model `{code}`: {field} must be finite and {bound}, got {value}")]
    InvalidValue {
        code: String,
        field: &'static str,
        bound: &'static str,
        value: f64,
    },
    /// A moisture fraction is not finite or is negative.
    #[error("moisture {field} must be finite and non-negative, got {value}")]
    InvalidMoisture { field: &'static str, value: f64 },
}

pub type Result<T> = core::result::Result<T, FuelError>;
