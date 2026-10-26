//! Hourly dead fuel moisture from 2 m temperature, relative humidity and
//! cloud cover, after Cohen and Deeming (1985) GTR PSW-82.

use firespread_fuel::units;

use crate::{EnsembleError, Result};

/// Temperature added at the fuel-atmosphere interface per state-of-weather
/// code 0-3, °F (PSW-82 p. 3 table).
pub const SKY_TEMPERATURE_ADD_F: [f64; 4] = [25.0, 19.0, 12.0, 5.0];
/// Relative humidity multiplier at the fuel-atmosphere interface per
/// state-of-weather code 0-3 (PSW-82 p. 3 table).
pub const SKY_RH_MULTIPLIER: [f64; 4] = [0.75, 0.83, 0.92, 1.00];
/// MC1 = 1.03 EMCPRM without fuel moisture sticks (PSW-82 p. 3).
pub const MC1_PER_EMCPRM: f64 = 1.03;
/// MC10 = 1.28 EMCPRM without fuel moisture sticks (PSW-82 p. 4).
pub const MC10_PER_EMCPRM: f64 = 1.28;

/// Equilibrium moisture content, percent, at dry-bulb temperature `temp_f`
/// (°F) and relative humidity `rh_pct` (percent), PSW-82 p. 1:
/// eq. (1a) 0.03229 + 0.281073 RH − 0.000578 T RH for RH < 10;
/// eq. (1b) 2.22749 + 0.160107 RH − 0.014784 T for 10 ≤ RH < 50;
/// eq. (1c) 21.0606 + 0.005565 RH² − 0.00035 RH T − 0.483199 RH for RH ≥ 50.
/// Any finite inputs are evaluated; the regressions are fitted to RH in
/// [0, 100].
#[must_use]
pub fn emc_pct(temp_f: f64, rh_pct: f64) -> f64 {
    if rh_pct < 10.0 {
        0.03229 + 0.281_073 * rh_pct - 0.000_578 * temp_f * rh_pct
    } else if rh_pct < 50.0 {
        2.22749 + 0.160_107 * rh_pct - 0.014_784 * temp_f
    } else {
        21.0606 + 0.005_565 * rh_pct * rh_pct - 0.000_35 * rh_pct * temp_f - 0.483_199 * rh_pct
    }
}

/// Cloud-fraction edges `[e1, e2, e3]` of the state-of-weather codes: code 0
/// below `e1`, 1 in `[e1, e2)`, 2 in `[e2, e3)`, 3 at or above `e3`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyEdges(pub [f64; 3]);

impl Default for SkyEdges {
    /// `[0.1, 0.55, 0.9]`: INT-169 Table 6 (p. 13) with its 0.5 to 0.6 gap
    /// split at 0.55.
    fn default() -> Self {
        Self([0.1, 0.55, 0.9])
    }
}

impl SkyEdges {
    /// The edges if they are finite and strictly increasing.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidSkyEdges`] otherwise.
    pub fn validated(self) -> Result<Self> {
        let [a, b, c] = self.0;
        if [a, b, c].iter().all(|e| e.is_finite()) && a < b && b < c {
            Ok(self)
        } else {
            Err(EnsembleError::InvalidSkyEdges { edges: self.0 })
        }
    }

    /// The state-of-weather code 0-3 of cloud fraction `fraction`: the
    /// number of edges at or below it. NaN gives 0.
    #[must_use]
    pub fn code(&self, fraction: f64) -> usize {
        self.0.iter().filter(|&&e| e <= fraction).count()
    }
}

/// The dead fuel moisture model of PSW-82 at one cell and hour.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DeadFuelMoisture {
    pub sky_edges: SkyEdges,
}

impl DeadFuelMoisture {
    /// EMCPRM (percent), the EMC at the fuel-atmosphere interface (PSW-82
    /// p. 3), at 2 m temperature `t2m_k` (K), relative humidity `rh_pct`
    /// (percent) and total cloud cover `tcc_pct` (percent): the sky code
    /// is [`SkyEdges::code`] of `tcc_pct / 100`; TMPPRM is the temperature in
    /// °F plus [`SKY_TEMPERATURE_ADD_F`]; RHPRM is `rh_pct` times
    /// [`SKY_RH_MULTIPLIER`], capped at 100; EMCPRM is [`emc_pct`] of
    /// TMPPRM and RHPRM.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidWeather`] when a value is not finite or the
    /// relative humidity is negative.
    pub fn emcprm_pct(&self, t2m_k: f64, rh_pct: f64, tcc_pct: f64) -> Result<f64> {
        let invalid = |field, bound, value| EnsembleError::InvalidWeather {
            field,
            bound,
            value,
        };
        if !t2m_k.is_finite() {
            return Err(invalid("t2m", "a finite value", t2m_k));
        }
        if !(rh_pct.is_finite() && rh_pct >= 0.0) {
            return Err(invalid("rh2m", "a finite value >= 0", rh_pct));
        }
        if !tcc_pct.is_finite() {
            return Err(invalid("tcc", "a finite value", tcc_pct));
        }
        let sky = self.sky_edges.code(units::percent_to_fraction(tcc_pct));
        let tmpprm = units::k_to_f(t2m_k) + SKY_TEMPERATURE_ADD_F[sky];
        let rhprm = (rh_pct * SKY_RH_MULTIPLIER[sky]).min(100.0);
        Ok(emc_pct(tmpprm, rhprm))
    }
}

/// 1-h dead fuel moisture, fraction, `1.03 EMCPRM / 100` (PSW-82 p. 3), for
/// EMCPRM `emcprm_pct` in percent.
#[must_use]
pub fn mc1_fraction(emcprm_pct: f64) -> f64 {
    units::percent_to_fraction(MC1_PER_EMCPRM * emcprm_pct)
}

/// 10-h dead fuel moisture, fraction, `1.28 EMCPRM / 100` (PSW-82 p. 4), for
/// EMCPRM `emcprm_pct` in percent.
#[must_use]
pub fn mc10_fraction(emcprm_pct: f64) -> f64 {
    units::percent_to_fraction(MC10_PER_EMCPRM * emcprm_pct)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, want: f64, what: &str) {
        assert!(
            (got - want).abs() <= 1e-9,
            "{what}: got {got}, hand value {want}"
        );
    }

    #[test]
    fn emc_hand_values_in_each_branch() {
        // (1a) T 80, RH 5: 0.03229 + 1.405365 − 0.2312.
        close(emc_pct(80.0, 5.0), 1.206_455, "1a");
        // (1b) T 70, RH 30: 2.22749 + 4.80321 − 1.03488.
        close(emc_pct(70.0, 30.0), 5.995_82, "1b");
        // (1c) T 60, RH 70: 21.0606 + 27.2685 − 1.47 − 33.82393.
        close(emc_pct(60.0, 70.0), 13.035_17, "1c");
        // Branch edges: RH 10 is (1b), RH 50 is (1c).
        // (1b) T 50, RH 10: 2.22749 + 1.60107 − 0.7392.
        close(emc_pct(50.0, 10.0), 3.089_36, "1b at RH 10");
        // (1c) T 50, RH 50: 21.0606 + 13.9125 − 0.875 − 24.15995.
        close(emc_pct(50.0, 50.0), 9.938_15, "1c at RH 50");
    }

    #[test]
    fn sky_code_edges() {
        let edges = SkyEdges::default();
        for (fraction, want) in [
            (0.0, 0),
            (0.099_999, 0),
            (0.1, 1),
            (0.549_999, 1),
            (0.55, 2),
            (0.899_999, 2),
            (0.9, 3),
            (1.0, 3),
        ] {
            assert_eq!(edges.code(fraction), want, "cloud fraction {fraction}");
        }
        assert!(SkyEdges([0.5, 0.2, 0.9]).validated().is_err());
        assert!(SkyEdges([0.1, 0.1, 0.9]).validated().is_err());
        assert!(SkyEdges::default().validated().is_ok());
    }

    #[test]
    fn emcprm_mc1_mc10_hand_values() {
        let model = DeadFuelMoisture::default();
        // 300 K = 80.33 °F; clear sky (code 0): TMPPRM 105.33, RHPRM 15;
        // (1b) 2.22749 + 2.401605 − 1.55719872 = 3.07189628.
        let e = model.emcprm_pct(300.0, 20.0, 0.0).unwrap();
        close(e, 3.071_896_28, "EMCPRM clear");
        close(mc1_fraction(e), 0.031_640_531_684, "MC1 = 1.03 EMCPRM");
        close(mc10_fraction(e), 0.039_320_272_384, "MC10 = 1.28 EMCPRM");
        // 280 K = 44.33 °F; overcast 95 % (code 3): TMPPRM 49.33, RHPRM
        // 105 · 1.00 capped at 100; (1c) 21.0606 + 55.65 − 1.72655 − 48.3199.
        let e = model.emcprm_pct(280.0, 105.0, 95.0).unwrap();
        close(e, 26.664_15, "EMCPRM capped");
        // 290 K = 62.33 °F; 60 % cloud (code 2): TMPPRM 74.33, RHPRM 36.8;
        // (1b) 2.22749 + 5.8919376 − 1.09889472 = 7.02053288.
        let e = model.emcprm_pct(290.0, 40.0, 60.0).unwrap();
        close(e, 7.020_532_88, "EMCPRM code 2");
        assert!(model.emcprm_pct(290.0, -1.0, 0.0).is_err());
        assert!(model.emcprm_pct(f64::NAN, 10.0, 0.0).is_err());
    }
}
