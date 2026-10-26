"""Polygon Jaccard of a bundle's entrants against its observations, next to the scorecard's.

For each run under ``--runs``, prints ``firebench.benchmarks.c001_caldor.jaccard_from_list``
(``jaccard_polygon`` in EPSG:5070) of each entrant ``.h5`` in ``--bundle`` against
``observations.h5`` at the window's target entry, and the raster Jaccard of that entrant's
``feds`` row in the run's ``scorecard.json``. An entrant whose ``.h5`` is absent is skipped.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import h5py

from firebench.benchmarks.c001_caldor import jaccard_from_list

from firebench_case.bundle import FIRESPREAD, GEOPOLYGONS, OBSERVATIONS, PERSISTENCE, entry_name, load_runs

ENTRANTS = [FIRESPREAD, PERSISTENCE, "uniform_growth"]
PROJECTION = "EPSG:5070"


def table(bundle: Path, runs: Path, data: Path) -> list[dict]:
    rows = []
    with h5py.File(bundle / f"{OBSERVATIONS}.h5", "r") as obs:
        for run in load_runs(runs, data):
            card = json.loads((run.run_dir / "scorecard.json").read_text())
            raster = {
                r["entrant"]: r["jaccard"]
                for r in card["rows"]
                if r["observation"] == "feds" and r["horizon_h"] == run.horizon_h
            }
            perim = f"/{GEOPOLYGONS}/{entry_name(run.target)}"
            for entrant in ENTRANTS:
                path = bundle / f"{entrant}.h5"
                if not path.is_file():
                    continue
                with h5py.File(path, "r") as model:
                    polygon = float(jaccard_from_list(model, obs, [perim], PROJECTION)[0])
                rows.append(
                    {
                        "window": run.window,
                        "entry": perim,
                        "entrant": entrant,
                        "jaccard_polygon": polygon,
                        "jaccard_raster": raster.get(entrant),
                    }
                )
    return rows


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--runs", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    args = parser.parse_args()
    print(f"{'window':<7}{'entrant':<16}{'jaccard_polygon':>16}{'jaccard_raster':>16}  entry")
    for r in table(args.bundle, args.runs, args.data):
        raster = "n/a" if r["jaccard_raster"] is None else f"{r['jaccard_raster']:.4f}"
        print(f"{r['window']:<7}{r['entrant']:<16}{r['jaccard_polygon']:>16.4f}{raster:>16}  {r['entry']}")


if __name__ == "__main__":
    main()
