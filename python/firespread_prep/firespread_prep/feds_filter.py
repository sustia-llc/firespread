"""Filter the FEDS 2022 large-fire perimeters and new-fire pixels to the master grid window."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import geopandas as gpd
import pandas as pd
import pyogrio
from pyproj import Transformer
from shapely.geometry import box

from .paths import (
    EXTENT_JSON,
    FEDS_FIREIDS_JSON,
    FEDS_NEWFIREPIX,
    FEDS_NEWFIREPIX_GPKG,
    FEDS_NEWFIREPIX_JSON,
    FEDS_PERIMETER,
    FEDS_PERIMETERS_GPKG,
)

BBOX_PAD_DEG = 0.1
DOMINANCE_AFTER = pd.Timestamp("2022-04-22T00:00:00")


def filter_bbox_4326() -> list[float]:
    extent = json.loads(EXTENT_JSON.read_text())
    w, s, e, n = extent["grid"]["bbox_4326"]
    return [w - BBOX_PAD_DEG, s - BBOX_PAD_DEG, e + BBOX_PAD_DEG, n + BBOX_PAD_DEG]


def bbox_in_layer_crs(bbox_4326: list[float], layer_crs: str) -> tuple[float, float, float, float]:
    """Reproject a WGS84 bounding box into the layer CRS, densifying the edges."""
    if layer_crs in ("EPSG:4326", "OGC:CRS84"):
        return tuple(bbox_4326)
    transformer = Transformer.from_crs("EPSG:4326", layer_crs, always_xy=True)
    poly = box(*bbox_4326).segmentize(0.01)
    xs, ys = poly.exterior.coords.xy
    px, py = transformer.transform(xs, ys)
    return (min(px), min(py), max(px), max(py))


def read_filtered(path: Path, bbox_4326: list[float]) -> gpd.GeoDataFrame:
    info = pyogrio.read_info(path)
    bbox = bbox_in_layer_crs(bbox_4326, info["crs"])
    print(f"{path.name}: crs={info['crs']} bbox_filter={bbox}")
    frame = pyogrio.read_dataframe(path, bbox=bbox)
    frame["t"] = pd.to_datetime(frame["t"])
    return frame.sort_values(["fireID", "t"]).reset_index(drop=True)


def fireid_table(frame: gpd.GeoDataFrame) -> list[dict]:
    rows = []
    for fireid, group in frame.groupby("fireID"):
        row = {
            "fireID": float(fireid),
            "t_first": group["t"].min().isoformat(),
            "t_last": group["t"].max().isoformat(),
            "n_timesteps": int(len(group)),
        }
        if "farea" in group:
            row["max_farea_km2"] = float(group["farea"].max())
        if "n_pixels" in group:
            row["max_n_pixels"] = int(group["n_pixels"].max())
        rows.append(row)
    key = "max_farea_km2" if rows and "max_farea_km2" in rows[0] else "n_timesteps"
    return sorted(rows, key=lambda r: r[key], reverse=True)


def dominance_report(perims: gpd.GeoDataFrame) -> dict:
    """Share of the summed farea held by the largest fireID at each step after DOMINANCE_AFTER."""
    after = perims[perims["t"] >= DOMINANCE_AFTER]
    if after.empty:
        return {"after": DOMINANCE_AFTER.isoformat(), "steps": []}
    steps = []
    for t, group in after.groupby("t"):
        total = float(group["farea"].sum())
        top = group.loc[group["farea"].idxmax()]
        steps.append(
            {
                "t": t.isoformat(),
                "n_fireids": int(group["fireID"].nunique()),
                "top_fireID": float(top["fireID"]),
                "top_farea_km2": float(top["farea"]),
                "top_share": float(top["farea"] / total) if total > 0 else None,
            }
        )
    return {"after": DOMINANCE_AFTER.isoformat(), "steps": steps}


def write_gpkg(frame: gpd.GeoDataFrame, path: Path) -> None:
    out = frame.copy()
    out["t"] = out["t"].dt.strftime("%Y-%m-%dT%H:%M:%S")
    for col in ("t_st", "t_ed"):
        if col in out and pd.api.types.is_datetime64_any_dtype(out[col]):
            out[col] = out[col].dt.strftime("%Y-%m-%dT%H:%M:%S")
    out.to_file(path, driver="GPKG", engine="pyogrio")


def main(argv: list[str] | None = None) -> int:
    bbox_4326 = filter_bbox_4326()
    print(f"bbox_4326 (grid window + {BBOX_PAD_DEG} deg): {bbox_4326}")

    perims = read_filtered(FEDS_PERIMETER, bbox_4326)
    perim_table = fireid_table(perims)
    dominance = dominance_report(perims)
    print(f"perimeter rows: {len(perims)}, fireIDs: {len(perim_table)}")
    print(pd.DataFrame(perim_table).to_string())
    print(pd.DataFrame(dominance["steps"]).to_string())
    FEDS_FIREIDS_JSON.write_text(
        json.dumps(
            {"bbox_filter_4326": bbox_4326, "fireids": perim_table, "dominance": dominance},
            indent=2,
        )
        + "\n"
    )
    write_gpkg(perims, FEDS_PERIMETERS_GPKG)
    print(f"wrote {FEDS_FIREIDS_JSON} and {FEDS_PERIMETERS_GPKG}")

    pix = read_filtered(FEDS_NEWFIREPIX, bbox_4326)
    pix_table = fireid_table(pix)
    print(f"newfirepix rows: {len(pix)}, fireIDs: {len(pix_table)}")
    print(f"newfirepix columns: {list(pix.columns)}")
    print(f"newfirepix geometry types: {pix.geom_type.value_counts().to_dict()}")
    print(pd.DataFrame(pix_table).to_string())
    FEDS_NEWFIREPIX_JSON.write_text(
        json.dumps({"bbox_filter_4326": bbox_4326, "fireids": pix_table}, indent=2) + "\n"
    )
    write_gpkg(pix, FEDS_NEWFIREPIX_GPKG)
    print(f"wrote {FEDS_NEWFIREPIX_JSON} and {FEDS_NEWFIREPIX_GPKG}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
