//! Elliptical fire shape of Finney (1998) RMRS-RP-4 pp. 6-7, in SI. The
//! ellipse has its rear focus at the ignition point and its axes in units of
//! spread rate.

/// Length-to-breadth ratio LB = 0.936 e^(0.2566 U_E) + 0.461 e^(−0.1548 U_E)
/// − 0.397 with U_E the effective midflame wind speed in m/s, truncated at 8
/// (RMRS-RP-4 p. 7 eq. [13]). Also floored at 1, which the formula equals at
/// U_E = 0, so that rounding cannot give LB < 1.
#[must_use]
pub fn length_to_breadth(u_e_m_s: f64) -> f64 {
    let lb = 0.936 * (0.2566 * u_e_m_s).exp() + 0.461 * (-0.1548 * u_e_m_s).exp() - 0.397;
    lb.clamp(1.0, 8.0)
}

/// Head-to-back ratio HB = (LB + (LB² − 1)^0.5) / (LB − (LB² − 1)^0.5)
/// (RMRS-RP-4 p. 7 eq. [14]) for LB ≥ 1.
pub(crate) fn head_to_back(lb: f64) -> f64 {
    let s = (lb * lb - 1.0).sqrt();
    (lb + s) / (lb - s)
}

/// Semi-minor axis a = 0.5 (R + R/HB) / LB (RMRS-RP-4 p. 7 eq. [15]).
pub(crate) fn semi_minor(head_ros: f64, hb: f64, lb: f64) -> f64 {
    0.5 * (head_ros + head_ros / hb) / lb
}

/// Eccentricity e = c/b = (1 − 1/LB²)^0.5, derived from RMRS-RP-4 p. 7
/// eqs. [14], [16] b = (R + R/HB)/2 and [17] c = b − R/HB, for LB ≥ 1.
pub(crate) fn eccentricity(lb: f64) -> f64 {
    (1.0 - 1.0 / (lb * lb)).sqrt()
}

/// Rate of spread r(ψ) = R (1 − e) / (1 − e cos ψ) along a ray from the rear
/// focus at angle `psi_rad` from the head direction, for head rate R and
/// eccentricity e in [0, 1); derived from RMRS-RP-4 p. 7 eqs. [15]-[17] as
/// the focal polar equation of the ellipse with semi-major axis b and focal
/// distance c. Gives R at ψ = 0 and the backing rate R/HB at ψ = π.
#[must_use]
pub fn ellipse_rate(head_ros: f64, eccentricity: f64, psi_rad: f64) -> f64 {
    head_ros * (1.0 - eccentricity) / (1.0 - eccentricity * psi_rad.cos())
}
