"""Hillshade of the master-grid elevation for the page basemap.

Reads `data/grid/Elev.tif` (EPSG:5070, 30 m, nodata -9999) and writes
`data/basemap/hillshade.tif`: one uint8 band on the same grid, values 1..255,
nodata 0. A cell is nodata when it lies on the grid edge or any cell of its 3x3
window is nodata.

Formula: ESRI, "How Hillshade works", ArcGIS Pro documentation
(https://doc.esri.com/en/arcgis-pro/latest/tool-reference/3d-analyst/how-hillshade-works.html),
which cites Burrough, P. A., and R. A. McDonell, 1998, Principles of
Geographical Information Systems, Oxford University Press:

    Hillshade = 255.0 * ((cos(Zenith_rad) * cos(Slope_rad))
                + (sin(Zenith_rad) * sin(Slope_rad) * cos(Azimuth_rad - Aspect_rad)))
    Zenith_deg = 90.0 - Altitude
    Azimuth_math = 360.0 - Azimuth + 90.0   (minus 360.0 if >= 360.0)
    [dz/dx] = ((c + 2f + i) - (a + 2d + g)) / (8 * cellsize)
    [dz/dy] = ((g + 2h + i) - (a + 2b + c)) / (8 * cellsize)
    Slope_rad = arctan(z_factor * sqrt([dz/dx]^2 + [dz/dy]^2))
    Aspect_rad = atan2([dz/dy], -[dz/dx]), plus 2*pi if negative, when [dz/dx] != 0;
                 pi/2 if [dz/dy] > 0, 2*pi - pi/2 if [dz/dy] < 0, when [dz/dx] == 0

with the 3x3 window a b c / d e f / g h i, row a-b-c to the north. A negative
hillshade is set to 0. Defaults: azimuth 315 deg, altitude 45 deg, z_factor 1.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np
import rasterio

from firespread_prep.paths import DATA_DIR, GRID_DIR

ELEV_TIF = GRID_DIR / "Elev.tif"
HILLSHADE_TIF = DATA_DIR / "basemap" / "hillshade.tif"
SRC_NODATA = -9999.0
OUT_NODATA = 0


def hillshade(
    elev: np.ndarray,
    cellsize: float,
    azimuth_deg: float = 315.0,
    altitude_deg: float = 45.0,
    z_factor: float = 1.0,
    nodata: float | None = SRC_NODATA,
) -> np.ndarray:
    """Float hillshade in [0, 255] of a north-up elevation array, NaN on the edge
    ring and wherever the 3x3 window holds `nodata` or NaN."""
    z = np.asarray(elev, dtype=np.float64)
    bad = ~np.isfinite(z)
    if nodata is not None:
        bad |= z == nodata
    out = np.full(z.shape, np.nan)
    if z.shape[0] < 3 or z.shape[1] < 3:
        return out

    a, b, c = z[:-2, :-2], z[:-2, 1:-1], z[:-2, 2:]
    d, f = z[1:-1, :-2], z[1:-1, 2:]
    g, h, i = z[2:, :-2], z[2:, 1:-1], z[2:, 2:]
    dzdx = ((c + 2 * f + i) - (a + 2 * d + g)) / (8 * cellsize)
    dzdy = ((g + 2 * h + i) - (a + 2 * b + c)) / (8 * cellsize)

    zenith = np.radians(90.0 - altitude_deg)
    az_math = 360.0 - azimuth_deg + 90.0
    if az_math >= 360.0:
        az_math -= 360.0
    azimuth = np.radians(az_math)

    slope = np.arctan(z_factor * np.hypot(dzdx, dzdy))
    aspect = np.arctan2(dzdy, -dzdx)
    aspect = np.where(aspect < 0, aspect + 2 * np.pi, aspect)
    aspect = np.where(
        dzdx == 0,
        np.where(dzdy > 0, np.pi / 2, np.where(dzdy < 0, 2 * np.pi - np.pi / 2, aspect)),
        aspect,
    )

    shade = 255.0 * (np.cos(zenith) * np.cos(slope) + np.sin(zenith) * np.sin(slope) * np.cos(azimuth - aspect))
    shade = np.maximum(shade, 0.0)

    window_bad = np.zeros_like(shade, dtype=bool)
    for dr in range(3):
        for dc in range(3):
            window_bad |= bad[dr : dr + shade.shape[0], dc : dc + shade.shape[1]]
    shade[window_bad] = np.nan
    out[1:-1, 1:-1] = shade
    return out


def to_uint8(shade: np.ndarray) -> np.ndarray:
    """Rounds valid cells to 1..255 and maps NaN to OUT_NODATA (0)."""
    q = np.clip(np.rint(np.nan_to_num(shade, nan=0.0)), 1, 255).astype(np.uint8)
    q[np.isnan(shade)] = OUT_NODATA
    return q


def write_hillshade(src_path: Path, dst_path: Path, azimuth_deg: float = 315.0, altitude_deg: float = 45.0) -> Path:
    with rasterio.open(src_path) as src:
        if src.res[0] != src.res[1]:
            raise SystemExit(f"{src_path}: non-square cells {src.res}")
        elev = src.read(1)
        shade = hillshade(elev, float(src.res[0]), azimuth_deg, altitude_deg, nodata=src.nodata)
        profile = {
            "driver": "COG",
            "dtype": "uint8",
            "count": 1,
            "width": src.width,
            "height": src.height,
            "crs": src.crs,
            "transform": src.transform,
            "nodata": OUT_NODATA,
            "compress": "deflate",
        }
    dst_path.parent.mkdir(parents=True, exist_ok=True)
    with rasterio.open(dst_path, "w", **profile) as dst:
        dst.write(to_uint8(shade), 1)
    return dst_path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.hillshade")
    parser.add_argument("--elev", type=Path, default=ELEV_TIF)
    parser.add_argument("--out", type=Path, default=HILLSHADE_TIF)
    parser.add_argument("--azimuth", type=float, default=315.0)
    parser.add_argument("--altitude", type=float, default=45.0)
    args = parser.parse_args(argv)
    out = write_hillshade(args.elev, args.out, args.azimuth, args.altitude)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
