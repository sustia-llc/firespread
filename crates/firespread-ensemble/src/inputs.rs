//! Static per-cell inputs of an ensemble run.

use std::path::Path;

use firespread_fuel::{Moistures, by_code};
use firespread_grid::{GridHeader, read_geotiff, read_geotiff_u8};
use firespread_rothermel::{DirectionalModel, FuelBed, WindLimit};
use firespread_spread::Grid;

use crate::fbfm40::{BURNABLE_FBFM40, Fbfm40, classify_fbfm40};
use crate::{EnsembleError, Result};

/// Model index of a non-burnable cell.
pub(crate) const NON_BURNABLE: u8 = u8::MAX;

/// Raster layers on one grid, row-major with the north row first.
#[derive(Debug, Clone, Copy)]
pub struct Layers<'a> {
    /// Geometry of every layer; `nodata` is the FBFM40 nodata value.
    pub header: GridHeader,
    /// LANDFIRE FBFM40 codes.
    pub fbfm40: &'a [f32],
    /// Slope, degrees.
    pub slope_deg: &'a [f32],
    /// Aspect, degrees clockwise from north of the downslope direction; -1 is flat.
    pub aspect_deg: &'a [f32],
    /// Water class; any code > 0 is non-burnable.
    pub water: &'a [u8],
    /// Burned at time 0 where the value is 1.
    pub ignited: &'a [u8],
}

/// Per-cell inputs shared by every member: the fuel model of each cell, the
/// terrain, the burnable mask, the arrival field at time 0, and one
/// [`DirectionalModel`] per entry of [`BURNABLE_FBFM40`] at the run's
/// moistures and wind limit.
///
/// A cell is non-burnable when its FBFM40 code is 91-99 or the FBFM40 nodata
/// value, or its water code is > 0. At a burnable cell, `slope_tan` is
/// `tan(slope_deg)` and the upslope azimuth is `(aspect_deg + 180) mod 360` in
/// radians; aspect -1 gives slope 0 and upslope azimuth 0.
#[derive(Debug, Clone)]
pub struct Inputs {
    header: GridHeader,
    grid: Grid,
    pub(crate) model_of_cell: Vec<u8>,
    pub(crate) burnable: Vec<bool>,
    pub(crate) slope_tan: Vec<f64>,
    pub(crate) upslope_azimuth_rad: Vec<f64>,
    pub(crate) initial_arrival_s: Vec<f32>,
    pub(crate) models: Vec<DirectionalModel>,
}

fn check_len(layer: &'static str, expected: usize, got: usize) -> Result<()> {
    if expected == got {
        Ok(())
    } else {
        Err(EnsembleError::LayerLength {
            layer,
            expected,
            got,
        })
    }
}

/// Water code [`apply_road_barriers`] writes at a barrier road cell.
pub const ROAD_BARRIER_WATER_CODE: u8 = u8::MAX;

/// Marks road cells as non-burnable through the water layer: at each cell
/// whose `water` code is 0 and whose `roads` code is in `codes`, sets `water`
/// to [`ROAD_BARRIER_WATER_CODE`]. Returns the number of cells changed. An
/// empty `codes` changes nothing.
///
/// # Errors
/// [`EnsembleError::LayerLength`] when `roads` and `water` differ in length.
pub fn apply_road_barriers(water: &mut [u8], roads: &[u8], codes: &[u8]) -> Result<usize> {
    check_len("roads", water.len(), roads.len())?;
    let mut changed = 0;
    for (w, r) in water.iter_mut().zip(roads) {
        if *w == 0 && codes.contains(r) {
            *w = ROAD_BARRIER_WATER_CODE;
            changed += 1;
        }
    }
    Ok(changed)
}

fn same_geometry(a: &GridHeader, b: &GridHeader) -> bool {
    GridHeader { nodata: 0.0, ..*a }.same_grid(&GridHeader { nodata: 0.0, ..*b })
}

impl Inputs {
    /// Builds the inputs from in-memory layers.
    ///
    /// # Errors
    /// [`EnsembleError::LayerLength`] when a layer does not hold `nx * ny`
    /// values, [`EnsembleError::NonSquareCells`] when `dx != dy`,
    /// [`EnsembleError::UnknownFuelCode`] for an FBFM40 code outside
    /// [`BURNABLE_FBFM40`], 91-99 and nodata,
    /// [`EnsembleError::InvalidTerrain`] when a burnable cell's slope is not in
    /// [0, 90) or its aspect is neither -1 nor in [0, 360], and the fuel,
    /// Rothermel and spread errors of building the models and grid.
    pub fn from_layers(
        layers: &Layers<'_>,
        moistures: &Moistures,
        limit: WindLimit,
    ) -> Result<Self> {
        let header = layers.header;
        if header.dx.to_bits() != header.dy.to_bits() {
            return Err(EnsembleError::NonSquareCells {
                dx: header.dx,
                dy: header.dy,
            });
        }
        let grid = Grid::new(header.nx, header.ny, header.dx as f32)?;
        let n = grid.len();
        check_len("fbfm40", n, layers.fbfm40.len())?;
        check_len("slope_deg", n, layers.slope_deg.len())?;
        check_len("aspect_deg", n, layers.aspect_deg.len())?;
        check_len("water", n, layers.water.len())?;
        check_len("ignited", n, layers.ignited.len())?;

        let models = BURNABLE_FBFM40
            .iter()
            .map(|&(_, code)| {
                let bed = FuelBed::from_model(&by_code(code)?.to_si(), moistures)?;
                Ok(DirectionalModel::new(&bed, limit)?)
            })
            .collect::<Result<Vec<_>>>()?;

        let mut model_of_cell = vec![NON_BURNABLE; n];
        let mut burnable = vec![false; n];
        let mut slope_tan = vec![0.0; n];
        let mut upslope_azimuth_rad = vec![0.0; n];
        for index in 0..n {
            let code = layers.fbfm40[index];
            if code.to_bits() == header.nodata.to_bits() || layers.water[index] > 0 {
                continue;
            }
            // `as u16` is exact: the code is integral and inside u16 range.
            let class = (code.fract() == 0.0 && (0.0..=f32::from(u16::MAX)).contains(&code))
                .then(|| classify_fbfm40(code as u16))
                .flatten()
                .ok_or(EnsembleError::UnknownFuelCode { index, code })?;
            let Fbfm40::Burnable(model) = class else {
                continue;
            };
            let slope = layers.slope_deg[index];
            if !(slope.is_finite() && (0.0..90.0).contains(&slope)) {
                return Err(EnsembleError::InvalidTerrain {
                    layer: "slope_deg",
                    bound: "a value in [0, 90)",
                    index,
                    value: slope,
                });
            }
            let aspect = layers.aspect_deg[index];
            if aspect == -1.0 {
                // Flat: slope 0, upslope azimuth 0.
            } else if aspect.is_finite() && (0.0..=360.0).contains(&aspect) {
                slope_tan[index] = f64::from(slope).to_radians().tan();
                upslope_azimuth_rad[index] = ((f64::from(aspect) + 180.0) % 360.0).to_radians();
            } else {
                return Err(EnsembleError::InvalidTerrain {
                    layer: "aspect_deg",
                    bound: "-1 or a value in [0, 360]",
                    index,
                    value: aspect,
                });
            }
            // `as u8` is exact: BURNABLE_FBFM40 has 40 entries.
            model_of_cell[index] = model as u8;
            burnable[index] = true;
        }

        let initial_arrival_s = layers
            .ignited
            .iter()
            .map(|&v| if v == 1 { 0.0 } else { f32::INFINITY })
            .collect();

        Ok(Self {
            header,
            grid,
            model_of_cell,
            burnable,
            slope_tan,
            upslope_azimuth_rad,
            initial_arrival_s,
            models,
        })
    }

    /// Loads `FBFM40.tif`, `SlpD.tif`, `Asp.tif` (f32) and `water.tif` (u8)
    /// from `grid_dir` and the u8 ignition raster at `ignition`, and builds the
    /// inputs as [`Inputs::from_layers`] does.
    ///
    /// # Errors
    /// The read errors of each raster, [`EnsembleError::LayerMismatch`] when a
    /// layer's geometry differs from FBFM40's, and the errors of
    /// [`Inputs::from_layers`].
    pub fn load(
        grid_dir: &Path,
        ignition: &Path,
        moistures: &Moistures,
        limit: WindLimit,
    ) -> Result<Self> {
        let fbfm40 = read_geotiff(grid_dir.join("FBFM40.tif"))?;
        let slope = read_geotiff(grid_dir.join("SlpD.tif"))?;
        let aspect = read_geotiff(grid_dir.join("Asp.tif"))?;
        let water = read_geotiff_u8(grid_dir.join("water.tif"))?;
        let ignited = read_geotiff_u8(ignition)?;
        for (layer, h) in [
            ("SlpD", &slope.header),
            ("Asp", &aspect.header),
            ("water", &water.header),
            ("ignition", &ignited.header),
        ] {
            if !same_geometry(&fbfm40.header, h) {
                return Err(EnsembleError::LayerMismatch { layer });
            }
        }
        Self::from_layers(
            &Layers {
                header: fbfm40.header,
                fbfm40: &fbfm40.data,
                slope_deg: &slope.data,
                aspect_deg: &aspect.data,
                water: &water.data,
                ignited: &ignited.data,
            },
            moistures,
            limit,
        )
    }

    /// Geometry of the input rasters.
    #[must_use]
    pub fn header(&self) -> &GridHeader {
        &self.header
    }

    /// Spread grid of the inputs.
    #[must_use]
    pub fn grid(&self) -> Grid {
        self.grid
    }

    /// Burnable mask, one value per cell.
    #[must_use]
    pub fn burnable(&self) -> &[bool] {
        &self.burnable
    }

    /// Arrival field at time 0: 0 at ignited cells, `f32::INFINITY` elsewhere.
    #[must_use]
    pub fn initial_arrival_s(&self) -> &[f32] {
        &self.initial_arrival_s
    }

    /// Number of cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.grid.len()
    }

    /// Always `false`: a grid has at least one cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }
}
