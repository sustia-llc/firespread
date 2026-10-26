#![forbid(unsafe_code)]
//! Scores of simulated fire perimeters against observed ones.

mod distance;

/// Errors raised while scoring perimeters.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MetricsError {
    /// The simulated and observed rasters differ in shape.
    #[error(
        "shape mismatch: simulated {simulated_rows}x{simulated_cols}, observed {observed_rows}x{observed_cols}"
    )]
    ShapeMismatch {
        simulated_rows: usize,
        simulated_cols: usize,
        observed_rows: usize,
        observed_cols: usize,
    },
    /// The scoring domain differs in shape from the scored grids.
    #[error(
        "domain shape mismatch: grid {grid_rows}x{grid_cols}, domain {domain_rows}x{domain_cols}"
    )]
    DomainShapeMismatch {
        grid_rows: usize,
        grid_cols: usize,
        domain_rows: usize,
        domain_cols: usize,
    },
    /// The perimeter in the score's denominator is empty, so the score is undefined.
    #[error("{metric} is undefined: the perimeter in its denominator is empty")]
    EmptyPerimeters { metric: &'static str },
    /// The scoring domain holds no cell, so the mean is undefined.
    #[error("{metric} is undefined: the scoring domain is empty")]
    EmptyDomain { metric: &'static str },
    /// A cell buffer's length is not `rows * cols`.
    #[error("cell buffer of length {len} does not fit a {rows}x{cols} grid")]
    LengthMismatch {
        rows: usize,
        cols: usize,
        len: usize,
    },
    /// A probability is non-finite or outside `[0, 1]`.
    #[error("probability {value} at cell {index} is outside [0, 1]")]
    InvalidProbability { index: usize, value: f32 },
    /// A scalar parameter is non-finite or outside its admissible range.
    #[error("parameter {name} = {value} is outside {admissible}")]
    InvalidParameter {
        name: &'static str,
        value: f64,
        admissible: &'static str,
    },
    /// No buffer distance of the start mask covers the target cell count.
    #[error(
        "no buffer of the {start_cells}-cell start mask reaches {target_cells} cells; the whole grid buffer holds {max_cells}"
    )]
    UnreachableTarget {
        start_cells: usize,
        target_cells: usize,
        max_cells: usize,
    },
}

pub type Result<T> = core::result::Result<T, MetricsError>;

fn check_len(rows: usize, cols: usize, len: usize) -> Result<()> {
    match rows.checked_mul(cols) {
        Some(n) if n == len => Ok(()),
        _ => Err(MetricsError::LengthMismatch { rows, cols, len }),
    }
}

/// A row-major boolean raster of `rows` by `cols` cells; `true` marks a burned cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    rows: usize,
    cols: usize,
    cells: Vec<bool>,
}

impl Mask {
    /// Wraps `cells` as a `rows` by `cols` mask, failing with
    /// [`MetricsError::LengthMismatch`] when `cells.len() != rows * cols`.
    pub fn new(rows: usize, cols: usize, cells: Vec<bool>) -> Result<Self> {
        check_len(rows, cols, cells.len())?;
        Ok(Self { rows, cols, cells })
    }

    /// An all-false `rows` by `cols` mask, failing when `rows * cols` overflows `usize`.
    pub fn empty(rows: usize, cols: usize) -> Result<Self> {
        let len =
            rows.checked_mul(cols)
                .ok_or(MetricsError::LengthMismatch { rows, cols, len: 0 })?;
        Ok(Self {
            rows,
            cols,
            cells: vec![false; len],
        })
    }

    /// Number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The row-major cells.
    pub fn cells(&self) -> &[bool] {
        &self.cells
    }

    /// Number of `true` cells.
    pub fn count(&self) -> usize {
        self.cells.iter().filter(|&&c| c).count()
    }
}

/// A row-major `f32` raster of `rows` by `cols` cells holding burn probabilities.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbabilityGrid {
    rows: usize,
    cols: usize,
    values: Vec<f32>,
}

impl ProbabilityGrid {
    /// Wraps `values` as a `rows` by `cols` grid, failing with
    /// [`MetricsError::LengthMismatch`] when `values.len() != rows * cols`; values are
    /// range-checked by the scores that read them.
    pub fn new(rows: usize, cols: usize, values: Vec<f32>) -> Result<Self> {
        check_len(rows, cols, values.len())?;
        Ok(Self { rows, cols, values })
    }

    /// Number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The row-major values.
    pub fn values(&self) -> &[f32] {
        &self.values
    }
}

fn check_shape(
    simulated_rows: usize,
    simulated_cols: usize,
    observed_rows: usize,
    observed_cols: usize,
) -> Result<()> {
    if simulated_rows == observed_rows && simulated_cols == observed_cols {
        Ok(())
    } else {
        Err(MetricsError::ShapeMismatch {
            simulated_rows,
            simulated_cols,
            observed_rows,
            observed_cols,
        })
    }
}

/// Cell counts `(|S∩O|, |S|, |O|)` of two same-shape masks.
fn counts(simulated: &Mask, observed: &Mask) -> Result<(usize, usize, usize)> {
    check_shape(simulated.rows, simulated.cols, observed.rows, observed.cols)?;
    let mut both = 0;
    let mut s = 0;
    let mut o = 0;
    for (&a, &b) in simulated.cells.iter().zip(&observed.cells) {
        both += usize::from(a && b);
        s += usize::from(a);
        o += usize::from(b);
    }
    Ok((both, s, o))
}

/// Jaccard index |S∩O| / |S∪O| of a simulated and an observed mask of one shape;
/// returns 1.0 when both masks are empty.
pub fn jaccard(simulated: &Mask, observed: &Mask) -> Result<f64> {
    let (both, s, o) = counts(simulated, observed)?;
    let union = s + o - both;
    if union == 0 {
        return Ok(1.0);
    }
    Ok(both as f64 / union as f64)
}

/// Sørensen–Dice index 2|S∩O| / (|S| + |O|) of a simulated and an observed mask of
/// one shape; returns 1.0 when both masks are empty.
pub fn dice(simulated: &Mask, observed: &Mask) -> Result<f64> {
    let (both, s, o) = counts(simulated, observed)?;
    let denom = s + o;
    if denom == 0 {
        return Ok(1.0);
    }
    Ok(2.0 * both as f64 / denom as f64)
}

/// Precision |S∩O| / |S| of a simulated and an observed mask of one shape; fails with
/// [`MetricsError::EmptyPerimeters`] when S is empty.
pub fn precision(simulated: &Mask, observed: &Mask) -> Result<f64> {
    let (both, s, _) = counts(simulated, observed)?;
    if s == 0 {
        return Err(MetricsError::EmptyPerimeters {
            metric: "precision",
        });
    }
    Ok(both as f64 / s as f64)
}

/// Recall |S∩O| / |O| of a simulated and an observed mask of one shape; fails with
/// [`MetricsError::EmptyPerimeters`] when O is empty.
pub fn recall(simulated: &Mask, observed: &Mask) -> Result<f64> {
    let (both, _, o) = counts(simulated, observed)?;
    if o == 0 {
        return Err(MetricsError::EmptyPerimeters { metric: "recall" });
    }
    Ok(both as f64 / o as f64)
}

/// Brier score: the mean, over the cells where `domain` is true, of (p − o)² in `f64`,
/// with p the cell's probability and o = 1 where `observed` is true and 0 elsewhere;
/// all three rasters share one shape, the domain holds at least one cell, and every
/// probability inside the domain is finite and in `[0, 1]`.
pub fn brier(probability: &ProbabilityGrid, observed: &Mask, domain: &Mask) -> Result<f64> {
    check_shape(
        probability.rows,
        probability.cols,
        observed.rows,
        observed.cols,
    )?;
    if domain.rows != probability.rows || domain.cols != probability.cols {
        return Err(MetricsError::DomainShapeMismatch {
            grid_rows: probability.rows,
            grid_cols: probability.cols,
            domain_rows: domain.rows,
            domain_cols: domain.cols,
        });
    }
    let mut sum = 0.0_f64;
    let mut n = 0_usize;
    for (index, ((&p, &o), &inside)) in probability
        .values
        .iter()
        .zip(&observed.cells)
        .zip(&domain.cells)
        .enumerate()
    {
        if !inside {
            continue;
        }
        if !(0.0..=1.0).contains(&p) {
            return Err(MetricsError::InvalidProbability { index, value: p });
        }
        let diff = f64::from(p) - if o { 1.0 } else { 0.0 };
        sum += diff * diff;
        n += 1;
    }
    if n == 0 {
        return Err(MetricsError::EmptyDomain { metric: "brier" });
    }
    Ok(sum / n as f64)
}

/// Persistence baseline: the forecast perimeter equals the start perimeter.
pub fn persistence(start: &Mask) -> Mask {
    start.clone()
}

fn check_param(name: &'static str, value: f64, positive: bool) -> Result<()> {
    let ok = value.is_finite() && if positive { value > 0.0 } else { value >= 0.0 };
    if ok {
        Ok(())
    } else {
        Err(MetricsError::InvalidParameter {
            name,
            value,
            admissible: if positive {
                "finite values > 0"
            } else {
                "finite values >= 0"
            },
        })
    }
}

/// Uniform-growth baseline over a square-cell grid of side `dx_m` metres: the cells
/// whose cell-centre Euclidean distance to the nearest `start` cell centre is at most
/// `rate_m_per_h * hours`; an empty start gives an all-false mask. `dx_m` is finite
/// and positive, `rate_m_per_h` and `hours` are finite and non-negative. Distances
/// come from the exact squared Euclidean distance transform of Felzenszwalb and
/// Huttenlocher, "Distance Transforms of Sampled Functions", Theory of Computing 8
/// (2012) 415–428, Algorithm 1 (p. 420) applied along every column and then every row
/// as in their Section 2.2 (p. 420).
pub fn uniform_growth(start: &Mask, dx_m: f64, rate_m_per_h: f64, hours: f64) -> Result<Mask> {
    check_param("dx_m", dx_m, true)?;
    check_param("rate_m_per_h", rate_m_per_h, false)?;
    check_param("hours", hours, false)?;
    if start.count() == 0 {
        return Mask::empty(start.rows, start.cols);
    }
    let radius_m = rate_m_per_h * hours;
    let limit = radius_m * radius_m;
    let cell_area = dx_m * dx_m;
    let squared = distance::squared_edt(start.rows, start.cols, &start.cells);
    let cells = squared.iter().map(|&d2| d2 * cell_area <= limit).collect();
    Mask::new(start.rows, start.cols, cells)
}

/// Fitted uniform-growth radius, metres: an `R` within `tolerance_m` above the
/// smallest distance `R*` for which `uniform_growth(start, dx_m, R, 1.0)` holds at
/// least `target_cells` cells, so `R* <= R <= R* + tolerance_m` and the buffer at `R`
/// holds at least `target_cells` cells. Returns 0 when `start` alone holds at least
/// `target_cells` cells. Found by bisection on `[0, hi]`, where `hi` is the first of
/// `dx_m, 2 dx_m, 4 dx_m, …` whose buffer reaches the target. Cell counts use the
/// distances of [`uniform_growth`]. `dx_m` and `tolerance_m` are finite and positive.
/// Fails with [`MetricsError::EmptyPerimeters`] for an empty `start` and
/// [`MetricsError::UnreachableTarget`] when `target_cells` exceeds the grid.
pub fn fit_uniform_growth_radius(
    start: &Mask,
    target_cells: usize,
    dx_m: f64,
    tolerance_m: f64,
) -> Result<f64> {
    check_param("dx_m", dx_m, true)?;
    check_param("tolerance_m", tolerance_m, true)?;
    let start_cells = start.count();
    if start_cells == 0 {
        return Err(MetricsError::EmptyPerimeters {
            metric: "uniform growth fit",
        });
    }
    if target_cells > start.cells.len() {
        return Err(MetricsError::UnreachableTarget {
            start_cells,
            target_cells,
            max_cells: start.cells.len(),
        });
    }
    if start_cells >= target_cells {
        return Ok(0.0);
    }
    let cell_area = dx_m * dx_m;
    let squared = distance::squared_edt(start.rows, start.cols, &start.cells);
    let count = |radius_m: f64| {
        let limit = radius_m * radius_m;
        squared
            .iter()
            .filter(|&&d2| d2 * cell_area <= limit)
            .count()
    };
    let mut lo = 0.0_f64;
    let mut hi = dx_m;
    while count(hi) < target_cells {
        lo = hi;
        hi *= 2.0;
    }
    while hi - lo > tolerance_m {
        let mid = 0.5 * (lo + hi);
        if count(mid) >= target_cells {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Ok(hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(rows: usize, cols: usize, bits: &[u8]) -> Mask {
        Mask::new(rows, cols, bits.iter().map(|&b| b == 1).collect()).unwrap()
    }

    const EPS: f64 = 1e-15;

    fn close(name: &str, got: f64, want: f64) {
        assert!(
            (got - want).abs() <= EPS,
            "{name}: got {got}, expected {want}, diff {}",
            got - want
        );
    }

    #[test]
    fn hand_fixture_shape_indices() {
        let s = mask(1, 4, &[1, 1, 0, 0]);
        let o = mask(1, 4, &[0, 1, 1, 0]);
        close("jaccard", jaccard(&s, &o).unwrap(), 1.0 / 3.0);
        close("dice", dice(&s, &o).unwrap(), 0.5);
        close("precision", precision(&s, &o).unwrap(), 0.5);
        close("recall", recall(&s, &o).unwrap(), 0.5);
    }

    #[test]
    fn hand_fixture_asymmetric_precision_recall() {
        // |S| = 3, |O| = 1, |S∩O| = 1: precision 1/3, recall 1.
        let s = mask(1, 4, &[1, 1, 1, 0]);
        let o = mask(1, 4, &[0, 1, 0, 0]);
        close("precision", precision(&s, &o).unwrap(), 1.0 / 3.0);
        close("recall", recall(&s, &o).unwrap(), 1.0);
    }

    #[test]
    fn both_empty_shape_indices_are_one() {
        let e = mask(2, 2, &[0, 0, 0, 0]);
        close("jaccard", jaccard(&e, &e).unwrap(), 1.0);
        close("dice", dice(&e, &e).unwrap(), 1.0);
    }

    #[test]
    fn empty_denominators_are_errors() {
        let e = mask(1, 2, &[0, 0]);
        let f = mask(1, 2, &[1, 0]);
        let got = precision(&e, &f);
        assert!(
            matches!(
                got,
                Err(MetricsError::EmptyPerimeters {
                    metric: "precision"
                })
            ),
            "precision with empty S: got {got:?}"
        );
        let got = recall(&f, &e);
        assert!(
            matches!(got, Err(MetricsError::EmptyPerimeters { metric: "recall" })),
            "recall with empty O: got {got:?}"
        );
    }

    #[test]
    fn shape_mismatch_is_reported() {
        let a = mask(1, 4, &[1, 0, 0, 0]);
        let b = mask(2, 2, &[1, 0, 0, 0]);
        let err = jaccard(&a, &b).unwrap_err();
        assert!(
            matches!(
                err,
                MetricsError::ShapeMismatch {
                    simulated_rows: 1,
                    simulated_cols: 4,
                    observed_rows: 2,
                    observed_cols: 2
                }
            ),
            "got {err:?}"
        );
        for (name, f) in [
            ("dice", dice as fn(&Mask, &Mask) -> Result<f64>),
            ("precision", precision),
            ("recall", recall),
        ] {
            let got = f(&a, &b);
            assert!(
                matches!(got, Err(MetricsError::ShapeMismatch { .. })),
                "{name}: got {got:?}"
            );
        }
    }

    #[test]
    fn mask_length_is_checked() {
        let got = Mask::new(2, 2, vec![true; 3]);
        assert!(
            matches!(
                got,
                Err(MetricsError::LengthMismatch {
                    rows: 2,
                    cols: 2,
                    len: 3
                })
            ),
            "got {got:?}"
        );
    }

    #[test]
    fn hand_fixture_brier() {
        let p = ProbabilityGrid::new(1, 4, vec![0.9, 0.1, 0.5, 0.0]).unwrap();
        let o = mask(1, 4, &[1, 0, 1, 0]);
        let d = mask(1, 4, &[1, 1, 1, 1]);
        // f32 inputs widened to f64: 0.9_f32 and 0.1_f32 are not exact, so the
        // tolerance covers the f32 representation error (~1.5e-8 per term).
        let got = brier(&p, &o, &d).unwrap();
        assert!(
            (got - 0.0675).abs() <= 1e-8,
            "brier: got {got}, expected 0.0675, diff {}",
            got - 0.0675
        );
    }

    #[test]
    fn brier_averages_over_domain_only() {
        // Domain keeps cells 0 and 2: ((0.9-1)^2 + (0.5-1)^2) / 2 = 0.13.
        let p = ProbabilityGrid::new(1, 4, vec![0.9, 0.1, 0.5, 0.0]).unwrap();
        let o = mask(1, 4, &[1, 0, 1, 0]);
        let d = mask(1, 4, &[1, 0, 1, 0]);
        let got = brier(&p, &o, &d).unwrap();
        assert!(
            (got - 0.13).abs() <= 1e-8,
            "brier: got {got}, expected 0.13, diff {}",
            got - 0.13
        );
    }

    #[test]
    fn brier_rejects_empty_domain_and_bad_probabilities() {
        let o = mask(1, 3, &[1, 0, 0]);
        let all = mask(1, 3, &[1, 1, 1]);
        let none = mask(1, 3, &[0, 0, 0]);
        let p = ProbabilityGrid::new(1, 3, vec![0.5, 0.5, 0.5]).unwrap();
        let got = brier(&p, &o, &none);
        assert!(
            matches!(got, Err(MetricsError::EmptyDomain { metric: "brier" })),
            "empty domain: got {got:?}"
        );
        for (bad, index) in [(1.5_f32, 1_usize), (-0.1, 2), (f32::NAN, 0)] {
            let mut v = vec![0.5_f32; 3];
            v[index] = bad;
            let p = ProbabilityGrid::new(1, 3, v).unwrap();
            let err = brier(&p, &o, &all).unwrap_err();
            assert!(
                matches!(err, MetricsError::InvalidProbability { index: i, .. } if i == index),
                "value {bad} at {index}: got {err:?}"
            );
        }
        let wrong = mask(3, 1, &[1, 1, 1]);
        let got = brier(&p, &o, &wrong);
        assert!(
            matches!(got, Err(MetricsError::DomainShapeMismatch { .. })),
            "3x1 domain on 1x3 grid: got {got:?}"
        );
        let got = brier(&p, &wrong, &all);
        assert!(
            matches!(got, Err(MetricsError::ShapeMismatch { .. })),
            "3x1 observed on 1x3 grid: got {got:?}"
        );
    }

    #[test]
    fn persistence_returns_start() {
        let s = mask(2, 2, &[1, 0, 0, 1]);
        assert_eq!(persistence(&s), s);
    }

    #[test]
    fn hand_fixture_uniform_growth_disc() {
        let mut bits = [0_u8; 49];
        bits[3 * 7 + 3] = 1;
        let start = mask(7, 7, &bits);
        let grown = uniform_growth(&start, 30.0, 30.0, 2.0).unwrap();
        for i in 0..7_i64 {
            for j in 0..7_i64 {
                let want = (i - 3).pow(2) + (j - 3).pow(2) <= 4;
                let got = grown.cells()[(i * 7 + j) as usize];
                assert_eq!(got, want, "cell ({i},{j}): got {got}, expected {want}");
            }
        }
        assert_eq!(grown.count(), 13, "burned cell count");
    }

    #[test]
    fn uniform_growth_empty_start_is_empty() {
        let start = mask(3, 3, &[0; 9]);
        let grown = uniform_growth(&start, 30.0, 1000.0, 10.0).unwrap();
        assert_eq!(grown.count(), 0, "burned cells from an empty start");
        assert_eq!((grown.rows(), grown.cols()), (3, 3));
    }

    #[test]
    fn hand_fixture_fitted_radius() {
        // One centre cell of a 7 x 7 grid, dx 30 m. Buffer cell counts by radius:
        // 1 below 30 m; 5 on [30, 42.43); 9 on [30√2, 60); 13 on [60, 67.08).
        let mut bits = [0_u8; 49];
        bits[3 * 7 + 3] = 1;
        let start = mask(7, 7, &bits);
        for (target, r_star) in [
            (1_usize, 0.0_f64),
            (2, 30.0),
            (5, 30.0),
            (6, 30.0 * 2_f64.sqrt()),
            (9, 30.0 * 2_f64.sqrt()),
            (10, 60.0),
            (13, 60.0),
        ] {
            let r = fit_uniform_growth_radius(&start, target, 30.0, 1.0).unwrap();
            assert!(
                r >= r_star && r <= r_star + 1.0,
                "target {target}: fitted {r} m, expected within [{r_star}, {}]",
                r_star + 1.0
            );
            let grown = uniform_growth(&start, 30.0, r, 1.0).unwrap().count();
            assert!(
                grown >= target,
                "target {target}: buffer at {r} m holds {grown} cells"
            );
        }
    }

    #[test]
    fn fitted_radius_is_monotone_and_minimal() {
        // An L-shaped start on a 20 x 25 grid; every target count from the start
        // size to the grid size.
        let (rows, cols) = (20, 25);
        let mut bits = vec![0_u8; rows * cols];
        for r in 5..12 {
            bits[r * cols + 8] = 1;
        }
        for c in 8..15 {
            bits[11 * cols + c] = 1;
        }
        let start = mask(rows, cols, &bits);
        let mut previous = 0.0_f64;
        for target in start.count()..=rows * cols {
            let r = fit_uniform_growth_radius(&start, target, 30.0, 1.0).unwrap();
            assert!(
                r >= previous,
                "target {target}: fitted {r} m below the previous fit {previous} m"
            );
            let at = uniform_growth(&start, 30.0, r, 1.0).unwrap().count();
            assert!(at >= target, "target {target}: buffer at {r} m holds {at}");
            if r > 1.0 {
                let below = uniform_growth(&start, 30.0, r - 1.0, 1.0).unwrap().count();
                assert!(
                    below < target,
                    "target {target}: buffer at {} m already holds {below}",
                    r - 1.0
                );
            }
            previous = r;
        }
    }

    #[test]
    fn fitted_radius_rejects_empty_start_and_unreachable_target() {
        let empty = mask(2, 2, &[0, 0, 0, 0]);
        let got = fit_uniform_growth_radius(&empty, 1, 30.0, 1.0);
        assert!(
            matches!(got, Err(MetricsError::EmptyPerimeters { .. })),
            "empty start: got {got:?}"
        );
        let one = mask(2, 2, &[1, 0, 0, 0]);
        let got = fit_uniform_growth_radius(&one, 5, 30.0, 1.0);
        assert!(
            matches!(
                got,
                Err(MetricsError::UnreachableTarget {
                    start_cells: 1,
                    target_cells: 5,
                    max_cells: 4
                })
            ),
            "target above the grid: got {got:?}"
        );
        let got = fit_uniform_growth_radius(&one, 4, 30.0, 0.0);
        assert!(
            matches!(
                got,
                Err(MetricsError::InvalidParameter {
                    name: "tolerance_m",
                    ..
                })
            ),
            "zero tolerance: got {got:?}"
        );
    }

    #[test]
    fn uniform_growth_rejects_bad_parameters() {
        let start = mask(1, 1, &[1]);
        for (dx, rate, hours, name) in [
            (0.0, 1.0, 1.0, "dx_m"),
            (f64::NAN, 1.0, 1.0, "dx_m"),
            (30.0, -1.0, 1.0, "rate_m_per_h"),
            (30.0, 1.0, f64::INFINITY, "hours"),
        ] {
            let err = uniform_growth(&start, dx, rate, hours).unwrap_err();
            assert!(
                matches!(err, MetricsError::InvalidParameter { name: n, .. } if n == name),
                "({dx}, {rate}, {hours}): got {err:?}"
            );
        }
    }
}
