//! The scorecard writer on a hand-computed 1 x 6 grid, and determinism of
//! `ensemble` then `score` for one seed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use firespread_agents::{EnsembleSpec, ForecastConfig, GridInputs, Source};
use firespread_ensemble::{
    BURNABLE_FBFM40, DeadFuelMoisture, HourFields, HourlyConfig, HourlyPerturbation, RawHour,
    WindowWeather,
};
use firespread_grid::{
    ForecastWindow, Grid, GridHeader, MANIFEST_VERSION, Manifest, WindLimitPolicy, read_geotiff_u8,
    write_cog,
};
use firespread_metrics::Mask;
use serde_json::Value;

use crate::ensemble::{
    MEMBERS_FILE, MemberArea, MembersFile, RUN_CONFIG_FILE, ensemble_spec, forecast, hourly_config,
    write_json,
};
use crate::score::{
    CONFUSION_FALSE_ALARM, CONFUSION_HIT, CONFUSION_MISS, CONFUSION_NONE, Observations,
    ScoreOptions, confusion_codes, score_run,
};
use crate::settings::{NightMoistureSettings, PerturbationSettings, RunSettings, Scenario};
use crate::timefmt::parse_hour;

const DX: f64 = 30.0;

fn header(nx: usize, ny: usize) -> GridHeader {
    GridHeader {
        nx,
        ny,
        dx: DX,
        dy: DX,
        origin_x: -876_180.0,
        origin_y: 1_499_820.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "firespread-cli-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mask(nx: usize, ny: usize, bits: &[u8]) -> Mask {
    Mask::new(ny, nx, bits.iter().map(|&b| b == 1).collect()).unwrap()
}

fn window() -> ForecastWindow {
    ForecastWindow {
        name: "w1".to_string(),
        start_t: "2022-04-22T12:00:00".to_string(),
        target_t: "2022-04-23T00:00:00".to_string(),
        fireids: vec![35131],
    }
}

fn start_h() -> i64 {
    parse_hour("2022-04-22T12:00:00").unwrap()
}

/// A 1 x 6 run with horizons 6 and 12 h: the h12 COG holds
/// p = [0, 0.5, 1, 0.25, 0.75, 0], and three members burned 1, 2, 3 cells at
/// 6 h and 10, 30, 20 cells at 12 h.
fn hand_run(dir: &Path) {
    let h = header(6, 1);
    write_cog(
        dir.join("burn_probability_h12.tif"),
        &Grid::new(h, vec![0.0, 0.5, 1.0, 0.25, 0.75, 0.0]).unwrap(),
    )
    .unwrap();
    Manifest {
        version: MANIFEST_VERSION,
        run_id: "hand".to_string(),
        window: window(),
        grid: h,
        inputs: BTreeMap::new(),
        hrrr_source: "none".to_string(),
        seed: 1,
        wind_limit: WindLimitPolicy::None,
        members: 3,
        horizons_h: vec![6, 12],
        outputs: [(
            "burn_probability_h12".to_string(),
            "burn_probability_h12.tif".to_string(),
        )]
        .into_iter()
        .collect(),
    }
    .write(dir.join("manifest.json"))
    .unwrap();
    let member = |m, a, b| MemberArea {
        member: m,
        ignited_cells: 1,
        burned_cells: vec![a, b],
    };
    write_json(
        &dir.join(MEMBERS_FILE),
        &MembersFile {
            run_id: "hand".to_string(),
            horizons_h: vec![6, 12],
            members: vec![member(0, 1, 10), member(1, 2, 30), member(2, 3, 20)],
        },
    )
    .unwrap();
}

/// Start {2}, FEDS target {1, 2, 3}, WFIGS {1, 2, 3, 4}, cell 0 not burnable.
fn hand_observations() -> Observations {
    Observations {
        header: header(6, 1),
        start_h: start_h(),
        target_h: start_h() + 12,
        start: mask(6, 1, &[0, 0, 1, 0, 0, 0]),
        previous: mask(6, 1, &[0, 0, 1, 0, 0, 0]),
        target: mask(6, 1, &[0, 1, 1, 1, 0, 0]),
        wfigs: mask(6, 1, &[0, 1, 1, 1, 1, 0]),
        burnable: mask(6, 1, &[0, 1, 1, 1, 1, 1]),
        observed_geojson: None,
    }
}

fn row<'a>(card: &'a Value, entrant: &str, observation: &str) -> &'a Value {
    let rows: Vec<&Value> = card["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["entrant"] == entrant && r["observation"] == observation)
        .collect();
    assert_eq!(rows.len(), 1, "rows of {entrant} / {observation}: {rows:?}");
    rows[0]
}

fn assert_metric(card: &Value, entrant: &str, observation: &str, metric: &str, want: Option<f64>) {
    let got = &row(card, entrant, observation)[metric];
    match want {
        None => assert!(
            got.is_null(),
            "{entrant} {observation} {metric}: got {got}, expected null"
        ),
        Some(w) => {
            let g = got.as_f64().unwrap_or(f64::NAN);
            assert!(
                (g - w).abs() <= 1e-12,
                "{entrant} {observation} {metric}: got {g}, expected {w}"
            );
        }
    }
}

#[test]
fn scorecard_matches_hand_values() {
    let dir = scratch("hand");
    hand_run(&dir);
    let opts = ScoreOptions {
        uniform_rate_m_per_h: Some(5.0),
        brier_radius_m: 60.0,
    };
    score_run(&dir, &hand_observations(), &opts).unwrap();
    let card: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("scorecard.json")).unwrap())
            .unwrap();
    let rows = card["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 6, "rows: {rows:?}");
    assert!(
        rows.iter().all(|r| r["horizon_h"] == 12),
        "a row outside the 12 h horizon: {rows:?}"
    );
    // Domain: within 60 m of cell 2 (cells 0-4) and burnable (not 0): {1, 2, 3, 4}.
    assert_eq!(card["brier_domain"]["cells"], 4);
    // firespread: p >= 0.5 gives S = {1, 2, 4}; O = {1, 2, 3}.
    // Brier over {1, 2, 3, 4}: (0.25 + 0 + 0.5625 + 0.5625) / 4 = 0.34375.
    for (metric, want) in [
        ("jaccard", 0.5),
        ("dice", 4.0 / 6.0),
        ("precision", 2.0 / 3.0),
        ("recall", 2.0 / 3.0),
        ("brier", 0.343_75),
    ] {
        assert_metric(&card, "firespread", "feds", metric, Some(want));
    }
    // persistence: S = {2}; Brier p = [0, 1, 0, 0] vs o = [1, 1, 1, 0]: 0.5.
    for (metric, want) in [
        ("jaccard", 1.0 / 3.0),
        ("dice", 0.5),
        ("precision", 1.0),
        ("recall", 1.0 / 3.0),
        ("brier", 0.5),
    ] {
        assert_metric(&card, "persistence", "feds", metric, Some(want));
    }
    // uniform_growth: 5 m/h for 12 h = 60 m, S = {0, 1, 2, 3, 4};
    // Brier p = [1, 1, 1, 1] vs o = [1, 1, 1, 0]: 0.25.
    for (metric, want) in [
        ("jaccard", 0.6),
        ("dice", 0.75),
        ("precision", 0.6),
        ("recall", 1.0),
        ("brier", 0.25),
    ] {
        assert_metric(&card, "uniform_growth", "feds", metric, Some(want));
    }
    // WFIGS {1, 2, 3, 4}: precision only.
    for (entrant, want) in [
        ("firespread", 1.0),
        ("persistence", 1.0),
        ("uniform_growth", 0.8),
    ] {
        assert_metric(&card, entrant, "wfigs_final", "precision", Some(want));
        for metric in ["jaccard", "dice", "recall", "brier"] {
            assert_metric(&card, entrant, "wfigs_final", metric, None);
        }
    }
    assert_eq!(card["uniform_growth"]["source"], "option");

    // Area: linear-interpolation percentiles of 900 m² cells.
    let area: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("area_by_hour.json")).unwrap())
            .unwrap();
    assert_eq!(
        area["hours_utc"],
        serde_json::json!(["2022-04-22T18:00:00Z", "2022-04-23T00:00:00Z"])
    );
    for (q, want_cells) in [
        ("p10", [1.2, 12.0]),
        ("p50", [2.0, 20.0]),
        ("p90", [2.8, 28.0]),
    ] {
        for (k, cells) in want_cells.iter().enumerate() {
            let got = area["ensemble_km2"][q][k].as_f64().unwrap();
            let want = cells * 900.0 / 1e6;
            assert!(
                (got - want).abs() <= 1e-15,
                "{q} horizon {k}: got {got} km², expected {want}"
            );
        }
    }
    assert_eq!(
        area["observed"],
        serde_json::json!([
            {"t": "2022-04-22T12:00:00Z", "area_km2": 0.0009},
            {"t": "2022-04-23T00:00:00Z", "area_km2": 0.0027}
        ])
    );
}

#[test]
fn confusion_codes_label_every_cell() {
    // 2 x 3: forecast {0, 1, 4}, target {0, 2, 5}.
    let forecast = mask(3, 2, &[1, 1, 0, 0, 1, 0]);
    let target = mask(3, 2, &[1, 0, 1, 0, 0, 1]);
    let got = confusion_codes(&forecast, &target).unwrap();
    let want = [
        CONFUSION_HIT,
        CONFUSION_FALSE_ALARM,
        CONFUSION_MISS,
        CONFUSION_NONE,
        CONFUSION_FALSE_ALARM,
        CONFUSION_MISS,
    ];
    assert_eq!(want, [1, 3, 2, 0, 3, 2], "code constants changed");
    assert_eq!(got, want, "codes: got {got:?}, expected {want:?}");
    let e = confusion_codes(&forecast, &mask(2, 3, &[0; 6])).unwrap_err();
    assert!(
        e.to_string().contains("target mask is 3 x 2"),
        "shape mismatch error: {e}"
    );
}

#[test]
fn score_run_writes_the_confusion_map() {
    let dir = scratch("confusion");
    hand_run(&dir);
    let opts = ScoreOptions {
        uniform_rate_m_per_h: Some(5.0),
        brier_radius_m: 60.0,
    };
    let out = score_run(&dir, &hand_observations(), &opts).unwrap();
    let path = dir.join("confusion_h12.tif");
    assert!(
        out.files.contains(&path),
        "{} not among the written files {:?}",
        path.display(),
        out.files
    );
    let grid = read_geotiff_u8(&path).unwrap();
    let want_header = GridHeader {
        nodata: 255.0,
        ..header(6, 1)
    };
    assert!(
        grid.header.same_grid(&want_header),
        "header: got {:?}, expected {want_header:?}",
        grid.header
    );
    // Forecast p >= 0.5: {1, 2, 4}; target {1, 2, 3}.
    assert_eq!(grid.data, [0, 1, 1, 2, 3, 0], "confusion cells");
}

#[test]
fn fitted_rate_is_the_fitted_radius_over_12_hours() {
    let dir = scratch("fit");
    hand_run(&dir);
    let mut obs = hand_observations();
    // P(t0 - 12 h) = {2}, P(t0) = {1, 2, 3}: R* = 30 m.
    obs.start = mask(6, 1, &[0, 1, 1, 1, 0, 0]);
    let opts = ScoreOptions {
        uniform_rate_m_per_h: None,
        brier_radius_m: 60.0,
    };
    let out = score_run(&dir, &obs, &opts).unwrap();
    let ug = &out.scorecard.uniform_growth;
    assert_eq!(ug.source, "fitted");
    let r = ug.fitted_radius_m.unwrap();
    assert!((30.0..=31.0).contains(&r), "fitted radius {r} m");
    assert!(
        (ug.rate_m_per_h - r / 12.0).abs() <= 1e-15,
        "rate {} vs radius {r} / 12",
        ug.rate_m_per_h
    );
    assert_eq!(
        (ug.fit_from.as_str(), ug.fit_to.as_str()),
        ("20220422T0000", "20220422T1200")
    );
}

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

const NX: usize = 60;

/// 60 x 60 flat GR2, a 3 x 3 start mask at the centre, two hours of wind.
fn synthetic() -> (GridInputs, WindowWeather, Vec<u8>) {
    let n = NX * NX;
    let mut ignited = vec![0_u8; n];
    for r in 29..32 {
        for c in 29..32 {
            ignited[r * NX + c] = 1;
        }
    }
    let grid = GridInputs {
        header: header(NX, NX),
        fbfm40: vec![102.0; n],
        slope_deg: vec![0.0; n],
        aspect_deg: vec![-1.0; n],
        water: vec![0; n],
        ignited: ignited.clone(),
        canopy_height_m: vec![0.0; n],
        canopy_base_height_m: vec![0.0; n],
        canopy_cover_pct: vec![0.0; n],
    };
    let weather = WindowWeather {
        hours: vec![hour(n, 3.0, 2.0), hour(n, 1.0, -3.0)],
        hours_utc: vec!["h0".to_string(), "h1".to_string()],
        fm100_pct: vec![5.0; n],
        fm1000_pct: vec![7.0; n],
    };
    (grid, weather, ignited)
}

async fn ensemble_then_score(runs_dir: &Path) -> (Vec<u8>, Vec<u8>) {
    let (grid, weather, ignited) = synthetic();
    let config = ForecastConfig {
        run_id: "w1-seed7-m24".to_string(),
        runs_dir: runs_dir.to_path_buf(),
        window: window(),
        hourly: HourlyConfig::default(),
        ensemble: EnsembleSpec {
            seed: 7,
            members: 24,
            horizons_h: vec![1, 2],
            perturbation: HourlyPerturbation::default(),
        },
        threads: 3,
    };
    let source = Source::Memory {
        grid: Box::new(grid),
        weather,
    };
    let summary = forecast(
        config,
        source,
        &RunSettings::default(),
        std::future::pending(),
    )
    .await
    .unwrap();
    let n = NX * NX;
    let mut target = vec![0_u8; n];
    for r in 25..36 {
        for c in 27..40 {
            target[r * NX + c] = 1;
        }
    }
    let obs = Observations {
        header: header(NX, NX),
        start_h: start_h(),
        target_h: start_h() + 2,
        start: mask(NX, NX, &ignited),
        previous: mask(NX, NX, &ignited),
        target: mask(NX, NX, &target),
        wfigs: mask(NX, NX, &vec![1; n]),
        burnable: mask(NX, NX, &vec![1; n]),
        observed_geojson: None,
    };
    let opts = ScoreOptions {
        uniform_rate_m_per_h: None,
        brier_radius_m: 600.0,
    };
    score_run(&summary.run_dir, &obs, &opts).unwrap();
    (
        std::fs::read(summary.run_dir.join("scorecard.json")).unwrap(),
        std::fs::read(summary.run_dir.join(MEMBERS_FILE)).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_seed_gives_byte_identical_scorecards() {
    let (a_dir, b_dir) = (scratch("det-a"), scratch("det-b"));
    let (card_a, members_a) = ensemble_then_score(&a_dir).await;
    let (card_b, members_b) = ensemble_then_score(&b_dir).await;
    let card: Value = serde_json::from_slice(&card_a).unwrap();
    let jaccard = row(&card, "firespread", "feds")["jaccard"]
        .as_f64()
        .unwrap();
    assert!(
        jaccard > 0.0 && jaccard < 1.0,
        "firespread Jaccard {jaccard}: the synthetic run scored trivially"
    );
    assert!(
        card_a == card_b,
        "scorecard.json differs between two runs of seed 7:\n{}\n---\n{}",
        String::from_utf8_lossy(&card_a),
        String::from_utf8_lossy(&card_b)
    );
    assert!(
        members_a == members_b,
        "{MEMBERS_FILE} differs between two runs of seed 7"
    );
    let _ = std::fs::remove_dir_all(a_dir);
    let _ = std::fs::remove_dir_all(b_dir);
}

/// Runs the synthetic window with UTC labels 02Z and 03Z (local 20:00 and
/// 21:00 at offset -6) under `settings`, and returns `run_config.json` and
/// `members.json`.
async fn run_with(runs_dir: &Path, settings: &RunSettings) -> (Value, Value) {
    let (grid, mut weather, _) = synthetic();
    weather.hours_utc = vec![
        "2022-05-10T02:00:00Z".to_string(),
        "2022-05-10T03:00:00Z".to_string(),
    ];
    let config = ForecastConfig {
        run_id: "w1-seed7-m24".to_string(),
        runs_dir: runs_dir.to_path_buf(),
        window: window(),
        hourly: hourly_config(settings).unwrap(),
        ensemble: EnsembleSpec {
            horizons_h: vec![1, 2],
            ..ensemble_spec(settings, 7, 24)
        },
        threads: 3,
    };
    let source = Source::Memory {
        grid: Box::new(grid),
        weather,
    };
    let summary = forecast(config, source, settings, std::future::pending())
        .await
        .unwrap();
    let read =
        |file| serde_json::from_slice(&std::fs::read(summary.run_dir.join(file)).unwrap()).unwrap();
    (read(RUN_CONFIG_FILE), read(MEMBERS_FILE))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn night_moisture_reaches_the_run_and_its_config() {
    let (off_dir, on_dir) = (scratch("night-off"), scratch("night-on"));
    let off = RunSettings::default();
    let on = RunSettings {
        night_moisture: Some(NightMoistureSettings {
            scenario: Scenario::D2,
            custom_pct: None,
            start_local: "20:00".to_string(),
            end_local: "07:59".to_string(),
            utc_offset_h: -6,
        }),
        ..RunSettings::default()
    };
    let (config_off, members_off) = run_with(&off_dir, &off).await;
    let (config_on, members_on) = run_with(&on_dir, &on).await;
    assert!(
        config_off.get("night_moisture").is_none(),
        "off run_config.json holds night_moisture: {config_off}"
    );
    let want = serde_json::json!({
        "scenario": "D2",
        "dead_pct": [6.0, 7.0, 8.0],
        "start_local": "20:00",
        "end_local": "07:59",
        "utc_offset_h": -6,
    });
    assert_eq!(
        config_on["night_moisture"], want,
        "on run_config.json: {config_on}"
    );
    assert_ne!(
        members_off["members"], members_on["members"],
        "night moisture left every member's burned cells unchanged"
    );
    let _ = std::fs::remove_dir_all(off_dir);
    let _ = std::fs::remove_dir_all(on_dir);
}

#[test]
fn hourly_config_puts_each_override_at_its_model_index() {
    let settings = RunSettings {
        spread_multiplier: 1.5,
        spread_multiplier_by_model: BTreeMap::from([
            ("GR2".to_string(), 2.0),
            ("TL3".to_string(), 0.25),
        ]),
        ..RunSettings::default()
    };
    let got = hourly_config(&settings).unwrap().spread_multiplier;
    for (i, &(_, model)) in BURNABLE_FBFM40.iter().enumerate() {
        let want = match model {
            "GR2" => 2.0,
            "TL3" => 0.25,
            _ => 1.5,
        };
        assert_eq!(
            got[i], want,
            "index {i} ({model}): got {}, expected {want}; all: {got:?}",
            got[i]
        );
    }
    assert_eq!(
        hourly_config(&RunSettings::default()).unwrap(),
        HourlyConfig::default(),
        "default settings changed the hourly configuration"
    );
}

#[test]
fn ensemble_spec_carries_the_perturbation_table() {
    let settings = RunSettings {
        perturbation: PerturbationSettings {
            ignition_radius_m: 100.0,
            wind_speed_sd: 0.3,
            wind_direction_sd_deg: 30.0,
            dead_moisture_log_sd: 0.05,
        },
        ..RunSettings::default()
    };
    let got = ensemble_spec(&settings, 7, 24).perturbation;
    assert_eq!(
        got,
        HourlyPerturbation {
            ignition_radius_m: 100.0,
            wind_speed_sd: 0.3,
            wind_direction_sd_rad: 30_f64.to_radians(),
            dead_moisture_log_sd: 0.05,
        },
        "perturbation of the spec: {got:?}"
    );
    assert_eq!(
        ensemble_spec(&RunSettings::default(), 7, 24).perturbation,
        HourlyPerturbation::default()
    );
}

fn burned_at_last_horizon(members: &Value) -> u64 {
    members["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            m["burned_cells"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_u64()
                .unwrap()
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calibration_settings_reach_the_run_and_its_config() {
    let dirs = [scratch("calib-1"), scratch("calib-2"), scratch("calib-by")];
    let doubled = RunSettings {
        spread_multiplier: 2.0,
        ..RunSettings::default()
    };
    let by_model = RunSettings {
        spread_multiplier_by_model: BTreeMap::from([("GR2".to_string(), 2.0)]),
        perturbation: PerturbationSettings {
            wind_speed_sd: 0.3,
            ..PerturbationSettings::default()
        },
        ..RunSettings::default()
    };
    let (config_1, members_1) = run_with(&dirs[0], &RunSettings::default()).await;
    let (config_2, members_2) = run_with(&dirs[1], &doubled).await;
    let (config_by, _) = run_with(&dirs[2], &by_model).await;

    let (burned_1, burned_2) = (
        burned_at_last_horizon(&members_1),
        burned_at_last_horizon(&members_2),
    );
    assert!(
        burned_2 > burned_1,
        "spread_multiplier 2 burned {burned_2} member-cells at 2 h, 1 burned {burned_1}"
    );

    for key in [
        "spread_multiplier",
        "spread_multiplier_by_model",
        "perturbation",
    ] {
        assert!(
            config_1.get(key).is_none(),
            "default run_config.json holds {key}: {config_1}"
        );
    }
    assert_eq!(
        config_2["spread_multiplier"], 2.0,
        "doubled run_config.json: {config_2}"
    );
    assert!(
        config_2.get("perturbation").is_none()
            && config_2.get("spread_multiplier_by_model").is_none(),
        "doubled run_config.json: {config_2}"
    );
    assert!(
        config_by.get("spread_multiplier").is_none(),
        "by-model run_config.json: {config_by}"
    );
    assert_eq!(
        config_by["spread_multiplier_by_model"],
        serde_json::json!({"GR2": 2.0}),
        "by-model run_config.json: {config_by}"
    );
    assert_eq!(
        config_by["perturbation"],
        serde_json::json!({
            "ignition_radius_m": 375.0,
            "wind_speed_sd": 0.3,
            "wind_direction_sd_deg": 15.0,
            "dead_moisture_log_sd": 0.1,
        }),
        "by-model run_config.json: {config_by}"
    );
    for dir in dirs {
        let _ = std::fs::remove_dir_all(dir);
    }
}
