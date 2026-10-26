//! Phase 3 gate: a 200-member hourly ensemble of window w1 on the Hermits
//! Peak/Calf Canyon grid with HRRR wind, temperature, humidity and cloud
//! cover and gridMET 100-h moisture, ignited from the window's FEDS start
//! mask. Writes one burn-probability COG per horizon (1, 3, 6, 12 h), a
//! `manifest.json`, the ensemble wall time to `wall_time.txt` and the
//! printout to `gate.txt` in `<out_dir>`.
//!
//! Usage: `cargo run --release -p firespread-ensemble --example phase3_gate --
//! <data_dir> <out_dir>`, where `<data_dir>` holds `windows.json`, `grid/`,
//! `weather/index.json` and `obs/feds/`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use firespread_ensemble::{
    HourlyConfig, HourlyInputs, HourlyPerturbation, run_hourly_ensemble, write_probability_cogs,
};
use firespread_grid::{ForecastWindow, MANIFEST_VERSION, Manifest, WindLimitPolicy};
use firespread_rothermel::WindLimit;
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

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let (Some(data_dir), Some(out_dir)) = (args.next(), args.next()) else {
        bail!("usage: phase3_gate <data_dir> <out_dir>");
    };
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let mut out = String::new();

    rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build_global()
        .context("building the rayon global pool")?;
    writeln!(out, "rayon threads: {}", rayon::current_num_threads())?;

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
    let ignition_rel = format!("obs/feds/{}.tif", feds_label(&window.label_start)?);

    let config = HourlyConfig::default();
    let perturbation = HourlyPerturbation::default();
    let hours = usize::try_from(HORIZONS_H[HORIZONS_H.len() - 1])?;
    writeln!(
        out,
        "window {WINDOW} {} to {}, ignition {ignition_rel}, members {MEMBERS}, seed {SEED}, \
         horizons {HORIZONS_H:?} h, {hours} weather hours",
        window.start_t, window.target_t
    )?;
    writeln!(out, "config {config:?}")?;
    writeln!(out, "perturbation {perturbation:?}")?;

    let t = Instant::now();
    let inputs = HourlyInputs::load(
        &data_dir.join("grid"),
        &data_dir.join("weather"),
        WINDOW,
        &data_dir.join(&ignition_rel),
        hours,
        &config,
    )?;
    let load_s = t.elapsed().as_secs_f64();
    let header = *inputs.inputs().header();
    let cell_km2 = header.dx * header.dy / 1e6;
    let burnable = inputs.inputs().burnable().iter().filter(|&&b| b).count();
    writeln!(
        out,
        "grid {}x{} dx {} m: {burnable} burnable, start mask {} cells ({:.3} km2)",
        header.nx,
        header.ny,
        header.dx,
        inputs.ignition().len(),
        inputs.ignition().len() as f64 * cell_km2
    )?;
    let waf: Vec<f64> = inputs
        .inputs()
        .burnable()
        .iter()
        .zip(inputs.waf())
        .filter(|(b, _)| **b)
        .map(|(_, &w)| f64::from(w))
        .collect();
    writeln!(
        out,
        "WAF over burnable cells: min {:.4} median {:.4} max {:.4}",
        waf.iter().copied().fold(f64::INFINITY, f64::min),
        median(waf.clone()),
        waf.iter().copied().fold(0.0, f64::max)
    )?;
    writeln!(
        out,
        "weather hours {:?}",
        inputs
            .weather()
            .hours_utc
            .first()
            .zip(inputs.weather().hours_utc.last())
    )?;
    writeln!(
        out,
        "input load (grid, canopy, start mask, {hours} h weather): {load_s:.3} s"
    )?;

    let t = Instant::now();
    let counts = run_hourly_ensemble(&inputs, &perturbation, SEED, MEMBERS, &HORIZONS_H)?;
    let gate_s = t.elapsed().as_secs_f64();
    writeln!(
        out,
        "GATE run_hourly_ensemble {MEMBERS} members on {} threads: {gate_s:.3} s",
        rayon::current_num_threads()
    )?;
    let wall = out_dir.join("wall_time.txt");
    std::fs::write(&wall, format!("{gate_s:.3}\n"))
        .with_context(|| format!("writing {}", wall.display()))?;

    for (k, h) in HORIZONS_H.iter().enumerate() {
        let any = counts.counts[k].iter().filter(|&&c| c > 0).count();
        let half = counts.counts[k]
            .iter()
            .filter(|&&c| 2 * u32::from(c) >= MEMBERS)
            .count();
        writeln!(
            out,
            "{h} h: burned by any member {any} cells ({:.3} km2), p >= 0.5 {half} cells ({:.3} km2)",
            any as f64 * cell_km2,
            half as f64 * cell_km2
        )?;
    }
    let expanded = median(
        counts
            .summaries
            .iter()
            .map(|s| s.stats.expanded as f64)
            .collect(),
    );
    writeln!(
        out,
        "cells expanded per member over all hours, median: {expanded}"
    )?;
    writeln!(out, "member 0 draw: {:?}", counts.summaries[0].draw)?;

    let t = Instant::now();
    let cogs = write_probability_cogs(&counts, &header, &out_dir)?;
    writeln!(
        out,
        "wrote {} COGs in {:.3} s",
        cogs.len(),
        t.elapsed().as_secs_f64()
    )?;

    let rel =
        |p: &PathBuf| -> String { p.strip_prefix(&out_dir).unwrap_or(p).display().to_string() };
    let inputs_map: BTreeMap<String, String> = [
        ("fbfm40", "grid/FBFM40.tif"),
        ("slope", "grid/SlpD.tif"),
        ("aspect", "grid/Asp.tif"),
        ("canopy_height", "grid/CH.tif"),
        ("canopy_base_height", "grid/CBH.tif"),
        ("canopy_cover", "grid/CC.tif"),
        ("water", "grid/water.tif"),
        ("start_mask", ignition_rel.as_str()),
        ("weather_index", "weather/index.json"),
        ("windows", "windows.json"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        run_id: format!("phase3_gate-{WINDOW}-seed{SEED}-m{MEMBERS}"),
        window: ForecastWindow {
            name: window.name,
            start_t: window.start_t,
            target_t: window.target_t,
            fireids: window.fireids,
        },
        grid: header,
        inputs: inputs_map,
        hrrr_source: "weather/index.json: HRRR u10, v10, t2m, rh2m, tcc per hour on the grid"
            .to_string(),
        seed: SEED,
        wind_limit: match config.limit {
            WindLimit::None => WindLimitPolicy::None,
            WindLimit::Rothermel => WindLimitPolicy::Rothermel,
            WindLimit::EffectiveWind => WindLimitPolicy::EffectiveWind,
        },
        members: MEMBERS,
        horizons_h: HORIZONS_H.to_vec(),
        outputs: cogs.iter().map(|(k, p)| (k.clone(), rel(p))).collect(),
    };
    let manifest_path = out_dir.join("manifest.json");
    manifest.write(&manifest_path)?;
    writeln!(out, "wrote {}", manifest_path.display())?;

    print!("{out}");
    let txt = out_dir.join("gate.txt");
    std::fs::write(&txt, &out).with_context(|| format!("writing {}", txt.display()))?;
    Ok(())
}
