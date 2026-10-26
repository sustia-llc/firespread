//! GeoTIFF reader and writer tests against GDAL-written fixtures and crafted files.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use firespread_grid::{
    Grid, GridError, GridHeader, TILE_SIZE, read_geotiff, read_geotiff_header, write_cog,
};
use serde_json::Value;
use tiff::decoder::{ChunkType, Decoder};
use tiff::encoder::colortype::{self, ColorType};
use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, TiffValue};
use tiff::tags::Tag;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("firespread-grid-geotiff");
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn sidecar(name: &str) -> (GridHeader, Vec<f32>) {
    let text = fs::read_to_string(fixture(name)).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let header = GridHeader {
        nx: v["nx"].as_u64().unwrap() as usize,
        ny: v["ny"].as_u64().unwrap() as usize,
        dx: v["dx"].as_f64().unwrap(),
        dy: v["dy"].as_f64().unwrap(),
        origin_x: v["origin_x"].as_f64().unwrap(),
        origin_y: v["origin_y"].as_f64().unwrap(),
        epsg: v["epsg"].as_u64().unwrap() as u32,
        nodata: v["nodata"].as_f64().unwrap() as f32,
    };
    let values = v["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            let f = x.as_f64().unwrap();
            let g = f as f32;
            assert_eq!(f64::from(g), f, "sidecar value {f} is not an exact f32");
            g
        })
        .collect();
    (header, values)
}

fn assert_bits_eq(got: &[f32], want: &[f32], nx: usize, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(
            g.to_bits(),
            w.to_bits(),
            "{what}: cell {i} (row {}, col {}) got {g}, want {w}",
            i / nx,
            i % nx
        );
    }
}

fn check_fixture(stem: &str) {
    let (want_header, want_values) = sidecar(&format!("{stem}.json"));
    let grid = read_geotiff(fixture(&format!("{stem}.tif"))).unwrap();
    assert_eq!(grid.header, want_header, "{stem} header");
    assert_bits_eq(&grid.data, &want_values, want_header.nx, stem);
    let header_only = read_geotiff_header(fixture(&format!("{stem}.tif"))).unwrap();
    assert_eq!(header_only, want_header, "{stem} read_geotiff_header");
}

#[test]
fn reads_fbfm40_fixture_exactly() {
    check_fixture("FBFM40_40");
}

#[test]
fn reads_slpd_fixture_exactly() {
    check_fixture("SlpD_40");
}

#[test]
fn fixtures_have_partial_edge_tiles() {
    let file = fs::File::open(fixture("SlpD_40.tif")).unwrap();
    let mut dec = Decoder::new(file).unwrap();
    assert_eq!(dec.get_chunk_type(), ChunkType::Tile);
    assert_eq!(dec.chunk_dimensions(), (16, 16));
    assert_eq!(dec.tile_count().unwrap(), 9);
    assert_eq!(dec.chunk_data_dimensions(8), (8, 8));
}

fn header_300x257() -> GridHeader {
    GridHeader {
        nx: 300,
        ny: 257,
        dx: 30.0,
        dy: 30.0,
        origin_x: -876_180.0,
        origin_y: 1_499_820.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

fn grid_300x257() -> Grid {
    let h = header_300x257();
    let mut data: Vec<f32> = (0..h.nx * h.ny)
        .map(|i| {
            if i % 13 == 0 {
                h.nodata
            } else {
                ((i * 7919) % 100_003) as f32 * 0.37 - 1000.0
            }
        })
        .collect();
    data[1] = -0.0;
    data[2] = f32::MIN_POSITIVE;
    data[3] = f32::MAX;
    data[h.nx * h.ny - 1] = 12.5;
    Grid::new(h, data).unwrap()
}

#[test]
fn write_then_read_is_bit_identical_on_300x257() {
    let grid = grid_300x257();
    let path = tmp("roundtrip_300x257.tif");
    write_cog(&path, &grid).unwrap();
    let back = read_geotiff(&path).unwrap();
    assert!(back.header.same_grid(&grid.header), "{:?}", back.header);
    assert_bits_eq(&back.data, &grid.data, 300, "300x257 round trip");
}

#[test]
fn written_file_is_tiled_256_deflate_with_ifd_first() {
    let grid = grid_300x257();
    let path = tmp("layout_300x257.tif");
    write_cog(&path, &grid).unwrap();
    let mut dec = Decoder::new(fs::File::open(&path).unwrap()).unwrap();
    assert_eq!(dec.ifd_pointer().map(|p| p.0), Some(8));
    assert_eq!(dec.get_chunk_type(), ChunkType::Tile);
    assert_eq!(dec.chunk_dimensions(), (256, 256));
    assert_eq!(dec.tile_count().unwrap(), 4);
    assert_eq!(dec.get_tag_u32(Tag::Compression).unwrap(), 8);
    let offsets = dec.get_tag_u32_vec(Tag::TileOffsets).unwrap();
    let counts = dec.get_tag_u32_vec(Tag::TileByteCounts).unwrap();
    assert_eq!(offsets.len(), 4);
    let ifd_end = 8 + 2 + 12 * 17 + 4;
    assert!(offsets[0] >= ifd_end, "first tile at {}", offsets[0]);
    for w in 0..3 {
        assert!(
            offsets[w] + counts[w] <= offsets[w + 1],
            "tile {w} at {} len {} overlaps tile {} at {}",
            offsets[w],
            counts[w],
            w + 1,
            offsets[w + 1]
        );
    }
    let len = fs::metadata(&path).unwrap().len() as u32;
    assert!(offsets[3] + counts[3] <= len);
    assert_eq!(dec.get_tag_ascii_string(Tag::GdalNodata).unwrap(), "-9999");
    assert_eq!(
        dec.get_tag_u16_vec(Tag::GeoKeyDirectoryTag).unwrap(),
        vec![1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 5070]
    );
}

#[test]
fn small_grid_is_one_256_tile_and_round_trips() {
    let grid = read_geotiff(fixture("FBFM40_40.tif")).unwrap();
    let path = tmp("fbfm40_40_rewritten.tif");
    write_cog(&path, &grid).unwrap();
    let mut dec = Decoder::new(fs::File::open(&path).unwrap()).unwrap();
    assert_eq!(dec.chunk_dimensions(), (TILE_SIZE as u32, TILE_SIZE as u32));
    assert_eq!(dec.tile_count().unwrap(), 1);
    let back = read_geotiff(&path).unwrap();
    assert_eq!(back.header, grid.header);
    assert_bits_eq(
        &back.data,
        &grid.data,
        grid.header.nx,
        "FBFM40_40 rewritten",
    );
}

// ---- crafted stripped files ----

#[derive(Clone)]
struct Geo {
    scale: Option<Vec<f64>>,
    scale_ascii: bool,
    tie: Option<Vec<f64>>,
    keys: Option<Vec<u16>>,
    nodata: Option<String>,
    nodata_short: bool,
    transform: bool,
    predictor: Option<u16>,
}

fn good_geo() -> Geo {
    Geo {
        scale: Some(vec![30.0, 30.0, 0.0]),
        scale_ascii: false,
        tie: Some(vec![0.0, 0.0, 0.0, -876_180.0, 1_499_820.0, 0.0]),
        keys: Some(vec![
            1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 5070,
        ]),
        nodata: Some("-9999".to_string()),
        nodata_short: false,
        transform: false,
        predictor: None,
    }
}

/// File name, mutation of the valid georeferencing, expected tag code.
type GeoCase = (&'static str, fn(&mut Geo), u16);
/// File name, mutation of a valid header, expected field name.
type HeaderCase = (&'static str, fn(&mut GridHeader), &'static str);

const W: u32 = 19;
const H: u32 = 23;

fn stripped<C: ColorType>(
    name: &str,
    data: &[C::Inner],
    compression: Compression,
    rows_per_strip: u32,
    geo: &Geo,
) -> PathBuf
where
    [C::Inner]: TiffValue,
{
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut buf)
            .unwrap()
            .with_compression(compression);
        let mut img = enc.new_image::<C>(W, H).unwrap();
        img.rows_per_strip(rows_per_strip).unwrap();
        let d = img.encoder();
        if let Some(s) = &geo.scale {
            if geo.scale_ascii {
                d.write_tag(Tag::ModelPixelScaleTag, "30 30 0").unwrap();
            } else {
                d.write_tag(Tag::ModelPixelScaleTag, &s[..]).unwrap();
            }
        }
        if let Some(t) = &geo.tie {
            d.write_tag(Tag::ModelTiepointTag, &t[..]).unwrap();
        }
        if let Some(k) = &geo.keys {
            d.write_tag(Tag::GeoKeyDirectoryTag, &k[..]).unwrap();
        }
        if let Some(n) = &geo.nodata {
            d.write_tag(Tag::GdalNodata, n.as_str()).unwrap();
        }
        if geo.nodata_short {
            d.write_tag(Tag::GdalNodata, 7_u16).unwrap();
        }
        if geo.transform {
            d.write_tag(Tag::ModelTransformationTag, &[0.0_f64; 16][..])
                .unwrap();
        }
        if let Some(p) = geo.predictor {
            d.write_tag(Tag::Predictor, p).unwrap();
        }
        img.write_data(data).unwrap();
    }
    let path = tmp(name);
    fs::write(&path, buf.into_inner()).unwrap();
    path
}

fn f32_cells() -> Vec<f32> {
    (0..W * H).map(|i| i as f32 * 0.5 - 7.0).collect()
}

fn stripped_f32(name: &str, geo: &Geo) -> PathBuf {
    stripped::<colortype::Gray32Float>(name, &f32_cells(), Compression::Uncompressed, 7, geo)
}

fn expected_header() -> GridHeader {
    GridHeader {
        nx: W as usize,
        ny: H as usize,
        dx: 30.0,
        dy: 30.0,
        origin_x: -876_180.0,
        origin_y: 1_499_820.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

#[test]
fn reads_stripped_uncompressed_with_partial_last_strip() {
    let path = stripped_f32("strip_none.tif", &good_geo());
    let grid = read_geotiff(&path).unwrap();
    assert_eq!(grid.header, expected_header());
    assert_bits_eq(
        &grid.data,
        &f32_cells(),
        W as usize,
        "stripped uncompressed",
    );
}

#[test]
fn reads_stripped_deflate() {
    let path = stripped::<colortype::Gray32Float>(
        "strip_deflate.tif",
        &f32_cells(),
        Compression::Deflate(DeflateLevel::Balanced),
        5,
        &good_geo(),
    );
    let mut dec = Decoder::new(fs::File::open(&path).unwrap()).unwrap();
    assert_eq!(dec.get_tag_u32(Tag::Compression).unwrap(), 8);
    let grid = read_geotiff(&path).unwrap();
    assert_eq!(grid.header, expected_header());
    assert_bits_eq(&grid.data, &f32_cells(), W as usize, "stripped deflate");
}

#[test]
fn reads_nan_nodata() {
    let mut geo = good_geo();
    geo.nodata = Some("nan".to_string());
    let header = read_geotiff_header(stripped_f32("nodata_nan.tif", &geo)).unwrap();
    assert!(header.nodata.is_nan(), "{header:?}");
}

fn err_of(path: &Path) -> GridError {
    match read_geotiff(path) {
        Ok(g) => panic!("{} read as {:?}", path.display(), g.header),
        Err(e) => e,
    }
}

#[test]
fn missing_file_is_io_error() {
    let e = err_of(&tmp("does_not_exist.tif"));
    assert!(matches!(e, GridError::Io { .. }), "{e:?}");
}

#[test]
fn garbage_is_tiff_error() {
    let path = tmp("garbage.tif");
    fs::write(&path, b"this is not a tiff file").unwrap();
    let e = err_of(&path);
    assert!(matches!(e, GridError::Tiff { .. }), "{e:?}");
}

#[test]
fn rgb_is_not_single_band() {
    let data = vec![0_u8; (W * H * 3) as usize];
    let path =
        stripped::<colortype::RGB8>("rgb8.tif", &data, Compression::Uncompressed, 7, &good_geo());
    let e = err_of(&path);
    assert!(
        matches!(e, GridError::NotSingleBand { samples: 3, .. }),
        "{e:?}"
    );
}

#[test]
fn non_f32_sample_formats_are_rejected() {
    let n = (W * H) as usize;
    let cases = [
        (
            stripped::<colortype::Gray16>(
                "u16.tif",
                &vec![0_u16; n],
                Compression::Uncompressed,
                7,
                &good_geo(),
            ),
            16,
            1,
        ),
        (
            stripped::<colortype::Gray32>(
                "u32.tif",
                &vec![0_u32; n],
                Compression::Uncompressed,
                7,
                &good_geo(),
            ),
            32,
            1,
        ),
        (
            stripped::<colortype::Gray64Float>(
                "f64.tif",
                &vec![0.0_f64; n],
                Compression::Uncompressed,
                7,
                &good_geo(),
            ),
            64,
            3,
        ),
    ];
    for (path, want_bits, want_format) in cases {
        let e = err_of(&path);
        assert!(
            matches!(e, GridError::UnsupportedSampleFormat { bits, format, .. }
                if bits == want_bits && format == want_format),
            "{}: {e:?}",
            path.display()
        );
    }
}

#[test]
fn packbits_is_unsupported_compression() {
    let path = stripped::<colortype::Gray32Float>(
        "packbits.tif",
        &f32_cells(),
        Compression::Packbits,
        7,
        &good_geo(),
    );
    let e = err_of(&path);
    assert!(
        matches!(e, GridError::UnsupportedCompression { code: 32773, .. }),
        "{e:?}"
    );
}

#[test]
fn predictor_is_rejected() {
    let mut geo = good_geo();
    geo.predictor = Some(3);
    let e = err_of(&stripped_f32("predictor3.tif", &geo));
    assert!(
        matches!(e, GridError::UnsupportedPredictor { code: 3, .. }),
        "{e:?}"
    );
}

#[test]
fn each_missing_geotag_is_reported() {
    let cases: [GeoCase; 4] = [
        ("no_scale.tif", |g| g.scale = None, 33550),
        ("no_tie.tif", |g| g.tie = None, 33922),
        ("no_keys.tif", |g| g.keys = None, 34735),
        ("no_nodata.tif", |g| g.nodata = None, 42113),
    ];
    for (name, mutate, want) in cases {
        let mut geo = good_geo();
        mutate(&mut geo);
        let e = err_of(&stripped_f32(name, &geo));
        assert!(
            matches!(e, GridError::MissingTag { tag, .. } if tag == want),
            "{name}: {e:?}"
        );
    }
}

#[test]
fn malformed_geotags_are_invalid() {
    let cases: [GeoCase; 8] = [
        (
            "scale_len2.tif",
            |g| g.scale = Some(vec![30.0, 30.0]),
            33550,
        ),
        ("scale_ascii.tif", |g| g.scale_ascii = true, 33550),
        (
            "tie_len5.tif",
            |g| g.tie = Some(vec![0.0, 0.0, 0.0, 1.0, 2.0]),
            33922,
        ),
        (
            "tie_nan.tif",
            |g| g.tie = Some(vec![0.0, 0.0, 0.0, f64::NAN, 2.0, 0.0]),
            33922,
        ),
        ("keys_len3.tif", |g| g.keys = Some(vec![1, 1, 0]), 34735),
        (
            "keys_short.tif",
            |g| g.keys = Some(vec![1, 1, 0, 2, 1024, 0, 1, 1]),
            34735,
        ),
        (
            "keys_location.tif",
            |g| g.keys = Some(vec![1, 1, 0, 1, 3072, 34737, 1, 0]),
            34735,
        ),
        (
            "nodata_short.tif",
            |g| {
                g.nodata = None;
                g.nodata_short = true;
            },
            42113,
        ),
    ];
    for (name, mutate, want) in cases {
        let mut geo = good_geo();
        mutate(&mut geo);
        let e = err_of(&stripped_f32(name, &geo));
        assert!(
            matches!(e, GridError::InvalidTag { tag, .. } if tag == want),
            "{name}: {e:?}"
        );
    }
}

#[test]
fn transformation_tag_is_rotated() {
    let mut geo = good_geo();
    geo.transform = true;
    let e = err_of(&stripped_f32("rotated.tif", &geo));
    assert!(matches!(e, GridError::Rotated { .. }), "{e:?}");
}

#[test]
fn two_tiepoints_are_rejected() {
    let mut geo = good_geo();
    geo.tie = Some(vec![
        0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 10.0, 10.0, 0.0, 301.0, -298.0, 0.0,
    ]);
    let e = err_of(&stripped_f32("two_ties.tif", &geo));
    assert!(
        matches!(e, GridError::MultipleTiepoints { count: 2, .. }),
        "{e:?}"
    );
}

#[test]
fn offset_raster_tiepoint_is_rejected() {
    let mut geo = good_geo();
    geo.tie = Some(vec![0.5, 0.5, 0.0, 1.0, 2.0, 0.0]);
    let e = err_of(&stripped_f32("tie_offset.tif", &geo));
    assert!(
        matches!(e, GridError::RasterTiepointOffset { i, j, .. } if i == 0.5 && j == 0.5),
        "{e:?}"
    );
}

#[test]
fn non_positive_scale_is_not_north_up() {
    for (name, scale) in [
        ("south_up.tif", vec![30.0, -30.0, 0.0]),
        ("zero_dx.tif", vec![0.0, 30.0, 0.0]),
    ] {
        let mut geo = good_geo();
        geo.scale = Some(scale.clone());
        let e = err_of(&stripped_f32(name, &geo));
        assert!(
            matches!(e, GridError::NotNorthUp { dx, dy, .. } if dx == scale[0] && dy == scale[1]),
            "{name}: {e:?}"
        );
    }
}

#[test]
fn pixel_is_point_is_rejected() {
    let mut geo = good_geo();
    geo.keys = Some(vec![
        1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 2, 3072, 0, 1, 5070,
    ]);
    let e = err_of(&stripped_f32("pixel_is_point.tif", &geo));
    assert!(
        matches!(e, GridError::NotPixelIsArea { raster_type: 2, .. }),
        "{e:?}"
    );
}

#[test]
fn absent_or_user_defined_epsg_is_missing() {
    for (name, keys) in [
        ("no_3072.tif", vec![1, 1, 0, 1, 1024, 0, 1, 1]),
        ("user_3072.tif", vec![1, 1, 0, 1, 3072, 0, 1, 32767]),
    ] {
        let mut geo = good_geo();
        geo.keys = Some(keys);
        let e = err_of(&stripped_f32(name, &geo));
        assert!(matches!(e, GridError::MissingEpsg { .. }), "{name}: {e:?}");
    }
}

#[test]
fn unparsable_nodata_is_invalid() {
    let mut geo = good_geo();
    geo.nodata = Some("abc".to_string());
    let e = err_of(&stripped_f32("nodata_abc.tif", &geo));
    assert!(
        matches!(&e, GridError::InvalidNodata { text, .. } if text.trim_end_matches('\0') == "abc"),
        "{e:?}"
    );
}

// ---- writer errors ----

fn write_err(grid: &Grid, name: &str) -> GridError {
    let path = tmp(name);
    match write_cog(&path, grid) {
        Ok(()) => panic!("{name} was written"),
        Err(e) => e,
    }
}

#[test]
fn writer_rejects_bad_dimensions() {
    let base = grid_300x257();
    let cases: [HeaderCase; 6] = [
        ("w_nx0.tif", |h| h.nx = 0, "nx"),
        ("w_nx_big.tif", |h| h.nx = 1 << 33, "nx"),
        ("w_ny0.tif", |h| h.ny = 0, "ny"),
        ("w_dx0.tif", |h| h.dx = 0.0, "dx"),
        ("w_dy_nan.tif", |h| h.dy = f64::NAN, "dy"),
        (
            "w_origin_inf.tif",
            |h| h.origin_y = f64::INFINITY,
            "origin_y",
        ),
    ];
    for (name, mutate, want) in cases {
        let mut g = base.clone();
        mutate(&mut g.header);
        let e = write_err(&g, name);
        assert!(
            matches!(e, GridError::InvalidDimension { field, .. } if field == want),
            "{name}: {e:?}"
        );
    }
}

#[test]
fn writer_rejects_wide_epsg() {
    let mut g = grid_300x257();
    g.header.epsg = 70_000;
    let e = write_err(&g, "w_epsg.tif");
    assert!(
        matches!(e, GridError::EpsgOutOfRange { epsg: 70_000 }),
        "{e:?}"
    );
}

#[test]
fn writer_rejects_wrong_data_length() {
    let mut g = grid_300x257();
    g.data.pop();
    let e = write_err(&g, "w_len.tif");
    assert!(
        matches!(
            e,
            GridError::DataLength {
                expected: 77_100,
                found: 77_099,
                ..
            }
        ),
        "{e:?}"
    );
}

#[test]
fn writer_reports_io_error() {
    let path = tmp("no_such_dir").join("out.tif");
    let e = write_cog(&path, &grid_300x257()).unwrap_err();
    assert!(matches!(e, GridError::Io { .. }), "{e:?}");
}
