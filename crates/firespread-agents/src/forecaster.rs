//! The owner of a run: token tree, task tracker, bus and executor.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use firespread_ensemble::{HourlyConfig, WindowWeather};
use firespread_grid::ForecastWindow;
use kanal::AsyncReceiver;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument;

use crate::Result;
use crate::agents::{
    Event, FORECAST, ForecastCtx, INBOX, PUBLISHER, PublishSpec, WEATHER, WeatherSource,
    forecast_agent, publisher_agent, weather_agent,
};
use crate::bus::{Bus, Delivery, KanalBus};
use crate::executor::Executor;
use crate::forecast::EnsembleSpec;
use crate::loaders::{GridInputs, load_grid_inputs};

/// The settings of one forecast run.
#[derive(Debug, Clone, PartialEq)]
pub struct ForecastConfig {
    /// Name of the run; its files go to `runs_dir/run_id`.
    pub run_id: String,
    pub runs_dir: PathBuf,
    /// The forecast window; `window.name` selects the weather index window.
    pub window: ForecastWindow,
    pub hourly: HourlyConfig,
    pub ensemble: EnsembleSpec,
    /// Threads of the compute pool; admission is twice this.
    pub threads: usize,
}

/// Where a run reads its rasters and weather.
#[derive(Debug, Clone)]
pub enum Source {
    /// `grid/` and `weather/index.json` under `data_dir`, and the start mask
    /// at `data_dir/start_mask`. Cells of `grid/roads.tif` whose code is in
    /// `road_barrier_codes` are non-burnable; an empty list reads no roads.
    Files {
        data_dir: PathBuf,
        start_mask: String,
        road_barrier_codes: Vec<u8>,
    },
    /// Rasters and weather already in memory.
    Memory {
        grid: Box<GridInputs>,
        weather: WindowWeather,
    },
}

/// The state [`Forecaster::shutdown`] observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    /// The root token was cancelled when the tracker was closed.
    pub cancelled_at_close: bool,
    /// Tracked tasks still alive when the drain returned.
    pub tasks_left: usize,
}

/// One forecast run: the root [`CancellationToken`], a [`TaskTracker`] of
/// the agents, the bus and its inbox, the [`Executor`], and the static
/// rasters.
#[derive(Debug)]
pub struct Forecaster {
    token: CancellationToken,
    tracker: TaskTracker,
    bus: Arc<KanalBus<Event>>,
    inbox: AsyncReceiver<Delivery<Event>>,
    exec: Arc<Executor>,
    grid: Arc<GridInputs>,
    road_barrier_cells: usize,
    run_dir: PathBuf,
}

const HRRR_SOURCE: &str = "weather/index.json: HRRR u10, v10, t2m, rh2m, tcc per hour on the grid";

fn files_inputs(start_mask: &str, roads: bool) -> BTreeMap<String, String> {
    let mut inputs: BTreeMap<String, String> = [
        ("fbfm40", "grid/FBFM40.tif"),
        ("slope", "grid/SlpD.tif"),
        ("aspect", "grid/Asp.tif"),
        ("canopy_height", "grid/CH.tif"),
        ("canopy_base_height", "grid/CBH.tif"),
        ("canopy_cover", "grid/CC.tif"),
        ("water", "grid/water.tif"),
        ("start_mask", start_mask),
        ("weather_index", "weather/index.json"),
        ("windows", "windows.json"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    if roads {
        inputs.insert("roads".to_string(), "grid/roads.tif".to_string());
    }
    inputs
}

impl Forecaster {
    /// Validates the ensemble, builds an [`Executor`] of `config.threads`
    /// threads and `2 * config.threads` admission permits, loads the static
    /// rasters of `source` on it, and spawns the Weather, Forecast and
    /// Publisher agents under the tracker, each on a child token inside a
    /// span named `agent`. The run starts at once: Weather loads the first
    /// `max(horizons_h)` hours of `config.window.name`, Forecast runs the
    /// ensemble, and Publisher writes into `runs_dir/run_id`. Progress,
    /// [`Event::Published`] and [`Event::Failed`] arrive on
    /// [`Forecaster::inbox`].
    ///
    /// # Errors
    /// The errors of [`EnsembleSpec::validated_horizon`],
    /// [`Executor::with_threads`], [`load_grid_inputs`], and
    /// [`crate::AgentError::DuplicateAgent`].
    pub async fn new(config: ForecastConfig, source: Source) -> Result<Self> {
        let horizon_max = config.ensemble.validated_horizon()?;
        let exec = Arc::new(Executor::with_threads(config.threads)?);
        let (grid, road_barrier_cells, weather, inputs, hrrr_source) = match source {
            Source::Files {
                data_dir,
                start_mask,
                road_barrier_codes,
            } => {
                let (grid, road_barrier_cells) = load_grid_inputs(
                    &exec,
                    &data_dir.join("grid"),
                    &data_dir.join(&start_mask),
                    &road_barrier_codes,
                )
                .await?;
                let weather = WeatherSource::Index {
                    weather_dir: data_dir.join("weather"),
                    window: config.window.name.clone(),
                    hours: usize::try_from(horizon_max).unwrap_or(usize::MAX),
                    header: grid.header,
                    moisture: config.hourly.moisture,
                };
                (
                    grid,
                    road_barrier_cells,
                    weather,
                    files_inputs(&start_mask, !road_barrier_codes.is_empty()),
                    HRRR_SOURCE.to_string(),
                )
            }
            Source::Memory { grid, weather } => (
                *grid,
                0,
                WeatherSource::Memory(weather),
                BTreeMap::new(),
                "in-memory weather".to_string(),
            ),
        };
        let grid = Arc::new(grid);
        let run_dir = config.runs_dir.join(&config.run_id);

        let bus = KanalBus::<Event>::new();
        let inbox = bus.register(INBOX)?;
        let forecast_rx = bus.register(FORECAST)?;
        let publisher_rx = bus.register(PUBLISHER)?;

        let this = Self {
            token: CancellationToken::new(),
            tracker: TaskTracker::new(),
            bus,
            inbox,
            exec,
            grid,
            road_barrier_cells,
            run_dir: run_dir.clone(),
        };

        let publish = PublishSpec {
            run_id: config.run_id.clone(),
            run_dir,
            window: config.window.clone(),
            inputs,
            hrrr_source,
            seed: config.ensemble.seed,
            wind_limit: config.hourly.limit,
            horizons_h: config.ensemble.horizons_h.clone(),
        };
        let (bus, exec, grid) = (
            Arc::clone(&this.bus),
            Arc::clone(&this.exec),
            Arc::clone(&this.grid),
        );
        this.spawn_agent(PUBLISHER, move |token| {
            publisher_agent(bus, exec, grid, publish, publisher_rx, token)
        });
        let ctx = ForecastCtx {
            bus: Arc::clone(&this.bus),
            exec: Arc::clone(&this.exec),
            grid: Arc::clone(&this.grid),
            hourly: config.hourly,
            spec: config.ensemble,
        };
        this.spawn_agent(FORECAST, move |token| {
            forecast_agent(ctx, forecast_rx, token)
        });
        let (bus, exec, window) = (
            Arc::clone(&this.bus),
            Arc::clone(&this.exec),
            config.window.name,
        );
        this.spawn_agent(WEATHER, move |token| {
            weather_agent(bus, exec, window, weather, token)
        });
        Ok(this)
    }

    /// Spawns `agent(child)` under the tracker inside a span `agent` with
    /// field `name`, where `child` is a new child of the root token.
    pub fn spawn_agent<F, Fut>(&self, name: &str, agent: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let span = tracing::info_span!("agent", name = %name);
        self.tracker
            .spawn(agent(self.token.child_token()).instrument(span));
    }

    /// The root token; cancelling it stops every agent.
    #[must_use]
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.token
    }

    /// A receiver of the inbox mailbox: per-member progress,
    /// [`Event::Published`] and [`Event::Failed`]. Receives fail once the bus
    /// is closed by [`Forecaster::shutdown`].
    #[must_use]
    pub fn inbox(&self) -> AsyncReceiver<Delivery<Event>> {
        self.inbox.clone()
    }

    /// The bus the agents exchange messages on.
    #[must_use]
    pub fn bus(&self) -> &Arc<KanalBus<Event>> {
        &self.bus
    }

    /// The compute pool and its admission semaphore.
    #[must_use]
    pub fn executor(&self) -> &Arc<Executor> {
        &self.exec
    }

    /// The static rasters of the run.
    #[must_use]
    pub fn grid(&self) -> &Arc<GridInputs> {
        &self.grid
    }

    /// Cells the road barrier codes of [`Source::Files`] made non-burnable;
    /// 0 for [`Source::Memory`].
    #[must_use]
    pub fn road_barrier_cells(&self) -> usize {
        self.road_barrier_cells
    }

    /// The directory Publisher writes into.
    #[must_use]
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Cancels the root token, closes the tracker, waits for every tracked
    /// task to end, in that order, then closes the bus.
    pub async fn shutdown(&self) -> ShutdownReport {
        self.token.cancel();
        let cancelled_at_close = self.token.is_cancelled();
        self.tracker.close();
        self.tracker.wait().await;
        let tasks_left = self.tracker.len();
        self.bus.close();
        ShutdownReport {
            cancelled_at_close,
            tasks_left,
        }
    }
}
