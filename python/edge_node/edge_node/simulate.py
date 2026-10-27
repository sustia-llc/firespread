"""Post a scripted smoke scenario from every node in a mesh file, no model or
video needed. This is the demo's fallback when the detector, the clips or a
laptop fails, and a rehearsal tool.

    edge-node-sim --server http://127.0.0.1:8080 --mesh ui/nodes/los-alamos.json --reset

Timeline at `--speed 1`: the scenario's false positive (one node, one bearing,
a few low-confidence alerts that then stop) runs from 3 s to 11 s and has
aged out of the dashboard's window by the time the smoke appears. The node
nearest the target that has it in view sees the smoke at 45 s, and each
further node in view joins 15 s after the previous one. Confidence climbs as the plume
grows. Bearings carry Gaussian noise of `--noise` degrees.
"""

from __future__ import annotations

import argparse
import json
import random
import time
import urllib.request

from .common import Node, load_mesh, mesh_nodes, post_alert
from .geo import distance_m, in_view, initial_bearing


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--server", default="http://127.0.0.1:8080")
    p.add_argument("--mesh", required=True)
    p.add_argument("--target", help="lat,lon or a name in scenario.targets; defaults to the mesh scenario target")
    p.add_argument("--speed", type=float, default=1.0, help="time multiplier, 2 runs twice as fast")
    p.add_argument("--duration", type=float, default=120.0, help="scenario seconds")
    p.add_argument("--interval", type=float, default=4.0, help="seconds between a node's alerts")
    p.add_argument("--noise", type=float, default=1.5, help="bearing noise, degrees (1 sigma)")
    p.add_argument("--no-false-positive", action="store_true")
    p.add_argument("--reset", action="store_true", help="clear the server's alerts first")
    p.add_argument("--seed", type=int, default=7)
    return p.parse_args(argv)


def plan(nodes: dict[str, Node], target: tuple[float, float]) -> list[tuple[float, Node]]:
    """(start second, node) for every node with the target in view, nearest first."""
    seeing = [
        n for n in nodes.values()
        if in_view(initial_bearing(n.lat, n.lon, *target), n.heading_deg, n.fov_deg)
    ]
    seeing.sort(key=lambda n: distance_m(n.lat, n.lon, *target))
    return [(45.0 + 15.0 * i, n) for i, n in enumerate(seeing)]


def reset(server: str) -> None:
    req = urllib.request.Request(server.rstrip("/") + "/api/alerts", method="DELETE")
    with urllib.request.urlopen(req, timeout=3) as resp:
        resp.read()


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    rng = random.Random(args.seed)
    mesh = load_mesh(args.mesh)
    nodes = mesh_nodes(mesh)
    scenario = mesh.get("scenario", {})
    named = scenario.get("targets", {})
    if args.target in named:
        target = tuple(named[args.target])
    elif args.target:
        lat, lon = (float(x) for x in args.target.split(","))
        target = (lat, lon)
    else:
        target = tuple(scenario["target"])
    schedule = plan(nodes, target)
    for start, n in schedule:
        b = initial_bearing(n.lat, n.lon, *target)
        d = distance_m(n.lat, n.lon, *target) / 1000
        print(f"{n.id}: sees smoke at {start:.0f} s, true bearing {b:.1f} deg, {d:.1f} km")
    events: list[tuple[float, Node, float, float, str]] = []
    for start, n in schedule:
        t = start
        while t <= args.duration:
            growth = min(1.0, (t - start) / 40.0)
            conf = 0.40 + 0.50 * growth + rng.uniform(-0.04, 0.04)
            bearing = initial_bearing(n.lat, n.lon, *target) + rng.gauss(0.0, args.noise)
            events.append((t, n, bearing, conf, "smoke"))
            t += args.interval
    fp = scenario.get("false_positive")
    if fp and not args.no_false_positive and fp["node"] in nodes:
        n = nodes[fp["node"]]
        for i, t in enumerate((3.0, 7.0, 11.0)):
            events.append((t, n, fp["bearing_deg"] + rng.gauss(0.0, 2.0), 0.42 + 0.03 * i, fp.get("label", "smoke")))
        print(f"{n.id}: false positive at {fp['bearing_deg']} deg from 3 s to 11 s ({fp.get('note', '')})")
    events.sort(key=lambda e: e[0])
    if args.reset:
        reset(args.server)
    t0 = time.monotonic()
    for t, n, bearing, conf, label in events:
        time.sleep(max(0.0, t / args.speed - (time.monotonic() - t0)))
        post_alert(args.server, n, bearing, conf, label, "sim")
    print(json.dumps({"alerts": len(events), "target": target}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
