//! The Indian financial year: 1 April to 31 March, by the date in India (UTC+5:30). An invoice
//! issued at 11 p.m. UTC on 31 March is already 1 April in India, so it belongs to the new year.

use chrono::{DateTime, Datelike, Duration, Utc};

const IST_OFFSET_MINUTES: i64 = 5 * 60 + 30;

/// `2026-27` for any moment from 1 April 2026 to 31 March 2027, Indian time.
pub fn financial_year(at: DateTime<Utc>) -> String {
    let india = at + Duration::minutes(IST_OFFSET_MINUTES);
    let start = if india.month() >= 4 {
        india.year()
    } else {
        india.year() - 1
    };
    format!("{start}-{:02}", (start + 1).rem_euclid(100))
}

/// `ZNO/2026-27/000042`.
pub fn invoice_number(prefix: &str, fy: &str, sequence: i32) -> String {
    format!("{prefix}/{fy}/{sequence:06}")
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn the_year_runs_april_to_march() {
        assert_eq!(financial_year(utc(2026, 4, 1, 12, 0)), "2026-27");
        assert_eq!(financial_year(utc(2026, 11, 10, 12, 0)), "2026-27");
        assert_eq!(financial_year(utc(2027, 1, 15, 12, 0)), "2026-27");
        assert_eq!(financial_year(utc(2027, 3, 31, 12, 0)), "2026-27");
        assert_eq!(financial_year(utc(2027, 4, 2, 12, 0)), "2027-28");
        assert_eq!(financial_year(utc(2026, 3, 30, 12, 0)), "2025-26");
    }

    #[test]
    fn the_boundary_is_midnight_in_india_not_in_utc() {
        // 18:29 UTC on 31 March is 23:59 in India: still the old year.
        assert_eq!(financial_year(utc(2027, 3, 31, 18, 29)), "2026-27");
        // 18:30 UTC is 00:00 on 1 April in India: the new year, while UTC still says 31 March.
        assert_eq!(financial_year(utc(2027, 3, 31, 18, 30)), "2027-28");
        assert_eq!(financial_year(utc(2027, 3, 31, 23, 59)), "2027-28");
        // The same at the turn of the calendar year changes nothing.
        assert_eq!(financial_year(utc(2026, 12, 31, 18, 30)), "2026-27");
    }

    #[test]
    fn the_century_turns_cleanly() {
        assert_eq!(financial_year(utc(2099, 6, 1, 0, 0)), "2099-00");
        assert_eq!(financial_year(utc(2100, 1, 1, 0, 0)), "2099-00");
    }

    #[test]
    fn numbers_are_padded_and_ordered() {
        assert_eq!(invoice_number("ZNO", "2026-27", 1), "ZNO/2026-27/000001");
        assert_eq!(invoice_number("ZNO", "2026-27", 42), "ZNO/2026-27/000042");
        assert_eq!(
            invoice_number("ZL", "2027-28", 1_234_567),
            "ZL/2027-28/1234567"
        );
    }
}
