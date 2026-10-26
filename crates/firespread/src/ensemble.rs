//! The `ensemble` command: one forecast run of a window through the agents
//! [`Forecaster`], with barrier roads folded into the water layer.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use firespread_agents::{
    EnsembleSpec, Event, Executor, ForecastConfig, Forecaster, GridInputs, Published, Source,
    load_grid_inputs,
};
use firespread_ensemble::{HourlyConfig, HourlyPerturbation, WindowWeather, apply_road_barriers};
use firespread_grid::{ForecastWindow, GridHeader, Manifest, read_geotiff_u8};
use serde::{Deserialize, Serialize};

use crate::settings::{RunSettings, load_window};
use crate::timefmt::{feds_label, parse_hour};

/// Longest wait for [`Forecaster::shutdown`] to drain the agents.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// The weather source the manifest records.
pub const HRRR_SOURCE: &str =
    "weather/index.json: HRRR u10, v10, t2m, rh2m, tcc per hour on the grid";

/// File of per-member burned cell counts in a run directory.
pub const MEMBERS_FILE: &str = "members.json";
/// File of the run's settings in a run directory.
pub const RUN_CONFIG_FILE: &str = "run_config.json";

/// What `firespread ensemble` was asked to run.
#[derive(Debug, Clone)]
pub struct EnsembleArgs {
    pub window: String,
    pub members: u32,
    pub seed: u64,
    pub threads: usize,
    pub runs_dir: PathBuf,
    pub data: PathBuf,
    pub settings: RunSettings,
}

/// The run id `<window>-seed<seed>-m<members>`.
#[must_use]
pub fn run_id(window: &str, seed: u64, members: u32) -> String {
    format!("{window}-seed{seed}-m{members}")
}

/// One member's burned cell counts, per horizon of the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberArea {
    pub member: u32,
    /// Start-mask cells inside the grid after the ignition shift.
    pub ignited_cells: usize,
    /// Cells with arrival time `<= 3600 h` s, per horizon `h`.
    pub burned_cells: Vec<usize>,
}

/// `members.json`: per-member burned cells of the completed members.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MembersFile {
    pub run_id: String,
    pub horizons_h: Vec<u32>,
    pub members: Vec<MemberArea>,
}

/// `run_config.json`: the settings a run was made with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfigFile {
    pub run_id: String,
    pub window: String,
    pub seed: u64,
    pub members_requested: u32,
    pub members_completed: u32,
    pub threads: usize,
    pub road_barrier_codes: Vec<u8>,
    /// Cells the barrier codes made non-burnable.
    pub road_barrier_cells: usize,
    pub horizons_h: Vec<u32>,
}

/// What a finished run reports.
#[derive(Debug)]
pub struct RunSummary {
    pub run_dir: PathBuf,
    pub completed: usize,
    pub failed: usize,
    /// Wall time of the ensemble, s.
    pub ensemble_s: f64,
    /// Wall time from `Forecaster::new` to the published run, s.
    pub total_s: f64,
}

/// The inputs map of the manifest for a run whose start mask is `start_mask`.
fn manifest_inputs(start_mask: &str, roads: bool) -> BTreeMap<String, String> {
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

fn geometry(h: &GridHeader) -> GridHeader {
    GridHeader { nodata: 0.0, ..*h }
}

/// Loads the window's static rasters, start mask and weather from `args.data`,
/// applies the barrier road codes, and runs [`forecast`].
pub async fn run(args: &EnsembleArgs, stop: impl Future<Output = ()>) -> Result<RunSummary> {
    let window = load_window(&args.data, &args.window)?;
    let start_mask = format!(
        "obs/feds/{}.tif",
        feds_label(parse_hour(&window.label_start)?)
    );
    let mut grid = {
        let exec = Executor::with_threads(args.threads)?;
        load_grid_inputs(&exec, &args.data.join("grid"), &args.data.join(&start_mask))
            .await
            .context("loading the static rasters")?
    };
    let codes = &args.settings.road_barrier_codes;
    let barrier_cells = if codes.is_empty() {
        0
    } else {
        let path = args.data.join("grid/roads.tif");
        let roads =
            read_geotiff_u8(&path).with_context(|| format!("reading {}", path.display()))?;
        ensure!(
            geometry(&roads.header).same_grid(&geometry(&grid.header)),
            "{} is not on the FBFM40 grid",
            path.display()
        );
        apply_road_barriers(&mut grid.water, &roads.data, codes)?
    };
    let hours = args
        .settings
        .horizons_h
        .iter()
        .copied()
        .max()
        .context("no horizons")?;
    let hourly = HourlyConfig::default();
    let weather = {
        let (dir, name, header, moisture) = (
            args.data.join("weather"),
            window.name.clone(),
            grid.header,
            hourly.moisture,
        );
        tokio::task::spawn_blocking(move || {
            WindowWeather::load(&dir, &name, hours as usize, &header, &moisture)
        })
        .await
        .context("weather load task")?
        .context("loading the weather")?
    };
    let config = ForecastConfig {
        run_id: run_id(&window.name, args.seed, args.members),
        runs_dir: args.runs_dir.clone(),
        window: ForecastWindow {
            name: window.name,
            start_t: window.start_t,
            target_t: window.target_t,
            fireids: window.fireids,
        },
        hourly,
        ensemble: EnsembleSpec {
            seed: args.seed,
            members: args.members,
            horizons_h: args.settings.horizons_h.clone(),
            perturbation: HourlyPerturbation::default(),
        },
        threads: args.threads,
    };
    let record = Record {
        inputs: manifest_inputs(&start_mask, !codes.is_empty()),
        hrrr_source: HRRR_SOURCE.to_string(),
        settings: args.settings.clone(),
        barrier_cells,
    };
    forecast(config, grid, weather, &record, stop).await
}

/// What [`forecast`] writes into the run directory besides the agents' files.
#[derive(Debug, Clone)]
pub struct Record {
    /// The manifest's `inputs`.
    pub inputs: BTreeMap<String, String>,
    /// The manifest's `hrrr_source`.
    pub hrrr_source: String,
    pub settings: RunSettings,
    /// Cells the barrier codes made non-burnable.
    pub barrier_cells: usize,
}

async fn drain(forecaster: &Forecaster) -> Result<()> {
    let report = tokio::time::timeout(DRAIN_TIMEOUT, forecaster.shutdown())
        .await
        .with_context(|| format!("the agents did not drain within {DRAIN_TIMEOUT:?}"))?;
    ensure!(
        report.tasks_left == 0,
        "{} agent tasks alive after the drain",
        report.tasks_left
    );
    Ok(())
}

/// Runs `config` on in-memory `grid` and `weather` through a [`Forecaster`]
/// until it publishes, then drains it within [`DRAIN_TIMEOUT`] and writes the
/// manifest's `inputs` and `hrrr_source` from `record`, [`MEMBERS_FILE`] and
/// [`RUN_CONFIG_FILE`]. When `stop` completes first, cancels the run, drains
/// it within [`DRAIN_TIMEOUT`] and fails.
pub async fn forecast(
    config: ForecastConfig,
    grid: GridInputs,
    weather: WindowWeather,
    record: &Record,
    stop: impl Future<Output = ()>,
) -> Result<RunSummary> {
    let (threads, members_requested) = (config.threads, config.ensemble.members);
    let t = Instant::now();
    let forecaster = Forecaster::new(
        config,
        Source::Memory {
            grid: Box::new(grid),
            weather,
        },
    )
    .await?;
    let inbox = forecaster.inbox();
    tokio::pin!(stop);
    let mut finished = 0_u32;
    let published: Box<Published> = loop {
        let delivery = tokio::select! {
            biased;
            () = &mut stop => {
                eprintln!("stop requested: cancelling the run");
                forecaster.cancellation_token().cancel();
                drain(&forecaster).await?;
                bail!("run interrupted after {finished} of {members_requested} members");
            }
            d = inbox.recv() => d.context("inbox closed before the run was published")?,
        };
        match delivery.message {
            Event::Progress(p) => {
                finished = p.finished;
                if p.finished % 20 == 0 || p.finished == p.members {
                    eprintln!("members {}/{}", p.finished, p.members);
                }
            }
            Event::Published(p) => break p,
            Event::Failed { agent, error } => {
                drain(&forecaster).await?;
                bail!("agent {agent} failed: {error}");
            }
            _ => {}
        }
    };
    let total_s = t.elapsed().as_secs_f64();
    drain(&forecaster).await?;

    let manifest_path = &published.manifest;
    let mut manifest = Manifest::read(manifest_path)?;
    manifest.inputs.clone_from(&record.inputs);
    manifest.hrrr_source.clone_from(&record.hrrr_source);
    manifest.write(manifest_path)?;

    let outcome = &published.outcome;
    let members = outcome
        .completed
        .iter()
        .zip(&outcome.counts.summaries)
        .map(|(&member, s)| MemberArea {
            member,
            ignited_cells: s.stats.ignited,
            burned_cells: s.burned_cells.clone(),
        })
        .collect();
    write_json(
        &published.run_dir.join(MEMBERS_FILE),
        &MembersFile {
            run_id: manifest.run_id.clone(),
            horizons_h: outcome.counts.horizons_h.clone(),
            members,
        },
    )?;
    write_json(
        &published.run_dir.join(RUN_CONFIG_FILE),
        &RunConfigFile {
            run_id: manifest.run_id.clone(),
            window: manifest.window.name.clone(),
            seed: manifest.seed,
            members_requested,
            members_completed: outcome.counts.members,
            threads,
            road_barrier_codes: record.settings.road_barrier_codes.clone(),
            road_barrier_cells: record.barrier_cells,
            horizons_h: record.settings.horizons_h.clone(),
        },
    )?;
    Ok(RunSummary {
        run_dir: published.run_dir.clone(),
        completed: outcome.completed.len(),
        failed: outcome.failures.len(),
        ensemble_s: outcome.wall_s,
        total_s,
    })
}

/// Writes `value` as pretty JSON with a trailing newline.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Reads JSON from `path`.
pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}
