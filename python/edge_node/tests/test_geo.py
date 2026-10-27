import math

from edge_node.debounce import Persistence
from edge_node.geo import distance_m, frame_bearing, in_view, initial_bearing


def test_bearing_cardinal():
    assert math.isclose(initial_bearing(35.0, -106.0, 35.1, -106.0), 0.0, abs_tol=1e-9)
    assert math.isclose(initial_bearing(35.0, -106.0, 35.0, -105.9), 90.0, abs_tol=0.1)
    assert math.isclose(initial_bearing(35.0, -106.0, 34.9, -106.0), 180.0, abs_tol=1e-9)
    assert math.isclose(initial_bearing(35.0, -106.0, 35.0, -106.1), 270.0, abs_tol=0.1)


def test_distance_one_degree_latitude():
    assert math.isclose(distance_m(35.0, -106.0, 36.0, -106.0), 111_195, rel_tol=1e-3)


def test_frame_bearing_edges_and_wrap():
    assert math.isclose(frame_bearing(0.5, 90.0, 60.0), 90.0)
    assert math.isclose(frame_bearing(0.0, 90.0, 60.0), 60.0)
    assert math.isclose(frame_bearing(1.0, 90.0, 60.0), 120.0)
    assert math.isclose(frame_bearing(0.0, 10.0, 60.0), 340.0)


def test_in_view_wraps_north():
    assert in_view(355.0, 10.0, 60.0)
    assert not in_view(300.0, 10.0, 60.0)


def test_persistence():
    p = Persistence(3, 5)
    seq = [True, False, True, False, True, True]
    assert [p.update(h) for h in seq] == [False, False, False, False, True, True]
