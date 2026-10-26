"""Writes the FireBench binary-mask shape-index oracle fixture.

Draws seeded random mask pairs with ``numpy.random.default_rng`` over a list of
shapes up to 64x64 and every ordered pair of cell densities from
``DENSITIES`` (so both-empty, one-empty and both-full pairs are included),
calls ``firebench.metrics.perimeter.shape_index.jaccard_binary`` and
``sorensen_dice_binary`` on each pair, and writes the masks as row-major
``0``/``1`` strings with both scores to a JSON fixture.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import firebench
import numpy as np
from firebench.metrics.perimeter.shape_index import jaccard_binary, sorensen_dice_binary

REPO = Path(__file__).resolve().parents[2]
DEFAULT_OUT = (
    REPO
    / "crates"
    / "firespread-metrics"
    / "tests"
    / "fixtures"
    / "firebench_metrics.json"
)

SEED = 20261026
SHAPES = [(1, 1), (1, 4), (7, 7), (16, 16), (32, 48), (64, 64)]
DENSITIES = [0.0, 0.01, 0.3, 0.7, 1.0]


def encode(mask: np.ndarray) -> str:
    return "".join("1" if cell else "0" for cell in mask.ravel(order="C"))


def build_cases(seed: int) -> list[dict[str, object]]:
    rng = np.random.default_rng(seed)
    cases: list[dict[str, object]] = []
    for rows, cols in SHAPES:
        for density_s in DENSITIES:
            for density_o in DENSITIES:
                simulated = rng.random((rows, cols)) < density_s
                observed = rng.random((rows, cols)) < density_o
                cases.append(
                    {
                        "rows": rows,
                        "cols": cols,
                        "density_simulated": density_s,
                        "density_observed": density_o,
                        "simulated": encode(simulated),
                        "observed": encode(observed),
                        "jaccard": float(jaccard_binary(simulated, observed)),
                        "dice": float(sorensen_dice_binary(simulated, observed)),
                    }
                )
    return cases


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args(argv)

    cases = build_cases(args.seed)
    fixture = {
        "oracle": {
            "functions": [
                "firebench.metrics.perimeter.shape_index.jaccard_binary",
                "firebench.metrics.perimeter.shape_index.sorensen_dice_binary",
            ],
            "firebench_version": getattr(firebench, "__version__", "unknown"),
            "numpy_version": np.__version__,
            "rng": "numpy.random.default_rng",
            "seed": args.seed,
            "shapes": SHAPES,
            "densities": DENSITIES,
            "mask_encoding": "row-major string, '1' = burned",
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
