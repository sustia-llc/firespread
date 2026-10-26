#![forbid(unsafe_code)]
//! Raster grid of fuel, terrain and weather cells for the spread engine.
//!
//! A [`Grid`] is a single band of `f32` cells on a north-up projected raster,
//! stored row-major with the north row first. [`read_geotiff`] loads one from a
//! single-band `f32` GeoTIFF, [`write_cog`] writes one as a tiled,
//! Deflate-compressed GeoTIFF, and [`Manifest`] records the inputs and outputs
//! of one forecast run.

mod cog;
mod geotiff;
pub mod manifest;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use cog::{TILE_SIZE, write_cog};
pub use geotiff::{read_geotiff, read_geotiff_header};
pub use manifest::{ForecastWindow, MANIFEST_VERSION, Manifest, WindLimitPolicy};

/// Errors raised by grid construction, indexing, GeoTIFF I/O and manifest I/O.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GridError {
    /// A cell index lies outside the grid.
    #[error("cell ({row}, {col}) is outside a {rows}x{cols} grid")]
    OutOfBounds {
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    },
    /// A grid dimension or cell size is zero, not finite, or too large for a classic TIFF.
    #[error("grid {field} must be positive and representable, got {value}")]
    InvalidDimension { field: &'static str, value: f64 },
    /// The cell vector length differs from `nx * ny`.
    #[error("grid data has {found} cells, header {nx}x{ny} needs {expected}")]
    DataLength {
        nx: usize,
        ny: usize,
        expected: usize,
        found: usize,
    },
    /// An operating-system I/O call failed.
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The TIFF decoder rejected the file.
    #[error("TIFF decode error in {path}: {source}")]
    Tiff {
        path: PathBuf,
        source: tiff::TiffError,
    },
    /// The image has more than one sample per pixel.
    #[error("{path} has {samples} samples per pixel, expected 1")]
    NotSingleBand { path: PathBuf, samples: u16 },
    /// The sample type is not 32-bit IEEE floating point.
    #[error(
        "{path} has {bits}-bit samples of SampleFormat {format}, expected 32-bit IEEE float (3)"
    )]
    UnsupportedSampleFormat {
        path: PathBuf,
        bits: u16,
        format: u16,
    },
    /// The compression scheme is neither none (1) nor Deflate (8 or 32946).
    #[error("{path} uses TIFF compression {code}, expected 1, 8 or 32946")]
    UnsupportedCompression { path: PathBuf, code: u16 },
    /// A predictor other than none (1) is set.
    #[error("{path} uses TIFF predictor {code}, expected 1")]
    UnsupportedPredictor { path: PathBuf, code: u16 },
    /// A required GeoTIFF tag is absent.
    #[error("{path} lacks required tag {tag}")]
    MissingTag { path: PathBuf, tag: u16 },
    /// A GeoTIFF tag has the wrong type, count or content.
    #[error("{path} tag {tag} is invalid: {detail}")]
    InvalidTag {
        path: PathBuf,
        tag: u16,
        detail: String,
    },
    /// The file carries a ModelTransformationTag (34264), so the raster may be rotated.
    #[error("{path} carries ModelTransformationTag 34264; only north-up rasters are supported")]
    Rotated { path: PathBuf },
    /// The ModelTiepointTag has more than one tiepoint.
    #[error("{path} has {count} tiepoints, expected 1")]
    MultipleTiepoints { path: PathBuf, count: usize },
    /// The single tiepoint does not anchor raster point (0, 0, 0).
    #[error("{path} tiepoint raster coordinate is ({i}, {j}, {k}), expected (0, 0, 0)")]
    RasterTiepointOffset {
        path: PathBuf,
        i: f64,
        j: f64,
        k: f64,
    },
    /// The pixel scale is not positive and finite in x and y.
    #[error("{path} pixel scale ({dx}, {dy}) is not a positive north-up scale")]
    NotNorthUp { path: PathBuf, dx: f64, dy: f64 },
    /// GTRasterTypeGeoKey (1025) is not RasterPixelIsArea (1).
    #[error("{path} GTRasterTypeGeoKey is {raster_type}, expected 1 (PixelIsArea)")]
    NotPixelIsArea { path: PathBuf, raster_type: u16 },
    /// ProjectedCSTypeGeoKey (3072) is absent or user-defined.
    #[error("{path} has no EPSG ProjectedCSTypeGeoKey (3072)")]
    MissingEpsg { path: PathBuf },
    /// The EPSG code does not fit the 16-bit GeoKey value field.
    #[error("EPSG code {epsg} does not fit a 16-bit GeoKey value")]
    EpsgOutOfRange { epsg: u32 },
    /// The GDAL_NODATA tag (42113) does not parse as an `f32`.
    #[error("{path} GDAL_NODATA value {text:?} is not an f32")]
    InvalidNodata { path: PathBuf, text: String },
    /// The encoded file would exceed the 4 GiB classic TIFF offset range.
    #[error("{path} would be {bytes} bytes, beyond the classic TIFF limit")]
    FileTooLarge { path: PathBuf, bytes: u64 },
    /// JSON encoding or decoding failed.
    #[error("JSON error on {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// A manifest carries a version this build does not read.
    #[error("{path} has manifest version {found}, expected {expected}")]
    UnsupportedManifestVersion {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
}

pub type Result<T> = core::result::Result<T, GridError>;

/// Georeferencing and shape of a north-up raster.
///
/// `origin_x` is the west edge and `origin_y` the north edge of the grid, in
/// the units of the projected CRS `epsg`; `dx` and `dy` are the positive cell
/// width and height. Cells equal to `nodata` carry no value.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridHeader {
    pub nx: usize,
    pub ny: usize,
    pub dx: f64,
    pub dy: f64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub epsg: u32,
    pub nodata: f32,
}

impl GridHeader {
    /// True when every field is equal; floating-point fields compare by bit pattern.
    #[must_use]
    pub fn same_grid(&self, other: &GridHeader) -> bool {
        self.nx == other.nx
            && self.ny == other.ny
            && self.dx.to_bits() == other.dx.to_bits()
            && self.dy.to_bits() == other.dy.to_bits()
            && self.origin_x.to_bits() == other.origin_x.to_bits()
            && self.origin_y.to_bits() == other.origin_y.to_bits()
            && self.epsg == other.epsg
            && self.nodata.to_bits() == other.nodata.to_bits()
    }

    /// Number of cells, `nx * ny`, or `None` on overflow.
    #[must_use]
    pub fn cell_count(&self) -> Option<usize> {
        self.nx.checked_mul(self.ny)
    }
}

impl PartialEq for GridHeader {
    fn eq(&self, other: &Self) -> bool {
        self.same_grid(other)
    }
}

/// A single band of `f32` cells, row-major with the north row first.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub header: GridHeader,
    pub data: Vec<f32>,
}

impl Grid {
    /// Builds a grid, failing when `data.len()` differs from `nx * ny`.
    pub fn new(header: GridHeader, data: Vec<f32>) -> Result<Grid> {
        check_data_len(&header, data.len())?;
        Ok(Grid { header, data })
    }

    /// Value at `row` (0 = north) and `col` (0 = west).
    pub fn get(&self, row: usize, col: usize) -> Result<f32> {
        let GridHeader { nx, ny, .. } = self.header;
        let out = GridError::OutOfBounds {
            row,
            col,
            rows: ny,
            cols: nx,
        };
        if row >= ny || col >= nx {
            return Err(out);
        }
        self.data.get(row * nx + col).copied().ok_or(out)
    }
}

pub(crate) fn check_data_len(header: &GridHeader, found: usize) -> Result<()> {
    let expected = header.cell_count().ok_or(GridError::InvalidDimension {
        field: "nx * ny",
        value: header.nx as f64 * header.ny as f64,
    })?;
    if found != expected {
        return Err(GridError::DataLength {
            nx: header.nx,
            ny: header.ny,
            expected,
            found,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> GridHeader {
        GridHeader {
            nx: 3,
            ny: 2,
            dx: 30.0,
            dy: 30.0,
            origin_x: -876_180.0,
            origin_y: 1_499_820.0,
            epsg: 5070,
            nodata: -9999.0,
        }
    }

    #[test]
    fn get_indexes_row_major_north_first() {
        let grid = Grid::new(header(), vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert_eq!(grid.get(0, 2).unwrap(), 2.0);
        assert_eq!(grid.get(1, 0).unwrap(), 3.0);
        assert!(matches!(
            grid.get(2, 0),
            Err(GridError::OutOfBounds {
                row: 2,
                col: 0,
                rows: 2,
                cols: 3
            })
        ));
        assert!(matches!(
            grid.get(0, 3),
            Err(GridError::OutOfBounds { col: 3, .. })
        ));
    }

    #[test]
    fn new_rejects_wrong_length() {
        let err = Grid::new(header(), vec![0.0; 5]).unwrap_err();
        assert!(
            matches!(
                err,
                GridError::DataLength {
                    expected: 6,
                    found: 5,
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn new_rejects_overflowing_cell_count() {
        let mut h = header();
        h.nx = usize::MAX;
        h.ny = 2;
        let err = Grid::new(h, vec![]).unwrap_err();
        assert!(
            matches!(
                err,
                GridError::InvalidDimension {
                    field: "nx * ny",
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn same_grid_checks_every_field() {
        let base = header();
        assert!(base.same_grid(&base));
        let variants: [fn(&mut GridHeader); 8] = [
            |h| h.nx += 1,
            |h| h.ny += 1,
            |h| h.dx = 31.0,
            |h| h.dy = 31.0,
            |h| h.origin_x += 1.0,
            |h| h.origin_y += 1.0,
            |h| h.epsg = 4326,
            |h| h.nodata = f32::NAN,
        ];
        for (i, mutate) in variants.iter().enumerate() {
            let mut other = base;
            mutate(&mut other);
            assert!(!base.same_grid(&other), "variant {i} compared equal");
        }
        let mut nan = base;
        nan.nodata = f32::NAN;
        assert!(nan.same_grid(&nan), "NaN nodata must equal itself");
    }
}
