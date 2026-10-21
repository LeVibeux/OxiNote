//! Moment.js-compatible date formatting, template expansion and the
//! [`Date`] / [`TemplateContext`] helpers.

use std::fmt;

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};

use crate::{DEFAULT_DATE_FORMAT, DEFAULT_TIME_FORMAT, Date, TemplateContext, TemplatesConfig};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

impl Date {
    /// A validated date, `None` if it does not exist (e.g. February 30).
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        NaiveDate::from_ymd_opt(year, month, day).map(Self::from)
    }

    /// Today in the local time zone.
    pub fn today() -> Self {
        Local::now().date_naive().into()
    }

    /// Convert to a [`chrono::NaiveDate`], `None` if the fields are invalid.
    pub fn to_naive(self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(self.year, self.month, self.day)
    }

    /// Parse `YYYY-MM-DD`.
    pub fn parse_iso(s: &str) -> Option<Self> {
        NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
            .ok()
            .map(Self::from)
    }

    /// The date `days` days later (negative = earlier). Invalid dates or
    /// overflow return `self` unchanged.
    pub fn add_days(self, days: i64) -> Self {
        self.to_naive()
            .and_then(|d| d.checked_add_signed(chrono::Duration::try_days(days)?))
            .map(Self::from)
            .unwrap_or(self)
    }

    /// Day of the week, 0 = Sunday ... 6 = Saturday (Moment's `d`).
    pub fn weekday(self) -> Option<u32> {
        self.to_naive().map(|d| d.weekday().num_days_from_sunday())
    }
}

impl From<NaiveDate> for Date {
    fn from(d: NaiveDate) -> Self {
        Self {
            year: d.year(),
            month: d.month(),
            day: d.day(),
        }
    }
}

impl fmt::Display for Date {
    /// `YYYY-MM-DD`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl TemplateContext {
    /// Context for a note titled `title` at the current local date and time.
    pub fn now(title: impl Into<String>) -> Self {
        Self::from_datetime(title, Local::now().naive_local())
    }

    /// Context for `title` at a given local date and time.
    pub fn from_datetime(title: impl Into<String>, dt: NaiveDateTime) -> Self {
        Self {
            title: title.into(),
            date: dt.date().into(),
            hour: dt.hour(),
            minute: dt.minute(),
            second: dt.second(),
        }
    }

    /// Context for `title` on `date` at midnight.
    pub fn on_date(title: impl Into<String>, date: Date) -> Self {
        Self {
            title: title.into(),
            date,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }

    /// The context's local date-time, `None` if the fields are invalid.
    pub fn naive_datetime(&self) -> Option<NaiveDateTime> {
        let time = NaiveTime::from_hms_opt(self.hour, self.minute, self.second)?;
        Some(self.date.to_naive()?.and_time(time))
    }
}

/// English ordinal: 1st, 2nd, 3rd, 4th, 11th, 12th, 13th, 21st, ...
fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// Moment tokens, longest first within each family.
const TOKENS: &[&str] = &[
    "YYYY", "YY", "Q", "MMMM", "MMM", "MM", "Mo", "M", "DDDD", "DDD", "DD", "Do", "D", "dddd",
    "ddd", "dd", "do", "d", "E", "e", "GGGG", "GG", "WW", "Wo", "W", "HH", "H", "hh", "h", "kk",
    "k", "mm", "m", "ss", "s", "A", "a", "X", "x",
];

fn render_token(token: &str, ctx: &TemplateContext) -> String {
    let d = ctx.date;
    let naive = d.to_naive();
    let h12 = match ctx.hour % 12 {
        0 => 12,
        h => h,
    };
    let month_name = MONTHS
        .get((d.month as usize).wrapping_sub(1))
        .copied()
        .unwrap_or("");
    let weekday = naive.map(|n| n.weekday().num_days_from_sunday() as usize);
    let weekday_name = weekday.and_then(|w| WEEKDAYS.get(w)).copied().unwrap_or("");
    let iso = naive.map(|n| n.iso_week());
    let opt = |v: Option<String>| v.unwrap_or_default();
    match token {
        "YYYY" => format!("{:04}", d.year),
        "YY" => format!("{:02}", d.year.rem_euclid(100)),
        "Q" => ((d.month.saturating_sub(1)) / 3 + 1).to_string(),
        "MMMM" => month_name.to_string(),
        "MMM" => month_name.chars().take(3).collect(),
        "MM" => format!("{:02}", d.month),
        "Mo" => ordinal(d.month),
        "M" => d.month.to_string(),
        "DDDD" => opt(naive.map(|n| format!("{:03}", n.ordinal()))),
        "DDD" => opt(naive.map(|n| n.ordinal().to_string())),
        "DD" => format!("{:02}", d.day),
        "Do" => ordinal(d.day),
        "D" => d.day.to_string(),
        "dddd" => weekday_name.to_string(),
        "ddd" => weekday_name.chars().take(3).collect(),
        "dd" => weekday_name.chars().take(2).collect(),
        "do" => opt(weekday.map(|w| ordinal(w as u32))),
        "d" | "e" => opt(weekday.map(|w| w.to_string())),
        "E" => opt(naive.map(|n| n.weekday().number_from_monday().to_string())),
        "GGGG" => opt(iso.map(|w| format!("{:04}", w.year()))),
        "GG" => opt(iso.map(|w| format!("{:02}", w.year().rem_euclid(100)))),
        "WW" => opt(iso.map(|w| format!("{:02}", w.week()))),
        "Wo" => opt(iso.map(|w| ordinal(w.week()))),
        "W" => opt(iso.map(|w| w.week().to_string())),
        "HH" => format!("{:02}", ctx.hour),
        "H" => ctx.hour.to_string(),
        "hh" => format!("{h12:02}"),
        "h" => h12.to_string(),
        "kk" => format!("{:02}", if ctx.hour == 0 { 24 } else { ctx.hour }),
        "k" => (if ctx.hour == 0 { 24 } else { ctx.hour }).to_string(),
        "mm" => format!("{:02}", ctx.minute),
        "m" => ctx.minute.to_string(),
        "ss" => format!("{:02}", ctx.second),
        "s" => ctx.second.to_string(),
        "A" => (if ctx.hour < 12 { "AM" } else { "PM" }).to_string(),
        "a" => (if ctx.hour < 12 { "am" } else { "pm" }).to_string(),
        "X" | "x" => {
            let ts = ctx
                .naive_datetime()
                .and_then(|dt| Local.from_local_datetime(&dt).earliest());
            opt(ts.map(|t| {
                if token == "X" {
                    t.timestamp().to_string()
                } else {
                    t.timestamp_millis().to_string()
                }
            }))
        }
        other => other.to_string(),
    }
}

/// Format a date/time with a Moment.js format string (YYYY, YY, MM, M, DD, D,
/// ddd, dddd, MMM, MMMM, HH, H, mm, ss, A, `[literal]`).
///
/// Also supported: `Q`, `Mo`, `Do`, `DDD`/`DDDD` (day of year), `d`, `dd`,
/// `do`, `e`, `E`, ISO weeks `W`/`WW`/`Wo` and week-years `GG`/`GGGG`, `h`/`hh`,
/// `k`/`kk`, `m`, `s`, `a`, `X` (unix seconds, local time zone), `x` (ms).
/// Names are English. Any other character is copied verbatim; an unclosed
/// `[` is literal.
pub fn format_moment(format: &str, ctx: &TemplateContext) -> String {
    let mut out = String::with_capacity(format.len() + 8);
    let mut rest = format;
    'outer: while let Some(c) = rest.chars().next() {
        if c == '['
            && let Some(end) = rest[1..].find(']')
        {
            out.push_str(&rest[1..1 + end]);
            rest = &rest[end + 2..];
            continue;
        }
        for token in TOKENS {
            if rest.starts_with(token) {
                out.push_str(&render_token(token, ctx));
                rest = &rest[token.len()..];
                continue 'outer;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Expand `{{title}}`, `{{date}}`, `{{time}}`, `{{date:FORMAT}}`, `{{time:FORMAT}}`
/// (Moment.js formats) in a template.
///
/// `{{date}}` / `{{time}}` use `config.date_format` / `config.time_format`
/// (defaults `YYYY-MM-DD` / `HH:mm`). Names are case-insensitive, whitespace
/// inside the braces is ignored, unknown placeholders are left untouched.
pub fn expand_template(template: &str, ctx: &TemplateContext, config: &TemplatesConfig) -> String {
    let date_fmt = config.date_format.as_deref().unwrap_or(DEFAULT_DATE_FORMAT);
    let time_fmt = config.time_format.as_deref().unwrap_or(DEFAULT_TIME_FORMAT);
    expand_with(template, |inner| {
        let (name, fmt) = match inner.split_once(':') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner, None),
        };
        match (name.to_ascii_lowercase().as_str(), fmt) {
            ("title", None) => Some(ctx.title.clone()),
            ("date", None) => Some(format_moment(date_fmt, ctx)),
            ("time", None) => Some(format_moment(time_fmt, ctx)),
            ("date" | "time", Some(f)) => Some(format_moment(f, ctx)),
            _ => None,
        }
    })
}

/// Replace every `{{ inner }}` for which `f(trimmed inner)` returns `Some`.
pub(crate) fn expand_with(template: &str, mut f: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        // A nested `{{` means this opening pair is not a placeholder.
        if inner.contains("{{") {
            out.push_str("{{");
            rest = after;
            continue;
        }
        match f(inner.trim()) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> TemplateContext {
        TemplateContext {
            title: "Note".into(),
            date: Date {
                year: 2026,
                month: 10,
                day: 20,
            },
            hour: 14,
            minute: 5,
            second: 9,
        }
    }

    #[test]
    fn basic_tokens() {
        let c = ctx();
        assert_eq!(format_moment("YYYY-MM-DD", &c), "2026-10-20");
        assert_eq!(format_moment("YY M D", &c), "26 10 20");
        assert_eq!(format_moment("HH:mm:ss", &c), "14:05:09");
        assert_eq!(format_moment("H:m:s", &c), "14:5:9");
        assert_eq!(format_moment("h:mm A / hh a", &c), "2:05 PM / 02 pm");
        assert_eq!(
            format_moment("dddd D MMMM YYYY", &c),
            "Tuesday 20 October 2026"
        );
        assert_eq!(format_moment("ddd, MMM Do", &c), "Tue, Oct 20th");
        assert_eq!(format_moment("d E dd", &c), "2 2 Tu");
        assert_eq!(format_moment("Q DDD DDDD", &c), "4 293 293");
    }

    #[test]
    fn literals_and_unknown() {
        let c = ctx();
        assert_eq!(format_moment("[Week] W, [YYYY]", &c), "Week 43, YYYY");
        assert_eq!(format_moment("YYYY-MM-DDTHH", &c), "2026-10-20T14");
        assert_eq!(format_moment("[unclo", &c), "[unclo");
        assert_eq!(format_moment("été", &c), "été");
        assert_eq!(
            format_moment("YYYY/MM/YYYY-MM-DD", &c),
            "2026/10/2026-10-20"
        );
    }

    #[test]
    fn ordinals() {
        let v: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31, 101, 111]
            .iter()
            .map(|n| ordinal(*n))
            .collect();
        assert_eq!(
            v,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "31st",
                "101st", "111th"
            ]
        );
    }

    #[test]
    fn iso_weeks() {
        let mut c = ctx();
        c.date = Date::new(2021, 1, 3).unwrap(); // Sunday of ISO week 53 of 2020
        assert_eq!(format_moment("GGGG-[W]WW", &c), "2020-W53");
        c.date = Date::new(2026, 1, 5).unwrap();
        assert_eq!(format_moment("W WW Wo", &c), "2 02 2nd");
    }

    #[test]
    fn midnight_and_noon() {
        let mut c = ctx();
        c.hour = 0;
        assert_eq!(format_moment("h A k", &c), "12 AM 24");
        c.hour = 12;
        assert_eq!(format_moment("h A", &c), "12 PM");
    }

    #[test]
    fn unix_timestamp_roundtrip() {
        let c = ctx();
        let x: i64 = format_moment("X", &c).parse().unwrap();
        let back = Local.timestamp_opt(x, 0).unwrap().naive_local();
        assert_eq!(back, c.naive_datetime().unwrap());
        let ms: i64 = format_moment("x", &c).parse().unwrap();
        assert_eq!(ms, x * 1000);
    }

    #[test]
    fn invalid_date_does_not_panic() {
        let mut c = ctx();
        c.date = Date {
            year: 2026,
            month: 13,
            day: 40,
        };
        let _ = format_moment("YYYY MMMM dddd W X DDD", &c);
    }

    #[test]
    fn expand() {
        let c = ctx();
        let cfg = TemplatesConfig::default();
        assert_eq!(
            expand_template("# {{title}}\n{{date}} {{time}}", &c, &cfg),
            "# Note\n2026-10-20 14:05"
        );
        assert_eq!(
            expand_template("{{ date:dddd }} {{TIME: HH[h]mm }} {{Title}}", &c, &cfg),
            "Tuesday 14h05 Note"
        );
        assert_eq!(
            expand_template("{{unknown}} {{title:x}} {{ {{title}} }} {{open", &c, &cfg),
            "{{unknown}} {{title:x}} {{ Note }} {{open"
        );
        let cfg = TemplatesConfig {
            folder: None,
            date_format: Some("DD/MM/YYYY".into()),
            time_format: Some("h:mm a".into()),
        };
        assert_eq!(
            expand_template("{{date}} {{time}}", &c, &cfg),
            "20/10/2026 2:05 pm"
        );
    }

    #[test]
    fn date_helpers() {
        let d = Date::new(2026, 2, 28).unwrap();
        assert_eq!(d.add_days(1), Date::new(2026, 3, 1).unwrap());
        assert_eq!(d.add_days(-59).to_string(), "2025-12-31");
        assert_eq!(Date::parse_iso("2026-10-20"), Date::new(2026, 10, 20));
        assert!(Date::new(2026, 2, 30).is_none());
        assert_eq!(Date::new(2026, 10, 20).unwrap().weekday(), Some(2));
        let today = Date::today();
        assert!(today.to_naive().is_some());
        let ctx = TemplateContext::now("x");
        assert_eq!(ctx.title, "x");
        assert!(ctx.naive_datetime().is_some());
    }
}
