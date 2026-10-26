# firespread demo

This guide prepares and runs the hackathon demo. Run every command from the
repository root. The commands assume the prepared data in `data/` (see the
README, "Prepare the data").

## 1. Build once

Build the `firespread` command and the `serve` example before the demo:

```
cargo build --release -p firespread
cargo build --release -p firespread --features serve --example serve
```

The command is then `target/release/firespread`, and the server is
`target/release/examples/serve`.

## 2. Prepare the three demo runs

Each window needs one ensemble run and one score. The runs go to
`data/runs/demo/`, which git ignores. Set `--threads` to the number of CPU
cores of the demo machine (`nproc` prints it).

Window w1, 12 h:

```
target/release/firespread ensemble --window w1 --members 200 --seed 7 \
  --threads 12 --runs-dir data/runs/demo --data data --config config/default.toml
target/release/firespread score --run data/runs/demo/w1-seed7-m200 --data data
```

Window w2, 12 h:

```
target/release/firespread ensemble --window w2 --members 200 --seed 7 \
  --threads 12 --runs-dir data/runs/demo --data data --config config/default.toml
target/release/firespread score --run data/runs/demo/w2-seed7-m200 --data data
```

Window w3, 24 h, with D3 night moisture:

```
target/release/firespread ensemble --window w3 --members 200 --seed 7 \
  --threads 12 --runs-dir data/runs/demo --data data --config config/w3.toml
target/release/firespread score --run data/runs/demo/w3-seed7-m200 --data data
```

On the 12-thread development machine, the ensembles took about 13 s for w1,
50 s for w2 and 240 s for w3. Only w1 is fast enough to run live.

## 3. Start the page

Start one server per window, each on its own port:

```
target/release/examples/serve --run data/runs/demo/w1-seed7-m200 --port 8761
target/release/examples/serve --run data/runs/demo/w2-seed7-m200 --port 8762
target/release/examples/serve --run data/runs/demo/w3-seed7-m200 --port 8763
```

Each server prints its URL. Open `http://127.0.0.1:8761/` for w1, and use
ports 8762 and 8763 for w2 and w3. Press Ctrl-C to stop a server.

The server listens on 127.0.0.1 only. To view the page from another computer,
open an SSH tunnel on that computer, and keep it open:

```
ssh -N -L 18761:127.0.0.1:8761 <user>@<demo-machine>
```

Then open `http://localhost:18761/` on that computer. Run the browser on the
same computer as the tunnel.

## 4. Street basemap (optional)

If `data/basemap/region.pmtiles` exists, the page draws a street basemap. If it
is missing, the page skips that layer. To make it, install the `pmtiles` CLI
(go-pmtiles) and run:

```
pmtiles extract https://build.protomaps.com/20261025.pmtiles \
  data/basemap/region.pmtiles \
  --bbox=-105.8431,35.5002,-104.9631,36.2037 --maxzoom=14
```

The box is the run grid from `data/extent.json`. The extract is about 3.4 MB.

## 5. What to show

### The page

1. Move the hour slider from h+1 to h+12. The colored area grows from the start
   perimeter. The start perimeter stays dark red, because every member burns it.
2. Turn the map layers on and off: burn probability, observed perimeter,
   hillshade and street basemap.
3. Change the scorecard metric: Jaccard, Dice, precision, recall and Brier.
   The chart compares firespread with persistence and uniform growth.
4. Point to the WFIGS table: the share of each forecast inside the official
   final perimeter.
5. Point to the burned area chart: the ensemble band and median against the
   two observed areas.
6. Hover over or tap any "?" button, entrant name, layer name or the slider. A
   short help text explains the item.

### A live forecast

Run the w1 ensemble and score live, with a new runs folder:

```
target/release/firespread ensemble --window w1 --members 200 --seed 7 \
  --threads 12 --runs-dir data/runs/live --data data --config config/default.toml
target/release/firespread score --run data/runs/live/w1-seed7-m200 --data data
```

The ensemble prints its progress every 20 members and its run time at the end.

### Reproducibility

The same seed and inputs give the same files. After the live run, compare the
two w1 scorecards:

```
sha256sum data/runs/demo/w1-seed7-m200/scorecard.json data/runs/live/w1-seed7-m200/scorecard.json
```

The two hashes are equal.

### The honest scorecard

The uncalibrated kernel loses to persistence in every window, and to uniform
growth in w2 and w3. These are the Jaccard values of the runs above, measured
on 2026-10-26:

| Window | firespread | Persistence | Uniform growth |
|---|---|---|---|
| w1 | 0.407 | 0.416 | 0.400 |
| w2 | 0.729 | 0.821 | 0.823 |
| w3 | 0.671 | 0.894 | 0.851 |

The scorecard makes this result visible. That is its purpose.

### Sensitivity tables

Road barriers, firespread Jaccard (seed 7, 200 members):

| Road codes that do not burn | w1 | w2 |
|---|---|---|
| none | 0.373 | 0.724 |
| 1 to 3 | 0.373 | 0.724 |
| 1 to 4 | 0.407 | 0.729 |
| 1 to 5 | 0.397 | 0.765 |

The default is codes 1 to 4, chosen on w1 alone.

Night dead fuel moisture in w3, with road codes 1 to 4:

| Night setting | Jaccard | Brier |
|---|---|---|
| off | 0.652 | 0.211 |
| D1 | 0.655 | 0.210 |
| D2 | 0.665 | 0.203 |
| D3 | 0.671 | 0.199 |

### Tests

```
cargo nextest run --workspace
```

The suite checks the spread model against FireBench and pyretechnics
reference values. It also checks that the same seed gives byte-identical
scorecards.

## 6. Not built yet

These items are on the roadmap and are not in the demo:

- The FireBench HDF5 package of the case.
- Boundary distance, Hausdorff distance and buffered overlap metrics.
- Crown fire.
- Calibration of the spread multiplier and wind perturbation.
