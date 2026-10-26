"""Anderson et al. 2015 shrubland comparison: engine single-class spread rate against observed.

``inputs`` writes one record per usable row of FireBench's ``Table_A1`` (experimental
fires) and ``Table_8`` (field fires) with the engine's single-class inputs. ``plot``
reads the engine's predictions and draws predicted against observed rate of spread.

Inputs per row:
- SAV ratio, dead moisture of extinction and heat come from the closest Scott and
  Burgan 40 model by ``firebench.tools.find_closest_fuel_class_by_properties``
  (weighted L1, weights 1 / target). Table A1 matches on total dry load and fuel
  height. Table 8 has no total load and matches on fuel height alone.
- Load is the observed fine dead load (A1 ``fldf``, Table 8 ``flf``), depth the
  observed fuel height, moisture the observed dead fine moisture.
- Midflame wind: Table A1 2 m wind as given. Table 8 10 m wind times its ``windrf``.
- Slope: Table A1 ``slope`` percent, negative values set to 0. Table 8 has none: 0.
Rows with any of these values missing (-9999) are skipped.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import sys
from importlib.resources import files
from pathlib import Path

from firebench.tools import (
    find_closest_fuel_class_by_properties,
    import_scott_burgan_40_fuel_model,
    read_data_file,
)
from firebench.tools.namespace import StandardVariableNames as svn
from firebench.tools.units import ureg

REPO = Path(__file__).resolve().parents[2]
SB40_CSV = REPO / "crates" / "firespread-fuel" / "data" / "data_ScottandBurgan40.csv"
OUT_DIR = REPO / "data" / "anderson_2015"
DATASET = "ros_model_validation/Anderson_2015"
NODATA = -9999.0
KMH_TO_M_S = 1.0 / 3.6
S_PER_MIN = 60.0


def sb40_rows() -> list[dict[str, str]]:
    with SB40_CSV.open(newline="") as handle:
        return list(csv.DictReader(handle))


def magnitude(table: dict, key: str, i: int, unit: str) -> float:
    value = table[key][i].to(unit).magnitude
    return float(value)


def missing(*values: float) -> bool:
    return any(math.isnan(v) or v == NODATA for v in values)


def record(source: str, i: int, sb40: list[dict[str, str]], index: int, **obs: float) -> dict:
    row = sb40[index - 1]
    return {
        "source": source,
        "row": i + 1,
        "sb40": row["fuel_model_code"],
        "sav_per_ft": float(row["sav_ratio_dead_1h"]),
        "mx_fraction": float(row["dead_fuel_extinction_moisture"]) / 100.0,
        "heat_btu_lb": float(row["heat_content"]),
        **obs,
    }


def table_a1(sb40_fb: dict, sb40: list[dict[str, str]]) -> list[dict]:
    t = read_data_file("Table_A1", DATASET)
    out = []
    for i in range(t["nb_fuel_classes"]):
        depth = magnitude(t, svn.FUEL_HEIGHT, i, "m")
        fine = magnitude(t, svn.FUEL_LOAD_FINE_DEAD, i, "kg/m^2")
        total = magnitude(t, svn.FUEL_LOAD_DRY_TOTAL, i, "kg/m^2")
        wind = magnitude(t, svn.WIND_SPEED, i, "m/s")
        fmc = magnitude(t, svn.FUEL_MOISTURE_CONTENT_ELEVATED_DEAD, i, "percent")
        slope = magnitude(t, svn.SLOPE_ANGLE, i, "percent")
        ros = magnitude(t, svn.RATE_OF_SPREAD, i, "m/s")
        if missing(depth, fine, total, wind, fmc, slope, ros):
            continue
        index = find_closest_fuel_class_by_properties(
            sb40_fb, {svn.FUEL_LOAD_DRY_TOTAL: t[svn.FUEL_LOAD_DRY_TOTAL][i], svn.FUEL_HEIGHT: t[svn.FUEL_HEIGHT][i]}
        )
        out.append(
            record(
                "Table_A1", i, sb40, index,
                load_kg_m2=fine, depth_m=depth, moisture_fraction=fmc / 100.0,
                midflame_wind_m_s=wind, slope_tan=max(slope, 0.0) / 100.0, observed_ros_m_s=ros,
            )
        )
    return out


def table_8(sb40_fb: dict, sb40: list[dict[str, str]]) -> list[dict]:
    # firebench 0.10.0 read_data_file fails on Table_8 (NaN cast to int32), so the
    # CSV is read directly with the units of Table_8.json.
    path = files("firebench") / "resources" / "data" / DATASET / "data_Table_8.csv"
    with path.open(newline="") as handle:
        rows = list(csv.DictReader(handle))
    out = []
    for i, r in enumerate(rows):
        depth = float(r["fheight"])
        fine = float(r["flf"])
        wind = float(r["wspd"]) * KMH_TO_M_S
        waf = float(r["windrf"])
        fmc = float(r["fmoistdf"])
        ros = float(r["ros"]) / S_PER_MIN
        if missing(depth, fine, float(r["wspd"]), waf, fmc, float(r["ros"])):
            continue
        index = find_closest_fuel_class_by_properties(sb40_fb, {svn.FUEL_HEIGHT: ureg.Quantity(depth, "m")})
        out.append(
            record(
                "Table_8", i, sb40, index,
                load_kg_m2=fine, depth_m=depth, moisture_fraction=fmc / 100.0,
                midflame_wind_m_s=wind * waf, slope_tan=0.0, observed_ros_m_s=ros,
            )
        )
    return out


def cmd_inputs(args: argparse.Namespace) -> int:
    sb40_fb = import_scott_burgan_40_fuel_model()
    sb40 = sb40_rows()
    records = table_a1(sb40_fb, sb40) + table_8(sb40_fb, sb40)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w") as handle:
        json.dump({"records": records}, handle, indent=1)
        handle.write("\n")
    counts = {s: sum(r["source"] == s for r in records) for s in ("Table_A1", "Table_8")}
    print(f"wrote {len(records)} records {counts} to {args.out}")
    return 0


def cmd_plot(args: argparse.Namespace) -> int:
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    with args.predictions.open() as handle:
        rows = json.load(handle)["records"]
    fig, ax = plt.subplots(figsize=(6, 6), constrained_layout=True)
    summary = {}
    for source, marker in (("Table_A1", "o"), ("Table_8", "s")):
        sel = [r for r in rows if r["source"] == source]
        obs = [r["observed_ros_m_s"] * S_PER_MIN for r in sel]
        pred = [r["predicted_ros_m_s"] * S_PER_MIN for r in sel]
        label = "experimental (Table A1)" if source == "Table_A1" else "field (Table 8)"
        zeros = sum(p <= 0 for p in pred)
        ax.loglog(
            [o for o, p in zip(obs, pred) if p > 0],
            [p for p in pred if p > 0],
            marker, ms=5, mfc="none",
            label=f"{label}, n={len(sel) - zeros} (+{zeros} predicted 0, not shown)",
        )
        logratio = [math.log10(p / o) for p, o in zip(pred, obs) if p > 0 and o > 0]
        summary[source] = {
            "n": len(sel),
            "n_predicted_zero": sum(p <= 0 for p in pred),
            "median_log10_pred_over_obs": sorted(logratio)[len(logratio) // 2] if logratio else None,
            "mean_abs_log10_pred_over_obs": sum(abs(x) for x in logratio) / len(logratio) if logratio else None,
            "within_factor_2": sum(abs(x) <= math.log10(2) for x in logratio),
        }
    lo, hi = 0.1, 1000.0
    ax.plot([lo, hi], [lo, hi], "k-", lw=0.8, label="1:1")
    ax.plot([lo, hi], [2 * lo, 2 * hi], "k:", lw=0.8, label="factor 2")
    ax.plot([lo, hi], [lo / 2, hi / 2], "k:", lw=0.8)
    ax.set_xlim(lo, hi)
    ax.set_ylim(lo, hi)
    ax.set_xlabel("Observed rate of spread [m/min]")
    ax.set_ylabel("firespread single-class predicted [m/min]")
    ax.set_title("Anderson et al. 2015 shrubland fires")
    ax.legend(loc="upper left", fontsize=8)
    ax.tick_params(direction="in", top=True, right=True, which="both")
    args.plot.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.plot, dpi=150)
    with args.summary.open("w") as handle:
        json.dump(summary, handle, indent=1)
        handle.write("\n")
    print(json.dumps(summary, indent=1))
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    p_in = sub.add_parser("inputs")
    p_in.add_argument("--out", type=Path, default=OUT_DIR / "inputs.json")
    p_in.set_defaults(func=cmd_inputs)
    p_plot = sub.add_parser("plot")
    p_plot.add_argument("--predictions", type=Path, default=OUT_DIR / "predictions.json")
    p_plot.add_argument("--plot", type=Path, default=OUT_DIR / "anderson_2015.png")
    p_plot.add_argument("--summary", type=Path, default=OUT_DIR / "summary.json")
    p_plot.set_defaults(func=cmd_plot)
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
