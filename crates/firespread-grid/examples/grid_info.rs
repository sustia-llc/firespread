//! Prints, for every `.tif` in a directory, the header and read time plus
//! either the valid-cell range and nodata count (files `read_geotiff` accepts)
//! or the per-code cell counts (files `read_geotiff_u8` accepts), the read
//! errors of every other `.tif`, then whether all read headers are equal with
//! and without comparing `nodata`.
//!
//! Usage: `cargo run --release -p firespread-grid --example grid_info -- <dir>`

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use firespread_grid::{GridError, GridHeader, read_geotiff, read_geotiff_u8};

fn header_text(h: &GridHeader) -> String {
    format!(
        "{}x{} dx={} dy={} origin=({}, {}) epsg={} nodata={}",
        h.nx, h.ny, h.dx, h.dy, h.origin_x, h.origin_y, h.epsg, h.nodata
    )
}

fn read_f32(path: &Path, name: &str) -> std::result::Result<GridHeader, GridError> {
    let start = Instant::now();
    let grid = read_geotiff(path)?;
    let ms = start.elapsed().as_secs_f64() * 1e3;
    let nodata = grid.header.nodata;
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
        "{name}: f32 {} | valid min={lo} max={hi} | nodata cells={nodata_count} | read {ms:.1} ms",
        header_text(&grid.header)
    );
    Ok(grid.header)
}

fn read_u8(path: &Path, name: &str) -> std::result::Result<GridHeader, GridError> {
    let start = Instant::now();
    let mask = read_geotiff_u8(path)?;
    let ms = start.elapsed().as_secs_f64() * 1e3;
    let mut counts = [0_usize; 256];
    for &v in &mask.data {
        counts[usize::from(v)] += 1;
    }
    let codes: Vec<String> = counts
        .iter()
        .enumerate()
        .filter(|&(_, &n)| n > 0)
        .map(|(code, n)| format!("{code}:{n}"))
        .collect();
    println!(
        "{name}: u8 {} | code:cells {} | read {ms:.1} ms",
        header_text(&mask.header),
        codes.join(" ")
    );
    Ok(mask.header)
}

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
        let header = match read_f32(path, &name) {
            Ok(h) => h,
            Err(GridError::UnsupportedSampleFormat { .. }) => match read_u8(path, &name) {
                Ok(h) => h,
                Err(e) => {
                    println!("{name}: not read: {e}");
                    continue;
                }
            },
            Err(e) => {
                println!("{name}: not read: {e}");
                continue;
            }
        };
        headers.push(header);
    }
    let same = headers.windows(2).all(|w| w[0].same_grid(&w[1]));
    let geometry = |h: &GridHeader| GridHeader { nodata: 0.0, ..*h };
    let same_geometry = headers
        .windows(2)
        .all(|w| geometry(&w[0]).same_grid(&geometry(&w[1])));
    println!(
        "all {} headers read of {} files same_grid: {same}, same_grid ignoring nodata: {same_geometry}",
        headers.len(),
        paths.len()
    );
    Ok(())
}
