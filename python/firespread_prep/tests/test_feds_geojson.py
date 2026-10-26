"""Unit test for firespread_prep.feds_geojson.export on two synthetic fires."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import geopandas as gpd
from pyproj import Transformer
from shapely.geometry import box

from firespread_prep import feds_geojson


class ExportTest(unittest.TestCase):
    def test_one_file_per_timestep_of_the_fire_in_epsg4326(self) -> None:
        sq = box(-842_000.0, 1_449_000.0, -841_000.0, 1_450_000.0)
        perims = gpd.GeoDataFrame(
            {
                "fireID": [35131, 35131, 39081],
                "t": ["2022-04-22T12:00:00", "2022-04-23T00:00:00", "2022-04-23T00:00:00"],
                "farea": [1.0, 2.0, 3.0],
            },
            geometry=[sq, sq, sq],
            crs="EPSG:5070",
        )
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            paths = feds_geojson.export(perims, 35131, out)
            self.assertEqual(sorted(p.name for p in paths), ["20220422T1200.geojson", "20220423T0000.geojson"])
            fc = json.loads((out / "20220423T0000.geojson").read_text())
            self.assertEqual(len(fc["features"]), 1, f"features: {fc['features']}")
            feature = fc["features"][0]
            self.assertEqual(feature["properties"], {"t": "2022-04-23T00:00:00", "fireID": 35131, "farea": 2.0})
            ring = feature["geometry"]["coordinates"][0]
            lon, lat = Transformer.from_crs(5070, 4326, always_xy=True).transform(-842_000.0, 1_449_000.0)
            self.assertTrue(
                any(abs(x - lon) < 1e-9 and abs(y - lat) < 1e-9 for x, y in ring),
                f"corner ({lon}, {lat}) not in ring {ring}",
            )
            self.assertTrue(all(-180 <= x <= 180 and -90 <= y <= 90 for x, y in ring), f"ring not lon/lat: {ring}")


if __name__ == "__main__":
    unittest.main()
