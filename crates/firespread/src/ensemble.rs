//! The `ensemble` command: one forecast run of a window through the agents
//! [`Forecaster`], with barrier road codes passed to its files source.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use firespread_agents::{EnsembleSpec, Event, ForecastConfig, Forecaster, Published, Source};
use firespread_ensemble::{HourlyConfig, HourlyPerturbation};
use firespread_grid::{ForecastWindow, Manifest};
use serde::{Deserialize, Serialize};

use crate::settings::{RunSettings, load_window};
use crate::timefmt::{feds_label, parse_hour};

/// Longest wait for [`Forecaster::shutdown`] to drain the agents.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Runs window `args.window` of `args.data` through [`forecast`] on
/// [`Source::Files`] with the settings' road barrier codes.
pub async fn run(args: &EnsembleArgs, stop: impl Future<Output = ()>) -> Result<RunSummary> {
    let window = load_window(&args.data, &args.window)?;
    let start_mask = format!(
        "obs/feds/{}.tif",
        feds_label(parse_hour(&window.label_start)?)
    );
    let config = ForecastConfig {
        run_id: run_id(&window.name, args.seed, args.members),
        runs_dir: args.runs_dir.clone(),
        window: ForecastWindow {
            name: window.name,
            start_t: window.start_t,
            target_t: window.target_t,
            fireids: window.fireids,
        },
        hourly: HourlyConfig::default(),
        ensemble: EnsembleSpec {
            seed: args.seed,
            members: args.members,
            horizons_h: args.settings.horizons_h.clone(),
            perturbation: HourlyPerturbation::default(),
        },
        threads: args.threads,
    };
    let source = Source::Files {
        data_dir: args.data.clone(),
        start_mask,
        road_barrier_codes: args.settings.road_barrier_codes.clone(),
    };
    forecast(config, source, &args.settings, stop).await
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

/// Runs `config` on `source` through a [`Forecaster`] until it publishes,
/// then drains it within [`DRAIN_TIMEOUT`] and writes [`MEMBERS_FILE`] and
/// [`RUN_CONFIG_FILE`], the latter recording `settings`. When `stop`
/// completes first, cancels the run, drains it within [`DRAIN_TIMEOUT`] and
/// fails.
pub async fn forecast(
    config: ForecastConfig,
    source: Source,
    settings: &RunSettings,
    stop: impl Future<Output = ()>,
) -> Result<RunSummary> {
    let (threads, members_requested) = (config.threads, config.ensemble.members);
    let t = Instant::now();
    let forecaster = Forecaster::new(config, source)
        .await
        .context("loading the run inputs")?;
    let road_barrier_cells = forecaster.road_barrier_cells();
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

    let manifest = Manifest::read(&published.manifest)?;
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
            road_barrier_codes: settings.road_barrier_codes.clone(),
            road_barrier_cells,
            horizons_h: settings.horizons_h.clone(),
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
