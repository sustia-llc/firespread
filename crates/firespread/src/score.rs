//! The `score` command: the scorecard, the burned area by hour and the
//! observed perimeter of one run.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use firespread_ensemble::{Fbfm40, classify_fbfm40};
use firespread_grid::{
    GridHeader, Manifest, MaskGrid, read_geotiff, read_geotiff_u8, write_cog_u8,
};
use firespread_metrics::{
    Mask, MetricsError, ProbabilityGrid, brier, dice, fit_uniform_growth_radius, jaccard,
    persistence, precision, recall, uniform_growth,
};
use serde::Serialize;

use crate::ensemble::{MEMBERS_FILE, MembersFile, read_json, write_json};
use crate::settings::Window;
use crate::timefmt::{feds_label, format_hour_utc, parse_hour};

/// A cell is in the forecast mask when its burn probability is at least this.
pub const FORECAST_THRESHOLD: f32 = 0.5;
/// Hours from the fit's earlier FEDS mask to the window start.
pub const FIT_LAG_H: i64 = 12;
/// Bisection tolerance of the fitted uniform-growth radius, m.
pub const FIT_TOLERANCE_M: f64 = 1.0;
/// Default radius of the Brier domain around the start perimeter, m.
pub const DEFAULT_BRIER_RADIUS_M: f64 = 5000.0;

/// Confusion code of a cell in neither the forecast nor the target mask.
pub const CONFUSION_NONE: u8 = 0;
/// Confusion code of a cell in both masks.
pub const CONFUSION_HIT: u8 = 1;
/// Confusion code of a cell in the target mask only.
pub const CONFUSION_MISS: u8 = 2;
/// Confusion code of a cell in the forecast mask only.
pub const CONFUSION_FALSE_ALARM: u8 = 3;
/// GDAL_NODATA of `confusion_h<N>.tif`; no cell carries it.
pub const CONFUSION_NODATA: f32 = 255.0;

const ENTRANTS: [&str; 3] = ["firespread", "persistence", "uniform_growth"];

/// The observed rasters a run is scored against, on the run's grid.
#[derive(Debug, Clone)]
pub struct Observations {
    pub header: GridHeader,
    /// Hours since 1970-01-01T00:00 UTC of the window start (the start label).
    pub start_h: i64,
    /// Hours since 1970-01-01T00:00 UTC of the observation (the target label).
    pub target_h: i64,
    /// FEDS mask at the start label, P(t0).
    pub start: Mask,
    /// FEDS mask at the start label minus [`FIT_LAG_H`], P(t0 − 12 h).
    pub previous: Mask,
    /// FEDS mask at the target label.
    pub target: Mask,
    /// WFIGS final perimeter.
    pub wfigs: Mask,
    /// FBFM40 burnable (one of the 40 models) and water code 0.
    pub burnable: Mask,
    /// GeoJSON of the target perimeter, copied to the run as `observed.geojson`.
    pub observed_geojson: Option<PathBuf>,
}

/// Options of `firespread score`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreOptions {
    /// Uniform-growth rate, m/h; `None` fits it.
    pub uniform_rate_m_per_h: Option<f64>,
    /// Radius of the Brier domain around the start perimeter, m.
    pub brier_radius_m: f64,
}

fn mask_of(h: &GridHeader, data: &[u8]) -> Result<Mask> {
    Ok(Mask::new(
        h.ny,
        h.nx,
        data.iter().map(|&v| v == 1).collect(),
    )?)
}

fn geometry(h: &GridHeader) -> GridHeader {
    GridHeader { nodata: 0.0, ..*h }
}

fn read_mask(path: &Path, header: &GridHeader) -> Result<Mask> {
    let grid = read_geotiff_u8(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        geometry(&grid.header).same_grid(&geometry(header)),
        "{} is not on the run's grid",
        path.display()
    );
    mask_of(header, &grid.data)
}

/// Reads the observations of `window` from `data` on the grid `header`:
/// `obs/feds/<label>.tif` at the start label, 12 h before it and the target
/// label, `obs/wfigs_final.tif`, the burnable mask from `grid/FBFM40.tif` and
/// `grid/water.tif`, and `obs/feds_geojson/<target label>.geojson`.
pub fn load_observations(
    data: &Path,
    window: &Window,
    header: &GridHeader,
) -> Result<Observations> {
    let start_h = parse_hour(&window.label_start)?;
    let target_h = parse_hour(&window.label_target)?;
    let feds = |h: i64| data.join(format!("obs/feds/{}.tif", feds_label(h)));
    let fbfm40_path = data.join("grid/FBFM40.tif");
    let fbfm40 =
        read_geotiff(&fbfm40_path).with_context(|| format!("reading {}", fbfm40_path.display()))?;
    ensure!(
        geometry(&fbfm40.header).same_grid(&geometry(header)),
        "{} is not on the run's grid",
        fbfm40_path.display()
    );
    let water_path = data.join("grid/water.tif");
    let water_grid = read_geotiff_u8(&water_path)
        .with_context(|| format!("reading {}", water_path.display()))?;
    ensure!(
        geometry(&water_grid.header).same_grid(&geometry(header)),
        "{} is not on the run's grid",
        water_path.display()
    );
    let nodata = fbfm40.header.nodata.to_bits();
    let burnable = fbfm40
        .data
        .iter()
        .zip(&water_grid.data)
        .map(|(&code, &w)| {
            w == 0
                && code.to_bits() != nodata
                && code.fract() == 0.0
                && (0.0..=f32::from(u16::MAX)).contains(&code)
                // `as u16` is exact: the code is integral and inside u16 range.
                && matches!(classify_fbfm40(code as u16), Some(Fbfm40::Burnable(_)))
        })
        .collect();
    let geojson = data.join(format!("obs/feds_geojson/{}.geojson", feds_label(target_h)));
    ensure!(
        geojson.is_file(),
        "{} is missing; run `python -m firespread_prep.feds_geojson`",
        geojson.display()
    );
    Ok(Observations {
        header: *header,
        start_h,
        target_h,
        start: read_mask(&feds(start_h), header)?,
        previous: read_mask(&feds(start_h - FIT_LAG_H), header)?,
        target: read_mask(&feds(target_h), header)?,
        wfigs: read_mask(&data.join("obs/wfigs_final.tif"), header)?,
        burnable: Mask::new(header.ny, header.nx, burnable)?,
        observed_geojson: Some(geojson),
    })
}

/// One scorecard row; a metric that does not apply, or whose denominator is
/// empty, is `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub entrant: &'static str,
    pub horizon_h: u32,
    /// `feds` or `wfigs_final`.
    pub observation: &'static str,
    pub jaccard: Option<f64>,
    pub dice: Option<f64>,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
    pub brier: Option<f64>,
}

/// How the uniform-growth rate was set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UniformGrowth {
    /// `fitted` or `option`.
    pub source: &'static str,
    pub rate_m_per_h: f64,
    /// Fitted radius R over [`FIT_LAG_H`] hours, m; `null` when given.
    pub fitted_radius_m: Option<f64>,
    /// FEDS labels the fit grows from and to.
    pub fit_from: String,
    pub fit_to: String,
}

/// The Brier domain.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrierDomain {
    pub radius_m: f64,
    pub cells: usize,
}

/// `scorecard.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scorecard {
    pub run_id: String,
    pub window: String,
    pub observed: String,
    pub forecast_threshold: f32,
    pub uniform_growth: UniformGrowth,
    pub brier_domain: BrierDomain,
    pub rows: Vec<Row>,
}

/// One observed area point of `area_by_hour.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservedArea {
    pub t: String,
    pub area_km2: f64,
}

/// Ensemble percentiles of `area_by_hour.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnsembleArea {
    pub p10: Vec<f64>,
    pub p50: Vec<f64>,
    pub p90: Vec<f64>,
}

/// `area_by_hour.json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AreaByHour {
    pub run_id: String,
    pub horizons_h: Vec<u32>,
    pub hours_utc: Vec<String>,
    pub ensemble_km2: EnsembleArea,
    pub observed: Vec<ObservedArea>,
}

/// What [`score_run`] wrote and computed.
#[derive(Debug, Clone)]
pub struct ScoreOutputs {
    pub scorecard: Scorecard,
    pub files: Vec<PathBuf>,
}

/// The `q` quantile of sorted `v` by linear interpolation between order
/// statistics at position `q (n − 1)`; `v` is non-empty.
fn quantile(v: &[f64], q: f64) -> f64 {
    let pos = q * (v.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    v[lo] + (pos - lo as f64) * (v[hi] - v[lo])
}

fn optional(r: firespread_metrics::Result<f64>) -> Result<Option<f64>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(MetricsError::EmptyPerimeters { .. }) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The confusion code of each cell of `forecast` against `target`, row-major:
/// [`CONFUSION_NONE`], [`CONFUSION_HIT`], [`CONFUSION_MISS`] or
/// [`CONFUSION_FALSE_ALARM`]. Fails when the masks differ in shape.
pub fn confusion_codes(forecast: &Mask, target: &Mask) -> Result<Vec<u8>> {
    ensure!(
        forecast.rows() == target.rows() && forecast.cols() == target.cols(),
        "forecast mask is {} x {}, target mask is {} x {}",
        forecast.rows(),
        forecast.cols(),
        target.rows(),
        target.cols()
    );
    Ok(forecast
        .cells()
        .iter()
        .zip(target.cells())
        .map(|(&f, &t)| match (f, t) {
            (false, false) => CONFUSION_NONE,
            (true, true) => CONFUSION_HIT,
            (false, true) => CONFUSION_MISS,
            (true, false) => CONFUSION_FALSE_ALARM,
        })
        .collect())
}

fn probability_of(mask: &Mask) -> Result<ProbabilityGrid> {
    Ok(ProbabilityGrid::new(
        mask.rows(),
        mask.cols(),
        mask.cells()
            .iter()
            .map(|&c| f32::from(u8::from(c)))
            .collect(),
    )?)
}

/// Scores the run in `run_dir` against `obs` and writes `scorecard.json`,
/// `area_by_hour.json`, `confusion_h<N>.tif` and, when `obs.observed_geojson`
/// is set, a copy of it as `observed.geojson` into `run_dir`.
///
/// The scored horizon is the target label minus the start label, which must
/// be one of the manifest's horizons. Entrants at that horizon, against the
/// FEDS target mask: `firespread`, the cells of `burn_probability_h<N>.tif`
/// with p ≥ [`FORECAST_THRESHOLD`], Brier on p; `persistence`, the start
/// mask; `uniform_growth`, the start mask grown at the rate of `opts` or the
/// rate fitted by [`fit_uniform_growth_radius`] from P(t0 − 12 h) to the
/// size of P(t0), divided by 12 h. The baselines' Brier takes p ∈ {0, 1}.
/// The Brier domain is the burnable cells within `opts.brier_radius_m` of
/// the start mask. Each entrant also gets a `wfigs_final` row with only
/// `precision`: its forecast cells inside the WFIGS final perimeter over its
/// forecast cells. `confusion_h<N>.tif` is a `u8` GeoTIFF on the run's grid
/// with GDAL_NODATA [`CONFUSION_NODATA`], holding [`confusion_codes`] of the
/// `firespread` forecast mask against the FEDS target mask.
pub fn score_run(run_dir: &Path, obs: &Observations, opts: &ScoreOptions) -> Result<ScoreOutputs> {
    let manifest = Manifest::read(run_dir.join("manifest.json"))?;
    let header = manifest.grid;
    ensure!(
        geometry(&header).same_grid(&geometry(&obs.header)),
        "the run's grid differs from the observations' grid"
    );
    ensure!(
        header.dx.to_bits() == header.dy.to_bits(),
        "cells are {} x {}, expected square cells",
        header.dx,
        header.dy
    );
    let dx = header.dx;
    let horizon = u32::try_from(obs.target_h - obs.start_h)
        .ok()
        .filter(|h| manifest.horizons_h.contains(h))
        .with_context(|| {
            format!(
                "the observation is {} h after the start, not one of the run's horizons {:?}",
                obs.target_h - obs.start_h,
                manifest.horizons_h
            )
        })?;
    let key = format!("burn_probability_h{horizon}");
    let cog = run_dir.join(
        manifest
            .outputs
            .get(&key)
            .with_context(|| format!("the manifest has no output {key}"))?,
    );
    let p = read_geotiff(&cog).with_context(|| format!("reading {}", cog.display()))?;
    ensure!(
        geometry(&p.header).same_grid(&geometry(&header)),
        "{} is not on the run's grid",
        cog.display()
    );
    let probability = ProbabilityGrid::new(header.ny, header.nx, p.data)?;
    let forecast = Mask::new(
        header.ny,
        header.nx,
        probability
            .values()
            .iter()
            .map(|&v| v >= FORECAST_THRESHOLD)
            .collect(),
    )?;

    let (rate, uniform_growth_info) = match opts.uniform_rate_m_per_h {
        Some(rate) => (
            rate,
            UniformGrowth {
                source: "option",
                rate_m_per_h: rate,
                fitted_radius_m: None,
                fit_from: feds_label(obs.start_h - FIT_LAG_H),
                fit_to: feds_label(obs.start_h),
            },
        ),
        None => {
            let radius =
                fit_uniform_growth_radius(&obs.previous, obs.start.count(), dx, FIT_TOLERANCE_M)?;
            let rate = radius / FIT_LAG_H as f64;
            (
                rate,
                UniformGrowth {
                    source: "fitted",
                    rate_m_per_h: rate,
                    fitted_radius_m: Some(radius),
                    fit_from: feds_label(obs.start_h - FIT_LAG_H),
                    fit_to: feds_label(obs.start_h),
                },
            )
        }
    };
    let persisted = persistence(&obs.start);
    let grown = uniform_growth(&obs.start, dx, rate, f64::from(horizon))?;

    let near = uniform_growth(&obs.start, dx, opts.brier_radius_m, 1.0)?;
    let domain = Mask::new(
        header.ny,
        header.nx,
        near.cells()
            .iter()
            .zip(obs.burnable.cells())
            .map(|(&a, &b)| a && b)
            .collect(),
    )?;

    let entrants = [
        (ENTRANTS[0], &forecast, probability),
        (ENTRANTS[1], &persisted, probability_of(&persisted)?),
        (ENTRANTS[2], &grown, probability_of(&grown)?),
    ];
    let mut rows = Vec::new();
    for (entrant, mask, p) in &entrants {
        rows.push(Row {
            entrant,
            horizon_h: horizon,
            observation: "feds",
            jaccard: Some(jaccard(mask, &obs.target)?),
            dice: Some(dice(mask, &obs.target)?),
            precision: optional(precision(mask, &obs.target))?,
            recall: optional(recall(mask, &obs.target))?,
            brier: Some(brier(p, &obs.target, &domain)?),
        });
    }
    for (entrant, mask, _) in &entrants {
        rows.push(Row {
            entrant,
            horizon_h: horizon,
            observation: "wfigs_final",
            jaccard: None,
            dice: None,
            precision: optional(precision(mask, &obs.wfigs))?,
            recall: None,
            brier: None,
        });
    }
    let scorecard = Scorecard {
        run_id: manifest.run_id.clone(),
        window: manifest.window.name.clone(),
        observed: format!(
            "FEDS {} (fireids {:?}); WFIGS final perimeter",
            feds_label(obs.target_h),
            manifest.window.fireids
        ),
        forecast_threshold: FORECAST_THRESHOLD,
        uniform_growth: uniform_growth_info,
        brier_domain: BrierDomain {
            radius_m: opts.brier_radius_m,
            cells: domain.count(),
        },
        rows,
    };

    let members: MembersFile = read_json(&run_dir.join(MEMBERS_FILE))?;
    ensure!(
        members.horizons_h == manifest.horizons_h,
        "{MEMBERS_FILE} horizons {:?} differ from the manifest's {:?}",
        members.horizons_h,
        manifest.horizons_h
    );
    ensure!(
        !members.members.is_empty(),
        "{MEMBERS_FILE} lists no member"
    );
    let km2 = |cells: usize| cells as f64 * header.dx * header.dy / 1e6;
    let mut ensemble = EnsembleArea {
        p10: Vec::new(),
        p50: Vec::new(),
        p90: Vec::new(),
    };
    for k in 0..members.horizons_h.len() {
        let mut areas = members
            .members
            .iter()
            .map(|m| m.burned_cells.get(k).copied().map(km2))
            .collect::<Option<Vec<f64>>>()
            .with_context(|| format!("a member of {MEMBERS_FILE} lacks horizon {k}"))?;
        areas.sort_by(f64::total_cmp);
        ensemble.p10.push(quantile(&areas, 0.1));
        ensemble.p50.push(quantile(&areas, 0.5));
        ensemble.p90.push(quantile(&areas, 0.9));
    }
    let area = AreaByHour {
        run_id: manifest.run_id.clone(),
        horizons_h: manifest.horizons_h.clone(),
        hours_utc: manifest
            .horizons_h
            .iter()
            .map(|&h| format_hour_utc(obs.start_h + i64::from(h)))
            .collect(),
        ensemble_km2: ensemble,
        observed: vec![
            ObservedArea {
                t: format_hour_utc(obs.start_h),
                area_km2: km2(obs.start.count()),
            },
            ObservedArea {
                t: format_hour_utc(obs.target_h),
                area_km2: km2(obs.target.count()),
            },
        ],
    };

    let mut files = vec![
        run_dir.join("scorecard.json"),
        run_dir.join("area_by_hour.json"),
    ];
    write_json(&files[0], &scorecard)?;
    write_json(&files[1], &area)?;
    let confusion_path = run_dir.join(format!("confusion_h{horizon}.tif"));
    let confusion = MaskGrid::new(
        GridHeader {
            nodata: CONFUSION_NODATA,
            ..header
        },
        confusion_codes(&forecast, &obs.target)?,
    )?;
    write_cog_u8(&confusion_path, &confusion)
        .with_context(|| format!("writing {}", confusion_path.display()))?;
    files.push(confusion_path);
    if let Some(src) = &obs.observed_geojson {
        let dst = run_dir.join("observed.geojson");
        std::fs::copy(src, &dst)
            .with_context(|| format!("copying {} to {}", src.display(), dst.display()))?;
        files.push(dst);
    }
    Ok(ScoreOutputs { scorecard, files })
}

/// Reads the run's manifest, loads the observations of its window from
/// `data`, and runs [`score_run`].
pub fn score(run_dir: &Path, data: &Path, opts: &ScoreOptions) -> Result<ScoreOutputs> {
    if let Some(rate) = opts.uniform_rate_m_per_h
        && !(rate.is_finite() && rate >= 0.0)
    {
        bail!("--uniform-rate-m-per-h {rate} is not a finite value >= 0");
    }
    ensure!(
        opts.brier_radius_m.is_finite() && opts.brier_radius_m >= 0.0,
        "--brier-radius-m {} is not a finite value >= 0",
        opts.brier_radius_m
    );
    let manifest = Manifest::read(run_dir.join("manifest.json"))?;
    let window = crate::settings::load_window(data, &manifest.window.name)?;
    let obs = load_observations(data, &window, &manifest.grid)?;
    score_run(run_dir, &obs, opts)
}
