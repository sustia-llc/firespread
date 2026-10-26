//! Directional spread (Finney 1998 RMRS-RP-4 pp. 6-7): vector sum of the wind
//! and slope factors, the length-to-breadth ratio and the focal ellipse.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use firespread_fuel::{Moistures, by_code};
use firespread_rothermel::{
    DirectionalEnvironment, Environment, FuelBed, WindLimit, ellipse_rate, length_to_breadth,
    spread, spread_directional,
};

const MOIST: Moistures = Moistures {
    dead_1h: 0.06,
    dead_10h: 0.07,
    dead_100h: 0.08,
    live_herb: 0.60,
    live_woody: 0.90,
};

const LIMITS: [WindLimit; 3] = [
    WindLimit::None,
    WindLimit::Rothermel,
    WindLimit::EffectiveWind,
];

fn bed(code: &str) -> FuelBed {
    FuelBed::from_model(&by_code(code).unwrap().to_si(), &MOIST).unwrap()
}

fn rel(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        a.abs()
    } else {
        ((a - b) / b).abs()
    }
}

/// The wind speed (m/s) whose φ_w equals the bed's φ_s at `slope_tan`: U_E of
/// the slope alone on a calm day.
fn wind_matching_slope(b: &FuelBed, slope_tan: f64) -> f64 {
    spread(
        b,
        &Environment {
            midflame_wind_m_s: 0.0,
            slope_tan,
        },
        WindLimit::None,
    )
    .unwrap()
    .effective_wind_m_s
}

// (a) Wind toward the upslope azimuth: |φ_w ŵ + φ_s ŝ| = φ_w + φ_s, so the
// head rate equals spread()'s R, and U_E matches, under every policy.
#[test]
fn aligned_wind_and_upslope_match_scalar_spread() {
    for code in ["GR2", "SH2", "TL3"] {
        let b = bed(code);
        for wind in [0.5, 3.0, 12.0] {
            for slope in [0.0, 0.3, 1.0] {
                for limit in LIMITS {
                    let scalar = spread(
                        &b,
                        &Environment {
                            midflame_wind_m_s: wind,
                            slope_tan: slope,
                        },
                        limit,
                    )
                    .unwrap();
                    let dir = spread_directional(
                        &b,
                        &DirectionalEnvironment {
                            midflame_wind_m_s: wind,
                            wind_toward_azimuth_rad: 1.0,
                            slope_tan: slope,
                            upslope_azimuth_rad: 1.0,
                        },
                        limit,
                    )
                    .unwrap();
                    let tag = format!("{code} U={wind} tan={slope} {limit:?}");
                    assert!(
                        rel(dir.head_ros_m_s, scalar.ros_m_s) < 1e-12,
                        "{tag}: head {} vs spread {}",
                        dir.head_ros_m_s,
                        scalar.ros_m_s
                    );
                    assert!(
                        rel(dir.effective_wind_m_s, scalar.effective_wind_m_s) < 1e-12,
                        "{tag}: U_E {} vs {}",
                        dir.effective_wind_m_s,
                        scalar.effective_wind_m_s
                    );
                    assert!(
                        (dir.max_spread_azimuth_rad - 1.0).abs() < 1e-12,
                        "{tag}: azimuth {}",
                        dir.max_spread_azimuth_rad
                    );
                    assert_eq!(
                        dir.reaction_intensity_kw_m2,
                        scalar.reaction_intensity_kw_m2
                    );
                    assert!(
                        rel(dir.flame_length_m, scalar.flame_length_m) < 1e-12,
                        "{tag}: flame {} vs {}",
                        dir.flame_length_m,
                        scalar.flame_length_m
                    );
                }
            }
        }
    }
}

// (b) Wind toward east (π/2), upslope north (0): the resultant is (φ_w, φ_s) in
// (east, north), so |φ_E| = (φ_w² + φ_s²)^0.5 and the azimuth is
// atan2(φ_w, φ_s). With the wind solved so that φ_w = φ_s the hand values are
// φ_E = √2 φ_s and azimuth π/4.
#[test]
fn perpendicular_wind_and_slope_add_as_vectors() {
    for code in ["GR2", "SH2", "TL3"] {
        let b = bed(code);
        let (wind, slope) = (4.0, 0.5);
        let scalar = spread(
            &b,
            &Environment {
                midflame_wind_m_s: wind,
                slope_tan: slope,
            },
            WindLimit::None,
        )
        .unwrap();
        let dir = spread_directional(
            &b,
            &DirectionalEnvironment {
                midflame_wind_m_s: wind,
                wind_toward_azimuth_rad: FRAC_PI_2,
                slope_tan: slope,
                upslope_azimuth_rad: 0.0,
            },
            WindLimit::None,
        )
        .unwrap();
        let want_phi = scalar.phi_w.hypot(scalar.phi_s);
        let want_az = scalar.phi_w.atan2(scalar.phi_s);
        assert!(
            rel(dir.phi_e, want_phi) < 1e-12,
            "{code}: phi_E {} vs hypot {want_phi} (sum {})",
            dir.phi_e,
            scalar.phi_w + scalar.phi_s
        );
        assert!(
            (dir.max_spread_azimuth_rad - want_az).abs() < 1e-12,
            "{code}: azimuth {} vs {want_az}",
            dir.max_spread_azimuth_rad
        );
        let r0 = dir.ros_no_wind_no_slope_m_s;
        assert!(
            rel(dir.head_ros_m_s, r0 * (1.0 + want_phi)) < 1e-12,
            "{code}: head {} vs {}",
            dir.head_ros_m_s,
            r0 * (1.0 + want_phi)
        );

        let matched = wind_matching_slope(&b, slope);
        let dir = spread_directional(
            &b,
            &DirectionalEnvironment {
                midflame_wind_m_s: matched,
                wind_toward_azimuth_rad: FRAC_PI_2,
                slope_tan: slope,
                upslope_azimuth_rad: 0.0,
            },
            WindLimit::None,
        )
        .unwrap();
        assert!(
            rel(dir.phi_e, 2f64.sqrt() * scalar.phi_s) < 1e-9,
            "{code}: phi_E {} vs sqrt2 phi_s {}",
            dir.phi_e,
            2f64.sqrt() * scalar.phi_s
        );
        assert!(
            (dir.max_spread_azimuth_rad - FRAC_PI_4).abs() < 1e-9,
            "{code}: azimuth {} vs pi/4",
            dir.max_spread_azimuth_rad
        );
    }
}

// (c) Wind blowing downslope with φ_w = φ_s: the resultant vanishes, so
// R = R_0 and LB = 1 (U_E = 0).
#[test]
fn opposed_equal_wind_and_slope_cancel() {
    for code in ["GR2", "SH2", "TL3"] {
        let b = bed(code);
        for slope in [0.2, 0.6] {
            let matched = wind_matching_slope(&b, slope);
            let dir = spread_directional(
                &b,
                &DirectionalEnvironment {
                    midflame_wind_m_s: matched,
                    wind_toward_azimuth_rad: 0.3 + PI,
                    slope_tan: slope,
                    upslope_azimuth_rad: 0.3,
                },
                WindLimit::None,
            )
            .unwrap();
            assert!(
                rel(dir.head_ros_m_s, dir.ros_no_wind_no_slope_m_s) < 1e-9,
                "{code} tan={slope}: head {} vs R0 {}",
                dir.head_ros_m_s,
                dir.ros_no_wind_no_slope_m_s
            );
            assert!(
                (dir.length_to_breadth - 1.0).abs() < 1e-9,
                "{code} tan={slope}: LB {} (U_E {})",
                dir.length_to_breadth,
                dir.effective_wind_m_s
            );
        }
    }
}

// (d) LB(0) = 0.936 + 0.461 − 0.397 = 1.0. LB(5) = 0.936 e^1.283 + 0.461 e^−0.774
// − 0.397; by hand e^1.283 ≈ 3.607446 and e^−0.774 ≈ 0.461165, so
// LB(5) ≈ 3.376569 + 0.212597 − 0.397 = 3.192166. LB reaches 8 near
// U_E = ln(8.397/0.936)/0.2566 ≈ 8.55 m/s, so LB(20) = 8 exactly.
#[test]
fn length_to_breadth_hand_values() {
    let lb0 = length_to_breadth(0.0);
    assert!((lb0 - 1.0).abs() < 1e-12, "LB(0) {lb0}, expected 1.0");
    let lb5 = length_to_breadth(5.0);
    let formula = 0.936 * 1.283f64.exp() + 0.461 * (-0.774f64).exp() - 0.397;
    assert!(
        (lb5 - formula).abs() < 1e-12,
        "LB(5) {lb5}, formula {formula}"
    );
    assert!((lb5 - 3.192166).abs() < 1e-5, "LB(5) {lb5}, hand 3.192166");
    assert_eq!(length_to_breadth(20.0), 8.0, "LB(20) truncates at 8");
    assert_eq!(length_to_breadth(200.0), 8.0, "LB(200) truncates at 8");
    assert!(length_to_breadth(8.0) < 8.0, "LB(8) below the cap");
}

/// HB, a, b, c of RMRS-RP-4 eqs. [14]-[17] at head rate `r`.
fn finney_axes(lb: f64, r: f64) -> (f64, f64, f64, f64) {
    let s = (lb * lb - 1.0).sqrt();
    let hb = (lb + s) / (lb - s);
    let a = 0.5 * (r + r / hb) / lb;
    let b = (r + r / hb) / 2.0;
    let c = b - r / hb;
    (hb, a, b, c)
}

// (e) r(0) = R, r(π) = R/HB, and with R = 1 the points r(ψ)(cos ψ, sin ψ)
// from the rear focus lie on ((x − c)/b)² + (y/a)² = 1.
#[test]
fn ellipse_rate_traces_the_finney_ellipse() {
    for u_e in [0.0, 1.0, 4.0, 8.0, 30.0] {
        let lb = length_to_breadth(u_e);
        let (hb, a, b, c) = finney_axes(lb, 1.0);
        let e = (1.0 - 1.0 / (lb * lb)).sqrt();
        assert!(
            (ellipse_rate(1.0, e, 0.0) - 1.0).abs() < 1e-12,
            "U_E={u_e}: r(0) {}",
            ellipse_rate(1.0, e, 0.0)
        );
        assert!(
            (ellipse_rate(1.0, e, PI) - 1.0 / hb).abs() < 1e-12,
            "U_E={u_e}: r(pi) {} vs 1/HB {}",
            ellipse_rate(1.0, e, PI),
            1.0 / hb
        );
        for k in 0..360 {
            let psi = TAU * f64::from(k) / 360.0;
            let r = ellipse_rate(1.0, e, psi);
            let (x, y) = (r * psi.cos(), r * psi.sin());
            let q = ((x - c) / b).powi(2) + (y / a).powi(2);
            assert!(
                (q - 1.0).abs() < 1e-9,
                "U_E={u_e} LB={lb} psi={psi}: quadric {q}"
            );
        }
    }
}

// The output's ellipse fields follow eqs. [13]-[17] at its own U_E and head.
#[test]
fn directional_output_carries_the_finney_ellipse() {
    let b = bed("GR2");
    let dir = spread_directional(
        &b,
        &DirectionalEnvironment {
            midflame_wind_m_s: 3.0,
            wind_toward_azimuth_rad: 2.0,
            slope_tan: 0.4,
            upslope_azimuth_rad: 4.0,
        },
        WindLimit::None,
    )
    .unwrap();
    let r = dir.head_ros_m_s;
    let lb = length_to_breadth(dir.effective_wind_m_s);
    let (hb, a, bb, c) = finney_axes(lb, r);
    assert_eq!(dir.length_to_breadth, lb);
    assert!(rel(dir.head_to_back, hb) < 1e-12, "{dir:?}");
    assert!(rel(dir.backing_ros_m_s, r / hb) < 1e-12, "{dir:?}");
    assert!(rel(dir.flanking_ros_m_s, a) < 1e-12, "{dir:?}");
    assert!(rel(dir.eccentricity, c / bb) < 1e-12, "{dir:?}");
    assert!(lb > 1.0, "{dir:?}");
    assert!((0.0..TAU).contains(&dir.max_spread_azimuth_rad), "{dir:?}");
}

// Zero resultant: upslope azimuth on a slope, else wind azimuth, else 0;
// azimuths are reported in [0, 2π).
#[test]
fn zero_resultant_azimuth_fallbacks() {
    let b = bed("GR2");
    let calm_flat = spread_directional(
        &b,
        &DirectionalEnvironment {
            midflame_wind_m_s: 0.0,
            wind_toward_azimuth_rad: 2.0,
            slope_tan: 0.0,
            upslope_azimuth_rad: 3.0,
        },
        WindLimit::None,
    )
    .unwrap();
    assert_eq!(calm_flat.max_spread_azimuth_rad, 0.0, "{calm_flat:?}");
    assert_eq!(calm_flat.head_ros_m_s, calm_flat.ros_no_wind_no_slope_m_s);
    assert_eq!(calm_flat.length_to_breadth, 1.0, "{calm_flat:?}");

    let wrapped = spread_directional(
        &b,
        &DirectionalEnvironment {
            midflame_wind_m_s: 2.0,
            wind_toward_azimuth_rad: -FRAC_PI_2,
            slope_tan: 0.0,
            upslope_azimuth_rad: 0.0,
        },
        WindLimit::None,
    )
    .unwrap();
    assert!(
        (wrapped.max_spread_azimuth_rad - 3.0 * FRAC_PI_2).abs() < 1e-12,
        "{wrapped:?}"
    );
}

// (f) At the moisture of extinction I_R = 0, so R_0 = 0 and every rate is
// exactly 0 while LB stays finite.
#[test]
fn zero_spread_at_extinction_stays_zero() {
    let wet = Moistures {
        dead_1h: 0.25,
        dead_10h: 0.25,
        dead_100h: 0.25,
        live_herb: 0.60,
        live_woody: 0.90,
    };
    for code in ["TL3", "SH2"] {
        let b = FuelBed::from_model(&by_code(code).unwrap().to_si(), &wet).unwrap();
        for limit in LIMITS {
            let dir = spread_directional(
                &b,
                &DirectionalEnvironment {
                    midflame_wind_m_s: 3.0,
                    wind_toward_azimuth_rad: 1.0,
                    slope_tan: 0.3,
                    upslope_azimuth_rad: 2.5,
                },
                limit,
            )
            .unwrap();
            assert_eq!(dir.head_ros_m_s, 0.0, "{code} {limit:?}: {dir:?}");
            assert_eq!(dir.backing_ros_m_s, 0.0, "{code} {limit:?}: {dir:?}");
            assert_eq!(dir.flanking_ros_m_s, 0.0, "{code} {limit:?}: {dir:?}");
            assert_eq!(
                dir.fireline_intensity_kw_m, 0.0,
                "{code} {limit:?}: {dir:?}"
            );
            assert!(
                dir.length_to_breadth.is_finite() && dir.length_to_breadth >= 1.0,
                "{code} {limit:?}: {dir:?}"
            );
            assert!(dir.eccentricity.is_finite(), "{code} {limit:?}: {dir:?}");
            assert_eq!(ellipse_rate(dir.head_ros_m_s, dir.eccentricity, 1.0), 0.0);
        }
    }
}

#[test]
fn non_finite_azimuth_is_an_error() {
    let err = spread_directional(
        &bed("GR2"),
        &DirectionalEnvironment {
            midflame_wind_m_s: 1.0,
            wind_toward_azimuth_rad: f64::NAN,
            slope_tan: 0.0,
            upslope_azimuth_rad: 0.0,
        },
        WindLimit::None,
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            firespread_rothermel::RothermelError::InvalidInput {
                field: "wind azimuth",
                ..
            }
        ),
        "{err}"
    );
}
