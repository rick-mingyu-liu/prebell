//! Eastern Time clock and NYSE trading calendar for the premarket brief window.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::{America::New_York, Tz};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayKind {
    Closed,
    Full,
    EarlyClose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotDecision {
    NotToday,
    TooEarly,
    InWindow { trading_date: NaiveDate },
    PastDeadline { trading_date: NaiveDate },
}

/// NYSE holiday table (closed days, then early-close days), as (year, month, day).
///
/// Limitation: only 2026 and 2027 are covered. Dates outside that range are
/// treated as `Full` on weekdays; a later plan extends the table.
const CLOSED: &[(i32, u32, u32)] = &[
    (2026, 1, 1), (2026, 1, 19), (2026, 2, 16), (2026, 4, 3), (2026, 5, 25),
    (2026, 6, 19), (2026, 7, 3), (2026, 9, 7), (2026, 11, 26), (2026, 12, 25),
    (2027, 1, 1), (2027, 1, 18), (2027, 2, 15), (2027, 3, 26), (2027, 5, 31),
    (2027, 6, 18), (2027, 7, 5), (2027, 9, 6), (2027, 11, 25), (2027, 12, 24),
];
const EARLY_CLOSE: &[(i32, u32, u32)] = &[(2026, 11, 27), (2026, 12, 24), (2027, 11, 26)];

/// Converts a UTC instant to America/New_York.
pub fn et(now_utc: DateTime<Utc>) -> DateTime<Tz> {
    now_utc.with_timezone(&New_York)
}

/// Classifies a calendar date on the NYSE calendar. Weekends are always `Closed`.
pub fn trading_day(date: NaiveDate) -> DayKind {
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return DayKind::Closed;
    }
    let key = (date.year(), date.month(), date.day());
    if CLOSED.contains(&key) {
        DayKind::Closed
    } else if EARLY_CLOSE.contains(&key) {
        DayKind::EarlyClose
    } else {
        DayKind::Full
    }
}

/// Decides whether `now_utc` is inside the 08:30:00 <= ET < 09:25:00 premarket window
/// on a trading day.
pub fn premarket_decision(now_utc: DateTime<Utc>) -> SlotDecision {
    let local = et(now_utc);
    let trading_date = local.date_naive();
    if trading_day(trading_date) == DayKind::Closed {
        return SlotDecision::NotToday;
    }
    let start = NaiveTime::from_hms_opt(8, 30, 0).unwrap();
    let end = NaiveTime::from_hms_opt(9, 25, 0).unwrap();
    let t = local.time();
    if t < start {
        SlotDecision::TooEarly
    } else if t < end {
        SlotDecision::InWindow { trading_date }
    } else {
        SlotDecision::PastDeadline { trading_date }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn dst_day_only_one_cron_counts() {
        assert!(matches!(premarket_decision(utc("2026-11-02T13:30:00Z")), SlotDecision::InWindow { .. }));
        assert_eq!(premarket_decision(utc("2026-11-02T12:30:00Z")), SlotDecision::TooEarly);
        assert!(matches!(premarket_decision(utc("2026-10-30T12:30:00Z")), SlotDecision::InWindow { .. }));
    }

    #[test]
    fn holidays() {
        assert_eq!(trading_day(d("2026-11-26")), DayKind::Closed);
        assert_eq!(trading_day(d("2026-11-27")), DayKind::EarlyClose);
        assert_eq!(trading_day(d("2026-12-25")), DayKind::Closed);
        assert_eq!(trading_day(d("2026-10-31")), DayKind::Closed);
        assert_eq!(premarket_decision(utc("2026-11-26T13:30:00Z")), SlotDecision::NotToday);
    }

    #[test]
    fn holidays_2027() {
        assert_eq!(trading_day(d("2027-03-26")), DayKind::Closed);
        assert_eq!(trading_day(d("2027-11-26")), DayKind::EarlyClose);
        assert_eq!(trading_day(d("2027-06-14")), DayKind::Full);
    }

    #[test]
    fn deadline() {
        assert!(matches!(premarket_decision(utc("2026-10-30T13:24:59Z")), SlotDecision::InWindow { .. }));
        assert!(matches!(premarket_decision(utc("2026-10-30T13:25:00Z")), SlotDecision::PastDeadline { .. }));
    }

    #[test]
    fn early_close_day_is_in_window() {
        assert_eq!(
            premarket_decision(utc("2026-11-27T13:30:00Z")),
            SlotDecision::InWindow { trading_date: d("2026-11-27") }
        );
    }
}
