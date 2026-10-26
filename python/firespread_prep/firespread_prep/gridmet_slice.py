"""Slice gridMET fm100 and fm1000 for the window dates over the master grid window."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
from pathlib import Path

import numpy as np
import rasterio
from rasterio.transform import from_origin
from rasterio.windows import from_bounds

from .paths import EXTENT_JSON, GRIDMET_DIR, WINDOWS_JSON

PRODUCTS = ("fm100", "fm1000")
SCALE = 0.1
PAD_CELLS = 2
EPOCH = dt.date(1900, 1, 1)


def nc_path(product: str) -> Path:
    return GRIDMET_DIR / f"{product}_2022.nc"


def subdataset(path: Path) -> str:
    """The netCDF subdataset name holding the fuel moisture variable."""
    with rasterio.open(path) as src:
        subs = src.subdatasets
        print(f"{path.name}: subdatasets={subs}")
        if not subs:
            return str(path)
        picks = [s for s in subs if "fuel_moisture" in s or "fm" in s.rsplit(":", 1)[-1]]
        return picks[0] if picks else subs[0]


def inspect(product: str) -> None:
    name = subdataset(nc_path(product))
    with rasterio.open(name) as src:
        print(f"{name}: crs={src.crs} shape={src.shape} count={src.count} dtype={src.dtypes[0]}")
        print(f"  transform={tuple(src.transform)[:6]} nodata={src.nodata} scales={src.scales[:3]} offsets={src.offsets[:3]}")
        print(f"  dataset tags: {src.tags()}")
        print(f"  band 1 tags: {src.tags(1)}")
        print(f"  band {src.count} tags: {src.tags(src.count)}")


def band_for_date(src: rasterio.DatasetReader, date: dt.date) -> int:
    """Band index whose NETCDF_DIM_day tag equals the date (days since 1900-01-01)."""
    want = (date - EPOCH).days
    for band in range(1, src.count + 1):
        tags = src.tags(band)
        day = tags.get("NETCDF_DIM_day")
        if day is not None and int(float(day)) == want:
            return band
    raise SystemExit(f"no band with NETCDF_DIM_day={want} ({date}) in {src.name}")


def window_dates() -> list[dt.date]:
    windows = json.loads(WINDOWS_JSON.read_text())
    days = {h[:10] for w in windows["windows"] for h in w["hours_utc"]}
    return sorted(dt.date.fromisoformat(d) for d in days)


def slice_product(product: str, dates: list[dt.date], bbox_4326: list[float]) -> None:
    name = subdataset(nc_path(product))
    with rasterio.open(name) as src:
        if src.scales[0] not in (1.0, SCALE):
            print(f"WARNING {product}: file scale {src.scales[0]} differs from {SCALE}")
        res_x, res_y = src.res
        w, s, e, n = bbox_4326
        pad_x, pad_y = PAD_CELLS * res_x, PAD_CELLS * res_y
        win = from_bounds(w - pad_x, s - pad_y, e + pad_x, n + pad_y, src.transform).round_offsets().round_lengths()
        transform = src.window_transform(win)
        for date in dates:
            band = band_for_date(src, date)
            raw = src.read(band, window=win).astype(np.float64)
            nodata_mask = (raw == src.nodata) if src.nodata is not None else np.zeros(raw.shape, bool)
            vals = np.where(nodata_mask, np.nan, raw * SCALE).astype(np.float32)
            out = GRIDMET_DIR / f"{product}_{date.isoformat()}.tif"
            profile = {
                "driver": "GTiff",
                "dtype": "float32",
                "count": 1,
                "width": vals.shape[1],
                "height": vals.shape[0],
                "crs": "EPSG:4326",
                "transform": transform,
                "nodata": np.nan,
                "compress": "deflate",
            }
            with rasterio.open(out, "w", **profile) as dst:
                dst.write(vals, 1)
            finite = vals[np.isfinite(vals)]
            print(f"{out.name}: band={band} shape={vals.shape} range=({finite.min():.2f}, {finite.max():.2f}) "
                  f"nodata_cells={int(nodata_mask.sum())}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.gridmet_slice")
    parser.add_argument("--inspect", action="store_true", help="print netCDF structure and exit")
    args = parser.parse_args(argv)
    for product in PRODUCTS:
        if not nc_path(product).exists():
            raise SystemExit(f"missing {nc_path(product)}")
    if args.inspect:
        for product in PRODUCTS:
            inspect(product)
        return 0
    extent = json.loads(EXTENT_JSON.read_text())
    dates = window_dates()
    print(f"dates: {[d.isoformat() for d in dates]}")
    for product in PRODUCTS:
        slice_product(product, dates, extent["grid"]["bbox_4326"])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
