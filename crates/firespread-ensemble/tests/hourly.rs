//! Hourly windows, member draws and determinism of the hourly ensemble.

use std::collections::BTreeSet;

use firespread_ensemble::{
    DeadFuelMoisture, HourFields, HourlyConfig, HourlyCounts, HourlyInputs, HourlyLayers,
    HourlyPerturbation, Layers, MemberDraw, RawHour, WindowWeather, run_hourly_ensemble,
    run_hourly_member,
};
use firespread_grid::GridHeader;
use firespread_spread::Workspace;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Normal, StandardUniform};

const DX: f64 = 30.0;

fn header(nx: usize, ny: usize) -> GridHeader {
    GridHeader {
        nx,
        ny,
        dx: DX,
        dy: DX,
        origin_x: 0.0,
        origin_y: 0.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

/// One hour of uniform weather: wind `(u10, v10)` m/s, 300 K, 20 % RH, clear.
fn hour(n: usize, u10: f32, v10: f32) -> HourFields {
    HourFields::from_raw(
        &RawHour {
            u10_m_s: &vec![u10; n],
            v10_m_s: &vec![v10; n],
            t2m_k: &vec![300.0; n],
            rh2m_pct: &vec![20.0; n],
            tcc_pct: &vec![0.0; n],
        },
        &DeadFuelMoisture::default(),
        n,
    )
    .unwrap()
}

fn weather(n: usize, winds: &[(f32, f32)]) -> WindowWeather {
    WindowWeather {
        hours: winds.iter().map(|&(u, v)| hour(n, u, v)).collect(),
        hours_utc: (0..winds.len()).map(|k| format!("hour {k}")).collect(),
        fm100_pct: vec![5.0; n],
        fm1000_pct: vec![7.0; n],
    }
}

/// Flat GR2 with no canopy, ignited at `ignited`.
fn flat_gr2(nx: usize, ny: usize, ignited: &[usize], weather: WindowWeather) -> HourlyInputs {
    let n = nx * ny;
    let mut mask = vec![0_u8; n];
    for &i in ignited {
        mask[i] = 1;
    }
    HourlyInputs::from_layers(
        &HourlyLayers {
            base: Layers {
                header: header(nx, ny),
                fbfm40: &vec![102.0; n],
                slope_deg: &vec![0.0; n],
                aspect_deg: &vec![-1.0; n],
                water: &vec![0; n],
                ignited: &mask,
            },
            canopy_height_m: &vec![0.0; n],
            canopy_base_height_m: &vec![0.0; n],
            canopy_cover_pct: &vec![0.0; n],
        },
        weather,
        &HourlyConfig::default(),
    )
    .unwrap()
}

const NO_PERTURBATION: MemberDraw = MemberDraw {
    ignition_east_m: 0.0,
    ignition_north_m: 0.0,
    ignition_shift_rows: 0,
    ignition_shift_cols: 0,
    wind_speed_factor: 1.0,
    wind_direction_offset_rad: 0.0,
    dead_moisture_factor: 1.0,
};

/// Easternmost and westernmost column offsets from `col` along row `row`
/// with arrival time `<= t`.
fn extent(arrival: &[f32], nx: usize, row: usize, col: usize, t: f32) -> (usize, usize) {
    let burned = |c: usize| arrival[row * nx + c] <= t;
    let east = (col..nx).take_while(|&c| burned(c)).count() - 1;
    let west = (0..=col).rev().take_while(|&c| burned(c)).count() - 1;
    (east, west)
}

#[test]
fn hour_two_wind_reversal_shows_in_the_arrival_field() {
    let (nx, ny) = (400, 21);
    let n = nx * ny;
    let (row, col) = (10, 200);
    let centre = row * nx + col;
    // 10 m wind 5.75 m/s toward east in hour 1, toward west in hour 2.
    let reversed = flat_gr2(nx, ny, &[centre], weather(n, &[(5.75, 0.0), (-5.75, 0.0)]));
    let steady = flat_gr2(nx, ny, &[centre], weather(n, &[(5.75, 0.0), (5.75, 0.0)]));

    // Head and backing rates of hour 1 at the ignition cell, m/s.
    let f = reversed.cell_fields(0, &NO_PERTURBATION, centre).unwrap();
    let head = f64::from(f.head_ros_m_s);
    let e = f64::from(f.eccentricity);
    let back = head * (1.0 - e) / (1.0 + e);
    let head_cells = head * 3600.0 / DX;
    let back_cells = back * 3600.0 / DX;
    assert!(
        head_cells - back_cells > 10.0 && head_cells < 190.0,
        "head {head_cells:.1} and backing {back_cells:.1} cells per hour: test is vacuous or too small"
    );

    let mut ws = Workspace::new();
    let mut one = Vec::new();
    let mut two = Vec::new();
    let mut two_steady = Vec::new();
    run_hourly_member(&reversed, &NO_PERTURBATION, &mut ws, &mut one, 1).unwrap();
    run_hourly_member(&reversed, &NO_PERTURBATION, &mut ws, &mut two, 2).unwrap();
    run_hourly_member(&steady, &NO_PERTURBATION, &mut ws, &mut two_steady, 2).unwrap();

    // Hour 1 of the two-hour run is the one-hour run. Hour 2 may set cells
    // to exactly 3600 s (candidates clamp to the window start), so the
    // comparison is over arrival < 3600 s.
    let before = |t: f32| t < 3600.0;
    let differ = (0..n)
        .filter(|&i| before(one[i]) != before(two[i]) || (before(one[i]) && one[i] != two[i]))
        .count();
    assert_eq!(differ, 0, "{differ} cells differ in hour 1");

    let (east1, west1) = extent(&one, nx, row, col, 3600.0);
    let (east2, west2) = extent(&two, nx, row, col, 7200.0);
    let tol = 2.0;
    assert!(
        (east1 as f64 - head_cells).abs() <= tol && (west1 as f64 - back_cells).abs() <= tol,
        "hour 1: east {east1} west {west1}, expected head {head_cells:.2} back {back_cells:.2}"
    );
    // Hour 2 reversed: the west flank runs at the head rate, the east at the
    // backing rate.
    let (gain_east, gain_west) = ((east2 - east1) as f64, (west2 - west1) as f64);
    assert!(
        (gain_west - head_cells).abs() <= tol && (gain_east - back_cells).abs() <= tol,
        "hour 2 reversed: gained east {gain_east} west {gain_west}, \
         expected east {back_cells:.2} west {head_cells:.2}"
    );
    // Steady wind for contrast: hour 2 keeps running east.
    let (east_s, west_s) = extent(&two_steady, nx, row, col, 7200.0);
    assert!(
        (east_s as f64 - 2.0 * head_cells).abs() <= 2.0 * tol
            && (west_s as f64 - 2.0 * back_cells).abs() <= 2.0 * tol,
        "steady: east {east_s} west {west_s}"
    );
}

/// The draws of `HourlyPerturbation::draw` recomputed in its documented
/// order: U, U, N(1, sd), N(0, sd), N(0, log sd).
fn documented_draw(p: &HourlyPerturbation, seed: u64, member: u32) -> MemberDraw {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    rng.set_stream(u64::from(member));
    let a: f64 = StandardUniform.sample(&mut rng);
    let b: f64 = StandardUniform.sample(&mut rng);
    let s = Normal::new(1.0, p.wind_speed_sd).unwrap().sample(&mut rng);
    let d = Normal::new(0.0, p.wind_direction_sd_rad)
        .unwrap()
        .sample(&mut rng);
    let m = Normal::new(0.0, p.dead_moisture_log_sd)
        .unwrap()
        .sample(&mut rng);
    let r = p.ignition_radius_m * a.sqrt();
    let theta = std::f64::consts::TAU * b;
    let (east, north) = (r * theta.sin(), r * theta.cos());
    MemberDraw {
        ignition_east_m: east,
        ignition_north_m: north,
        ignition_shift_rows: -(north / DX).round() as i64,
        ignition_shift_cols: (east / DX).round() as i64,
        wind_speed_factor: s.max(0.0),
        wind_direction_offset_rad: d,
        dead_moisture_factor: m.exp(),
    }
}

#[test]
fn draws_follow_the_documented_order() {
    let p = HourlyPerturbation::default();
    for member in 0..32 {
        let got = p.draw(7, member, DX).unwrap();
        let want = documented_draw(&p, 7, member);
        assert_eq!(got, want, "member {member}");
        let r = got.ignition_east_m.hypot(got.ignition_north_m);
        assert!(r <= 375.0, "member {member}: offset {r} m");
    }
    assert_ne!(p.draw(7, 0, DX).unwrap(), p.draw(8, 0, DX).unwrap());
}

const MEMBERS: u32 = 24;
const SEED: u64 = 7;

fn varied() -> HourlyInputs {
    let (nx, ny) = (90, 90);
    let n = nx * ny;
    let mut fbfm40 = vec![102.0_f32; n];
    let mut ch = vec![0.0_f32; n];
    let mut cbh = vec![0.0_f32; n];
    let mut cc = vec![0.0_f32; n];
    let mut mask = vec![0_u8; n];
    for r in 0..ny {
        for c in 0..nx {
            let i = r * nx + c;
            if r < 30 {
                fbfm40[i] = 165.0;
                ch[i] = 15.0;
                cbh[i] = 3.0;
                cc[i] = 40.0;
            }
            if c == 70 {
                fbfm40[i] = 91.0;
            }
            if (44..=46).contains(&r) && (44..=46).contains(&c) {
                mask[i] = 1;
            }
        }
    }
    let slope = vec![12.0_f32; n];
    let aspect = vec![135.0_f32; n];
    HourlyInputs::from_layers(
        &HourlyLayers {
            base: Layers {
                header: header(nx, ny),
                fbfm40: &fbfm40,
                slope_deg: &slope,
                aspect_deg: &aspect,
                water: &vec![0; n],
                ignited: &mask,
            },
            canopy_height_m: &ch,
            canopy_base_height_m: &cbh,
            canopy_cover_pct: &cc,
        },
        weather(n, &[(4.0, 3.0), (2.0, -4.0)]),
        &HourlyConfig::default(),
    )
    .unwrap()
}

fn run_on(threads: usize, inputs: &HourlyInputs) -> HourlyCounts {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| {
            run_hourly_ensemble(
                inputs,
                &HourlyPerturbation::default(),
                SEED,
                MEMBERS,
                &[1, 2],
            )
        })
        .unwrap()
}

#[test]
fn per_member_draws_are_identical_across_runs_and_thread_counts() {
    let inputs = varied();
    let one = run_on(1, &inputs);
    let twelve = run_on(12, &inputs);
    let again = run_on(12, &inputs);
    let p = HourlyPerturbation::default();
    for (i, ((a, b), c)) in one
        .summaries
        .iter()
        .zip(&twelve.summaries)
        .zip(&again.summaries)
        .enumerate()
    {
        assert_eq!(a.draw, b.draw, "member {i}: 1 vs 12 threads");
        assert_eq!(b.draw, c.draw, "member {i}: run 2 vs run 3");
        let want = p.draw(SEED, u32::try_from(i).unwrap(), DX).unwrap();
        assert_eq!(a.draw, want, "member {i} ran another member's draws");
        assert_eq!(a, b, "member {i} summary, 1 vs 12 threads");
    }
    assert_eq!(one, twelve);
    assert_eq!(twelve, again);

    // Non-degenerate: members drew distinct values and burned different
    // areas, and some cell burned in some but not all members.
    let draws: BTreeSet<[u64; 4]> = one
        .summaries
        .iter()
        .map(|s| {
            [
                s.draw.ignition_east_m.to_bits(),
                s.draw.wind_speed_factor.to_bits(),
                s.draw.wind_direction_offset_rad.to_bits(),
                s.draw.dead_moisture_factor.to_bits(),
            ]
        })
        .collect();
    assert_eq!(draws.len(), MEMBERS as usize);
    let areas: BTreeSet<usize> = one.summaries.iter().map(|s| s.burned_cells[1]).collect();
    assert!(areas.len() > 1, "every member burned {areas:?} cells");
    let partial = one.counts[1]
        .iter()
        .filter(|&&c| c > 0 && u32::from(c) < MEMBERS)
        .count();
    assert!(partial > 0, "no partially burned cell");
    assert!(
        one.summaries
            .iter()
            .all(|s| s.burned_cells[0] <= s.burned_cells[1] && s.burned_cells[1] < 90 * 90),
        "burned area shrinks with horizon or fills the grid"
    );
}

#[test]
fn ensemble_needs_enough_hours() {
    let inputs = varied();
    let err =
        run_hourly_ensemble(&inputs, &HourlyPerturbation::default(), SEED, 2, &[1, 3]).unwrap_err();
    assert!(
        matches!(
            err,
            firespread_ensemble::EnsembleError::TooFewHours {
                available: 2,
                horizon_h: 3
            }
        ),
        "{err}"
    );
}
