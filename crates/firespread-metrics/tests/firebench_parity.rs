//! Parity of `jaccard` and `dice` with FireBench `jaccard_binary` and
//! `sorensen_dice_binary` on the seeded masks of `fixtures/firebench_metrics.json`.

use firespread_metrics::{Mask, dice, jaccard};
use serde::Deserialize;

const TOLERANCE: f64 = 1e-9;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    rows: usize,
    cols: usize,
    simulated: String,
    observed: String,
    jaccard: f64,
    dice: f64,
}

fn decode(rows: usize, cols: usize, bits: &str) -> Mask {
    let cells = bits
        .chars()
        .map(|c| match c {
            '0' => false,
            '1' => true,
            other => panic!("fixture mask holds {other:?}"),
        })
        .collect();
    Mask::new(rows, cols, cells).unwrap()
}

#[test]
fn jaccard_and_dice_match_firebench() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/firebench_metrics.json"
    ))
    .unwrap();
    let fixture: Fixture = serde_json::from_str(&text).unwrap();
    assert!(!fixture.cases.is_empty(), "fixture has no cases");

    let mut both_empty = 0;
    let mut failures = Vec::new();
    for (index, case) in fixture.cases.iter().enumerate() {
        let s = decode(case.rows, case.cols, &case.simulated);
        let o = decode(case.rows, case.cols, &case.observed);
        if s.count() == 0 && o.count() == 0 {
            both_empty += 1;
        }
        for (name, ours, theirs) in [
            ("jaccard", jaccard(&s, &o).unwrap(), case.jaccard),
            ("dice", dice(&s, &o).unwrap(), case.dice),
        ] {
            let diff = ours - theirs;
            if diff.abs() > TOLERANCE {
                failures.push(format!(
                    "case {index} ({}x{}) {name}: ours {ours}, firebench {theirs}, diff {diff:e}",
                    case.rows, case.cols
                ));
            }
        }
    }
    assert!(both_empty > 0, "fixture lacks a both-empty case");
    assert!(
        failures.is_empty(),
        "{} parity failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
