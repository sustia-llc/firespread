#![forbid(unsafe_code)]
//! Rothermel (1972) surface fire spread model as documented in Andrews (2018)
//! RMRS-GTR-371, with SI quantities at this API. The paper's units live in
//! one private module.

mod ellipse;
mod error;
mod paper;

pub use ellipse::{ellipse_rate, length_to_breadth};
pub use error::{Result, RothermelError};
pub use firespread_fuel::Category;
use firespread_fuel::{FuelClass, FuelModelSi, Moistures, units};

/// One fuel class in SI, with its particle properties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpreadClass {
    pub category: Category,
    /// Oven-dry load, kg/m2.
    pub load_kg_m2: f64,
    /// Surface-area-to-volume ratio, 1/m.
    pub sav_per_m: f64,
    /// Moisture content, fraction of oven-dry weight.
    pub moisture: f64,
    /// Heat content, J/kg.
    pub heat_j_per_kg: f64,
    /// Total mineral content S_T, fraction.
    pub total_mineral_content: f64,
    /// Effective mineral content S_e, fraction.
    pub effective_mineral_content: f64,
    /// Oven-dry particle density, kg/m3.
    pub particle_density_kg_m3: f64,
}

impl SpreadClass {
    /// Builds a class from a fuel-model class, taking heat content from the
    /// model and the Scott and Burgan mineral contents and particle density.
    #[must_use]
    pub fn from_fuel(class: &FuelClass, model: &FuelModelSi) -> Self {
        Self {
            category: class.category,
            load_kg_m2: class.load_kg_m2,
            sav_per_m: class.sav_per_m,
            moisture: class.moisture,
            heat_j_per_kg: model.heat_j_per_kg,
            total_mineral_content: firespread_fuel::TOTAL_MINERAL_CONTENT,
            effective_mineral_content: firespread_fuel::EFFECTIVE_MINERAL_CONTENT,
            particle_density_kg_m3: units::lb_ft3_to_kg_m3(
                firespread_fuel::PARTICLE_DENSITY_LB_PER_FT3,
            ),
        }
    }

    fn to_paper(self) -> paper::Class {
        paper::Class {
            category: self.category,
            load_lb_ft2: units::kg_m2_to_lb_ft2(self.load_kg_m2),
            sav_per_ft: units::per_m_to_per_ft(self.sav_per_m),
            moisture: self.moisture,
            heat_btu_lb: units::j_kg_to_btu_lb(self.heat_j_per_kg),
            s_t: self.total_mineral_content,
            s_e: self.effective_mineral_content,
            rho_p_lb_ft3: units::kg_m3_to_lb_ft3(self.particle_density_kg_m3),
        }
    }
}

/// A heterogeneous fuel bed in SI.
#[derive(Debug, Clone, PartialEq)]
pub struct FuelBed {
    pub classes: Vec<SpreadClass>,
    /// Fuel bed depth, m.
    pub depth_m: f64,
    /// Dead fuel moisture of extinction, fraction.
    pub dead_mx: f64,
}

impl FuelBed {
    /// Builds the bed of a fuel model at the given moistures, applying the
    /// model's dynamic herbaceous transfer.
    pub fn from_model(model: &FuelModelSi, moistures: &Moistures) -> firespread_fuel::Result<Self> {
        let classes = firespread_fuel::fuel_classes(model, moistures)?
            .iter()
            .map(|c| SpreadClass::from_fuel(c, model))
            .collect();
        Ok(Self {
            classes,
            depth_m: model.depth_m,
            dead_mx: model.dead_mx,
        })
    }
}

/// Wind and slope in SI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    /// Midflame wind speed, m/s.
    pub midflame_wind_m_s: f64,
    /// Slope steepness, rise over run.
    pub slope_tan: f64,
}

/// Which wind limit, if any, the spread computation imposes (Andrews 2018
/// §3.2.7 p. 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindLimit {
    /// No limit: R rises with wind without bound.
    #[default]
    None,
    /// Rothermel (1972) eq. 86: the midflame wind fed to φ_w is capped at
    /// 0.9 I_R (ft/min per Btu/ft2-min).
    Rothermel,
    /// The cap is applied to the effective wind speed U_E of §4.1 instead:
    /// when U_E exceeds 0.9 I_R, R uses φ_E recomputed at the capped U_E.
    EffectiveWind,
}

/// Spread model output in SI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpreadOutput {
    /// Rate of spread R, m/s.
    pub ros_m_s: f64,
    /// Zero-wind, zero-slope rate of spread R_0, m/s.
    pub ros_no_wind_no_slope_m_s: f64,
    /// Reaction intensity I_R, kW/m2.
    pub reaction_intensity_kw_m2: f64,
    /// Propagating flux ratio ξ.
    pub propagating_flux_ratio: f64,
    /// Heat sink ρ_b ε Q_ig, kJ/m3.
    pub heat_sink_kj_m3: f64,
    /// Characteristic surface-area-to-volume ratio σ, 1/m.
    pub characteristic_sav_per_m: f64,
    /// Mean packing ratio β.
    pub packing_ratio: f64,
    /// Relative packing ratio β/β_op.
    pub relative_packing_ratio: f64,
    /// Wind factor φ_w from the wind the policy fed to it.
    pub phi_w: f64,
    /// Slope factor φ_s.
    pub phi_s: f64,
    /// Effective factor φ_E that R used.
    pub phi_e: f64,
    /// The policy that was active.
    pub wind_limit: WindLimit,
    /// Whether the policy reduced the wind.
    pub wind_limited: bool,
    /// Midflame wind fed to φ_w after the policy, m/s.
    pub wind_used_m_s: f64,
    /// Effective midflame wind speed U_E (§4.1), m/s.
    pub effective_wind_m_s: f64,
    /// Live fuel moisture of extinction, fraction; `None` without live fuel.
    pub live_mx: Option<f64>,
    /// Residence time t_r, s.
    pub residence_time_s: f64,
    /// Byram fireline intensity I_B, kW/m.
    pub fireline_intensity_kw_m: f64,
    /// Byram flame length F_B, m.
    pub flame_length_m: f64,
}

/// Runs the final (heterogeneous) model of Andrews (2018) Table 6 on a bed
/// under the given wind policy.
pub fn spread(bed: &FuelBed, env: &Environment, limit: WindLimit) -> Result<SpreadOutput> {
    let classes: Vec<paper::Class> = bed.classes.iter().map(|c| c.to_paper()).collect();
    let paper_bed = paper::Bed {
        classes: &classes,
        depth_ft: units::m_to_ft(bed.depth_m),
        dead_mx: bed.dead_mx,
    };
    let paper_env = paper::Env {
        wind_ft_min: units::m_s_to_ft_min(env.midflame_wind_m_s),
        slope_tan: env.slope_tan,
    };
    let out = paper::spread(&paper_bed, &paper_env, limit)?;
    Ok(SpreadOutput {
        ros_m_s: units::ft_min_to_m_s(out.ros_ft_min),
        ros_no_wind_no_slope_m_s: units::ft_min_to_m_s(out.r0_ft_min),
        reaction_intensity_kw_m2: units::btu_ft2_min_to_w_m2(out.reaction_intensity_btu_ft2_min)
            / 1000.0,
        propagating_flux_ratio: out.xi,
        heat_sink_kj_m3: units::btu_ft3_to_j_m3(out.heat_sink_btu_ft3) / 1000.0,
        characteristic_sav_per_m: units::per_ft_to_per_m(out.sigma_per_ft),
        packing_ratio: out.beta,
        relative_packing_ratio: out.beta / out.beta_op,
        phi_w: out.phi_w,
        phi_s: out.phi_s,
        phi_e: out.phi_e,
        wind_limit: limit,
        wind_limited: out.wind_limited,
        wind_used_m_s: units::ft_min_to_m_s(out.wind_used_ft_min),
        effective_wind_m_s: units::ft_min_to_m_s(out.effective_wind_ft_min),
        live_mx: out.live_mx,
        residence_time_s: units::min_to_s(out.residence_time_min),
        fireline_intensity_kw_m: units::btu_ft_s_to_w_m(out.fireline_intensity_btu_ft_s) / 1000.0,
        flame_length_m: units::ft_to_m(out.flame_length_ft),
    })
}

/// Wind and slope with directions in SI. Azimuths are radians clockwise from
/// north in the horizontal plane; any finite value is accepted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalEnvironment {
    /// Midflame wind speed, m/s.
    pub midflame_wind_m_s: f64,
    /// Azimuth the wind blows toward, rad.
    pub wind_toward_azimuth_rad: f64,
    /// Slope steepness, rise over run.
    pub slope_tan: f64,
    /// Upslope azimuth, rad.
    pub upslope_azimuth_rad: f64,
}

/// Directional spread output in SI: the head of the fire and the Finney
/// (1998) RMRS-RP-4 p. 7 ellipse around it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalOutput {
    /// Head rate of spread R = R_0 (1 + φ_E), m/s.
    pub head_ros_m_s: f64,
    /// Azimuth of maximum spread, rad clockwise from north in [0, 2π).
    pub max_spread_azimuth_rad: f64,
    /// Magnitude φ_E of the resultant of φ_w and φ_s that R used.
    pub phi_e: f64,
    /// Effective midflame wind speed U_E of the resultant, m/s.
    pub effective_wind_m_s: f64,
    /// Length-to-breadth ratio LB (eq. [13]).
    pub length_to_breadth: f64,
    /// Head-to-back ratio HB (eq. [14]).
    pub head_to_back: f64,
    /// Backing rate of spread R/HB, m/s.
    pub backing_ros_m_s: f64,
    /// Flanking rate of spread, the semi-minor axis a of eq. [15], m/s.
    pub flanking_ros_m_s: f64,
    /// Eccentricity e = (1 − 1/LB²)^0.5 of the ellipse.
    pub eccentricity: f64,
    /// Zero-wind, zero-slope rate of spread R_0, m/s.
    pub ros_no_wind_no_slope_m_s: f64,
    /// Reaction intensity I_R, kW/m2.
    pub reaction_intensity_kw_m2: f64,
    /// Byram fireline intensity I_B at the head rate, kW/m.
    pub fireline_intensity_kw_m: f64,
    /// Byram flame length F_B at the head rate, m.
    pub flame_length_m: f64,
}

/// Runs the final model of Andrews (2018) Table 6 with wind and slope
/// combined as vectors (Finney 1998 RMRS-RP-4 p. 6 eqs. [11], [12]) under the
/// given wind policy, and fits the RMRS-RP-4 p. 7 ellipse (eqs. [13]-[17])
/// at the resultant's effective wind. The vectors are summed in the
/// horizontal plane, not projected onto the slope plane as RP-4 eq. [6] does.
/// When the resultant is zero the azimuth is the upslope azimuth if the slope
/// is positive, else the wind azimuth if the wind is positive, else 0.
pub fn spread_directional(
    bed: &FuelBed,
    env: &DirectionalEnvironment,
    limit: WindLimit,
) -> Result<DirectionalOutput> {
    let classes: Vec<paper::Class> = bed.classes.iter().map(|c| c.to_paper()).collect();
    let paper_bed = paper::Bed {
        classes: &classes,
        depth_ft: units::m_to_ft(bed.depth_m),
        dead_mx: bed.dead_mx,
    };
    let out = paper::spread_directional(&paper_bed, &dir_env_to_paper(env), limit)?;
    Ok(directional_output_to_si(&out))
}

/// [`spread_directional`] for one bed and wind policy with every term that
/// depends on neither wind nor slope computed once, in [`DirectionalModel::new`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalModel {
    inner: paper::DirModel,
}

impl DirectionalModel {
    /// Validates `bed` and computes its wind- and slope-independent terms
    /// under `limit`.
    ///
    /// # Errors
    /// The bed errors of [`spread_directional`]: an invalid depth, moisture of
    /// extinction or class, no fuel, or a zero heat sink.
    pub fn new(bed: &FuelBed, limit: WindLimit) -> Result<Self> {
        let classes: Vec<paper::Class> = bed.classes.iter().map(|c| c.to_paper()).collect();
        let paper_bed = paper::Bed {
            classes: &classes,
            depth_ft: units::m_to_ft(bed.depth_m),
            dead_mx: bed.dead_mx,
        };
        Ok(Self {
            inner: paper::DirModel::new(&paper_bed, limit)?,
        })
    }

    /// Returns `spread_directional(bed, env, limit)` bit for bit, for the
    /// `bed` and `limit` this model was built from.
    ///
    /// # Errors
    /// [`RothermelError::InvalidInput`] when the wind speed or slope is
    /// negative or not finite, or an azimuth is not finite.
    pub fn eval(&self, env: &DirectionalEnvironment) -> Result<DirectionalOutput> {
        let out = self.inner.eval(&dir_env_to_paper(env))?;
        Ok(directional_output_to_si(&out))
    }
}

fn dir_env_to_paper(env: &DirectionalEnvironment) -> paper::DirEnv {
    paper::DirEnv {
        wind_ft_min: units::m_s_to_ft_min(env.midflame_wind_m_s),
        wind_toward_azimuth_rad: env.wind_toward_azimuth_rad,
        slope_tan: env.slope_tan,
        upslope_azimuth_rad: env.upslope_azimuth_rad,
    }
}

fn directional_output_to_si(out: &paper::DirOutput) -> DirectionalOutput {
    let head = units::ft_min_to_m_s(out.ros_ft_min);
    let u_e = units::ft_min_to_m_s(out.effective_wind_ft_min);
    let lb = ellipse::length_to_breadth(u_e);
    let hb = ellipse::head_to_back(lb);
    DirectionalOutput {
        head_ros_m_s: head,
        max_spread_azimuth_rad: out.max_spread_azimuth_rad,
        phi_e: out.phi_e,
        effective_wind_m_s: u_e,
        length_to_breadth: lb,
        head_to_back: hb,
        backing_ros_m_s: head / hb,
        flanking_ros_m_s: ellipse::semi_minor(head, hb, lb),
        eccentricity: ellipse::eccentricity(lb),
        ros_no_wind_no_slope_m_s: units::ft_min_to_m_s(out.r0_ft_min),
        reaction_intensity_kw_m2: units::btu_ft2_min_to_w_m2(out.reaction_intensity_btu_ft2_min)
            / 1000.0,
        fireline_intensity_kw_m: units::btu_ft_s_to_w_m(out.fireline_intensity_btu_ft_s) / 1000.0,
        flame_length_m: units::ft_to_m(out.flame_length_ft),
    }
}

/// Basic single-class model (Andrews 2018 §3.1) with the Rothermel (1972)
/// eq. 39 exponent A and no wind limit, using the Scott and Burgan mineral
/// contents and particle density. Returns the rate of spread in m/s. Serves
/// the FireBench `Rothermel_SFIRE` oracle test.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the oracle's positional signature"
)]
pub fn single_class(
    sav_per_m: f64,
    load_kg_m2: f64,
    depth_m: f64,
    mx: f64,
    heat_j_per_kg: f64,
    moisture: f64,
    wind_m_s: f64,
    slope_tan: f64,
) -> Result<f64> {
    let ros_ft_min = paper::single_class(
        units::per_m_to_per_ft(sav_per_m),
        units::kg_m2_to_lb_ft2(load_kg_m2),
        units::m_to_ft(depth_m),
        mx,
        units::j_kg_to_btu_lb(heat_j_per_kg),
        moisture,
        units::m_s_to_ft_min(wind_m_s),
        slope_tan,
        firespread_fuel::TOTAL_MINERAL_CONTENT,
        firespread_fuel::EFFECTIVE_MINERAL_CONTENT,
        firespread_fuel::PARTICLE_DENSITY_LB_PER_FT3,
    )?;
    Ok(units::ft_min_to_m_s(ros_ft_min))
}
