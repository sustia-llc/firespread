"""Print layer metadata and a few rows of the FEDS FlatGeobuf files."""

from __future__ import annotations

import sys

import pyogrio

from .paths import FEDS_NEWFIREPIX, FEDS_PERIMETER


def main(argv: list[str] | None = None) -> int:
    for path in (FEDS_PERIMETER, FEDS_NEWFIREPIX):
        info = pyogrio.read_info(path)
        print(f"== {path.name}")
        for key in ("crs", "fields", "dtypes", "geometry_type", "features", "total_bounds"):
            print(f"  {key}: {info.get(key)}")
        frame = pyogrio.read_dataframe(path, max_features=3)
        print(frame.drop(columns="geometry").to_string())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
