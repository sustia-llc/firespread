"""Unit test for firebench_case.bundle.build on a synthetic one-window run directory."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import geopandas as gpd
import h5py
import numpy as np
import rasterio
from rasterio.transform import from_origin

from firebench.standardize.tools import validate_h5_referenced_files

from firebench_case import bundle

N = 20
DX = 30.0
ORIGIN_X = -842_000.0
ORIGIN_Y = 1_450_000.0
EPSG = 5070
START = slice(8, 12)
TARGET = slice(5, 15)
FORECAST = slice(6, 13)


def write_tif(path: Path, array: np.ndarray, nodata: float) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with rasterio.open(
        path,
        "w",
        driver="GTiff",
        height=N,
        width=N,
        count=1,
        dtype=array.dtype,
        crs=f"EPSG:{EPSG}",
        transform=from_origin(ORIGIN_X, ORIGIN_Y, DX, DX),
        nodata=nodata,
    ) as dst:
        dst.write(array, 1)


def block(s: slice, dtype, inside, outside) -> np.ndarray:
    a = np.full((N, N), outside, dtype=dtype)
    a[s, s] = inside
    return a


def make_case(root: Path) -> tuple[Path, Path]:
    data = root / "data"
    runs = root / "runs"
    (data).mkdir(parents=True)
    (data / "windows.json").write_text(
        json.dumps({"windows": [{"name": "w1", "label_start": "2022-04-22T12:00:00", "label_target": "2022-04-23T00:00:00"}]})
    )
    write_tif(data / "obs" / "feds" / "20220422T1200.tif", block(START, np.uint8, 1, 0), 255)
    write_tif(data / "obs" / "feds" / "20220423T0000.tif", block(TARGET, np.uint8, 1, 0), 255)
    run = runs / "w1-seed7-m2"
    p = block(FORECAST, np.float32, 0.9, 0.1)
    p[0, 0] = -9999.0
    write_tif(run / "burn_probability_h12.tif", p, -9999.0)
    write_tif(run / "burn_probability_h6.tif", block(START, np.float32, 0.9, 0.0), -9999.0)
    (run / "manifest.json").write_text(
        json.dumps(
            {
                "run_id": "w1-seed7-m2",
                "window": {"name": "w1"},
                "grid": {
                    "nx": N,
                    "ny": N,
                    "dx": DX,
                    "dy": DX,
                    "origin_x": ORIGIN_X,
                    "origin_y": ORIGIN_Y,
                    "epsg": EPSG,
                    "nodata": -9999.0,
                },
                "horizons_h": [6, 12],
                "outputs": {
                    "burn_probability_h6": "burn_probability_h6.tif",
                    "burn_probability_h12": "burn_probability_h12.tif",
                },
            }
        )
    )
    return runs, data


def kml_area_m2(path: Path) -> float:
    return float(gpd.read_file(path, driver="KML").to_crs(EPSG).area.sum())


def cells_m2(s: slice) -> float:
    return (s.stop - s.start) ** 2 * DX * DX


class BundleTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.runs, self.data = make_case(root)
        self.out = root / "out"
        self.result = bundle.build(self.runs, self.data, self.out)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_three_validators_pass_on_every_file(self) -> None:
        start = "HermitsPeak_2022-04-22T12:00+00:00"
        target = "HermitsPeak_2022-04-23T00:00+00:00"
        expected = {"observations": [start, target], "firespread": [target], "persistence": [target]}
        self.assertEqual({k: v["entries"] for k, v in self.result.items()}, expected)
        for name, info in self.result.items():
            self.assertIsNone(info["validate_h5_std"], name)
            self.assertEqual(info["validate_h5_referenced_files"], (True, None), name)
            self.assertEqual(info["validate_h5_requirement"], (True, None), name)

    def test_polygons_are_the_burned_cells_not_their_complement(self) -> None:
        # The contour at 0.5 between cell centers lies on the cell edges, less the
        # four cut corners of a square block: area within 1% + 2 cells of the block's.
        cases = [
            ("observations_HermitsPeak_20220423T0000.kml", TARGET),
            ("observations_HermitsPeak_20220422T1200.kml", START),
            ("firespread_HermitsPeak_20220423T0000.kml", FORECAST),
            ("persistence_HermitsPeak_20220423T0000.kml", START),
        ]
        for name, s in cases:
            area = kml_area_m2(self.out / name)
            expected = cells_m2(s)
            self.assertLess(abs(area - expected), 0.01 * expected + 2 * DX * DX, f"{name}: {area} m2 vs {expected} m2")

    def test_spatial_2d_holds_every_cog_and_the_feds_target(self) -> None:
        with h5py.File(self.result["firespread"]["path"], "r") as h5:
            groups = sorted(h5["spatial_2d"].keys())
            self.assertEqual(groups, ["HermitsPeak_2022-04-22T18:00+00:00", "HermitsPeak_2022-04-23T00:00+00:00"])
            d = h5["spatial_2d/HermitsPeak_2022-04-23T00:00+00:00/burn_probability"]
            self.assertEqual(d.attrs["units"], "dimensionless")
            self.assertAlmostEqual(float(d[7, 7]), 0.9, places=6)
        with h5py.File(self.result["observations"]["path"], "r") as h5:
            d = h5["spatial_2d/HermitsPeak_FEDS_2022-04-23T00:00+00:00/burned_mask"]
            self.assertEqual(int(d[:].sum()), (TARGET.stop - TARGET.start) ** 2)

    def test_changed_kml_byte_fails_referenced_files(self) -> None:
        kml = self.out / "firespread_HermitsPeak_20220423T0000.kml"
        raw = bytearray(kml.read_bytes())
        raw[-2] ^= 0x01
        kml.write_bytes(bytes(raw))
        with h5py.File(self.result["firespread"]["path"], "r") as h5:
            ok, issue = validate_h5_referenced_files(h5)
        self.assertFalse(ok, f"validator passed a changed KML: {issue}")
        self.assertIn("SHA-256 mismatch", issue)


if __name__ == "__main__":
    unittest.main()
