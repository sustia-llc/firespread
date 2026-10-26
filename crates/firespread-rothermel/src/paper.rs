//! Rothermel (1972) surface fire spread model in the paper's units (ft, lb,
//! Btu, min), as tabulated in Andrews (2018) RMRS-GTR-371. This is the only
//! module that holds US customary quantities; the crate root converts to and
//! from SI. Each coefficient's doc comment names the printed page of
//! RMRS-GTR-371 and the Rothermel (1972) equation number Table 6 cites.

use firespread_fuel::Category;

use crate::{Result, RothermelError, WindLimit};

/// One fuel class in the paper's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Class {
    pub category: Category,
    /// Oven-dry load (w0)_ij, lb/ft2.
    pub load_lb_ft2: f64,
    /// Surface-area-to-volume ratio σ_ij, 1/ft.
    pub sav_per_ft: f64,
    /// Moisture content (M_f)_ij, fraction.
    pub moisture: f64,
    /// Heat content h_ij, Btu/lb.
    pub heat_btu_lb: f64,
    /// Total mineral content (S_T)_ij, fraction.
    pub s_t: f64,
    /// Effective mineral content (S_e)_ij, fraction.
    pub s_e: f64,
    /// Particle density (ρ_p)_ij, lb/ft3.
    pub rho_p_lb_ft3: f64,
}

/// Fuel bed in the paper's units.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bed<'a> {
    pub classes: &'a [Class],
    /// Fuel bed depth δ, ft.
    pub depth_ft: f64,
    /// Dead fuel moisture of extinction (M_x)_1, fraction.
    pub dead_mx: f64,
}

/// Environment in the paper's units.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Env {
    /// Midflame wind speed U, ft/min.
    pub wind_ft_min: f64,
    /// Slope steepness tan φ, fraction.
    pub slope_tan: f64,
}

/// Model output in the paper's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Output {
    /// Rate of spread R, ft/min.
    pub ros_ft_min: f64,
    /// Zero-wind, zero-slope rate of spread R_0, ft/min.
    pub r0_ft_min: f64,
    /// Reaction intensity I_R, Btu/ft2-min.
    pub reaction_intensity_btu_ft2_min: f64,
    /// Propagating flux ratio ξ.
    pub xi: f64,
    /// Heat sink ρ_b ε Q_ig, Btu/ft3.
    pub heat_sink_btu_ft3: f64,
    /// Characteristic surface-area-to-volume ratio σ, 1/ft.
    pub sigma_per_ft: f64,
    /// Mean packing ratio β.
    pub beta: f64,
    /// Optimum packing ratio β_op.
    pub beta_op: f64,
    /// Wind factor φ_w from the wind the policy fed to it.
    pub phi_w: f64,
    /// Slope factor φ_s.
    pub phi_s: f64,
    /// Effective factor φ_E that R used: φ_w + φ_s, or the limited value
    /// under `WindLimit::EffectiveWind`.
    pub phi_e: f64,
    /// Midflame wind fed to φ_w after the policy, ft/min.
    pub wind_used_ft_min: f64,
    /// Effective midflame wind speed U_E, ft/min.
    pub effective_wind_ft_min: f64,
    /// Whether the policy reduced the wind.
    pub wind_limited: bool,
    /// Live fuel moisture of extinction (M_x)_2, fraction; `None` without live fuel.
    pub live_mx: Option<f64>,
    /// Residence time t_r, min.
    pub residence_time_min: f64,
    /// Byram fireline intensity I_B, Btu/ft/s.
    pub fireline_intensity_btu_ft_s: f64,
    /// Byram flame length F_B, ft.
    pub flame_length_ft: f64,
}

/// Optimum packing ratio β_op = 3.348 σ^−0.8189 (p. 12 §3.1.2; Table 6b p. 18, eq. 69).
pub(crate) fn beta_op(sigma: f64) -> f64 {
    3.348 * sigma.powf(-0.8189)
}

/// Maximum reaction velocity Γ'_max = σ^1.5 (495 + 0.0594 σ^1.5)^−1, 1/min
/// (p. 14 §3.1.3; Table 6c p. 19, eq. 68).
pub(crate) fn gamma_max(sigma: f64) -> f64 {
    let s15 = sigma.powf(1.5);
    s15 / (495.0 + 0.0594 * s15)
}

/// Albini (1976a) exponent A = 133 σ^−0.7913 (p. 14; Table 6c p. 19,
/// Albini p. 15 replacing eq. 39).
pub(crate) fn a_albini(sigma: f64) -> f64 {
    133.0 * sigma.powf(-0.7913)
}

/// Rothermel (1972) eq. 39 exponent A = (4.774 σ^0.1 − 7.27)^−1. Not printed
/// in RMRS-GTR-371; given by the phase brief for the SFIRE oracle.
pub(crate) fn a_rothermel(sigma: f64) -> f64 {
    1.0 / (4.774 * sigma.powf(0.1) - 7.27)
}

/// Optimum reaction velocity Γ' = Γ'_max (β/β_op)^A exp[A(1 − β/β_op)], 1/min
/// (p. 13 §3.1.3; Table 6c p. 19, eq. 67).
pub(crate) fn gamma_prime(gamma_max: f64, ratio: f64, a: f64) -> f64 {
    gamma_max * ratio.powf(a) * (a * (1.0 - ratio)).exp()
}

/// Moisture damping η_M = 1 − 2.59 r_M + 5.11 r_M² − 3.52 r_M³ with
/// r_M = M_f/M_x (p. 14; Table 6b p. 18, eq. 64, 65; Rothermel 1972 eq. 29).
/// Returns exactly 0.0 for r_M ≥ 1, where the polynomial is 0 at r_M = 1.
pub(crate) fn eta_m(mf: f64, mx: f64) -> f64 {
    let r = mf / mx;
    if r >= 1.0 {
        return 0.0;
    }
    1.0 - 2.59 * r + 5.11 * r * r - 3.52 * r * r * r
}

/// Mineral damping η_s = 0.174 S_e^−0.19 (max 1.0) (p. 14; Table 6b p. 18, eq. 62).
pub(crate) fn eta_s(se: f64) -> f64 {
    (0.174 * se.powf(-0.19)).min(1.0)
}

/// Propagating flux ratio ξ = (192 + 0.2595 σ)^−1 exp[(0.792 + 0.681 σ^0.5)(β + 0.1)]
/// (p. 13 §3.1.3; Table 6c p. 19, eq. 76).
pub(crate) fn xi(sigma: f64, beta: f64) -> f64 {
    ((0.792 + 0.681 * sigma.sqrt()) * (beta + 0.1)).exp() / (192.0 + 0.2595 * sigma)
}

/// Heat of preignition Q_ig = 250 + 1,116 M_f, Btu/lb (p. 15 §3.1.4; Table 6c p. 19, eq. 78).
pub(crate) fn q_ig(mf: f64) -> f64 {
    250.0 + 1116.0 * mf
}

/// Effective heating number ε = exp(−138/σ) (p. 15 §3.1.4; inside the heat
/// sink of Table 6c p. 19, eq. 77).
pub(crate) fn epsilon(sigma: f64) -> f64 {
    (-138.0 / sigma).exp()
}

/// Slope factor φ_s = 5.275 β^−0.3 (tan φ)² (p. 14; Table 6b p. 18, eq. 80).
pub(crate) fn phi_s(beta: f64, slope_tan: f64) -> f64 {
    5.275 * beta.powf(-0.3) * slope_tan * slope_tan
}

/// Wind coefficient C = 7.47 exp(−0.133 σ^0.55) (p. 15; Table 6b p. 18, eq. 82).
pub(crate) fn wind_c(sigma: f64) -> f64 {
    7.47 * (-0.133 * sigma.powf(0.55)).exp()
}

/// Wind exponent B = 0.02526 σ^0.54 (p. 15; Table 6b p. 18, eq. 83).
pub(crate) fn wind_b(sigma: f64) -> f64 {
    0.02526 * sigma.powf(0.54)
}

/// Wind exponent E = 0.715 exp(−3.59 × 10^−4 σ) (p. 15; Table 6b p. 18, eq. 84).
pub(crate) fn wind_e(sigma: f64) -> f64 {
    0.715 * (-3.59e-4 * sigma).exp()
}

/// Wind factor φ_w = C U^B (β/β_op)^−E, U in ft/min (p. 14; Table 6b p. 18, eq. 79, 81).
pub(crate) fn phi_w(c: f64, b: f64, e: f64, wind_ft_min: f64, ratio: f64) -> f64 {
    c * wind_ft_min.powf(b) * ratio.powf(-e)
}

/// Wind limit U = 0.9 I_R, ft/min with I_R in Btu/ft2-min (p. 25 §3.2.7;
/// Table 6b p. 18, eq. 86).
pub(crate) fn wind_limit_ft_min(ir: f64) -> f64 {
    0.9 * ir
}

/// Effective midflame wind speed U_E = [φ_E (β/β_op)^E / C]^(1/B), ft/min
/// (p. 27 §4.1; Table 7 p. 26, Albini 1976b).
pub(crate) fn effective_wind(phi_e: f64, c: f64, b: f64, e: f64, ratio: f64) -> f64 {
    (phi_e * ratio.powf(e) / c).powf(1.0 / b)
}

/// Residence time t_r = 384/σ, min (p. 27 §4.2; Table 7 p. 26, Anderson 1969).
pub(crate) fn residence_time_min(sigma: f64) -> f64 {
    384.0 / sigma
}

/// Byram fireline intensity I_B = I_R t_r R / 60, Btu/ft/s (Table 7 p. 26,
/// Albini 1976a, Byram 1959).
pub(crate) fn fireline_intensity(ir: f64, tr_min: f64, ros_ft_min: f64) -> f64 {
    ir * tr_min * ros_ft_min / 60.0
}

/// Byram flame length F_B = 0.45 I_B^0.46, ft (Table 7 p. 26, Byram 1959).
pub(crate) fn flame_length(ib: f64) -> f64 {
    0.45 * ib.powf(0.46)
}

/// Size subclass of σ for the net-load weighting g_ij (Table 6a p. 17, Albini
/// p. 15): σ ≥ 1,200; 1,200 > σ ≥ 192; 192 > σ ≥ 96; 96 > σ ≥ 48;
/// 48 > σ ≥ 16; σ < 16 gets g_ij = 0.
fn subclass(sigma: f64) -> Option<u8> {
    if sigma >= 1200.0 {
        Some(0)
    } else if sigma >= 192.0 {
        Some(1)
    } else if sigma >= 96.0 {
        Some(2)
    } else if sigma >= 48.0 {
        Some(3)
    } else if sigma >= 16.0 {
        Some(4)
    } else {
        None
    }
}

/// Live fuel moisture of extinction (M_x)_2 = 2.9 W (1 − M_f,dead/(M_x)_1) −
/// 0.226 (min (M_x)_1), with dead-to-live load ratio
/// W = Σ_j (w0)_1j exp(−138/σ_1j) / Σ_j (w0)_2j exp(−500/σ_2j) and fine dead
/// moisture M_f,dead = Σ_j (M_f)_1j (w0)_1j exp(−138/σ_1j) / Σ_j (w0)_1j
/// exp(−138/σ_1j) (Table 6a p. 17, Albini p. 16 replacing eq. 88).
#[cfg(test)]
fn live_mx(dead: &[&Class], live: &[&Class], mx1: f64) -> f64 {
    LiveMxTerms::new(dead, live).map_or(mx1, |t| t.eval(|j| dead[j].moisture, mx1))
}

/// The moisture-independent terms of [`live_mx`]: per dead class its load
/// (w0)_1j and exp(−138/σ_1j), the dead fine load and 2.9 W. `None` when the
/// dead or live fine load is not positive, where (M_x)_2 = (M_x)_1.
#[derive(Debug, Clone, PartialEq)]
struct LiveMxTerms {
    dead_load: Vec<f64>,
    dead_exp: Vec<f64>,
    dead_fine: f64,
    two_nine_w: f64,
}

impl LiveMxTerms {
    fn new(dead: &[&Class], live: &[&Class]) -> Option<Self> {
        let dead_exp: Vec<f64> = dead.iter().map(|c| (-138.0 / c.sav_per_ft).exp()).collect();
        let dead_fine: f64 = dead
            .iter()
            .zip(&dead_exp)
            .map(|(c, e)| c.load_lb_ft2 * e)
            .sum();
        let live_fine: f64 = live
            .iter()
            .map(|c| c.load_lb_ft2 * (-500.0 / c.sav_per_ft).exp())
            .sum();
        if dead_fine <= 0.0 || live_fine <= 0.0 {
            return None;
        }
        let w = dead_fine / live_fine;
        Some(Self {
            dead_load: dead.iter().map(|c| c.load_lb_ft2).collect(),
            dead_exp,
            dead_fine,
            two_nine_w: 2.9 * w,
        })
    }

    /// (M_x)_2 with dead class `j` (in dead class order) at moisture
    /// `moisture(j)`.
    fn eval(&self, moisture: impl Fn(usize) -> f64, mx1: f64) -> f64 {
        let mf_dead = self
            .dead_load
            .iter()
            .zip(&self.dead_exp)
            .enumerate()
            .map(|(j, (load, e))| moisture(j) * load * e)
            .sum::<f64>()
            / self.dead_fine;
        (self.two_nine_w * (1.0 - mf_dead / mx1) - 0.226).max(mx1)
    }
}

fn check(field: &'static str, value: f64, positive: bool) -> Result<f64> {
    let ok = value.is_finite() && if positive { value > 0.0 } else { value >= 0.0 };
    if ok {
        Ok(value)
    } else {
        Err(RothermelError::InvalidInput {
            field,
            bound: if positive { "positive" } else { "non-negative" },
            value,
        })
    }
}

fn check_class(index: usize, field: &'static str, value: f64, positive: bool) -> Result<f64> {
    let ok = value.is_finite() && if positive { value > 0.0 } else { value >= 0.0 };
    if ok {
        Ok(value)
    } else {
        Err(RothermelError::InvalidClass {
            index,
            field,
            bound: if positive { "positive" } else { "non-negative" },
            value,
        })
    }
}

fn validate_class(index: usize, c: &Class) -> Result<()> {
    check_class(index, "load", c.load_lb_ft2, false)?;
    check_class(index, "sav", c.sav_per_ft, true)?;
    check_class(index, "moisture", c.moisture, false)?;
    check_class(index, "heat", c.heat_btu_lb, true)?;
    check_class(index, "total mineral content", c.s_t, false)?;
    check_class(index, "effective mineral content", c.s_e, true)?;
    check_class(index, "particle density", c.rho_p_lb_ft3, true)?;
    Ok(())
}

/// Per-category characteristic values (Table 6a-6b).
struct CategoryValues {
    /// f_i = A_i/A_T (eq. 57).
    f_i: f64,
    /// f_ij = A_ij/A_i (eq. 56), in class order.
    f_ij: Vec<f64>,
    /// (w_n)_i = Σ_j g_ij (w_n)_ij (Albini p. 15 replacing eq. 59).
    wn: f64,
    /// h_i = Σ_j f_ij h_ij (eq. 61).
    h: f64,
    /// (η_s)_i from (S_e)_i = Σ_j f_ij (S_e)_ij (eq. 62, 63).
    eta_s: f64,
    /// σ_i = Σ_j f_ij σ_ij (eq. 72).
    sigma: f64,
}

fn category_values(classes: &[&Class], a_i: f64, a_t: f64) -> CategoryValues {
    let f_ij: Vec<f64> = classes
        .iter()
        .map(|c| c.sav_per_ft * c.load_lb_ft2 / c.rho_p_lb_ft3 / a_i)
        .collect();
    // g_ij = Σ over the subclass j belongs to of f_ik (Table 6a p. 17).
    let g_ij: Vec<f64> = classes
        .iter()
        .map(|c| match subclass(c.sav_per_ft) {
            None => 0.0,
            Some(s) => classes
                .iter()
                .zip(&f_ij)
                .filter(|(k, _)| subclass(k.sav_per_ft) == Some(s))
                .map(|(_, f)| f)
                .sum(),
        })
        .collect();
    let weighted = |value: &dyn Fn(&Class) -> f64| -> f64 {
        classes.iter().zip(&f_ij).map(|(c, f)| f * value(c)).sum()
    };
    // (w_n)_ij = (w0)_ij (1 − (S_T)_ij) (Albini p. 14 replacing eq. 60).
    let wn = classes
        .iter()
        .zip(&g_ij)
        .map(|(c, g)| g * c.load_lb_ft2 * (1.0 - c.s_t))
        .sum();
    CategoryValues {
        f_i: a_i / a_t,
        wn,
        h: weighted(&|c| c.heat_btu_lb),
        eta_s: eta_s(weighted(&|c| c.s_e)),
        sigma: weighted(&|c| c.sav_per_ft),
        f_ij,
    }
}

/// Wind-independent quantities of the final model plus the validated wind
/// speed and slope.
struct Base {
    wind: f64,
    r0: f64,
    ir: f64,
    xi: f64,
    heat_sink: f64,
    sigma: f64,
    beta: f64,
    beta_op: f64,
    ratio: f64,
    phi_s: f64,
    c: f64,
    b: f64,
    e: f64,
    u_max: f64,
    live_mx: Option<f64>,
    tr: f64,
}

/// Final (heterogeneous) model, Table 6a-6c (pp. 17-19), with the wind policy
/// of §3.2.7 (p. 25) and the related models of Table 7 (p. 26).
pub(crate) fn spread(bed: &Bed<'_>, env: &Env, limit: WindLimit) -> Result<Output> {
    let Base {
        wind,
        r0,
        ir,
        xi,
        heat_sink,
        sigma,
        beta,
        beta_op,
        ratio,
        phi_s,
        c,
        b,
        e,
        u_max,
        live_mx: live_mx_value,
        tr,
    } = base(bed, env)?;
    let (wind_used, phi_w_value, phi_e, u_e, limited) = match limit {
        WindLimit::None => {
            let pw = phi_w(c, b, e, wind, ratio);
            let pe = pw + phi_s;
            (wind, pw, pe, effective_wind(pe, c, b, e, ratio), false)
        }
        WindLimit::Rothermel => {
            let (u, limited) = if wind > u_max {
                (u_max, true)
            } else {
                (wind, false)
            };
            let pw = phi_w(c, b, e, u, ratio);
            let pe = pw + phi_s;
            (u, pw, pe, effective_wind(pe, c, b, e, ratio), limited)
        }
        WindLimit::EffectiveWind => {
            let pw = phi_w(c, b, e, wind, ratio);
            let pe = pw + phi_s;
            let u_e = effective_wind(pe, c, b, e, ratio);
            if u_e > u_max {
                (wind, pw, phi_w(c, b, e, u_max, ratio), u_max, true)
            } else {
                (wind, pw, pe, u_e, false)
            }
        }
    };
    // R = R_0 (1 + φ_w + φ_s) (eq. 75).
    let ros = r0 * (1.0 + phi_e);

    let ib = fireline_intensity(ir, tr, ros);
    Ok(Output {
        ros_ft_min: ros,
        r0_ft_min: r0,
        reaction_intensity_btu_ft2_min: ir,
        xi,
        heat_sink_btu_ft3: heat_sink,
        sigma_per_ft: sigma,
        beta,
        beta_op,
        phi_w: phi_w_value,
        phi_s,
        phi_e,
        wind_used_ft_min: wind_used,
        effective_wind_ft_min: u_e,
        wind_limited: limited,
        live_mx: live_mx_value,
        residence_time_min: tr,
        fireline_intensity_btu_ft_s: ib,
        flame_length_ft: flame_length(ib),
    })
}

/// Wind and slope with directions, in the paper's units. Azimuths are
/// radians clockwise from north in the horizontal plane.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DirEnv {
    /// Midflame wind speed U, ft/min.
    pub wind_ft_min: f64,
    /// Azimuth the wind blows toward, rad.
    pub wind_toward_azimuth_rad: f64,
    /// Slope steepness tan φ, fraction.
    pub slope_tan: f64,
    /// Upslope azimuth, rad.
    pub upslope_azimuth_rad: f64,
}

/// Directional model output in the paper's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DirOutput {
    /// Head rate of spread R_0 (1 + φ_E), ft/min.
    pub ros_ft_min: f64,
    /// Zero-wind, zero-slope rate of spread R_0, ft/min.
    pub r0_ft_min: f64,
    /// Reaction intensity I_R, Btu/ft2-min.
    pub reaction_intensity_btu_ft2_min: f64,
    /// Azimuth of maximum spread, rad in [0, 2π).
    pub max_spread_azimuth_rad: f64,
    /// Magnitude φ_E of the resultant of φ_w and φ_s, or the limited value.
    pub phi_e: f64,
    /// Effective midflame wind speed U_E, ft/min.
    pub effective_wind_ft_min: f64,
    /// Byram fireline intensity I_B at the head rate, Btu/ft/s.
    pub fireline_intensity_btu_ft_s: f64,
    /// Byram flame length F_B at the head rate, ft.
    pub flame_length_ft: f64,
}

/// Maps an angle in radians onto [0, 2π).
fn normalize_azimuth(rad: f64) -> f64 {
    let r = rad.rem_euclid(core::f64::consts::TAU);
    if r >= core::f64::consts::TAU { 0.0 } else { r }
}

fn check_finite(field: &'static str, value: f64) -> Result<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(RothermelError::InvalidInput {
            field,
            bound: "real",
            value,
        })
    }
}

/// Final model with wind and slope as vectors: Finney (1998) RMRS-RP-4 p. 6
/// eqs. [11], [12] take the direction of maximum spread as the angle of the
/// resultant of φ_w (along the azimuth the wind blows toward) and φ_s
/// (upslope), and R = R_0 (1 + |φ_w ŵ + φ_s ŝ|); p. 7 takes U_E as the wind
/// that alone gives that resultant (eq. [12] solved for U, here the inverse of
/// Andrews 2018 §4.1 p. 27). The wind policy is that of `spread`:
/// `Rothermel` caps U before φ_w; `EffectiveWind` caps U_E of the resultant
/// at 0.9 I_R and recomputes φ_E there, keeping the resultant's direction.
/// When the resultant is zero the azimuth is the upslope azimuth if
/// tan φ > 0, else the wind azimuth if U > 0, else 0.
///
/// Restriction: the vectors are summed in the horizontal plane with map
/// azimuths; RP-4 eq. [6] instead projects the angles onto the local surface
/// plane.
pub(crate) fn spread_directional(
    bed: &Bed<'_>,
    env: &DirEnv,
    limit: WindLimit,
) -> Result<DirOutput> {
    let base = base(
        bed,
        &Env {
            wind_ft_min: env.wind_ft_min,
            slope_tan: env.slope_tan,
        },
    )?;
    let wind_az = check_finite("wind azimuth", env.wind_toward_azimuth_rad)?;
    let upslope_az = check_finite("upslope azimuth", env.upslope_azimuth_rad)?;
    let Base {
        wind,
        r0,
        ir,
        ratio,
        phi_s,
        c,
        b,
        e,
        u_max,
        tr,
        ..
    } = base;

    let wind_used = match limit {
        WindLimit::Rothermel => wind.min(u_max),
        WindLimit::None | WindLimit::EffectiveWind => wind,
    };
    let pw = phi_w(c, b, e, wind_used, ratio);
    // Components (east, north) of φ_w ŵ + φ_s ŝ with ŵ = (sin θ_w, cos θ_w).
    let east = pw * wind_az.sin() + phi_s * upslope_az.sin();
    let north = pw * wind_az.cos() + phi_s * upslope_az.cos();
    let resultant = east.hypot(north);
    let azimuth = if resultant > 0.0 {
        normalize_azimuth(east.atan2(north))
    } else if env.slope_tan > 0.0 {
        normalize_azimuth(upslope_az)
    } else if wind > 0.0 {
        normalize_azimuth(wind_az)
    } else {
        0.0
    };
    let u_e = effective_wind(resultant, c, b, e, ratio);
    let (phi_e, u_e) = match limit {
        WindLimit::EffectiveWind if u_e > u_max => (phi_w(c, b, e, u_max, ratio), u_max),
        _ => (resultant, u_e),
    };
    // R = R_0 (1 + φ_E) (Rothermel 1972 eq. 75 with the vector φ_E).
    let ros = r0 * (1.0 + phi_e);
    let ib = fireline_intensity(ir, tr, ros);
    Ok(DirOutput {
        ros_ft_min: ros,
        r0_ft_min: r0,
        reaction_intensity_btu_ft2_min: ir,
        max_spread_azimuth_rad: azimuth,
        phi_e,
        effective_wind_ft_min: u_e,
        fireline_intensity_btu_ft_s: ib,
        flame_length_ft: flame_length(ib),
    })
}

/// The terms of `spread_directional` that depend on the bed and the wind
/// policy alone, with each product and power formed as `spread_directional`
/// forms it, so that `eval` returns its output bit for bit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DirModel {
    limit: WindLimit,
    /// R_0, ft/min.
    r0: f64,
    /// I_R, Btu/ft2-min.
    ir: f64,
    /// Wind coefficient C.
    c: f64,
    /// Wind exponent B.
    b: f64,
    /// 1/B.
    inv_b: f64,
    /// (β/β_op)^−E.
    ratio_neg_e: f64,
    /// (β/β_op)^E.
    ratio_e: f64,
    /// 5.275 β^−0.3, the slope factor at tan φ = 1.
    phi_s_coef: f64,
    /// Wind limit 0.9 I_R, ft/min.
    u_max: f64,
    /// φ_w at the wind limit.
    phi_w_at_u_max: f64,
    /// I_R t_r, Btu/ft2.
    ir_tr: f64,
}

impl DirModel {
    /// Validates the bed and computes its wind- and slope-independent terms.
    pub(crate) fn new(bed: &Bed<'_>, limit: WindLimit) -> Result<Self> {
        DirTerms::new(bed, limit)?.at(|index| bed.classes[index].moisture)
    }

    /// `spread_directional` of the bed this model was built from, at `env`.
    /// Validates `env` as `spread_directional` does.
    pub(crate) fn eval(&self, env: &DirEnv) -> Result<DirOutput> {
        let wind = check("wind", env.wind_ft_min, false)?;
        let slope_tan = check("slope", env.slope_tan, false)?;
        let wind_az = check_finite("wind azimuth", env.wind_toward_azimuth_rad)?;
        let upslope_az = check_finite("upslope azimuth", env.upslope_azimuth_rad)?;
        let phi_s = self.phi_s_coef * slope_tan * slope_tan;
        let wind_used = match self.limit {
            WindLimit::Rothermel => wind.min(self.u_max),
            WindLimit::None | WindLimit::EffectiveWind => wind,
        };
        let pw = self.c * wind_used.powf(self.b) * self.ratio_neg_e;
        let east = pw * wind_az.sin() + phi_s * upslope_az.sin();
        let north = pw * wind_az.cos() + phi_s * upslope_az.cos();
        let resultant = east.hypot(north);
        let azimuth = if resultant > 0.0 {
            normalize_azimuth(east.atan2(north))
        } else if slope_tan > 0.0 {
            normalize_azimuth(upslope_az)
        } else if wind > 0.0 {
            normalize_azimuth(wind_az)
        } else {
            0.0
        };
        let u_e = (resultant * self.ratio_e / self.c).powf(self.inv_b);
        let (phi_e, u_e) = match self.limit {
            WindLimit::EffectiveWind if u_e > self.u_max => (self.phi_w_at_u_max, self.u_max),
            _ => (resultant, u_e),
        };
        let ros = self.r0 * (1.0 + phi_e);
        let ib = self.ir_tr * ros / 60.0;
        Ok(DirOutput {
            ros_ft_min: ros,
            r0_ft_min: self.r0,
            reaction_intensity_btu_ft2_min: self.ir,
            max_spread_azimuth_rad: azimuth,
            phi_e,
            effective_wind_ft_min: u_e,
            fireline_intensity_btu_ft_s: ib,
            flame_length_ft: flame_length(ib),
        })
    }
}

/// Validates the bed and environment and computes the wind-independent part
/// of the final model, Table 6a-6c (pp. 17-19).
fn base(bed: &Bed<'_>, env: &Env) -> Result<Base> {
    check("depth", bed.depth_ft, true)?;
    check("dead moisture of extinction", bed.dead_mx, true)?;
    let wind = check("wind", env.wind_ft_min, false)?;
    let slope_tan = check("slope", env.slope_tan, false)?;
    let terms = BedTerms::new(bed)?;
    let m = terms.eval(|index| bed.classes[index].moisture)?;
    Ok(Base {
        wind,
        r0: m.r0,
        ir: m.ir,
        xi: terms.xi,
        heat_sink: m.heat_sink,
        sigma: terms.sigma,
        beta: terms.beta,
        beta_op: terms.beta_op,
        ratio: terms.ratio,
        phi_s: phi_s(terms.beta, slope_tan),
        c: terms.c,
        b: terms.b,
        e: terms.e,
        u_max: wind_limit_ft_min(m.ir),
        live_mx: m.live_mx,
        tr: terms.tr,
    })
}

/// One category's moisture-independent terms of the final model.
#[derive(Debug, Clone, PartialEq)]
struct CategoryTerms {
    /// Bed index of each class of the category, in bed order.
    classes: Vec<usize>,
    /// f_i = A_i/A_T (eq. 57).
    f_i: f64,
    /// f_ij (eq. 56), in class order.
    f_ij: Vec<f64>,
    /// f_ij exp(−138/σ_ij), in class order.
    f_eps: Vec<f64>,
    /// (w_n)_i h_i.
    wn_h: f64,
    /// (η_s)_i.
    eta_s: f64,
}

/// The terms of [`BedTerms::eval`] that depend on the class moistures.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MoistureTerms {
    r0: f64,
    ir: f64,
    heat_sink: f64,
    live_mx: Option<f64>,
}

/// The terms of the final model (Table 6a-6c, pp. 17-19) that depend on
/// neither wind, slope nor class moistures, for a validated bed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BedTerms {
    n_classes: usize,
    mx1: f64,
    dead: Option<CategoryTerms>,
    live: Option<CategoryTerms>,
    /// Bed index of each dead class, in bed order.
    dead_classes: Vec<usize>,
    live_mx: Option<LiveMxTerms>,
    sigma: f64,
    beta: f64,
    beta_op: f64,
    ratio: f64,
    /// Γ' (eq. 67).
    gamma: f64,
    xi: f64,
    rho_b: f64,
    c: f64,
    b: f64,
    e: f64,
    tr: f64,
}

impl BedTerms {
    /// Validates the bed (depth, (M_x)_1 and every class, as `spread` does)
    /// and computes its moisture-independent terms.
    pub(crate) fn new(bed: &Bed<'_>) -> Result<Self> {
        let depth = check("depth", bed.depth_ft, true)?;
        let mx1 = check("dead moisture of extinction", bed.dead_mx, true)?;
        for (index, c) in bed.classes.iter().enumerate() {
            validate_class(index, c)?;
        }
        let of = |category: Category| -> Vec<usize> {
            (0..bed.classes.len())
                .filter(|&i| bed.classes[i].category == category)
                .collect()
        };
        let dead_classes = of(Category::Dead);
        let live_classes = of(Category::Live);
        let dead: Vec<&Class> = dead_classes.iter().map(|&i| &bed.classes[i]).collect();
        let live: Vec<&Class> = live_classes.iter().map(|&i| &bed.classes[i]).collect();
        // A_ij = σ_ij (w0)_ij / (ρ_p)_ij (eq. 53); A_i = Σ_j A_ij (eq. 54); A_T = Σ_i A_i (eq. 55).
        let area = |cs: &[&Class]| -> f64 {
            cs.iter()
                .map(|c| c.sav_per_ft * c.load_lb_ft2 / c.rho_p_lb_ft3)
                .sum()
        };
        let a_dead = area(&dead);
        let a_live = area(&live);
        let a_t = a_dead + a_live;
        if a_t <= 0.0 {
            return Err(RothermelError::NoFuel {
                classes: bed.classes.len(),
            });
        }
        let category = |indices: &[usize], cs: &[&Class], a_i: f64| -> (CategoryTerms, f64) {
            let v = category_values(cs, a_i, a_t);
            let f_eps = cs
                .iter()
                .zip(&v.f_ij)
                .map(|(c, f)| f * epsilon(c.sav_per_ft))
                .collect();
            let sigma_term = v.f_i * v.sigma;
            (
                CategoryTerms {
                    classes: indices.to_vec(),
                    f_i: v.f_i,
                    f_eps,
                    wn_h: v.wn * v.h,
                    eta_s: v.eta_s,
                    f_ij: v.f_ij,
                },
                sigma_term,
            )
        };
        let dead_terms = (a_dead > 0.0).then(|| category(&dead_classes, &dead, a_dead));
        let live_terms = (a_live > 0.0).then(|| category(&live_classes, &live, a_live));

        // σ = Σ_i f_i σ_i (eq. 71); ρ_b = (1/δ) ΣΣ (w0)_ij (eq. 74);
        // β = (1/δ) ΣΣ (w0)_ij/(ρ_p)_ij (eq. 73).
        let sigma: f64 = [&dead_terms, &live_terms]
            .into_iter()
            .flatten()
            .map(|(_, s)| s)
            .sum();
        let rho_b = bed.classes.iter().map(|c| c.load_lb_ft2).sum::<f64>() / depth;
        let beta = bed
            .classes
            .iter()
            .map(|c| c.load_lb_ft2 / c.rho_p_lb_ft3)
            .sum::<f64>()
            / depth;
        let beta_op = beta_op(sigma);
        let ratio = beta / beta_op;
        Ok(Self {
            n_classes: bed.classes.len(),
            mx1,
            live_mx: if live_terms.is_some() {
                LiveMxTerms::new(&dead, &live)
            } else {
                None
            },
            dead: dead_terms.map(|(t, _)| t),
            live: live_terms.map(|(t, _)| t),
            dead_classes,
            sigma,
            beta,
            beta_op,
            ratio,
            gamma: gamma_prime(gamma_max(sigma), ratio, a_albini(sigma)),
            xi: xi(sigma, beta),
            rho_b,
            c: wind_c(sigma),
            b: wind_b(sigma),
            e: wind_e(sigma),
            tr: residence_time_min(sigma),
        })
    }

    /// R_0, I_R, the heat sink and (M_x)_2 with the class at bed index `i` at
    /// moisture `moisture(i)`.
    ///
    /// # Errors
    /// [`RothermelError::InvalidClass`] when a moisture is negative or not
    /// finite, and [`RothermelError::ZeroHeatSink`] when the heat sink is not
    /// positive.
    fn eval(&self, moisture: impl Fn(usize) -> f64) -> Result<MoistureTerms> {
        for index in 0..self.n_classes {
            check_class(index, "moisture", moisture(index), false)?;
        }
        let mx2 = self.live.as_ref().map(|_| match &self.live_mx {
            Some(t) => t.eval(|j| moisture(self.dead_classes[j]), self.mx1),
            None => self.mx1,
        });
        // (M_f)_i = Σ_j f_ij (M_f)_ij (eq. 66).
        let mf = |t: &CategoryTerms| -> f64 {
            t.classes
                .iter()
                .zip(&t.f_ij)
                .map(|(&i, f)| f * moisture(i))
                .sum()
        };
        let cats = [
            self.dead.as_ref().map(|t| (t, self.mx1)),
            self.live.as_ref().zip(mx2),
        ];
        // I_R = Γ' Σ_i (w_n)_i h_i (η_M)_i (η_s)_i (Table 6c, Albini p. 17 replacing eq. 58).
        let ir = self.gamma
            * cats
                .iter()
                .flatten()
                .map(|(t, mx)| t.wn_h * eta_m(mf(t), *mx) * t.eta_s)
                .sum::<f64>();
        // Heat sink ρ_b ε Q_ig = ρ_b Σ_i f_i Σ_j f_ij exp(−138/σ_ij) (Q_ig)_ij (eq. 77).
        let heat_sink = self.rho_b
            * cats
                .iter()
                .flatten()
                .map(|(t, _)| {
                    t.f_i
                        * t.classes
                            .iter()
                            .zip(&t.f_eps)
                            .map(|(&i, fe)| fe * q_ig(moisture(i)))
                            .sum::<f64>()
                })
                .sum::<f64>();
        if !heat_sink.is_finite() || heat_sink <= 0.0 {
            return Err(RothermelError::ZeroHeatSink {
                heat_sink_btu_ft3: heat_sink,
            });
        }
        // R_0 = I_R ξ / (ρ_b ε Q_ig) (eq. 75).
        Ok(MoistureTerms {
            r0: ir * self.xi / heat_sink,
            ir,
            heat_sink,
            live_mx: mx2,
        })
    }

    /// Number of classes of the bed.
    pub(crate) fn n_classes(&self) -> usize {
        self.n_classes
    }
}

/// The terms of [`DirModel`] that depend on neither the class moistures nor
/// wind or slope.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DirTerms {
    bed: BedTerms,
    limit: WindLimit,
    inv_b: f64,
    ratio_neg_e: f64,
    ratio_e: f64,
    phi_s_coef: f64,
}

impl DirTerms {
    /// Validates the bed and computes its moisture-independent terms.
    pub(crate) fn new(bed: &Bed<'_>, limit: WindLimit) -> Result<Self> {
        let bed = BedTerms::new(bed)?;
        Ok(Self {
            limit,
            inv_b: 1.0 / bed.b,
            ratio_neg_e: bed.ratio.powf(-bed.e),
            ratio_e: bed.ratio.powf(bed.e),
            phi_s_coef: 5.275 * bed.beta.powf(-0.3),
            bed,
        })
    }

    /// Number of classes of the bed.
    pub(crate) fn n_classes(&self) -> usize {
        self.bed.n_classes()
    }

    /// The [`DirModel`] of the bed with the class at bed index `i` at
    /// moisture `moisture(i)`.
    ///
    /// # Errors
    /// The errors of [`BedTerms::eval`].
    pub(crate) fn at(&self, moisture: impl Fn(usize) -> f64) -> Result<DirModel> {
        let m = self.bed.eval(moisture)?;
        let u_max = wind_limit_ft_min(m.ir);
        Ok(DirModel {
            limit: self.limit,
            r0: m.r0,
            ir: m.ir,
            c: self.bed.c,
            b: self.bed.b,
            inv_b: self.inv_b,
            ratio_neg_e: self.ratio_neg_e,
            ratio_e: self.ratio_e,
            phi_s_coef: self.phi_s_coef,
            u_max,
            phi_w_at_u_max: self.bed.c * u_max.powf(self.bed.b) * self.ratio_neg_e,
            ir_tr: m.ir * self.bed.tr,
        })
    }
}

/// Basic single-class model of §3.1 (pp. 12-15) with the Rothermel (1972)
/// eq. 39 exponent A and no wind limit. Returns R in ft/min.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the oracle's positional signature"
)]
pub(crate) fn single_class(
    sav_per_ft: f64,
    load_lb_ft2: f64,
    depth_ft: f64,
    mx: f64,
    heat_btu_lb: f64,
    moisture: f64,
    wind_ft_min: f64,
    slope_tan: f64,
    s_t: f64,
    s_e: f64,
    rho_p_lb_ft3: f64,
) -> Result<f64> {
    let sigma = check("sav", sav_per_ft, true)?;
    let w0 = check("load", load_lb_ft2, true)?;
    let depth = check("depth", depth_ft, true)?;
    let mx = check("moisture of extinction", mx, true)?;
    let heat = check("heat", heat_btu_lb, true)?;
    let mf = check("moisture", moisture, false)?;
    let wind = check("wind", wind_ft_min, false)?;
    let slope_tan = check("slope", slope_tan, false)?;
    let s_t = check("total mineral content", s_t, false)?;
    let s_e = check("effective mineral content", s_e, true)?;
    let rho_p = check("particle density", rho_p_lb_ft3, true)?;

    // w_n = w0 (1 − S_T); ρ_b = w0/δ; β = ρ_b/ρ_p (p. 12 §3.1.2).
    let wn = w0 * (1.0 - s_t);
    let rho_b = w0 / depth;
    let beta = rho_b / rho_p;
    let ratio = beta / beta_op(sigma);
    let gamma = gamma_prime(gamma_max(sigma), ratio, a_rothermel(sigma));
    // I_R = Γ w_n h with Γ = Γ' η_M η_s (p. 13 §3.1.3).
    let ir = gamma * wn * heat * eta_m(mf, mx) * eta_s(s_e);
    // R = I_R ξ (1 + φ_w + φ_s) / (ρ_b ε Q_ig) (p. 11 eq. 52; pp. 13-15).
    let heat_sink = rho_b * epsilon(sigma) * q_ig(mf);
    let pw = phi_w(wind_c(sigma), wind_b(sigma), wind_e(sigma), wind, ratio);
    Ok(ir * xi(sigma, beta) / heat_sink * (1.0 + pw + phi_s(beta, slope_tan)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, want: f64, tol: f64, what: &str) {
        assert!(
            (got - want).abs() <= tol * want.abs().max(1.0),
            "{what}: got {got}, expected {want}"
        );
    }

    #[test]
    fn coefficient_identities_at_unit_arguments() {
        close(beta_op(1.0), 3.348, 1e-12, "beta_op(1)");
        close(gamma_max(1.0), 1.0 / 495.0594, 1e-12, "gamma_max(1)");
        close(a_albini(1.0), 133.0, 1e-12, "A_albini(1)");
        close(
            a_rothermel(1024.0),
            1.0 / (4.774 * 2.0 - 7.27),
            1e-12,
            "A_rothermel(1024)",
        );
        close(eta_m(0.0, 0.2), 1.0, 1e-12, "eta_m(0)");
        close(eta_m(0.2, 0.2), 0.0, 1e-12, "eta_m(r=1)");
        close(eta_m(0.5, 0.2), 0.0, 1e-12, "eta_m(r>1 clamps to 1)");
        close(
            eta_m(0.1, 0.2),
            1.0 - 1.295 + 1.2775 - 0.44,
            1e-12,
            "eta_m(r=0.5)",
        );
        close(eta_s(1.0), 0.174, 1e-12, "eta_s(1)");
        close(eta_s(1e-12), 1.0, 1e-12, "eta_s clamps to 1");
        close(q_ig(0.0), 250.0, 1e-12, "q_ig(0)");
        close(q_ig(1.0), 1366.0, 1e-12, "q_ig(1)");
        close(epsilon(138.0), (-1.0f64).exp(), 1e-12, "epsilon(138)");
        close(phi_s(1.0, 2.0), 21.1, 1e-12, "phi_s(beta=1, tan=2)");
        close(wind_c(0.0), 7.47, 1e-12, "C(0)");
        close(wind_b(1.0), 0.02526, 1e-12, "B(1)");
        close(wind_e(0.0), 0.715, 1e-12, "E(0)");
        close(xi(0.0, -0.1), 1.0 / 192.0, 1e-12, "xi(0, -0.1)");
        close(wind_limit_ft_min(1000.0), 900.0, 1e-12, "wind limit");
        close(residence_time_min(384.0), 1.0, 1e-12, "t_r(384)");
        close(fireline_intensity(60.0, 1.0, 1.0), 1.0, 1e-12, "I_B");
        close(flame_length(1.0), 0.45, 1e-12, "F_B(1)");
    }

    #[test]
    fn exponents_at_doubled_argument() {
        // B(2σ)/B(σ) = 2^0.54, A(2σ)/A(σ) = 2^-0.7913, β_op(2σ)/β_op(σ) = 2^-0.8189.
        close(
            wind_b(2000.0) / wind_b(1000.0),
            2f64.powf(0.54),
            1e-12,
            "B ratio",
        );
        close(
            a_albini(2000.0) / a_albini(1000.0),
            2f64.powf(-0.7913),
            1e-12,
            "A ratio",
        );
        close(
            beta_op(2000.0) / beta_op(1000.0),
            2f64.powf(-0.8189),
            1e-12,
            "beta_op ratio",
        );
        // C: ln(C(σ)/7.47) = −0.133 σ^0.55; E: ln(E(σ)/0.715) = −3.59e-4 σ.
        close(
            (wind_c(1000.0) / 7.47).ln(),
            -0.133 * 1000f64.powf(0.55),
            1e-12,
            "C exponent",
        );
        close((wind_e(1000.0) / 0.715).ln(), -0.359, 1e-12, "E exponent");
        // ξ: ln(ξ(σ, β)(192 + 0.2595σ)) = (0.792 + 0.681 σ^0.5)(β + 0.1).
        close(
            (xi(400.0, 0.1) * (192.0 + 0.2595 * 400.0)).ln(),
            (0.792 + 0.681 * 20.0) * 0.2,
            1e-12,
            "xi exponent",
        );
    }

    #[test]
    fn effective_wind_inverts_phi_w() {
        let sigma = 1800.0;
        let (c, b, e) = (wind_c(sigma), wind_b(sigma), wind_e(sigma));
        for ratio in [0.2, 1.0, 3.5] {
            for u in [0.0, 88.0, 500.0, 2000.0] {
                let back = effective_wind(phi_w(c, b, e, u, ratio), c, b, e, ratio);
                close(back, u, 1e-9, "U_E(phi_w(U))");
            }
        }
    }

    #[test]
    fn subclass_bins() {
        assert_eq!(subclass(2000.0), Some(0));
        assert_eq!(subclass(1200.0), Some(0));
        assert_eq!(subclass(1199.9), Some(1));
        assert_eq!(subclass(192.0), Some(1));
        assert_eq!(subclass(109.0), Some(2));
        assert_eq!(subclass(96.0), Some(2));
        assert_eq!(subclass(48.0), Some(3));
        assert_eq!(subclass(30.0), Some(4));
        assert_eq!(subclass(16.0), Some(4));
        assert_eq!(subclass(15.9), None);
    }

    fn class(category: Category, load: f64, sav: f64, moisture: f64) -> Class {
        Class {
            category,
            load_lb_ft2: load,
            sav_per_ft: sav,
            moisture,
            heat_btu_lb: 8000.0,
            s_t: 0.0555,
            s_e: 0.01,
            rho_p_lb_ft3: 32.0,
        }
    }

    #[test]
    fn live_mx_hand_value() {
        // σ_1 = 138 and σ_2 = 500 make both exponentials e^-1; equal loads give W = 1;
        // M_f,dead = 0.1, M_x1 = 0.2: (M_x)_2 = 2.9 (1 − 0.5) − 0.226 = 1.224.
        let dead = class(Category::Dead, 0.05, 138.0, 0.1);
        let live = class(Category::Live, 0.05, 500.0, 1.0);
        close(live_mx(&[&dead], &[&live], 0.2), 1.224, 1e-12, "live Mx");
        // Floor at (M_x)_1: a tiny dead load drives W to 0 and the formula to −0.226.
        let tiny = class(Category::Dead, 1e-9, 138.0, 0.1);
        close(
            live_mx(&[&tiny], &[&live], 0.2),
            0.2,
            1e-12,
            "live Mx floor",
        );
    }

    #[test]
    fn splitting_a_class_in_two_leaves_spread_unchanged() {
        let whole = [class(Category::Dead, 0.0459, 2000.0, 0.08)];
        let halves = [
            class(Category::Dead, 0.02295, 2000.0, 0.08),
            class(Category::Dead, 0.02295, 2000.0, 0.08),
        ];
        let env = Env {
            wind_ft_min: 300.0,
            slope_tan: 0.2,
        };
        let a = spread(
            &Bed {
                classes: &whole,
                depth_ft: 1.0,
                dead_mx: 0.15,
            },
            &env,
            WindLimit::None,
        )
        .unwrap();
        let b = spread(
            &Bed {
                classes: &halves,
                depth_ft: 1.0,
                dead_mx: 0.15,
            },
            &env,
            WindLimit::None,
        )
        .unwrap();
        close(b.ros_ft_min, a.ros_ft_min, 1e-12, "R");
        close(
            b.reaction_intensity_btu_ft2_min,
            a.reaction_intensity_btu_ft2_min,
            1e-12,
            "I_R",
        );
        close(b.heat_sink_btu_ft3, a.heat_sink_btu_ft3, 1e-12, "heat sink");
        assert!(a.ros_ft_min > 0.0, "{a:?}");
    }

    #[test]
    fn no_load_is_an_error() {
        let classes = [class(Category::Dead, 0.0, 2000.0, 0.08)];
        let err = spread(
            &Bed {
                classes: &classes,
                depth_ft: 1.0,
                dead_mx: 0.15,
            },
            &Env {
                wind_ft_min: 0.0,
                slope_tan: 0.0,
            },
            WindLimit::None,
        )
        .unwrap_err();
        assert!(
            matches!(err, RothermelError::NoFuel { classes: 1 }),
            "{err}"
        );
    }
}
