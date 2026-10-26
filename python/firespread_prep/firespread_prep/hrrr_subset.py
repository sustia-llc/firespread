"""Fetch six HRRR surface analysis records per window hour by HTTP Range from the NOAA S3 bucket."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import requests

from .paths import HRRR_DIR, WINDOWS_JSON

BASE = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com"

# (output tag, inventory VAR, inventory LEVEL, record number per the NCEP wrfsfcf00 inventory)
RECORDS = (
    ("TMP2m", "TMP", "2 m above ground", 71),
    ("RH2m", "RH", "2 m above ground", 75),
    ("UGRD10m", "UGRD", "10 m above ground", 77),
    ("VGRD10m", "VGRD", "10 m above ground", 78),
    ("GUST", "GUST", "surface", 9),
    ("TCDC", "TCDC", "entire atmosphere", 116),
)


def hour_key(hour_iso: str) -> tuple[str, str]:
    """'2022-04-22T13:00:00Z' -> ('20220422', '13')."""
    return hour_iso[0:4] + hour_iso[5:7] + hour_iso[8:10], hour_iso[11:13]


def grib_url(ymd: str, hh: str) -> str:
    return f"{BASE}/hrrr.{ymd}/conus/hrrr.t{hh}z.wrfsfcf00.grib2"


def parse_idx(text: str) -> list[tuple[int, int, str, str]]:
    """Return (record number, byte offset, VAR, LEVEL) per inventory line."""
    out = []
    for line in text.splitlines():
        parts = line.split(":")
        if len(parts) < 5:
            continue
        out.append((int(parts[0]), int(parts[1]), parts[3], parts[4]))
    return out


def byte_ranges(idx: list[tuple[int, int, str, str]]) -> dict[str, tuple[int, int | None, int]]:
    """Map output tag -> (start, end inclusive or None, record number) by VAR:LEVEL match."""
    ranges = {}
    for i, (num, offset, var, level) in enumerate(idx):
        for tag, want_var, want_level, _ in RECORDS:
            if var == want_var and level == want_level:
                end = idx[i + 1][1] - 1 if i + 1 < len(idx) else None
                ranges[tag] = (offset, end, num)
    return ranges


def fetch_hour(session: requests.Session, hour_iso: str) -> list[tuple[Path, int, bool]]:
    ymd, hh = hour_key(hour_iso)
    url = grib_url(ymd, hh)
    outputs = [(tag, HRRR_DIR / f"hrrr.{ymd}.t{hh}z.f00.{tag}.grib2") for tag, *_ in RECORDS]
    if all(path.exists() for _, path in outputs):
        return [(path, path.stat().st_size, False) for _, path in outputs]

    idx_resp = session.get(url + ".idx", timeout=60)
    idx_resp.raise_for_status()
    idx = parse_idx(idx_resp.text)
    ranges = byte_ranges(idx)
    for tag, _, _, expected_num in RECORDS:
        if tag not in ranges:
            raise SystemExit(f"{url}.idx: no record for {tag}")
        if ranges[tag][2] != expected_num:
            print(f"WARNING {url}.idx: {tag} is record {ranges[tag][2]}, brief says {expected_num}")

    results = []
    for tag, path in outputs:
        if path.exists():
            results.append((path, path.stat().st_size, False))
            continue
        start, end, _ = ranges[tag]
        header = f"bytes={start}-" if end is None else f"bytes={start}-{end}"
        resp = session.get(url, headers={"Range": header}, timeout=120)
        if resp.status_code != 206:
            raise SystemExit(f"{url} {header}: HTTP {resp.status_code}")
        path.write_bytes(resp.content)
        results.append((path, len(resp.content), True))
    return results


def check_rasterio(path: Path) -> None:
    import rasterio

    with rasterio.open(path) as src:
        print(f"rasterio opened {path.name}: driver={src.driver} crs={src.crs} shape={src.shape} "
              f"count={src.count} dtype={src.dtypes[0]} transform={tuple(src.transform)[:6]}")
        print(f"  tags: {src.tags(1)}")


def main(argv: list[str] | None = None) -> int:
    windows = json.loads(WINDOWS_JSON.read_text())
    hours = sorted({h for w in windows["windows"] for h in w["hours_utc"]})
    print(f"{len(hours)} distinct hours, {len(hours) * len(RECORDS)} fragments")
    HRRR_DIR.mkdir(parents=True, exist_ok=True)

    total = 0
    fetched = 0
    paths = []
    with requests.Session() as session:
        for hour in hours:
            for path, size, downloaded in fetch_hour(session, hour):
                total += size
                fetched += int(downloaded)
                paths.append(path)
                print(f"{'GET ' if downloaded else 'have'} {path.name} {size} bytes")
    print(f"fragments: {len(paths)}, downloaded now: {fetched}, total bytes: {total}")

    import rasterio

    drivers = rasterio.drivers.raster_driver_extensions()
    if "grib2" not in drivers and "grb2" not in drivers and "grib" not in drivers:
        print("rasterio GRIB driver missing: no grib extension in raster_driver_extensions()")
        return 1
    check_rasterio(paths[0])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
