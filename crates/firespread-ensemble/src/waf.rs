//! The wind adjustment factor (WAF) from 20 ft wind to midflame wind, after
//! Andrews (2012) RMRS-GTR-266.

use firespread_fuel::units;

use crate::{EnsembleError, Result};

/// Ratio of 10 m wind to 20 ft wind, U20ft = U10 / 1.15 (Andrews 2012
/// RMRS-GTR-266 p. 16, after Turner and Lawson 1978).
pub const U10_PER_U20FT: f64 = 1.15;

/// Crown fill portion above which BehavePlus uses the sheltered WAF
/// (RMRS-GTR-266 p. 8).
pub const BEHAVEPLUS_SHELTER_FILL: f64 = 0.05;

/// The 20 ft wind speed of a 10 m wind speed `u10_m_s`, `u10_m_s / 1.15`,
/// m/s, for any finite input.
#[must_use]
pub fn u20ft_from_u10(u10_m_s: f64) -> f64 {
    u10_m_s / U10_PER_U20FT
}

/// ln((20 + 0.36 H) / (0.13 H)) with H in ft, the log-profile term of
/// RMRS-GTR-266 eqs. \[1\] and \[2\] (p. 8).
fn log_profile(h_ft: f64) -> f64 {
    ((20.0 + 0.36 * h_ft) / (0.13 * h_ft)).ln()
}

fn positive(field: &'static str, value: f64) -> Result<f64> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(EnsembleError::InvalidCanopy {
            field,
            bound: "a finite value > 0",
            value,
        })
    }
}

/// Unsheltered WAF, RMRS-GTR-266 eq. \[1\] p. 8:
/// 1.83 / ln((20 + 0.36 H) / (0.13 H)), H the fuel bed depth in ft, for a
/// fuel bed depth `fuel_depth_m` in metres.
///
/// # Errors
/// [`EnsembleError::InvalidCanopy`] unless `fuel_depth_m` is finite and > 0.
pub fn waf_unsheltered(fuel_depth_m: f64) -> Result<f64> {
    let h_ft = units::m_to_ft(positive("fuel depth", fuel_depth_m)?);
    Ok(1.83 / log_profile(h_ft))
}

/// Sheltered WAF, RMRS-GTR-266 eq. \[2\] p. 8:
/// 0.555 / (√(f H) ln((20 + 0.36 H) / (0.13 H))), H the canopy height in ft
/// and f the crown fill portion, for a canopy height `canopy_height_m` in
/// metres and a crown fill portion `crown_fill` (fraction).
///
/// # Errors
/// [`EnsembleError::InvalidCanopy`] unless both inputs are finite and > 0.
pub fn waf_sheltered(canopy_height_m: f64, crown_fill: f64) -> Result<f64> {
    let h_ft = units::m_to_ft(positive("canopy height", canopy_height_m)?);
    let f = positive("crown fill portion", crown_fill)?;
    Ok(0.555 / ((f * h_ft).sqrt() * log_profile(h_ft)))
}

/// The canopy of one cell, as LANDFIRE CH, CBH and CC give it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canopy {
    /// Canopy height CH, m.
    pub height_m: f64,
    /// Canopy base height CBH, m.
    pub base_height_m: f64,
    /// Canopy cover CC, percent.
    pub cover_pct: f64,
}

/// How a WAF is chosen from the unsheltered and sheltered models of
/// RMRS-GTR-266.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WafRule {
    /// FuelCalc (p. 33): the smaller of the unsheltered and sheltered WAF,
    /// with f = (CC/100)/3 · CR (eqs. \[9\], \[10\], p. 11) and
    /// CR = (CH − CBH)/CH; the unsheltered WAF where f is 0.
    #[default]
    Min,
    /// BehavePlus (p. 8): the sheltered WAF where f > 5 %, else the
    /// unsheltered WAF, with f as in [`WafRule::Min`].
    BehavePlus,
    /// FARSITE (p. 32): f = CC·π/1200, the sheltered WAF where CC > 0, else
    /// the unsheltered WAF.
    Farsite,
    /// [`WafRule::Min`] with CR = 1 (p. 11).
    MinCrownRatioOne,
}

impl WafRule {
    /// The crown fill portion f (fraction) of `canopy` under this rule: 0
    /// where CH ≤ 0, CC = 0, or (for [`WafRule::Min`] and
    /// [`WafRule::BehavePlus`]) CBH ≥ CH.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidCanopy`] when CH or CBH is negative or not
    /// finite, or CC is outside [0, 100].
    pub fn crown_fill(self, canopy: &Canopy) -> Result<f64> {
        let Canopy {
            height_m: ch,
            base_height_m: cbh,
            cover_pct: cc,
        } = *canopy;
        let invalid = |field, bound, value| EnsembleError::InvalidCanopy {
            field,
            bound,
            value,
        };
        if !(ch.is_finite() && ch >= 0.0) {
            return Err(invalid("canopy height", "a finite value >= 0", ch));
        }
        if !(cbh.is_finite() && cbh >= 0.0) {
            return Err(invalid("canopy base height", "a finite value >= 0", cbh));
        }
        if !(cc.is_finite() && (0.0..=100.0).contains(&cc)) {
            return Err(invalid("canopy cover", "a value in [0, 100]", cc));
        }
        if ch <= 0.0 {
            return Ok(0.0);
        }
        let cover = units::percent_to_fraction(cc);
        Ok(match self {
            Self::Min | Self::BehavePlus => {
                let crown_ratio = ((ch - cbh) / ch).max(0.0);
                cover / 3.0 * crown_ratio
            }
            Self::MinCrownRatioOne => cover / 3.0,
            Self::Farsite => cover * core::f64::consts::PI / 12.0,
        })
    }

    /// The WAF of a cell with fuel bed depth `fuel_depth_m` (m) under
    /// `canopy`: the unsheltered WAF where the crown fill portion is 0 (no
    /// canopy height, no cover, or CBH ≥ CH), else as the rule says.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidCanopy`] for the errors of
    /// [`WafRule::crown_fill`] and a fuel depth that is not finite and > 0.
    pub fn waf(self, fuel_depth_m: f64, canopy: &Canopy) -> Result<f64> {
        let unsheltered = waf_unsheltered(fuel_depth_m)?;
        let f = self.crown_fill(canopy)?;
        if f <= 0.0 {
            return Ok(unsheltered);
        }
        let sheltered = || waf_sheltered(canopy.height_m, f);
        Ok(match self {
            Self::Min | Self::MinCrownRatioOne => unsheltered.min(sheltered()?),
            Self::BehavePlus => {
                if f > BEHAVEPLUS_SHELTER_FILL {
                    sheltered()?
                } else {
                    unsheltered
                }
            }
            Self::Farsite => sheltered()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, want: f64, what: &str) {
        assert!(
            (got - want).abs() <= 2e-6,
            "{what}: got {got}, hand value {want}"
        );
    }

    /// 1 ft and 2 ft in metres.
    const FT1: f64 = 0.3048;
    const FT2: f64 = 0.6096;

    #[test]
    fn u20ft_is_u10_over_1_15() {
        for (u10, want) in [(11.5, 10.0), (2.3, 2.0)] {
            let got = u20ft_from_u10(u10);
            assert!(
                (got - want).abs() < 1e-12,
                "U10 {u10}: U20ft {got}, expected {want}"
            );
        }
    }

    #[test]
    fn unsheltered_waf_hand_values() {
        // H = 1 ft: 1.83 / ln(20.36 / 0.13) = 1.83 / 5.053793 = 0.362104.
        close(waf_unsheltered(FT1).unwrap(), 0.362_104, "WAF_u(1 ft)");
        // H = 2 ft: 1.83 / ln(20.72 / 0.26) = 1.83 / 4.378174 = 0.417982.
        close(waf_unsheltered(FT2).unwrap(), 0.417_982, "WAF_u(2 ft)");
    }

    #[test]
    fn sheltered_waf_hand_values() {
        // CH 100 ft, CBH 50 ft, CC 50 %: CR 0.5, f = 0.5/3 · 0.5 = 0.083333;
        // √(8.33333) = 2.886751, ln(56/13) = 1.460402,
        // 0.555 / 4.215817 = 0.131647.
        let a = Canopy {
            height_m: 30.48,
            base_height_m: 15.24,
            cover_pct: 50.0,
        };
        let f = WafRule::Min.crown_fill(&a).unwrap();
        close(f, 0.5 / 3.0 * 0.5, "f(a)");
        close(waf_sheltered(a.height_m, f).unwrap(), 0.131_647, "WAF_s(a)");
        // CH 15 m = 49.212598 ft, CBH 3 m, CC 30 %: CR 0.8, f = 0.08;
        // √(3.937008) = 1.984190, ln(37.716535 / 6.397638) = 1.774163,
        // 0.555 / 3.520276 = 0.157658.
        let b = Canopy {
            height_m: 15.0,
            base_height_m: 3.0,
            cover_pct: 30.0,
        };
        let f = WafRule::Min.crown_fill(&b).unwrap();
        close(f, 0.08, "f(b)");
        close(waf_sheltered(b.height_m, f).unwrap(), 0.157_658, "WAF_s(b)");
    }

    #[test]
    fn min_rule_takes_the_smaller_waf() {
        // Sheltered below unsheltered: the sheltered values above.
        let a = Canopy {
            height_m: 30.48,
            base_height_m: 15.24,
            cover_pct: 50.0,
        };
        close(WafRule::Min.waf(FT1, &a).unwrap(), 0.131_647, "min(a)");
        let b = Canopy {
            height_m: 15.0,
            base_height_m: 3.0,
            cover_pct: 30.0,
        };
        close(WafRule::Min.waf(FT2, &b).unwrap(), 0.157_658, "min(b)");
        // Sheltered above unsheltered: CH 3 m, CBH 2.5 m, CC 10 % gives
        // f = 0.005556 and WAF_s = 0.81495 > WAF_u(1 ft) = 0.362104.
        let c = Canopy {
            height_m: 3.0,
            base_height_m: 2.5,
            cover_pct: 10.0,
        };
        let f = WafRule::Min.crown_fill(&c).unwrap();
        assert!(waf_sheltered(c.height_m, f).unwrap() > 0.8);
        close(WafRule::Min.waf(FT1, &c).unwrap(), 0.362_104, "min(c)");
    }

    #[test]
    fn min_rule_without_shelter_is_unsheltered() {
        let u = waf_unsheltered(FT1).unwrap();
        for (what, canopy) in [
            (
                "CH = 0",
                Canopy {
                    height_m: 0.0,
                    base_height_m: 0.0,
                    cover_pct: 60.0,
                },
            ),
            (
                "CBH = CH",
                Canopy {
                    height_m: 10.0,
                    base_height_m: 10.0,
                    cover_pct: 60.0,
                },
            ),
            (
                "CBH > CH",
                Canopy {
                    height_m: 10.0,
                    base_height_m: 12.0,
                    cover_pct: 60.0,
                },
            ),
            (
                "CC = 0",
                Canopy {
                    height_m: 20.0,
                    base_height_m: 2.0,
                    cover_pct: 0.0,
                },
            ),
        ] {
            assert_eq!(
                WafRule::Min.crown_fill(&canopy).unwrap(),
                0.0,
                "{what}: crown fill"
            );
            assert_eq!(WafRule::Min.waf(FT1, &canopy).unwrap(), u, "{what}");
        }
    }

    #[test]
    fn behaveplus_shelters_above_five_percent_fill() {
        let a = Canopy {
            height_m: 30.48,
            base_height_m: 15.24,
            cover_pct: 50.0,
        };
        close(
            WafRule::BehavePlus.waf(FT1, &a).unwrap(),
            0.131_647,
            "f 8.3 %",
        );
        // CC 10 %, CR 0.5: f = 1.67 % < 5 %.
        let thin = Canopy {
            cover_pct: 10.0,
            ..a
        };
        close(
            WafRule::BehavePlus.waf(FT1, &thin).unwrap(),
            0.362_104,
            "f 1.7 %",
        );
    }

    #[test]
    fn farsite_uses_cc_pi_over_1200() {
        // CH 100 ft, CC 50 %: f = 0.5 π/12 = 0.130900, √13.0900 = 3.618007,
        // 0.555 / (3.618007 · 1.460402) = 0.105039.
        let a = Canopy {
            height_m: 30.48,
            base_height_m: 15.24,
            cover_pct: 50.0,
        };
        close(WafRule::Farsite.waf(FT1, &a).unwrap(), 0.105_039, "FARSITE");
    }

    #[test]
    fn crown_ratio_one_ignores_cbh() {
        // CH 100 ft, CC 50 %, CR 1: f = 1/6, √16.6667 = 4.082483,
        // 0.555 / (4.082483 · 1.460402) = 0.093088.
        let a = Canopy {
            height_m: 30.48,
            base_height_m: 15.24,
            cover_pct: 50.0,
        };
        close(
            WafRule::MinCrownRatioOne.waf(FT1, &a).unwrap(),
            0.093_088,
            "CR = 1",
        );
    }

    #[test]
    fn invalid_canopy_is_an_error() {
        let ok = Canopy {
            height_m: 10.0,
            base_height_m: 2.0,
            cover_pct: 40.0,
        };
        for bad in [
            Canopy {
                height_m: -1.0,
                ..ok
            },
            Canopy {
                base_height_m: f64::NAN,
                ..ok
            },
            Canopy {
                cover_pct: 101.0,
                ..ok
            },
        ] {
            assert!(
                matches!(
                    WafRule::Min.waf(FT1, &bad),
                    Err(EnsembleError::InvalidCanopy { .. })
                ),
                "{bad:?}"
            );
        }
        assert!(waf_unsheltered(0.0).is_err());
    }
}
