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

/// Checks the shared preconditions of the distance metrics: one shape, `dx_m`
/// finite and positive, and neither mask empty.
fn check_distance_inputs(
    simulated: &Mask,
    observed: &Mask,
    dx_m: f64,
    metric: &'static str,
) -> Result<()> {
    check_shape(simulated.rows, simulated.cols, observed.rows, observed.cols)?;
    check_param("dx_m", dx_m, true)?;
    if simulated.count() == 0 || observed.count() == 0 {
        return Err(MetricsError::EmptyPerimeters { metric });
    }
    Ok(())
}

/// The boundary of `mask`: its cells that have a 4-neighbour outside the mask
/// or lie on the grid edge, row-major.
fn boundary(mask: &Mask) -> Vec<bool> {
    let (rows, cols) = (mask.rows, mask.cols);
    let inside = |r: usize, c: usize| mask.cells[r * cols + c];
    let mut out = vec![false; mask.cells.len()];
    for r in 0..rows {
        for c in 0..cols {
            if !inside(r, c) {
                continue;
            }
            out[r * cols + c] = r == 0
                || c == 0
                || r + 1 == rows
                || c + 1 == cols
                || !inside(r - 1, c)
                || !inside(r + 1, c)
                || !inside(r, c - 1)
                || !inside(r, c + 1);
        }
    }
    out
}

/// Distance, metres, from each `true` cell of `from` to the nearest `true` cell
/// of `to`, in row-major order of `from`; `to` holds at least one `true` cell.
fn distances_to(rows: usize, cols: usize, from: &[bool], to: &[bool], dx_m: f64) -> Vec<f64> {
    let squared = distance::squared_edt(rows, cols, to);
    from.iter()
        .zip(&squared)
        .filter(|(f, _)| **f)
        .map(|(_, &d2)| d2.sqrt() * dx_m)
        .collect()
}

/// Boundary distances, metres, of two same-shape non-empty masks: from each
/// cell of ∂A to the nearest cell of ∂B, then from each cell of ∂B to the
/// nearest cell of ∂A.
fn boundary_distances(a: &Mask, b: &Mask, dx_m: f64) -> (Vec<f64>, Vec<f64>) {
    let (da, db) = (boundary(a), boundary(b));
    (
        distances_to(a.rows, a.cols, &da, &db, dx_m),
        distances_to(a.rows, a.cols, &db, &da, dx_m),
    )
}

/// Mean boundary distance, metres, of a simulated mask A and an observed mask B
/// of one shape on square cells of side `dx_m`: (Σ_{a∈∂A} d(a, ∂B) +
/// Σ_{b∈∂B} d(b, ∂A)) / (|∂A| + |∂B|), where the boundary ∂M of a mask is its
/// cells that have a 4-neighbour outside the mask or lie on the grid edge, and
/// d is the Euclidean distance between cell centres to the nearest cell of the
/// set. Distances come from the exact squared Euclidean distance transform of
/// [`uniform_growth`]. `dx_m` is finite and positive; fails with
/// [`MetricsError::EmptyPerimeters`] when either mask is empty.
pub fn boundary_mean_m(simulated: &Mask, observed: &Mask, dx_m: f64) -> Result<f64> {
    check_distance_inputs(simulated, observed, dx_m, "boundary_mean_m")?;
    let (ab, ba) = boundary_distances(simulated, observed, dx_m);
    let sum: f64 = ab.iter().chain(&ba).sum();
    Ok(sum / (ab.len() + ba.len()) as f64)
}

/// Hausdorff distance, metres, of a simulated mask A and an observed mask B of
/// one shape on square cells of side `dx_m`: max(max_{a∈∂A} d(a, ∂B),
/// max_{b∈∂B} d(b, ∂A)), with ∂ and d as in [`boundary_mean_m`]. `dx_m` is
/// finite and positive; fails with [`MetricsError::EmptyPerimeters`] when
/// either mask is empty.
pub fn hausdorff_m(simulated: &Mask, observed: &Mask, dx_m: f64) -> Result<f64> {
    check_distance_inputs(simulated, observed, dx_m, "hausdorff_m")?;
    let (ab, ba) = boundary_distances(simulated, observed, dx_m);
    Ok(ab.iter().chain(&ba).copied().fold(0.0, f64::max))
}

/// Share of the `true` cells of `from` whose Euclidean cell-centre distance,
/// metres, to the nearest `true` cell of `to` is at most `buffer_m`.
fn buffered_share(from: &Mask, to: &Mask, dx_m: f64, buffer_m: f64) -> f64 {
    let d = distances_to(from.rows, from.cols, &from.cells, &to.cells, dx_m);
    let within = d.iter().filter(|&&m| m <= buffer_m).count();
    within as f64 / d.len() as f64
}

/// Buffered precision of a simulated and an observed mask of one shape on
/// square cells of side `dx_m`: the share of simulated cells whose Euclidean
/// cell-centre distance to the nearest observed cell is at most `buffer_m`,
/// with distances as in [`boundary_mean_m`]. `dx_m` is finite and positive and
/// `buffer_m` finite and non-negative; fails with
/// [`MetricsError::EmptyPerimeters`] when either mask is empty.
pub fn buffered_precision(
    simulated: &Mask,
    observed: &Mask,
    dx_m: f64,
    buffer_m: f64,
) -> Result<f64> {
    check_distance_inputs(simulated, observed, dx_m, "buffered_precision")?;
    check_param("buffer_m", buffer_m, false)?;
    Ok(buffered_share(simulated, observed, dx_m, buffer_m))
}

/// Buffered recall of a simulated and an observed mask of one shape on square
/// cells of side `dx_m`: the share of observed cells whose Euclidean
/// cell-centre distance to the nearest simulated cell is at most `buffer_m`,
/// with distances as in [`boundary_mean_m`]. `dx_m` is finite and positive and
/// `buffer_m` finite and non-negative; fails with
/// [`MetricsError::EmptyPerimeters`] when either mask is empty.
pub fn buffered_recall(simulated: &Mask, observed: &Mask, dx_m: f64, buffer_m: f64) -> Result<f64> {
    check_distance_inputs(simulated, observed, dx_m, "buffered_recall")?;
    check_param("buffer_m", buffer_m, false)?;
    Ok(buffered_share(observed, simulated, dx_m, buffer_m))
}

/// Buffered Dice 2PR / (P + R) of the [`buffered_precision`] P and the
/// [`buffered_recall`] R of a simulated and an observed mask of one shape, and
/// 0 when P + R is 0. `dx_m` is finite and positive and `buffer_m` finite and
/// non-negative; fails with [`MetricsError::EmptyPerimeters`] when either mask
/// is empty.
pub fn buffered_dice(simulated: &Mask, observed: &Mask, dx_m: f64, buffer_m: f64) -> Result<f64> {
    check_distance_inputs(simulated, observed, dx_m, "buffered_dice")?;
    check_param("buffer_m", buffer_m, false)?;
    let p = buffered_share(simulated, observed, dx_m, buffer_m);
    let r = buffered_share(observed, simulated, dx_m, buffer_m);
    if p + r == 0.0 {
        return Ok(0.0);
    }
    Ok(2.0 * p * r / (p + r))
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

    const DX: f64 = 30.0;
    const EPS_M: f64 = 1e-12;

    fn close_m(name: &str, got: f64, want: f64) {
        assert!(
            (got - want).abs() <= EPS_M,
            "{name}: got {got}, expected {want}, diff {}",
            got - want
        );
    }

    /// A `rows` by `cols` mask holding the cells `(r, c)` of `cells`.
    fn cells_mask(rows: usize, cols: usize, cells: &[(usize, usize)]) -> Mask {
        let mut bits = vec![0_u8; rows * cols];
        for &(r, c) in cells {
            bits[r * cols + c] = 1;
        }
        mask(rows, cols, &bits)
    }

    /// The square of side `side` centred at `(centre, centre)`.
    fn square(rows: usize, centre: usize, side: usize) -> Mask {
        let h = side / 2;
        let cells: Vec<(usize, usize)> = (centre - h..=centre + h)
            .flat_map(|r| (centre - h..=centre + h).map(move |c| (r, c)))
            .collect();
        cells_mask(rows, rows, &cells)
    }

    #[test]
    fn boundary_of_a_full_grid_is_its_edge() {
        let full = mask(4, 4, &[1; 16]);
        let b = boundary(&full);
        for r in 0..4 {
            for c in 0..4 {
                let want = r == 0 || c == 0 || r == 3 || c == 3;
                assert_eq!(b[r * 4 + c], want, "cell ({r},{c}) on the boundary");
            }
        }
        assert_eq!(b.iter().filter(|&&x| x).count(), 12, "boundary cell count");
    }

    #[test]
    fn hand_fixture_equal_masks() {
        let a = square(7, 3, 3);
        close_m("boundary mean", boundary_mean_m(&a, &a, DX).unwrap(), 0.0);
        close_m("hausdorff", hausdorff_m(&a, &a, DX).unwrap(), 0.0);
        for (name, f) in [
            (
                "buffered precision",
                buffered_precision as fn(&Mask, &Mask, f64, f64) -> Result<f64>,
            ),
            ("buffered recall", buffered_recall),
            ("buffered dice", buffered_dice),
        ] {
            close_m(name, f(&a, &a, DX, 0.0).unwrap(), 1.0);
        }
    }

    #[test]
    fn hand_fixture_two_cells_four_apart() {
        let s = cells_mask(3, 9, &[(1, 2)]);
        let o = cells_mask(3, 9, &[(1, 6)]);
        close_m("boundary mean", boundary_mean_m(&s, &o, DX).unwrap(), 120.0);
        close_m("hausdorff", hausdorff_m(&s, &o, DX).unwrap(), 120.0);
        for (buffer, want) in [(120.0, 1.0), (119.0, 0.0)] {
            close_m(
                &format!("buffered precision at {buffer} m"),
                buffered_precision(&s, &o, DX, buffer).unwrap(),
                want,
            );
            close_m(
                &format!("buffered recall at {buffer} m"),
                buffered_recall(&s, &o, DX, buffer).unwrap(),
                want,
            );
            close_m(
                &format!("buffered dice at {buffer} m"),
                buffered_dice(&s, &o, DX, buffer).unwrap(),
                want,
            );
        }
    }

    #[test]
    fn hand_fixture_nested_squares() {
        let s = square(7, 3, 3);
        let o = square(7, 3, 5);
        let root2 = 2_f64.sqrt();
        let want_mean = (8.0 * 30.0 + 12.0 * 30.0 + 4.0 * 30.0 * root2) / 24.0;
        close_m(
            "boundary mean",
            boundary_mean_m(&s, &o, DX).unwrap(),
            want_mean,
        );
        close_m("hausdorff", hausdorff_m(&s, &o, DX).unwrap(), 30.0 * root2);
        close_m(
            "buffered precision at 0 m",
            buffered_precision(&s, &o, DX, 0.0).unwrap(),
            1.0,
        );
        close_m(
            "buffered recall at 0 m",
            buffered_recall(&s, &o, DX, 0.0).unwrap(),
            9.0 / 25.0,
        );
        close_m(
            "buffered recall at 30√2 m",
            buffered_recall(&s, &o, DX, 30.0 * root2).unwrap(),
            1.0,
        );
        close_m(
            "buffered dice at 0 m",
            buffered_dice(&s, &o, DX, 0.0).unwrap(),
            2.0 * (9.0 / 25.0) / (1.0 + 9.0 / 25.0),
        );
    }

    #[test]
    fn hand_fixture_grid_with_a_hole() {
        // Observed: the full 7 x 7 grid, boundary its 24 edge cells. Simulated:
        // the grid minus the centre (3, 3), boundary the 24 edge cells and the 4
        // 4-neighbours of the centre, each 2 cells from the grid edge; the
        // diagonal neighbours of the centre are off it.
        let o = mask(7, 7, &[1; 49]);
        let mut bits = [1_u8; 49];
        bits[3 * 7 + 3] = 0;
        let s = mask(7, 7, &bits);
        close_m(
            "boundary mean",
            boundary_mean_m(&s, &o, DX).unwrap(),
            4.0 * 60.0 / 52.0,
        );
        close_m("hausdorff", hausdorff_m(&s, &o, DX).unwrap(), 60.0);
    }

    #[test]
    fn distance_metrics_reject_empty_masks() {
        let e = mask(2, 2, &[0, 0, 0, 0]);
        let f = mask(2, 2, &[1, 0, 0, 0]);
        for (s, o) in [(&e, &f), (&f, &e), (&e, &e)] {
            for (name, got) in [
                ("boundary_mean_m", boundary_mean_m(s, o, DX)),
                ("hausdorff_m", hausdorff_m(s, o, DX)),
                ("buffered_precision", buffered_precision(s, o, DX, 375.0)),
                ("buffered_recall", buffered_recall(s, o, DX, 375.0)),
                ("buffered_dice", buffered_dice(s, o, DX, 375.0)),
            ] {
                assert!(
                    matches!(got, Err(MetricsError::EmptyPerimeters { metric }) if metric == name),
                    "{name} of {:?} against {:?}: got {got:?}",
                    s.cells(),
                    o.cells()
                );
            }
        }
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
