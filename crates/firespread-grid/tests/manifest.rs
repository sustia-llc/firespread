//! Manifest round trip and agreement with the hand-written JSON Schema.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use firespread_grid::{
    ForecastWindow, GridError, GridHeader, MANIFEST_VERSION, Manifest, WindLimitPolicy,
};
use serde_json::Value;

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("firespread-grid-manifest");
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn schema() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schema")
        .join("manifest.schema.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn sample() -> Manifest {
    Manifest {
        version: MANIFEST_VERSION,
        run_id: "hpcc-2022-04-22T12".to_string(),
        window: ForecastWindow {
            name: "w01".to_string(),
            start_t: "2022-04-22T12:00:00Z".to_string(),
            target_t: "2022-04-23T00:00:00Z".to_string(),
            fireids: vec![17, 4_000_000_000],
        },
        grid: GridHeader {
            nx: 2400,
            ny: 2400,
            dx: 30.0,
            dy: 30.0,
            origin_x: -876_180.0,
            origin_y: 1_499_820.0,
            epsg: 5070,
            nodata: -9999.0,
        },
        inputs: BTreeMap::from([
            ("FBFM40".to_string(), "data/grid/FBFM40.tif".to_string()),
            ("SlpD".to_string(), "data/grid/SlpD.tif".to_string()),
        ]),
        hrrr_source: "noaa-hrrr-bdp-pds".to_string(),
        seed: u64::MAX,
        wind_limit: WindLimitPolicy::EffectiveWind,
        members: 200,
        horizons_h: vec![1, 3, 6, 12],
        outputs: BTreeMap::from([("p_burn_12h".to_string(), "out/p12.tif".to_string())]),
    }
}

#[test]
fn round_trip_is_equal() {
    let m = sample();
    let path = tmp("roundtrip.json");
    m.write(&path).unwrap();
    assert_eq!(Manifest::read(&path).unwrap(), m);
}

#[test]
fn wind_limit_serializes_snake_case() {
    for (policy, want) in [
        (WindLimitPolicy::None, "none"),
        (WindLimitPolicy::Rothermel, "rothermel"),
        (WindLimitPolicy::EffectiveWind, "effective_wind"),
    ] {
        assert_eq!(serde_json::to_value(policy).unwrap(), Value::from(want));
    }
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

fn required(schema: &Value) -> BTreeSet<String> {
    schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap().to_string())
        .collect()
}

/// Checks `required` and `properties` against the serialized object at every
/// level whose schema lists `properties`, and returns how many levels it checked.
fn check_object(schema: &Value, instance: &Value, at: &str) -> usize {
    assert_eq!(schema["type"], "object", "{at}: type");
    assert_eq!(
        schema["additionalProperties"],
        Value::Bool(false),
        "{at}: additionalProperties"
    );
    let serialized = keys(instance);
    assert_eq!(required(schema), serialized, "{at}: required vs serialized");
    assert_eq!(
        keys(&schema["properties"]),
        serialized,
        "{at}: properties vs serialized"
    );
    let mut levels = 1;
    for (name, sub) in schema["properties"].as_object().unwrap() {
        let child = &instance[name];
        if sub.get("properties").is_some() {
            levels += check_object(sub, child, &format!("{at}.{name}"));
        } else if sub["type"] == "object" {
            assert!(child.is_object(), "{at}.{name}: serialized as {child}");
            assert_eq!(
                sub["additionalProperties"]["type"], "string",
                "{at}.{name}: map values"
            );
        }
    }
    levels
}

#[test]
fn schema_keys_match_serialized_keys() {
    let schema = schema();
    let instance = serde_json::to_value(sample()).unwrap();
    let levels = check_object(&schema, &instance, "$");
    assert_eq!(levels, 3, "root, window and grid");
    let variants: Vec<&str> = schema["properties"]["wind_limit"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(variants, ["none", "rothermel", "effective_wind"]);
    assert_eq!(
        schema["properties"]["version"]["const"],
        Value::from(MANIFEST_VERSION)
    );
}

fn read_err(name: &str, text: &str) -> GridError {
    let path = tmp(name);
    fs::write(&path, text).unwrap();
    match Manifest::read(&path) {
        Ok(m) => panic!("{name} read as {m:?}"),
        Err(e) => e,
    }
}

#[test]
fn other_version_is_rejected() {
    let mut v = serde_json::to_value(sample()).unwrap();
    v["version"] = Value::from(2);
    let e = read_err("v2.json", &v.to_string());
    assert!(
        matches!(
            e,
            GridError::UnsupportedManifestVersion {
                found: 2,
                expected: 1,
                ..
            }
        ),
        "{e:?}"
    );
}

#[test]
fn unknown_field_and_malformed_json_are_json_errors() {
    let mut v = serde_json::to_value(sample()).unwrap();
    v["grid"]["extra"] = Value::from(1);
    let e = read_err("extra.json", &v.to_string());
    assert!(matches!(e, GridError::Json { .. }), "{e:?}");
    let e = read_err("broken.json", "{");
    assert!(matches!(e, GridError::Json { .. }), "{e:?}");
}

#[test]
fn missing_manifest_is_io_error() {
    let e = Manifest::read(tmp("absent.json")).unwrap_err();
    assert!(matches!(e, GridError::Io { .. }), "{e:?}");
}
