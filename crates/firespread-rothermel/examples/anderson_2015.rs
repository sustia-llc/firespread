//! Adds `predicted_ros_m_s` from `single_class` to each Anderson et al. (2015)
//! input record. Arguments: input JSON path (default
//! `data/anderson_2015/inputs.json` under the workspace root) and output JSON
//! path (default `data/anderson_2015/predictions.json` under the workspace
//! root). A record on which `single_class`
//! errors gets `predicted_ros_m_s: null` and an `error` string. Prints the ok
//! and error counts, and the count and results of records whose moisture is at
//! or above the dead moisture of extinction.

use std::path::PathBuf;

use anyhow::{Context, Result};
use firespread_fuel::units;
use serde_json::{Map, Value};

const DEFAULT_IN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/anderson_2015/inputs.json"
);
const DEFAULT_OUT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/anderson_2015/predictions.json"
);

fn field(record: &Map<String, Value>, name: &str) -> Result<f64> {
    record
        .get(name)
        .and_then(Value::as_f64)
        .with_context(|| format!("record field {name} missing or not a number: {record:?}"))
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT_IN.to_owned()));
    let output = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT_OUT.to_owned()));

    let text =
        std::fs::read_to_string(&input).with_context(|| format!("reading {}", input.display()))?;
    let mut doc: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", input.display()))?;
    let records = doc
        .get_mut("records")
        .and_then(Value::as_array_mut)
        .context("top-level \"records\" array missing")?;

    let (mut ok, mut err) = (0usize, 0usize);
    let mut extinct: Vec<String> = Vec::new();
    for value in records.iter_mut() {
        let record = value
            .as_object_mut()
            .context("record is not a JSON object")?;
        let moisture = field(record, "moisture_fraction")?;
        let mx = field(record, "mx_fraction")?;
        let result = firespread_rothermel::single_class(
            units::per_ft_to_per_m(field(record, "sav_per_ft")?),
            field(record, "load_kg_m2")?,
            field(record, "depth_m")?,
            mx,
            units::btu_lb_to_j_kg(field(record, "heat_btu_lb")?),
            moisture,
            field(record, "midflame_wind_m_s")?,
            field(record, "slope_tan")?,
        );
        let outcome = match result {
            Ok(ros) => {
                ok += 1;
                record.insert("predicted_ros_m_s".to_owned(), Value::from(ros));
                format!("Ok({ros:e})")
            }
            Err(e) => {
                err += 1;
                record.insert("predicted_ros_m_s".to_owned(), Value::Null);
                record.insert("error".to_owned(), Value::from(e.to_string()));
                format!("Err({e})")
            }
        };
        if moisture >= mx {
            let id = format!(
                "{} row {}",
                record.get("source").unwrap_or(&Value::Null),
                record.get("row").unwrap_or(&Value::Null)
            );
            extinct.push(format!("{id}: moisture {moisture} >= mx {mx}: {outcome}"));
        }
    }

    let out = serde_json::to_string_pretty(&doc)?;
    std::fs::write(&output, out + "\n").with_context(|| format!("writing {}", output.display()))?;
    println!("ok {ok}, err {err}");
    println!("moisture >= mx: {}", extinct.len());
    for line in &extinct {
        println!("  {line}");
    }
    Ok(())
}
