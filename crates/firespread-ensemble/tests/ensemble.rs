//! Determinism, draw independence and the FBFM40 map of the ensemble crate.

use std::collections::BTreeSet;
use std::f32::consts::PI;

use firespread_ensemble::{
    BURNABLE_FBFM40, Counts, EnsembleError, Fbfm40, Inputs, Layers, MemberBuffers, MemberFields,
    MemberParams, Perturbation, classify_fbfm40, run_ensemble, run_member,
};
use firespread_fuel::{Moistures, by_code, scott_burgan_40};
use firespread_grid::GridHeader;
use firespread_rothermel::WindLimit;
use firespread_spread::Workspace;

const MOIST: Moistures = Moistures {
    dead_1h: 0.03,
    dead_10h: 0.04,
    dead_100h: 0.05,
    live_herb: 0.30,
    live_woody: 0.60,
};

const N: usize = 100;
const HORIZONS_S: [f32; 3] = [300.0, 900.0, 1800.0];
const SEED: u64 = 7;
const MEMBERS: u32 = 16;

fn header(nx: usize, ny: usize) -> GridHeader {
    GridHeader {
        nx,
        ny,
        dx: 30.0,
        dy: 30.0,
        origin_x: 0.0,
        origin_y: 0.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

/// GR2 with a TU1 block, a water strip and a non-burnable column, on a 10°
/// east-facing slope, ignited in a 3x3 block at the centre.
fn synthetic() -> Inputs {
    let mut fbfm40 = vec![102.0_f32; N * N];
    let mut water = vec![0_u8; N * N];
    let mut ignited = vec![0_u8; N * N];
    for r in 0..N {
        for c in 0..N {
            let i = r * N + c;
            if r < 30 && c < 30 {
                fbfm40[i] = 161.0;
            }
            if c == 75 {
                fbfm40[i] = 91.0;
            }
            if r == 20 && c > 40 {
                water[i] = 1;
            }
            if (49..=51).contains(&r) && (49..=51).contains(&c) {
                ignited[i] = 1;
            }
        }
    }
    let slope = vec![10.0_f32; N * N];
    let aspect = vec![90.0_f32; N * N];
    Inputs::from_layers(
        &Layers {
            header: header(N, N),
            fbfm40: &fbfm40,
            slope_deg: &slope,
            aspect_deg: &aspect,
            water: &water,
            ignited: &ignited,
        },
        &MOIST,
        WindLimit::None,
    )
    .unwrap()
}

fn perturbation() -> Perturbation {
    Perturbation {
        base: MemberParams {
            midflame_wind_m_s: 3.0,
            wind_toward_azimuth_rad: std::f64::consts::FRAC_PI_4,
        },
        speed_sd_m_s: 1.5,
        direction_sd_rad: 0.5,
    }
}

fn run_on(threads: usize, inputs: &Inputs) -> Counts {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
        .install(|| run_ensemble(inputs, &perturbation(), SEED, MEMBERS, &HORIZONS_S))
        .unwrap()
}

#[test]
fn same_seed_gives_identical_counts_on_one_and_four_threads() {
    let inputs = synthetic();
    let one = run_on(1, &inputs);
    let four = run_on(4, &inputs);
    let again = run_on(4, &inputs);
    // Non-degenerate: some cell burned by some but not all members, and the
    // members burned different areas.
    let partial = one.counts[2]
        .iter()
        .filter(|&&c| c > 0 && u32::from(c) < MEMBERS)
        .count();
    assert!(partial > 0, "no partially burned cell: the test is vacuous");
    let areas: BTreeSet<usize> = one.summaries.iter().map(|s| s.burned_cells[2]).collect();
    assert!(areas.len() > 1, "every member burned {areas:?} cells");
    assert!(
        one.summaries
            .iter()
            .all(|s| s.burned_cells[0] <= s.burned_cells[2]),
        "burned area shrinks with horizon"
    );

    for (k, (a, b)) in one.counts.iter().zip(&four.counts).enumerate() {
        let diff = a.iter().zip(b).filter(|(x, y)| x != y).count();
        assert_eq!(diff, 0, "horizon {k}: {diff} cells differ, 1 vs 4 threads");
    }
    for (i, (a, b)) in one.summaries.iter().zip(&four.summaries).enumerate() {
        assert_eq!(a, b, "member {i}: 1 thread {a:?}, 4 threads {b:?}");
    }
    assert_eq!(one, four);
    assert_eq!(four, again);
}

#[test]
fn member_draws_depend_on_seed_and_index_only() {
    let inputs = synthetic();
    let p = perturbation();
    for threads in [1, 4] {
        let counts = run_on(threads, &inputs);
        for (i, s) in counts.summaries.iter().enumerate() {
            let want = p.draw(SEED, u32::try_from(i).unwrap()).unwrap();
            assert_eq!(
                s.params, want,
                "{threads} threads, member {i}: ran {:?}, draw gives {want:?}",
                s.params
            );
        }
    }
    let draws: BTreeSet<u64> = (0..MEMBERS)
        .map(|i| p.draw(SEED, i).unwrap().midflame_wind_m_s.to_bits())
        .collect();
    assert_eq!(draws.len(), MEMBERS as usize, "members drew equal speeds");
    assert_ne!(p.draw(SEED, 0).unwrap(), p.draw(SEED + 1, 0).unwrap());
}

#[test]
fn probability_is_count_over_members() {
    let counts = run_on(2, &synthetic());
    for k in 0..HORIZONS_S.len() {
        let p = counts.probability(k).unwrap();
        for (cell, (&c, &q)) in counts.counts[k].iter().zip(&p).enumerate() {
            assert_eq!(q, f32::from(c) / MEMBERS as f32, "horizon {k} cell {cell}");
        }
    }
    assert!(counts.probability(HORIZONS_S.len()).is_none());
}

#[test]
fn fbfm40_map_covers_the_40_burnable_models() {
    let codes: BTreeSet<u16> = BURNABLE_FBFM40.iter().map(|&(c, _)| c).collect();
    assert_eq!(codes.len(), 40);
    let named: BTreeSet<&str> = BURNABLE_FBFM40.iter().map(|&(_, m)| m).collect();
    let table: BTreeSet<String> = scott_burgan_40()
        .unwrap()
        .into_iter()
        .map(|m| m.code)
        .collect();
    assert_eq!(
        named,
        table.iter().map(String::as_str).collect(),
        "FBFM40 map vs Scott and Burgan table"
    );
    for (i, &(code, model)) in BURNABLE_FBFM40.iter().enumerate() {
        assert_eq!(classify_fbfm40(code), Some(Fbfm40::Burnable(i)), "{code}");
        by_code(model).unwrap();
    }
    let ranges = [
        (101..=109, "GR"),
        (121..=124, "GS"),
        (141..=149, "SH"),
        (161..=165, "TU"),
        (181..=189, "TL"),
        (201..=204, "SB"),
    ];
    for (range, prefix) in ranges {
        for (k, code) in range.enumerate() {
            let Some(Fbfm40::Burnable(i)) = classify_fbfm40(code) else {
                panic!("{code} is not burnable");
            };
            assert_eq!(BURNABLE_FBFM40[i].1, format!("{prefix}{}", k + 1), "{code}");
        }
    }
    for code in 91..=99 {
        assert_eq!(classify_fbfm40(code), Some(Fbfm40::NonBurnable), "{code}");
    }
    for code in [0, 90, 100, 110, 120, 125, 150, 166, 190, 205, 255] {
        assert_eq!(classify_fbfm40(code), None, "{code}");
    }
}

#[test]
fn lazy_member_matches_filled_fields_bit_for_bit() {
    let inputs = synthetic();
    let p = perturbation();
    let horizon = HORIZONS_S[2];
    let mut ws = Workspace::new();
    let mut eager = MemberBuffers::new(&inputs);
    let mut lazy = MemberBuffers::new(&inputs);
    let mut burned = BTreeSet::new();
    for member in 0..MEMBERS {
        let params = p.draw(SEED, member).unwrap();
        eager.fill_fields(&inputs, &params).unwrap();
        let eager_stats = eager.spread(&inputs, &mut ws, horizon).unwrap();
        let lazy_stats = run_member(&inputs, &params, &mut ws, &mut lazy, horizon).unwrap();
        let (a, b) = (eager.arrival_s(), lazy.arrival_s());
        let differ: Vec<usize> = (0..a.len())
            .filter(|&i| a[i].to_bits() != b[i].to_bits())
            .collect();
        assert!(
            differ.is_empty(),
            "member {member}: {} cells differ, first {:?}: eager {} lazy {}",
            differ.len(),
            differ.first(),
            differ.first().map_or(0.0, |&i| a[i]),
            differ.first().map_or(0.0, |&i| b[i])
        );
        assert_eq!(eager_stats, lazy_stats, "member {member}");
        burned.insert(lazy_stats.burned);
    }
    assert!(
        burned.len() > 1 && burned.iter().all(|&b| b > 100),
        "burned cells per member {burned:?}"
    );
}

#[test]
fn member_fields_eval_matches_fill_fields() {
    let inputs = synthetic();
    let params = perturbation().draw(SEED, 3).unwrap();
    let mut buffers = MemberBuffers::new(&inputs);
    buffers.fill_fields(&inputs, &params).unwrap();
    let provider = MemberFields::new(&inputs, params);
    for cell in 0..inputs.len() {
        let f = provider.eval(cell).unwrap();
        assert_eq!(
            (
                f.head_ros_m_s.to_bits(),
                f.eccentricity.to_bits(),
                f.max_azimuth_rad.to_bits()
            ),
            (
                buffers.head_ros_m_s()[cell].to_bits(),
                buffers.eccentricity()[cell].to_bits(),
                buffers.max_azimuth_rad()[cell].to_bits()
            ),
            "cell {cell}: eval {f:?}"
        );
    }
}

fn one_row(
    fbfm40: &[f32],
    slope: &[f32],
    aspect: &[f32],
    water: &[u8],
) -> Result<Inputs, EnsembleError> {
    let n = fbfm40.len();
    Inputs::from_layers(
        &Layers {
            header: header(n, 1),
            fbfm40,
            slope_deg: slope,
            aspect_deg: aspect,
            water,
            ignited: &vec![0; n],
        },
        &MOIST,
        WindLimit::None,
    )
}

#[test]
fn terrain_and_burnable_mask_follow_the_layers() {
    // Cells: east-facing 45° slope; flat (-1) with a nonzero slope value;
    // north-facing 20°; water; NB1; nodata.
    let inputs = one_row(
        &[102.0, 102.0, 102.0, 102.0, 91.0, -9999.0],
        &[45.0, 30.0, 20.0, 5.0, 5.0, 5.0],
        &[90.0, -1.0, 0.0, 90.0, 90.0, 90.0],
        &[0, 0, 0, 2, 0, 0],
    )
    .unwrap();
    assert_eq!(inputs.burnable(), &[true, true, true, false, false, false]);
    let mut buffers = MemberBuffers::new(&inputs);
    buffers
        .fill_fields(
            &inputs,
            &MemberParams {
                midflame_wind_m_s: 0.0,
                wind_toward_azimuth_rad: 0.0,
            },
        )
        .unwrap();
    let az = buffers.max_azimuth_rad();
    let ecc = buffers.eccentricity();
    let head = buffers.head_ros_m_s();
    // Calm: the head points upslope, opposite the aspect.
    assert!((az[0] - 1.5 * PI).abs() < 1e-6, "east-facing: {}", az[0]);
    assert!((az[2] - PI).abs() < 1e-6, "north-facing: {}", az[2]);
    // Flat: no slope effect, a circle at R_0, below the sloped cells' rates.
    assert_eq!((az[1], ecc[1]), (0.0, 0.0));
    assert!(ecc[0] > 0.0 && head[0] > head[2] && head[2] > head[1]);
    assert_eq!(&head[3..], &[0.0, 0.0, 0.0]);
}

#[test]
fn bad_layers_are_errors() {
    let err = one_row(&[150.0], &[5.0], &[90.0], &[0]).unwrap_err();
    assert!(
        matches!(err, EnsembleError::UnknownFuelCode { index: 0, code } if code == 150.0),
        "{err}"
    );
    let err = one_row(&[101.5], &[5.0], &[90.0], &[0]).unwrap_err();
    assert!(
        matches!(err, EnsembleError::UnknownFuelCode { .. }),
        "{err}"
    );
    let err = one_row(&[101.0], &[90.0], &[90.0], &[0]).unwrap_err();
    assert!(
        matches!(
            err,
            EnsembleError::InvalidTerrain {
                layer: "slope_deg",
                ..
            }
        ),
        "{err}"
    );
    let err = one_row(&[101.0], &[5.0], &[-2.0], &[0]).unwrap_err();
    assert!(
        matches!(
            err,
            EnsembleError::InvalidTerrain {
                layer: "aspect_deg",
                ..
            }
        ),
        "{err}"
    );
    // Terrain at non-burnable cells is not read.
    one_row(&[91.0], &[f32::NAN], &[-7.0], &[0]).unwrap();
    let err = one_row(&[101.0, 101.0], &[5.0], &[90.0, 90.0], &[0, 0]).unwrap_err();
    assert!(
        matches!(
            err,
            EnsembleError::LayerLength {
                layer: "slope_deg",
                expected: 2,
                got: 1
            }
        ),
        "{err}"
    );
}

#[test]
fn run_ensemble_rejects_bad_arguments() {
    let inputs = synthetic();
    let p = perturbation();
    assert!(matches!(
        run_ensemble(&inputs, &p, SEED, 0, &HORIZONS_S),
        Err(EnsembleError::Empty)
    ));
    assert!(matches!(
        run_ensemble(&inputs, &p, SEED, 65_536, &HORIZONS_S),
        Err(EnsembleError::TooManyMembers { .. })
    ));
    assert!(matches!(
        run_ensemble(&inputs, &p, SEED, 1, &[]),
        Err(EnsembleError::NoHorizons)
    ));
    assert!(matches!(
        run_ensemble(&inputs, &p, SEED, 1, &[60.0, f32::NAN]),
        Err(EnsembleError::InvalidHorizon { index: 1, .. })
    ));
    let bad = Perturbation {
        speed_sd_m_s: -1.0,
        ..p
    };
    assert!(matches!(
        run_ensemble(&inputs, &bad, SEED, 1, &HORIZONS_S),
        Err(EnsembleError::InvalidPerturbation {
            field: "speed sd",
            ..
        })
    ));
}
