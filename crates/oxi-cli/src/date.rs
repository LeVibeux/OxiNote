//! Minimal calendar helpers (no date crate needed).

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use oxi_vault::Date;

/// Parse `YYYY-MM-DD`.
pub fn parse_date(s: &str) -> Result<Date> {
    let parts: Vec<&str> = s.trim().split('-').collect();
    let invalid = || anyhow::anyhow!("invalid date {s:?}, expected YYYY-MM-DD");
    let [y, m, d] = parts.as_slice() else {
        return Err(invalid());
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return Err(invalid());
    }
    let year: i32 = y.parse().map_err(|_| invalid())?;
    let month: u32 = m.parse().map_err(|_| invalid())?;
    let day: u32 = d.parse().map_err(|_| invalid())?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        bail!("invalid date {s:?}: no such day");
    }
    Ok(Date { year, month, day })
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Today's date in UTC.
pub fn today_utc() -> Date {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil_from_days((secs / 86_400) as i64)
}

/// Convert days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
pub fn civil_from_days(days: i64) -> Date {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    Date {
        year: y as i32,
        month: m as u32,
        day: d as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        let d = parse_date("2026-10-20").unwrap();
        assert_eq!((d.year, d.month, d.day), (2026, 10, 20));
        assert!(parse_date("2024-02-29").is_ok());
        assert!(parse_date("2023-02-29").is_err());
        assert!(parse_date("2026-13-01").is_err());
        assert!(parse_date("20-10-2026").is_err());
        assert!(parse_date("yesterday").is_err());
    }

    #[test]
    fn civil() {
        let d = civil_from_days(0);
        assert_eq!((d.year, d.month, d.day), (1970, 1, 1));
        let d = civil_from_days(20_746);
        assert_eq!((d.year, d.month, d.day), (2026, 10, 20));
        let d = civil_from_days(11_016);
        assert_eq!((d.year, d.month, d.day), (2000, 2, 29));
    }
}
