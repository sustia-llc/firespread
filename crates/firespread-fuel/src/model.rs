//! The Scott and Burgan (2005) 40-model table in the paper's units and in SI.

use serde::Deserialize;

use crate::{FuelError, Result, units};

/// Surface-area-to-volume ratio of the dead 10-h class, 1/ft (Scott and Burgan 2005 pp. 2-3).
pub const SAV_10H_PER_FT: f64 = 109.0;
/// Surface-area-to-volume ratio of the dead 100-h class, 1/ft (Scott and Burgan 2005 pp. 2-3).
pub const SAV_100H_PER_FT: f64 = 30.0;
/// Total mineral content S_T, fraction (Scott and Burgan 2005 pp. 2-3).
pub const TOTAL_MINERAL_CONTENT: f64 = 0.0555;
/// Effective (silica-free) mineral content S_e, fraction (Scott and Burgan 2005 pp. 2-3).
pub const EFFECTIVE_MINERAL_CONTENT: f64 = 0.01;
/// Oven-dry fuel particle density rho_p, lb/ft3 (Scott and Burgan 2005 pp. 2-3).
pub const PARTICLE_DENSITY_LB_PER_FT3: f64 = 32.0;

/// Sentinel the FireBench table writes for a live class that carries no load.
const NO_LOAD_SAV_SENTINEL: f64 = 9999.0;

const TABLE_CSV: &str = include_str!("../data/data_ScottandBurgan40.csv");

#[derive(Debug, Deserialize)]
struct Row {
    fuel_model_code: String,
    fuel_load_1h: f64,
    fuel_load_10h: f64,
    fuel_load_100h: f64,
    fuel_load_live_herb: f64,
    fuel_load_live_woody: f64,
    fuel_model_type: String,
    sav_ratio_dead_1h: f64,
    sav_ratio_live_herb: f64,
    sav_ratio_live_woody: f64,
    fuel_bed_depth: f64,
    dead_fuel_extinction_moisture: f64,
    heat_content: f64,
}

/// One Scott and Burgan fuel model in the paper's units: loads in short tons
/// per acre, surface-area-to-volume ratios in 1/ft, depth in ft, dead
/// moisture of extinction in percent, heat content in Btu/lb. A live class
/// without load carries `None` for its surface-area-to-volume ratio.
#[derive(Debug, Clone, PartialEq)]
pub struct FuelModel {
    pub code: String,
    pub load_1h_t_ac: f64,
    pub load_10h_t_ac: f64,
    pub load_100h_t_ac: f64,
    pub load_live_herb_t_ac: f64,
    pub load_live_woody_t_ac: f64,
    pub dynamic: bool,
    pub sav_dead_1h_per_ft: f64,
    pub sav_live_herb_per_ft: Option<f64>,
    pub sav_live_woody_per_ft: Option<f64>,
    pub depth_ft: f64,
    pub dead_mx_percent: f64,
    pub heat_btu_per_lb: f64,
}

/// The same fuel model in SI: loads in kg/m2, surface-area-to-volume ratios
/// in 1/m, depth in m, dead moisture of extinction as a fraction, heat
/// content in J/kg.
#[derive(Debug, Clone, PartialEq)]
pub struct FuelModelSi {
    pub code: String,
    pub load_1h_kg_m2: f64,
    pub load_10h_kg_m2: f64,
    pub load_100h_kg_m2: f64,
    pub load_live_herb_kg_m2: f64,
    pub load_live_woody_kg_m2: f64,
    pub dynamic: bool,
    pub sav_dead_1h_per_m: f64,
    pub sav_live_herb_per_m: Option<f64>,
    pub sav_live_woody_per_m: Option<f64>,
    pub depth_m: f64,
    pub dead_mx: f64,
    pub heat_j_per_kg: f64,
}

impl FuelModel {
    /// Converts every field to SI.
    #[must_use]
    pub fn to_si(&self) -> FuelModelSi {
        FuelModelSi {
            code: self.code.clone(),
            load_1h_kg_m2: units::t_ac_to_kg_m2(self.load_1h_t_ac),
            load_10h_kg_m2: units::t_ac_to_kg_m2(self.load_10h_t_ac),
            load_100h_kg_m2: units::t_ac_to_kg_m2(self.load_100h_t_ac),
            load_live_herb_kg_m2: units::t_ac_to_kg_m2(self.load_live_herb_t_ac),
            load_live_woody_kg_m2: units::t_ac_to_kg_m2(self.load_live_woody_t_ac),
            dynamic: self.dynamic,
            sav_dead_1h_per_m: units::per_ft_to_per_m(self.sav_dead_1h_per_ft),
            sav_live_herb_per_m: self.sav_live_herb_per_ft.map(units::per_ft_to_per_m),
            sav_live_woody_per_m: self.sav_live_woody_per_ft.map(units::per_ft_to_per_m),
            depth_m: units::ft_to_m(self.depth_ft),
            dead_mx: units::percent_to_fraction(self.dead_mx_percent),
            heat_j_per_kg: units::btu_lb_to_j_kg(self.heat_btu_per_lb),
        }
    }

    /// Converts every field from SI back to the paper's units.
    #[must_use]
    pub fn from_si(si: &FuelModelSi) -> Self {
        Self {
            code: si.code.clone(),
            load_1h_t_ac: units::kg_m2_to_t_ac(si.load_1h_kg_m2),
            load_10h_t_ac: units::kg_m2_to_t_ac(si.load_10h_kg_m2),
            load_100h_t_ac: units::kg_m2_to_t_ac(si.load_100h_kg_m2),
            load_live_herb_t_ac: units::kg_m2_to_t_ac(si.load_live_herb_kg_m2),
            load_live_woody_t_ac: units::kg_m2_to_t_ac(si.load_live_woody_kg_m2),
            dynamic: si.dynamic,
            sav_dead_1h_per_ft: units::per_m_to_per_ft(si.sav_dead_1h_per_m),
            sav_live_herb_per_ft: si.sav_live_herb_per_m.map(units::per_m_to_per_ft),
            sav_live_woody_per_ft: si.sav_live_woody_per_m.map(units::per_m_to_per_ft),
            depth_ft: units::m_to_ft(si.depth_m),
            dead_mx_percent: units::fraction_to_percent(si.dead_mx),
            heat_btu_per_lb: units::j_kg_to_btu_lb(si.heat_j_per_kg),
        }
    }
}

impl FuelModelSi {
    /// Total oven-dry load over the five classes, kg/m2.
    #[must_use]
    pub fn total_load_kg_m2(&self) -> f64 {
        self.load_1h_kg_m2
            + self.load_10h_kg_m2
            + self.load_100h_kg_m2
            + self.load_live_herb_kg_m2
            + self.load_live_woody_kg_m2
    }
}

fn check(code: &str, field: &'static str, value: f64, positive: bool) -> Result<f64> {
    let ok = value.is_finite() && if positive { value > 0.0 } else { value >= 0.0 };
    if ok {
        Ok(value)
    } else {
        Err(FuelError::InvalidValue {
            code: code.to_owned(),
            field,
            bound: if positive { "positive" } else { "non-negative" },
            value,
        })
    }
}

fn live_sav(code: &str, field: &'static str, value: f64) -> Result<Option<f64>> {
    if value == NO_LOAD_SAV_SENTINEL {
        Ok(None)
    } else {
        check(code, field, value, true).map(Some)
    }
}

fn row_to_model(index: usize, row: Row) -> Result<FuelModel> {
    let code = row.fuel_model_code;
    let dynamic = match row.fuel_model_type.as_str() {
        "dynamic" => true,
        "N/A" => false,
        other => {
            return Err(FuelError::UnknownModelType {
                row: index + 1,
                value: other.to_owned(),
            });
        }
    };
    Ok(FuelModel {
        load_1h_t_ac: check(&code, "fuel_load_1h", row.fuel_load_1h, false)?,
        load_10h_t_ac: check(&code, "fuel_load_10h", row.fuel_load_10h, false)?,
        load_100h_t_ac: check(&code, "fuel_load_100h", row.fuel_load_100h, false)?,
        load_live_herb_t_ac: check(&code, "fuel_load_live_herb", row.fuel_load_live_herb, false)?,
        load_live_woody_t_ac: check(
            &code,
            "fuel_load_live_woody",
            row.fuel_load_live_woody,
            false,
        )?,
        dynamic,
        sav_dead_1h_per_ft: check(&code, "sav_ratio_dead_1h", row.sav_ratio_dead_1h, true)?,
        sav_live_herb_per_ft: live_sav(&code, "sav_ratio_live_herb", row.sav_ratio_live_herb)?,
        sav_live_woody_per_ft: live_sav(&code, "sav_ratio_live_woody", row.sav_ratio_live_woody)?,
        depth_ft: check(&code, "fuel_bed_depth", row.fuel_bed_depth, true)?,
        dead_mx_percent: check(
            &code,
            "dead_fuel_extinction_moisture",
            row.dead_fuel_extinction_moisture,
            true,
        )?,
        heat_btu_per_lb: check(&code, "heat_content", row.heat_content, true)?,
        code,
    })
}

/// Parses the embedded Scott and Burgan 40 table into fuel models in the
/// paper's units, in table order.
pub fn scott_burgan_40() -> Result<Vec<FuelModel>> {
    let mut reader = csv::Reader::from_reader(TABLE_CSV.as_bytes());
    reader
        .deserialize::<Row>()
        .enumerate()
        .map(|(index, row)| row_to_model(index, row?))
        .collect()
}

/// Returns the fuel model whose code (for example `GR1`) matches exactly.
pub fn by_code(code: &str) -> Result<FuelModel> {
    scott_burgan_40()?
        .into_iter()
        .find(|m| m.code == code)
        .ok_or_else(|| FuelError::UnknownCode {
            code: code.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(a: f64, b: f64) -> f64 {
        if b == 0.0 {
            a.abs()
        } else {
            ((a - b) / b).abs()
        }
    }

    #[test]
    fn table_has_forty_models() {
        let models = scott_burgan_40().unwrap();
        assert_eq!(models.len(), 40, "got {} models", models.len());
    }

    #[test]
    fn gr1_row() {
        let gr1 = by_code("GR1").unwrap();
        assert_eq!(gr1.load_1h_t_ac, 0.10);
        assert_eq!(gr1.load_10h_t_ac, 0.0);
        assert_eq!(gr1.load_100h_t_ac, 0.0);
        assert_eq!(gr1.load_live_herb_t_ac, 0.30);
        assert_eq!(gr1.load_live_woody_t_ac, 0.0);
        assert!(gr1.dynamic);
        assert_eq!(gr1.sav_dead_1h_per_ft, 2200.0);
        assert_eq!(gr1.sav_live_herb_per_ft, Some(2000.0));
        assert_eq!(gr1.sav_live_woody_per_ft, None);
        assert_eq!(gr1.depth_ft, 0.4);
        assert_eq!(gr1.dead_mx_percent, 15.0);
        assert_eq!(gr1.heat_btu_per_lb, 8000.0);
    }

    /// Scott and Burgan (2005) list GR6 at 9,000 Btu/lb; the FireBench table
    /// embedded here writes 8,000. This pins what the table says.
    #[test]
    fn gr6_heat_as_tabulated() {
        let gr6 = by_code("GR6").unwrap();
        assert_eq!(
            gr6.heat_btu_per_lb, 8000.0,
            "GR6 heat_content from the FireBench table"
        );
    }

    #[test]
    fn sh2_is_static_without_herb() {
        let sh2 = by_code("SH2").unwrap();
        assert!(!sh2.dynamic);
        assert_eq!(sh2.load_live_herb_t_ac, 0.0);
        assert_eq!(sh2.sav_live_herb_per_ft, None);
        assert_eq!(sh2.sav_live_woody_per_ft, Some(1600.0));
    }

    #[test]
    fn unknown_code_is_an_error() {
        let err = by_code("ZZ9").unwrap_err();
        assert!(
            matches!(err, FuelError::UnknownCode { ref code } if code == "ZZ9"),
            "{err}"
        );
    }

    #[test]
    fn gr1_si_values() {
        let si = by_code("GR1").unwrap().to_si();
        assert!(
            rel(si.load_1h_kg_m2, 0.022_417_024) < 1e-7,
            "{}",
            si.load_1h_kg_m2
        );
        assert!(
            rel(si.load_live_herb_kg_m2, 0.067_251_072) < 1e-7,
            "{}",
            si.load_live_herb_kg_m2
        );
        assert!(
            rel(si.sav_dead_1h_per_m, 7_217.847_769) < 1e-8,
            "{}",
            si.sav_dead_1h_per_m
        );
        assert!(rel(si.sav_live_herb_per_m.unwrap(), 6_561.679_790) < 1e-8);
        assert_eq!(si.sav_live_woody_per_m, None);
        assert!(rel(si.depth_m, 0.12192) < 1e-12, "{}", si.depth_m);
        assert!(rel(si.dead_mx, 0.15) < 1e-12, "{}", si.dead_mx);
        assert!(
            rel(si.heat_j_per_kg, 18_608_000.0) < 1e-9,
            "{}",
            si.heat_j_per_kg
        );
    }

    #[test]
    fn si_round_trip_within_1e9_relative() {
        for model in scott_burgan_40().unwrap() {
            let back = FuelModel::from_si(&model.to_si());
            let code = &model.code;
            let pairs = [
                ("load_1h", model.load_1h_t_ac, back.load_1h_t_ac),
                ("load_10h", model.load_10h_t_ac, back.load_10h_t_ac),
                ("load_100h", model.load_100h_t_ac, back.load_100h_t_ac),
                (
                    "load_live_herb",
                    model.load_live_herb_t_ac,
                    back.load_live_herb_t_ac,
                ),
                (
                    "load_live_woody",
                    model.load_live_woody_t_ac,
                    back.load_live_woody_t_ac,
                ),
                (
                    "sav_dead_1h",
                    model.sav_dead_1h_per_ft,
                    back.sav_dead_1h_per_ft,
                ),
                ("depth", model.depth_ft, back.depth_ft),
                ("dead_mx", model.dead_mx_percent, back.dead_mx_percent),
                ("heat", model.heat_btu_per_lb, back.heat_btu_per_lb),
            ];
            for (field, a, b) in pairs {
                assert!(rel(b, a) < 1e-9, "{code} {field}: {a} -> {b}");
            }
            assert_eq!(model.dynamic, back.dynamic, "{code} dynamic");
            for (field, a, b) in [
                (
                    "sav_live_herb",
                    model.sav_live_herb_per_ft,
                    back.sav_live_herb_per_ft,
                ),
                (
                    "sav_live_woody",
                    model.sav_live_woody_per_ft,
                    back.sav_live_woody_per_ft,
                ),
            ] {
                match (a, b) {
                    (None, None) => {}
                    (Some(a), Some(b)) => assert!(rel(b, a) < 1e-9, "{code} {field}: {a} -> {b}"),
                    _ => panic!("{code} {field}: {a:?} -> {b:?}"),
                }
            }
        }
    }
}
