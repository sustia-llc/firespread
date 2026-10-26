#![forbid(unsafe_code)]
//! The forecast runtime: a [`Forecaster`] owns a root cancellation token, a
//! task tracker, an in-process [`Bus`] and a bounded rayon [`Executor`];
//! loaders read the static rasters once; the Weather, Forecast and Publisher
//! agents run as tokio tasks that load the window's weather, run the
//! ensemble member by member on the executor, and write the burn-probability
//! COGs and manifest of the run.

mod agents;
mod bus;
mod error;
mod executor;
mod forecast;
mod forecaster;
mod loaders;

pub use agents::{Event, FORECAST, INBOX, PUBLISHER, Published, WEATHER};
pub use bus::{Bus, Delivery, KanalBus};
pub use error::{AgentError, Result};
pub use executor::Executor;
pub use firespread_ensemble::panic_message;
pub use forecast::{EnsembleOutcome, EnsembleSpec, MemberOutcome, MemberProgress, run_members};
pub use forecaster::{ForecastConfig, Forecaster, ShutdownReport, Source};
pub use loaders::{GridInputs, load_grid_inputs};
