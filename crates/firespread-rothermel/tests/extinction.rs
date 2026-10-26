//! Rate of spread at or above the moisture of extinction is exactly zero, for
//! the single-class model and for the heterogeneous model on dead-only and
//! dead-plus-live beds.

use firespread_fuel::{Moistures, by_code};
use firespread_rothermel::{Environment, FuelBed, WindLimit, single_class, spread};

const ENV: Environment = Environment {
    midflame_wind_m_s: 2.0,
    slope_tan: 0.3,
};

fn single(moisture: f64) -> f64 {
    single_class(6000.0, 0.5, 0.3, 0.15, 18.6e6, moisture, 2.0, 0.3).unwrap()
}

#[test]
fn single_class_is_zero_at_moisture_equal_to_extinction() {
    let ros = single(0.15);
    assert_eq!(ros, 0.0, "moisture 0.15 = mx 0.15: ros {ros:e} m/s");
}

#[test]
fn single_class_is_zero_above_extinction() {
    for moisture in [0.151, 0.2, 0.5] {
        let ros = single(moisture);
        assert_eq!(ros, 0.0, "moisture {moisture} > mx 0.15: ros {ros:e} m/s");
    }
}

#[test]
fn single_class_is_positive_below_extinction() {
    let ros = single(0.149);
    assert!(ros > 0.0, "moisture 0.149 < mx 0.15: ros {ros:e} m/s");
}

fn wet(live_woody: f64) -> Moistures {
    Moistures {
        dead_1h: 0.25,
        dead_10h: 0.25,
        dead_100h: 0.25,
        live_herb: 0.60,
        live_woody,
    }
}

#[test]
fn spread_is_zero_on_dead_only_bed_above_extinction() {
    // TL3: dead loads only, dead Mx 0.20.
    let bed = FuelBed::from_model(&by_code("TL3").unwrap().to_si(), &wet(0.90)).unwrap();
    let out = spread(&bed, &ENV, WindLimit::None).unwrap();
    assert_eq!(out.live_mx, None, "TL3 has no live fuel: {out:?}");
    assert_eq!(
        out.ros_m_s, 0.0,
        "TL3 dead 0.25 >= Mx 0.20: ros {:e} m/s",
        out.ros_m_s
    );
    assert_eq!(
        out.ros_no_wind_no_slope_m_s, 0.0,
        "TL3 R0 {:e} m/s",
        out.ros_no_wind_no_slope_m_s
    );
}

#[test]
fn spread_is_zero_on_dead_and_live_bed_above_both_extinctions() {
    // SH2: static, live woody load, dead Mx 0.15. Dead moisture 0.25 > Mx 0.15 makes
    // 2.9 W (1 - M_f,dead / Mx1) - 0.226 negative, so live Mx clamps to Mx1 = 0.15,
    // and live woody moisture 0.90 is above it.
    let bed = FuelBed::from_model(&by_code("SH2").unwrap().to_si(), &wet(0.90)).unwrap();
    let out = spread(&bed, &ENV, WindLimit::None).unwrap();
    assert_eq!(out.live_mx, Some(0.15), "SH2 live Mx: {out:?}");
    assert_eq!(
        out.ros_m_s, 0.0,
        "SH2 dead 0.25 >= 0.15, live 0.90 >= 0.15: ros {:e} m/s",
        out.ros_m_s
    );
    assert_eq!(
        out.reaction_intensity_kw_m2, 0.0,
        "SH2 I_R {:e} kW/m2",
        out.reaction_intensity_kw_m2
    );
}
