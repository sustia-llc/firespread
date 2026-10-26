"""Rasterize observed perimeters onto the master grid as u8 GeoTIFF.

Writes one file per FEDS 12-hour timestep, the union of every fire id at that
timestep, to ``data/obs/feds/<YYYYmmddTHHMM>.tif``, and the union of the WFIGS
final perimeters to ``data/obs/wfigs_final.tif``. Cells are 1 inside a
perimeter (cell centre inside, rasterio ``all_touched=False``), else 0.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import geopandas as gpd
import numpy as np
import pandas as pd
import rasterio
from rasterio.features import rasterize

from .align import target_grid
from .paths import DATA_DIR, EXTENT_JSON, FEDS_PERIMETERS_GPKG, WFIGS_FILES

OBS_DIR = DATA_DIR / "obs"
NODATA_U8 = 255


def write_mask(path: Path, mask: np.ndarray, extent: dict) -> None:
    transform, width, height, crs = target_grid(extent)
    profile = {
        "driver": "GTiff",
        "dtype": "uint8",
        "count": 1,
        "width": width,
        "height": height,
        "crs": crs,
        "transform": transform,
        "nodata": NODATA_U8,
        "compress": "deflate",
        "tiled": True,
        "blockxsize": 256,
        "blockysize": 256,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    with rasterio.open(path, "w", **profile) as dst:
        dst.write(mask.astype(np.uint8), 1)


def burn(geoms: gpd.GeoSeries, extent: dict) -> np.ndarray:
    transform, width, height, crs = target_grid(extent)
    shapes = [(g, 1) for g in geoms.to_crs(crs).geometry if g is not None and not g.is_empty]
    if not shapes:
        return np.zeros((height, width), dtype=np.uint8)
    return rasterize(shapes, out_shape=(height, width), transform=transform, fill=0, all_touched=False, dtype="uint8")


def wfigs_union() -> gpd.GeoSeries:
    frames = [gpd.read_file(p) for p in WFIGS_FILES]
    return gpd.GeoSeries(pd.concat([f.geometry.to_crs(frames[0].crs) for f in frames]), crs=frames[0].crs)


def complex_fireids(perims: gpd.GeoDataFrame) -> set[int]:
    """FEDS fire ids whose perimeter at any timestep intersects the WFIGS final perimeter union."""
    union = wfigs_union().to_crs(perims.crs).union_all()
    hits = perims[perims.geometry.intersects(union)]
    return {int(f) for f in hits["fireID"].unique()}


def feds(extent: dict, out_dir: Path) -> list[dict]:
    perims = gpd.read_file(FEDS_PERIMETERS_GPKG)
    perims["t"] = pd.to_datetime(perims["t"])
    keep = complex_fireids(perims)
    dropped = sorted({int(f) for f in perims["fireID"].unique()} - keep)
    print(f"complex fire ids {sorted(keep)}, dropped {dropped}")
    perims = perims[perims["fireID"].astype(int).isin(keep)]
    dx = extent["grid"]["dx"]
    index = []
    for t, group in perims.groupby("t"):
        mask = burn(group.geometry, extent)
        out = out_dir / "feds" / f"{t.strftime('%Y%m%dT%H%M')}.tif"
        write_mask(out, mask, extent)
        cells = int(mask.sum())
        per_fire = {
            int(fid): {
                "cells": int(burn(sub.geometry, extent).sum()),
                "farea_km2": float(sub["farea"].sum()),
                "geom_types": sorted(sub.geometry.geom_type.unique()),
                "bounds_5070": [round(v) for v in sub.geometry.to_crs(5070).total_bounds],
            }
            for fid, sub in group.groupby("fireID")
        }
        index.append(
            {
                "t": t.isoformat(),
                "path": str(out),
                "fireids": sorted(int(f) for f in group["fireID"].unique()),
                "per_fire": per_fire,
                "burned_cells": cells,
                "burned_km2": cells * dx * dx / 1e6,
                "feds_farea_km2": float(group["farea"].sum()),
            }
        )
    return index


def wfigs(extent: dict, out_dir: Path) -> dict:
    mask = burn(wfigs_union(), extent)
    out = out_dir / "wfigs_final.tif"
    write_mask(out, mask, extent)
    dx = extent["grid"]["dx"]
    return {"path": str(out), "burned_cells": int(mask.sum()), "burned_km2": int(mask.sum()) * dx * dx / 1e6}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.rasterize")
    parser.add_argument("--out", type=Path, default=OBS_DIR)
    args = parser.parse_args(argv)
    extent = json.loads(EXTENT_JSON.read_text())
    index = {"feds": feds(extent, args.out), "wfigs": wfigs(extent, args.out)}
    (args.out / "index.json").write_text(json.dumps(index, indent=1) + "\n")
    print(f"feds timesteps: {len(index['feds'])}")
    for row in index["feds"]:
        print(f"{row['t']} fireids={row['fireids']} burned_km2={row['burned_km2']:.2f} feds_farea_km2={row['feds_farea_km2']:.2f}")
    print(f"wfigs: {index['wfigs']}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
