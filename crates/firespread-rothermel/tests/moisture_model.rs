//! `MoistureModel::at` against `DirectionalModel::new` at the same moistures.

use firespread_fuel::{Moistures, scott_burgan_40};
use firespread_rothermel::{
    DeadMoistures, DirectionalEnvironment, DirectionalModel, FuelBed, MoistureModel,
    RothermelError, WindLimit,
};

/// Scott and Burgan (2005) D1/L1 moisture scenario.
const REFERENCE: Moistures = Moistures {
    dead_1h: 0.03,
    dead_10h: 0.04,
    dead_100h: 0.05,
    live_herb: 0.30,
    live_woody: 0.60,
};

/// Dead moistures from very dry to above most models' extinction moisture.
const DEAD: [DeadMoistures; 4] = [
    DeadMoistures {
        dead_1h: 0.02,
        dead_10h: 0.03,
        dead_100h: 0.05,
    },
    DeadMoistures {
        dead_1h: 0.071,
        dead_10h: 0.093,
        dead_100h: 0.12,
    },
    DeadMoistures {
        dead_1h: 0.15,
        dead_10h: 0.04,
        dead_100h: 0.09,
    },
    DeadMoistures {
        dead_1h: 0.45,
        dead_10h: 0.5,
        dead_100h: 0.6,
    },
];

const LIVE: [(f64, f64); 2] = [(0.30, 0.60), (0.90, 1.20)];

const LIMITS: [WindLimit; 3] = [
    WindLimit::None,
    WindLimit::Rothermel,
    WindLimit::EffectiveWind,
];

#[test]
fn at_equals_directional_model_new_at_the_same_moistures() {
    let models = scott_burgan_40().unwrap();
    let env = DirectionalEnvironment {
        midflame_wind_m_s: 4.0,
        wind_toward_azimuth_rad: 0.7,
        slope_tan: 0.4,
        upslope_azimuth_rad: 2.0,
    };
    let mut cases = 0_usize;
    let mut distinct_heads = std::collections::BTreeSet::new();
    for model in &models {
        let si = model.to_si();
        for (live_herb, live_woody) in LIVE {
            let reference = Moistures {
                live_herb,
                live_woody,
                ..REFERENCE
            };
            for limit in LIMITS {
                let mm = MoistureModel::new(&si, &reference, limit).unwrap();
                for dead in DEAD {
                    let m = Moistures {
                        dead_1h: dead.dead_1h,
                        dead_10h: dead.dead_10h,
                        dead_100h: dead.dead_100h,
                        ..reference
                    };
                    let want = DirectionalModel::new(&FuelBed::from_model(&si, &m).unwrap(), limit)
                        .unwrap();
                    let got = mm.at(&dead).unwrap();
                    assert_eq!(
                        got, want,
                        "{} {limit:?} live ({live_herb}, {live_woody}) {dead:?}",
                        model.code
                    );
                    let (g, w) = (got.eval(&env).unwrap(), want.eval(&env).unwrap());
                    assert_eq!(
                        g.head_ros_m_s.to_bits(),
                        w.head_ros_m_s.to_bits(),
                        "{} {dead:?}: head {} vs {}",
                        model.code,
                        g.head_ros_m_s,
                        w.head_ros_m_s
                    );
                    distinct_heads.insert(g.head_ros_m_s.to_bits());
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 40 * 2 * 3 * 4);
    // Non-degenerate: the dead moistures change the head rate.
    assert!(
        distinct_heads.len() > 40 * 2 * 3,
        "only {} distinct head rates",
        distinct_heads.len()
    );
}

#[test]
fn at_rejects_invalid_dead_moisture() {
    let si = scott_burgan_40().unwrap().remove(1).to_si();
    let mm = MoistureModel::new(&si, &REFERENCE, WindLimit::None).unwrap();
    for bad in [-0.01, f64::NAN, f64::INFINITY] {
        let err = mm
            .at(&DeadMoistures {
                dead_1h: bad,
                dead_10h: 0.04,
                dead_100h: 0.05,
            })
            .unwrap_err();
        assert!(
            matches!(
                err,
                RothermelError::InvalidClass {
                    field: "moisture",
                    ..
                }
            ),
            "{bad}: {err}"
        );
    }
}
