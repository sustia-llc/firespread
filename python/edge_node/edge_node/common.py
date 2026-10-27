"""Node configuration and alert posting, shared by `node` and `simulate`."""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


@dataclass(frozen=True)
class Node:
    id: str
    name: str
    lat: float
    lon: float
    heading_deg: float
    fov_deg: float


def load_mesh(path: str | Path) -> dict:
    """Read a mesh file such as `ui/nodes/los-alamos.json`."""
    return json.loads(Path(path).read_text(encoding="utf-8"))


def mesh_nodes(mesh: dict) -> dict[str, Node]:
    return {
        n["id"]: Node(n["id"], n["name"], n["lat"], n["lon"], n["heading_deg"], n["fov_deg"])
        for n in mesh["nodes"]
    }


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def post_alert(
    server: str,
    node: Node,
    bearing_deg: float,
    confidence: float,
    label: str,
    source: str,
    timeout_s: float = 3.0,
) -> bool:
    """POST one alert to `<server>/api/alerts`. Returns False and prints the
    reason when the server cannot be reached, so a node keeps running through
    a network drop."""
    body = {
        "node_id": node.id,
        "lat": node.lat,
        "lon": node.lon,
        "bearing_deg": round(bearing_deg % 360.0, 2),
        "confidence": round(min(max(confidence, 0.0), 1.0), 3),
        "label": label,
        "t": utc_now(),
        "source": source,
    }
    req = urllib.request.Request(
        server.rstrip("/") + "/api/alerts",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            ok = resp.status == 201
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        print(f"[{node.id}] post failed: {e}")
        return False
    print(f"[{node.id}] {label} {body['confidence']:.2f} at {body['bearing_deg']:.1f} deg ({source})")
    return ok
