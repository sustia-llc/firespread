"""Run one detection node over a video file or a webcam.

    edge-node --server http://192.168.1.20:8080 --mesh ui/nodes/los-alamos.json \
        --node fuller-lodge --video clips/smoke1.mp4 --weights dfire_yolov5s.pt

Frames are sampled at `--fps`. A box of class `--labels` above `--conf` counts
as a hit; the node posts an alert when `--persist K/M` of the last M frames
were hits, at most once every `--cooldown` seconds.

The bearing comes from the box's horizontal position and the node's heading
and field of view. A recorded clip was not filmed from the node's real site,
so for the mesh demo `--aim scenario` replaces that bearing with the bearing
from the node to the mesh file's scenario target, plus the box's offset from
the frame centre scaled down by `--aim-jitter`. The detection, its timing and
its confidence stay real; only the direction is staged.
"""

from __future__ import annotations

import argparse
import sys
import time

from .common import Node, load_mesh, mesh_nodes, post_alert
from .debounce import Persistence
from .detect import Box, load_detector
from .geo import frame_bearing, initial_bearing


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--server", default="http://127.0.0.1:8080")
    p.add_argument("--mesh", required=True, help="mesh JSON, e.g. ui/nodes/los-alamos.json")
    p.add_argument("--node", required=True, help="node id in the mesh file")
    p.add_argument("--video", required=True, help="video path, or a webcam index such as 0")
    p.add_argument("--weights", required=True, help="detector weights file")
    p.add_argument("--backend", choices=["yolov5", "ultralytics"], default="yolov5")
    p.add_argument("--device", default="cpu")
    p.add_argument("--labels", default="smoke,fire", help="classes that count as hits")
    p.add_argument("--conf", type=float, default=0.35)
    p.add_argument("--fps", type=float, default=2.0, help="frames sampled per second of video")
    p.add_argument("--persist", default="3/5", help="K/M persistence filter")
    p.add_argument("--cooldown", type=float, default=4.0, help="seconds between alerts")
    p.add_argument("--aim", choices=["frame", "scenario"], default="frame")
    p.add_argument("--aim-jitter", type=float, default=0.1, help="share of the frame offset kept with --aim scenario")
    p.add_argument("--fast", action="store_true", help="do not pace a file to real time")
    p.add_argument("--loop", action="store_true", help="restart the file at its end")
    p.add_argument("--show", action="store_true", help="draw boxes in a window")
    p.add_argument("--dry-run", action="store_true", help="print alerts instead of posting")
    return p.parse_args(argv)


def alert_bearing(args: argparse.Namespace, node: Node, box: Box, target: tuple[float, float] | None) -> float:
    b = frame_bearing(box.x_center, node.heading_deg, node.fov_deg)
    if args.aim == "scenario" and target is not None:
        offset = (b - node.heading_deg + 180.0) % 360.0 - 180.0
        return initial_bearing(node.lat, node.lon, target[0], target[1]) + args.aim_jitter * offset
    return b


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    import cv2  # noqa: PLC0415, optional dependency

    mesh = load_mesh(args.mesh)
    nodes = mesh_nodes(mesh)
    if args.node not in nodes:
        print(f"node {args.node!r} not in {args.mesh}; known: {', '.join(nodes)}", file=sys.stderr)
        return 2
    node = nodes[args.node]
    target = tuple(mesh.get("scenario", {}).get("target", ())) or None
    if args.aim == "scenario" and target is None:
        print("--aim scenario needs scenario.target in the mesh file", file=sys.stderr)
        return 2
    k, m = (int(x) for x in args.persist.split("/"))
    labels = {s.strip() for s in args.labels.split(",") if s.strip()}

    detector = load_detector(args.backend, args.weights, args.conf, args.device)
    source = int(args.video) if args.video.isdigit() else args.video
    cap = cv2.VideoCapture(source)
    if not cap.isOpened():
        print(f"cannot open video {args.video!r}", file=sys.stderr)
        return 2
    is_file = not isinstance(source, int)
    video_fps = cap.get(cv2.CAP_PROP_FPS) or 30.0
    step = max(1, round(video_fps / args.fps)) if is_file else 1
    persistence = Persistence(k, m)
    last_post = 0.0
    frame_no = 0
    print(f"[{node.id}] {node.name}: heading {node.heading_deg} deg, fov {node.fov_deg} deg, sampling every {step} frames")

    while True:
        t0 = time.monotonic()
        ok, frame = cap.read()
        if not ok:
            if is_file and args.loop:
                cap.set(cv2.CAP_PROP_POS_FRAMES, 0)
                continue
            break
        frame_no += 1
        if is_file and (frame_no - 1) % step:
            continue
        boxes = [b for b in detector(frame) if b.label in labels]
        best = max(boxes, key=lambda b: b.confidence, default=None)
        fire = persistence.update(best is not None)
        if fire and best is not None and time.monotonic() - last_post >= args.cooldown:
            bearing = alert_bearing(args, node, best, target)
            if args.dry_run:
                print(f"[{node.id}] would post {best.label} {best.confidence:.2f} at {bearing:.1f} deg")
            else:
                post_alert(args.server, node, bearing, best.confidence, best.label, f"yolo-{args.backend}")
            last_post = time.monotonic()
        if args.show:
            h, w = frame.shape[:2]
            for b in boxes:
                color = (0, 0, 255) if fire else (0, 200, 255)
                cv2.rectangle(frame, (int(b.x1 * w), int(b.y1 * h)), (int(b.x2 * w), int(b.y2 * h)), color, 2)
                cv2.putText(frame, f"{b.label} {b.confidence:.2f}", (int(b.x1 * w), max(15, int(b.y1 * h) - 5)),
                            cv2.FONT_HERSHEY_SIMPLEX, 0.6, color, 2)
            status = "ALERT" if fire else f"{sum(persistence.hits)}/{m} hits"
            cv2.putText(frame, f"{node.id}  {status}", (10, 30), cv2.FONT_HERSHEY_SIMPLEX, 0.9, (255, 255, 255), 2)
            cv2.imshow(f"edge node {node.id}", frame)
            if cv2.waitKey(1) & 0xFF == ord("q"):
                break
        if is_file and not args.fast:
            # Pace the file to real time at the sampled rate.
            time.sleep(max(0.0, step / video_fps - (time.monotonic() - t0)))
    cap.release()
    if args.show:
        cv2.destroyAllWindows()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
