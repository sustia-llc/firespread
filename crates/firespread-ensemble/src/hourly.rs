//! Ensembles over a window of hourly weather: per-cell wind from 10 m wind
//! and the wind adjustment factor, per-cell dead fuel moisture from the
//! hour's temperature, humidity and cloud cover, and spread advanced hour by
//! hour.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};

use firespread_fuel::{Moistures, by_code, units};
use firespread_grid::{GridHeader, read_geotiff, read_geotiff_u8, write_cog};
use firespread_rothermel::{DeadMoistures, DirectionalEnvironment, MoistureModel, WindLimit};
use firespread_spread::{CellFields, FieldProvider, Workspace};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Normal, StandardUniform};
use rayon::prelude::*;

use crate::fbfm40::BURNABLE_FBFM40;
use crate::inputs::{Inputs, Layers, NON_BURNABLE};
use crate::moisture::{DeadFuelMoisture, mc1_fraction, mc10_fraction};
use crate::waf::{Canopy, WafRule};
use crate::weather::{HourFields, WindowWeather};
use crate::{EnsembleError, Result};

/// Seconds per hour of weather.
pub const S_PER_HOUR: f32 = 3600.0;

/// Number of burnable fuel models, the length of
/// [`HourlyConfig::spread_multiplier`].
pub const N_MODELS: usize = BURNABLE_FBFM40.len();

/// Settings of an hourly run that no member perturbs.
#[derive(Debug, Clone, PartialEq)]
pub struct HourlyConfig {
    /// Live herbaceous moisture, fraction.
    pub live_herb: f64,
    /// Live woody moisture, fraction.
    pub live_woody: f64,
    pub limit: WindLimit,
    pub waf_rule: WafRule,
    pub moisture: DeadFuelMoisture,
    /// Factor on the head rate of spread per entry of [`BURNABLE_FBFM40`].
    pub spread_multiplier: [f64; N_MODELS],
}

impl Default for HourlyConfig {
    /// Live herbaceous 30 % and live woody 60 % (the Scott and Burgan 2005
    /// L1 scenario of the Phase 2 gate), [`WindLimit::None`], the default
    /// [`WafRule`] and [`DeadFuelMoisture`], and spread multiplier 1 for every
    /// fuel model.
    fn default() -> Self {
        Self {
            live_herb: 0.30,
            live_woody: 0.60,
            limit: WindLimit::None,
            waf_rule: WafRule::default(),
            moisture: DeadFuelMoisture::default(),
            spread_multiplier: [1.0; N_MODELS],
        }
    }
}

/// Raster layers of an hourly run on one grid, row-major with the north row
/// first.
#[derive(Debug, Clone, Copy)]
pub struct HourlyLayers<'a> {
    /// Fuel, terrain, water and the window's start mask (`ignited`).
    pub base: Layers<'a>,
    /// LANDFIRE canopy height CH, m.
    pub canopy_height_m: &'a [f32],
    /// LANDFIRE canopy base height CBH, m.
    pub canopy_base_height_m: &'a [f32],
    /// LANDFIRE canopy cover CC, percent.
    pub canopy_cover_pct: &'a [f32],
}

/// Inputs shared by every member of an hourly run: fuel and terrain as
/// [`Inputs`] builds them, one [`MoistureModel`] per entry of
/// [`BURNABLE_FBFM40`], the WAF of each burnable cell, the cells of the
/// start mask, and the window's weather.
#[derive(Debug, Clone)]
pub struct HourlyInputs {
    inputs: Inputs,
    models: Vec<MoistureModel>,
    waf: Vec<f32>,
    ignition: Vec<(usize, usize)>,
    weather: WindowWeather,
    spread_multiplier: [f64; N_MODELS],
}

fn cell_error(index: usize, e: EnsembleError) -> EnsembleError {
    match e {
        EnsembleError::InvalidCanopy {
            field,
            bound,
            value,
        } => EnsembleError::InvalidCell {
            layer: field,
            bound,
            index,
            value,
        },
        other => other,
    }
}

impl HourlyInputs {
    /// Builds the inputs from in-memory layers and weather. A cell of
    /// `layers.base.ignited` is in the start mask when it holds 1. The WAF of
    /// a burnable cell is `config.waf_rule` at its fuel model's bed depth
    /// and its canopy.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidSpreadMultiplier`], the errors of
    /// [`Inputs::from_layers`], [`EnsembleError::LayerLength`] when a canopy
    /// or weather layer does not hold one value per cell,
    /// [`EnsembleError::InvalidCell`] for an invalid canopy value or a
    /// negative 100-h moisture at a burnable cell, and the fuel and
    /// Rothermel errors of building the moisture models.
    pub fn from_layers(
        layers: &HourlyLayers<'_>,
        weather: WindowWeather,
        config: &HourlyConfig,
    ) -> Result<Self> {
        for (&(_, model), &value) in BURNABLE_FBFM40.iter().zip(&config.spread_multiplier) {
            if !(value.is_finite() && value >= 0.0) {
                return Err(EnsembleError::InvalidSpreadMultiplier { model, value });
            }
        }
        let reference = Moistures {
            dead_1h: 0.03,
            dead_10h: 0.04,
            dead_100h: 0.05,
            live_herb: config.live_herb,
            live_woody: config.live_woody,
        };
        let inputs = Inputs::from_layers(&layers.base, &reference, config.limit)?;
        let n = inputs.len();
        let check = |layer, got| {
            if got == n {
                Ok(())
            } else {
                Err(EnsembleError::LayerLength {
                    layer,
                    expected: n,
                    got,
                })
            }
        };
        check("canopy_height_m", layers.canopy_height_m.len())?;
        check("canopy_base_height_m", layers.canopy_base_height_m.len())?;
        check("canopy_cover_pct", layers.canopy_cover_pct.len())?;
        check("fm100", weather.fm100_pct.len())?;
        check("fm1000", weather.fm1000_pct.len())?;
        for hour in &weather.hours {
            check("u20ft", hour.u20ft_m_s.len())?;
            check("wind azimuth", hour.wind_toward_azimuth_rad.len())?;
            check("emcprm", hour.emcprm_pct.len())?;
        }

        let mut models = Vec::with_capacity(N_MODELS);
        let mut depth_m = Vec::with_capacity(N_MODELS);
        for &(_, code) in &BURNABLE_FBFM40 {
            let si = by_code(code)?.to_si();
            models.push(MoistureModel::new(&si, &reference, config.limit)?);
            depth_m.push(si.depth_m);
        }

        let mut waf = vec![0.0_f32; n];
        for (index, w) in waf.iter_mut().enumerate() {
            let m = inputs.model_of_cell[index];
            if m == NON_BURNABLE {
                continue;
            }
            let canopy = Canopy {
                height_m: f64::from(layers.canopy_height_m[index]),
                base_height_m: f64::from(layers.canopy_base_height_m[index]),
                cover_pct: f64::from(layers.canopy_cover_pct[index]),
            };
            *w = config
                .waf_rule
                .waf(depth_m[usize::from(m)], &canopy)
                .map_err(|e| cell_error(index, e))? as f32;
            let fm100 = weather.fm100_pct[index];
            if !(fm100.is_finite() && fm100 >= 0.0) {
                return Err(EnsembleError::InvalidCell {
                    layer: "fm100",
                    bound: "a finite value >= 0",
                    index,
                    value: f64::from(fm100),
                });
            }
        }

        let nx = inputs.grid().nx();
        let ignition = layers
            .base
            .ignited
            .iter()
            .enumerate()
            .filter(|(_, v)| **v == 1)
            .map(|(i, _)| (i / nx, i % nx))
            .collect();

        Ok(Self {
            inputs,
            models,
            waf,
            ignition,
            weather,
            spread_multiplier: config.spread_multiplier,
        })
    }

    /// Loads `FBFM40.tif`, `SlpD.tif`, `Asp.tif`, `CH.tif`, `CBH.tif`,
    /// `CC.tif` (f32) and `water.tif` (u8) from `grid_dir`, the u8 start mask
    /// at `ignition`, and the first `hours` hours of window `window` with
    /// [`WindowWeather::load`], and builds the inputs as
    /// [`HourlyInputs::from_layers`] does.
    ///
    /// # Errors
    /// The read errors of each raster, [`EnsembleError::LayerMismatch`] when
    /// a layer's geometry differs from FBFM40's, the errors of
    /// [`WindowWeather::load`], and the errors of
    /// [`HourlyInputs::from_layers`].
    pub fn load(
        grid_dir: &Path,
        weather_dir: &Path,
        window: &str,
        ignition: &Path,
        hours: usize,
        config: &HourlyConfig,
    ) -> Result<Self> {
        let fbfm40 = read_geotiff(grid_dir.join("FBFM40.tif"))?;
        let slope = read_geotiff(grid_dir.join("SlpD.tif"))?;
        let aspect = read_geotiff(grid_dir.join("Asp.tif"))?;
        let ch = read_geotiff(grid_dir.join("CH.tif"))?;
        let cbh = read_geotiff(grid_dir.join("CBH.tif"))?;
        let cc = read_geotiff(grid_dir.join("CC.tif"))?;
        let water = read_geotiff_u8(grid_dir.join("water.tif"))?;
        let ignited = read_geotiff_u8(ignition)?;
        let geometry = |h: &GridHeader| GridHeader { nodata: 0.0, ..*h };
        for (layer, h) in [
            ("SlpD", &slope.header),
            ("Asp", &aspect.header),
            ("CH", &ch.header),
            ("CBH", &cbh.header),
            ("CC", &cc.header),
            ("water", &water.header),
            ("ignition", &ignited.header),
        ] {
            if !geometry(&fbfm40.header).same_grid(&geometry(h)) {
                return Err(EnsembleError::LayerMismatch { layer });
            }
        }
        let weather =
            WindowWeather::load(weather_dir, window, hours, &fbfm40.header, &config.moisture)?;
        Self::from_layers(
            &HourlyLayers {
                base: Layers {
                    header: fbfm40.header,
                    fbfm40: &fbfm40.data,
                    slope_deg: &slope.data,
                    aspect_deg: &aspect.data,
                    water: &water.data,
                    ignited: &ignited.data,
                },
                canopy_height_m: &ch.data,
                canopy_base_height_m: &cbh.data,
                canopy_cover_pct: &cc.data,
            },
            weather,
            config,
        )
    }

    /// The static inputs: grid, fuel models, terrain and burnable mask.
    #[must_use]
    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// The window's weather.
    #[must_use]
    pub fn weather(&self) -> &WindowWeather {
        &self.weather
    }

    /// WAF per cell; 0 at non-burnable cells.
    #[must_use]
    pub fn waf(&self) -> &[f32] {
        &self.waf
    }

    /// Cells `(row, col)` of the start mask, in index order.
    #[must_use]
    pub fn ignition(&self) -> &[(usize, usize)] {
        &self.ignition
    }

    /// The fields of cell `cell` in hour `hour` for a member with draws
    /// `draw`, all 0 at a non-burnable cell. At a burnable cell of fuel
    /// model `m`: midflame wind `u20ft · WAF · draw.wind_speed_factor`
    /// toward `azimuth + draw.wind_direction_offset_rad`; dead 1-h and 10-h
    /// moisture [`mc1_fraction`] and [`mc10_fraction`] of the hour's EMCPRM,
    /// each times `draw.dead_moisture_factor`; dead 100-h moisture the
    /// gridMET 100-h moisture; the head rate of [`MoistureModel::at`] then
    /// `eval` times the spread multiplier of `m`, with the eccentricity and
    /// head azimuth of the same evaluation, each cast to `f32`.
    ///
    /// # Errors
    /// The Rothermel errors of the evaluation.
    ///
    /// # Panics
    /// When `hour` is not below the number of weather hours or `cell` is not
    /// below the number of cells.
    pub fn cell_fields(&self, hour: usize, draw: &MemberDraw, cell: usize) -> Result<CellFields> {
        HourProvider {
            inputs: self,
            hour: &self.weather.hours[hour],
            draw,
        }
        .eval(cell)
    }
}

/// The per-member perturbations of an hourly run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HourlyPerturbation {
    /// Radius of the disc the start-mask offset is uniform in, m.
    pub ignition_radius_m: f64,
    /// Standard deviation of the wind speed factor N(1, sd), truncated at 0.
    pub wind_speed_sd: f64,
    /// Standard deviation of the wind direction offset N(0, sd), rad.
    pub wind_direction_sd_rad: f64,
    /// Standard deviation of the log of the dead moisture factor
    /// exp(N(0, sd)).
    pub dead_moisture_log_sd: f64,
}

impl Default for HourlyPerturbation {
    /// Radius 375 m, speed sd 0.2, direction sd 15°, log-moisture sd 0.1.
    fn default() -> Self {
        Self {
            ignition_radius_m: 375.0,
            wind_speed_sd: 0.2,
            wind_direction_sd_rad: 15_f64.to_radians(),
            dead_moisture_log_sd: 0.1,
        }
    }
}

/// What one member drew, held over all hours of the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberDraw {
    /// East offset of the start mask, m.
    pub ignition_east_m: f64,
    /// North offset of the start mask, m.
    pub ignition_north_m: f64,
    /// The offset in cells: `-round(north / dx)` rows and `round(east / dx)`
    /// columns.
    pub ignition_shift_rows: i64,
    pub ignition_shift_cols: i64,
    /// Factor on the midflame wind speed, >= 0.
    pub wind_speed_factor: f64,
    /// Offset added to the wind's toward azimuth, rad.
    pub wind_direction_offset_rad: f64,
    /// Factor on the dead 1-h and 10-h moistures.
    pub dead_moisture_factor: f64,
}

fn non_negative(field: &'static str, value: f64) -> Result<f64> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(EnsembleError::InvalidPerturbation {
            field,
            bound: "a finite value >= 0",
            value,
        })
    }
}

impl HourlyPerturbation {
    /// The draws of member `member` of the ensemble seeded with `seed` on a
    /// grid of cell side `dx_m`. A `ChaCha8Rng` seeded by
    /// `seed_from_u64(seed)` on stream `member` draws, in this order:
    /// 1. `a ~ U[0, 1)`; 2. `b ~ U[0, 1)`: the start-mask offset is
    ///    `r = radius √a` along azimuth `θ = 2π b` (clockwise from north),
    ///    east `r sin θ`, north `r cos θ`;
    /// 3. the wind speed factor `max(N(1, wind_speed_sd), 0)`;
    /// 4. the wind direction offset `N(0, wind_direction_sd_rad)`;
    /// 5. the dead moisture factor `exp(N(0, dead_moisture_log_sd))`.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidPerturbation`] when a radius or standard
    /// deviation is negative or not finite, or `dx_m` is not finite and > 0.
    pub fn draw(&self, seed: u64, member: u32, dx_m: f64) -> Result<MemberDraw> {
        let radius = non_negative("ignition radius", self.ignition_radius_m)?;
        let speed_sd = non_negative("wind speed sd", self.wind_speed_sd)?;
        let direction_sd = non_negative("wind direction sd", self.wind_direction_sd_rad)?;
        let log_sd = non_negative("dead moisture log sd", self.dead_moisture_log_sd)?;
        if !(dx_m.is_finite() && dx_m > 0.0) {
            return Err(EnsembleError::InvalidPerturbation {
                field: "cell size",
                bound: "a finite value > 0",
                value: dx_m,
            });
        }
        let normal = |field, mean, sd| {
            Normal::new(mean, sd).map_err(|_| EnsembleError::InvalidPerturbation {
                field,
                bound: "a finite value >= 0",
                value: sd,
            })
        };
        let speed = normal("wind speed sd", 1.0, speed_sd)?;
        let direction = normal("wind direction sd", 0.0, direction_sd)?;
        let log_moisture = normal("dead moisture log sd", 0.0, log_sd)?;
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        rng.set_stream(u64::from(member));
        let a: f64 = StandardUniform.sample(&mut rng);
        let b: f64 = StandardUniform.sample(&mut rng);
        let s = speed.sample(&mut rng);
        let d = direction.sample(&mut rng);
        let m = log_moisture.sample(&mut rng);
        let r = radius * a.sqrt();
        let theta = core::f64::consts::TAU * b;
        let (east, north) = (r * theta.sin(), r * theta.cos());
        Ok(MemberDraw {
            ignition_east_m: east,
            ignition_north_m: north,
            // `as` saturates; |offset / dx| <= radius / dx is finite.
            ignition_shift_rows: -(north / dx_m).round() as i64,
            ignition_shift_cols: (east / dx_m).round() as i64,
            wind_speed_factor: s.max(0.0),
            wind_direction_offset_rad: d,
            dead_moisture_factor: m.exp(),
        })
    }
}

struct HourProvider<'a> {
    inputs: &'a HourlyInputs,
    hour: &'a HourFields,
    draw: &'a MemberDraw,
}

impl HourProvider<'_> {
    fn eval(&self, cell: usize) -> Result<CellFields> {
        let h = self.inputs;
        let m = h.inputs.model_of_cell[cell];
        if m == NON_BURNABLE {
            return Ok(CellFields::default());
        }
        let m = usize::from(m);
        let emc = f64::from(self.hour.emcprm_pct[cell]);
        let factor = self.draw.dead_moisture_factor;
        let model = h.models[m].at(&DeadMoistures {
            dead_1h: mc1_fraction(emc) * factor,
            dead_10h: mc10_fraction(emc) * factor,
            dead_100h: units::percent_to_fraction(f64::from(h.weather.fm100_pct[cell])),
        })?;
        let out = model.eval(&DirectionalEnvironment {
            midflame_wind_m_s: f64::from(self.hour.u20ft_m_s[cell])
                * f64::from(h.waf[cell])
                * self.draw.wind_speed_factor,
            wind_toward_azimuth_rad: f64::from(self.hour.wind_toward_azimuth_rad[cell])
                + self.draw.wind_direction_offset_rad,
            slope_tan: h.inputs.slope_tan[cell],
            upslope_azimuth_rad: h.inputs.upslope_azimuth_rad[cell],
        })?;
        Ok(CellFields {
            head_ros_m_s: (out.head_ros_m_s * h.spread_multiplier[m]) as f32,
            eccentricity: out.eccentricity as f32,
            max_azimuth_rad: out.max_spread_azimuth_rad as f32,
        })
    }
}

impl FieldProvider for HourProvider<'_> {
    type Error = EnsembleError;

    fn fields(&mut self, cell: usize) -> Result<CellFields> {
        self.eval(cell)
    }
}

/// Counts from one [`run_hourly_member`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HourlyStats {
    /// Start-mask cells inside the grid after the shift.
    pub ignited: usize,
    /// Cells expanded, summed over the hours.
    pub expanded: usize,
}

/// Runs one member to `horizon_h` hours. Sets `arrival_s` to one value per
/// cell: 0 at each start-mask cell shifted by the draw's
/// `(ignition_shift_rows, ignition_shift_cols)` that lands inside the grid,
/// `f32::INFINITY` elsewhere. Then, for each hour `k` below `horizon_h`,
/// runs one spread window `[3600 k, 3600 (k + 1)]` s on the arrival field the
/// previous hour left, with each expanded cell's fields from
/// [`HourlyInputs::cell_fields`] of hour `k`.
///
/// # Errors
/// [`EnsembleError::TooFewHours`] when the weather holds fewer than
/// `horizon_h` hours, and the errors of
/// [`Workspace::simulate_window_with`].
pub fn run_hourly_member(
    inputs: &HourlyInputs,
    draw: &MemberDraw,
    workspace: &mut Workspace,
    arrival_s: &mut Vec<f32>,
    horizon_h: u32,
) -> Result<HourlyStats> {
    let hours = usize::try_from(horizon_h).unwrap_or(usize::MAX);
    if hours > inputs.weather.hours.len() {
        return Err(EnsembleError::TooFewHours {
            available: inputs.weather.hours.len(),
            horizon_h,
        });
    }
    let grid = inputs.inputs.grid();
    let (nx, ny) = (grid.nx(), grid.ny());
    arrival_s.clear();
    arrival_s.resize(grid.len(), f32::INFINITY);
    let mut stats = HourlyStats::default();
    for &(row, col) in &inputs.ignition {
        let r = i64::try_from(row).unwrap_or(i64::MAX) + draw.ignition_shift_rows;
        let c = i64::try_from(col).unwrap_or(i64::MAX) + draw.ignition_shift_cols;
        if let (Ok(r), Ok(c)) = (usize::try_from(r), usize::try_from(c))
            && r < ny
            && c < nx
        {
            arrival_s[r * nx + c] = 0.0;
            stats.ignited += 1;
        }
    }
    for (k, hour) in inputs.weather.hours[..hours].iter().enumerate() {
        let t0 = k as f32 * S_PER_HOUR;
        let mut provider = HourProvider { inputs, hour, draw };
        let s = workspace.simulate_window_with(
            grid,
            inputs.inputs.burnable(),
            &mut provider,
            arrival_s,
            t0,
            t0 + S_PER_HOUR,
        )?;
        stats.expanded += s.expanded;
    }
    Ok(stats)
}

/// What one member of an hourly ensemble drew and burned.
#[derive(Debug, Clone, PartialEq)]
pub struct HourlyMemberSummary {
    pub draw: MemberDraw,
    pub stats: HourlyStats,
    /// Cells with arrival time `<= 3600 horizons_h[k]` s, per horizon `k`.
    pub burned_cells: Vec<usize>,
}

/// Burn counts of an hourly ensemble.
#[derive(Debug, Clone, PartialEq)]
pub struct HourlyCounts {
    pub members: u32,
    pub horizons_h: Vec<u32>,
    /// `counts[k][cell]`: members whose arrival time at `cell` is
    /// `<= 3600 horizons_h[k]` s.
    pub counts: Vec<Vec<u16>>,
    /// One summary per member, by member index.
    pub summaries: Vec<HourlyMemberSummary>,
}

impl HourlyCounts {
    /// Burn probability `counts[k][cell] / members` per cell for horizon
    /// `k`, or `None` when `k` is out of range.
    #[must_use]
    pub fn probability(&self, k: usize) -> Option<Vec<f32>> {
        let members = self.members as f32;
        self.counts
            .get(k)
            .map(|c| c.iter().map(|&n| f32::from(n) / members).collect())
    }
}

type Slot = (Workspace, Vec<f32>);

/// Runs `members` members on the current rayon pool. Member `i` takes the
/// draws `perturbation.draw(seed, i, dx)`, runs [`run_hourly_member`] to the
/// largest horizon, and adds 1 to `counts[k][cell]` for each horizon `k` with
/// arrival time at `cell <= 3600 horizons_h[k]` s. Counts are integer sums,
/// so the result does not depend on the pool's thread count or schedule.
///
/// # Errors
/// [`EnsembleError::Empty`] when `members` is 0,
/// [`EnsembleError::TooManyMembers`] above `u16::MAX`,
/// [`EnsembleError::NoHorizons`] for an empty horizon list,
/// [`EnsembleError::TooFewHours`] when the weather holds fewer hours than
/// the largest horizon, the errors of [`HourlyPerturbation::draw`], and
/// [`EnsembleError::MemberFailed`] wrapping the error of the lowest-indexed
/// failing member.
pub fn run_hourly_ensemble(
    inputs: &HourlyInputs,
    perturbation: &HourlyPerturbation,
    seed: u64,
    members: u32,
    horizons_h: &[u32],
) -> Result<HourlyCounts> {
    if members == 0 {
        return Err(EnsembleError::Empty);
    }
    let max = u32::from(u16::MAX);
    if members > max {
        return Err(EnsembleError::TooManyMembers { members, max });
    }
    let Some(&horizon_max) = horizons_h.iter().max() else {
        return Err(EnsembleError::NoHorizons);
    };
    if usize::try_from(horizon_max).unwrap_or(usize::MAX) > inputs.weather.hours.len() {
        return Err(EnsembleError::TooFewHours {
            available: inputs.weather.hours.len(),
            horizon_h: horizon_max,
        });
    }
    let dx = f64::from(inputs.inputs.grid().dx_m());
    perturbation.draw(seed, 0, dx)?;
    let horizons_s: Vec<f32> = horizons_h.iter().map(|&h| h as f32 * S_PER_HOUR).collect();
    let horizon_max_s = horizon_max as f32 * S_PER_HOUR;
    let n = inputs.inputs.len();
    let counts: Vec<AtomicU16> = (0..n * horizons_h.len())
        .map(|_| AtomicU16::new(0))
        .collect();
    let pool: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

    let run = |member: u32| -> Result<HourlyMemberSummary> {
        let draw = perturbation.draw(seed, member, dx)?;
        let slot = pool
            .lock()
            .map_err(|_| EnsembleError::BufferPoolPoisoned)?
            .pop();
        let (mut workspace, mut arrival) = slot.unwrap_or_else(|| (Workspace::new(), Vec::new()));
        let stats = run_hourly_member(inputs, &draw, &mut workspace, &mut arrival, horizon_max)?;
        let mut burned_cells = vec![0_usize; horizons_s.len()];
        for (cell, &t) in arrival.iter().enumerate() {
            if t > horizon_max_s {
                continue;
            }
            for (k, &h) in horizons_s.iter().enumerate() {
                if t <= h {
                    counts[k * n + cell].fetch_add(1, Ordering::Relaxed);
                    burned_cells[k] += 1;
                }
            }
        }
        pool.lock()
            .map_err(|_| EnsembleError::BufferPoolPoisoned)?
            .push((workspace, arrival));
        Ok(HourlyMemberSummary {
            draw,
            stats,
            burned_cells,
        })
    };

    let summaries = (0..members)
        .into_par_iter()
        .map(|member| {
            run(member).map_err(|e| EnsembleError::MemberFailed {
                member: member as usize,
                members: members as usize,
                reason: e.to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let flat: Vec<u16> = counts.into_iter().map(AtomicU16::into_inner).collect();
    Ok(HourlyCounts {
        members,
        horizons_h: horizons_h.to_vec(),
        counts: flat.chunks(n).map(<[u16]>::to_vec).collect(),
        summaries,
    })
}

/// Writes the burn probability of each horizon `h` of `counts` as a COG
/// `burn_probability_h<N>.tif` in `out_dir` on the grid of `header`, and
/// returns the paths keyed `burn_probability_h<N>`, with `N` the horizon in
/// hours.
///
/// # Errors
/// The grid errors of building and writing each COG.
pub fn write_probability_cogs(
    counts: &HourlyCounts,
    header: &GridHeader,
    out_dir: &Path,
) -> Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    for (k, h) in counts.horizons_h.iter().enumerate() {
        let probability = counts.probability(k).ok_or(EnsembleError::NoHorizons)?;
        let key = format!("burn_probability_h{h}");
        let path = out_dir.join(format!("{key}.tif"));
        write_cog(&path, &firespread_grid::Grid::new(*header, probability)?)?;
        out.insert(key, path);
    }
    Ok(out)
}
