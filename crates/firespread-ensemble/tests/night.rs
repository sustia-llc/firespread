//! Night dead fuel moisture: scenario values, night-hour classification and
//! the dead moistures that reach the Rothermel evaluation.

use firespread_ensemble::moisture::{mc1_fraction, mc10_fraction};
use firespread_ensemble::{
    DeadFuelMoisture, EnsembleError, HourFields, HourlyConfig, HourlyInputs, HourlyLayers, Layers,
    MemberDraw, NightMoisture, NightScenario, RawHour, WindowWeather,
};
use firespread_grid::GridHeader;

const NX: usize = 3;
const N: usize = NX * NX;
const CELL: usize = 4;
const FM100_PCT: f32 = 5.0;

fn hour() -> HourFields {
    HourFields::from_raw(
        &RawHour {
            u10_m_s: &[3.0; N],
            v10_m_s: &[1.0; N],
            t2m_k: &[300.0; N],
            rh2m_pct: &[20.0; N],
            tcc_pct: &[0.0; N],
        },
        &DeadFuelMoisture::default(),
        N,
    )
    .unwrap()
}

fn inputs(labels: &[&str], night: Option<NightMoisture>) -> Result<HourlyInputs, EnsembleError> {
    let weather = WindowWeather {
        hours: labels.iter().map(|_| hour()).collect(),
        hours_utc: labels.iter().map(|l| (*l).to_string()).collect(),
        fm100_pct: vec![FM100_PCT; N],
        fm1000_pct: vec![7.0; N],
    };
    HourlyInputs::from_layers(
        &HourlyLayers {
            base: Layers {
                header: GridHeader {
                    nx: NX,
                    ny: NX,
                    dx: 30.0,
                    dy: 30.0,
                    origin_x: 0.0,
                    origin_y: 0.0,
                    epsg: 5070,
                    nodata: -9999.0,
                },
                fbfm40: &[102.0; N],
                slope_deg: &[0.0; N],
                aspect_deg: &[-1.0; N],
                water: &[0; N],
                ignited: &[0; N],
            },
            canopy_height_m: &[0.0; N],
            canopy_base_height_m: &[0.0; N],
            canopy_cover_pct: &[0.0; N],
        },
        weather,
        &HourlyConfig {
            night_moisture: night,
            ..HourlyConfig::default()
        },
    )
}

fn draw(dead_moisture_factor: f64) -> MemberDraw {
    MemberDraw {
        ignition_east_m: 0.0,
        ignition_north_m: 0.0,
        ignition_shift_rows: 0,
        ignition_shift_cols: 0,
        wind_speed_factor: 1.0,
        wind_direction_offset_rad: 0.0,
        dead_moisture_factor,
    }
}

#[test]
fn scenarios_are_the_table_3_triples() {
    for (scenario, want) in [
        (NightScenario::D1, [3.0, 4.0, 5.0]),
        (NightScenario::D2, [6.0, 7.0, 8.0]),
        (NightScenario::D3, [9.0, 10.0, 11.0]),
    ] {
        let got = scenario.dead_pct();
        assert_eq!(got, want, "{scenario:?} gave {got:?}, expected {want:?}");
        assert_eq!(NightMoisture::new(scenario).dead_pct, want, "{scenario:?}");
    }
    let d = NightMoisture::new(NightScenario::D2);
    assert_eq!(
        (d.start_local_min, d.end_local_min, d.utc_offset_h),
        (20 * 60, 7 * 60 + 59, -6),
        "default period {d:?}"
    );
}

#[test]
fn boundary_hours_take_day_or_night_moisture() {
    let labels = [
        "2022-05-10T01:00:00Z",
        "2022-05-10T02:00:00Z",
        "2022-05-10T13:00:00Z",
        "2022-05-10T14:00:00Z",
    ];
    let h = inputs(&labels, Some(NightMoisture::new(NightScenario::D2))).unwrap();
    let night = h.night_hours();
    assert_eq!(
        night,
        [false, true, true, false],
        "local 19:00, 20:00, 07:00, 08:00 classified {night:?}"
    );
    let emc = f64::from(h.weather().hours[0].emcprm_pct[CELL]);
    let day = [
        mc1_fraction(emc),
        mc10_fraction(emc),
        f64::from(FM100_PCT) / 100.0,
    ];
    let d2 = [0.06, 0.07, 0.08];
    assert_ne!(day, d2, "day and night moisture coincide: test is vacuous");
    for (k, (label, want)) in labels.iter().zip([day, d2, d2, day]).enumerate() {
        let m = h.dead_moistures(k, &draw(1.0), CELL);
        let got = [m.dead_1h, m.dead_10h, m.dead_100h];
        assert_eq!(got, want, "hour {label}: got {got:?}, expected {want:?}");
    }

    // The flag reaches the Rothermel evaluation: identical weather in every
    // hour, so the head rate differs only between day and night hours.
    let head = |k| h.cell_fields(k, &draw(1.0), CELL).unwrap().head_ros_m_s;
    let rates = [head(0), head(1), head(2), head(3)];
    assert!(
        rates[0] == rates[3] && rates[1] == rates[2] && rates[0] != rates[1],
        "head rates {rates:?}, expected day, night, night, day"
    );
}

#[test]
fn w3_hours_at_offset_minus_six() {
    // 2022-05-10T12Z through 2022-05-11T12Z.
    let labels: Vec<String> = (12..37)
        .map(|h: u32| format!("2022-05-{:02}T{:02}:00:00Z", 10 + h / 24, h % 24))
        .collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let h = inputs(&refs, Some(NightMoisture::new(NightScenario::D2))).unwrap();
    // 12Z, 13Z night; 14Z to 01Z day; 02Z to 11Z night; 12Z night.
    let mut want = vec![true, true];
    want.extend([false; 12]);
    want.extend([true; 10]);
    want.push(true);
    assert_eq!(h.night_hours(), want, "labels {labels:?}");
}

#[test]
fn custom_triple_and_factor_on_one_and_ten_hour() {
    let night = NightMoisture {
        dead_pct: [2.5, 3.5, 4.5],
        ..NightMoisture::new(NightScenario::D2)
    };
    let h = inputs(&["2022-05-10T05:00:00Z"], Some(night)).unwrap();
    let m = h.dead_moistures(0, &draw(1.5), CELL);
    let got = [m.dead_1h, m.dead_10h, m.dead_100h];
    let want = [0.0375, 0.0525, 0.045];
    assert!(
        got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1e-15),
        "got {got:?}, expected {want:?}"
    );
}

#[test]
fn labels_are_read_only_with_the_setting_on() {
    let labels = ["hour 0", "hour 1"];
    let off = inputs(&labels, None).unwrap();
    assert_eq!(off.night_hours(), [false, false], "off run has night hours");
    let err = inputs(&labels, Some(NightMoisture::new(NightScenario::D1))).unwrap_err();
    assert!(
        matches!(&err, EnsembleError::InvalidHourLabel { index: 0, label } if label == "hour 0"),
        "{err}"
    );
}

#[test]
fn out_of_range_settings_are_rejected() {
    let base = NightMoisture::new(NightScenario::D1);
    for (name, bad) in [
        (
            "zero percent",
            NightMoisture {
                dead_pct: [0.0, 4.0, 5.0],
                ..base
            },
        ),
        (
            "NaN percent",
            NightMoisture {
                dead_pct: [3.0, f64::NAN, 5.0],
                ..base
            },
        ),
        (
            "above 100",
            NightMoisture {
                dead_pct: [3.0, 4.0, 100.5],
                ..base
            },
        ),
        (
            "start 24:00",
            NightMoisture {
                start_local_min: 1440,
                ..base
            },
        ),
        (
            "start equals end",
            NightMoisture {
                start_local_min: 600,
                end_local_min: 600,
                ..base
            },
        ),
        (
            "offset 15",
            NightMoisture {
                utc_offset_h: 15,
                ..base
            },
        ),
    ] {
        let got = inputs(&["2022-05-10T05:00:00Z"], Some(bad));
        assert!(
            matches!(got, Err(EnsembleError::InvalidNightMoisture { .. })),
            "{name}: {got:?}"
        );
    }
}
