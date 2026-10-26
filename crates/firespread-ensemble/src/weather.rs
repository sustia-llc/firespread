//! Hourly weather of one forecast window on the fuel grid.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use firespread_grid::{GridHeader, read_geotiff};
use rayon::prelude::*;
use serde::Deserialize;

use crate::moisture::DeadFuelMoisture;
use crate::waf::u20ft_from_u10;
use crate::{EnsembleError, Result};

/// One hour of raw weather, one value per cell, row-major with the north row
/// first.
#[derive(Debug, Clone, Copy)]
pub struct RawHour<'a> {
    /// 10 m wind component along +x (increasing column), m/s.
    pub u10_m_s: &'a [f32],
    /// 10 m wind component along +y (toward row 0), m/s.
    pub v10_m_s: &'a [f32],
    /// 2 m temperature, K.
    pub t2m_k: &'a [f32],
    /// 2 m relative humidity, percent.
    pub rh2m_pct: &'a [f32],
    /// Total cloud cover, percent.
    pub tcc_pct: &'a [f32],
}

/// The per-cell fields of one hour that every member shares.
#[derive(Debug, Clone, PartialEq)]
pub struct HourFields {
    /// 20 ft wind speed `hypot(u10, v10) / 1.15`, m/s.
    pub u20ft_m_s: Vec<f32>,
    /// Azimuth the wind blows toward, `atan2(u10, v10)`, rad clockwise from
    /// grid north (row 0).
    pub wind_toward_azimuth_rad: Vec<f32>,
    /// EMCPRM of [`DeadFuelMoisture::emcprm_pct`], percent.
    pub emcprm_pct: Vec<f32>,
}

fn check_len(layer: &'static str, expected: usize, got: usize) -> Result<()> {
    if expected == got {
        Ok(())
    } else {
        Err(EnsembleError::LayerLength {
            layer,
            expected,
            got,
        })
    }
}

impl HourFields {
    /// The fields of `raw` under `moisture`, for `n` cells.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidSkyEdges`] when the sky edges of `moisture`
    /// are not valid, [`EnsembleError::LayerLength`] when a layer does not
    /// hold `n` values, [`EnsembleError::InvalidCell`] when a wind component
    /// is not finite, and the errors of [`DeadFuelMoisture::emcprm_pct`].
    pub fn from_raw(raw: &RawHour<'_>, moisture: &DeadFuelMoisture, n: usize) -> Result<Self> {
        moisture.sky_edges.validated()?;
        check_len("u10", n, raw.u10_m_s.len())?;
        check_len("v10", n, raw.v10_m_s.len())?;
        check_len("t2m", n, raw.t2m_k.len())?;
        check_len("rh2m", n, raw.rh2m_pct.len())?;
        check_len("tcc", n, raw.tcc_pct.len())?;
        let mut u20 = Vec::with_capacity(n);
        let mut az = Vec::with_capacity(n);
        let mut emc = Vec::with_capacity(n);
        for index in 0..n {
            let (u, v) = (f64::from(raw.u10_m_s[index]), f64::from(raw.v10_m_s[index]));
            for (layer, value) in [("u10", u), ("v10", v)] {
                if !value.is_finite() {
                    return Err(EnsembleError::InvalidCell {
                        layer,
                        bound: "a finite value",
                        index,
                        value,
                    });
                }
            }
            u20.push(u20ft_from_u10(u.hypot(v)) as f32);
            az.push(u.atan2(v) as f32);
            emc.push(moisture.emcprm_pct(
                f64::from(raw.t2m_k[index]),
                f64::from(raw.rh2m_pct[index]),
                f64::from(raw.tcc_pct[index]),
            )? as f32);
        }
        Ok(Self {
            u20ft_m_s: u20,
            wind_toward_azimuth_rad: az,
            emcprm_pct: emc,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_fields_hand_values() {
        // (u10, v10) = (3.45, 4.6): |U10| = 5.75, U20ft = 5.75 / 1.15 = 5,
        // toward atan2(3.45, 4.6) = atan(0.75) = 0.643501 rad. 300 K, 20 %,
        // clear: EMCPRM 3.07189628 (see the moisture module's hand values).
        let one = |v: f32| [v];
        let h = HourFields::from_raw(
            &RawHour {
                u10_m_s: &one(3.45),
                v10_m_s: &one(4.6),
                t2m_k: &one(300.0),
                rh2m_pct: &one(20.0),
                tcc_pct: &one(0.0),
            },
            &DeadFuelMoisture::default(),
            1,
        )
        .unwrap();
        assert!((h.u20ft_m_s[0] - 5.0).abs() < 1e-5, "{}", h.u20ft_m_s[0]);
        assert!(
            (h.wind_toward_azimuth_rad[0] - 0.643_501).abs() < 1e-5,
            "{}",
            h.wind_toward_azimuth_rad[0]
        );
        assert!(
            (h.emcprm_pct[0] - 3.071_896).abs() < 1e-5,
            "{}",
            h.emcprm_pct[0]
        );
    }
}

/// The weather of one window: per-hour fields in hour order, gridMET 100-h
/// and 1000-h dead fuel moisture, and the hours' UTC labels. Hour `k`
/// applies over `[k, k + 1)` hours from the window start.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowWeather {
    pub hours: Vec<HourFields>,
    /// UTC label of each hour, as the index gives it.
    pub hours_utc: Vec<String>,
    /// gridMET 100-h dead fuel moisture, percent.
    pub fm100_pct: Vec<f32>,
    /// gridMET 1000-h dead fuel moisture, percent. Read and carried; the
    /// surface spread model has no 1000-h class and does not use it.
    pub fm1000_pct: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct Index {
    units: BTreeMap<String, String>,
    windows: Vec<IndexWindow>,
}

#[derive(Debug, Deserialize)]
struct IndexWindow {
    name: String,
    gridmet: BTreeMap<String, IndexFile>,
    hours: Vec<IndexHour>,
}

#[derive(Debug, Deserialize)]
struct IndexHour {
    hour_utc: String,
    files: BTreeMap<String, IndexFile>,
}

#[derive(Debug, Deserialize)]
struct IndexFile {
    path: String,
}

/// Units the loader reads each index layer in.
const UNITS: [(&str, &str); 7] = [
    ("u10", "m s-1"),
    ("v10", "m s-1"),
    ("t2m", "K"),
    ("rh2m", "%"),
    ("tcc", "%"),
    ("fm100", "%"),
    ("fm1000", "%"),
];

fn read_layer(path: &Path, header: &GridHeader) -> Result<Vec<f32>> {
    let grid = read_geotiff(path)?;
    let same = GridHeader {
        nodata: 0.0,
        ..grid.header
    }
    .same_grid(&GridHeader {
        nodata: 0.0,
        ..*header
    });
    if !same {
        return Err(EnsembleError::WeatherGrid {
            path: path.to_path_buf(),
        });
    }
    let nodata = grid.header.nodata;
    if let Some(index) = grid
        .data
        .iter()
        .position(|v| !v.is_finite() || v.to_bits() == nodata.to_bits())
    {
        return Err(EnsembleError::InvalidCell {
            layer: "weather",
            bound: "a finite value other than nodata",
            index,
            value: f64::from(grid.data[index]),
        });
    }
    Ok(grid.data)
}

impl WindowWeather {
    /// Loads the first `hours` hours of window `window` from the weather
    /// index `<weather_dir>/index.json`, with each listed path relative to
    /// `weather_dir`. Each raster must lie on the grid of `header` (nodata
    /// aside) and hold no nodata or non-finite value.
    ///
    /// # Errors
    /// [`EnsembleError::WeatherIndex`] when the index cannot be read or
    /// parsed, [`EnsembleError::WeatherUnit`] when the index names another
    /// unit for a layer, [`EnsembleError::MissingWindow`],
    /// [`EnsembleError::MissingWeatherFile`],
    /// [`EnsembleError::TooFewHours`] when the window lists fewer than
    /// `hours` hours, [`EnsembleError::WeatherGrid`], the raster read errors,
    /// [`EnsembleError::InvalidCell`], and the errors of
    /// [`HourFields::from_raw`].
    pub fn load(
        weather_dir: &Path,
        window: &str,
        hours: usize,
        header: &GridHeader,
        moisture: &DeadFuelMoisture,
    ) -> Result<Self> {
        let index_path = weather_dir.join("index.json");
        let index_err = |reason: String| EnsembleError::WeatherIndex {
            path: index_path.clone(),
            reason,
        };
        let text = std::fs::read_to_string(&index_path).map_err(|e| index_err(e.to_string()))?;
        let index: Index = serde_json::from_str(&text).map_err(|e| index_err(e.to_string()))?;
        for (field, expected) in UNITS {
            let found = index.units.get(field);
            if found.map(String::as_str) != Some(expected) {
                return Err(EnsembleError::WeatherUnit {
                    field,
                    found: found.cloned(),
                    expected,
                });
            }
        }
        let w = index
            .windows
            .iter()
            .find(|w| w.name == window)
            .ok_or_else(|| EnsembleError::MissingWindow {
                window: window.to_string(),
            })?;
        if w.hours.len() < hours {
            return Err(EnsembleError::TooFewHours {
                available: w.hours.len(),
                horizon_h: u32::try_from(hours).unwrap_or(u32::MAX),
            });
        }
        let path_of = |files: &BTreeMap<String, IndexFile>,
                       layer: &'static str,
                       at: &str|
         -> Result<PathBuf> {
            files
                .get(layer)
                .map(|f| weather_dir.join(&f.path))
                .ok_or_else(|| EnsembleError::MissingWeatherFile {
                    window: window.to_string(),
                    layer,
                    at: at.to_string(),
                })
        };
        let n = header.cell_count().ok_or(EnsembleError::LayerLength {
            layer: "weather",
            expected: usize::MAX,
            got: 0,
        })?;
        let fm100_pct = read_layer(&path_of(&w.gridmet, "fm100", "gridmet")?, header)?;
        let fm1000_pct = read_layer(&path_of(&w.gridmet, "fm1000", "gridmet")?, header)?;
        let fields = w.hours[..hours]
            .par_iter()
            .map(|h| -> Result<HourFields> {
                let at = h.hour_utc.as_str();
                let read = |layer| read_layer(&path_of(&h.files, layer, at)?, header);
                let (u10, v10, t2m, rh2m, tcc) = (
                    read("u10")?,
                    read("v10")?,
                    read("t2m")?,
                    read("rh2m")?,
                    read("tcc")?,
                );
                HourFields::from_raw(
                    &RawHour {
                        u10_m_s: &u10,
                        v10_m_s: &v10,
                        t2m_k: &t2m,
                        rh2m_pct: &rh2m,
                        tcc_pct: &tcc,
                    },
                    moisture,
                    n,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            hours: fields,
            hours_utc: w.hours[..hours]
                .iter()
                .map(|h| h.hour_utc.clone())
                .collect(),
            fm100_pct,
            fm1000_pct,
        })
    }
}
