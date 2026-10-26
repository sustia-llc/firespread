//! Phase 2 gate: a 200-member ensemble on the Hermits Peak/Calf Canyon grid,
//! ignited from the FEDS 2022-04-22T12:00 perimeter, with burn counts at 1,
//! 3, 6 and 12 h. Writes the 12 h burn probability as a COG and the printout
//! to `<out_dir>/gate.txt`.
//!
//! Usage: `cargo run --release -p firespread-ensemble --example phase2_gate --
//! <data_dir> <out_dir>`, where `<data_dir>` holds `grid/` and
//! `obs/feds/20220422T1200.tif`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use firespread_ensemble::{Inputs, MemberBuffers, MemberParams, Perturbation, run_ensemble};
use firespread_fuel::Moistures;
use firespread_grid::{Grid, write_cog};
use firespread_rothermel::WindLimit;
use firespread_spread::Workspace;

/// Scott and Burgan (2005) D1/L1: dead 3/4/5 %, live herb 30 %, live woody 60 %.
const MOIST: Moistures = Moistures {
    dead_1h: 0.03,
    dead_10h: 0.04,
    dead_100h: 0.05,
    live_herb: 0.30,
    live_woody: 0.60,
};
const THREADS: usize = 12;
const MEMBERS: u32 = 200;
const SEED: u64 = 7;
const HORIZONS_H: [f32; 4] = [1.0, 3.0, 6.0, 12.0];
const SINGLE_THREAD_MEMBERS: u32 = 3;

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
        bail!("usage: phase2_gate <data_dir> <out_dir>");
    };
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let mut out = String::new();

    rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build_global()
        .context("building the rayon global pool")?;
    writeln!(out, "rayon threads: {}", rayon::current_num_threads())?;

    let perturbation = Perturbation {
        base: MemberParams {
            midflame_wind_m_s: 6.0,
            wind_toward_azimuth_rad: 45_f64.to_radians(),
        },
        speed_sd_m_s: 1.0,
        direction_sd_rad: 15_f64.to_radians(),
    };
    writeln!(
        out,
        "members {MEMBERS}, seed {SEED}, horizons {HORIZONS_H:?} h, WindLimit::None, \
         wind 6.0 m/s toward 45 deg, sd 1.0 m/s and 15 deg, moistures {MOIST:?}"
    )?;

    let t = Instant::now();
    let inputs = Inputs::load(
        &data_dir.join("grid"),
        &data_dir.join("obs/feds/20220422T1200.tif"),
        &MOIST,
        WindLimit::None,
    )?;
    let load_s = t.elapsed().as_secs_f64();
    let header = *inputs.header();
    let cell_km2 = header.dx * header.dy / 1e6;
    let burnable = inputs.burnable().iter().filter(|&&b| b).count();
    let ignited = inputs
        .initial_arrival_s()
        .iter()
        .filter(|t| t.is_finite())
        .count();
    writeln!(
        out,
        "grid {}x{} dx {} m: {} cells, {burnable} burnable, {ignited} ignited ({:.3} km2)",
        header.nx,
        header.ny,
        header.dx,
        inputs.len(),
        ignited as f64 * cell_km2
    )?;
    writeln!(
        out,
        "input load (4 grid rasters + ignition, 40 DirectionalModels, per-cell terrain): {load_s:.3} s"
    )?;

    let horizons_s: Vec<f32> = HORIZONS_H.iter().map(|h| h * 3600.0).collect();
    let horizon_max = horizons_s[horizons_s.len() - 1];

    let mut workspace = Workspace::new();
    let mut buffers = MemberBuffers::new(&inputs);
    let (mut fields_s, mut spread_s, mut total_s) = (Vec::new(), Vec::new(), Vec::new());
    for member in 0..SINGLE_THREAD_MEMBERS {
        let params = perturbation.draw(SEED, member)?;
        let t = Instant::now();
        buffers.fill_fields(&inputs, &params)?;
        let f = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let stats = buffers.spread(&inputs, &mut workspace, horizon_max)?;
        let s = t.elapsed().as_secs_f64();
        writeln!(
            out,
            "single-thread member {member}: wind {:.3} m/s toward {:.2} deg; fields {f:.3} s, \
             spread {s:.3} s, total {:.3} s; expanded {} cells",
            params.midflame_wind_m_s,
            params.wind_toward_azimuth_rad.to_degrees(),
            f + s,
            stats.expanded
        )?;
        fields_s.push(f);
        spread_s.push(s);
        total_s.push(f + s);
    }
    writeln!(
        out,
        "single-thread per-member median: total {:.3} s (field precompute {:.3} s, spread {:.3} s)",
        median(total_s),
        median(fields_s),
        median(spread_s)
    )?;

    let t = Instant::now();
    let counts = run_ensemble(&inputs, &perturbation, SEED, MEMBERS, &horizons_s)?;
    let gate_s = t.elapsed().as_secs_f64();
    writeln!(
        out,
        "GATE run_ensemble {MEMBERS} members on {} threads: {gate_s:.3} s",
        rayon::current_num_threads()
    )?;

    let k12 = horizons_s.len() - 1;
    let half = counts.counts[k12]
        .iter()
        .filter(|&&c| 2 * u32::from(c) >= MEMBERS)
        .count();
    writeln!(
        out,
        "12 h burn probability >= 0.5: {half} cells, {:.3} km2",
        half as f64 * cell_km2
    )?;
    let m0 = counts.summaries[0].burned_cells[k12];
    writeln!(
        out,
        "member 0 burned by 12 h: {m0} cells, {:.3} km2",
        m0 as f64 * cell_km2
    )?;
    for (k, h) in HORIZONS_H.iter().enumerate() {
        let any = counts.counts[k].iter().filter(|&&c| c > 0).count();
        writeln!(
            out,
            "{h} h: cells burned by any member {any} ({:.3} km2)",
            any as f64 * cell_km2
        )?;
    }
    let expanded = median(counts.summaries.iter().map(|s| s.expanded as f64).collect());
    writeln!(out, "cells expanded per member, median: {expanded}")?;

    let probability = counts
        .probability(k12)
        .context("12 h horizon is out of range")?;
    let cog = out_dir.join("p12h.tif");
    let t = Instant::now();
    write_cog(&cog, &Grid::new(header, probability)?)?;
    writeln!(
        out,
        "wrote {} in {:.3} s",
        cog.display(),
        t.elapsed().as_secs_f64()
    )?;

    print!("{out}");
    let txt = out_dir.join("gate.txt");
    std::fs::write(&txt, &out).with_context(|| format!("writing {}", txt.display()))?;
    Ok(())
}
