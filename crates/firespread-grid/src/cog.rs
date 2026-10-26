//! Tiled, Deflate-compressed single-band `f32` and `u8` GeoTIFF writers.
//!
//! The file is a classic little-endian TIFF: the 8-byte header, one IFD at
//! offset 8, the out-of-line tag values, then every tile in row-major order.

use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::geotiff::{
    COMPRESSION_DEFLATE, KEY_GT_MODEL_TYPE, KEY_GT_RASTER_TYPE, KEY_PROJECTED_CS_TYPE,
    MODEL_TYPE_PROJECTED, RASTER_PIXEL_IS_AREA, SAMPLE_FORMAT_IEEE_FP, SAMPLE_FORMAT_UINT,
    TAG_GDAL_NODATA, TAG_GEO_KEY_DIRECTORY, TAG_MODEL_PIXEL_SCALE, TAG_MODEL_TIEPOINT,
};
use crate::{Grid, GridError, GridHeader, MaskGrid, Result, check_data_len};

/// Tile width and height, in cells, of every file [`write_cog`] and
/// [`write_cog_u8`] produce.
pub const TILE_SIZE: usize = 256;

/// A cell type the writer encodes as little-endian sample bytes.
trait CogSample: Copy {
    /// TIFF BitsPerSample of this type.
    const BITS: u16;
    /// TIFF SampleFormat of this type.
    const FORMAT: u16;
    /// Appends the little-endian bytes of `self` to `out`.
    fn put(self, out: &mut Vec<u8>);
}

impl CogSample for f32 {
    const BITS: u16 = 32;
    const FORMAT: u16 = SAMPLE_FORMAT_IEEE_FP;
    fn put(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
}

impl CogSample for u8 {
    const BITS: u16 = 8;
    const FORMAT: u16 = SAMPLE_FORMAT_UINT;
    fn put(self, out: &mut Vec<u8>) {
        out.push(self);
    }
}

const TYPE_ASCII: u16 = 2;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_DOUBLE: u16 = 12;
const TILE_OFFSETS_INDEX: usize = 10;

/// Writes `grid` as a single-band `f32` GeoTIFF with 256 by 256 Deflate tiles.
///
/// Edge tiles are padded with the header's `nodata` value. The file carries
/// ModelPixelScaleTag, ModelTiepointTag at raster (0, 0, 0), a GeoKey directory
/// with GTModelTypeGeoKey = 1, GTRasterTypeGeoKey = 1 and ProjectedCSTypeGeoKey
/// = `epsg`, and GDAL_NODATA. The file has no overviews. Fails when a
/// dimension is zero or exceeds `u32`, a cell size or origin is not finite and
/// positive, `epsg` exceeds `u16`, the data length differs from `nx * ny`, or
/// the file would exceed 4 GiB.
pub fn write_cog(path: impl AsRef<Path>, grid: &Grid) -> Result<()> {
    let path = path.as_ref();
    let bytes = encode(path, &grid.header, &grid.data, grid.header.nodata)?;
    write_file(path, bytes)
}

/// Writes `grid` as a single-band 8-bit unsigned GeoTIFF (SampleFormat 1)
/// with 256 by 256 Deflate tiles.
///
/// The layout, georeferencing tags and GDAL_NODATA are those of
/// [`write_cog`]; edge tiles are padded with `nodata` as a `u8`. Fails on the
/// inputs [`write_cog`] fails on, and when the header's `nodata` is not an
/// integer in 0..=255.
pub fn write_cog_u8(path: impl AsRef<Path>, grid: &MaskGrid) -> Result<()> {
    let path = path.as_ref();
    let nodata = grid.header.nodata;
    if !(nodata.fract() == 0.0 && (0.0..=255.0).contains(&nodata)) {
        return Err(GridError::NodataNotU8 { nodata });
    }
    // `as u8` is exact: `nodata` is integral and inside 0..=255.
    let bytes = encode(path, &grid.header, &grid.data, nodata as u8)?;
    write_file(path, bytes)
}

fn write_file(path: &Path, bytes: Vec<u8>) -> Result<()> {
    std::fs::write(path, bytes).map_err(|source| GridError::Io {
        path: path.to_path_buf(),
        source,
    })
}

struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    bytes: Vec<u8>,
}

impl Entry {
    fn shorts(tag: u16, values: &[u16]) -> Entry {
        Entry {
            tag,
            kind: TYPE_SHORT,
            count: values.len() as u32,
            bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    fn longs(tag: u16, values: &[u32]) -> Entry {
        Entry {
            tag,
            kind: TYPE_LONG,
            count: values.len() as u32,
            bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    fn doubles(tag: u16, values: &[f64]) -> Entry {
        Entry {
            tag,
            kind: TYPE_DOUBLE,
            count: values.len() as u32,
            bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    fn ascii(tag: u16, text: &str) -> Entry {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        Entry {
            tag,
            kind: TYPE_ASCII,
            count: bytes.len() as u32,
            bytes,
        }
    }
}

fn validate(h: &GridHeader, cells: usize) -> Result<(u32, u32, u16)> {
    let dim = |field: &'static str, n: usize| -> Result<u32> {
        u32::try_from(n)
            .ok()
            .filter(|&v| v > 0)
            .ok_or(GridError::InvalidDimension {
                field,
                value: n as f64,
            })
    };
    let width = dim("nx", h.nx)?;
    let height = dim("ny", h.ny)?;
    for (field, value) in [("dx", h.dx), ("dy", h.dy)] {
        if !(value.is_finite() && value > 0.0) {
            return Err(GridError::InvalidDimension { field, value });
        }
    }
    for (field, value) in [("origin_x", h.origin_x), ("origin_y", h.origin_y)] {
        if !value.is_finite() {
            return Err(GridError::InvalidDimension { field, value });
        }
    }
    let epsg = u16::try_from(h.epsg).map_err(|_| GridError::EpsgOutOfRange { epsg: h.epsg })?;
    check_data_len(h, cells)?;
    Ok((width, height, epsg))
}

fn compress_tiles<T: CogSample>(
    path: &Path,
    h: &GridHeader,
    data: &[T],
    pad: T,
) -> Result<Vec<Vec<u8>>> {
    let nx = h.nx;
    let ny = h.ny;
    let across = nx.div_ceil(TILE_SIZE);
    let down = ny.div_ceil(TILE_SIZE);
    let mut raw = Vec::with_capacity(TILE_SIZE * TILE_SIZE * usize::from(T::BITS / 8));
    let mut tiles = Vec::with_capacity(across * down);
    for tr in 0..down {
        for tc in 0..across {
            raw.clear();
            for r in 0..TILE_SIZE {
                let row = tr * TILE_SIZE + r;
                for c in 0..TILE_SIZE {
                    let col = tc * TILE_SIZE + c;
                    if row < ny && col < nx {
                        data[row * nx + col].put(&mut raw);
                    } else {
                        pad.put(&mut raw);
                    }
                }
            }
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            let io = |source| GridError::Io {
                path: path.to_path_buf(),
                source,
            };
            enc.write_all(&raw).map_err(io)?;
            tiles.push(enc.finish().map_err(io)?);
        }
    }
    Ok(tiles)
}

fn encode<T: CogSample>(path: &Path, h: &GridHeader, data: &[T], pad: T) -> Result<Vec<u8>> {
    let (width, height, epsg) = validate(h, data.len())?;
    let tiles = compress_tiles(path, h, data, pad)?;
    let too_large = |bytes: u64| GridError::FileTooLarge {
        path: path.to_path_buf(),
        bytes,
    };
    let tile_size = TILE_SIZE as u16;

    let mut byte_counts = Vec::with_capacity(tiles.len());
    for t in &tiles {
        byte_counts.push(u32::try_from(t.len()).map_err(|_| too_large(t.len() as u64))?);
    }

    // TileOffsets holds placeholders until the value area is laid out.
    let mut entries = vec![
        Entry::longs(256, &[width]),
        Entry::longs(257, &[height]),
        Entry::shorts(258, &[T::BITS]),
        Entry::shorts(259, &[COMPRESSION_DEFLATE]),
        Entry::shorts(262, &[1]),
        Entry::shorts(277, &[1]),
        Entry::shorts(284, &[1]),
        Entry::shorts(317, &[1]),
        Entry::shorts(322, &[tile_size]),
        Entry::shorts(323, &[tile_size]),
        Entry::longs(324, &vec![0; tiles.len()]),
        Entry::longs(325, &byte_counts),
        Entry::shorts(339, &[T::FORMAT]),
        Entry::doubles(TAG_MODEL_PIXEL_SCALE, &[h.dx, h.dy, 0.0]),
        Entry::doubles(
            TAG_MODEL_TIEPOINT,
            &[0.0, 0.0, 0.0, h.origin_x, h.origin_y, 0.0],
        ),
        Entry::shorts(
            TAG_GEO_KEY_DIRECTORY,
            &[
                1,
                1,
                0,
                3,
                KEY_GT_MODEL_TYPE,
                0,
                1,
                MODEL_TYPE_PROJECTED,
                KEY_GT_RASTER_TYPE,
                0,
                1,
                RASTER_PIXEL_IS_AREA,
                KEY_PROJECTED_CS_TYPE,
                0,
                1,
                epsg,
            ],
        ),
        Entry::ascii(TAG_GDAL_NODATA, &h.nodata.to_string()),
    ];

    let ifd_offset: u64 = 8;
    let ifd_len = 2 + 12 * entries.len() as u64 + 4;
    let mut value_offset = ifd_offset + ifd_len;
    let mut value_offsets = Vec::with_capacity(entries.len());
    for e in &entries {
        if e.bytes.len() > 4 {
            value_offsets.push(Some(value_offset));
            value_offset += (e.bytes.len() as u64).next_multiple_of(2);
        } else {
            value_offsets.push(None);
        }
    }
    let mut tile_offsets = Vec::with_capacity(tiles.len());
    let mut cursor = value_offset;
    for t in &tiles {
        tile_offsets.push(u32::try_from(cursor).map_err(|_| too_large(cursor))?);
        cursor += (t.len() as u64).next_multiple_of(2);
    }
    let total = cursor;
    let total_usize = usize::try_from(total).map_err(|_| too_large(total))?;
    u32::try_from(total).map_err(|_| too_large(total))?;
    entries[TILE_OFFSETS_INDEX] = Entry::longs(324, &tile_offsets);

    let mut out = Vec::with_capacity(total_usize);
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42_u16.to_le_bytes());
    out.extend_from_slice(&(ifd_offset as u32).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (e, off) in entries.iter().zip(&value_offsets) {
        out.extend_from_slice(&e.tag.to_le_bytes());
        out.extend_from_slice(&e.kind.to_le_bytes());
        out.extend_from_slice(&e.count.to_le_bytes());
        let mut field = [0_u8; 4];
        match off {
            Some(o) => field.copy_from_slice(&(*o as u32).to_le_bytes()),
            None => field[..e.bytes.len()].copy_from_slice(&e.bytes),
        }
        out.extend_from_slice(&field);
    }
    out.extend_from_slice(&0_u32.to_le_bytes());
    for e in entries.iter().filter(|e| e.bytes.len() > 4) {
        out.extend_from_slice(&e.bytes);
        if e.bytes.len() % 2 == 1 {
            out.push(0);
        }
    }
    for t in &tiles {
        out.extend_from_slice(t);
        if t.len() % 2 == 1 {
            out.push(0);
        }
    }
    debug_assert_eq!(out.len() as u64, total);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_geotiff_u8;

    fn header(nx: usize, ny: usize, nodata: f32) -> GridHeader {
        GridHeader {
            nx,
            ny,
            dx: 30.0,
            dy: 30.0,
            origin_x: -876_180.0,
            origin_y: 1_499_820.0,
            epsg: 5070,
            nodata,
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("firespread-grid-cog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn write_cog_u8_round_trips_through_read_geotiff_u8() {
        // 300 x 257 has partial edge tiles in both directions.
        let h = header(300, 257, 255.0);
        let data: Vec<u8> = (0..h.ny)
            .flat_map(|r| (0..h.nx).map(move |c| ((r * 31 + c * 17) % 256) as u8))
            .collect();
        let grid = MaskGrid::new(h, data).unwrap();
        let path = tmp("roundtrip_u8_300x257.tif");
        write_cog_u8(&path, &grid).unwrap();
        let back = read_geotiff_u8(&path).unwrap();
        assert!(
            back.header.same_grid(&grid.header),
            "header: got {:?}, wrote {:?}",
            back.header,
            grid.header
        );
        if let Some(i) = (0..grid.data.len()).find(|&i| back.data[i] != grid.data[i]) {
            panic!(
                "cell ({}, {}): got {}, wrote {}",
                i / h.nx,
                i % h.nx,
                back.data[i],
                grid.data[i]
            );
        }
    }

    #[test]
    fn write_cog_u8_rejects_nodata_outside_u8() {
        for nodata in [-9999.0, 256.0, 0.5, f32::NAN] {
            let grid = MaskGrid::new(header(2, 1, nodata), vec![0, 1]).unwrap();
            let Err(e) = write_cog_u8(tmp("bad_nodata.tif"), &grid) else {
                panic!("nodata {nodata} was written");
            };
            assert!(
                matches!(e, GridError::NodataNotU8 { .. }),
                "nodata {nodata}: {e:?}"
            );
        }
    }
}
