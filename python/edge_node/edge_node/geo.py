"""Bearings on a sphere and from a camera frame."""

from __future__ import annotations

import math

EARTH_RADIUS_M = 6_371_008.8


def initial_bearing(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
    """Initial great-circle bearing from point 1 to point 2, degrees in [0, 360)."""
    p1, p2 = math.radians(lat1), math.radians(lat2)
    dl = math.radians(lon2 - lon1)
    y = math.sin(dl) * math.cos(p2)
    x = math.cos(p1) * math.sin(p2) - math.sin(p1) * math.cos(p2) * math.cos(dl)
    return math.degrees(math.atan2(y, x)) % 360.0


def distance_m(lat1: float, lon1: float, lat2: float, lon2: float) -> float:
    """Haversine distance in metres."""
    p1, p2 = math.radians(lat1), math.radians(lat2)
    dp, dl = p2 - p1, math.radians(lon2 - lon1)
    a = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 2 * EARTH_RADIUS_M * math.asin(math.sqrt(a))


def frame_bearing(x_center: float, heading_deg: float, fov_deg: float) -> float:
    """Bearing of a detection whose box centre sits at `x_center` (0 = left
    edge, 1 = right edge) in a camera pointed at `heading_deg` with horizontal
    field of view `fov_deg`. Uses a pinhole camera, so the edges map to
    heading plus or minus half the field of view."""
    half = math.radians(fov_deg) / 2
    offset = math.atan((2 * x_center - 1) * math.tan(half))
    return (heading_deg + math.degrees(offset)) % 360.0


def in_view(bearing_deg: float, heading_deg: float, fov_deg: float) -> bool:
    """Whether a bearing lies inside the camera's field of view."""
    diff = (bearing_deg - heading_deg + 180.0) % 360.0 - 180.0
    return abs(diff) <= fov_deg / 2
