//! Agents gate: the Phase 3 gate run of window w1 (seed 7, 200 members,
//! horizons 1, 3, 6, 12 h, 12 pool threads) through the [`Forecaster`]:
//! loaders, Weather, Forecast and Publisher. Writes the run into
//! `<runs_dir>/agents_gate-w1-seed7-m200/`, the wall time from
//! `Forecaster::new` to the published run to `wall_time.txt` there, and the
//! printout to `gate.txt` there.
//!
//! Usage: `cargo run --release -p firespread-agents --example agents_gate --
//! <data_dir> <runs_dir>`, where `<data_dir>` holds `windows.json`, `grid/`,
//! `weather/index.json` and `obs/feds/`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use firespread_agents::{EnsembleSpec, Event, ForecastConfig, Forecaster, Source};
use firespread_ensemble::{HourlyConfig, HourlyPerturbation};
use firespread_grid::ForecastWindow;
use serde::Deserialize;

const WINDOW: &str = "w1";
const THREADS: usize = 12;
const MEMBERS: u32 = 200;
const SEED: u64 = 7;
const HORIZONS_H: [u32; 4] = [1, 3, 6, 12];

#[derive(Deserialize)]
struct Windows {
    windows: Vec<Window>,
}

#[derive(Deserialize)]
struct Window {
    name: String,
    label_start: String,
    start_t: String,
    target_t: String,
    fireids: Vec<u32>,
}

/// `2022-04-22T12:00:00` to `20220422T1200`.
fn feds_label(t: &str) -> Result<String> {
    let digits: String = t.chars().filter(|c| *c != '-' && *c != ':').collect();
    digits
        .get(..13)
        .map(str::to_string)
        .with_context(|| format!("label {t:?} is shorter than YYYY-MM-DDTHH:MM"))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let (Some(data_dir), Some(runs_dir)) = (args.next(), args.next()) else {
        bail!("usage: agents_gate <data_dir> <runs_dir>");
    };
    let mut out = String::new();

    let windows_path = data_dir.join("windows.json");
    let windows: Windows = serde_json::from_str(
        &std::fs::read_to_string(&windows_path)
            .with_context(|| format!("reading {}", windows_path.display()))?,
    )
    .with_context(|| format!("parsing {}", windows_path.display()))?;
    let window = windows
        .windows
        .into_iter()
        .find(|w| w.name == WINDOW)
        .with_context(|| format!("{WINDOW} not in {}", windows_path.display()))?;
    let start_mask = format!("obs/feds/{}.tif", feds_label(&window.label_start)?);

    let config = ForecastConfig {
        run_id: format!("agents_gate-{WINDOW}-seed{SEED}-m{MEMBERS}"),
        runs_dir,
        window: ForecastWindow {
            name: window.name,
            start_t: window.start_t,
            target_t: window.target_t,
            fireids: window.fireids,
        },
        hourly: HourlyConfig::default(),
        ensemble: EnsembleSpec {
            seed: SEED,
            members: MEMBERS,
            horizons_h: HORIZONS_H.to_vec(),
            perturbation: HourlyPerturbation::default(),
        },
        threads: THREADS,
    };
    writeln!(
        out,
        "window {WINDOW}, start mask {start_mask}, members {MEMBERS}, seed {SEED}, \
         horizons {HORIZONS_H:?} h, pool threads {THREADS}"
    )?;

    let t = Instant::now();
    let forecaster = Forecaster::new(
        config,
        Source::Files {
            data_dir,
            start_mask,
        },
    )
    .await?;
    let load_s = t.elapsed().as_secs_f64();
    writeln!(
        out,
        "Forecaster::new (static rasters, spawn): {load_s:.3} s, admission {}",
        forecaster.executor().max_in_flight()
    )?;
    let inbox = forecaster.inbox();
    let mut progress = 0_u32;
    let published = loop {
        let delivery = inbox.recv().await.context("inbox closed before publish")?;
        match delivery.message {
            Event::Progress(_) => progress += 1,
            Event::Published(p) => break p,
            Event::Failed { agent, error } => {
                forecaster.shutdown().await;
                bail!("{agent} failed: {error}");
            }
            _ => {}
        }
    };
    let total_s = t.elapsed().as_secs_f64();
    let report = forecaster.shutdown().await;
    let outcome = &published.outcome;
    writeln!(
        out,
        "GATE agents path {MEMBERS} members on {THREADS} threads: {total_s:.3} s from \
         Forecaster::new to published; ensemble {:.3} s",
        outcome.wall_s
    )?;
    writeln!(
        out,
        "progress events {progress}, completed {}, failed {}, cancelled {}",
        outcome.completed.len(),
        outcome.failures.len(),
        outcome.cancelled
    )?;
    writeln!(out, "shutdown {report:?}")?;
    for (key, path) in &published.outputs {
        writeln!(out, "{key}: {}", path.display())?;
    }
    writeln!(out, "wrote {}", published.manifest.display())?;
    let wall = published.run_dir.join("wall_time.txt");
    std::fs::write(&wall, format!("{total_s:.3}\n"))
        .with_context(|| format!("writing {}", wall.display()))?;
    print!("{out}");
    let txt = published.run_dir.join("gate.txt");
    std::fs::write(&txt, &out).with_context(|| format!("writing {}", txt.display()))?;
    Ok(())
}
