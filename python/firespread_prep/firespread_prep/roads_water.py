"""Download OSM roads and NHD water for the grid and rasterize them as u8 GeoTIFF.

Roads: OSM ``highway`` ways from the Overpass API inside the grid's WGS84
bounding box, written to ``data/vector/osm_roads.geojson`` and rasterized to
``data/grid/roads.tif`` with ``all_touched=True`` so a line forms a connected
cell chain. Codes: 0 none, 1 motorway or trunk, 2 primary, 3 secondary,
4 tertiary, 5 unclassified, residential or service, 6 any other highway value.
Where classes overlap, the lowest code wins.

Water: NHD MapServer layers 12 (Waterbody, large scale) and 9 (Area, large
scale) from hydro.nationalmap.gov, written to ``data/vector/nhd_*.geojson``
and rasterized to ``data/grid/water.tif`` (cell centre rule). Codes: 0 none,
1 waterbody, 2 area. Where they overlap, the waterbody wins.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import geopandas as gpd
import numpy as np
from rasterio.features import rasterize
from shapely.geometry import LineString

from .align import target_grid
from .paths import DATA_DIR, EXTENT_JSON, GRID_DIR
from .rasterize import write_mask

VECTOR_DIR = DATA_DIR / "vector"
OVERPASS_URLS = ("https://overpass-api.de/api/interpreter", "https://overpass.kumi.systems/api/interpreter")
ATTEMPTS_PER_URL = 2
NHD_URL = "https://hydro.nationalmap.gov/arcgis/rest/services/nhd/MapServer/{layer}/query"
NHD_LAYERS = {12: ("waterbody", 1), 9: ("area", 2)}
NHD_PAGE = 1000
ROAD_CODES = {
    "motorway": 1, "motorway_link": 1, "trunk": 1, "trunk_link": 1,
    "primary": 2, "primary_link": 2,
    "secondary": 3, "secondary_link": 3,
    "tertiary": 4, "tertiary_link": 4,
    "unclassified": 5, "residential": 5, "service": 5,
}
OTHER_ROAD = 6
TIMEOUT_S = 300


def fetch(url: str, data: bytes | None = None) -> bytes:
    req = urllib.request.Request(url, data=data, headers={"User-Agent": "firespread-prep"})
    with urllib.request.urlopen(req, timeout=TIMEOUT_S) as resp:
        return resp.read()


def download_roads(bbox_4326: list[float], out: Path) -> gpd.GeoDataFrame:
    w, s, e, n = bbox_4326
    query = f'[out:json][timeout:{TIMEOUT_S}];way["highway"]({s},{w},{n},{e});out geom;'
    body = urllib.parse.urlencode({"data": query}).encode()
    raw = None
    errors = []
    for url in OVERPASS_URLS:
        for attempt in range(ATTEMPTS_PER_URL):
            try:
                raw = json.loads(fetch(url, body))
                break
            except (urllib.error.URLError, TimeoutError) as err:
                errors.append(f"{url} attempt {attempt + 1}: {err}")
                print(errors[-1])
        if raw is not None:
            break
    if raw is None:
        raise SystemExit(f"Overpass failed on every endpoint: {errors}")
    rows = []
    for el in raw.get("elements", []):
        coords = [(p["lon"], p["lat"]) for p in el.get("geometry", [])]
        if len(coords) < 2:
            continue
        hw = el.get("tags", {}).get("highway", "")
        rows.append({"osm_id": el["id"], "highway": hw, "code": ROAD_CODES.get(hw, OTHER_ROAD), "geometry": LineString(coords)})
    frame = gpd.GeoDataFrame(rows, crs="EPSG:4326")
    out.parent.mkdir(parents=True, exist_ok=True)
    frame.to_file(out, driver="GeoJSON")
    return frame


def download_nhd(bbox_4326: list[float], layer: int, out: Path) -> gpd.GeoDataFrame:
    w, s, e, n = bbox_4326
    params = {
        "geometry": f"{w},{s},{e},{n}",
        "geometryType": "esriGeometryEnvelope",
        "inSR": "4326",
        "spatialRel": "esriSpatialRelIntersects",
        "outFields": "*",
        "outSR": "4326",
        "f": "geojson",
    }
    features = []
    offset = 0
    while True:
        page = {**params, "resultOffset": str(offset), "resultRecordCount": str(NHD_PAGE)}
        data = json.loads(fetch(NHD_URL.format(layer=layer) + "?" + urllib.parse.urlencode(page)))
        batch = data.get("features", [])
        features.extend(batch)
        more = data.get("exceededTransferLimit") or data.get("properties", {}).get("exceededTransferLimit")
        if not more or not batch:
            break
        offset += len(batch)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({"type": "FeatureCollection", "features": features}))
    if not features:
        return gpd.GeoDataFrame(geometry=[], crs="EPSG:4326")
    return gpd.read_file(out)


def burn_codes(frames: list[tuple[gpd.GeoSeries, int]], extent: dict, all_touched: bool) -> np.ndarray:
    """Burn each (geometries, code) pair; where pairs overlap, the lowest nonzero code wins."""
    transform, width, height, crs = target_grid(extent)
    out = np.zeros((height, width), dtype=np.uint8)
    for geoms, code in sorted(frames, key=lambda pair: -pair[1]):
        shapes = [(g, code) for g in geoms.to_crs(crs) if g is not None and not g.is_empty]
        if shapes:
            rasterize(shapes, out=out, transform=transform, all_touched=all_touched, dtype="uint8")
    return out


def summary(name: str, arr: np.ndarray, dx: float) -> dict:
    codes, counts = np.unique(arr, return_counts=True)
    return {"layer": name, "cells_by_code": {int(c): int(k) for c, k in zip(codes, counts)}, "dx": dx}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m firespread_prep.roads_water")
    parser.add_argument("--skip-download", action="store_true")
    args = parser.parse_args(argv)
    extent = json.loads(EXTENT_JSON.read_text())
    bbox = extent["grid"]["bbox_4326"]
    dx = extent["grid"]["dx"]

    roads_path = VECTOR_DIR / "osm_roads.geojson"
    roads = gpd.read_file(roads_path) if args.skip_download else download_roads(bbox, roads_path)
    print(f"osm roads: {len(roads)} ways, by highway: {roads['highway'].value_counts().head(12).to_dict()}")
    road_frames = [(roads[roads["code"] == c].geometry, int(c)) for c in roads["code"].unique()]
    roads_arr = burn_codes(road_frames, extent, all_touched=True)
    write_mask(GRID_DIR / "roads.tif", roads_arr, extent)

    water_frames = []
    for layer, (name, code) in NHD_LAYERS.items():
        path = VECTOR_DIR / f"nhd_{name}.geojson"
        frame = gpd.read_file(path) if args.skip_download else download_nhd(bbox, layer, path)
        print(f"nhd {name} (layer {layer}): {len(frame)} features")
        water_frames.append((frame.geometry, code))
    water_arr = burn_codes(water_frames, extent, all_touched=False)
    write_mask(GRID_DIR / "water.tif", water_arr, extent)

    report = [summary("roads", roads_arr, dx), summary("water", water_arr, dx)]
    (VECTOR_DIR / "roads_water_summary.json").write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps(report, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
