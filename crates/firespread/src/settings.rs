//! Run settings read from a TOML file, and the window list of `windows.json`.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use firespread_ensemble::{NightMoisture, NightScenario};
use serde::{Deserialize, Serialize};

/// Road codes of `grid/roads.tif`: 1 motorway or trunk, 2 primary,
/// 3 secondary, 4 tertiary, 5 unclassified, residential or service, 6 other.
pub const ROAD_CODES: std::ops::RangeInclusive<u8> = 1..=6;

/// Dead fuel moisture scenario of the `[night_moisture]` table: one of
/// Scott and Burgan (2005) RMRS-GTR-153 Table 3 (p. 8), or `custom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scenario {
    D1,
    D2,
    D3,
    #[serde(rename = "custom")]
    Custom,
}

/// The `[night_moisture]` table: dead fuel moisture held at `scenario`'s
/// values over the night period `start_local` to `end_local` (`HH:MM`,
/// local time at UTC offset `utc_offset_h`). The default period 20:00 to
/// 07:59 is the night time of Rothermel (1983) INT-143 Table E (p. 19).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NightMoistureSettings {
    pub scenario: Scenario,
    /// Dead 1-h, 10-h and 100-h moisture, percent, each finite and in
    /// (0, 100]; given exactly when `scenario` is `custom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_pct: Option<Vec<f64>>,
    /// Start of the night period, `HH:MM`; default `20:00`.
    #[serde(default = "default_start_local")]
    pub start_local: String,
    /// End of the night period, `HH:MM`; default `07:59`.
    #[serde(default = "default_end_local")]
    pub end_local: String,
    /// Hours added to UTC to give local time, in [-12, 14]; default -6.
    #[serde(default = "default_utc_offset_h")]
    pub utc_offset_h: i32,
}

fn default_start_local() -> String {
    "20:00".to_string()
}

fn default_end_local() -> String {
    "07:59".to_string()
}

fn default_utc_offset_h() -> i32 {
    -6
}

/// Minutes after midnight of `HH:MM` with `HH` 00-23 and `MM` 00-59.
fn parse_hh_mm(field: &str, text: &str) -> Result<u32> {
    let b = text.as_bytes();
    let digit = |i: usize| char::from(b[i]).to_digit(10);
    let parsed = (b.len() == 5 && b[2] == b':')
        .then(|| Some((digit(0)? * 10 + digit(1)?, digit(3)? * 10 + digit(4)?)))
        .flatten();
    match parsed {
        Some((h, m)) if h < 24 && m < 60 => Ok(h * 60 + m),
        _ => bail!("night_moisture.{field} is {text:?}, expected HH:MM with HH 00-23, MM 00-59"),
    }
}

impl NightMoistureSettings {
    /// The engine setting: the scenario's dead moisture (the Table 3 values
    /// for D1 to D3, `custom_pct` for `custom`) over the parsed period.
    pub fn to_night(&self) -> Result<NightMoisture> {
        let dead_pct = match (self.scenario, &self.custom_pct) {
            (Scenario::Custom, Some(pct)) => {
                let Ok(pct) = <[f64; 3]>::try_from(pct.as_slice()) else {
                    bail!(
                        "night_moisture.custom_pct holds {} values, expected 3",
                        pct.len()
                    );
                };
                if let Some(bad) = pct
                    .iter()
                    .find(|v| !(v.is_finite() && **v > 0.0 && **v <= 100.0))
                {
                    bail!("night_moisture.custom_pct holds {bad}, expected a percent in (0, 100]");
                }
                pct
            }
            (Scenario::Custom, None) => {
                bail!("night_moisture.custom_pct is required for scenario \"custom\"")
            }
            (s, Some(_)) => {
                bail!("night_moisture.custom_pct is given for scenario {s:?}, expected \"custom\"")
            }
            (Scenario::D1, None) => NightScenario::D1.dead_pct(),
            (Scenario::D2, None) => NightScenario::D2.dead_pct(),
            (Scenario::D3, None) => NightScenario::D3.dead_pct(),
        };
        let start_local_min = parse_hh_mm("start_local", &self.start_local)?;
        let end_local_min = parse_hh_mm("end_local", &self.end_local)?;
        ensure!(
            start_local_min != end_local_min,
            "night_moisture.start_local equals end_local ({})",
            self.start_local
        );
        ensure!(
            (-12..=14).contains(&self.utc_offset_h),
            "night_moisture.utc_offset_h is {}, expected -12 to 14",
            self.utc_offset_h
        );
        Ok(NightMoisture {
            dead_pct,
            start_local_min,
            end_local_min,
            utc_offset_h: self.utc_offset_h,
        }
        .validated()?)
    }
}

/// Settings of an `ensemble` run that the command line does not carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunSettings {
    /// Road codes made non-burnable; empty by default.
    pub road_barrier_codes: Vec<u8>,
    /// Forecast horizons, hours; default 1, 3, 6, 12.
    pub horizons_h: Vec<u32>,
    /// Night dead fuel moisture; off when absent.
    pub night_moisture: Option<NightMoistureSettings>,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            road_barrier_codes: Vec::new(),
            horizons_h: vec![1, 3, 6, 12],
            night_moisture: None,
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
        if let Some(night) = &self.night_moisture {
            night.to_night()?;
        }
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

    fn night(name: &str, table: &str) -> Result<NightMoisture> {
        let path = write(name, &format!("[night_moisture]\n{table}"));
        let settings = RunSettings::load(Some(&path))?;
        settings
            .night_moisture
            .context("night_moisture absent")?
            .to_night()
    }

    #[test]
    fn night_moisture_table_is_read_with_defaults() {
        assert_eq!(RunSettings::default().night_moisture, None, "on by default");
        for (scenario, want) in [
            ("D1", [3.0, 4.0, 5.0]),
            ("D2", [6.0, 7.0, 8.0]),
            ("D3", [9.0, 10.0, 11.0]),
        ] {
            let got = night(scenario, &format!("scenario = \"{scenario}\"\n")).unwrap();
            assert_eq!(
                got,
                NightMoisture {
                    dead_pct: want,
                    start_local_min: 1200,
                    end_local_min: 479,
                    utc_offset_h: -6,
                },
                "{scenario}: {got:?}"
            );
        }
        let got = night(
            "custom",
            "scenario = \"custom\"\ncustom_pct = [2.5, 3.5, 4.5]\n\
             start_local = \"21:30\"\nend_local = \"06:00\"\nutc_offset_h = -7\n",
        )
        .unwrap();
        assert_eq!(
            got,
            NightMoisture {
                dead_pct: [2.5, 3.5, 4.5],
                start_local_min: 1290,
                end_local_min: 360,
                utc_offset_h: -7,
            },
            "custom: {got:?}"
        );
    }

    #[test]
    fn bad_night_moisture_tables_are_rejected() {
        for (name, table, needle) in [
            ("custom-missing", "scenario = \"custom\"\n", "required"),
            (
                "custom-given",
                "scenario = \"D2\"\ncustom_pct = [1, 2, 3]\n",
                "is given",
            ),
            (
                "custom-two",
                "scenario = \"custom\"\ncustom_pct = [1, 2]\n",
                "2 values",
            ),
            (
                "custom-zero",
                "scenario = \"custom\"\ncustom_pct = [0, 2, 3]\n",
                "holds 0",
            ),
            (
                "custom-high",
                "scenario = \"custom\"\ncustom_pct = [1, 2, 101]\n",
                "holds 101",
            ),
            (
                "hh24",
                "scenario = \"D1\"\nstart_local = \"24:00\"\n",
                "start_local",
            ),
            (
                "mm60",
                "scenario = \"D1\"\nend_local = \"07:60\"\n",
                "end_local",
            ),
            (
                "short",
                "scenario = \"D1\"\nstart_local = \"8:00\"\n",
                "start_local",
            ),
            (
                "equal",
                "scenario = \"D1\"\nstart_local = \"07:59\"\n",
                "equals",
            ),
            (
                "offset15",
                "scenario = \"D1\"\nutc_offset_h = 15\n",
                "utc_offset_h is 15",
            ),
            (
                "unknown",
                "scenario = \"D1\"\nstart = \"20:00\"\n",
                "unknown field `start`",
            ),
            ("scenario-d4", "scenario = \"D4\"\n", "D4"),
            ("no-scenario", "utc_offset_h = -6\n", "scenario"),
        ] {
            let err = match night(name, table) {
                Ok(got) => panic!("{name}: read {got:?}"),
                Err(err) => format!("{err:#}"),
            };
            assert!(err.contains(needle), "{name}: {err}");
        }
    }
}
