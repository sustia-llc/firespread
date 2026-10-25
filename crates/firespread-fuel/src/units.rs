//! Unit factors between the fire-behaviour papers' US customary units and SI.
//! Every conversion in the workspace derives from the five base factors here.

/// Kilograms per short ton.
pub const KG_PER_SHORT_TON: f64 = 907.184_74;
/// Square metres per acre.
pub const M2_PER_ACRE: f64 = 4_046.856_422_4;
/// Metres per foot.
pub const M_PER_FT: f64 = 0.3048;
/// Joules per British thermal unit (International Table).
pub const J_PER_BTU: f64 = 1_055.055_852_62;
/// Kilograms per pound.
pub const KG_PER_LB: f64 = 0.453_592_37;

/// Seconds per minute.
pub const S_PER_MIN: f64 = 60.0;

/// Short tons per acre to kilograms per square metre.
#[must_use]
pub fn t_ac_to_kg_m2(t_ac: f64) -> f64 {
    t_ac * KG_PER_SHORT_TON / M2_PER_ACRE
}

/// Kilograms per square metre to short tons per acre.
#[must_use]
pub fn kg_m2_to_t_ac(kg_m2: f64) -> f64 {
    kg_m2 * M2_PER_ACRE / KG_PER_SHORT_TON
}

/// Pounds per square foot to kilograms per square metre.
#[must_use]
pub fn lb_ft2_to_kg_m2(lb_ft2: f64) -> f64 {
    lb_ft2 * KG_PER_LB / (M_PER_FT * M_PER_FT)
}

/// Kilograms per square metre to pounds per square foot.
#[must_use]
pub fn kg_m2_to_lb_ft2(kg_m2: f64) -> f64 {
    kg_m2 * (M_PER_FT * M_PER_FT) / KG_PER_LB
}

/// Pounds per cubic foot to kilograms per cubic metre.
#[must_use]
pub fn lb_ft3_to_kg_m3(lb_ft3: f64) -> f64 {
    lb_ft3 * KG_PER_LB / (M_PER_FT * M_PER_FT * M_PER_FT)
}

/// Kilograms per cubic metre to pounds per cubic foot.
#[must_use]
pub fn kg_m3_to_lb_ft3(kg_m3: f64) -> f64 {
    kg_m3 * (M_PER_FT * M_PER_FT * M_PER_FT) / KG_PER_LB
}

/// Reciprocal feet to reciprocal metres.
#[must_use]
pub fn per_ft_to_per_m(per_ft: f64) -> f64 {
    per_ft / M_PER_FT
}

/// Reciprocal metres to reciprocal feet.
#[must_use]
pub fn per_m_to_per_ft(per_m: f64) -> f64 {
    per_m * M_PER_FT
}

/// Feet to metres.
#[must_use]
pub fn ft_to_m(ft: f64) -> f64 {
    ft * M_PER_FT
}

/// Metres to feet.
#[must_use]
pub fn m_to_ft(m: f64) -> f64 {
    m / M_PER_FT
}

/// Percent to fraction.
#[must_use]
pub fn percent_to_fraction(percent: f64) -> f64 {
    percent / 100.0
}

/// Fraction to percent.
#[must_use]
pub fn fraction_to_percent(fraction: f64) -> f64 {
    fraction * 100.0
}

/// British thermal units per pound to joules per kilogram.
#[must_use]
pub fn btu_lb_to_j_kg(btu_lb: f64) -> f64 {
    btu_lb * J_PER_BTU / KG_PER_LB
}

/// Joules per kilogram to British thermal units per pound.
#[must_use]
pub fn j_kg_to_btu_lb(j_kg: f64) -> f64 {
    j_kg * KG_PER_LB / J_PER_BTU
}

/// Feet per minute to metres per second.
#[must_use]
pub fn ft_min_to_m_s(ft_min: f64) -> f64 {
    ft_min * M_PER_FT / S_PER_MIN
}

/// Metres per second to feet per minute.
#[must_use]
pub fn m_s_to_ft_min(m_s: f64) -> f64 {
    m_s * S_PER_MIN / M_PER_FT
}

/// British thermal units per square foot per minute to watts per square metre.
#[must_use]
pub fn btu_ft2_min_to_w_m2(btu_ft2_min: f64) -> f64 {
    btu_ft2_min * J_PER_BTU / (M_PER_FT * M_PER_FT * S_PER_MIN)
}

/// British thermal units per cubic foot to joules per cubic metre.
#[must_use]
pub fn btu_ft3_to_j_m3(btu_ft3: f64) -> f64 {
    btu_ft3 * J_PER_BTU / (M_PER_FT * M_PER_FT * M_PER_FT)
}

/// British thermal units per foot per second to watts per metre.
#[must_use]
pub fn btu_ft_s_to_w_m(btu_ft_s: f64) -> f64 {
    btu_ft_s * J_PER_BTU / M_PER_FT
}

/// Minutes to seconds.
#[must_use]
pub fn min_to_s(min: f64) -> f64 {
    min * S_PER_MIN
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(a: f64, b: f64) -> f64 {
        ((a - b) / b).abs()
    }

    #[test]
    fn one_ton_per_acre_in_kg_m2() {
        let got = t_ac_to_kg_m2(1.0);
        assert!(
            rel(got, 0.224_170_24) < 1e-7,
            "1 t/ac = {got} kg/m2, expected 0.22417024"
        );
    }

    #[test]
    fn btu_per_lb_is_2326_j_per_kg() {
        let got = btu_lb_to_j_kg(1.0);
        assert!(
            rel(got, 2326.0) < 1e-9,
            "1 Btu/lb = {got} J/kg, expected 2326"
        );
    }

    #[test]
    fn ft_per_min_to_m_per_s_is_0_00508() {
        let got = ft_min_to_m_s(1.0);
        assert!(
            rel(got, 0.00508) < 1e-12,
            "1 ft/min = {got} m/s, expected 0.00508"
        );
    }

    #[test]
    fn round_trips() {
        for x in [0.1, 1.0, 37.5, 2200.0] {
            assert!(rel(kg_m2_to_t_ac(t_ac_to_kg_m2(x)), x) < 1e-12);
            assert!(rel(per_m_to_per_ft(per_ft_to_per_m(x)), x) < 1e-12);
            assert!(rel(m_to_ft(ft_to_m(x)), x) < 1e-12);
            assert!(rel(j_kg_to_btu_lb(btu_lb_to_j_kg(x)), x) < 1e-12);
            assert!(rel(kg_m2_to_lb_ft2(lb_ft2_to_kg_m2(x)), x) < 1e-12);
            assert!(rel(kg_m3_to_lb_ft3(lb_ft3_to_kg_m3(x)), x) < 1e-12);
            assert!(rel(m_s_to_ft_min(ft_min_to_m_s(x)), x) < 1e-12);
        }
    }
}
