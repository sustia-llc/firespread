"""Export FEDS perimeters of one fire as GeoJSON in EPSG:4326, one file per timestep.

Reads ``data/feds/hermits_perimeters.gpkg``, keeps the rows of ``--fireid``
(default 35131), and writes ``data/obs/feds_geojson/<YYYYmmddTHHMM>.geojson``
per timestep ``t``: a FeatureCollection of that fire's perimeter rows with
properties ``t`` (ISO 8601), ``fireID`` and ``farea``.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import geopandas as gpd
import pandas as pd

from .paths import DATA_DIR, FEDS_PERIMETERS_GPKG

OUT_DIR = DATA_DIR / "obs" / "feds_geojson"
FIREID = 35131


def export(perims: gpd.GeoDataFrame, fireid: int, out_dir: Path) -> list[Path]:
    """Writes one GeoJSON per timestep of ``fireid`` in ``perims`` to ``out_dir``; returns the paths."""
    rows = perims[perims["fireID"].astype(int) == fireid].copy()
    rows["t"] = pd.to_datetime(rows["t"])
    rows = rows.to_crs(4326)
    out_dir.mkdir(parents=True, exist_ok=True)
    written = []
    for t, group in rows.groupby("t"):
        features = []
        for _, row in group.iterrows():
            if row.geometry is None or row.geometry.is_empty:
                continue
            features.append(
                {
                    "type": "Feature",
                    "properties": {"t": t.isoformat(), "fireID": int(row["fireID"]), "farea": float(row["farea"])},
                    "geometry": json.loads(gpd.GeoSeries([row.geometry]).to_json())["features"][0]["geometry"],
                }
            )
        path = out_dir / f"{t.strftime('%Y%m%dT%H%M')}.geojson"
        path.write_text(json.dumps({"type": "FeatureCollection", "features": features}) + "\n")
        written.append(path)
    return written


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.feds_geojson")
    parser.add_argument("--perimeters", type=Path, default=FEDS_PERIMETERS_GPKG)
    parser.add_argument("--fireid", type=int, default=FIREID)
    parser.add_argument("--out", type=Path, default=OUT_DIR)
    args = parser.parse_args(argv)
    paths = export(gpd.read_file(args.perimeters), args.fireid, args.out)
    print(f"fire {args.fireid}: {len(paths)} timesteps written to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
