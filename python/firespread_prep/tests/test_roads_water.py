"""Unit test for firespread_prep.roads_water.burn_codes on a synthetic 5 by 5 grid."""

from __future__ import annotations

import unittest

import geopandas as gpd
import numpy as np
from shapely.geometry import LineString

from firespread_prep import roads_water

EXTENT = {"grid": {"epsg": 5070, "nx": 5, "ny": 5, "dx": 30.0, "origin_x": 0.0, "origin_y": 150.0}}


class BurnCodesTest(unittest.TestCase):
    def test_lowest_code_wins_where_classes_overlap(self) -> None:
        # Both lines run along the middle row (y = 75, cell row 2); the track (6) also covers row 2 only.
        track = gpd.GeoSeries([LineString([(0, 75), (150, 75)])], crs="EPSG:5070")
        motorway = gpd.GeoSeries([LineString([(45, 75), (105, 75)])], crs="EPSG:5070")
        # Given lowest code first, so burn order alone would let the track overwrite the motorway.
        arr = roads_water.burn_codes([(motorway, 1), (track, 6)], EXTENT, all_touched=True)
        expected_row = np.array([6, 1, 1, 1, 6], dtype=np.uint8)
        np.testing.assert_array_equal(arr[2], expected_row, err_msg=f"row 2 = {arr[2]}")
        self.assertEqual(int(arr[[0, 1, 3, 4]].sum()), 0, f"off-row cells burned: {arr}")


if __name__ == "__main__":
    unittest.main()
