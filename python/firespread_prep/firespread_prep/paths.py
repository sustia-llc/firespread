"""Filesystem locations shared by every firespread_prep script."""

from __future__ import annotations

import os
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
DATA_DIR = Path(os.environ.get("FIRESPREAD_DATA", REPO_ROOT / "data"))

WFIGS_DIR = DATA_DIR / "wfigs"
FEDS_DIR = DATA_DIR / "feds"
HRRR_DIR = DATA_DIR / "hrrr"
GRIDMET_DIR = DATA_DIR / "gridmet"
LANDFIRE_DIR = DATA_DIR / "landfire"
GRID_DIR = DATA_DIR / "grid"

EXTENT_JSON = DATA_DIR / "extent.json"
WINDOWS_JSON = DATA_DIR / "windows.json"

WFIGS_FILES = (
    WFIGS_DIR / "hermits_peak_947.geojson",
    WFIGS_DIR / "calf_canyon_49422.geojson",
)
FEDS_PERIMETER = FEDS_DIR / "feds_2022_lf_perimeter.fgb"
FEDS_NEWFIREPIX = FEDS_DIR / "feds_2022_lf_newfirepix.fgb"
FEDS_FIREIDS_JSON = FEDS_DIR / "hermits_fireids.json"
FEDS_PERIMETERS_GPKG = FEDS_DIR / "hermits_perimeters.gpkg"
FEDS_NEWFIREPIX_JSON = FEDS_DIR / "hermits_newfirepix_fireids.json"
FEDS_NEWFIREPIX_GPKG = FEDS_DIR / "hermits_newfirepix.gpkg"
