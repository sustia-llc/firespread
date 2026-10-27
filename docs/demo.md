# firespread demo

This guide prepares and runs the hackathon demo. Run every command from the
repository root.

## 1. Download and prepare the data

All data goes in `data/`, which git ignores. The prep scripts need uv and
network access. Create the Python environment once:

```
uv sync
```

### Files you download by hand

The prep scripts read five files that you download first.

The two official final perimeters come from the WFIGS interagency perimeter
service, as GeoJSON in EPSG:4326. OBJECTID 947 is Hermits Peak and OBJECTID
49422 is Calf Canyon:

```
curl -o data/wfigs/hermits_peak_947.geojson --create-dirs \
  "https://services3.arcgis.com/T4QMspbfLg3qTGWY/arcgis/rest/services/WFIGS_Interagency_Perimeters/FeatureServer/0/query?where=OBJECTID%3D947&outFields=*&f=geojson"
curl -o data/wfigs/calf_canyon_49422.geojson --create-dirs \
  "https://services3.arcgis.com/T4QMspbfLg3qTGWY/arcgis/rest/services/WFIGS_Interagency_Perimeters/FeatureServer/0/query?where=OBJECTID%3D49422&outFields=*&f=geojson"
```

The FEDS 2022 perimeters come from Zenodo record 22921773 (CC-BY-4.0). FEDS
is the NASA Fire Event Data Suite. The perimeter file is about 370 MB:

```
curl -L -o data/feds/feds_2022_lf_perimeter.fgb --create-dirs \
  "https://zenodo.org/records/22921773/files/feds_2022_lf_perimeter.fgb?download=1"
curl -L -o data/feds/feds_2022_lf_newfirepix.fgb --create-dirs \
  "https://zenodo.org/records/22921773/files/feds_2022_lf_newfirepix.fgb?download=1"
```

The 100-hour and 1000-hour dead fuel moisture come from gridMET, about 130 MB
each:

```
curl -o data/gridmet/fm100_2022.nc --create-dirs \
  https://www.northwestknowledge.net/metdata/data/fm100_2022.nc
curl -o data/gridmet/fm1000_2022.nc --create-dirs \
  https://www.northwestknowledge.net/metdata/data/fm1000_2022.nc
```

### Files the scripts download

Run the scripts in this order. Each script reads the output of the scripts
before it.

```
uv run python -m firespread_prep.extent
uv run python -m firespread_prep.feds_filter
uv run python -m firespread_prep.windows
uv run python -m firespread_prep.hrrr_subset
uv run python -m firespread_prep.gridmet_slice
uv run python -m firespread_prep.lfps_submit submit --email <your email>
uv run python -m firespread_prep.lfps_submit poll
uv run python -m firespread_prep.align data/landfire/<jobId>.zip
uv run python -m firespread_prep.feds_geojson
uv run python -m firespread_prep.rasterize
uv run python -m firespread_prep.roads_water
uv run python -m firespread_prep.weather_align
uv run python -m firespread_prep.hillshade
```

Three scripts download data:

- `hrrr_subset` gets the HRRR weather records from the NOAA S3 bucket.
- `lfps_submit` gets the LANDFIRE fuel, canopy and terrain layers from the
  LANDFIRE Product Service. The service needs an email address. `poll` waits
  for the job and saves the zip as `data/landfire/<jobId>.zip`. `<jobId>` is
  the ID that the service gives each request. `poll` prints the zip path, so
  give that path to `align`.
- `roads_water` gets roads from OpenStreetMap through Overpass, and water from
  the USGS National Hydrography Dataset.

`python/firespread_prep/README.md` describes each script and its outputs.

## 2. Build once

Build the `firespread` command and the `serve` example before the demo:

```
cargo build --release -p firespread
cargo build --release -p firespread --features serve --example serve
```

The command is then `target/release/firespread`, and the server is
`target/release/examples/serve`.

## 3. Prepare the three demo runs

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
49 s for w2 and 222 s for w3. Only w1 is fast enough to run live.

On the development machine, `data/runs/s4-calib/` already holds the three
runs, made with the same commands and configuration files, and scored. Their
scores are the ones in the README. To use them, put `data/runs/s4-calib` in
place of `data/runs/demo` in the server commands below.

## 4. Start the page

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

## 5. Street basemap (optional)

If `data/basemap/region.pmtiles` exists, the page draws a street basemap. If it
is missing, the page skips that layer. To make it, install the `pmtiles` CLI
(go-pmtiles) and run:

```
pmtiles extract https://build.protomaps.com/20261025.pmtiles \
  data/basemap/region.pmtiles \
  --bbox=-105.8431,35.5002,-104.9631,36.2037 --maxzoom=14
```

The box is the run grid from `data/extent.json`. The extract is about 3.4 MB.

## 6. What to show

### The page

1. Move the hour slider from h+1 to h+12. The colored area grows from the start
   perimeter. The start perimeter stays dark red, because every member burns it.
2. Turn the map layers on and off: observed perimeter, hillshade and street
   basemap.
3. Select the confusion map radio button. It replaces the burn probability
   map and shows where the forecast was right and wrong at the scored hour:
   green hits, blue misses and magenta false alarms. The hour slider is off
   while the confusion map shows. Select precision in the metric menu, and
   the misses fade, because precision does not count them. Select recall,
   and the false alarms fade. Select burn probability to go back. Runs scored
   before this layer existed have no confusion file. To add it, run
   `firespread score` on the run again.
4. Change the scorecard metric: Jaccard, Dice, precision, recall, Brier, mean
   boundary distance, Hausdorff distance, and buffered precision, recall and
   Dice. The chart compares firespread with persistence and uniform growth.
   The two distance metrics use an axis in meters. In w1, show mean boundary
   distance and buffered Dice: firespread is the best of the three entrants
   on both, but not on Jaccard.
5. Point to the WFIGS table: the share of each forecast inside the official
   final perimeter.
6. Point to the burned area chart: the ensemble band and median against the
   two observed areas.
7. Hover over or tap any "?" button, entrant name, layer name or the slider. A
   short help text explains the item.
8. Open the diagram links below the charts: input layers, cell logic and
   forecast hours. Each opens in a new tab. Start the server from the
   repository root, because it reads `docs/diagrams/` by default.

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

firespread has a lower Jaccard than persistence in every window. It has a
higher Jaccard than uniform growth in w1, and a lower one in w2 and w3. These
are the Jaccard values of the runs above, measured on 2026-10-26:

| Window | firespread | Persistence | Uniform growth |
|---|---|---|---|
| w1 | 0.408 | 0.416 | 0.400 |
| w2 | 0.735 | 0.821 | 0.823 |
| w3 | 0.679 | 0.894 | 0.851 |

The scorecard makes this result visible. That is its purpose.

### Calibration log

Each step changed one setting from the earlier defaults (spread multiplier
1.0, wind speed sd 0.2, wind direction sd 15°). Each row is the firespread
score against FEDS at h12 in w1, seed 7, 200 members, road codes 1 to 4,
measured on 2026-10-26. Only w1 was used to choose a value.

| Step | Jaccard | Brier | Precision | Recall |
|---|---|---|---|---|
| earlier defaults | 0.4068 | 0.1548 | 0.728 | 0.480 |
| spread multiplier 1.5 | 0.3897 | 0.1873 | 0.641 | 0.499 |
| spread multiplier 2 | 0.3725 | 0.2194 | 0.571 | 0.518 |
| spread multiplier 3 | 0.3407 | 0.2914 | 0.462 | 0.564 |
| spread multiplier 5 | 0.2984 | 0.4304 | 0.358 | 0.643 |
| wind speed sd 0.4 | 0.4084 | 0.1489 | 0.734 | 0.479 |
| wind direction sd 30° | 0.4048 | 0.1532 | 0.725 | 0.478 |
| wind speed sd 0.4 and direction sd 30° | 0.4065 | 0.1481 | 0.733 | 0.477 |

The rule was the highest Jaccard. Wind speed sd 0.4 won. It also had the
higher Jaccard at two more seeds on w1:

| Seed | Jaccard, sd 0.2 | Jaccard, sd 0.4 | Brier, sd 0.2 | Brier, sd 0.4 |
|---|---|---|---|---|
| 7 | 0.4068 | 0.4084 | 0.1548 | 0.1489 |
| 8 | 0.4076 | 0.4098 | 0.1542 | 0.1484 |
| 9 | 0.4081 | 0.4095 | 0.1551 | 0.1502 |

Both configuration files now set `wind_speed_sd = 0.4`. The spread
multiplier stays 1.0.

### Sensitivity tables

These tables were measured with wind speed sd 0.2, before the calibration.

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
cargo nextest run --workspace --features firespread/serve
```

The suite checks the spread model against FireBench and pyretechnics
reference values. It also checks that the same seed gives byte-identical
scorecards. The README, "Tests", gives the Python test commands.

### The FireBench bundle

The case is packaged in the FireBench standard format, so another model can
be scored on the same perimeters. The README, "FireBench bundle", gives the
commands. Show that the four files pass FireBench's validators, and that
FireBench's polygon Jaccard agrees with the scorecard within 0.0003 for all
three entrants.

## 7. Not built yet

These items are on the roadmap and are not in the demo:

- Crown fire.
- Lean proofs (Phase 5a, after the hackathon). The first proofs cover the
  metrics: Jaccard and Dice lie in [0, 1], and Dice equals 2J / (1 + J). Later
  proofs cover the Rothermel factor algebra and the unit conversions. Each
  theorem gets a Rust test that checks the engine against it.
