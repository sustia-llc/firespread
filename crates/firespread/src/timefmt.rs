//! Whole UTC hours: ISO 8601 labels to hours since 1970-01-01T00:00 and back.

use anyhow::{Context, Result, bail};

/// Days from 1970-01-01 to the proleptic Gregorian date `y-m-d`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date `(y, m, d)` of day `z` after 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn field(t: &str, range: std::ops::Range<usize>, name: &str) -> Result<i64> {
    t.get(range)
        .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
        .with_context(|| format!("time {t:?}: {name} is not a number"))?
        .parse()
        .with_context(|| format!("time {t:?}: {name}"))
}

/// Hours since 1970-01-01T00:00 UTC of `YYYY-MM-DDTHH:MM`, optionally
/// followed by `:SS` and `Z`; minutes and seconds must be 0.
pub fn parse_hour(t: &str) -> Result<i64> {
    let b = t.as_bytes();
    if b.len() < 16 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' {
        bail!("time {t:?} is not YYYY-MM-DDTHH:MM");
    }
    let rest = t
        .get(16..)
        .with_context(|| format!("time {t:?} is not ASCII"))?;
    let rest = rest.strip_suffix('Z').unwrap_or(rest);
    if !(rest.is_empty() || rest == ":00") {
        bail!("time {t:?} has a non-zero second or an unknown suffix");
    }
    let (y, m, d) = (
        field(t, 0..4, "year")?,
        field(t, 5..7, "month")?,
        field(t, 8..10, "day")?,
    );
    let (h, min) = (field(t, 11..13, "hour")?, field(t, 14..16, "minute")?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || min != 0 {
        bail!("time {t:?} is not a whole UTC hour");
    }
    let days = days_from_civil(y, m, d);
    if civil_from_days(days) != (y, m, d) {
        bail!("time {t:?} is not a calendar date");
    }
    Ok(days * 24 + h)
}

fn split(hours: i64) -> (i64, i64, i64, i64) {
    let (y, m, d) = civil_from_days(hours.div_euclid(24));
    (y, m, d, hours.rem_euclid(24))
}

/// `YYYY-MM-DDTHH:00:00Z` of `hours` after 1970-01-01T00:00 UTC.
pub fn format_hour_utc(hours: i64) -> String {
    let (y, m, d, h) = split(hours);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:00:00Z")
}

/// The FEDS file label `YYYYMMDDTHH00` of `hours` after 1970-01-01T00:00 UTC.
pub fn feds_label(hours: i64) -> String {
    let (y, m, d, h) = split(hours);
    format!("{y:04}{m:02}{d:02}T{h:02}00")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_steps_across_days_and_months() {
        let t0 = parse_hour("2022-04-22T12:00:00").unwrap();
        assert_eq!(t0, parse_hour("2022-04-22T12:00Z").unwrap());
        assert_eq!(format_hour_utc(t0), "2022-04-22T12:00:00Z");
        assert_eq!(feds_label(t0), "20220422T1200");
        assert_eq!(feds_label(t0 - 12), "20220422T0000");
        assert_eq!(format_hour_utc(t0 + 12), "2022-04-23T00:00:00Z");
        let may = parse_hour("2022-04-30T18:00:00").unwrap();
        assert_eq!(format_hour_utc(may + 12), "2022-05-01T06:00:00Z");
        assert_eq!(
            format_hour_utc(parse_hour("2024-03-01T00:00").unwrap() - 1),
            "2024-02-29T23:00:00Z"
        );
        // 2022-04-22 is 19104 days after 1970-01-01.
        assert_eq!(t0, 19_104 * 24 + 12);
    }

    #[test]
    fn rejects_malformed_times() {
        for t in [
            "2022-04-22",
            "2022-04-22T12:30:00",
            "2022-04-22T12:00:01",
            "2022-13-01T00:00:00",
            "2022-02-30T00:00:00",
            "2022-04-22T24:00:00",
            "2022-04-22 12:00:00",
        ] {
            assert!(parse_hour(t).is_err(), "{t:?} parsed");
        }
    }
}
