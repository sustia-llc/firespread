//! Runs the multi-class `spread` on every case of the pyretechnics oracle fixture.
//!
//! PYRETECHNICS developed by Spatial Informatics Group, LLC and contributors.
//!
//! The fixture is written by `python/oracle/pyretechnics_multi_class_fixture.py`.
//! These tests check the fixture's coverage and that the engine returns a finite,
//! non-negative rate of spread on each case; they do not compare against the
//! oracle's values.

use std::collections::{BTreeMap, BTreeSet};

use firespread_fuel::{FuelModelSi, Moistures};
use firespread_rothermel::{Environment, FuelBed, WindLimit};
use serde::Deserialize;

const FIXTURE: &str = include_str!("fixtures/pyretechnics_multi_class.json");
const ATTRIBUTION: &str =
    "PYRETECHNICS developed by Spatial Informatics Group, LLC and contributors.";
const SCENARIOS: [&str; 3] = ["D1", "D2", "D3"];
const LIVE_SCENARIO: &str = "L2";

#[derive(Debug, Deserialize)]
struct Fixture {
    oracle: Oracle,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Oracle {
    attribution: String,
    dead_scenarios_percent: BTreeMap<String, [f64; 3]>,
    live_scenario_percent: BTreeMap<String, [f64; 2]>,
}

#[derive(Debug, Deserialize)]
struct Case {
    model: String,
    dead_scenario: String,
    live_scenario: String,
    wind_m_s: f64,
    slope_percent: f64,
}

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).unwrap()
}

#[test]
fn fixture_covers_every_model_and_dead_scenario_with_attribution() {
    let fixture = fixture();
    assert_eq!(fixture.cases.len(), 3000, "case count");
    assert_eq!(
        fixture.oracle.attribution, ATTRIBUTION,
        "oracle.attribution"
    );
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
    let scenarios: BTreeSet<&str> = fixture
        .cases
        .iter()
        .map(|c| c.dead_scenario.as_str())
        .collect();
    assert_eq!(
        scenarios,
        SCENARIOS.into_iter().collect::<BTreeSet<_>>(),
        "dead scenarios"
    );
}

#[test]
fn spread_is_finite_and_non_negative_on_every_case() {
    let fixture = fixture();
    let [live_herb, live_woody] = fixture.oracle.live_scenario_percent[LIVE_SCENARIO];
    let mut models: BTreeMap<String, FuelModelSi> = BTreeMap::new();
    for case in &fixture.cases {
        assert_eq!(case.live_scenario, LIVE_SCENARIO, "{case:?}");
        let model = models
            .entry(case.model.clone())
            .or_insert_with(|| firespread_fuel::by_code(&case.model).unwrap().to_si());
        let [d1, d10, d100] = fixture.oracle.dead_scenarios_percent[&case.dead_scenario];
        let moistures = Moistures {
            dead_1h: d1 / 100.0,
            dead_10h: d10 / 100.0,
            dead_100h: d100 / 100.0,
            live_herb: live_herb / 100.0,
            live_woody: live_woody / 100.0,
        };
        let bed = FuelBed::from_model(model, &moistures).unwrap();
        let env = Environment {
            midflame_wind_m_s: case.wind_m_s,
            slope_tan: case.slope_percent / 100.0,
        };
        let out = firespread_rothermel::spread(&bed, &env, WindLimit::None).unwrap();
        assert!(
            out.ros_m_s.is_finite() && out.ros_m_s >= 0.0,
            "{case:?}: ros {} m/s",
            out.ros_m_s
        );
    }
}
