"""Hermits Peak case and firespread forecasts in the FireBench standard format 1.0.

``build(runs, data, out)`` reads every run directory under ``runs`` (a directory with a
``manifest.json``), the window of each run from ``data/windows.json`` and the FEDS masks
``data/obs/feds/<label>.tif``, and writes into ``out``:

- ``observations.h5``: one ``/polygons/HermitsPeak_<time>`` entry per FEDS mask at each
  window's start and target label, and the FEDS target masks in ``/spatial_2d``.
- ``firespread.h5``: one polygon entry per window at its target label, from the run's
  ``burn_probability_h<N>.tif`` with N the target label minus the start label in hours,
  and every ``burn_probability_h<N>.tif`` of every run in ``/spatial_2d``.
- ``persistence.h5``: one polygon entry per window at its target label, from the FEDS mask
  at the start label.
- ``uniform_growth.h5``: one polygon entry per window at its target label, from the run's
  ``uniform_growth_h<N>.tif`` written by ``firespread score``. When a run lacks that file,
  ``uniform_growth.h5`` is not written and one line naming the runs to re-score is printed.
- one KML file per polygon entry, in ``out`` next to the ``.h5`` files.

Masks are the cells equal to 1. A polygon is ``firebench.tools.array_to_geopolygons`` at
iso value 0.5 of ``1 - mask`` or ``1 - p`` (nodata as p = 0) on the cell-center longitude
and latitude in EPSG:4326, reprojected from the EPSG code of the run's manifest; that
function returns the region where the field is at most the iso value, so the polygon is
the mask, or p >= 0.5. Labels are UTC; entry names and ``time`` attributes are
``YYYY-MM-DDTHH:MM+00:00``. A ``/spatial_2d`` group holds the arrays of
``firebench.standardize.import_tif`` with projection EPSG:4326, whose
``position_lat``/``position_lon`` are the cell upper-left corners; groups on the same grid
share one pair of position datasets by hard link.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path

import h5py
import hdf5plugin
import numpy as np
import rasterio
from pyproj import Transformer

from firebench.standardize import GEOPOLYGONS, SPATIAL_2D, new_std_file
from firebench.standardize.tools import (
    import_tif,
    validate_h5_referenced_files,
    validate_h5_requirement,
    validate_h5_std,
)
from firebench.tools import array_to_geopolygons

CASE = "HermitsPeak"
ISO_VALUE = 0.5
KML_CRS = "EPSG:4326"
ENTRY_ATTRS = ["rel_path", "time"]
COMPRESSION_LVL = 3
OBSERVATIONS = "observations"
FIRESPREAD = "firespread"
PERSISTENCE = "persistence"
UNIFORM_GROWTH = "uniform_growth"


@dataclass(frozen=True)
class Run:
    run_dir: Path
    run_id: str
    window: str
    epsg: int
    grid: dict
    start: datetime
    target: datetime
    horizon_h: int
    outputs: dict[str, str]

    @property
    def target_cog(self) -> Path:
        return self.run_dir / self.outputs[f"burn_probability_h{self.horizon_h}"]

    @property
    def uniform_growth_tif(self) -> Path:
        return self.run_dir / f"{UNIFORM_GROWTH}_h{self.horizon_h}.tif"


def utc_label(label: str) -> datetime:
    t = datetime.fromisoformat(label.removesuffix("Z"))
    if t.tzinfo is None:
        t = t.replace(tzinfo=timezone.utc)
    return t.astimezone(timezone.utc)


def iso_time(t: datetime) -> str:
    return t.strftime("%Y-%m-%dT%H:%M+00:00")


def feds_label(t: datetime) -> str:
    return t.strftime("%Y%m%dT%H%M")


def entry_name(t: datetime) -> str:
    return f"{CASE}_{iso_time(t)}"


def load_runs(runs: Path, data: Path) -> list[Run]:
    windows = {w["name"]: w for w in json.loads((data / "windows.json").read_text())["windows"]}
    out = []
    for manifest_path in sorted(runs.glob("*/manifest.json")):
        m = json.loads(manifest_path.read_text())
        name = m["window"]["name"]
        if name not in windows:
            raise ValueError(f"window {name!r} of {manifest_path} is not in {data / 'windows.json'}")
        start = utc_label(windows[name]["label_start"])
        target = utc_label(windows[name]["label_target"])
        horizon = (target - start) / timedelta(hours=1)
        if horizon != int(horizon) or int(horizon) not in m["horizons_h"]:
            raise ValueError(f"{manifest_path}: target is {horizon} h after start, not in {m['horizons_h']}")
        out.append(
            Run(
                run_dir=manifest_path.parent,
                run_id=m["run_id"],
                window=name,
                epsg=int(m["grid"]["epsg"]),
                grid=m["grid"],
                start=start,
                target=target,
                horizon_h=int(horizon),
                outputs=m["outputs"],
            )
        )
    if not out:
        raise ValueError(f"no run directory with a manifest.json under {runs}")
    names = [r.window for r in out]
    if len(set(names)) != len(names):
        raise ValueError(f"more than one run per window under {runs}: {names}")
    return out


def read_band(path: Path, run: Run) -> tuple[np.ndarray, rasterio.Affine, float | None]:
    g = run.grid
    with rasterio.open(path) as src:
        if src.crs is None or src.crs.to_epsg() != run.epsg:
            raise ValueError(f"{path}: CRS {src.crs} is not EPSG:{run.epsg} of {run.run_id}")
        expected = rasterio.Affine(g["dx"], 0.0, g["origin_x"], 0.0, -g["dy"], g["origin_y"])
        if src.transform != expected or src.shape != (g["ny"], g["nx"]):
            raise ValueError(f"{path} is not on the grid of {run.run_id}")
        return src.read(1), src.transform, src.nodata


def cell_centers_lonlat(transform: rasterio.Affine, shape: tuple[int, int], epsg: int):
    rows, cols = shape
    jj, ii = np.meshgrid(np.arange(cols) + 0.5, np.arange(rows) + 0.5)
    x = transform.c + transform.a * jj + transform.b * ii
    y = transform.f + transform.d * jj + transform.e * ii
    lon, lat = Transformer.from_crs(epsg, KML_CRS, always_xy=True).transform(x, y)
    return np.asarray(lon), np.asarray(lat)


def mask_field(path: Path, run: Run) -> tuple[np.ndarray, rasterio.Affine]:
    data, transform, _ = read_band(path, run)
    return np.where(data == 1, 0.0, 1.0), transform


def probability_field(path: Path, run: Run) -> tuple[np.ndarray, rasterio.Affine]:
    data, transform, nodata = read_band(path, run)
    p = data.astype(np.float64)
    invalid = ~np.isfinite(p) if nodata is None else (~np.isfinite(p) | (data == nodata))
    p[invalid] = 0.0
    return 1.0 - p, transform


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def write_polygon_entry(
    h5: h5py.File, out: Path, prefix: str, t: datetime, field: np.ndarray, transform, run: Run
) -> Path:
    lon, lat = cell_centers_lonlat(transform, field.shape, run.epsg)
    gdf = array_to_geopolygons(field, lon, lat, ISO_VALUE, original_crs=KML_CRS)
    kml = out / f"{prefix}_{CASE}_{feds_label(t)}.kml"
    kml.unlink(missing_ok=True)
    gdf.to_file(kml, driver="KML")
    dset = h5.create_dataset(f"{GEOPOLYGONS}/{entry_name(t)}", data=h5py.Empty("u1"))
    dset.attrs["rel_path"] = kml.name
    dset.attrs["time"] = iso_time(t)
    dset.attrs["file_size_bytes"] = np.int64(kml.stat().st_size)
    dset.attrs["sha256"] = sha256_of(kml)
    dset.attrs["source_run"] = run.run_id
    return kml


def write_raster_group(
    h5: h5py.File,
    positions: dict,
    group_name: str,
    tif: Path,
    variable: str,
    units: str,
    attrs: dict,
) -> None:
    lat, lon, data, crs, nodata = import_tif(str(tif), projection=KML_CRS)
    g = h5.create_group(f"/{SPATIAL_2D}/{group_name}")
    g.attrs["crs"] = str(crs)
    g.attrs["data_source"] = str(tif.name)
    g.attrs["position_reference"] = "cell_upper_left_corner"
    for key, value in attrs.items():
        g.attrs[key] = value
    with rasterio.open(tif) as src:
        key = (tuple(src.transform), src.shape, str(src.crs))
    if key in positions:
        g["position_lat"], g["position_lon"] = positions[key]
    else:
        for name, values in (("position_lat", lat), ("position_lon", lon)):
            d = g.create_dataset(name, data=values, dtype=np.float64, **hdf5plugin.Zstd(clevel=COMPRESSION_LVL))
            d.attrs["units"] = "degrees"
        positions[key] = (g["position_lat"], g["position_lon"])
    d = g.create_dataset(variable, data=data, dtype=data.dtype, **hdf5plugin.Zstd(clevel=COMPRESSION_LVL))
    d.attrs["units"] = units
    if nodata is not None:
        d.attrs["_FillValue"] = np.array(nodata, dtype=data.dtype)


def validate(path: Path, entries: list[str]) -> dict:
    with h5py.File(path, "r") as h5:
        std = validate_h5_std(h5)
        referenced = validate_h5_referenced_files(h5)
        required = validate_h5_requirement(h5, {f"/{GEOPOLYGONS}/{e}": ENTRY_ATTRS for e in entries})
    return {"validate_h5_std": std, "validate_h5_referenced_files": referenced, "validate_h5_requirement": required}


def build(runs: Path, data: Path, out: Path, authors: str = "firespread", overwrite: bool = False) -> dict:
    """Writes the bundle into ``out`` and returns, per ``.h5`` file, its path, entry names
    and the three FireBench validators' return values."""
    run_list = load_runs(runs, data)
    out.mkdir(parents=True, exist_ok=True)
    feds = lambda t: data / "obs" / "feds" / f"{feds_label(t)}.tif"  # noqa: E731
    result = {}

    path = out / f"{OBSERVATIONS}.h5"
    h5 = new_std_file(str(path), authors, overwrite=overwrite)
    try:
        h5.attrs["description"] = f"{CASE} FEDS perimeters at the start and target label of each window"
        positions: dict = {}
        entries = []
        for run in run_list:
            for t in (run.start, run.target):
                if entry_name(t) in entries:
                    continue
                field, transform = mask_field(feds(t), run)
                write_polygon_entry(h5, out, OBSERVATIONS, t, field, transform, run)
                entries.append(entry_name(t))
            write_raster_group(
                h5,
                positions,
                f"{CASE}_FEDS_{iso_time(run.target)}",
                feds(run.target),
                "burned_mask",
                "dimensionless",
                {"time": iso_time(run.target), "window": run.window},
            )
    finally:
        h5.close()
    result[OBSERVATIONS] = {"path": path, "entries": entries}

    path = out / f"{FIRESPREAD}.h5"
    h5 = new_std_file(str(path), authors, overwrite=overwrite)
    try:
        h5.attrs["description"] = f"firespread burn probability on the {CASE} windows; polygons at p >= 0.5"
        positions = {}
        entries = []
        for run in run_list:
            field, transform = probability_field(run.target_cog, run)
            write_polygon_entry(h5, out, FIRESPREAD, run.target, field, transform, run)
            entries.append(entry_name(run.target))
            for key, rel in sorted(run.outputs.items()):
                if not key.startswith("burn_probability_h"):
                    continue
                horizon = int(key.removeprefix("burn_probability_h"))
                valid = run.start + timedelta(hours=horizon)
                write_raster_group(
                    h5,
                    positions,
                    f"{CASE}_{iso_time(valid)}",
                    run.run_dir / rel,
                    "burn_probability",
                    "dimensionless",
                    {"time": iso_time(valid), "run_id": run.run_id, "horizon_h": np.int64(horizon)},
                )
    finally:
        h5.close()
    result[FIRESPREAD] = {"path": path, "entries": entries}

    path = out / f"{PERSISTENCE}.h5"
    h5 = new_std_file(str(path), authors, overwrite=overwrite)
    try:
        h5.attrs["description"] = f"persistence baseline on the {CASE} windows: the FEDS mask at the start label"
        entries = []
        for run in run_list:
            field, transform = mask_field(feds(run.start), run)
            write_polygon_entry(h5, out, PERSISTENCE, run.target, field, transform, run)
            entries.append(entry_name(run.target))
    finally:
        h5.close()
    result[PERSISTENCE] = {"path": path, "entries": entries}

    missing = [run.run_id for run in run_list if not run.uniform_growth_tif.is_file()]
    if missing:
        print(f"{UNIFORM_GROWTH}.h5 skipped: no {UNIFORM_GROWTH}_h<N>.tif in run(s) {', '.join(missing)}; re-score them")
    else:
        path = out / f"{UNIFORM_GROWTH}.h5"
        h5 = new_std_file(str(path), authors, overwrite=overwrite)
        try:
            h5.attrs["description"] = (
                f"uniform growth baseline on the {CASE} windows: the scorecard's uniform_growth mask"
            )
            entries = []
            for run in run_list:
                field, transform = mask_field(run.uniform_growth_tif, run)
                write_polygon_entry(h5, out, UNIFORM_GROWTH, run.target, field, transform, run)
                entries.append(entry_name(run.target))
        finally:
            h5.close()
        result[UNIFORM_GROWTH] = {"path": path, "entries": entries}

    for info in result.values():
        info.update(validate(info["path"], info["entries"]))
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--runs", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--authors", default="firespread")
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()
    result = build(args.runs, args.data, args.out, args.authors, args.overwrite)
    for name, info in result.items():
        print(f"{name}: {info['path']}")
        for e in info["entries"]:
            print(f"  /{GEOPOLYGONS}/{e}")
        for v in ("validate_h5_std", "validate_h5_referenced_files", "validate_h5_requirement"):
            print(f"  {v}: {info[v]}")
    for f in sorted(args.out.iterdir()):
        print(f"{f.stat().st_size:>12}  {f.name}")


if __name__ == "__main__":
    main()
