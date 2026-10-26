# Hackathon memo, 2026-10-27

This memo holds the freeze checklist, the claims that the release review
checks, and the review ledger. The review is `/code-review medium
freeze/hackathon`, run until no finding refutes a claim. Then the tag
`v0.1.0` follows.

## Freeze checklist

Before 14:30 on 2026-10-27:

- [ ] Push `main`. The push closes issues #1 to #6.
- [ ] Open the page in a browser on w1 and make sure that these work:
  - the burn probability slider and the confusion map radio pair
  - the five tolerant metrics in the metric menu, with the meter axis for the
    two distance metrics
  - the three diagram links
  - the confusion map colors on the hillshade
- [ ] Choose the demo runs: serve `data/runs/s4-calib/`, or build
  `data/runs/demo/` with `docs/demo.md` section 2.
- [ ] Optional: issue #7 (page fixture) and issue #8 (`firebench_case`
  install).
- [ ] 14:30: create the branch `freeze/hackathon` from `main`.
- [ ] Take screenshots: the w1 probability map at h+12, the w1 confusion map,
  the scorecard at Jaccard and at buffered Dice.
- [ ] Copy the score tables from the README into the pitch.

## Claims

Each claim names the command that shows it.

| Id | Claim | Command |
|---|---|---|
| C1 | The same seed and inputs give a byte-identical `scorecard.json`. | Run w1 twice with seed 7 into two runs folders, then `sha256sum` both scorecards. |
| C2 | The w1, w2 and w3 scorecards of `data/runs/s4-calib` hash to `40f7baa3…21a3`, `cf26589a…0df20` and `de186c63…9275`. | `sha256sum data/runs/s4-calib/*/scorecard.json` |
| C3 | firespread has a lower Jaccard than persistence in w1, w2 and w3, and a higher Jaccard than uniform growth in w1 only. | The `feds` rows of the three scorecards. |
| C4 | In w1, firespread has the lowest mean boundary distance and the highest buffered Dice of the three entrants. | The `feds` rows of the w1 scorecard. |
| C5 | The wind speed sd 0.4 was chosen on w1 alone, and it has the higher w1 Jaccard at seeds 7, 8 and 9. | The calibration log in `docs/demo.md`. |
| C6 | The four FireBench files pass `validate_h5_std`, `validate_h5_referenced_files` and `validate_h5_requirement`. | `python -m firebench_case.bundle` output. |
| C7 | FireBench's polygon Jaccard agrees with the raster Jaccard within 0.0003 for every entrant and window. | `python -m firebench_case.check` |
| C8 | The single-class Rothermel mode matches FireBench `Rothermel_SFIRE` within 1 % on 2615 cases. | `single_class_matches_firebench_rothermel_sfire_within_1_percent` |
| C9 | The w1 ensemble of 200 members runs in under 30 s on the 12-thread development machine. | The run time that `firespread ensemble` prints. |

## Review ledger

| Round | Finding | Ruling | Commit or issue |
|---|---|---|---|
