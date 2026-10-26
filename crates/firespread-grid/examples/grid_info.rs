//! Prints the header, valid-cell range, nodata count and read time of every
//! `.tif` in a directory that `read_geotiff` accepts, the read error of every
//! other `.tif`, then whether all read headers describe the same grid.
//!
//! Usage: `cargo run --release -p firespread-grid --example grid_info -- <dir>`

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use firespread_grid::{GridHeader, read_geotiff};

fn main() -> Result<()> {
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        bail!("usage: grid_info <dir>");
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    paths.retain(|p| p.extension().is_some_and(|x| x == "tif"));
    paths.sort();
    if paths.is_empty() {
        bail!("no .tif files in {}", dir.display());
    }

    let mut headers: Vec<GridHeader> = Vec::new();
    for path in &paths {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let start = Instant::now();
        let grid = match read_geotiff(path) {
            Ok(grid) => grid,
            Err(e) => {
                println!("{name}: not read: {e}");
                continue;
            }
        };
        let ms = start.elapsed().as_secs_f64() * 1e3;
        let h = grid.header;
        let nodata = h.nodata;
        let is_nodata = |v: f32| v == nodata || v.is_nan();
        let mut nodata_count = 0_usize;
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for &v in &grid.data {
            if is_nodata(v) {
                nodata_count += 1;
            } else {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        println!(
            "{name}: {}x{} dx={} dy={} origin=({}, {}) epsg={} nodata={} | valid min={lo} max={hi} | nodata cells={nodata_count} | read {ms:.1} ms",
            h.nx, h.ny, h.dx, h.dy, h.origin_x, h.origin_y, h.epsg, h.nodata
        );
        headers.push(h);
    }
    let same = headers.windows(2).all(|w| w[0].same_grid(&w[1]));
    println!(
        "all {} headers read of {} files same_grid: {same}",
        headers.len(),
        paths.len()
    );
    Ok(())
}
