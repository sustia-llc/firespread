//! Dynamic live-to-dead herbaceous load transfer and the resulting fuel classes.

use crate::{FuelError, FuelModelSi, Result, model, units};

/// Live or dead fuel category (Andrews 2018 Table 5: i = 1 dead, i = 2 live).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Dead,
    Live,
}

/// Size class of a fuel class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassKind {
    Dead1h,
    Dead10h,
    Dead100h,
    /// Herbaceous load moved from live to dead by the dynamic transfer; it
    /// keeps the live-herb surface-area-to-volume ratio and takes the dead
    /// 1-h moisture.
    DeadHerb,
    LiveHerb,
    LiveWoody,
}

/// One fuel class in SI, ready for the spread model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuelClass {
    pub category: Category,
    pub kind: ClassKind,
    pub load_kg_m2: f64,
    pub sav_per_m: f64,
    /// Moisture content, fraction of oven-dry weight.
    pub moisture: f64,
}

/// Moisture contents by class, fractions of oven-dry weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moistures {
    pub dead_1h: f64,
    pub dead_10h: f64,
    pub dead_100h: f64,
    pub live_herb: f64,
    pub live_woody: f64,
}

/// Live herbaceous moisture (fraction) at or below which all herbaceous load
/// is cured to dead (Scott and Burgan 2005 p. 6).
pub const TRANSFER_MOISTURE_ALL: f64 = 0.30;
/// Live herbaceous moisture (fraction) at or above which no herbaceous load
/// is cured (Scott and Burgan 2005 p. 6).
pub const TRANSFER_MOISTURE_NONE: f64 = 1.20;

/// Fraction of live herbaceous load transferred to the dead herbaceous class:
/// 0 at live herb moisture >= 1.20, 1 at <= 0.30, linear between (Scott and
/// Burgan 2005 p. 6; Andrews 2018 §4.6.2 and Table 7 write the same line
/// rounded as T = -1.11 M + 1.33, Burgan 1979).
#[must_use]
pub fn transfer_fraction(live_herb_moisture: f64) -> f64 {
    ((TRANSFER_MOISTURE_NONE - live_herb_moisture)
        / (TRANSFER_MOISTURE_NONE - TRANSFER_MOISTURE_ALL))
        .clamp(0.0, 1.0)
}

fn check_moisture(field: &'static str, value: f64) -> Result<f64> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(FuelError::InvalidMoisture { field, value })
    }
}

impl Moistures {
    fn validated(&self) -> Result<Self> {
        Ok(Self {
            dead_1h: check_moisture("dead_1h", self.dead_1h)?,
            dead_10h: check_moisture("dead_10h", self.dead_10h)?,
            dead_100h: check_moisture("dead_100h", self.dead_100h)?,
            live_herb: check_moisture("live_herb", self.live_herb)?,
            live_woody: check_moisture("live_woody", self.live_woody)?,
        })
    }
}

fn live_sav(model: &FuelModelSi, class: &'static str, sav: Option<f64>, load: f64) -> Result<f64> {
    sav.ok_or_else(|| FuelError::MissingSav {
        code: model.code.clone(),
        class,
        load_kg_m2: load,
    })
}

/// Builds the loaded fuel classes of a model at the given moistures. A
/// dynamic model moves `transfer_fraction(live_herb)` of its live herbaceous
/// load into a dead herbaceous class; a static model keeps all herbaceous
/// load live. Classes with zero load are omitted.
pub fn fuel_classes(model: &FuelModelSi, moistures: &Moistures) -> Result<Vec<FuelClass>> {
    let m = moistures.validated()?;
    let mut classes = Vec::with_capacity(6);
    let mut push = |category, kind, load_kg_m2, sav_per_m, moisture| {
        if load_kg_m2 > 0.0 {
            classes.push(FuelClass {
                category,
                kind,
                load_kg_m2,
                sav_per_m,
                moisture,
            });
        }
    };

    push(
        Category::Dead,
        ClassKind::Dead1h,
        model.load_1h_kg_m2,
        model.sav_dead_1h_per_m,
        m.dead_1h,
    );
    push(
        Category::Dead,
        ClassKind::Dead10h,
        model.load_10h_kg_m2,
        units::per_ft_to_per_m(model::SAV_10H_PER_FT),
        m.dead_10h,
    );
    push(
        Category::Dead,
        ClassKind::Dead100h,
        model.load_100h_kg_m2,
        units::per_ft_to_per_m(model::SAV_100H_PER_FT),
        m.dead_100h,
    );

    let herb = model.load_live_herb_kg_m2;
    if herb > 0.0 {
        let sav = live_sav(model, "live herbaceous", model.sav_live_herb_per_m, herb)?;
        let t = if model.dynamic {
            transfer_fraction(m.live_herb)
        } else {
            0.0
        };
        push(
            Category::Dead,
            ClassKind::DeadHerb,
            t * herb,
            sav,
            m.dead_1h,
        );
        push(
            Category::Live,
            ClassKind::LiveHerb,
            (1.0 - t) * herb,
            sav,
            m.live_herb,
        );
    }

    let woody = model.load_live_woody_kg_m2;
    if woody > 0.0 {
        let sav = live_sav(model, "live woody", model.sav_live_woody_per_m, woody)?;
        push(
            Category::Live,
            ClassKind::LiveWoody,
            woody,
            sav,
            m.live_woody,
        );
    }

    Ok(classes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::by_code;

    const MOIST: Moistures = Moistures {
        dead_1h: 0.08,
        dead_10h: 0.09,
        dead_100h: 0.10,
        live_herb: 0.75,
        live_woody: 0.90,
    };

    fn load_of(classes: &[FuelClass], kind: ClassKind) -> Option<f64> {
        classes
            .iter()
            .find(|c| c.kind == kind)
            .map(|c| c.load_kg_m2)
    }

    #[test]
    fn transfer_fraction_endpoints_and_midpoint() {
        assert_eq!(transfer_fraction(1.20), 0.0);
        assert_eq!(transfer_fraction(1.50), 0.0);
        assert_eq!(transfer_fraction(0.30), 1.0);
        assert_eq!(transfer_fraction(0.10), 1.0);
        let mid = transfer_fraction(0.75);
        assert!((mid - 0.5).abs() < 1e-12, "T(0.75) = {mid}, expected 0.5");
        let t = transfer_fraction(0.90);
        assert!((t - 1.0 / 3.0).abs() < 1e-12, "T(0.90) = {t}, expected 1/3");
    }

    #[test]
    fn gr2_at_75_percent_moves_half_the_herb_and_conserves_load() {
        let si = by_code("GR2").unwrap().to_si();
        let classes = fuel_classes(&si, &MOIST).unwrap();
        // 0.5 t/ac = 0.5 * 907.18474 / 4046.8564224 kg/m2
        let half = 0.112_085_12;
        let dead_herb = load_of(&classes, ClassKind::DeadHerb).unwrap();
        let live_herb = load_of(&classes, ClassKind::LiveHerb).unwrap();
        assert!(
            (dead_herb - half).abs() < 1e-7,
            "dead herb {dead_herb}, expected {half}"
        );
        assert!(
            (live_herb - half).abs() < 1e-7,
            "live herb {live_herb}, expected {half}"
        );
        let total: f64 = classes.iter().map(|c| c.load_kg_m2).sum();
        let expected = si.total_load_kg_m2();
        assert!(
            (total - expected).abs() < 1e-12,
            "total {total}, expected {expected}"
        );

        let dead = classes
            .iter()
            .find(|c| c.kind == ClassKind::DeadHerb)
            .unwrap();
        assert_eq!(dead.category, Category::Dead);
        assert_eq!(dead.moisture, MOIST.dead_1h);
        assert_eq!(dead.sav_per_m, si.sav_live_herb_per_m.unwrap());
        let live = classes
            .iter()
            .find(|c| c.kind == ClassKind::LiveHerb)
            .unwrap();
        assert_eq!(live.category, Category::Live);
        assert_eq!(live.moisture, MOIST.live_herb);
    }

    #[test]
    fn sh2_has_no_herb_so_transfer_changes_nothing() {
        let si = by_code("SH2").unwrap().to_si();
        let cured = Moistures {
            live_herb: 0.10,
            ..MOIST
        };
        let green = Moistures {
            live_herb: 1.50,
            ..MOIST
        };
        let a = fuel_classes(&si, &cured).unwrap();
        let b = fuel_classes(&si, &green).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 4, "{a:?}");
        assert!(load_of(&a, ClassKind::DeadHerb).is_none());
        assert!(load_of(&a, ClassKind::LiveHerb).is_none());
        let total: f64 = a.iter().map(|c| c.load_kg_m2).sum();
        assert!((total - si.total_load_kg_m2()).abs() < 1e-12);
    }

    #[test]
    fn static_model_with_herb_keeps_it_live() {
        // SH4: fuel_model_type N/A, live herb load 0, so take TU2 (N/A, herb 0)
        // is also empty; build a synthetic static model from GR2.
        let mut si = by_code("GR2").unwrap().to_si();
        si.dynamic = false;
        let classes = fuel_classes(
            &si,
            &Moistures {
                live_herb: 0.10,
                ..MOIST
            },
        )
        .unwrap();
        assert!(load_of(&classes, ClassKind::DeadHerb).is_none());
        assert_eq!(
            load_of(&classes, ClassKind::LiveHerb),
            Some(si.load_live_herb_kg_m2)
        );
    }

    #[test]
    fn negative_moisture_is_an_error() {
        let si = by_code("GR1").unwrap().to_si();
        let err = fuel_classes(
            &si,
            &Moistures {
                dead_10h: -0.1,
                ..MOIST
            },
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                FuelError::InvalidMoisture {
                    field: "dead_10h",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn every_model_builds_classes() {
        for model in crate::scott_burgan_40().unwrap() {
            let si = model.to_si();
            let classes = fuel_classes(&si, &MOIST).unwrap();
            let total: f64 = classes.iter().map(|c| c.load_kg_m2).sum();
            assert!(
                (total - si.total_load_kg_m2()).abs() < 1e-12,
                "{}",
                model.code
            );
            assert!(classes.iter().all(|c| c.sav_per_m > 0.0), "{}", model.code);
        }
    }
}
