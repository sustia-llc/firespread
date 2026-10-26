//! Pins `single_class` to the FireBench `Rothermel_SFIRE` oracle at 1 %
//! relative error on the rate of spread. The fixture is written by
//! `python/oracle/firebench_rothermel_fixture.py`.

use std::collections::BTreeSet;

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
    moisture_scenario: String,
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
const SCENARIOS: [&str; 3] = ["D1", "D2", "D3"];

#[test]
fn fixture_covers_every_model_and_moisture_scenario() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).unwrap();
    let codes: Vec<String> = firespread_fuel::scott_burgan_40()
        .unwrap()
        .into_iter()
        .map(|m| m.code)
        .collect();
    let models: BTreeSet<&str> = fixture.cases.iter().map(|c| c.model.as_str()).collect();
    for model in &models {
        assert!(
            codes.iter().any(|c| c == model),
            "fixture model {model} is not a Scott and Burgan 40 code"
        );
    }
    let missing: Vec<&String> = codes
        .iter()
        .filter(|c| !models.contains(c.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "models missing from fixture: {missing:?}"
    );
    assert_eq!(models.len(), 40, "distinct models: {models:?}");
    for case in &fixture.cases {
        assert!(
            SCENARIOS.contains(&case.moisture_scenario.as_str()),
            "{} has moisture scenario {}, expected one of {SCENARIOS:?}",
            case.model,
            case.moisture_scenario
        );
    }
    let pairs: BTreeSet<(&str, &str)> = fixture
        .cases
        .iter()
        .map(|c| (c.model.as_str(), c.moisture_scenario.as_str()))
        .collect();
    let missing_pairs: Vec<(&str, &str)> = codes
        .iter()
        .flat_map(|c| SCENARIOS.iter().map(move |s| (c.as_str(), *s)))
        .filter(|p| !pairs.contains(p))
        .collect();
    assert!(
        missing_pairs.is_empty(),
        "(model, scenario) pairs with no case: {missing_pairs:?}"
    );
}

#[test]
fn single_class_matches_firebench_rothermel_sfire_within_1_percent() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).unwrap();
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
