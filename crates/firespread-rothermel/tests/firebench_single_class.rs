//! Pins `single_class` to the FireBench `Rothermel_SFIRE` oracle at 1 %
//! relative error on the rate of spread. The fixture is written by
//! `python/oracle/firebench_rothermel_fixture.py`.

use firespread_fuel::units;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    oracle: Oracle,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Oracle {
    heat_btu_per_lb: f64,
    clamp_m_s: f64,
}

#[derive(Debug, Deserialize)]
struct Case {
    model: String,
    fgi_kg_m2: f64,
    fueldepth_m: f64,
    savr_per_ft: f64,
    fuelmce_passed: f64,
    fmc_percent: f64,
    wind_m_s: f64,
    slope_percent: f64,
    ros_m_s: f64,
}

const FIXTURE: &str = include_str!("fixtures/firebench_single_class.json");
const TOLERANCE: f64 = 0.01;

#[test]
fn single_class_matches_firebench_rothermel_sfire_within_1_percent() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).unwrap();
    assert!(
        fixture.cases.len() >= 100,
        "only {} cases",
        fixture.cases.len()
    );
    let heat_j_per_kg = units::btu_lb_to_j_kg(fixture.oracle.heat_btu_per_lb);
    let mut worst = (0.0f64, String::new());
    for case in &fixture.cases {
        assert_ne!(
            case.ros_m_s, fixture.oracle.clamp_m_s,
            "clamped case in fixture: {case:?}"
        );
        let got = firespread_rothermel::single_class(
            units::per_ft_to_per_m(case.savr_per_ft),
            case.fgi_kg_m2,
            case.fueldepth_m,
            case.fuelmce_passed,
            heat_j_per_kg,
            units::percent_to_fraction(case.fmc_percent),
            case.wind_m_s,
            case.slope_percent / 100.0,
        )
        .unwrap();
        let rel = ((got - case.ros_m_s) / case.ros_m_s).abs();
        if rel > worst.0 {
            worst = (
                rel,
                format!(
                    "{} wind {} m/s slope {} %: got {got} m/s, oracle {} m/s",
                    case.model, case.wind_m_s, case.slope_percent, case.ros_m_s
                ),
            );
        }
        assert!(
            rel <= TOLERANCE,
            "{} wind {} m/s slope {} %: got {got} m/s, oracle {} m/s, relative error {rel}",
            case.model,
            case.wind_m_s,
            case.slope_percent,
            case.ros_m_s
        );
    }
    eprintln!("worst relative error {} at {}", worst.0, worst.1);
}
