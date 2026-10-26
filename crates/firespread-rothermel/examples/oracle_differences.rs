//! Prints markdown tables of the relative differences between the multi-class
//! `spread` rate of spread and two oracle fixtures: pyretechnics multi-class
//! `max_spread_rate_m_s` and FireBench `Rothermel_SFIRE` single-class `ros_m_s`.
//!
//! PYRETECHNICS developed by Spatial Informatics Group, LLC and contributors.
//!
//! `rel = (engine - oracle) / oracle` over cases with `oracle > 0`. Cases with
//! `oracle == 0` are counted separately and listed when the engine is positive.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use firespread_fuel::Moistures;
use firespread_rothermel::{Environment, FuelBed, WindLimit};
use serde::Deserialize;

const PYRE: &str = include_str!("../tests/fixtures/pyretechnics_multi_class.json");
const FB: &str = include_str!("../tests/fixtures/firebench_single_class.json");
const LIVE_SCENARIO: &str = "L2";
const SCENARIOS: [&str; 3] = ["D1", "D2", "D3"];

#[derive(Deserialize)]
struct PyreFixture {
    oracle: PyreOracle,
    cases: Vec<PyreCase>,
}

#[derive(Deserialize)]
struct PyreOracle {
    dead_scenarios_percent: BTreeMap<String, [f64; 3]>,
    live_scenario_percent: BTreeMap<String, [f64; 2]>,
}

#[derive(Deserialize)]
struct PyreCase {
    model: String,
    dead_scenario: String,
    wind_m_s: f64,
    slope_percent: f64,
    base_spread_rate_m_s: f64,
    max_spread_rate_m_s: f64,
}

#[derive(Deserialize)]
struct FbFixture {
    cases: Vec<FbCase>,
}

#[derive(Deserialize)]
struct FbCase {
    model: String,
    moisture_scenario: String,
    wind_m_s: f64,
    slope_percent: f64,
    ros_m_s: f64,
}

type Key = (String, String, u64, u64);

fn key(model: &str, scenario: &str, wind: f64, slope: f64) -> Key {
    (
        model.to_owned(),
        scenario.to_owned(),
        wind.to_bits(),
        slope.to_bits(),
    )
}

/// One relative difference and where it was observed.
#[derive(Clone)]
struct Rel {
    rel: f64,
    model: String,
    scenario: String,
    wind: f64,
    slope: f64,
}

impl Rel {
    fn place(&self, with_model: bool) -> String {
        if with_model {
            format!(
                "{} {} {} m/s {} %",
                self.model, self.scenario, self.wind, self.slope
            )
        } else {
            format!("{} {} m/s {} %", self.scenario, self.wind, self.slope)
        }
    }
}

/// A case whose oracle rate of spread is zero.
struct Zero {
    source: &'static str,
    at: Rel,
    engine: f64,
}

#[derive(Default)]
struct Stats {
    pyre: Vec<Rel>,
    fb: Vec<Rel>,
}

fn median_abs(rels: &[Rel]) -> Option<f64> {
    let mut abs: Vec<f64> = rels.iter().map(|r| r.rel.abs()).collect();
    if abs.is_empty() {
        return None;
    }
    abs.sort_by(f64::total_cmp);
    let n = abs.len();
    Some(if n % 2 == 1 {
        abs[n / 2]
    } else {
        f64::midpoint(abs[n / 2 - 1], abs[n / 2])
    })
}

fn max_abs(rels: &[Rel]) -> Option<&Rel> {
    rels.iter()
        .max_by(|a, b| a.rel.abs().total_cmp(&b.rel.abs()))
}

#[expect(clippy::cast_precision_loss, reason = "case counts are far below 2^52")]
fn signed_mean(rels: &[Rel]) -> Option<f64> {
    if rels.is_empty() {
        None
    } else {
        Some(rels.iter().map(|r| r.rel).sum::<f64>() / rels.len() as f64)
    }
}

fn sci(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |x| format!("{x:.3e}"))
}

fn row(name: &str, s: &Stats, with_model: bool) -> String {
    let pyre_max = max_abs(&s.pyre).map_or_else(
        || "-".to_owned(),
        |r| format!("{:.3e} ({})", r.rel.abs(), r.place(with_model)),
    );
    format!(
        "| {name} | {} | {} | {pyre_max} | {} | {} | {} | {} |",
        s.pyre.len(),
        sci(median_abs(&s.pyre)),
        sci(signed_mean(&s.pyre)),
        s.fb.len(),
        sci(median_abs(&s.fb)),
        sci(max_abs(&s.fb).map(|r| r.rel.abs())),
    )
}

/// The pyretechnics subsets of the split table: wind only, slope only, both.
type Split = (&'static str, fn(&Rel) -> bool);

const SPLITS: [Split; 3] = [
    ("wind only", |r| r.slope == 0.0 && r.wind > 0.0),
    ("slope only", |r| r.wind == 0.0 && r.slope > 0.0),
    ("both", |r| r.wind > 0.0 && r.slope > 0.0),
];

fn split_row(name: &str, rels: &[Rel]) -> String {
    let cells: Vec<String> = SPLITS
        .iter()
        .map(|(_, keep)| {
            let subset: Vec<Rel> = rels.iter().filter(|r| keep(r)).cloned().collect();
            format!(
                "{} | {} | {}",
                subset.len(),
                sci(signed_mean(&subset)),
                sci(max_abs(&subset).map(|r| r.rel.abs()))
            )
        })
        .collect();
    format!("| {name} | {} |", cells.join(" | "))
}

/// One GR1 D1 case with the engine terms.
struct Gr1Row {
    wind: f64,
    slope: f64,
    engine: f64,
    oracle: f64,
    phi_w: f64,
    phi_s: f64,
    r0: f64,
}

fn main() -> Result<()> {
    let pyre: PyreFixture = serde_json::from_str(PYRE).context("parsing pyretechnics fixture")?;
    let fb: FbFixture = serde_json::from_str(FB).context("parsing FireBench fixture")?;
    let fb_ros: HashMap<Key, f64> = fb
        .cases
        .iter()
        .map(|c| {
            (
                key(&c.model, &c.moisture_scenario, c.wind_m_s, c.slope_percent),
                c.ros_m_s,
            )
        })
        .collect();
    let [live_herb, live_woody] = *pyre
        .oracle
        .live_scenario_percent
        .get(LIVE_SCENARIO)
        .context("live scenario L2 missing from the pyretechnics header")?;

    let codes: Vec<String> = firespread_fuel::scott_burgan_40()?
        .into_iter()
        .map(|m| m.code)
        .collect();
    let mut models = BTreeMap::new();
    for code in &codes {
        models.insert(code.clone(), firespread_fuel::by_code(code)?.to_si());
    }

    let mut stats: BTreeMap<String, Stats> = BTreeMap::new();
    let mut all = Stats::default();
    let mut zeros: Vec<Zero> = Vec::new();
    let mut fb_missing = 0usize;
    // R0 per (model, scenario): (engine, oracle).
    let mut r0: BTreeMap<(String, String), (f64, f64)> = BTreeMap::new();
    let mut gr1_d1: Vec<Gr1Row> = Vec::new();

    for case in &pyre.cases {
        let model = models
            .get(&case.model)
            .with_context(|| format!("unknown model {}", case.model))?;
        let [d1, d10, d100] = *pyre
            .oracle
            .dead_scenarios_percent
            .get(&case.dead_scenario)
            .with_context(|| format!("unknown dead scenario {}", case.dead_scenario))?;
        let moistures = Moistures {
            dead_1h: d1 / 100.0,
            dead_10h: d10 / 100.0,
            dead_100h: d100 / 100.0,
            live_herb: live_herb / 100.0,
            live_woody: live_woody / 100.0,
        };
        let bed = FuelBed::from_model(model, &moistures)?;
        let env = Environment {
            midflame_wind_m_s: case.wind_m_s,
            slope_tan: case.slope_percent / 100.0,
        };
        let out = firespread_rothermel::spread(&bed, &env, WindLimit::None)?;
        let at = |rel| Rel {
            rel,
            model: case.model.clone(),
            scenario: case.dead_scenario.clone(),
            wind: case.wind_m_s,
            slope: case.slope_percent,
        };
        let entry = stats.entry(case.model.clone()).or_default();

        let oracle = case.max_spread_rate_m_s;
        if case.model == "GR1" && case.dead_scenario == "D1" {
            gr1_d1.push(Gr1Row {
                wind: case.wind_m_s,
                slope: case.slope_percent,
                engine: out.ros_m_s,
                oracle,
                phi_w: out.phi_w,
                phi_s: out.phi_s,
                r0: out.ros_no_wind_no_slope_m_s,
            });
        }
        if oracle > 0.0 {
            let r = at((out.ros_m_s - oracle) / oracle);
            entry.pyre.push(r.clone());
            all.pyre.push(r);
        } else {
            zeros.push(Zero {
                source: "pyre",
                at: at(0.0),
                engine: out.ros_m_s,
            });
        }

        match fb_ros.get(&key(
            &case.model,
            &case.dead_scenario,
            case.wind_m_s,
            case.slope_percent,
        )) {
            None => fb_missing += 1,
            Some(&oracle) if oracle > 0.0 => {
                let r = at((out.ros_m_s - oracle) / oracle);
                entry.fb.push(r.clone());
                all.fb.push(r);
            }
            Some(_) => zeros.push(Zero {
                source: "fb",
                at: at(0.0),
                engine: out.ros_m_s,
            }),
        }

        if case.wind_m_s == 0.0 && case.slope_percent == 0.0 {
            r0.insert(
                (case.model.clone(), case.dead_scenario.clone()),
                (out.ros_no_wind_no_slope_m_s, case.base_spread_rate_m_s),
            );
        }
    }

    println!("# Engine multi-class `spread` against the oracle fixtures");
    println!();
    println!("PYRETECHNICS developed by Spatial Informatics Group, LLC and contributors.");
    println!();
    println!(
        "Engine: `FuelBed::from_model` at the pyretechnics header's dead scenario and live \
         scenario {LIVE_SCENARIO}, `spread` with `WindLimit::None`. rel = (engine - oracle) / \
         oracle over cases with oracle > 0, as a fraction."
    );
    println!();
    println!("## Rate of spread");
    println!();
    println!(
        "| model | pyre_n | pyre median abs rel | pyre max abs rel (at) | pyre signed mean rel \
         | fb_n | fb median abs rel | fb max abs rel |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    let empty = Stats::default();
    for code in &codes {
        println!("{}", row(code, stats.get(code).unwrap_or(&empty), false));
    }
    println!("{}", row("all", &all, true));
    println!();
    println!("FireBench cases absent from its fixture (clamped at 6 m/s), skipped: {fb_missing}");
    println!();
    println!("## Oracle zeros");
    println!();
    let n0 = |source| zeros.iter().filter(|z| z.source == source).count();
    println!("pyre n0: {}; fb n0: {}", n0("pyre"), n0("fb"));
    println!();
    let positive: Vec<&Zero> = zeros.iter().filter(|z| z.engine > 0.0).collect();
    if positive.is_empty() {
        println!("No case with oracle == 0 has engine > 0.");
    } else {
        println!("| source | case | engine ros m/s |");
        println!("|---|---|---|");
        for z in positive {
            println!("| {} | {} | {:.6e} |", z.source, z.at.place(true), z.engine);
        }
    }
    println!();
    println!(
        "## R0 (wind 0, slope 0): engine `ros_no_wind_no_slope_m_s` vs pyre `base_spread_rate_m_s`"
    );
    println!();
    println!("| model | D1 rel | D2 rel | D3 rel |");
    println!("|---|---|---|---|");
    for code in &codes {
        let cells: Vec<String> = SCENARIOS
            .iter()
            .map(|s| match r0.get(&(code.clone(), (*s).to_owned())) {
                None => "-".to_owned(),
                Some(&(engine, oracle)) if oracle > 0.0 => {
                    format!("{:.3e}", (engine - oracle) / oracle)
                }
                Some(&(engine, _)) => format!("oracle 0, engine {engine:.3e}"),
            })
            .collect();
        println!("| {code} | {} |", cells.join(" | "));
    }
    println!();
    println!("## pyre split by term");
    println!();
    println!(
        "Subsets of the pyretechnics comparison: wind only (slope 0, wind > 0), slope only \
         (wind 0, slope > 0), both (wind > 0, slope > 0)."
    );
    println!();
    let headers: Vec<String> = SPLITS
        .iter()
        .map(|(name, _)| format!("{name} n | {name} signed mean rel | {name} max abs rel"))
        .collect();
    println!("| model | {} |", headers.join(" | "));
    println!("|---|---|---|---|---|---|---|---|---|---|");
    for code in &codes {
        println!(
            "{}",
            split_row(code, &stats.get(code).unwrap_or(&empty).pyre)
        );
    }
    println!("{}", split_row("all", &all.pyre));
    println!();
    let [_, _, (_, both)] = SPLITS;
    let (above, below): (Vec<&Rel>, Vec<&Rel>) = all
        .pyre
        .iter()
        .filter(|r| both(r))
        .partition(|r| r.rel >= 0.0);
    let n_both = above.len() + below.len();
    println!(
        "Both (wind and slope): engine >= oracle in {} of {n_both} cases; engine < oracle in {} \
         of {n_both} cases.",
        above.len(),
        below.len()
    );
    println!();
    println!("## GR1, D1: every case");
    println!();
    println!(
        "| wind m/s | slope % | engine ros m/s | oracle ros m/s | rel | engine phi_w | engine phi_s \
         | engine ros_no_wind_no_slope m/s |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for r in &gr1_d1 {
        let rel = if r.oracle > 0.0 {
            format!("{:.3e}", (r.engine - r.oracle) / r.oracle)
        } else {
            "oracle 0".to_owned()
        };
        println!(
            "| {} | {} | {:.6e} | {:.6e} | {rel} | {:.6e} | {:.6e} | {:.6e} |",
            r.wind, r.slope, r.engine, r.oracle, r.phi_w, r.phi_s, r.r0
        );
    }
    Ok(())
}
