"""Writes the FireBench Rothermel_SFIRE single-class oracle fixture.

Reads the Scott and Burgan 40 table embedded in firespread-fuel, calls
``firebench.ros_models.surface_for_vegetation.Rothermel_SFIRE.rothermel`` for
GR1, GR4, SH2, TU1 and TL3 as single dead 1-h classes over a wind by slope
grid, and writes the cases whose rate of spread the oracle did not clamp to
its 6.0 m/s ceiling.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import sys
from pathlib import Path

import firebench
from firebench.ros_models.surface_for_vegetation import Rothermel_SFIRE

REPO = Path(__file__).resolve().parents[2]
DEFAULT_CSV = REPO / "crates" / "firespread-fuel" / "data" / "data_ScottandBurgan40.csv"
DEFAULT_OUT = (
    REPO
    / "crates"
    / "firespread-rothermel"
    / "tests"
    / "fixtures"
    / "firebench_single_class.json"
)

MODELS = ["GR1", "GR4", "SH2", "TU1", "TL3"]
WIND_M_S = [0.0, 2.0, 5.0, 10.0, 15.0]
SLOPE_PERCENT = [0.0, 10.0, 30.0, 60.0, 100.0]
FMC_PERCENT = 8.0

KG_PER_SHORT_TON = 907.18474
M2_PER_ACRE = 4046.8564224
M_PER_FT = 0.3048

FUELDENS_LB_FT3 = 32.0
ST = 0.0555
SE = 0.01
ICHAP = 0
ORACLE_CLAMP_M_S = 6.0
# surface_for_vegetation.py: cmbcnst = 17.433e06 J/kg; fuelheat = cmbcnst * 4.30e-04 Btu/lb.
ORACLE_HEAT_BTU_LB = 17.433e06 * 4.30e-04


def t_ac_to_kg_m2(t_ac: float) -> float:
    return t_ac * KG_PER_SHORT_TON / M2_PER_ACRE


def load_models(csv_path: Path) -> dict[str, dict[str, str]]:
    with csv_path.open(newline="") as handle:
        return {row["fuel_model_code"]: row for row in csv.DictReader(handle)}


def build_cases(rows: dict[str, dict[str, str]]) -> list[dict[str, float | str]]:
    cases: list[dict[str, float | str]] = []
    for code in MODELS:
        row = rows[code]
        fgi = t_ac_to_kg_m2(float(row["fuel_load_1h"]))
        depth_m = float(row["fuel_bed_depth"]) * M_PER_FT
        savr = float(row["sav_ratio_dead_1h"])
        # The oracle divides the moisture fraction by fuelmce as given
        # (rtemp1 = fuelmc_g / fuelmce), so fuelmce is passed as a fraction.
        fuelmce = float(row["dead_fuel_extinction_moisture"]) / 100.0
        for wind in WIND_M_S:
            for slope_pct in SLOPE_PERCENT:
                slope_deg = math.degrees(math.atan(slope_pct / 100.0))
                ros = Rothermel_SFIRE.rothermel(
                    fgi=fgi,
                    fueldepthm=depth_m,
                    fueldens=FUELDENS_LB_FT3,
                    savr=savr,
                    fuelmce=fuelmce,
                    st=ST,
                    se=SE,
                    ichap=ICHAP,
                    wind=wind,
                    slope=slope_deg,
                    fmc=FMC_PERCENT,
                )
                if ros == ORACLE_CLAMP_M_S:
                    continue
                cases.append(
                    {
                        "model": code,
                        "fgi_kg_m2": fgi,
                        "fueldepth_m": depth_m,
                        "savr_per_ft": savr,
                        "fuelmce_passed": fuelmce,
                        "fmc_percent": FMC_PERCENT,
                        "wind_m_s": wind,
                        "slope_percent": slope_pct,
                        "slope_deg": slope_deg,
                        "ros_m_s": float(ros),
                    }
                )
    return cases


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csv", type=Path, default=DEFAULT_CSV)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)

    rows = load_models(args.csv)
    cases = build_cases(rows)
    fixture = {
        "oracle": {
            "function": "firebench.ros_models.surface_for_vegetation.Rothermel_SFIRE.rothermel",
            "firebench_version": getattr(firebench, "__version__", "unknown"),
            "heat_btu_per_lb": ORACLE_HEAT_BTU_LB,
            "fueldens_lb_ft3": FUELDENS_LB_FT3,
            "st": ST,
            "se": SE,
            "ichap": ICHAP,
            "net_load": "fuelload / (1 + st)",
            "fuelmce_convention": "fraction: the oracle computes fuelmc_g / fuelmce with fuelmc_g = fmc / 100",
            "clamp_m_s": ORACLE_CLAMP_M_S,
            "skipped_clamped_cases": len(MODELS) * len(WIND_M_S) * len(SLOPE_PERCENT) - len(cases),
        },
        "cases": cases,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w") as handle:
        json.dump(fixture, handle, indent=2, sort_keys=True)
        handle.write("\n")
    print(f"wrote {len(cases)} cases to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
