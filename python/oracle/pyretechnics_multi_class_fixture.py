"""Writes the pyretechnics multi-class oracle fixture.

PYRETECHNICS developed by Spatial Informatics Group, LLC and contributors.

Runs under the pyretechnics Community License in its own venv
(``~/.venvs/pyretechnics``, Python 3.12). Calls
``pyretechnics.fuel_models.get_fuel_model``, ``moisturize``,
``pyretechnics.surface_fire.calc_surface_fire_behavior_no_wind_no_slope`` and
``calc_surface_fire_behavior_max`` for all 40 Scott and Burgan models over the
dead moisture scenarios D1, D2 and D3 with live scenario L2, a midflame wind by
slope grid with the wind blowing upslope, and no wind limit.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from importlib.metadata import version
from pathlib import Path

from pyretechnics.fuel_models import get_fuel_model, moisturize
from pyretechnics.surface_fire import (
    calc_surface_fire_behavior_max,
    calc_surface_fire_behavior_no_wind_no_slope,
)

REPO = Path(__file__).resolve().parents[2]
DEFAULT_OUT = (
    REPO
    / "crates"
    / "firespread-rothermel"
    / "tests"
    / "fixtures"
    / "pyretechnics_multi_class.json"
)

# LANDFIRE FBFM40 numbers for the Scott and Burgan 40 codes.
GROUPS = [("GR", 101, 9), ("GS", 121, 4), ("SH", 141, 9), ("TU", 161, 5), ("TL", 181, 9), ("SB", 201, 4)]
MODELS = [(f"{prefix}{i}", first + i - 1) for prefix, first, count in GROUPS for i in range(1, count + 1)]

WIND_M_S = [0.0, 2.0, 5.0, 10.0, 15.0]
SLOPE_PERCENT = [0.0, 10.0, 30.0, 60.0, 100.0]
# Scott and Burgan 2005 RMRS-GTR-153 Table 3 (p. 8), dead 1-h, 10-h, 100-h, and
# Table 4 (p. 8) live scenario L2, herbaceous and woody, all in percent.
DEAD_SCENARIOS = {"D1": (3.0, 4.0, 5.0), "D2": (6.0, 7.0, 8.0), "D3": (9.0, 10.0, 11.0)}
LIVE_SCENARIO = ("L2", 60.0, 90.0)
# Aspect 0 faces north, so upslope points south; wind from the north blows upslope.
ASPECT_DEG = 0.0
UPWIND_DEG = 0.0
S_PER_MIN = 60.0


def build_cases() -> list[dict[str, float | str]]:
    live_name, live_herb, live_woody = LIVE_SCENARIO
    cases: list[dict[str, float | str]] = []
    for code, number in MODELS:
        fuel_model = get_fuel_model(number)
        if fuel_model["number"] != number:
            raise ValueError(f"{code}: asked for {number}, got {fuel_model['number']}")
        for scenario, (d1, d10, d100) in DEAD_SCENARIOS.items():
            moisture = [d1 / 100.0, d10 / 100.0, d100 / 100.0, 0.0, live_herb / 100.0, live_woody / 100.0]
            moisturized = moisturize(fuel_model, moisture)
            base = calc_surface_fire_behavior_no_wind_no_slope(moisturized)
            for wind in WIND_M_S:
                for slope_pct in SLOPE_PERCENT:
                    out = calc_surface_fire_behavior_max(
                        base,
                        wind * S_PER_MIN,
                        UPWIND_DEG,
                        slope_pct / 100.0,
                        ASPECT_DEG,
                        use_wind_limit=False,
                    )
                    cases.append(
                        {
                            "model": code,
                            "fbfm40": number,
                            "dead_scenario": scenario,
                            "live_scenario": live_name,
                            "wind_m_s": wind,
                            "slope_percent": slope_pct,
                            "base_spread_rate_m_s": base["base_spread_rate"] / S_PER_MIN,
                            "max_spread_rate_m_s": out["max_spread_rate"] / S_PER_MIN,
                            "max_fireline_intensity_kw_m": out["max_fireline_intensity"],
                            "max_flame_length_m": out["max_flame_length"],
                        }
                    )
    return cases


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)

    cases = build_cases()
    if any(not math.isfinite(float(c["max_spread_rate_m_s"])) for c in cases):
        raise ValueError("non-finite spread rate in oracle output")
    fixture = {
        "oracle": {
            "attribution": "PYRETECHNICS developed by Spatial Informatics Group, LLC and contributors.",
            "license": "pyretechnics Community License",
            "pyretechnics_version": version("pyretechnics"),
            "functions": [
                "pyretechnics.fuel_models.get_fuel_model",
                "pyretechnics.fuel_models.moisturize",
                "pyretechnics.surface_fire.calc_surface_fire_behavior_no_wind_no_slope",
                "pyretechnics.surface_fire.calc_surface_fire_behavior_max",
            ],
            "use_wind_limit": False,
            "spread_rate_adjustment": 1.0,
            "aspect_deg": ASPECT_DEG,
            "upwind_direction_deg": UPWIND_DEG,
            "dead_scenarios_percent": {k: list(v) for k, v in DEAD_SCENARIOS.items()},
            "live_scenario_percent": {LIVE_SCENARIO[0]: [LIVE_SCENARIO[1], LIVE_SCENARIO[2]]},
        },
        "cases": cases,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w") as handle:
        json.dump(fixture, handle, indent=1, sort_keys=True)
        handle.write("\n")
    print(f"wrote {len(cases)} cases to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
