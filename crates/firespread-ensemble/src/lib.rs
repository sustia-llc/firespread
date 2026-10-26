#![forbid(unsafe_code)]
//! Ensembles of spread runs over perturbed inputs: static per-cell
//! [`Inputs`], one member's fields and spread window ([`run_member`]), and
//! seeded parallel ensembles of burn counts ([`run_ensemble`]).

mod ensemble;
mod error;
mod fbfm40;
mod inputs;
mod member;

pub use ensemble::{Counts, MemberSummary, Perturbation, run_ensemble};
pub use error::{EnsembleError, Result};
pub use fbfm40::{BURNABLE_FBFM40, Fbfm40, classify_fbfm40};
pub use inputs::{Inputs, Layers};
pub use member::{MemberBuffers, MemberParams, run_member};
