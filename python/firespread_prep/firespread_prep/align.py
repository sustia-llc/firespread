"""Split an LFPS product zip into one Float32 GeoTIFF per layer on the master grid."""

from __future__ import annotations

import argparse
import json
import sys
import zipfile
from pathlib import Path

import numpy as np
import rasterio
from rasterio.crs import CRS
from rasterio.enums import Resampling
from rasterio.transform import from_origin
from rasterio.warp import reproject

from .lfps_submit import LAYER_LIST
from .paths import EXTENT_JSON, GRID_DIR, LANDFIRE_DIR

NODATA = -9999.0
CATEGORICAL = {"FBFM40", "FDist", "Asp"}
DECODE_DIVISOR = {"CBH": 10.0, "CH": 10.0, "CBD": 100.0}


def short_name(layer: str) -> str:
    """LF2022_FBFM40 -> FBFM40."""
    return layer.split("_", 1)[1] if "_" in layer else layer


def target_grid(extent: dict) -> tuple[rasterio.Affine, int, int, CRS]:
    grid = extent["grid"]
    transform = from_origin(grid["origin_x"], grid["origin_y"], grid["dx"], grid["dx"])
    return transform, int(grid["nx"]), int(grid["ny"]), CRS.from_epsg(grid["epsg"])


def resampling_for(name: str) -> Resampling:
    return Resampling.nearest if name in CATEGORICAL else Resampling.bilinear


def decode(name: str, arr: np.ndarray, nodata: float = NODATA) -> np.ndarray:
    divisor = DECODE_DIVISOR.get(name)
    if divisor is None:
        return arr
    out = arr.copy()
    valid = arr != nodata
    out[valid] = arr[valid] / divisor
    return out


def warp_band(src: rasterio.DatasetReader, band: int, extent: dict, resampling: Resampling) -> np.ndarray:
    transform, width, height, crs = target_grid(extent)
    dest = np.full((height, width), NODATA, dtype=np.float32)
    reproject(
        source=rasterio.band(src, band),
        destination=dest,
        src_transform=src.transform,
        src_crs=src.crs,
        src_nodata=src.nodata,
        dst_transform=transform,
        dst_crs=crs,
        dst_nodata=NODATA,
        resampling=resampling,
    )
    return dest


def write_layer(path: Path, arr: np.ndarray, extent: dict) -> None:
    transform, width, height, crs = target_grid(extent)
    profile = {
        "driver": "GTiff",
        "dtype": "float32",
        "count": 1,
        "width": width,
        "height": height,
        "crs": crs,
        "transform": transform,
        "nodata": NODATA,
        "compress": "deflate",
        "tiled": True,
        "blockxsize": 256,
        "blockysize": 256,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    with rasterio.open(path, "w", **profile) as dst:
        dst.write(arr.astype(np.float32), 1)


def unzip(zip_path: Path, out_dir: Path) -> Path:
    """Extract the zip into out_dir and return the single .tif inside it."""
    out_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(zip_path) as zf:
        zf.extractall(out_dir)
    tifs = sorted(out_dir.rglob("*.tif"))
    if len(tifs) != 1:
        raise SystemExit(f"expected one .tif in {zip_path}, found {len(tifs)}: {tifs}")
    return tifs[0]


def split_tif(tif: Path, layers: list[str], extent: dict, out_dir: Path) -> list[Path]:
    written = []
    with rasterio.open(tif) as src:
        print(f"{tif}: crs={src.crs} shape={src.shape} bands={src.count} nodata={src.nodata}")
        print(f"band descriptions: {src.descriptions}")
        if src.count != len(layers):
            raise SystemExit(f"band count {src.count} != layer count {len(layers)}: {layers}")
        for band, layer in enumerate(layers, start=1):
            name = short_name(layer)
            resampling = resampling_for(name)
            arr = decode(name, warp_band(src, band, extent, resampling))
            valid = arr[arr != NODATA]
            rng = (float(valid.min()), float(valid.max())) if valid.size else None
            out = out_dir / f"{name}.tif"
            write_layer(out, arr, extent)
            print(f"{name}: band={band} resampling={resampling.name} range={rng} -> {out}")
            written.append(out)
    return written


def split_zip(zip_path: Path, layers: list[str], extent: dict, out_dir: Path, extract_dir: Path) -> list[Path]:
    tif = unzip(zip_path, extract_dir)
    return split_tif(tif, layers, extent, out_dir)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.align")
    parser.add_argument("zip", type=Path, help="LFPS product zip")
    parser.add_argument("--layers", default=LAYER_LIST, help="semicolon list in band order")
    parser.add_argument("--out", type=Path, default=GRID_DIR)
    args = parser.parse_args(argv)
    extent = json.loads(EXTENT_JSON.read_text())
    layers = args.layers.split(";")
    extract_dir = LANDFIRE_DIR / args.zip.stem
    written = split_zip(args.zip, layers, extent, args.out, extract_dir)
    print(f"wrote {len(written)} layer(s) to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
