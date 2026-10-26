//! One-shot loads of the static rasters of a run: fuels, terrain and the
//! start mask.

use std::path::{Path, PathBuf};

use firespread_ensemble::{EnsembleError, HourlyLayers, Layers};
use firespread_grid::{Grid, GridHeader, MaskGrid, read_geotiff, read_geotiff_u8};

use crate::executor::Executor;
use crate::{AgentError, Result};

/// The static rasters of a run on one grid, row-major with the north row
/// first.
#[derive(Debug, Clone, PartialEq)]
pub struct GridInputs {
    /// Geometry of every layer; `nodata` is the FBFM40 nodata value.
    pub header: GridHeader,
    /// LANDFIRE FBFM40 codes.
    pub fbfm40: Vec<f32>,
    /// Slope, degrees.
    pub slope_deg: Vec<f32>,
    /// Aspect, degrees; -1 is flat.
    pub aspect_deg: Vec<f32>,
    /// Water class; any code > 0 is non-burnable.
    pub water: Vec<u8>,
    /// The window's start mask: burned at time 0 where the value is 1.
    pub ignited: Vec<u8>,
    /// LANDFIRE canopy height CH, m.
    pub canopy_height_m: Vec<f32>,
    /// LANDFIRE canopy base height CBH, m.
    pub canopy_base_height_m: Vec<f32>,
    /// LANDFIRE canopy cover CC, percent.
    pub canopy_cover_pct: Vec<f32>,
}

impl GridInputs {
    /// The layers as [`firespread_ensemble::HourlyInputs::from_layers`]
    /// takes them.
    #[must_use]
    pub fn layers(&self) -> HourlyLayers<'_> {
        HourlyLayers {
            base: Layers {
                header: self.header,
                fbfm40: &self.fbfm40,
                slope_deg: &self.slope_deg,
                aspect_deg: &self.aspect_deg,
                water: &self.water,
                ignited: &self.ignited,
            },
            canopy_height_m: &self.canopy_height_m,
            canopy_base_height_m: &self.canopy_base_height_m,
            canopy_cover_pct: &self.canopy_cover_pct,
        }
    }
}

struct Fuels {
    fbfm40: Grid,
    ch: Grid,
    cbh: Grid,
    cc: Grid,
    water: MaskGrid,
}

struct Terrain {
    slope: Grid,
    aspect: Grid,
}

fn grid_err(stage: &'static str) -> impl FnOnce(firespread_grid::GridError) -> AgentError {
    move |source| AgentError::Grid { stage, source }
}

/// Reads `FBFM40.tif`, `CH.tif`, `CBH.tif`, `CC.tif` (f32) and `water.tif`
/// (u8) from `grid_dir` on `exec`.
async fn load_fuels(exec: &Executor, grid_dir: PathBuf) -> Result<Fuels> {
    exec.run("fuel load", move || -> Result<Fuels> {
        let f32_layer = |name: &str| {
            read_geotiff(grid_dir.join(name)).map_err(grid_err("reading the fuel layers"))
        };
        Ok(Fuels {
            fbfm40: f32_layer("FBFM40.tif")?,
            ch: f32_layer("CH.tif")?,
            cbh: f32_layer("CBH.tif")?,
            cc: f32_layer("CC.tif")?,
            water: read_geotiff_u8(grid_dir.join("water.tif"))
                .map_err(grid_err("reading the water layer"))?,
        })
    })
    .await?
}

/// Reads `SlpD.tif` and `Asp.tif` (f32) from `grid_dir` on `exec`.
async fn load_terrain(exec: &Executor, grid_dir: PathBuf) -> Result<Terrain> {
    exec.run("terrain load", move || -> Result<Terrain> {
        let layer = |name: &str| {
            read_geotiff(grid_dir.join(name)).map_err(grid_err("reading the terrain layers"))
        };
        Ok(Terrain {
            slope: layer("SlpD.tif")?,
            aspect: layer("Asp.tif")?,
        })
    })
    .await?
}

/// Reads the u8 start mask at `path` on `exec`.
async fn load_start_mask(exec: &Executor, path: PathBuf) -> Result<MaskGrid> {
    exec.run("start mask load", move || {
        read_geotiff_u8(path).map_err(grid_err("reading the start mask"))
    })
    .await?
}

/// Loads the fuels, terrain and start mask concurrently, each read on
/// `exec`: `FBFM40.tif`, `SlpD.tif`, `Asp.tif`, `CH.tif`, `CBH.tif`,
/// `CC.tif` (f32) and `water.tif` (u8) from `grid_dir`, and the u8 start
/// mask at `start_mask`.
///
/// # Errors
/// [`AgentError::Grid`] for a failed read, [`AgentError::Ensemble`] wrapping
/// [`EnsembleError::LayerMismatch`] when a layer's geometry, nodata aside,
/// differs from FBFM40's, and the errors of [`Executor::run`].
pub async fn load_grid_inputs(
    exec: &Executor,
    grid_dir: &Path,
    start_mask: &Path,
) -> Result<GridInputs> {
    let (fuels, terrain, mask) = tokio::try_join!(
        load_fuels(exec, grid_dir.to_path_buf()),
        load_terrain(exec, grid_dir.to_path_buf()),
        load_start_mask(exec, start_mask.to_path_buf()),
    )?;
    let geometry = |h: &GridHeader| GridHeader { nodata: 0.0, ..*h };
    let base = geometry(&fuels.fbfm40.header);
    for (layer, h) in [
        ("SlpD", &terrain.slope.header),
        ("Asp", &terrain.aspect.header),
        ("CH", &fuels.ch.header),
        ("CBH", &fuels.cbh.header),
        ("CC", &fuels.cc.header),
        ("water", &fuels.water.header),
        ("ignition", &mask.header),
    ] {
        if !base.same_grid(&geometry(h)) {
            return Err(AgentError::Ensemble {
                stage: "checking the layer geometry",
                source: EnsembleError::LayerMismatch { layer },
            });
        }
    }
    Ok(GridInputs {
        header: fuels.fbfm40.header,
        fbfm40: fuels.fbfm40.data,
        slope_deg: terrain.slope.data,
        aspect_deg: terrain.aspect.data,
        water: fuels.water.data,
        ignited: mask.data,
        canopy_height_m: fuels.ch.data,
        canopy_base_height_m: fuels.cbh.data,
        canopy_cover_pct: fuels.cc.data,
    })
}
