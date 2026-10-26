//! Single-band `f32` GeoTIFF reader on the pure-Rust `tiff` decoder.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use tiff::decoder::{ChunkType, Decoder};
use tiff::tags::Tag;

use crate::{Grid, GridError, GridHeader, Result};

pub(crate) const TAG_MODEL_PIXEL_SCALE: u16 = 33550;
pub(crate) const TAG_MODEL_TIEPOINT: u16 = 33922;
pub(crate) const TAG_GEO_KEY_DIRECTORY: u16 = 34735;
pub(crate) const TAG_GDAL_NODATA: u16 = 42113;

pub(crate) const KEY_GT_MODEL_TYPE: u16 = 1024;
pub(crate) const KEY_GT_RASTER_TYPE: u16 = 1025;
pub(crate) const KEY_PROJECTED_CS_TYPE: u16 = 3072;
pub(crate) const MODEL_TYPE_PROJECTED: u16 = 1;
pub(crate) const RASTER_PIXEL_IS_AREA: u16 = 1;
const GEOKEY_USER_DEFINED: u16 = 32767;

pub(crate) const COMPRESSION_NONE: u16 = 1;
pub(crate) const COMPRESSION_DEFLATE: u16 = 8;
const COMPRESSION_OLD_DEFLATE: u16 = 32946;
pub(crate) const SAMPLE_FORMAT_IEEE_FP: u16 = 3;

type TiffDecoder = Decoder<BufReader<File>>;

/// Reads the header of a single-band `f32` GeoTIFF after validating its layout.
///
/// Accepts the same files as [`read_geotiff`] and fails on the same layouts.
pub fn read_geotiff_header(path: impl AsRef<Path>) -> Result<GridHeader> {
    let path = path.as_ref();
    let mut dec = open(path)?;
    header_from(&mut dec, path)
}

/// Reads a single-band `f32` GeoTIFF into a [`Grid`].
///
/// Accepts tiled or stripped layouts, including partial edge chunks, with no
/// compression or Deflate and no predictor. The georeferencing must be one
/// ModelTiepointTag anchored at raster (0, 0, 0), a positive ModelPixelScaleTag,
/// no ModelTransformationTag, GTRasterTypeGeoKey absent or PixelIsArea, an EPSG
/// ProjectedCSTypeGeoKey, and a GDAL_NODATA tag that parses as `f32`.
pub fn read_geotiff(path: impl AsRef<Path>) -> Result<Grid> {
    let path = path.as_ref();
    let mut dec = open(path)?;
    let header = header_from(&mut dec, path)?;
    let data = read_cells(&mut dec, path, &header)?;
    Ok(Grid { header, data })
}

fn open(path: &Path) -> Result<TiffDecoder> {
    let file = File::open(path).map_err(|source| GridError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Decoder::new(BufReader::new(file)).map_err(|source| tiff_err(path, source))
}

fn tiff_err(path: &Path, source: tiff::TiffError) -> GridError {
    GridError::Tiff {
        path: path.to_path_buf(),
        source,
    }
}

fn header_from(dec: &mut TiffDecoder, path: &Path) -> Result<GridHeader> {
    check_band_layout(dec, path)?;
    let (width, height) = dec.dimensions().map_err(|e| tiff_err(path, e))?;

    if dec
        .find_tag(Tag::ModelTransformationTag)
        .map_err(|e| tiff_err(path, e))?
        .is_some()
    {
        return Err(GridError::Rotated {
            path: path.to_path_buf(),
        });
    }

    let scale = f64_tag(dec, path, Tag::ModelPixelScaleTag, TAG_MODEL_PIXEL_SCALE)?;
    if scale.len() != 3 {
        return Err(GridError::InvalidTag {
            path: path.to_path_buf(),
            tag: TAG_MODEL_PIXEL_SCALE,
            detail: format!("expected 3 values, got {}", scale.len()),
        });
    }
    let (dx, dy) = (scale[0], scale[1]);
    if !(dx.is_finite() && dx > 0.0 && dy.is_finite() && dy > 0.0) {
        return Err(GridError::NotNorthUp {
            path: path.to_path_buf(),
            dx,
            dy,
        });
    }

    let tie = f64_tag(dec, path, Tag::ModelTiepointTag, TAG_MODEL_TIEPOINT)?;
    if tie.is_empty() || tie.len() % 6 != 0 {
        return Err(GridError::InvalidTag {
            path: path.to_path_buf(),
            tag: TAG_MODEL_TIEPOINT,
            detail: format!("expected a multiple of 6 values, got {}", tie.len()),
        });
    }
    if tie.len() != 6 {
        return Err(GridError::MultipleTiepoints {
            path: path.to_path_buf(),
            count: tie.len() / 6,
        });
    }
    let (i, j, k) = (tie[0], tie[1], tie[2]);
    if i != 0.0 || j != 0.0 || k != 0.0 {
        return Err(GridError::RasterTiepointOffset {
            path: path.to_path_buf(),
            i,
            j,
            k,
        });
    }
    let (origin_x, origin_y) = (tie[3], tie[4]);
    if !(origin_x.is_finite() && origin_y.is_finite()) {
        return Err(GridError::InvalidTag {
            path: path.to_path_buf(),
            tag: TAG_MODEL_TIEPOINT,
            detail: format!("model coordinate ({origin_x}, {origin_y}) is not finite"),
        });
    }

    let epsg = epsg_from_geokeys(dec, path)?;
    let nodata = nodata_tag(dec, path)?;

    Ok(GridHeader {
        nx: width as usize,
        ny: height as usize,
        dx,
        dy,
        origin_x,
        origin_y,
        epsg,
        nodata,
    })
}

fn check_band_layout(dec: &mut TiffDecoder, path: &Path) -> Result<()> {
    let samples = dec
        .find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
        .map_err(|e| tiff_err(path, e))?
        .unwrap_or(1);
    if samples != 1 {
        return Err(GridError::NotSingleBand {
            path: path.to_path_buf(),
            samples,
        });
    }
    let bits = first_u16(dec, path, Tag::BitsPerSample)?.unwrap_or(1);
    let format = first_u16(dec, path, Tag::SampleFormat)?.unwrap_or(1);
    if bits != 32 || format != SAMPLE_FORMAT_IEEE_FP {
        return Err(GridError::UnsupportedSampleFormat {
            path: path.to_path_buf(),
            bits,
            format,
        });
    }
    let code = dec
        .find_tag_unsigned::<u16>(Tag::Compression)
        .map_err(|e| tiff_err(path, e))?
        .unwrap_or(COMPRESSION_NONE);
    if !matches!(
        code,
        COMPRESSION_NONE | COMPRESSION_DEFLATE | COMPRESSION_OLD_DEFLATE
    ) {
        return Err(GridError::UnsupportedCompression {
            path: path.to_path_buf(),
            code,
        });
    }
    let predictor = dec
        .find_tag_unsigned::<u16>(Tag::Predictor)
        .map_err(|e| tiff_err(path, e))?
        .unwrap_or(1);
    if predictor != 1 {
        return Err(GridError::UnsupportedPredictor {
            path: path.to_path_buf(),
            code: predictor,
        });
    }
    Ok(())
}

fn first_u16(dec: &mut TiffDecoder, path: &Path, tag: Tag) -> Result<Option<u16>> {
    let values = dec
        .find_tag_unsigned_vec::<u16>(tag)
        .map_err(|e| tiff_err(path, e))?;
    Ok(values.and_then(|v| v.first().copied()))
}

fn f64_tag(dec: &mut TiffDecoder, path: &Path, tag: Tag, code: u16) -> Result<Vec<f64>> {
    let value = dec
        .find_tag(tag)
        .map_err(|e| tiff_err(path, e))?
        .ok_or_else(|| GridError::MissingTag {
            path: path.to_path_buf(),
            tag: code,
        })?;
    value.into_f64_vec().map_err(|e| GridError::InvalidTag {
        path: path.to_path_buf(),
        tag: code,
        detail: e.to_string(),
    })
}

fn epsg_from_geokeys(dec: &mut TiffDecoder, path: &Path) -> Result<u32> {
    let invalid = |detail: String| GridError::InvalidTag {
        path: path.to_path_buf(),
        tag: TAG_GEO_KEY_DIRECTORY,
        detail,
    };
    let keys = dec
        .find_tag(Tag::GeoKeyDirectoryTag)
        .map_err(|e| tiff_err(path, e))?
        .ok_or_else(|| GridError::MissingTag {
            path: path.to_path_buf(),
            tag: TAG_GEO_KEY_DIRECTORY,
        })?
        .into_u16_vec()
        .map_err(|e| invalid(e.to_string()))?;
    if keys.len() < 4 {
        return Err(invalid(format!(
            "header needs 4 values, got {}",
            keys.len()
        )));
    }
    let count = usize::from(keys[3]);
    let entries = keys.get(4..4 + 4 * count).ok_or_else(|| {
        invalid(format!(
            "{count} keys need {} values, got {}",
            4 + 4 * count,
            keys.len()
        ))
    })?;
    let mut epsg = None;
    for &[id, location, _count, value] in entries.as_chunks::<4>().0 {
        match id {
            KEY_GT_RASTER_TYPE | KEY_PROJECTED_CS_TYPE if location != 0 => {
                return Err(invalid(format!(
                    "key {id} stored in tag {location}, expected an inline SHORT"
                )));
            }
            KEY_GT_RASTER_TYPE if value != RASTER_PIXEL_IS_AREA => {
                return Err(GridError::NotPixelIsArea {
                    path: path.to_path_buf(),
                    raster_type: value,
                });
            }
            KEY_PROJECTED_CS_TYPE if value != 0 && value != GEOKEY_USER_DEFINED => {
                epsg = Some(u32::from(value));
            }
            _ => {}
        }
    }
    epsg.ok_or_else(|| GridError::MissingEpsg {
        path: path.to_path_buf(),
    })
}

fn nodata_tag(dec: &mut TiffDecoder, path: &Path) -> Result<f32> {
    let text = dec
        .find_tag(Tag::GdalNodata)
        .map_err(|e| tiff_err(path, e))?
        .ok_or_else(|| GridError::MissingTag {
            path: path.to_path_buf(),
            tag: TAG_GDAL_NODATA,
        })?
        .into_string()
        .map_err(|e| GridError::InvalidTag {
            path: path.to_path_buf(),
            tag: TAG_GDAL_NODATA,
            detail: e.to_string(),
        })?;
    let trimmed = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    trimmed
        .parse::<f32>()
        .map_err(|_| GridError::InvalidNodata {
            path: path.to_path_buf(),
            text,
        })
}

fn read_cells(dec: &mut TiffDecoder, path: &Path, header: &GridHeader) -> Result<Vec<f32>> {
    let nx = header.nx;
    let mut data = vec![0.0_f32; nx * header.ny];
    let (cw, ch) = dec.chunk_dimensions();
    let (cw, ch) = (cw as usize, ch as usize);
    let across = nx.div_ceil(cw);
    let chunks = match dec.get_chunk_type() {
        ChunkType::Tile => dec.tile_count(),
        ChunkType::Strip => dec.strip_count(),
    }
    .map_err(|e| tiff_err(path, e))?;
    let mut buf = vec![0_u8; cw * ch * 4];
    for index in 0..chunks {
        let (w, h) = dec.chunk_data_dimensions(index);
        let (w, h) = (w as usize, h as usize);
        let bytes = &mut buf[..w * h * 4];
        dec.read_chunk_bytes(index, bytes)
            .map_err(|e| tiff_err(path, e))?;
        let i = index as usize;
        let row0 = (i / across) * ch;
        let col0 = (i % across) * cw;
        for (r, src_row) in bytes.chunks_exact(w * 4).enumerate() {
            let start = (row0 + r) * nx + col0;
            let dst = &mut data[start..start + w];
            for (cell, word) in dst.iter_mut().zip(src_row.as_chunks::<4>().0) {
                *cell = f32::from_ne_bytes(*word);
            }
        }
    }
    Ok(data)
}
