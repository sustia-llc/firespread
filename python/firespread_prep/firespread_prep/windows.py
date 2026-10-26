"""Pick the three FEDS timestep pairs that bound the scored spread windows."""

from __future__ import annotations

import json
import sys

import geopandas as gpd
import pandas as pd

from .paths import FEDS_NEWFIREPIX_GPKG, FEDS_PERIMETERS_GPKG, WINDOWS_JSON
from .rasterize import complex_fireids

WINDOW_LABELS = (
    ("w1", "2022-04-22T12:00:00", "2022-04-23T00:00:00"),
    ("w2", "2022-05-01T12:00:00", "2022-05-02T00:00:00"),
    ("w3", "2022-05-10T12:00:00", "2022-05-11T12:00:00"),
)

PERIM_COLUMNS = ("fireID", "t", "farea", "fperim", "n_pixels", "n_newpixels", "meanFRP")


def nearest_t(perims: gpd.GeoDataFrame, label: pd.Timestamp) -> pd.Timestamp:
    ts = perims["t"].drop_duplicates()
    return ts.iloc[(ts - label).abs().argmin()]


def rows_at(perims: gpd.GeoDataFrame, t: pd.Timestamp) -> list[dict]:
    at = perims[perims["t"] == t].sort_values("farea", ascending=False)
    out = []
    for _, row in at.iterrows():
        rec = {}
        for col in PERIM_COLUMNS:
            val = row[col]
            if isinstance(val, pd.Timestamp):
                rec[col] = val.isoformat()
            elif pd.isna(val):
                rec[col] = None
            else:
                rec[col] = float(val) if col in ("farea", "fperim", "meanFRP") else int(val)
        out.append(rec)
    return out


def newfirepix_between(pix: gpd.GeoDataFrame, start: pd.Timestamp, target: pd.Timestamp) -> list[dict]:
    between = pix[(pix["t"] > start) & (pix["t"] <= target)].sort_values(["t", "fireID"])
    out = []
    for _, row in between.iterrows():
        geom = row.geometry
        n_points = len(geom.geoms) if hasattr(geom, "geoms") else 1
        out.append({"fireID": int(row["fireID"]), "t": row["t"].isoformat(), "n_points": n_points})
    return out


def hours_between(start: pd.Timestamp, target: pd.Timestamp) -> list[str]:
    hours = pd.date_range(start.floor("h"), target.ceil("h"), freq="h")
    return [h.strftime("%Y-%m-%dT%H:00:00Z") for h in hours]


def build_windows(perims: gpd.GeoDataFrame, pix: gpd.GeoDataFrame) -> dict:
    windows = []
    for name, label_start, label_target in WINDOW_LABELS:
        start = nearest_t(perims, pd.Timestamp(label_start))
        target = nearest_t(perims, pd.Timestamp(label_target))
        start_rows = rows_at(perims, start)
        target_rows = rows_at(perims, target)
        fireids = [r["fireID"] for r in target_rows]
        windows.append(
            {
                "name": name,
                "label_start": label_start,
                "label_target": label_target,
                "start_t": start.isoformat(),
                "target_t": target.isoformat(),
                "fireids": fireids,
                "fireid_dominant": fireids[0] if fireids else None,
                "start_perimeters": start_rows,
                "target_perimeters": target_rows,
                "newfirepix_between": newfirepix_between(pix, start, target),
                "overpass_times_utc": None,
                "hours_utc": hours_between(start, target),
            }
        )
    pix_columns = [c for c in pix.columns if c != "geometry"]
    return {
        "source_perimeters": str(FEDS_PERIMETERS_GPKG),
        "source_newfirepix": str(FEDS_NEWFIREPIX_GPKG),
        "overpass_note": (
            "FEDS newfirepix rows carry only the 12-hour timestep label t "
            f"(columns: {pix_columns}); no per-detection VIIRS overpass time is present"
        ),
        "t_note": "FEDS t labels are 12-hour bin labels; treated here as UTC instants (unverified)",
        "windows": windows,
    }


def main(argv: list[str] | None = None) -> int:
    perims = gpd.read_file(FEDS_PERIMETERS_GPKG)
    perims["t"] = pd.to_datetime(perims["t"])
    keep = complex_fireids(perims)
    perims = perims[perims["fireID"].astype(int).isin(keep)]
    pix = gpd.read_file(FEDS_NEWFIREPIX_GPKG)
    pix["t"] = pd.to_datetime(pix["t"])
    pix = pix[pix["fireID"].astype(int).isin(keep)]
    result = build_windows(perims, pix)
    WINDOWS_JSON.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    print(f"wrote {WINDOWS_JSON}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
