#![forbid(unsafe_code)]
//! Minimum-travel-time fire spread over a grid of square cells.
//!
//! Cells are stored row-major: cell `(row, col)` has index `row * nx + col`,
//! row 0 is the north row and `col` increases eastward. Bearings are radians
//! clockwise from north. Arrival times are seconds; `f32::INFINITY` marks an
//! unburned cell.
//!
//! Each burning cell `u` spreads to the cells of the 32-neighbour [`STENCIL`]
//! (offsets `(±1,0)`, `(0,±1)`, `(±1,±1)`, `(±1,±2)`, `(±2,±1)`, `(±1,±3)`,
//! `(±3,±1)`, `(±2,±3)`, `(±3,±2)` in `(row, col)`), over an edge of length
//! `dx * sqrt(dr² + dc²)`. Along an edge of bearing `θ` the spread rate is the
//! polar form of the fire ellipse about its rear focus (Finney 1998,
//! RMRS-RP-4, eqs. 15-17), `r = R_u (1 - e_u) / (1 - e_u cos(θ - φ_u))`, with
//! head rate `R_u`, eccentricity `e_u` and head azimuth `φ_u` taken from the
//! source cell only. The edge travel time is the edge length over `r`; an edge
//! with `r <= 0` or a non-finite `r` does not exist. An edge exists only when
//! its destination is burnable and it is not blocked by its [`Crossing`]: a
//! move is blocked when a cell whose interior the open segment between the two
//! cell centres meets is non-burnable, or when the segment passes through a
//! cell corner and both cells it touches only at that corner are non-burnable.

mod error;

use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub use error::{Result, SpreadError};

/// Grid geometry: `nx` columns, `ny` rows, square cells of side `dx_m` metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    nx: usize,
    ny: usize,
    dx_m: f32,
}

impl Grid {
    /// Builds a grid of `nx` columns and `ny` rows with cell side `dx_m`.
    ///
    /// # Errors
    /// [`SpreadError::EmptyGrid`] when `nx` or `ny` is zero,
    /// [`SpreadError::GridTooLarge`] when `nx * ny` exceeds `u32::MAX`, and
    /// [`SpreadError::InvalidCellSize`] when `dx_m` is not positive and finite.
    pub fn new(nx: usize, ny: usize, dx_m: f32) -> Result<Self> {
        if nx == 0 || ny == 0 {
            return Err(SpreadError::EmptyGrid { nx, ny });
        }
        let max = u64::from(u32::MAX);
        let fits = nx
            .checked_mul(ny)
            .and_then(|n| u64::try_from(n).ok())
            .is_some_and(|n| n <= max);
        if !fits {
            return Err(SpreadError::GridTooLarge { nx, ny, max });
        }
        if !dx_m.is_finite() || dx_m <= 0.0 {
            return Err(SpreadError::InvalidCellSize {
                dx_m: f64::from(dx_m),
            });
        }
        Ok(Self { nx, ny, dx_m })
    }

    /// Number of columns.
    #[must_use]
    pub fn nx(&self) -> usize {
        self.nx
    }

    /// Number of rows.
    #[must_use]
    pub fn ny(&self) -> usize {
        self.ny
    }

    /// Cell side, metres.
    #[must_use]
    pub fn dx_m(&self) -> f32 {
        self.dx_m
    }

    /// Number of cells, `nx * ny`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nx * self.ny
    }

    /// Always `false`: a grid has at least one cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Index of cell `(row, col)`.
    ///
    /// # Errors
    /// [`SpreadError::IgnitionOutOfBounds`] when the cell is outside the grid.
    pub fn index(&self, row: usize, col: usize) -> Result<usize> {
        if row >= self.ny || col >= self.nx {
            return Err(SpreadError::IgnitionOutOfBounds {
                row,
                col,
                rows: self.ny,
                cols: self.nx,
            });
        }
        Ok(row * self.nx + col)
    }

    /// An arrival field with every cell unburned.
    #[must_use]
    pub fn unburned(&self) -> Vec<f32> {
        vec![f32::INFINITY; self.len()]
    }

    /// Sets the arrival time of each `(row, col)` in `cells` to the smaller of
    /// its current value and `t_s`.
    ///
    /// # Errors
    /// [`SpreadError::FieldLength`] when `arrival` does not hold one value per
    /// cell, [`SpreadError::InvalidIgnitionTime`] when `t_s` is not finite, and
    /// [`SpreadError::IgnitionOutOfBounds`] when a cell is outside the grid;
    /// `arrival` is unchanged on error.
    pub fn ignite(&self, arrival: &mut [f32], cells: &[(usize, usize)], t_s: f32) -> Result<()> {
        check_len("arrival_s", self.len(), arrival.len())?;
        if !t_s.is_finite() {
            return Err(SpreadError::InvalidIgnitionTime {
                t_s: f64::from(t_s),
            });
        }
        for &(row, col) in cells {
            self.index(row, col)?;
        }
        for &(row, col) in cells {
            let i = row * self.nx + col;
            arrival[i] = arrival[i].min(t_s);
        }
        Ok(())
    }
}

fn check_len(field: &'static str, expected: usize, got: usize) -> Result<()> {
    if expected == got {
        Ok(())
    } else {
        Err(SpreadError::FieldLength {
            field,
            expected,
            got,
        })
    }
}

/// Per-cell inputs for one window, as structure-of-arrays slices of length
/// `grid.len()`.
#[derive(Debug, Clone, Copy)]
pub struct SpreadFields<'a> {
    grid: Grid,
    head_ros_m_s: &'a [f32],
    eccentricity: &'a [f32],
    max_azimuth_rad: &'a [f32],
    burnable: &'a [bool],
}

impl<'a> SpreadFields<'a> {
    /// Bundles head spread rate (m/s), ellipse eccentricity (0 = circle),
    /// head azimuth (radians clockwise from north) and burnability per cell.
    ///
    /// # Errors
    /// [`SpreadError::FieldLength`] when a slice does not hold one value per
    /// cell.
    pub fn new(
        grid: Grid,
        head_ros_m_s: &'a [f32],
        eccentricity: &'a [f32],
        max_azimuth_rad: &'a [f32],
        burnable: &'a [bool],
    ) -> Result<Self> {
        let n = grid.len();
        check_len("head_ros_m_s", n, head_ros_m_s.len())?;
        check_len("eccentricity", n, eccentricity.len())?;
        check_len("max_azimuth_rad", n, max_azimuth_rad.len())?;
        check_len("burnable", n, burnable.len())?;
        Ok(Self {
            grid,
            head_ros_m_s,
            eccentricity,
            max_azimuth_rad,
            burnable,
        })
    }

    /// The grid these fields cover.
    #[must_use]
    pub fn grid(&self) -> Grid {
        self.grid
    }
}

/// Counts from one call of [`Workspace::simulate_window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowStats {
    /// Burned cells the window started from.
    pub seeds: usize,
    /// Cells given a finite arrival time by this window.
    pub burned: usize,
    /// Cells whose outgoing edges were relaxed.
    pub expanded: usize,
}

/// Number of moves in [`STENCIL`].
pub const STENCIL_LEN: usize = 32;

/// The stencil as `(row offset, col offset)`, ordered by bearing clockwise
/// from north.
pub const STENCIL: [(isize, isize); STENCIL_LEN] = [
    (-1, 0),
    (-3, 1),
    (-2, 1),
    (-3, 2),
    (-1, 1),
    (-2, 3),
    (-1, 2),
    (-1, 3),
    (0, 1),
    (1, 3),
    (1, 2),
    (2, 3),
    (1, 1),
    (3, 2),
    (2, 1),
    (3, 1),
    (1, 0),
    (3, -1),
    (2, -1),
    (3, -2),
    (1, -1),
    (2, -3),
    (1, -2),
    (1, -3),
    (0, -1),
    (-1, -3),
    (-1, -2),
    (-2, -3),
    (-1, -1),
    (-3, -2),
    (-2, -1),
    (-3, -1),
];

const MAX_CELLS: usize = 4;
const MAX_CORNERS: usize = 2;

type Offset = (isize, isize);

/// Cells a [`STENCIL`] move passes between its source cell `(0, 0)` and its
/// destination, as `(row, col)` offsets from the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crossing {
    cells: [Offset; MAX_CELLS],
    n_cells: usize,
    corners: [(Offset, Offset); MAX_CORNERS],
    n_corners: usize,
}

impl Crossing {
    /// Cells other than source and destination whose interior the open
    /// segment between the two cell centres meets, in order along the segment.
    #[must_use]
    pub fn cells(&self) -> &[Offset] {
        &self.cells[..self.n_cells]
    }

    /// For each cell corner the segment passes through, the two cells it
    /// touches only at that corner.
    #[must_use]
    pub fn corner_pairs(&self) -> &[(Offset, Offset)] {
        &self.corners[..self.n_corners]
    }
}

/// The [`Crossing`] of move `(dr, dc)`, or `None` when the move is not in
/// [`STENCIL`].
#[must_use]
pub fn crossing(dr: isize, dc: isize) -> Option<Crossing> {
    STENCIL.contains(&(dr, dc)).then(|| crossing_of(dr, dc))
}

/// Walks the segment from `(0, 0)` to `(a, b)`, `a, b >= 0`, through the cell
/// boundaries `row = k + 1/2` and `col = m + 1/2`, then mirrors by the signs of
/// `(dr, dc)`. Boundary crossings are compared exactly as numerators over
/// `2ab`; a row and a column crossing at the same point is a corner.
fn crossing_of(dr: isize, dc: isize) -> Crossing {
    let (a, b) = (dr.unsigned_abs(), dc.unsigned_abs());
    let (sr, sc) = (dr.signum(), dc.signum());
    let mut out = Crossing {
        cells: [(0, 0); MAX_CELLS],
        n_cells: 0,
        corners: [((0, 0), (0, 0)); MAX_CORNERS],
        n_corners: 0,
    };
    let (mut r, mut c) = (0_usize, 0_usize);
    let (mut k, mut m) = (0_usize, 0_usize);
    let signed = |r: usize, c: usize| -> Offset {
        let (r, c) = (r.cast_signed(), c.cast_signed());
        (sr * r, sc * c)
    };
    while k < a || m < b {
        let row_event = (k < a).then(|| (2 * k + 1) * b);
        let col_event = (m < b).then(|| (2 * m + 1) * a);
        match (row_event, col_event) {
            (Some(x), Some(y)) if x == y => {
                if let Some(slot) = out.corners.get_mut(out.n_corners) {
                    *slot = (signed(r + 1, c), signed(r, c + 1));
                    out.n_corners += 1;
                }
                r += 1;
                c += 1;
                k += 1;
                m += 1;
            }
            (Some(x), Some(y)) if x < y => {
                r += 1;
                k += 1;
            }
            (Some(_), None) => {
                r += 1;
                k += 1;
            }
            _ => {
                c += 1;
                m += 1;
            }
        }
        if (r, c) != (a, b)
            && let Some(slot) = out.cells.get_mut(out.n_cells)
        {
            *slot = signed(r, c);
            out.n_cells += 1;
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct Edge {
    dr: isize,
    dc: isize,
    offset: isize,
    len_m: f32,
    cos: f32,
    sin: f32,
    cells: [isize; MAX_CELLS],
    n_cells: usize,
    corners: [(isize, isize); MAX_CORNERS],
    n_corners: usize,
}

impl Edge {
    fn blocked(&self, burnable: &[bool], u: usize) -> bool {
        let open = |off: isize| burnable[u.wrapping_add_signed(off)];
        self.cells[..self.n_cells].iter().any(|&o| !open(o))
            || self.corners[..self.n_corners]
                .iter()
                .any(|&(p, q)| !open(p) && !open(q))
    }
}

fn stencil(grid: Grid) -> Result<[Edge; STENCIL_LEN]> {
    let nx = isize::try_from(grid.nx).map_err(|_| SpreadError::GridTooLarge {
        nx: grid.nx,
        ny: grid.ny,
        max: u64::from(u32::MAX),
    })?;
    let dx = f64::from(grid.dx_m);
    Ok(STENCIL.map(|(dr, dc)| {
        let (fr, fc) = (dr as f64, dc as f64);
        let cells = fr.hypot(fc);
        let lin = |(r, c): Offset| r * nx + c;
        let x = crossing_of(dr, dc);
        let mut edge = Edge {
            dr,
            dc,
            offset: lin((dr, dc)),
            len_m: (dx * cells) as f32,
            cos: (-fr / cells) as f32,
            sin: (fc / cells) as f32,
            cells: [0; MAX_CELLS],
            n_cells: x.n_cells,
            corners: [(0, 0); MAX_CORNERS],
            n_corners: x.n_corners,
        };
        for (slot, &o) in edge.cells.iter_mut().zip(x.cells()) {
            *slot = lin(o);
        }
        for (slot, &(p, q)) in edge.corners.iter_mut().zip(x.corner_pairs()) {
            *slot = (lin(p), lin(q));
        }
        edge
    }))
}

/// Maps an `f32` to a `u32` whose unsigned order is the IEEE total order.
fn order_key(t: f32) -> u32 {
    let b = t.to_bits();
    if b >> 31 == 1 { !b } else { b | 0x8000_0000 }
}

fn heap_key(t: f32, cell: usize) -> u64 {
    (u64::from(order_key(t)) << 32) | cell as u64
}

/// Heap storage for [`Workspace::simulate_window`]. Each call clears the heap
/// and keeps its capacity, so a call allocates only when the heap grows past
/// its largest size in earlier calls.
#[derive(Debug, Default)]
pub struct Workspace {
    heap: BinaryHeap<Reverse<u64>>,
}

impl Workspace {
    /// An empty workspace.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Propagates the fire over `[t_start_s, t_end_s]`.
    ///
    /// Cells with `arrival_s <= t_start_s` are burned and keep their value;
    /// every other cell is reset to `f32::INFINITY` and recomputed. The search
    /// starts from each burned cell with at least one unburned, burnable
    /// stencil neighbour, at that cell's own arrival time. A candidate arrival
    /// at `v` from `u` is `max(t_u + travel(u, v), t_start_s)`; a cell keeps
    /// the smallest candidate if it is at most `t_end_s` and stays
    /// `f32::INFINITY` otherwise. Dijkstra order is by arrival time, then by
    /// cell index, so the output is a function of the inputs alone.
    ///
    /// Windows `[a, b]` then `[b, c]` over the same fields, with the second
    /// run on the first one's output, give the arrival field of one window
    /// `[a, c]`. Where the field at `b` holds burned cells the first window
    /// did not propagate (cells ignited between the calls), candidates from
    /// them that fall below `b` are clamped to `b`, so the two runs differ.
    /// `t_end_s` may be `f32::INFINITY`.
    ///
    /// # Errors
    /// [`SpreadError::FieldLength`] when `arrival_s` does not hold one value
    /// per cell, [`SpreadError::InvalidWindow`] unless `t_start_s` is finite and
    /// `t_start_s <= t_end_s`, and [`SpreadError::NanArrival`] when `arrival_s`
    /// holds a NaN; `arrival_s` is unchanged on error.
    pub fn simulate_window(
        &mut self,
        fields: &SpreadFields<'_>,
        arrival_s: &mut [f32],
        t_start_s: f32,
        t_end_s: f32,
    ) -> Result<WindowStats> {
        let grid = fields.grid;
        check_len("arrival_s", grid.len(), arrival_s.len())?;
        if !t_start_s.is_finite() || t_end_s.is_nan() || t_end_s < t_start_s {
            return Err(SpreadError::InvalidWindow {
                t_start_s: f64::from(t_start_s),
                t_end_s: f64::from(t_end_s),
            });
        }
        if let Some(index) = arrival_s.iter().position(|t| t.is_nan()) {
            return Err(SpreadError::NanArrival { index });
        }
        let edges = stencil(grid)?;
        let (nx, ny) = (grid.nx, grid.ny);
        let burnable = fields.burnable;

        for t in arrival_s.iter_mut() {
            if *t > t_start_s {
                *t = f32::INFINITY;
            }
        }

        self.heap.clear();
        let mut stats = WindowStats::default();
        for (u, &t_u) in arrival_s.iter().enumerate() {
            if t_u > t_start_s {
                continue;
            }
            let (row, col) = (u / nx, u % nx);
            let open = edges.iter().any(|edge| {
                neighbour(row, col, nx, ny, edge)
                    .is_some_and(|v| burnable[v] && arrival_s[v] > t_start_s)
            });
            if open {
                stats.seeds += 1;
                self.heap.push(Reverse(heap_key(t_u, u)));
            }
        }

        while let Some(Reverse(key)) = self.heap.pop() {
            let u = (key & 0xFFFF_FFFF) as usize;
            let t_u = arrival_s[u];
            if u64::from(order_key(t_u)) != key >> 32 {
                continue;
            }
            stats.expanded += 1;
            let head = fields.head_ros_m_s[u];
            let e = fields.eccentricity[u];
            let (sin_phi, cos_phi) = fields.max_azimuth_rad[u].sin_cos();
            let numerator = head * (1.0 - e);
            let (row, col) = (u / nx, u % nx);
            let interior = row >= 3 && row + 3 < ny && col >= 3 && col + 3 < nx;
            for edge in &edges {
                let v = if interior {
                    u.wrapping_add_signed(edge.offset)
                } else {
                    match neighbour(row, col, nx, ny, edge) {
                        Some(v) => v,
                        None => continue,
                    }
                };
                if arrival_s[v] <= t_u || !burnable[v] {
                    continue;
                }
                if edge.blocked(burnable, u) {
                    continue;
                }
                let cos_d = edge.cos * cos_phi + edge.sin * sin_phi;
                let rate = numerator / (1.0 - e * cos_d);
                if !rate.is_finite() || rate <= 0.0 {
                    continue;
                }
                let candidate = (t_u + edge.len_m / rate).max(t_start_s);
                if candidate < arrival_s[v] && candidate <= t_end_s {
                    if arrival_s[v].is_infinite() {
                        stats.burned += 1;
                    }
                    arrival_s[v] = candidate;
                    self.heap.push(Reverse(heap_key(candidate, v)));
                }
            }
        }
        Ok(stats)
    }
}

fn neighbour(row: usize, col: usize, nx: usize, ny: usize, edge: &Edge) -> Option<usize> {
    let r = row.checked_add_signed(edge.dr).filter(|&r| r < ny)?;
    let c = col.checked_add_signed(edge.dc).filter(|&c| c < nx)?;
    Some(r * nx + c)
}

/// [`Workspace::simulate_window`] with a fresh [`Workspace`].
///
/// # Errors
/// As [`Workspace::simulate_window`].
pub fn simulate_window(
    fields: &SpreadFields<'_>,
    arrival_s: &mut [f32],
    t_start_s: f32,
    t_end_s: f32,
) -> Result<WindowStats> {
    Workspace::new().simulate_window(fields, arrival_s, t_start_s, t_end_s)
}
