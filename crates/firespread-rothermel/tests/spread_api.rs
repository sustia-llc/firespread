//! Behaviour of the SI API: fuel-model beds, wind policies and related models.

use firespread_fuel::{Moistures, by_code, units};
use firespread_rothermel::{Category, Environment, FuelBed, SpreadClass, WindLimit, spread};

const MOIST: Moistures = Moistures {
    dead_1h: 0.06,
    dead_10h: 0.07,
    dead_100h: 0.08,
    live_herb: 0.60,
    live_woody: 0.90,
};

fn bed(code: &str) -> FuelBed {
    FuelBed::from_model(&by_code(code).unwrap().to_si(), &MOIST).unwrap()
}

fn rel(a: f64, b: f64) -> f64 {
    ((a - b) / b).abs()
}

#[test]
fn every_fuel_model_spreads_with_finite_positive_outputs() {
    let env = Environment {
        midflame_wind_m_s: 3.0,
        slope_tan: 0.1,
    };
    for model in firespread_fuel::scott_burgan_40().unwrap() {
        let out = spread(&bed(&model.code), &env, WindLimit::None).unwrap();
        let code = &model.code;
        assert!(
            out.ros_m_s.is_finite() && out.ros_m_s > 0.0,
            "{code}: {out:?}"
        );
        assert!(
            out.ros_m_s >= out.ros_no_wind_no_slope_m_s,
            "{code}: {out:?}"
        );
        assert!(out.reaction_intensity_kw_m2 > 0.0, "{code}: {out:?}");
        assert!(out.heat_sink_kj_m3 > 0.0, "{code}: {out:?}");
        assert!(out.flame_length_m > 0.0, "{code}: {out:?}");
        assert_eq!(out.wind_limit, WindLimit::None);
        assert!(!out.wind_limited, "{code}: {out:?}");
        let has_live = model.load_live_herb_t_ac > 0.0 || model.load_live_woody_t_ac > 0.0;
        assert_eq!(out.live_mx.is_some(), has_live, "{code}: {out:?}");
        if let Some(mx2) = out.live_mx {
            assert!(
                mx2 >= units::percent_to_fraction(model.dead_mx_percent),
                "{code}: {out:?}"
            );
        }
    }
}

#[test]
fn no_wind_no_slope_gives_r0_and_zero_factors() {
    let out = spread(
        &bed("GR1"),
        &Environment {
            midflame_wind_m_s: 0.0,
            slope_tan: 0.0,
        },
        WindLimit::None,
    )
    .unwrap();
    assert_eq!(out.phi_w, 0.0);
    assert_eq!(out.phi_s, 0.0);
    assert_eq!(out.phi_e, 0.0);
    assert_eq!(out.effective_wind_m_s, 0.0);
    assert_eq!(out.ros_m_s, out.ros_no_wind_no_slope_m_s);
}

#[test]
fn effective_wind_equals_midflame_wind_on_flat_ground() {
    for code in ["GR1", "SH2", "TL3"] {
        for wind in [0.5, 2.0, 7.0] {
            let out = spread(
                &bed(code),
                &Environment {
                    midflame_wind_m_s: wind,
                    slope_tan: 0.0,
                },
                WindLimit::None,
            )
            .unwrap();
            assert!(
                rel(out.effective_wind_m_s, wind) < 1e-9,
                "{code} {wind}: {out:?}"
            );
            assert!(
                rel(out.wind_used_m_s, wind) < 1e-12,
                "{code} {wind}: {out:?}"
            );
        }
    }
}

#[test]
fn slope_alone_gives_a_positive_effective_wind() {
    let out = spread(
        &bed("TU1"),
        &Environment {
            midflame_wind_m_s: 0.0,
            slope_tan: 0.3,
        },
        WindLimit::None,
    )
    .unwrap();
    assert_eq!(out.phi_w, 0.0);
    assert!(out.phi_s > 0.0, "{out:?}");
    assert!(out.effective_wind_m_s > 0.0, "{out:?}");
}

#[test]
fn rothermel_wind_limit_caps_the_wind_at_0_9_ir() {
    let b = bed("TL3");
    let gale = Environment {
        midflame_wind_m_s: 40.0,
        slope_tan: 0.0,
    };
    let free = spread(&b, &gale, WindLimit::None).unwrap();
    let capped = spread(&b, &gale, WindLimit::Rothermel).unwrap();
    assert_eq!(capped.wind_limit, WindLimit::Rothermel);
    assert!(capped.wind_limited, "{capped:?}");
    assert!(!free.wind_limited, "{free:?}");
    assert!(capped.ros_m_s < free.ros_m_s, "{capped:?} vs {free:?}");
    // U_max = 0.9 I_R with U in ft/min and I_R in Btu/ft2-min.
    let ir_btu_ft2_min = capped.reaction_intensity_kw_m2 * 1000.0 / units::btu_ft2_min_to_w_m2(1.0);
    let u_max_m_s = units::ft_min_to_m_s(0.9 * ir_btu_ft2_min);
    assert!(rel(capped.wind_used_m_s, u_max_m_s) < 1e-9, "{capped:?}");
    assert!(capped.wind_used_m_s < gale.midflame_wind_m_s);

    let calm = Environment {
        midflame_wind_m_s: 0.5,
        slope_tan: 0.0,
    };
    let under = spread(&b, &calm, WindLimit::Rothermel).unwrap();
    assert!(!under.wind_limited, "{under:?}");
    assert_eq!(
        under.ros_m_s,
        spread(&b, &calm, WindLimit::None).unwrap().ros_m_s
    );
}

#[test]
fn effective_wind_limit_caps_the_effective_wind() {
    let b = bed("TL3");
    let steep_gale = Environment {
        midflame_wind_m_s: 40.0,
        slope_tan: 1.0,
    };
    let free = spread(&b, &steep_gale, WindLimit::None).unwrap();
    let capped = spread(&b, &steep_gale, WindLimit::EffectiveWind).unwrap();
    assert_eq!(capped.wind_limit, WindLimit::EffectiveWind);
    assert!(capped.wind_limited, "{capped:?}");
    assert!(capped.ros_m_s < free.ros_m_s, "{capped:?} vs {free:?}");
    assert!(
        capped.effective_wind_m_s < free.effective_wind_m_s,
        "{capped:?}"
    );
    assert!(capped.phi_e < free.phi_e, "{capped:?}");
    // The unlimited wind and slope factors are still reported.
    assert_eq!(capped.phi_w, free.phi_w);
    assert_eq!(capped.phi_s, free.phi_s);
    assert_eq!(capped.wind_used_m_s, steep_gale.midflame_wind_m_s);
    assert!(
        rel(
            capped.ros_m_s,
            capped.ros_no_wind_no_slope_m_s * (1.0 + capped.phi_e)
        ) < 1e-12
    );
}

#[test]
fn byram_models_follow_table_7() {
    let out = spread(
        &bed("GR4"),
        &Environment {
            midflame_wind_m_s: 2.0,
            slope_tan: 0.0,
        },
        WindLimit::None,
    )
    .unwrap();
    // t_r = 384/σ min.
    let sigma_per_ft = units::per_m_to_per_ft(out.characteristic_sav_per_m);
    assert!(
        rel(out.residence_time_s, 60.0 * 384.0 / sigma_per_ft) < 1e-12,
        "{out:?}"
    );
    // I_B = I_R t_r R / 60 Btu/ft/s.
    let ir = out.reaction_intensity_kw_m2 * 1000.0 / units::btu_ft2_min_to_w_m2(1.0);
    let r = units::m_s_to_ft_min(out.ros_m_s);
    let ib = ir * (384.0 / sigma_per_ft) * r / 60.0;
    assert!(
        rel(
            out.fireline_intensity_kw_m,
            units::btu_ft_s_to_w_m(ib) / 1000.0
        ) < 1e-12,
        "{out:?}"
    );
    // F_B = 0.45 I_B^0.46 ft.
    assert!(
        rel(out.flame_length_m, units::ft_to_m(0.45 * ib.powf(0.46))) < 1e-12,
        "{out:?}"
    );
}

#[test]
fn live_moisture_of_extinction_hand_value() {
    // σ_1 = 138/ft and σ_2 = 500/ft give e^-1 in both sums; equal loads give W = 1;
    // M_f,dead = 0.1 and M_x1 = 0.2 give (M_x)_2 = 2.9 (1 − 0.5) − 0.226 = 1.224.
    let class = |category, sav_per_ft: f64, moisture| SpreadClass {
        category,
        load_kg_m2: 0.2,
        sav_per_m: units::per_ft_to_per_m(sav_per_ft),
        moisture,
        heat_j_per_kg: 18_608_000.0,
        total_mineral_content: 0.0555,
        effective_mineral_content: 0.01,
        particle_density_kg_m3: units::lb_ft3_to_kg_m3(32.0),
    };
    let b = FuelBed {
        classes: vec![
            class(Category::Dead, 138.0, 0.1),
            class(Category::Live, 500.0, 1.0),
        ],
        depth_m: 0.5,
        dead_mx: 0.2,
    };
    let out = spread(
        &b,
        &Environment {
            midflame_wind_m_s: 1.0,
            slope_tan: 0.0,
        },
        WindLimit::None,
    )
    .unwrap();
    let mx2 = out.live_mx.unwrap();
    assert!((mx2 - 1.224).abs() < 1e-12, "live Mx {mx2}, expected 1.224");
}

#[test]
fn dynamic_transfer_changes_gr1_spread() {
    let si = by_code("GR1").unwrap().to_si();
    let env = Environment {
        midflame_wind_m_s: 2.0,
        slope_tan: 0.0,
    };
    let cured = FuelBed::from_model(
        &si,
        &Moistures {
            live_herb: 0.30,
            ..MOIST
        },
    )
    .unwrap();
    let green = FuelBed::from_model(
        &si,
        &Moistures {
            live_herb: 1.20,
            ..MOIST
        },
    )
    .unwrap();
    let r_cured = spread(&cured, &env, WindLimit::None).unwrap();
    let r_green = spread(&green, &env, WindLimit::None).unwrap();
    assert!(
        r_cured.ros_m_s > r_green.ros_m_s,
        "{r_cured:?} vs {r_green:?}"
    );
    assert!(r_cured.live_mx.is_none(), "{r_cured:?}");
    assert!(r_green.live_mx.is_some(), "{r_green:?}");
}

#[test]
fn invalid_inputs_are_errors() {
    let b = bed("GR1");
    let err = spread(
        &b,
        &Environment {
            midflame_wind_m_s: -1.0,
            slope_tan: 0.0,
        },
        WindLimit::None,
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            firespread_rothermel::RothermelError::InvalidInput { field: "wind", .. }
        ),
        "{err}"
    );
    let err = firespread_rothermel::single_class(6000.0, 0.0, 0.3, 0.15, 18e6, 0.08, 0.0, 0.0)
        .unwrap_err();
    assert!(
        matches!(
            err,
            firespread_rothermel::RothermelError::InvalidInput { field: "load", .. }
        ),
        "{err}"
    );
}
