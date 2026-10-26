//! `DirectionalModel::eval` against `spread_directional`, bit for bit.

use std::f64::consts::{FRAC_PI_2, PI};

use firespread_fuel::{Moistures, scott_burgan_40};
use firespread_rothermel::{
    DirectionalEnvironment, DirectionalModel, DirectionalOutput, FuelBed, RothermelError,
    WindLimit, spread_directional,
};

/// Scott and Burgan (2005) D1/L1 moisture scenario.
const MOIST: Moistures = Moistures {
    dead_1h: 0.03,
    dead_10h: 0.04,
    dead_100h: 0.05,
    live_herb: 0.30,
    live_woody: 0.60,
};

const LIMITS: [WindLimit; 3] = [
    WindLimit::None,
    WindLimit::Rothermel,
    WindLimit::EffectiveWind,
];
const WINDS_M_S: [f64; 3] = [0.0, 3.0, 15.0];
const SLOPES: [f64; 3] = [0.0, 0.3, 1.0];
/// (wind toward, upslope) azimuth pairs, rad: aligned, opposed, perpendicular,
/// and both outside [0, 2π).
const AZIMUTHS: [(f64, f64); 4] = [(0.0, 0.0), (1.0, 1.0 + PI), (FRAC_PI_2, 0.0), (7.5, -2.0)];

fn fields(o: &DirectionalOutput) -> [(&'static str, f64); 13] {
    [
        ("head_ros_m_s", o.head_ros_m_s),
        ("max_spread_azimuth_rad", o.max_spread_azimuth_rad),
        ("phi_e", o.phi_e),
        ("effective_wind_m_s", o.effective_wind_m_s),
        ("length_to_breadth", o.length_to_breadth),
        ("head_to_back", o.head_to_back),
        ("backing_ros_m_s", o.backing_ros_m_s),
        ("flanking_ros_m_s", o.flanking_ros_m_s),
        ("eccentricity", o.eccentricity),
        ("ros_no_wind_no_slope_m_s", o.ros_no_wind_no_slope_m_s),
        ("reaction_intensity_kw_m2", o.reaction_intensity_kw_m2),
        ("fireline_intensity_kw_m", o.fireline_intensity_kw_m),
        ("flame_length_m", o.flame_length_m),
    ]
}

#[test]
fn eval_is_bit_identical_to_spread_directional() {
    let models = scott_burgan_40().unwrap();
    assert_eq!(models.len(), 40);
    let mut cases = 0_usize;
    let mut capped = [0_usize; 3];
    for model in &models {
        let bed = FuelBed::from_model(&model.to_si(), &MOIST).unwrap();
        for (li, limit) in LIMITS.into_iter().enumerate() {
            let dm = DirectionalModel::new(&bed, limit).unwrap();
            for wind in WINDS_M_S {
                for slope in SLOPES {
                    for (wind_az, up_az) in AZIMUTHS {
                        let env = DirectionalEnvironment {
                            midflame_wind_m_s: wind,
                            wind_toward_azimuth_rad: wind_az,
                            slope_tan: slope,
                            upslope_azimuth_rad: up_az,
                        };
                        let want = spread_directional(&bed, &env, limit).unwrap();
                        let got = dm.eval(&env).unwrap();
                        for ((name, g), (_, w)) in fields(&got).into_iter().zip(fields(&want)) {
                            assert_eq!(
                                g.to_bits(),
                                w.to_bits(),
                                "{} {limit:?} {env:?} {name}: eval {g:e}, spread_directional {w:e}",
                                model.code
                            );
                        }
                        let unlimited = spread_directional(&bed, &env, WindLimit::None).unwrap();
                        if want.head_ros_m_s != unlimited.head_ros_m_s {
                            capped[li] += 1;
                        }
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 40 * 3 * 3 * 3 * 4);
    assert!(
        capped[1] > 0 && capped[2] > 0,
        "the wind caps never bound: {capped:?} cases differ from WindLimit::None"
    );
}

#[test]
fn eval_rejects_what_spread_directional_rejects() {
    let model = scott_burgan_40().unwrap().remove(0);
    let bed = FuelBed::from_model(&model.to_si(), &MOIST).unwrap();
    let ok = DirectionalEnvironment {
        midflame_wind_m_s: 2.0,
        wind_toward_azimuth_rad: 0.5,
        slope_tan: 0.2,
        upslope_azimuth_rad: 1.5,
    };
    let bad = [
        DirectionalEnvironment {
            midflame_wind_m_s: -1.0,
            ..ok
        },
        DirectionalEnvironment {
            slope_tan: f64::NAN,
            ..ok
        },
        DirectionalEnvironment {
            wind_toward_azimuth_rad: f64::INFINITY,
            ..ok
        },
        DirectionalEnvironment {
            upslope_azimuth_rad: f64::NAN,
            ..ok
        },
    ];
    for limit in LIMITS {
        let dm = DirectionalModel::new(&bed, limit).unwrap();
        for env in bad {
            let got = dm.eval(&env).unwrap_err();
            let want = spread_directional(&bed, &env, limit).unwrap_err();
            match (&got, &want) {
                (
                    RothermelError::InvalidInput { field: a, .. },
                    RothermelError::InvalidInput { field: b, .. },
                ) => assert_eq!(a, b, "{env:?}"),
                _ => panic!("{env:?}: eval {got:?}, spread_directional {want:?}"),
            }
        }
    }
}
