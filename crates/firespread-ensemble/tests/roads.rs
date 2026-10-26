//! Road codes listed as barriers become non-burnable through the water layer
//! and stop the spread; unlisted codes do not.

use firespread_ensemble::{
    DeadFuelMoisture, HourFields, HourlyConfig, HourlyInputs, HourlyLayers, HourlyPerturbation,
    Layers, ROAD_BARRIER_WATER_CODE, RawHour, WindowWeather, apply_road_barriers,
    run_hourly_member,
};
use firespread_grid::GridHeader;
use firespread_spread::Workspace;

const NX: usize = 20;
const NY: usize = 40;
const HOURS: u32 = 3;
/// Row of a full-width primary road (code 2).
const PRIMARY_ROW: usize = 10;
/// Row of a full-width residential road (code 5).
const RESIDENTIAL_ROW: usize = 20;

fn header() -> GridHeader {
    GridHeader {
        nx: NX,
        ny: NY,
        dx: 30.0,
        dy: 30.0,
        origin_x: 0.0,
        origin_y: 0.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

/// Wind toward the south (increasing row), 300 K, 20 % RH, clear.
fn weather(n: usize) -> WindowWeather {
    let hour = HourFields::from_raw(
        &RawHour {
            u10_m_s: &vec![0.0; n],
            v10_m_s: &vec![-8.0; n],
            t2m_k: &vec![300.0; n],
            rh2m_pct: &vec![20.0; n],
            tcc_pct: &vec![0.0; n],
        },
        &DeadFuelMoisture::default(),
        n,
    )
    .unwrap();
    WindowWeather {
        hours: vec![hour; HOURS as usize],
        hours_utc: (0..HOURS).map(|k| format!("hour {k}")).collect(),
        fm100_pct: vec![5.0; n],
        fm1000_pct: vec![7.0; n],
    }
}

fn roads() -> Vec<u8> {
    let mut roads = vec![0_u8; NX * NY];
    for c in 0..NX {
        roads[PRIMARY_ROW * NX + c] = 2;
        roads[RESIDENTIAL_ROW * NX + c] = 5;
    }
    roads
}

/// The deepest row with a finite arrival time after `HOURS` hours of flat
/// GR2 ignited along row 2, with `codes` as barrier road codes, and the
/// number of cells the barrier changed.
fn deepest_burned_row(codes: &[u8]) -> (usize, usize) {
    let n = NX * NY;
    let mut water = vec![0_u8; n];
    let changed = apply_road_barriers(&mut water, &roads(), codes).unwrap();
    let mut ignited = vec![0_u8; n];
    for c in 0..NX {
        ignited[2 * NX + c] = 1;
    }
    let inputs = HourlyInputs::from_layers(
        &HourlyLayers {
            base: Layers {
                header: header(),
                fbfm40: &vec![102.0; n],
                slope_deg: &vec![0.0; n],
                aspect_deg: &vec![-1.0; n],
                water: &water,
                ignited: &ignited,
            },
            canopy_height_m: &vec![0.0; n],
            canopy_base_height_m: &vec![0.0; n],
            canopy_cover_pct: &vec![0.0; n],
        },
        weather(n),
        &HourlyConfig::default(),
    )
    .unwrap();
    let draw = HourlyPerturbation {
        ignition_radius_m: 0.0,
        wind_speed_sd: 0.0,
        wind_direction_sd_rad: 0.0,
        dead_moisture_log_sd: 0.0,
    }
    .draw(1, 0, 30.0)
    .unwrap();
    let mut arrival = Vec::new();
    run_hourly_member(&inputs, &draw, &mut Workspace::new(), &mut arrival, HOURS).unwrap();
    let deepest = (0..n)
        .filter(|&i| arrival[i].is_finite())
        .map(|i| i / NX)
        .max()
        .unwrap();
    (deepest, changed)
}

#[test]
fn apply_road_barriers_marks_listed_codes_only() {
    let mut water = vec![0_u8, 1, 0, 0, 2, 0];
    let roads = [2_u8, 2, 5, 0, 0, 3];
    let changed = apply_road_barriers(&mut water, &roads, &[2, 3]).unwrap();
    assert_eq!(changed, 2, "cells changed");
    assert_eq!(
        water,
        [ROAD_BARRIER_WATER_CODE, 1, 0, 0, 2, ROAD_BARRIER_WATER_CODE],
        "water after barriers 2 and 3"
    );
    let mut unchanged = vec![0_u8; 6];
    assert_eq!(apply_road_barriers(&mut unchanged, &roads, &[]).unwrap(), 0);
    assert_eq!(unchanged, [0; 6], "empty code list changed the water layer");
    assert!(apply_road_barriers(&mut unchanged, &roads[..5], &[2]).is_err());
}

#[test]
fn listed_road_codes_stop_the_spread() {
    let (none, changed) = deepest_burned_row(&[]);
    assert_eq!(changed, 0);
    assert!(
        none > RESIDENTIAL_ROW,
        "with no barrier codes the fire reached row {none}, expected past row {RESIDENTIAL_ROW}"
    );

    let (primary, changed) = deepest_burned_row(&[2]);
    assert_eq!(changed, NX, "cells of code 2");
    assert!(
        primary < PRIMARY_ROW,
        "with code 2 a barrier the fire reached row {primary}, expected above row {PRIMARY_ROW}"
    );

    let (residential, changed) = deepest_burned_row(&[5]);
    assert_eq!(changed, NX, "cells of code 5");
    assert!(
        residential > PRIMARY_ROW && residential < RESIDENTIAL_ROW,
        "with code 5 a barrier the fire reached row {residential}, expected between rows \
         {PRIMARY_ROW} and {RESIDENTIAL_ROW}"
    );

    let (others, changed) = deepest_burned_row(&[1, 3, 4, 6]);
    assert_eq!(changed, 0, "cells of codes 1, 3, 4, 6");
    assert_eq!(
        others, none,
        "codes absent from the grid changed the spread: row {others} vs {none}"
    );
}
