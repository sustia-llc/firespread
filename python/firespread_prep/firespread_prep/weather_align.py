"""Align hourly HRRR 10 m wind, 2 m temperature, 2 m RH and total cloud cover, and per-window gridMET fm100 and fm1000, onto the master grid.

For each window in `data/windows.json` and each hour in its `hours_utc`, reads
the HRRR fragments `data/hrrr/hrrr.YYYYMMDD.tHHz.f00.<tag>.grib2` with GDAL's
GRIB driver under `GRIB_NORMALIZE_UNITS=NO`, warps them bilinearly onto the
`data/extent.json` grid and writes `data/weather/<window>/<YYYYMMDDTHH>/`
`u10.tif` and `v10.tif` (m/s), `t2m.tif` (K), `rh2m.tif` (%) and `tcc.tif`
(TCDC entire atmosphere, %). `u10` is the
component along the master grid's +x (column) axis and `v10` along its +y axis
(toward row 0). The source wind axes are read from GRIB2 section 3: template
3.30 octet 47, flag table 3.3 bit 5, set means u and v are along the source
grid's +x and +y, clear means east and north. Each master cell's rotation maps
a 1 m step along each source axis through pyproj into EPSG:5070 and uses the
normalised steps as the columns of a 2x2 matrix applied to the warped (u, v).
Writes `data/weather/<window>/fm100.tif` and `fm1000.tif` (%) from the gridMET
slice dated by the UTC date of the window's first hour, and
`data/weather/index.json` with each window's ordered hours, file paths
(relative to the index), gridMET date and per-output min and max over valid
cells. Every output is single-band Float32 with nodata -9999.

GRIB2 octet positions:
section 0, https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/grib2_sect0.shtml ;
section 3 header, https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/grib2_sect3.shtml ;
template 3.30, https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/grib2_temp3-30.shtml ;
flag table 3.3, https://www.nco.ncep.noaa.gov/pmb/docs/grib2/grib2_doc/grib2_table3-3.shtml ;
flag bits numbered 1 (most significant) to 8,
https://confluence.ecmwf.int/spaces/UDOC/pages/212440350/What+are+Code+and+Flag+tables+-+ecCodes+GRIB+and+BUFR+FAQ .
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import sys
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import rasterio
from pyproj import CRS as ProjCRS
from pyproj import Transformer
from rasterio.enums import Resampling
from rasterio.warp import reproject

from .align import NODATA, target_grid, write_layer
from .hrrr_subset import hour_key
from .paths import EXTENT_JSON, GRIDMET_DIR, HRRR_DIR, WEATHER_DIR, WINDOWS_JSON

# output name -> (HRRR fragment tag, GRIB_UNIT tag GDAL reports with GRIB_NORMALIZE_UNITS=NO)
HRRR_VARS = {
    "u10": ("UGRD10m", "[m/s]"),
    "v10": ("VGRD10m", "[m/s]"),
    "t2m": ("TMP2m", "[K]"),
    "rh2m": ("RH2m", "[%]"),
    "tcc": ("TCDC", "[%]"),
}
GRIDMET_PRODUCTS = ("fm100", "fm1000")
UNITS = {"u10": "m s-1", "v10": "m s-1", "t2m": "K", "rh2m": "%", "tcc": "%", "fm100": "%", "fm1000": "%"}

LAMBERT_TEMPLATE = 30
UV_GRID_RELATIVE = 0x08  # flag table 3.3 bit 5
FLAGS_RESERVED = 0xC7  # flag table 3.3 bits 1-2 and 6-8
STEP_M = 1.0


@dataclass(frozen=True)
class LambertGrid:
    """Grid definition template 3.30 fields; angles in degrees, lengths in metres."""

    template: int
    nx: int
    ny: int
    la1: float
    lo1: float
    flags: int
    lad: float
    lov: float
    dx: float
    dy: float
    scan_mode: int
    latin1: float
    latin2: float

    @property
    def uv_grid_relative(self) -> bool:
        return bool(self.flags & UV_GRID_RELATIVE)


def _uint(sec: bytes, first: int, last: int) -> int:
    """Unsigned big-endian integer over 1-based inclusive octets."""
    return int.from_bytes(sec[first - 1 : last], "big")


def _sint(sec: bytes, first: int, last: int) -> int:
    """GRIB2 sign-magnitude integer: the most significant bit is the sign."""
    raw = _uint(sec, first, last)
    sign_bit = 1 << (8 * (last - first + 1) - 1)
    return -(raw & ~sign_bit) if raw & sign_bit else raw


def section3(data: bytes) -> bytes:
    """Section 3 of the first GRIB2 message in data, found by walking section lengths."""
    if data[:4] != b"GRIB" or data[7] != 2:
        raise ValueError(f"not a GRIB2 message: head={data[:8]!r}")
    pos = 16
    while pos + 5 <= len(data) and data[pos : pos + 4] != b"7777":
        length = _uint(data, pos + 1, pos + 4)
        number = data[pos + 4]
        if length < 5:
            raise ValueError(f"section {number} at byte {pos} has length {length}")
        if number == 3:
            return data[pos : pos + length]
        pos += length
    raise ValueError("no section 3 before end of message")


def parse_section3(path: Path) -> LambertGrid:
    """Parse grid definition template 3.30 from the first message of a GRIB2 file."""
    sec = section3(path.read_bytes())
    template = _uint(sec, 13, 14)
    if template != LAMBERT_TEMPLATE:
        raise ValueError(f"{path.name}: grid definition template {template}, expected {LAMBERT_TEMPLATE}")
    return LambertGrid(
        template=template,
        nx=_uint(sec, 31, 34),
        ny=_uint(sec, 35, 38),
        la1=_sint(sec, 39, 42) * 1e-6,
        lo1=_uint(sec, 43, 46) * 1e-6,
        flags=sec[47 - 1],
        lad=_sint(sec, 48, 51) * 1e-6,
        lov=_uint(sec, 52, 55) * 1e-6,
        dx=_uint(sec, 56, 59) * 1e-3,
        dy=_uint(sec, 60, 63) * 1e-3,
        scan_mode=sec[65 - 1],
        latin1=_sint(sec, 66, 69) * 1e-6,
        latin2=_sint(sec, 70, 73) * 1e-6,
    )


def check_against_raster(grid: LambertGrid, src: rasterio.DatasetReader, name: str) -> None:
    """Raise ValueError unless the parsed template agrees with GDAL's view of the same file."""
    problems = []
    if (grid.nx, grid.ny) != (src.width, src.height):
        problems.append(f"Nx,Ny={grid.nx},{grid.ny} vs rasterio {src.width},{src.height}")
    if abs(grid.dx - abs(src.transform.a)) > 1e-3 or abs(grid.dy - abs(src.transform.e)) > 1e-3:
        problems.append(f"Dx,Dy={grid.dx},{grid.dy} vs rasterio res {src.res}")
    params = ProjCRS.from_wkt(src.crs.to_wkt()).to_dict()
    lov = (grid.lov + 180.0) % 360.0 - 180.0
    if abs(lov - params.get("lon_0", np.nan)) > 1e-6:
        problems.append(f"LoV={lov} vs lon_0={params.get('lon_0')}")
    if abs(grid.lad - params.get("lat_0", np.nan)) > 1e-6:
        problems.append(f"LaD={grid.lad} vs lat_0={params.get('lat_0')}")
    if grid.flags & FLAGS_RESERVED:
        problems.append(f"flags=0x{grid.flags:02x} sets reserved bits (mask 0x{FLAGS_RESERVED:02x})")
    if problems:
        raise ValueError(f"{name}: section 3 disagrees with rasterio: " + "; ".join(problems))


def master_centres(extent: dict) -> tuple[np.ndarray, np.ndarray]:
    """EPSG:5070 x and y of every master-grid cell centre, shape (ny, nx)."""
    transform, width, height, _ = target_grid(extent)
    cols = np.arange(width) + 0.5
    rows = np.arange(height) + 0.5
    xs = transform.c + cols * transform.a
    ys = transform.f + rows * transform.e
    return np.meshgrid(xs, ys)


def _unit(x: np.ndarray, y: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    n = np.hypot(x, y)
    return x / n, y / n


def _radii(geodetic: ProjCRS, lat_deg: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Meridional (M) and prime-vertical (N) radii of curvature, metres, on geodetic's ellipsoid."""
    ell = geodetic.ellipsoid
    a, b = ell.semi_major_metre, ell.semi_minor_metre
    e2 = 1.0 - (b / a) ** 2
    s = np.sin(np.radians(lat_deg))
    w = np.sqrt(1.0 - e2 * s * s)
    return a * (1.0 - e2) / w**3, a / w


def east_north_in_crs(
    lon: np.ndarray, lat: np.ndarray, crs: ProjCRS
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """Unit vectors (ex, ey, nx, ny) in crs axes of local east and north at (lon, lat) on crs's own datum."""
    geo = crs.geodetic_crs
    fwd = Transformer.from_crs(geo, crs, always_xy=True)
    m, n = _radii(geo, lat)
    dlon = np.degrees(STEP_M / (n * np.cos(np.radians(lat))))
    dlat = np.degrees(STEP_M / m)
    x0, y0 = fwd.transform(lon, lat)
    x1, y1 = fwd.transform(lon + dlon, lat)
    x2, y2 = fwd.transform(lon, lat + dlat)
    ex, ey = _unit(np.asarray(x1) - x0, np.asarray(y1) - y0)
    nx, ny = _unit(np.asarray(x2) - x0, np.asarray(y2) - y0)
    return ex, ey, nx, ny


def grid_axes_in_east_north(
    lon: np.ndarray, lat: np.ndarray, crs: ProjCRS
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """Unit (east, north) components of crs's +x axis (ae, an) and +y axis (be, bn) at (lon, lat)."""
    geo = crs.geodetic_crs
    fwd = Transformer.from_crs(geo, crs, always_xy=True)
    inv = Transformer.from_crs(crs, geo, always_xy=True)
    m, n = _radii(geo, lat)
    coslat = np.cos(np.radians(lat))
    sx, sy = fwd.transform(lon, lat)
    lon1, lat1 = inv.transform(np.asarray(sx) + STEP_M, sy)
    lon2, lat2 = inv.transform(sx, np.asarray(sy) + STEP_M)
    ae, an = _unit(np.radians(np.asarray(lon1) - lon) * n * coslat, np.radians(np.asarray(lat1) - lat) * m)
    be, bn = _unit(np.radians(np.asarray(lon2) - lon) * n * coslat, np.radians(np.asarray(lat2) - lat) * m)
    return ae, an, be, bn


def rotation_basis(
    xs: np.ndarray, ys: np.ndarray, src_crs: ProjCRS, dst_crs: ProjCRS, grid_relative: bool
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """Unit vectors (ax, ay, bx, by) in dst_crs axes of the source u axis (a) and v axis (b) at dst points (xs, ys).

    With grid_relative the source axes are src_crs +x and +y; otherwise east and north. Each
    point's longitude and latitude on dst_crs's datum are taken as the same numbers on
    src_crs's datum. Source axes are resolved into local east and north with 1 m steps on
    src_crs's own datum, and east and north into dst_crs axes with 1 m steps on dst_crs's
    datum, so each step stays within one projection and its own ellipsoid or sphere.
    """
    to_geo = Transformer.from_crs(dst_crs, dst_crs.geodetic_crs, always_xy=True)
    lon, lat = (np.asarray(c) for c in to_geo.transform(xs, ys))
    ex, ey, nx, ny = east_north_in_crs(lon, lat, dst_crs)
    if not grid_relative:
        return ex, ey, nx, ny
    ae, an, be, bn = grid_axes_in_east_north(lon, lat, src_crs)
    ax, ay = _unit(ae * ex + an * nx, ae * ey + an * ny)
    bx, by = _unit(be * ex + bn * nx, be * ey + bn * ny)
    return ax, ay, bx, by


def rotate(u: np.ndarray, v: np.ndarray, basis: tuple[np.ndarray, ...]) -> tuple[np.ndarray, np.ndarray]:
    """(u, v) on the source axes -> components on the master +x and +y axes; NODATA stays NODATA."""
    ax, ay, bx, by = basis
    um = u * ax + v * bx
    vm = u * ay + v * by
    bad = (u == NODATA) | (v == NODATA)
    um = np.where(bad, NODATA, um).astype(np.float32)
    vm = np.where(bad, NODATA, vm).astype(np.float32)
    return um, vm


def basis_angle_deg(basis: tuple[np.ndarray, ...]) -> np.ndarray:
    """Counter-clockwise angle from master +x to the source u axis, degrees."""
    ax, ay, _, _ = basis
    return np.degrees(np.arctan2(ay, ax))


def warp(src: rasterio.DatasetReader, extent: dict, src_nodata: float | None) -> np.ndarray:
    transform, width, height, crs = target_grid(extent)
    dest = np.full((height, width), NODATA, dtype=np.float32)
    reproject(
        source=rasterio.band(src, 1),
        destination=dest,
        src_transform=src.transform,
        src_crs=src.crs,
        src_nodata=src_nodata,
        dst_transform=transform,
        dst_crs=crs,
        dst_nodata=NODATA,
        resampling=Resampling.bilinear,
    )
    return dest


def stats(arr: np.ndarray) -> dict:
    valid = arr[arr != NODATA]
    if valid.size == 0:
        return {"min": None, "max": None, "nodata_cells": int(arr.size)}
    return {"min": float(valid.min()), "max": float(valid.max()), "nodata_cells": int(arr.size - valid.size)}


def hour_dir_name(hour_iso: str) -> str:
    """'2022-04-22T13:00:00Z' -> '20220422T13'."""
    ymd, hh = hour_key(hour_iso)
    return f"{ymd}T{hh}"


def fragment(hrrr_dir: Path, hour_iso: str, tag: str) -> Path:
    ymd, hh = hour_key(hour_iso)
    return hrrr_dir / f"hrrr.{ymd}.t{hh}z.f00.{tag}.grib2"


class BasisCache:
    """Rotation bases on one master grid, keyed by source CRS WKT and the grid-relative flag."""

    def __init__(self, extent: dict) -> None:
        self.xs, self.ys = master_centres(extent)
        self.dst_crs = ProjCRS.from_epsg(int(extent["grid"]["epsg"]))
        self._cache: dict[tuple[str, bool], tuple[np.ndarray, ...]] = {}

    def get(self, src_wkt: str, grid_relative: bool) -> tuple[np.ndarray, ...]:
        key = (src_wkt, grid_relative)
        if key not in self._cache:
            self._cache[key] = rotation_basis(self.xs, self.ys, ProjCRS.from_wkt(src_wkt), self.dst_crs, grid_relative)
        return self._cache[key]

    def angle_range(self) -> dict | None:
        """Min and max of basis_angle_deg over every cached basis, or None when empty."""
        angles = [basis_angle_deg(b) for b in self._cache.values()]
        if not angles:
            return None
        return {"min": float(min(a.min() for a in angles)), "max": float(max(a.max() for a in angles))}


def align_hour(hour_iso: str, hrrr_dir: Path, out_dir: Path, extent: dict, bases: BasisCache) -> dict:
    """Write every HRRR_VARS output for one hour into out_dir; return the index entry with absolute paths."""
    arrays: dict[str, np.ndarray] = {}
    flags: dict[str, int] = {}
    src_wkt = None
    with rasterio.Env(GRIB_NORMALIZE_UNITS="NO"):
        for name, (tag, unit) in HRRR_VARS.items():
            path = fragment(hrrr_dir, hour_iso, tag)
            with rasterio.open(path) as src:
                got_unit = src.tags(1).get("GRIB_UNIT")
                if got_unit != unit:
                    raise ValueError(f"{path.name}: GRIB_UNIT={got_unit!r}, expected {unit!r}")
                if name in ("u10", "v10"):
                    grid = parse_section3(path)
                    check_against_raster(grid, src, path.name)
                    flags[name] = grid.flags
                    wkt = src.crs.to_wkt()
                    if src_wkt is not None and wkt != src_wkt:
                        raise ValueError(f"{path.name}: CRS differs from the u10 fragment")
                    src_wkt = wkt
                arrays[name] = warp(src, extent, src.nodata)
    if flags["u10"] != flags["v10"]:
        raise ValueError(f"{hour_iso}: u10 flags 0x{flags['u10']:02x} != v10 flags 0x{flags['v10']:02x}")
    grid_relative = bool(flags["u10"] & UV_GRID_RELATIVE)
    basis = bases.get(src_wkt, grid_relative)
    arrays["u10"], arrays["v10"] = rotate(arrays["u10"], arrays["v10"], basis)

    files = {}
    for name in HRRR_VARS:
        path = out_dir / f"{name}.tif"
        write_layer(path, arrays[name], extent)
        files[name] = {"path": path, **stats(arrays[name])}
    return {"hour_utc": hour_iso, "uv_flags": flags["u10"], "uv_grid_relative": grid_relative, "files": files}


def align_gridmet(date: dt.date, gridmet_dir: Path, out_dir: Path, extent: dict) -> dict:
    files = {}
    for product in GRIDMET_PRODUCTS:
        src_path = gridmet_dir / f"{product}_{date.isoformat()}.tif"
        with rasterio.open(src_path) as src:
            arr = warp(src, extent, src.nodata)
        path = out_dir / f"{product}.tif"
        write_layer(path, arr, extent)
        files[product] = {"path": path, "source": src_path, **stats(arr)}
    return files


def relativise(entry: dict | list, root: Path) -> dict | list:
    """Copy of entry with every Path made a POSIX string relative to root."""
    if isinstance(entry, dict):
        return {k: relativise(v, root) for k, v in entry.items()}
    if isinstance(entry, list):
        return [relativise(v, root) for v in entry]
    if isinstance(entry, Path):
        return entry.resolve().relative_to(root.resolve(), walk_up=True).as_posix()
    return entry


def write_index(index_path: Path, index: dict) -> dict:
    """Write index with paths relative to index_path's directory; return the JSON-ready dict."""
    out = relativise(index, index_path.parent)
    index_path.parent.mkdir(parents=True, exist_ok=True)
    index_path.write_text(json.dumps(out, indent=2) + "\n")
    return out


def read_index(index_path: Path) -> dict:
    return json.loads(index_path.read_text())


def resolve(index_path: Path, rel: str) -> Path:
    return (index_path.parent / rel).resolve()


def align_window(window: dict, hrrr_dir: Path, gridmet_dir: Path, weather_dir: Path, extent: dict,
                 bases: BasisCache, hours: list[str] | None = None) -> dict:
    name = window["name"]
    hours = window["hours_utc"] if hours is None else hours
    date = dt.date.fromisoformat(window["hours_utc"][0][:10])
    entry = {
        "name": name,
        "gridmet_date": date.isoformat(),
        "gridmet": align_gridmet(date, gridmet_dir, weather_dir / name, extent),
        "hours": [],
    }
    for hour in hours:
        entry["hours"].append(align_hour(hour, hrrr_dir, weather_dir / name / hour_dir_name(hour), extent, bases))
    return entry


def build_index(extent: dict, windows: list[dict], bases: BasisCache) -> dict:
    return {
        "grid": {k: extent["grid"][k] for k in ("epsg", "nx", "ny", "dx", "origin_x", "origin_y")},
        "nodata": NODATA,
        "units": UNITS,
        "wind_axes": "u10 along master +x (increasing column), v10 along master +y (toward row 0)",
        "uv_rotation_deg": bases.angle_range(),
        "windows": windows,
    }


def print_entry(entry: dict) -> None:
    for product, f in entry["gridmet"].items():
        print(f"  {product}: date={entry['gridmet_date']} range=({f['min']}, {f['max']}) -> {f['path']}")
    for hour in entry["hours"]:
        ranges = " ".join(
            f"{n}=({f['min']:.2f},{f['max']:.2f})" if f["min"] is not None else f"{n}=(all nodata)"
            for n, f in hour["files"].items()
        )
        print(f"  {hour['hour_utc']} flags=0x{hour['uv_flags']:02x} grid_relative={hour['uv_grid_relative']} {ranges}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.weather_align")
    parser.add_argument("--windows", nargs="*", help="window names to align (default: all)")
    parser.add_argument("--out", type=Path, default=WEATHER_DIR)
    args = parser.parse_args(argv)
    extent = json.loads(EXTENT_JSON.read_text())
    windows = json.loads(WINDOWS_JSON.read_text())["windows"]
    if args.windows:
        windows = [w for w in windows if w["name"] in args.windows]
    bases = BasisCache(extent)
    entries = []
    for window in windows:
        print(f"{window['name']}: {len(window['hours_utc'])} hour(s)")
        entry = align_window(window, HRRR_DIR, GRIDMET_DIR, args.out, extent, bases)
        print_entry(entry)
        entries.append(entry)
    index = build_index(extent, entries, bases)
    print(f"uv rotation over grid (deg, master +x to source u axis): {index['uv_rotation_deg']}")
    index_path = args.out / "index.json"
    write_index(index_path, index)
    print(f"wrote {index_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
