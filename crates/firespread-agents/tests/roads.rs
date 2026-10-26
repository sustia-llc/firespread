//! Road barrier codes passed through `Source::Files` make the listed road
//! cells of `grid/roads.tif` non-burnable; unlisted codes and an empty list
//! leave them burnable.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use firespread_agents::{EnsembleSpec, ForecastConfig, Forecaster, Source};
use firespread_ensemble::{
    DeadFuelMoisture, HourFields, HourlyConfig, HourlyInputs, HourlyPerturbation,
    ROAD_BARRIER_WATER_CODE, RawHour, WindowWeather,
};
use firespread_grid::{ForecastWindow, Grid, GridHeader, write_cog};
use tiff::encoder::colortype::Gray8;
use tiff::encoder::{Compression, TiffEncoder};
use tiff::tags::Tag;

const NX: usize = 10;
const NY: usize = 8;
/// Row of a full-width primary road (code 2).
const PRIMARY_ROW: usize = 3;
/// Row of a full-width residential road (code 5).
const RESIDENTIAL_ROW: usize = 6;
/// Column of the one water cell on the primary road.
const WATER_COL: usize = 4;

fn header() -> GridHeader {
    GridHeader {
        nx: NX,
        ny: NY,
        dx: 30.0,
        dy: 30.0,
        origin_x: -876_180.0,
        origin_y: 1_499_820.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

/// Writes `data` as a single-band u8 GeoTIFF on [`header`].
fn write_u8(path: &Path, data: &[u8]) {
    let h = header();
    let mut buf = Cursor::new(Vec::new());
    {
        let mut enc = TiffEncoder::new(&mut buf)
            .unwrap()
            .with_compression(Compression::Uncompressed);
        let mut img = enc.new_image::<Gray8>(NX as u32, NY as u32).unwrap();
        let d = img.encoder();
        d.write_tag(Tag::ModelPixelScaleTag, &[h.dx, h.dy, 0.0][..])
            .unwrap();
        d.write_tag(
            Tag::ModelTiepointTag,
            &[0.0, 0.0, 0.0, h.origin_x, h.origin_y, 0.0][..],
        )
        .unwrap();
        d.write_tag(
            Tag::GeoKeyDirectoryTag,
            &[
                1_u16, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 5070,
            ][..],
        )
        .unwrap();
        d.write_tag(Tag::GdalNodata, "255").unwrap();
        img.write_data(data).unwrap();
    }
    fs::write(path, buf.into_inner()).unwrap();
}

fn write_f32(path: &Path, value: f32) {
    write_cog(path, &Grid::new(header(), vec![value; NX * NY]).unwrap()).unwrap();
}

/// A data directory of flat GR2 with a water cell at
/// (`PRIMARY_ROW`, `WATER_COL`), road code 2 along `PRIMARY_ROW`, road code 5
/// along `RESIDENTIAL_ROW`, and an ignition at cell 0.
fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "firespread-agents-roads-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    let grid = dir.join("grid");
    fs::create_dir_all(&grid).unwrap();
    fs::create_dir_all(dir.join("obs")).unwrap();
    write_f32(&grid.join("FBFM40.tif"), 102.0);
    write_f32(&grid.join("SlpD.tif"), 0.0);
    write_f32(&grid.join("Asp.tif"), -1.0);
    for layer in ["CH.tif", "CBH.tif", "CC.tif"] {
        write_f32(&grid.join(layer), 0.0);
    }
    let n = NX * NY;
    let mut water = vec![0_u8; n];
    water[PRIMARY_ROW * NX + WATER_COL] = 1;
    write_u8(&grid.join("water.tif"), &water);
    let mut roads = vec![0_u8; n];
    for c in 0..NX {
        roads[PRIMARY_ROW * NX + c] = 2;
        roads[RESIDENTIAL_ROW * NX + c] = 5;
    }
    write_u8(&grid.join("roads.tif"), &roads);
    let mut start = vec![0_u8; n];
    start[0] = 1;
    write_u8(&dir.join("obs/start.tif"), &start);
    dir
}

fn config(name: &str, runs_dir: PathBuf) -> ForecastConfig {
    ForecastConfig {
        run_id: name.to_string(),
        runs_dir,
        window: ForecastWindow {
            name: "test".to_string(),
            start_t: "2022-04-22T12:00:00".to_string(),
            target_t: "2022-04-23T00:00:00".to_string(),
            fireids: vec![1],
        },
        hourly: HourlyConfig::default(),
        ensemble: EnsembleSpec {
            seed: 7,
            members: 1,
            horizons_h: vec![1],
            perturbation: HourlyPerturbation::default(),
        },
        threads: 1,
    }
}

fn weather() -> WindowWeather {
    let n = NX * NY;
    let hour = HourFields::from_raw(
        &RawHour {
            u10_m_s: &vec![0.0; n],
            v10_m_s: &vec![0.0; n],
            t2m_k: &vec![300.0; n],
            rh2m_pct: &vec![20.0; n],
            tcc_pct: &vec![0.0; n],
        },
        &DeadFuelMoisture::default(),
        n,
    )
    .unwrap();
    WindowWeather {
        hours: vec![hour],
        hours_utc: vec!["hour 0".to_string()],
        fm100_pct: vec![5.0; n],
        fm1000_pct: vec![7.0; n],
    }
}

/// The water layer, burnable mask and barrier cell count the files path
/// loads with `codes`.
async fn load(name: &str, codes: &[u8]) -> (Vec<u8>, Vec<bool>, usize) {
    let dir = data_dir(name);
    let forecaster = Forecaster::new(
        config(name, dir.join("runs")),
        Source::Files {
            data_dir: dir.clone(),
            start_mask: "obs/start.tif".to_string(),
            road_barrier_codes: codes.to_vec(),
        },
    )
    .await
    .unwrap();
    let grid = forecaster.grid().clone();
    let cells = forecaster.road_barrier_cells();
    forecaster.shutdown().await;
    let inputs =
        HourlyInputs::from_layers(&grid.layers(), weather(), &HourlyConfig::default()).unwrap();
    let burnable = inputs.inputs().burnable().to_vec();
    let _ = fs::remove_dir_all(dir);
    (grid.water.clone(), burnable, cells)
}

fn row<T: Copy>(v: &[T], r: usize) -> Vec<T> {
    v[r * NX..(r + 1) * NX].to_vec()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listed_road_codes_are_non_burnable_through_the_files_path() {
    let (water, burnable, cells) = load("code2", &[2]).await;
    let mut want_water = vec![ROAD_BARRIER_WATER_CODE; NX];
    want_water[WATER_COL] = 1;
    assert_eq!(
        row(&water, PRIMARY_ROW),
        want_water,
        "water along the code-2 road with codes [2]"
    );
    assert_eq!(
        cells,
        NX - 1,
        "barrier cells with codes [2]: the road row less its water cell"
    );
    assert_eq!(
        row(&burnable, PRIMARY_ROW),
        vec![false; NX],
        "burnable along the code-2 road with codes [2]"
    );
    assert_eq!(
        row(&water, RESIDENTIAL_ROW),
        vec![0; NX],
        "water along the unlisted code-5 road with codes [2]"
    );
    assert_eq!(
        row(&burnable, RESIDENTIAL_ROW),
        vec![true; NX],
        "burnable along the unlisted code-5 road with codes [2]"
    );

    let (water, burnable, cells) = load("none", &[]).await;
    assert_eq!(cells, 0, "barrier cells with no codes");
    let mut want_water = vec![0; NX];
    want_water[WATER_COL] = 1;
    assert_eq!(
        row(&water, PRIMARY_ROW),
        want_water,
        "water along the code-2 road with no codes"
    );
    assert_eq!(
        burnable.iter().filter(|&&b| !b).count(),
        1,
        "non-burnable cells with no codes: only the water cell"
    );
}
