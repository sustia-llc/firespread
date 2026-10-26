"""Write a small GDAL-made GeoTIFF fixture for the firespread-grid reader tests.

Crops a window of each named layer in ``data/grid/`` and writes it with the
``align.write_layer`` profile except for 16 by 16 tiles, so that a 40 by 40
window has partial edge tiles. A JSON sidecar records the expected header and
every cell value in row-major order.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np
import rasterio
from rasterio.windows import Window

from .align import NODATA
from .paths import GRID_DIR, REPO_ROOT

DEFAULT_OUT = REPO_ROOT / "crates" / "firespread-grid" / "tests" / "fixtures"
TILE = 16


def crop(src_path: Path, out_dir: Path, row: int, col: int, size: int) -> dict:
    with rasterio.open(src_path) as src:
        window = Window(col, row, size, size)
        arr = src.read(1, window=window).astype(np.float32)
        transform = src.window_transform(window)
        profile = {
            "driver": "GTiff",
            "dtype": "float32",
            "count": 1,
            "width": size,
            "height": size,
            "crs": src.crs,
            "transform": transform,
            "nodata": NODATA,
            "compress": "deflate",
            "tiled": True,
            "blockxsize": TILE,
            "blockysize": TILE,
        }
        epsg = src.crs.to_epsg()
    out = out_dir / f"{src_path.stem}_{size}.tif"
    with rasterio.open(out, "w", **profile) as dst:
        dst.write(arr, 1)
    return {
        "file": out.name,
        "nx": size,
        "ny": size,
        "dx": transform.a,
        "dy": -transform.e,
        "origin_x": transform.c,
        "origin_y": transform.f,
        "epsg": epsg,
        "nodata": NODATA,
        "tile": TILE,
        "source_window": {"row": row, "col": col},
        "values": [float(v) for v in arr.ravel()],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.grid_fixture")
    parser.add_argument("--layers", default="FBFM40;SlpD")
    parser.add_argument("--row", type=int, default=1180)
    parser.add_argument("--col", type=int, default=1180)
    parser.add_argument("--size", type=int, default=40)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    for name in args.layers.split(";"):
        meta = crop(GRID_DIR / f"{name}.tif", args.out, args.row, args.col, args.size)
        sidecar = args.out / f"{name}_{args.size}.json"
        sidecar.write_text(json.dumps(meta, indent=1) + "\n")
        vals = np.array(meta["values"])
        print(f"{meta['file']}: origin=({meta['origin_x']}, {meta['origin_y']}) "
              f"nodata_cells={int((vals == NODATA).sum())} range=({vals.min()}, {vals.max()})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
