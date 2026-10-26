//! `uniform_growth` against an all-pairs brute-force distance check on seeded
//! random masks.

use firespread_metrics::{Mask, uniform_growth};

/// SplitMix64 step, a seeded generator for test masks.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn brute_force(start: &Mask, dx_m: f64, radius_m: f64) -> Vec<bool> {
    let (rows, cols) = (start.rows(), start.cols());
    let sources: Vec<(i64, i64)> = (0..rows * cols)
        .filter(|&k| start.cells()[k])
        .map(|k| ((k / cols) as i64, (k % cols) as i64))
        .collect();
    let limit = radius_m * radius_m;
    (0..rows * cols)
        .map(|k| {
            let (i, j) = ((k / cols) as i64, (k % cols) as i64);
            sources.iter().any(|&(si, sj)| {
                let d2 = ((i - si).pow(2) + (j - sj).pow(2)) as f64;
                d2 * dx_m * dx_m <= limit
            })
        })
        .collect()
}

#[test]
fn uniform_growth_matches_brute_force() {
    let mut state = 0x5EED_2026_u64;
    let shapes = [(1, 1), (1, 9), (9, 1), (5, 7), (13, 11), (24, 31)];
    // Per-mille densities of start cells.
    let densities = [0_u64, 5, 30, 200, 1000];
    // (dx_m, rate_m_per_h, hours): radii 0, 30, 45, 60, 105, 300 m on 30 m cells.
    let params = [
        (30.0, 0.0, 3.0),
        (30.0, 30.0, 1.0),
        (30.0, 15.0, 3.0),
        (30.0, 30.0, 2.0),
        (30.0, 35.0, 3.0),
        (30.0, 100.0, 3.0),
    ];
    let mut checked = 0;
    for &(rows, cols) in &shapes {
        for &density in &densities {
            for _ in 0..4 {
                let cells = (0..rows * cols)
                    .map(|_| next(&mut state) % 1000 < density)
                    .collect();
                let start = Mask::new(rows, cols, cells).unwrap();
                for &(dx, rate, hours) in &params {
                    let got = uniform_growth(&start, dx, rate, hours).unwrap();
                    let want = brute_force(&start, dx, rate * hours);
                    for (k, (&g, &w)) in got.cells().iter().zip(&want).enumerate() {
                        assert_eq!(
                            g,
                            w,
                            "{rows}x{cols} density {density}/1000 radius {} m: cell ({}, {}) got {g}, brute force {w}",
                            rate * hours,
                            k / cols,
                            k % cols
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 6 * 5 * 4 * 6);
}
