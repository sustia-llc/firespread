# Detection mesh demo

A simulation of a smoke-detection mesh. Laptops and phones stand in for
camera nodes. Each node posts an alert (node id, position, bearing to the
smoke, confidence, time) to the `serve` example, and the dashboard draws each
node's bearing line and triangulates the smoke where two or more lines
agree. A single node's alert stays "unconfirmed", which is how the mesh
suppresses false positives such as clouds and dust.

What is real and what is staged:

- Real: the detector (a pretrained YOLO smoke and fire model on a laptop
  CPU), its timing and confidence on recorded smoke and false-positive
  clips, the alert network, and the triangulation.
- Staged: node positions are approximate public viewpoints, not surveyed
  sites, and a recorded clip was not filmed from its node. With
  `--aim scenario` the node posts the bearing from its position to the mesh
  file's scenario target, plus a small share of the box's offset in the
  frame. Say so in the pitch.

## 1. The server laptop

```
cargo build --release --example serve --features serve
./target/release/examples/serve --run ui/fixtures/fixture-w1 --bind 0.0.0.0 --port 8080
```

Open `http://127.0.0.1:8080/detect.html`. Find the laptop's LAN address
(`ipconfig getifaddr en0` on macOS, `hostname -I` on Linux); the other
devices use `http://<LAN address>:8080`. If the venue Wi-Fi isolates
clients, put every device on one phone hotspot. The map tiles come from
OpenStreetMap online; without network the nodes, lines and fix still draw
on a plain background.

The mesh file is `ui/nodes/los-alamos.json`. Another file in `ui/nodes/`
is picked with `detect.html?mesh=<name>`.

Settings on the dashboard: the alert window (seconds an alert stays active,
default 30) and the minimum confidence. "Reset alerts" clears the server for
every viewer.

A confirmed fix shows a link to the burn-probability page (`index.html`) of
the run that `serve` loads with `--run`. The fixture run is a small test
grid. For the demo, use `--run data/runs/demo/w1-seed7-m200` (`docs/demo.md`,
section 3). That run starts from its own
perimeter, not from the fix.

## 2. A detector node (laptop)

Install once, on good network, in its own environment. The `yolov5`
package pulls PyTorch, a large download:

```
uv venv .venv-edge && source .venv-edge/bin/activate
uv pip install -e "python/edge_node[yolov5]"
```

Weights: the YOLOv5 smoke and fire weights linked from the D-Fire README
(github.com/gaiasd/DFireDataset), classes `smoke` and `fire`. YOLOv8 or
YOLO11 weights work with `--backend ultralytics` and the `[ultralytics]`
extra.

Check the detector on a clip first, without the server:

```
edge-node --mesh ui/nodes/los-alamos.json --node fuller-lodge \
  --video clips/smoke1.mp4 --weights weights/dfire_yolov5s.pt --show --dry-run
```

Then run it against the server:

```
edge-node --server http://<LAN address>:8080 --mesh ui/nodes/los-alamos.json \
  --node fuller-lodge --video clips/smoke1.mp4 --weights weights/dfire_yolov5s.pt \
  --aim scenario --show --loop
```

`--persist 3/5` (default) needs a detection in 3 of the last 5 sampled
frames before an alert; `--conf` sets the box threshold (default 0.35);
`--fps` the sampling rate (default 2). A webcam is `--video 0`. Run a cloud
or dust clip on one node to show a false positive staying unconfirmed.

## 3. A phone node

Open `http://<LAN address>:8080/node.html` on the phone, pick a node, set
the bearing and press Report smoke. "Demo: aim at the scenario target" fills
in the scenario bearing.

## 4. Scripted scenario (fallback and rehearsal)

No model or video needed. From `python/edge_node`:

```
uv run --no-project python -m edge_node.simulate \
  --mesh ../../ui/nodes/los-alamos.json --server http://127.0.0.1:8080 --reset
```

At `--speed 1`: White Rock sees a cloud at 3 s to 11 s (one node,
unconfirmed, then it ages out). Pajarito Mountain sees the smoke at 45 s,
the townsite joins at 60 s (the first confirmed fix), then Bandelier and
White Rock. About two minutes in all. `--speed 2` halves it; lower the
dashboard's alert window by the same factor so the cloud ages out before
the smoke appears.

`--target nnw` uses the named target in `scenario.targets`, north-northwest
of the townsite. The `fuller-lodge-nnw` camera sees it first, then White
Rock and Bandelier.

## 5. Tests

```
cargo test -p firespread --features serve --test serve
(cd python/edge_node && uv run --no-project --with pytest python -m pytest tests)
```
