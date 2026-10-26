//! Lazy [`FieldProvider`]s against the slice-backed [`SpreadFields`] on
//! random small grids, and the number of provider calls per expanded cell.

use firespread_spread::{
    CellFields, FieldProvider, Grid, SpreadError, SpreadFields, WindowStats, Workspace,
};

const CASES: u64 = 50;

/// SplitMix64 of `x`.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform value in `[0, 1)` from `(seed, salt, i)`.
fn unit(seed: u64, salt: u64, i: u64) -> f32 {
    let bits = mix(mix(seed ^ salt.wrapping_mul(0x1000_0001)) ^ i) >> 40;
    bits as f32 / (1_u64 << 24) as f32
}

/// The fields of `cell` in case `seed`: head rate in `[0, 2)` m/s with one
/// cell in eight at 0, eccentricity in `[0, 0.95)`, azimuth in `[0, 2π)`.
fn random_fields(seed: u64, cell: usize) -> CellFields {
    let i = cell as u64;
    let head = if unit(seed, 1, i) < 0.125 {
        0.0
    } else {
        2.0 * unit(seed, 2, i)
    };
    CellFields {
        head_ros_m_s: head,
        eccentricity: 0.95 * unit(seed, 3, i),
        max_azimuth_rad: std::f32::consts::TAU * unit(seed, 4, i),
    }
}

/// Computes [`random_fields`] when queried.
struct Lazy {
    seed: u64,
}

impl FieldProvider for Lazy {
    type Error = SpreadError;

    fn fields(&mut self, cell: usize) -> Result<CellFields, SpreadError> {
        Ok(random_fields(self.seed, cell))
    }
}

/// Counts the calls per cell of a wrapped provider.
struct Counting<P> {
    inner: P,
    calls: Vec<u32>,
}

impl<P: FieldProvider> FieldProvider for Counting<P> {
    type Error = P::Error;

    fn fields(&mut self, cell: usize) -> Result<CellFields, P::Error> {
        self.calls[cell] += 1;
        self.inner.fields(cell)
    }
}

struct Case {
    seed: u64,
    grid: Grid,
    head: Vec<f32>,
    ecc: Vec<f32>,
    az: Vec<f32>,
    burnable: Vec<bool>,
    ignition: Vec<f32>,
    /// Window boundaries: one window `[w0, w1]` or two `[w0, w1]`, `[w1, w2]`.
    windows: Vec<f32>,
}

impl Case {
    fn new(seed: u64) -> Self {
        let nx = 5 + (mix(seed ^ 11) % 36) as usize;
        let ny = 5 + (mix(seed ^ 12) % 36) as usize;
        let grid = Grid::new(nx, ny, 30.0).expect("valid grid");
        let n = grid.len();
        let fields: Vec<CellFields> = (0..n).map(|c| random_fields(seed, c)).collect();
        let burnable = (0..n).map(|c| unit(seed, 5, c as u64) >= 0.15).collect();
        let mut ignition = grid.unburned();
        let ignitions = 1 + mix(seed ^ 13) % 3;
        for k in 0..ignitions {
            let cell = (mix(seed ^ (100 + k)) % n as u64) as usize;
            ignition[cell] = ignition[cell].min(-60.0 * unit(seed, 6, k));
        }
        let end = |salt| {
            if unit(seed, salt, 0) < 0.25 {
                f32::INFINITY
            } else {
                600.0 + 5400.0 * unit(seed, salt, 1)
            }
        };
        let windows = if seed.is_multiple_of(2) {
            vec![0.0, end(7)]
        } else {
            let mid = 120.0 + 1800.0 * unit(seed, 8, 0);
            vec![0.0, mid, mid + end(9)]
        };
        Self {
            seed,
            grid,
            head: fields.iter().map(|f| f.head_ros_m_s).collect(),
            ecc: fields.iter().map(|f| f.eccentricity).collect(),
            az: fields.iter().map(|f| f.max_azimuth_rad).collect(),
            burnable,
            ignition,
            windows,
        }
    }

    fn eager(&self) -> SpreadFields<'_> {
        SpreadFields::new(self.grid, &self.head, &self.ecc, &self.az, &self.burnable)
            .expect("lengths match")
    }

    /// Runs every window over `provider`, calling `after` with each window's
    /// stats and provider.
    fn run<P: FieldProvider<Error = SpreadError>>(
        &self,
        provider: &mut P,
        mut after: impl FnMut(&WindowStats, &mut P),
    ) -> (Vec<f32>, Vec<WindowStats>) {
        let mut ws = Workspace::new();
        let mut arrival = self.ignition.clone();
        let mut stats = Vec::new();
        for w in self.windows.windows(2) {
            let s = ws
                .simulate_window_with(
                    self.grid,
                    &self.burnable,
                    provider,
                    &mut arrival,
                    w[0],
                    w[1],
                )
                .expect("valid window");
            after(&s, provider);
            stats.push(s);
        }
        (arrival, stats)
    }
}

#[test]
fn lazy_and_slice_providers_give_bit_identical_arrivals() {
    let (mut burned, mut two_window) = (0_usize, 0_usize);
    for seed in 0..CASES {
        let case = Case::new(seed);
        let mut ws = Workspace::new();
        let mut eager = case.ignition.clone();
        let mut eager_stats = Vec::new();
        for w in case.windows.windows(2) {
            eager_stats.push(
                ws.simulate_window(&case.eager(), &mut eager, w[0], w[1])
                    .expect("valid window"),
            );
        }
        let (lazy, lazy_stats) = case.run(&mut Lazy { seed: case.seed }, |_, _| {});
        let differ: Vec<usize> = (0..eager.len())
            .filter(|&i| eager[i].to_bits() != lazy[i].to_bits())
            .collect();
        assert!(
            differ.is_empty(),
            "seed {seed}: {} cells differ, first {:?}: eager {} lazy {}",
            differ.len(),
            differ.first(),
            differ.first().map_or(0.0, |&i| eager[i]),
            differ.first().map_or(0.0, |&i| lazy[i])
        );
        assert_eq!(eager_stats, lazy_stats, "seed {seed}");
        burned += lazy_stats.iter().map(|s| s.burned).sum::<usize>();
        two_window += usize::from(lazy_stats.len() == 2 && lazy_stats[1].expanded > 0);
    }
    assert!(burned > 10_000, "only {burned} cells burned over all cases");
    assert!(two_window > 5, "only {two_window} second windows expanded");
}

#[test]
fn provider_is_called_once_per_expanded_cell_per_window() {
    let mut total = 0_usize;
    for seed in 0..CASES {
        let case = Case::new(seed);
        let mut counting = Counting {
            inner: case.eager(),
            calls: vec![0; case.grid.len()],
        };
        case.run(&mut counting, |stats, p| {
            let calls: u64 = p.calls.iter().map(|&c| u64::from(c)).sum();
            let max = p.calls.iter().copied().max().unwrap_or(0);
            let worst = p.calls.iter().position(|&c| c == max);
            assert!(
                max <= 1,
                "seed {seed}: cell {worst:?} queried {max} times in one window"
            );
            assert_eq!(
                calls, stats.expanded as u64,
                "seed {seed}: {calls} calls for {} expanded cells",
                stats.expanded
            );
            total += stats.expanded;
            p.calls.fill(0);
        });
    }
    assert!(total > 10_000, "only {total} cells expanded over all cases");
}

/// Fails at one cell.
struct FailAt {
    inner: Lazy,
    cell: usize,
}

#[derive(Debug, PartialEq)]
enum TestError {
    Spread(String),
    Provider(usize),
}

impl From<SpreadError> for TestError {
    fn from(e: SpreadError) -> Self {
        Self::Spread(e.to_string())
    }
}

impl FieldProvider for FailAt {
    type Error = TestError;

    fn fields(&mut self, cell: usize) -> Result<CellFields, TestError> {
        if cell == self.cell {
            return Err(TestError::Provider(cell));
        }
        Ok(self.inner.fields(cell)?)
    }
}

#[test]
fn provider_errors_and_spread_errors_reach_the_caller() {
    let grid = Grid::new(5, 5, 30.0).expect("valid grid");
    let burnable = vec![true; 25];
    let mut arrival = grid.unburned();
    grid.ignite(&mut arrival, &[(2, 2)], 0.0).expect("on grid");
    let mut p = FailAt {
        inner: Lazy { seed: 1 },
        cell: 12,
    };
    let err =
        Workspace::new().simulate_window_with(grid, &burnable, &mut p, &mut arrival, 0.0, 1e6);
    assert_eq!(err, Err(TestError::Provider(12)));

    let err = Workspace::new().simulate_window_with(
        grid,
        &burnable[..24],
        &mut p,
        &mut arrival,
        0.0,
        1e6,
    );
    assert_eq!(
        err,
        Err(TestError::Spread(
            SpreadError::FieldLength {
                field: "burnable",
                expected: 25,
                got: 24
            }
            .to_string()
        ))
    );
}
