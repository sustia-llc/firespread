"""Union the WFIGS final perimeters and define the 30 m EPSG:5070 master grid window."""

from __future__ import annotations

import json
import math
import sys

import geopandas as gpd
from pyproj import Transformer
from shapely.geometry import box
from shapely.ops import unary_union

from .paths import EXTENT_JSON, WFIGS_FILES

GRID_EPSG = 5070
GRID_DX = 30.0
WINDOW_M = 72_000.0  # 2400 cells; the WFIGS union spans 70,615 m north to south


def snap_down(value: float, step: float) -> float:
    return math.floor(value / step) * step


def snap_up(value: float, step: float) -> float:
    return math.ceil(value / step) * step


def bounds_to_4326(xmin: float, ymin: float, xmax: float, ymax: float, epsg: int) -> list[float]:
    """Project a bounding box to EPSG:4326 by densifying its edges."""
    transformer = Transformer.from_crs(f"EPSG:{epsg}", "EPSG:4326", always_xy=True)
    poly = box(xmin, ymin, xmax, ymax).segmentize(1000.0)
    xs, ys = poly.exterior.coords.xy
    lon, lat = transformer.transform(xs, ys)
    return [min(lon), min(lat), max(lon), max(lat)]


def compute_extent() -> dict:
    frames = [gpd.read_file(path) for path in WFIGS_FILES]
    for path, frame in zip(WFIGS_FILES, frames):
        print(f"{path.name}: {len(frame)} feature(s), crs={frame.crs}")
    merged = gpd.GeoDataFrame(
        geometry=[unary_union([g for f in frames for g in f.geometry])],
        crs=frames[0].crs,
    ).to_crs(epsg=4326)
    union_4326 = merged.geometry.iloc[0]
    bbox_4326 = list(union_4326.bounds)

    union_5070 = merged.to_crs(epsg=GRID_EPSG).geometry.iloc[0]
    bbox_5070 = list(union_5070.bounds)
    centroid = union_5070.centroid

    n_cells = math.ceil(WINDOW_M / GRID_DX)
    half = n_cells * GRID_DX / 2.0
    # Centre the window on the bounding box, not the area centroid, so the
    # whole WFIGS union fits when the window exceeds the box.
    box_cx = (bbox_5070[0] + bbox_5070[2]) / 2.0
    box_cy = (bbox_5070[1] + bbox_5070[3]) / 2.0
    xmin = snap_down(box_cx - half, GRID_DX)
    ymin = snap_down(box_cy - half, GRID_DX)
    xmax = xmin + n_cells * GRID_DX
    ymax = ymin + n_cells * GRID_DX

    covers = (
        xmin <= bbox_5070[0]
        and ymin <= bbox_5070[1]
        and xmax >= bbox_5070[2]
        and ymax >= bbox_5070[3]
    )

    return {
        "bbox_4326": bbox_4326,
        "bbox_5070": bbox_5070,
        "centroid_5070": [centroid.x, centroid.y],
        "union_area_km2": union_5070.area / 1e6,
        "grid": {
            "epsg": GRID_EPSG,
            "nx": n_cells,
            "ny": n_cells,
            "dx": GRID_DX,
            "origin_x": xmin,
            "origin_y": ymax,
            "origin_corner": "upper_left",
            "bounds_5070": [xmin, ymin, xmax, ymax],
            "bbox_4326": bounds_to_4326(xmin, ymin, xmax, ymax, GRID_EPSG),
            "covers_wfigs_union": covers,
        },
    }


def main(argv: list[str] | None = None) -> int:
    extent = compute_extent()
    EXTENT_JSON.parent.mkdir(parents=True, exist_ok=True)
    EXTENT_JSON.write_text(json.dumps(extent, indent=2) + "\n")
    print(json.dumps(extent, indent=2))
    print(f"wrote {EXTENT_JSON}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
