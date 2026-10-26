//! Times one `simulate_window` over a 2000x2000 grid of uniform fuel
//! (R = 0.5 m/s, length-to-breadth 2, head north) from one central ignition
//! with no horizon; prints the median of 5 runs on one reused `Workspace`.

use std::time::{Duration, Instant};

use firespread_spread::{Grid, SpreadFields, Workspace};

fn main() -> anyhow::Result<()> {
    const N: usize = 2000;
    const RUNS: usize = 5;
    let grid = Grid::new(N, N, 30.0)?;
    let n = grid.len();
    let lb: f32 = 2.0;
    let head = vec![0.5_f32; n];
    let ecc = vec![(1.0 - 1.0 / (lb * lb)).sqrt(); n];
    let az = vec![0.0_f32; n];
    let burnable = vec![true; n];
    let fields = SpreadFields::new(grid, &head, &ecc, &az, &burnable)?;

    let mut ws = Workspace::new();
    let mut arrival = grid.unburned();
    let mut times: Vec<Duration> = Vec::with_capacity(RUNS);
    let mut burned = 0;
    for _ in 0..RUNS {
        arrival.fill(f32::INFINITY);
        grid.ignite(&mut arrival, &[(N / 2, N / 2)], 0.0)?;
        let start = Instant::now();
        let stats = ws.simulate_window(&fields, &mut arrival, 0.0, f32::INFINITY)?;
        times.push(start.elapsed());
        burned = arrival.iter().filter(|t| t.is_finite()).count();
        println!(
            "run: {:.3} s, seeds {}, burned {}, expanded {}",
            times.last().map_or(0.0, Duration::as_secs_f64),
            stats.seeds,
            stats.burned,
            stats.expanded
        );
    }
    times.sort();
    println!(
        "grid {N}x{N}: median simulate_window {:.3} s over {RUNS} runs, {burned} cells burned of {n}",
        times[RUNS / 2].as_secs_f64()
    );
    Ok(())
}
