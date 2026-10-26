"""Writes the synthetic run directory `fixture-w1/` used to check the page.

Run from this directory: `python -m make_fixture`. Output: a version-1 run
manifest, four 8x8 Float32 EPSG:5070 COGs `burn_probability_h{1,3,6,12}.tif`
with concentric-ring probabilities, an EPSG:4326 `observed.geojson` square,
`scorecard.json` and `area_by_hour.json`. Every value is synthetic.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
import rasterio
from pyproj import Transformer
from rasterio.transform import from_origin

OUT = Path(__file__).resolve().parent / "fixture-w1"
RUN_ID = "fixture-w1"
N = 8
DX = 1000.0
# Upper-left corner 32 km inside the master grid's upper-left (-876180, 1499820).
ORIGIN_X = -844180.0
ORIGIN_Y = 1467820.0
NODATA = -9999.0
HOURS = [1, 3, 6, 12]
START = "2022-04-22T12:00:00"
TARGET = "2022-04-23T00:00:00"


def rings(hour: int) -> np.ndarray:
    """p = 1 - d / R in 0.1 steps, R = 1.2 * sqrt(hour) cells from the grid centre."""
    rows, cols = np.mgrid[0:N, 0:N].astype(np.float64)
    d = np.hypot(rows - (N - 1) / 2, cols - (N - 1) / 2)
    p = np.clip(1.0 - d / (1.2 * np.sqrt(hour)), 0.0, 1.0)
    return (np.round(p * 10) / 10).astype(np.float32)


def write_cog(path: Path, arr: np.ndarray) -> None:
    profile = {
        "driver": "COG",
        "dtype": "float32",
        "count": 1,
        "width": N,
        "height": N,
        "crs": "EPSG:5070",
        "transform": from_origin(ORIGIN_X, ORIGIN_Y, DX, DX),
        "nodata": NODATA,
        "compress": "deflate",
        "blocksize": 16,
        "overviews": "NONE",
    }
    with rasterio.open(path, "w", **profile) as dst:
        dst.write(arr, 1)


def observed_square() -> dict:
    to_ll = Transformer.from_crs("EPSG:5070", "EPSG:4326", always_xy=True)
    x0, y0 = ORIGIN_X + 2 * DX, ORIGIN_Y - 2 * DX
    x1, y1 = ORIGIN_X + 6 * DX, ORIGIN_Y - 6 * DX
    ring = [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
    coords = [[round(v, 6) for v in to_ll.transform(x, y)] for x, y in ring]
    return {
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "properties": {"t": TARGET, "source": "synthetic"},
                "geometry": {"type": "Polygon", "coordinates": [coords]},
            }
        ],
    }


def manifest(outputs: dict[str, str]) -> dict:
    return {
        "version": 1,
        "run_id": RUN_ID,
        "window": {"name": "w1", "start_t": START, "target_t": TARGET, "fireids": [0]},
        "grid": {
            "nx": N,
            "ny": N,
            "dx": DX,
            "dy": DX,
            "origin_x": ORIGIN_X,
            "origin_y": ORIGIN_Y,
            "epsg": 5070,
            "nodata": NODATA,
        },
        "inputs": {},
        "hrrr_source": "synthetic",
        "seed": 0,
        "wind_limit": "rothermel",
        "members": 0,
        "horizons_h": HOURS,
        "outputs": outputs,
    }


def scorecard() -> dict:
    rows = []
    for k, h in enumerate(HOURS):
        rows.append({"entrant": "firespread", "horizon_h": h, "jaccard": 0.5 - 0.1 * k, "dice": 0.6 - 0.1 * k,
                     "precision": 0.7 - 0.1 * k, "recall": 0.8 - 0.1 * k, "brier": 0.1 + 0.05 * k})
        rows.append({"entrant": "persistence", "horizon_h": h, "jaccard": 0.4 - 0.1 * k, "dice": 0.5 - 0.1 * k,
                     "precision": 0.9, "recall": 0.4 - 0.1 * k, "brier": None})
        rows.append({"entrant": "uniform_growth", "horizon_h": h, "jaccard": 0.45 - 0.1 * k, "dice": 0.55 - 0.1 * k,
                     "precision": None, "recall": None, "brier": None})
    for r in rows:
        for key in ("jaccard", "dice", "precision", "recall", "brier"):
            if r[key] is not None:
                r[key] = round(r[key], 4)
    return {"run_id": RUN_ID, "window": "w1", "observed": "synthetic", "rows": rows}


def area_by_hour() -> dict:
    hours = [f"2022-04-22T{12 + h:02d}:00:00Z" if 12 + h < 24 else "2022-04-23T00:00:00Z" for h in range(1, 13)]
    p50 = [2.0 * h for h in range(1, 13)]
    return {
        "run_id": RUN_ID,
        "hours_utc": hours,
        "ensemble_km2": {"p10": [1.0 * h for h in range(1, 13)], "p50": p50, "p90": [3.0 * h for h in range(1, 13)]},
        "observed": [{"t": "2022-04-22T12:00:00Z", "area_km2": 1.0}, {"t": "2022-04-23T00:00:00Z", "area_km2": 20.0}],
    }


def dump(path: Path, obj: dict) -> None:
    path.write_text(json.dumps(obj, indent=2) + "\n")


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    outputs = {}
    for h in HOURS:
        name = f"burn_probability_h{h}.tif"
        write_cog(OUT / name, rings(h))
        outputs[f"burn_probability_h{h}"] = name
    dump(OUT / "manifest.json", manifest(outputs))
    dump(OUT / "observed.geojson", observed_square())
    dump(OUT / "scorecard.json", scorecard())
    dump(OUT / "area_by_hour.json", area_by_hour())
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
