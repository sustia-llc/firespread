//! Versioned JSON manifest of one forecast run.
//!
//! The JSON layout is described by `schema/manifest.schema.json` in this crate.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{GridError, GridHeader, Result};

/// The manifest version this build reads and writes.
pub const MANIFEST_VERSION: u32 = 1;

/// The wind limit a run passes to the spread kernel, one variant per
/// `firespread_rothermel::WindLimit` variant; serialized in snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindLimitPolicy {
    /// `WindLimit::None`.
    None,
    /// `WindLimit::Rothermel`.
    Rothermel,
    /// `WindLimit::EffectiveWind`.
    EffectiveWind,
}

/// A named forecast window: start and target times as ISO 8601 strings, and a
/// list of fire identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastWindow {
    pub name: String,
    pub start_t: String,
    pub target_t: String,
    pub fireids: Vec<u32>,
}

/// Inputs, parameters and outputs of one forecast run.
///
/// `inputs` and `outputs` map a layer name to a file path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub run_id: String,
    pub window: ForecastWindow,
    pub grid: GridHeader,
    pub inputs: BTreeMap<String, String>,
    pub hrrr_source: String,
    pub seed: u64,
    pub wind_limit: WindLimitPolicy,
    pub members: u32,
    pub horizons_h: Vec<u32>,
    pub outputs: BTreeMap<String, String>,
}

impl Manifest {
    /// Reads a manifest from JSON, failing when `version` is not [`MANIFEST_VERSION`].
    pub fn read(path: impl AsRef<Path>) -> Result<Manifest> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| GridError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let manifest: Manifest = serde_json::from_str(&text).map_err(|source| GridError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        if manifest.version != MANIFEST_VERSION {
            return Err(GridError::UnsupportedManifestVersion {
                path: path.to_path_buf(),
                found: manifest.version,
                expected: MANIFEST_VERSION,
            });
        }
        Ok(manifest)
    }

    /// Writes the manifest as pretty-printed JSON with a trailing newline.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let mut text = serde_json::to_string_pretty(self).map_err(|source| GridError::Json {
            path: path.to_path_buf(),
            source,
        })?;
        text.push('\n');
        std::fs::write(path, text).map_err(|source| GridError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}
