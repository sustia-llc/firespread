//! Run settings read from a TOML file, and the window list of `windows.json`.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

/// Road codes of `grid/roads.tif`: 1 motorway or trunk, 2 primary,
/// 3 secondary, 4 tertiary, 5 unclassified, residential or service, 6 other.
pub const ROAD_CODES: std::ops::RangeInclusive<u8> = 1..=6;

/// Settings of an `ensemble` run that the command line does not carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunSettings {
    /// Road codes made non-burnable; empty by default.
    pub road_barrier_codes: Vec<u8>,
    /// Forecast horizons, hours; default 1, 3, 6, 12.
    pub horizons_h: Vec<u32>,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            road_barrier_codes: Vec::new(),
            horizons_h: vec![1, 3, 6, 12],
        }
    }
}

impl RunSettings {
    /// The settings in the TOML file at `path`, or the defaults for `None`.
    /// Road codes must lie in [`ROAD_CODES`] and are returned sorted without
    /// repeats; the horizon list must be non-empty with every horizon > 0.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let settings = match path {
            None => Self::default(),
            Some(path) => config::Config::builder()
                .add_source(config::File::from(path).format(config::FileFormat::Toml))
                .build()
                .with_context(|| format!("reading {}", path.display()))?
                .try_deserialize::<Self>()
                .with_context(|| format!("parsing {}", path.display()))?,
        };
        settings.validated()
    }

    fn validated(mut self) -> Result<Self> {
        if let Some(bad) = self
            .road_barrier_codes
            .iter()
            .find(|c| !ROAD_CODES.contains(c))
        {
            bail!("road_barrier_codes holds {bad}, expected codes 1 to 6");
        }
        self.road_barrier_codes.sort_unstable();
        self.road_barrier_codes.dedup();
        ensure!(!self.horizons_h.is_empty(), "horizons_h is empty");
        ensure!(
            self.horizons_h.iter().all(|&h| h > 0),
            "horizons_h holds 0: {:?}",
            self.horizons_h
        );
        Ok(self)
    }
}

/// One forecast window of `windows.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Window {
    pub name: String,
    /// FEDS label of the start mask, `YYYY-MM-DDTHH:MM:SS`.
    pub label_start: String,
    /// FEDS label of the observation, `YYYY-MM-DDTHH:MM:SS`.
    pub label_target: String,
    pub start_t: String,
    pub target_t: String,
    pub fireids: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct Windows {
    windows: Vec<Window>,
}

/// The window `name` of `<data>/windows.json`.
pub fn load_window(data: &Path, name: &str) -> Result<Window> {
    let path = data.join("windows.json");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let windows: Windows =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    windows
        .windows
        .into_iter()
        .find(|w| w.name == name)
        .with_context(|| format!("window {name:?} is not in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, text: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "firespread-settings-{name}-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn toml_settings_are_read_sorted_and_checked() {
        assert_eq!(RunSettings::load(None).unwrap(), RunSettings::default());
        let path = write("ok", "road_barrier_codes = [3, 1, 2, 1]\n");
        let got = RunSettings::load(Some(&path)).unwrap();
        assert_eq!(got.road_barrier_codes, [1, 2, 3]);
        assert_eq!(got.horizons_h, [1, 3, 6, 12], "default horizons");

        let path = write("code7", "road_barrier_codes = [7]\n");
        let err = RunSettings::load(Some(&path)).unwrap_err();
        assert!(format!("{err:#}").contains("holds 7"), "{err:#}");

        let path = write("unknown", "road_barrier = [1]\n");
        assert!(RunSettings::load(Some(&path)).is_err(), "unknown key read");

        let path = write("h0", "horizons_h = [0, 12]\n");
        assert!(RunSettings::load(Some(&path)).is_err(), "horizon 0 read");
    }
}
