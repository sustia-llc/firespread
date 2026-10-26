# Rothermel engine against its two oracles

PYRETECHNICS developed by Spatial Informatics Group, LLC and contributors.

This file records how `firespread-rothermel` compares with FireBench `Rothermel_SFIRE` and with pyretechnics. Only the single-class comparison with FireBench is a gate. The multi-class numbers are documented differences.

## Inputs

Both fixtures cover the 40 Scott and Burgan models. They use the dead moisture scenarios D1, D2 and D3 of Scott and Burgan 2005 Table 3 (p. 8). Wind is 0, 2, 5, 10 and 15 m/s, and slope is 0, 10, 30, 60 and 100 %. No wind limit is applied.

| fixture | generator | cases | oracle inputs |
|---|---|---|---|
| `firebench_single_class.json` | `python/oracle/firebench_rothermel_fixture.py` | 2615 | 1-h load and SAV only, the oracle's fixed heat. The oracle clamps 385 cases at 6 m/s, and the fixture omits them. |
| `pyretechnics_multi_class.json` | `python/oracle/pyretechnics_multi_class_fixture.py` | 3000 | The full fuel model, live scenario L2 (60 % herbaceous, 90 % woody), wind blowing upslope. |

Regenerate the fixtures with the venvs `~/.venvs/firespread` (FireBench) and `~/.venvs/pyretechnics`. Print the comparison tables with `cargo run -p firespread-rothermel --example oracle_differences`.

## Single-class mode against FireBench (gate, 1 %)

The test `single_class_matches_firebench_rothermel_sfire_within_1_percent` passes on all 2615 cases. The worst relative error is 0.316 %, for SH7 at zero wind and zero slope.

## Multi-class engine against pyretechnics (documented)

The relative error is (engine − oracle) / oracle.

| subset | cases | signed mean | max absolute |
|---|---|---|---|
| wind 0, slope 0 | 120 | not computed | 2.0e-2 (SH1) |
| wind only | 480 | −6.6e-4 | 3.1e-2 (SH1) |
| slope only | 480 | −4.6e-4 | 2.0e-2 (SH1) |
| wind and slope | 1920 | −1.26e-1 | 4.1e-1 (TU4, D1, 15 m/s, 100 %) |

With wind alone or slope alone, every model except SH1 agrees within 4.7e-3. With wind and slope together, the engine is lower than pyretechnics in all 1920 cases. The `oracle_differences` example prints this count after the split table. The engine computes R = R0 (1 + φw + φs), Rothermel 1972 eq. 52. Take GR1 at D1, 15 m/s and 100 % slope. There, R0 = 4.6025e-3 m/s, φw = 451.04 and φs = 37.60, so R = 2.2536 m/s. pyretechnics gives 3.7335 m/s for the same case. FireBench passes the 1 % gate on the single-class cases that combine wind and slope.

## Multi-class engine against FireBench (documented)

This compares the engine's full fuel model at L2 with FireBench's 1-h single class, over 2615 cases. The median absolute relative error is 0.678 and the maximum is 1.295 (GR6). The two compute different fuel beds.
