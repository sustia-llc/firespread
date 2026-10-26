//! Exact squared Euclidean distance transform on a regular grid.

/// One-dimensional squared Euclidean distance transform D(p) = min_q (p − q)² + f(q)
/// over the grid `0..f.len()`, written into `out`; sites with non-finite `f` are not
/// parabola roots, and an `f` with no finite site gives `+∞` everywhere. This is
/// Felzenszwalb and Huttenlocher (2012), Algorithm 1 (p. 420), with the envelope
/// started at the first finite site instead of site 0. `v` holds at least `f.len()`
/// entries and `z` at least `f.len() + 1`.
fn transform_1d(f: &[f64], out: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0_usize;
    let mut started = false;
    for q in 0..n {
        if !f[q].is_finite() {
            continue;
        }
        if !started {
            v[0] = q;
            z[0] = f64::NEG_INFINITY;
            z[1] = f64::INFINITY;
            started = true;
            continue;
        }
        let qf = q as f64;
        loop {
            let p = v[k];
            let pf = p as f64;
            let s = ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * qf - 2.0 * pf);
            if s <= z[k] {
                k = k
                    .checked_sub(1)
                    .expect("invariant: z[0] = -inf and s is finite, so s <= z[0] never holds");
            } else {
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = f64::INFINITY;
                break;
            }
        }
    }
    if !started {
        out.fill(f64::INFINITY);
        return;
    }
    k = 0;
    for (q, slot) in out.iter_mut().enumerate().take(n) {
        let qf = q as f64;
        while z[k + 1] < qf {
            k += 1;
        }
        let d = qf - v[k] as f64;
        *slot = d * d + f[v[k]];
    }
}

/// Squared Euclidean distance, in cell units, from every cell centre of a row-major
/// `rows` by `cols` grid to the nearest centre of a `true` cell in `sources`
/// (`+∞` everywhere when no cell is `true`): the 1-D transform along every column,
/// then along every row of that result (Felzenszwalb and Huttenlocher 2012,
/// Section 2.2, p. 420). `sources.len()` equals `rows * cols`.
pub(crate) fn squared_edt(rows: usize, cols: usize, sources: &[bool]) -> Vec<f64> {
    let mut grid: Vec<f64> = sources
        .iter()
        .map(|&s| if s { 0.0 } else { f64::INFINITY })
        .collect();
    let n = rows.max(cols);
    let mut f = vec![0.0; n];
    let mut out = vec![0.0; n];
    let mut v = vec![0_usize; n];
    let mut z = vec![0.0; n + 1];

    for c in 0..cols {
        for r in 0..rows {
            f[r] = grid[r * cols + c];
        }
        transform_1d(&f[..rows], &mut out[..rows], &mut v, &mut z);
        for r in 0..rows {
            grid[r * cols + c] = out[r];
        }
    }
    for r in 0..rows {
        let row = &mut grid[r * cols..(r + 1) * cols];
        f[..cols].copy_from_slice(row);
        transform_1d(&f[..cols], &mut out[..cols], &mut v, &mut z);
        row.copy_from_slice(&out[..cols]);
    }
    grid
}
