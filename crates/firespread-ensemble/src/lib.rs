#![forbid(unsafe_code)]
//! Ensembles of spread runs over perturbed inputs: static per-cell
//! [`Inputs`], one member's fields and spread window ([`run_member`]), and
//! seeded parallel ensembles of burn counts ([`run_ensemble`]); and the same
//! over a window of hourly weather ([`HourlyInputs`], [`run_hourly_member`],
//! [`run_hourly_ensemble`]) with the wind adjustment factor ([`waf`]) and
//! hourly dead fuel moisture ([`moisture`]).

mod ensemble;
mod error;
mod fbfm40;
mod hourly;
mod inputs;
mod member;
pub mod moisture;
pub mod waf;
mod weather;

pub use ensemble::{Counts, MemberSummary, Perturbation, run_ensemble};
pub use error::{EnsembleError, Result};
pub use fbfm40::{BURNABLE_FBFM40, Fbfm40, classify_fbfm40};
pub use hourly::{
    HourlyConfig, HourlyCounts, HourlyInputs, HourlyLayers, HourlyMemberSummary,
    HourlyPerturbation, HourlyStats, MemberDraw, N_MODELS, S_PER_HOUR, run_hourly_ensemble,
    run_hourly_member, write_probability_cogs,
};
pub use inputs::{Inputs, Layers, ROAD_BARRIER_WATER_CODE, apply_road_barriers};
pub use member::{MemberBuffers, MemberFields, MemberParams, run_member};
pub use moisture::{DeadFuelMoisture, SkyEdges};
pub use waf::{Canopy, WafRule};
pub use weather::{HourFields, RawHour, WindowWeather};
