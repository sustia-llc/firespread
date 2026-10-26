"""Unit tests for firespread_prep.hillshade on synthetic planes and the ESRI worked example."""

from __future__ import annotations

import math
import tempfile
import unittest
from pathlib import Path

import numpy as np
import rasterio
from rasterio.transform import from_origin

from firespread_prep import hillshade as hs

DX = 30.0


def plane(gx: float, gy_north: float, n: int = 7) -> np.ndarray:
    """z = gx * east + gy_north * north on an n by n north-up grid (row 0 north)."""
    rows, cols = np.mgrid[0:n, 0:n].astype(np.float64)
    east = cols * DX
    north = -rows * DX
    return 2000.0 + gx * east + gy_north * north


def analytic(slope_rad: float, facing_deg: float, azimuth_deg: float = 315.0, altitude_deg: float = 45.0) -> float:
    """Lambertian value of a plane whose downslope faces compass bearing `facing_deg`."""
    zenith = math.radians(90.0 - altitude_deg)
    return 255.0 * (
        math.cos(zenith) * math.cos(slope_rad)
        + math.sin(zenith) * math.sin(slope_rad) * math.cos(math.radians(azimuth_deg - facing_deg))
    )


class HillshadeTest(unittest.TestCase):
    def test_plane_facing_the_sun(self) -> None:
        # Rises toward the south-east, so its downslope faces 315 deg.
        g = 0.4
        z = plane(g, -g)
        slope = math.atan(g * math.sqrt(2.0))
        expected = analytic(slope, 315.0)
        self.assertAlmostEqual(expected, 255.0 * math.cos(math.radians(45.0) - slope), places=9)
        got = hs.hillshade(z, DX)
        interior = got[1:-1, 1:-1]
        np.testing.assert_allclose(interior, expected, rtol=0, atol=1e-9, err_msg=f"expected {expected}, got\n{interior}")
        self.assertTrue(np.isnan(got[0]).all() and np.isnan(got[:, -1]).all(), "edge ring is not nodata")

    def test_plane_facing_away(self) -> None:
        # Rises toward the north-west, so its downslope faces 135 deg.
        g = 0.4
        z = plane(-g, g)
        slope = math.atan(g * math.sqrt(2.0))
        expected = analytic(slope, 135.0)
        got = hs.hillshade(z, DX)[1:-1, 1:-1]
        np.testing.assert_allclose(got, expected, rtol=0, atol=1e-9, err_msg=f"expected {expected}, got\n{got}")

    def test_plane_facing_north(self) -> None:
        # Rises toward the south, downslope faces 0 deg; azimuth differs by 45 deg.
        g = 0.3
        z = plane(0.0, -g)
        expected = analytic(math.atan(g), 0.0)
        got = hs.hillshade(z, DX)[1:-1, 1:-1]
        np.testing.assert_allclose(got, expected, rtol=0, atol=1e-9, err_msg=f"expected {expected}, got\n{got}")

    def test_steep_lee_slope_clamps_to_zero(self) -> None:
        g = 3.0
        z = plane(-g, g)
        got = hs.hillshade(z, DX)[1:-1, 1:-1]
        self.assertLess(analytic(math.atan(g * math.sqrt(2.0)), 135.0), 0.0)
        np.testing.assert_array_equal(got, 0.0)

    def test_esri_worked_example(self) -> None:
        # ESRI "How Hillshade works": cell size 5, the neighbours below, integer output 154.
        z = np.array([[2450.0, 2461.0, 2483.0], [2452.0, 2470.0, 2483.0], [2447.0, 2455.0, 2477.0]])
        got = hs.hillshade(z, 5.0)
        self.assertEqual(int(hs.to_uint8(got)[1, 1]), 154, f"float value {got[1, 1]}")

    def test_nodata_window_is_nodata(self) -> None:
        z = plane(0.4, -0.4)
        z[3, 3] = hs.SRC_NODATA
        got = hs.hillshade(z, DX)
        self.assertTrue(np.isnan(got[2:5, 2:5]).all(), f"\n{got}")
        self.assertFalse(np.isnan(got[1, 1]))
        q = hs.to_uint8(got)
        self.assertEqual(int(q[3, 3]), hs.OUT_NODATA)
        self.assertGreaterEqual(int(q[1, 1]), 1)

    def test_write_keeps_the_grid(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "Elev.tif"
            dst = Path(tmp) / "basemap" / "hillshade.tif"
            z = plane(0.4, -0.4).astype(np.float32)
            transform = from_origin(-876180.0, 1499820.0, DX, DX)
            with rasterio.open(
                src, "w", driver="GTiff", dtype="float32", count=1, width=7, height=7,
                crs="EPSG:5070", transform=transform, nodata=hs.SRC_NODATA,
            ) as f:
                f.write(z, 1)
            hs.write_hillshade(src, dst)
            with rasterio.open(dst) as f:
                self.assertEqual(f.crs.to_epsg(), 5070)
                self.assertEqual(f.transform, transform)
                self.assertEqual(f.shape, (7, 7))
                self.assertEqual(f.dtypes, ("uint8",))
                self.assertEqual(f.nodata, hs.OUT_NODATA)
                out = f.read(1)
            expected = round(255.0 * math.cos(math.radians(45.0) - math.atan(0.4 * math.sqrt(2.0))))
            np.testing.assert_array_equal(out[1:-1, 1:-1], expected)
            np.testing.assert_array_equal(out[0], hs.OUT_NODATA)


if __name__ == "__main__":
    unittest.main()
