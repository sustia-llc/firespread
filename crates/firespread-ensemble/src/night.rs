//! Fixed dead fuel moisture for the night hours of a window.

use crate::{EnsembleError, Result};

/// Minutes per day.
pub const MINUTES_PER_DAY: u32 = 24 * 60;

/// A dead fuel moisture scenario of Scott and Burgan (2005) RMRS-GTR-153
/// Table 3 (p. 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightScenario {
    /// Dead 1-h 3 %, 10-h 4 %, 100-h 5 %.
    D1,
    /// Dead 1-h 6 %, 10-h 7 %, 100-h 8 %.
    D2,
    /// Dead 1-h 9 %, 10-h 10 %, 100-h 11 %.
    D3,
}

impl NightScenario {
    /// The scenario's dead 1-h, 10-h and 100-h moisture, percent, from
    /// Scott and Burgan (2005) RMRS-GTR-153 Table 3 (p. 8).
    #[must_use]
    pub fn dead_pct(self) -> [f64; 3] {
        match self {
            Self::D1 => [3.0, 4.0, 5.0],
            Self::D2 => [6.0, 7.0, 8.0],
            Self::D3 => [9.0, 10.0, 11.0],
        }
    }
}

/// Dead fuel moisture held fixed over the night hours of a window. An hour
/// whose UTC label `YYYY-MM-DDTHH:MM:SSZ` (the start of the hour) falls at
/// local time `t = UTC + utc_offset_h`, wrapped to `[00:00, 24:00)`, is a
/// night hour when `t >= start` or `t <= end` for `start > end`, and when
/// `start <= t <= end` otherwise. The default period 20:00 to 07:59 is the
/// night time of Rothermel (1983) INT-143 Table E (p. 19).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NightMoisture {
    /// Dead 1-h, 10-h and 100-h moisture of a night hour, percent, each
    /// finite and in (0, 100].
    pub dead_pct: [f64; 3],
    /// Start of the night period, minutes after local midnight, below 1440.
    pub start_local_min: u32,
    /// End of the night period, minutes after local midnight, below 1440 and
    /// not equal to `start_local_min`.
    pub end_local_min: u32,
    /// Hours added to UTC to give local time, in [-12, 14].
    pub utc_offset_h: i32,
}

impl NightMoisture {
    /// The moisture of `scenario` over 20:00 to 07:59 local time at UTC
    /// offset -6 h.
    #[must_use]
    pub fn new(scenario: NightScenario) -> Self {
        Self {
            dead_pct: scenario.dead_pct(),
            start_local_min: 20 * 60,
            end_local_min: 7 * 60 + 59,
            utc_offset_h: -6,
        }
    }

    /// The setting if every field is in its documented range.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidNightMoisture`] naming the first field out of
    /// range.
    pub fn validated(self) -> Result<Self> {
        let bad = |field, bound, value| {
            Err(EnsembleError::InvalidNightMoisture {
                field,
                bound,
                value,
            })
        };
        for (field, value) in ["dead 1-h", "dead 10-h", "dead 100-h"]
            .into_iter()
            .zip(self.dead_pct)
        {
            if !(value.is_finite() && value > 0.0 && value <= 100.0) {
                return bad(field, "a finite percent in (0, 100]", value);
            }
        }
        for (field, value) in [("start", self.start_local_min), ("end", self.end_local_min)] {
            if value >= MINUTES_PER_DAY {
                return bad(field, "minutes below 1440", f64::from(value));
            }
        }
        if self.start_local_min == self.end_local_min {
            return bad(
                "end",
                "a time other than the start",
                f64::from(self.end_local_min),
            );
        }
        if !(-12..=14).contains(&self.utc_offset_h) {
            return bad(
                "UTC offset",
                "hours in [-12, 14]",
                f64::from(self.utc_offset_h),
            );
        }
        Ok(self)
    }

    /// Whether the hour labelled `label` is a night hour.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidHourLabel`] when `label` is not
    /// `YYYY-MM-DDTHH:MM:SSZ`; `index` is the hour's index in the window.
    pub fn is_night(&self, index: usize, label: &str) -> Result<bool> {
        let utc = utc_minute_of_day(label).ok_or_else(|| EnsembleError::InvalidHourLabel {
            index,
            label: label.to_string(),
        })?;
        let local = (i64::from(utc) + i64::from(self.utc_offset_h) * 60)
            .rem_euclid(i64::from(MINUTES_PER_DAY));
        let (start, end) = (
            i64::from(self.start_local_min),
            i64::from(self.end_local_min),
        );
        Ok(if start > end {
            local >= start || local <= end
        } else {
            start <= local && local <= end
        })
    }
}

/// Minutes after UTC midnight of a label `YYYY-MM-DDTHH:MM:SSZ` with month
/// 01-12, day 01-31, hour 00-23, minute 00-59 and second 00-59.
fn utc_minute_of_day(label: &str) -> Option<u32> {
    let b = label.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let digits = &b[range];
        digits.iter().all(u8::is_ascii_digit).then(|| {
            digits
                .iter()
                .fold(0, |acc, &d| acc * 10 + u32::from(d - b'0'))
        })
    };
    number(0..4)?;
    let (month, day) = (number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    ((1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60)
        .then_some(hour * 60 + minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_parse_to_utc_minutes() {
        assert_eq!(utc_minute_of_day("2022-05-10T12:00:00Z"), Some(720));
        assert_eq!(utc_minute_of_day("2022-05-10T23:59:59Z"), Some(1439));
        for bad in [
            "h0",
            "hour 0",
            "2022-05-10T24:00:00Z",
            "2022-05-10T12:60:00Z",
            "2022-13-10T12:00:00Z",
            "2022-05-10 12:00:00Z",
            "2022-05-10T12:00:00",
            "2022-05-10T1a:00:00Z",
        ] {
            assert_eq!(utc_minute_of_day(bad), None, "{bad:?} parsed");
        }
    }
}
