//! Serde readers for timestamp columns.
//!
//! A column defaulted to `CURRENT_TIMESTAMP` holds `YYYY-MM-DD HH:MM:SS`, and rusqlite
//! writes a bound `DateTime<Utc>` as `YYYY-MM-DD HH:MM:SS.fff+00:00`. chrono's serde
//! format only accepts `YYYY-MM-DDTHH:MM:SS`, so `from_rows` failed on real rows.

use chrono::{DateTime, NaiveDateTime};
use serde::{Deserialize, Deserializer, de::Error};

fn parse(s: &str) -> Option<NaiveDateTime> {
    ["%F %T%.f%:z", "%FT%T%.f%:z"]
        .iter()
        .find_map(|fmt| DateTime::parse_from_str(s, fmt).ok().map(|dt| dt.naive_utc()))
        .or_else(|| {
            ["%F %T%.f", "%FT%T%.f"]
                .iter()
                .find_map(|fmt| NaiveDateTime::parse_from_str(s, fmt).ok())
        })
}

pub(crate) fn naive<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveDateTime, D::Error> {
    let s = String::deserialize(deserializer)?;
    parse(&s).ok_or_else(|| D::Error::custom(format!("unrecognised timestamp: {s}")))
}

pub(crate) fn naive_opt<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<NaiveDateTime>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|s| parse(&s).ok_or_else(|| D::Error::custom(format!("unrecognised timestamp: {s}"))))
        .transpose()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn parses_every_form_sqlite_and_rusqlite_write() {
        let expected = NaiveDate::from_ymd_opt(2026, 11, 11)
            .unwrap()
            .and_hms_opt(8, 0, 1)
            .unwrap();
        for s in [
            "2026-11-11 08:00:01",
            "2026-11-11T08:00:01",
            "2026-11-11 08:00:01.000",
            "2026-11-11 08:00:01+00:00",
            "2026-11-11T08:00:01.000+00:00",
            "2026-11-11 10:00:01+02:00",
        ] {
            assert_eq!(parse(s), Some(expected), "{s}");
        }
        assert_eq!(parse("not a time"), None);
    }
}
