"""Tests for firespread_prep.weather_align: GRIB2 section 3 parsing, wind rotation, index.json round trip."""

from __future__ import annotations

import json
import math
import tempfile
import unittest
from pathlib import Path

import numpy as np
import rasterio
from pyproj import CRS, Transformer

from firespread_prep import weather_align as wa
from firespread_prep.paths import EXTENT_JSON, GRIDMET_DIR, HRRR_DIR, WINDOWS_JSON

HOUR = "2022-04-22T12:00:00Z"
U_FRAGMENT = wa.fragment(HRRR_DIR, HOUR, "UGRD10m")
HRRR_LCC = CRS.from_proj4(
    "+proj=lcc +lat_0=38.5 +lon_0=-97.5 +lat_1=38.5 +lat_2=38.5 +R=6371229 +x_0=0 +y_0=0 +units=m +no_defs"
)
MASTER = CRS.from_epsg(5070)


def tiny_extent(n: int = 4) -> dict:
    grid = json.loads(EXTENT_JSON.read_text())["grid"]
    return {"grid": {"epsg": 5070, "nx": n, "ny": n, "dx": 30.0, "origin_x": grid["origin_x"], "origin_y": grid["origin_y"]}}


def master_point(lon: float, lat: float) -> tuple[float, float]:
    return Transformer.from_crs(HRRR_LCC.geodetic_crs, MASTER, always_xy=True).transform(lon, lat)


def expected_x_axis_angle(lon: float, lat: float, step: float = 0.5) -> float:
    """Angle in degrees, in EPSG:5070, of the segment from (lon, lat) to a point step metres along HRRR +x."""
    to_src = Transformer.from_crs(HRRR_LCC.geodetic_crs, HRRR_LCC, always_xy=True)
    to_dst = Transformer.from_crs(HRRR_LCC, MASTER, always_xy=True)
    sx, sy = to_src.transform(lon, lat)
    x0, y0 = to_dst.transform(sx, sy)
    x1, y1 = to_dst.transform(sx + step, sy)
    return math.degrees(math.atan2(y1 - y0, x1 - x0))


@unittest.skipUnless(U_FRAGMENT.exists(), f"missing {U_FRAGMENT}")
class Section3Test(unittest.TestCase):
    def test_parse_real_fragment_matches_rasterio(self) -> None:
        grid = wa.parse_section3(U_FRAGMENT)
        self.assertEqual(grid.template, 30, f"template={grid.template}")
        with rasterio.open(U_FRAGMENT) as src:
            self.assertEqual((grid.nx, grid.ny), (src.width, src.height),
                             f"parsed Nx,Ny={grid.nx},{grid.ny} rasterio={src.width},{src.height}")
            wa.check_against_raster(grid, src, U_FRAGMENT.name)


class RotationTest(unittest.TestCase):
    def basis_at(self, lon: float, lat: float) -> tuple[np.ndarray, ...]:
        x, y = master_point(lon, lat)
        return wa.rotation_basis(np.array([[x]]), np.array([[y]]), HRRR_LCC, MASTER, grid_relative=True)

    def test_source_x_unit_vector_angle_at_central_meridian(self) -> None:
        lon, lat = -97.5, 36.0
        expected = expected_x_axis_angle(lon, lat)
        self.assertGreater(abs(expected), 0.1, f"degenerate case: expected angle {expected} deg")
        um, vm = wa.rotate(np.array([[1.0]]), np.array([[0.0]]), self.basis_at(lon, lat))
        got = math.degrees(math.atan2(float(vm[0, 0]), float(um[0, 0])))
        self.assertAlmostEqual(got, expected, delta=1e-5, msg=f"rotated +x angle {got} deg, expected {expected} deg")

    def test_magnitude_preserved(self) -> None:
        rng = np.random.default_rng(7)
        for lon, lat in ((-97.5, 36.0), (-105.4, 35.85), (-120.0, 45.0)):
            basis = self.basis_at(lon, lat)
            u, v = rng.uniform(-1.0, 1.0, size=(2, 1, 1))
            um, vm = wa.rotate(u, v, basis)
            before = math.hypot(float(u[0, 0]), float(v[0, 0]))
            after = math.hypot(float(um[0, 0]), float(vm[0, 0]))
            self.assertAlmostEqual(after, before, delta=1e-6,
                                   msg=f"|(u,v)| {before} -> {after} at lon={lon} lat={lat}")


@unittest.skipUnless(U_FRAGMENT.exists() and (GRIDMET_DIR / "fm100_2022-04-22.tif").exists(), "missing HRRR or gridMET inputs")
class IndexRoundTripTest(unittest.TestCase):
    def test_one_hour(self) -> None:
        extent = tiny_extent()
        window = next(w for w in json.loads(WINDOWS_JSON.read_text())["windows"] if w["name"] == "w1")
        with tempfile.TemporaryDirectory() as tmp:
            weather = Path(tmp) / "weather"
            bases = wa.BasisCache(extent)
            entry = wa.align_window(window, HRRR_DIR, GRIDMET_DIR, weather, extent, bases, hours=[HOUR])
            index_path = weather / "index.json"
            written = wa.write_index(index_path, wa.build_index(extent, [entry], bases))
            back = wa.read_index(index_path)
            self.assertEqual(back, written)

            (w1,) = back["windows"]
            self.assertEqual(w1["gridmet_date"], "2022-04-22")
            self.assertEqual([h["hour_utc"] for h in w1["hours"]], [HOUR])
            self.assertEqual(w1["hours"][0]["files"]["u10"]["path"], "w1/20220422T12/u10.tif")
            records = list(w1["gridmet"].items()) + list(w1["hours"][0]["files"].items())
            self.assertEqual({n for n, _ in records}, {"fm100", "fm1000", "u10", "v10", "t2m", "rh2m", "tcc"})
            for name, rec in records:
                path = wa.resolve(index_path, rec["path"])
                self.assertTrue(path.exists(), f"{name}: {rec['path']} -> {path} missing")
                with rasterio.open(path) as src:
                    self.assertEqual((src.count, src.dtypes[0], src.nodata, src.shape), (1, "float32", wa.NODATA, (4, 4)))
                    arr = src.read(1)
                valid = arr[arr != wa.NODATA]
                self.assertEqual(valid.size, 16, f"{name}: {16 - valid.size} nodata cells")
                self.assertEqual((rec["min"], rec["max"]), (float(valid.min()), float(valid.max())),
                                 f"{name}: index min/max {rec['min']},{rec['max']} vs file {valid.min()},{valid.max()}")


if __name__ == "__main__":
    unittest.main()
