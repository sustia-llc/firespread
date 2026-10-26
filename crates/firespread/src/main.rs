#![forbid(unsafe_code)]
//! Command-line entry point for the firespread engine: `ensemble` runs a
//! forecast window through the agents runtime, `score` scores a run.

mod ensemble;
mod score;
mod settings;
mod timefmt;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::ensemble::EnsembleArgs;
use crate::score::{DEFAULT_BRIER_RADIUS_M, DEFAULT_BUFFER_M, ScoreOptions};
use crate::settings::RunSettings;

#[derive(Debug, Parser)]
#[command(
    name = "firespread",
    version,
    about = "Wildfire spread forecast and scorecard"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the ensemble of a window and write its COGs, manifest and member areas.
    Ensemble {
        /// Window name in `<data>/windows.json`.
        #[arg(long)]
        window: String,
        #[arg(long)]
        members: u32,
        #[arg(long)]
        seed: u64,
        /// Threads of the compute pool.
        #[arg(long)]
        threads: usize,
        /// The run goes to `<runs-dir>/<window>-seed<seed>-m<members>`.
        #[arg(long)]
        runs_dir: PathBuf,
        /// Data directory holding `windows.json`, `grid/`, `weather/` and `obs/`.
        #[arg(long)]
        data: PathBuf,
        /// TOML file of run settings (`road_barrier_codes`, `horizons_h`).
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Score a run: `scorecard.json`, `area_by_hour.json`, `confusion_h<N>.tif`,
    /// `uniform_growth_h<N>.tif`, `observed.geojson`.
    Score {
        /// Run directory written by `ensemble`.
        #[arg(long)]
        run: PathBuf,
        /// Data directory holding `windows.json`, `grid/` and `obs/`.
        #[arg(long)]
        data: PathBuf,
        /// Uniform-growth rate, m/h; fitted from FEDS when absent.
        #[arg(long)]
        uniform_rate_m_per_h: Option<f64>,
        /// Radius of the Brier domain around the start perimeter, m.
        #[arg(long, default_value_t = DEFAULT_BRIER_RADIUS_M)]
        brier_radius_m: f64,
        /// Tolerance of the buffered precision, recall and Dice, m.
        #[arg(long, default_value_t = DEFAULT_BUFFER_M)]
        buffer_m: f64,
    },
}

/// Completes on the first SIGINT or SIGTERM. A handler that cannot be
/// installed never completes.
async fn stop_signal() {
    let interrupt = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        () = interrupt => eprintln!("SIGINT received"),
        () = terminate => eprintln!("SIGTERM received"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Ensemble {
            window,
            members,
            seed,
            threads,
            runs_dir,
            data,
            config,
        } => {
            let settings = RunSettings::load(config.as_deref())?;
            let args = EnsembleArgs {
                window,
                members,
                seed,
                threads,
                runs_dir,
                data,
                settings,
            };
            let summary = ensemble::run(&args, stop_signal()).await?;
            println!("run_dir {}", summary.run_dir.display());
            println!(
                "members completed {}, failed {}; ensemble {:.3} s, Forecaster::new to published {:.3} s",
                summary.completed, summary.failed, summary.ensemble_s, summary.total_s
            );
        }
        Command::Score {
            run,
            data,
            uniform_rate_m_per_h,
            brier_radius_m,
            buffer_m,
        } => {
            let opts = ScoreOptions {
                uniform_rate_m_per_h,
                brier_radius_m,
                buffer_m,
            };
            let out = tokio::task::spawn_blocking(move || score::score(&run, &data, &opts))
                .await
                .context("score task")??;
            for f in &out.files {
                println!("wrote {}", f.display());
            }
            println!("{}", serde_json::to_string_pretty(&out.scorecard)?);
        }
    }
    Ok(())
}
