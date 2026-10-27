# Changelog

## Unreleased

The first release, planned as `v0.1.0` after the hackathon review.

### Added

- Fuel: the 40 Scott and Burgan (2005) fuel models in SI units, with the
  transfer of live herbaceous load to dead. GR6 uses a heat content of
  9000 Btu/lb.
- Rothermel: the surface spread model of Rothermel (1972) as Andrews (2018)
  documents it, with directional spread on the Finney (1998) ellipse. A
  single-class mode matches FireBench `Rothermel_SFIRE` within 1 %.
- Spread: minimum travel time over a 32-neighbor stencil.
- Ensemble: hourly HRRR weather, the wind adjustment factor, hourly 1-h and
  10-h dead fuel moisture, seeded per-member perturbations, and a parallel
  run with byte-identical output for the same seed.
- Metrics: Jaccard, Dice, precision, recall, Brier, mean boundary distance,
  Hausdorff distance, and buffered precision, recall and Dice. The
  persistence and uniform growth baselines.
- Command `firespread ensemble`, with a TOML configuration: road barriers,
  forecast hours, night dead fuel moisture, spread multipliers and the
  perturbation table.
- Command `firespread score`: `scorecard.json`, `area_by_hour.json`, the
  confusion map `confusion_h<N>.tif` and the uniform growth mask
  `uniform_growth_h<N>.tif`.
- Configuration files `config/default.toml` and `config/w3.toml`, with the
  wind speed perturbation sd of 0.4 calibrated on window w1.
- Detection mesh demo: `serve --bind` and an in-memory `/api/alerts`, the
  dashboard `detect.html` that triangulates smoke from node bearings, the
  phone node `node.html`, and the `python/edge_node` package (a YOLO smoke
  detector over video, and a scripted scenario). See `docs/edge-detect.md`.
- Static page and its `serve` example: burn probability by hour, the
  confusion map, the observed perimeter, hillshade, street basemap, the
  scorecard chart, the WFIGS table, the burned area chart, help texts and
  links to the diagrams.
- Python data preparation for the Hermits Peak and Calf Canyon case.
- The FireBench standard-format bundle of the case, and its check against
  the scorecards.
- License: MIT OR Apache-2.0.
