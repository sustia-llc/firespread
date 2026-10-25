"""Unit test for firespread_prep.align on a synthetic 3 by 3 LFPS-style zip."""

from __future__ import annotations

import tempfile
import unittest
import zipfile
from pathlib import Path

import numpy as np
import rasterio
from rasterio.transform import from_origin

from firespread_prep import align

EXTENT = {
    "grid": {"epsg": 5070, "nx": 3, "ny": 3, "dx": 30.0, "origin_x": -876300.0, "origin_y": 1497480.0}
}
LAYERS = ["LF2022_FBFM40", "LF2022_CBH", "LF2022_CBD"]
SRC_NODATA = -9999

FBFM40 = np.array([[101, 102, 103], [121, 122, SRC_NODATA], [141, 142, 143]], dtype=np.int16)
CBH = np.array([[15, 20, 25], [0, 5, SRC_NODATA], [100, 35, 40]], dtype=np.int16)
CBD = np.array([[12, 4, 0], [25, 7, SRC_NODATA], [30, 1, 9]], dtype=np.int16)


def write_zip(tmp: Path) -> Path:
    tmp.mkdir(parents=True, exist_ok=True)
    tif = tmp / "job.tif"
    grid = EXTENT["grid"]
    transform = from_origin(grid["origin_x"], grid["origin_y"], grid["dx"], grid["dx"])
    with rasterio.open(
        tif,
        "w",
        driver="GTiff",
        dtype="int16",
        count=3,
        width=3,
        height=3,
        crs="EPSG:5070",
        transform=transform,
        nodata=SRC_NODATA,
    ) as dst:
        dst.write(FBFM40, 1)
        dst.write(CBH, 2)
        dst.write(CBD, 3)
    zip_path = tmp / "job.zip"
    with zipfile.ZipFile(zip_path, "w") as zf:
        zf.write(tif, arcname="job.tif")
    tif.unlink()
    return zip_path


class AlignTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.zip_path = write_zip(self.tmp / "in")

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def read(self, name: str) -> np.ndarray:
        with rasterio.open(self.tmp / "grid" / f"{name}.tif") as src:
            self.assertEqual(src.dtypes, ("float32",), src.dtypes)
            self.assertEqual(src.nodata, align.NODATA)
            self.assertEqual(src.crs.to_epsg(), 5070)
            self.assertEqual(src.profile["compress"], "deflate")
            self.assertEqual(src.shape, (3, 3))
            return src.read(1)

    def test_split_decodes_and_preserves_grid(self) -> None:
        written = align.split_zip(self.zip_path, LAYERS, EXTENT, self.tmp / "grid", self.tmp / "extract")
        self.assertEqual([p.name for p in written], ["FBFM40.tif", "CBH.tif", "CBD.tif"])

        fbfm = self.read("FBFM40")
        expected = FBFM40.astype(np.float32)
        expected[1, 2] = align.NODATA
        np.testing.assert_array_equal(fbfm, expected, err_msg=f"FBFM40 nearest warp\n{fbfm}")

        cbh = self.read("CBH")
        expected = CBH.astype(np.float32) / 10.0
        expected[1, 2] = align.NODATA
        np.testing.assert_allclose(cbh, expected, rtol=0, atol=1e-6, err_msg=f"CBH /10 decode\n{cbh}")
        self.assertAlmostEqual(float(cbh[0, 0]), 1.5, places=6, msg=f"CBH[0,0]={cbh[0, 0]}")

        cbd = self.read("CBD")
        expected = CBD.astype(np.float32) / 100.0
        expected[1, 2] = align.NODATA
        np.testing.assert_allclose(cbd, expected, rtol=0, atol=1e-6, err_msg=f"CBD /100 decode\n{cbd}")
        self.assertAlmostEqual(float(cbd[0, 0]), 0.12, places=6, msg=f"CBD[0,0]={cbd[0, 0]}")

    def test_resampling_choice(self) -> None:
        self.assertEqual(align.resampling_for("FBFM40").name, "nearest")
        self.assertEqual(align.resampling_for("FDist").name, "nearest")
        self.assertEqual(align.resampling_for("Asp").name, "nearest")
        self.assertEqual(align.resampling_for("CBH").name, "bilinear")
        self.assertEqual(align.resampling_for("Elev").name, "bilinear")

    def test_band_count_mismatch_fails(self) -> None:
        with self.assertRaises(SystemExit):
            align.split_zip(self.zip_path, LAYERS[:2], EXTENT, self.tmp / "grid", self.tmp / "extract")


if __name__ == "__main__":
    unittest.main()
