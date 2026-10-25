#![forbid(unsafe_code)]
//! Scott and Burgan (2005) standard fire behavior fuel models: the 40-model
//! table in the paper's units, SI conversion, and dynamic live-herbaceous
//! load transfer into fuel classes.

mod error;
mod model;
mod transfer;
pub mod units;

pub use error::{FuelError, Result};
pub use model::{
    EFFECTIVE_MINERAL_CONTENT, FuelModel, FuelModelSi, PARTICLE_DENSITY_LB_PER_FT3, SAV_10H_PER_FT,
    SAV_100H_PER_FT, TOTAL_MINERAL_CONTENT, by_code, scott_burgan_40,
};
pub use transfer::{
    Category, ClassKind, FuelClass, Moistures, TRANSFER_MOISTURE_ALL, TRANSFER_MOISTURE_NONE,
    fuel_classes, transfer_fraction,
};
