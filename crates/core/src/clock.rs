//! Eastern Time clock, NYSE trading calendar, and each content type's
//! publishing window.

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
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
    (2026, 1, 1),
    (2026, 1, 19),
    (2026, 2, 16),
    (2026, 4, 3),
    (2026, 5, 25),
    (2026, 6, 19),
    (2026, 7, 3),
    (2026, 9, 7),
    (2026, 11, 26),
    (2026, 12, 25),
    (2027, 1, 1),
    (2027, 1, 18),
    (2027, 2, 15),
    (2027, 3, 26),
    (2027, 5, 31),
    (2027, 6, 18),
    (2027, 7, 5),
    (2027, 9, 6),
    (2027, 11, 25),
    (2027, 12, 24),
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

/// The close (16:00 ET, or 13:00 ET on an early-close day) of the last
/// trading day before `now_utc`'s ET date.
pub fn previous_close(now_utc: DateTime<Utc>) -> DateTime<Utc> {
    let mut day = et(now_utc).date_naive();
    loop {
        day = day.pred_opt().expect("date in range");
        let close = match trading_day(day) {
            DayKind::Closed => continue,
            DayKind::EarlyClose => hm(13, 0),
            DayKind::Full => hm(16, 0),
        };
        return New_York
            .from_local_datetime(&day.and_time(close))
            .earliest()
            .expect("the close is never in a DST gap")
            .with_timezone(&Utc);
    }
}

/// A publishing window in ET: `start <= t < end` is in the window, and `end`
/// is the deadline after which the day is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).expect("valid built-in time")
}

/// Premarket brief: 08:30-09:25 ET on every trading day.
pub fn premarket_window(_day: DayKind) -> Window {
    Window {
        start: hm(8, 30),
        end: hm(9, 25),
    }
}

/// After-close recap: 45 minutes after the close (16:45 ET, or 13:45 ET on
/// a 1 PM early-close day), late up to 19:00 ET either way.
pub fn recap_window(day: DayKind) -> Window {
    let start = match day {
        DayKind::EarlyClose => hm(13, 45),
        DayKind::Full | DayKind::Closed => hm(16, 45),
    };
    Window {
        start,
        end: hm(19, 0),
    }
}

/// Places `now_utc` against the ET trading day's window.
pub fn decide(now_utc: DateTime<Utc>, window: impl Fn(DayKind) -> Window) -> SlotDecision {
    let local = et(now_utc);
    let trading_date = local.date_naive();
    let day = trading_day(trading_date);
    if day == DayKind::Closed {
        return SlotDecision::NotToday;
    }
    let w = window(day);
    let t = local.time();
    if t < w.start {
        SlotDecision::TooEarly
    } else if t < w.end {
        SlotDecision::InWindow { trading_date }
    } else {
        SlotDecision::PastDeadline { trading_date }
    }
}

/// Decides whether `now_utc` is inside the 08:30:00 <= ET < 09:25:00 premarket window
/// on a trading day.
pub fn premarket_decision(now_utc: DateTime<Utc>) -> SlotDecision {
    decide(now_utc, premarket_window)
}

/// Decides whether `now_utc` is inside the recap window on a trading day.
pub fn recap_decision(now_utc: DateTime<Utc>) -> SlotDecision {
    decide(now_utc, recap_window)
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
    fn previous_close_skips_weekends_and_holidays() {
        for (now, close) in [
            // Thursday premarket: Wednesday 16:00 EDT.
            ("2026-10-29T13:20:00Z", "2026-10-28T20:00:00Z"),
            // Monday premarket and recap (EST): Friday 16:00 EDT.
            ("2026-11-02T14:20:00Z", "2026-10-30T20:00:00Z"),
            ("2026-11-02T21:45:00Z", "2026-10-30T20:00:00Z"),
            // Monday after Good Friday: Thursday.
            ("2026-04-06T13:20:00Z", "2026-04-02T20:00:00Z"),
            // Day after Thanksgiving: Wednesday 16:00 EST.
            ("2026-11-27T14:20:00Z", "2026-11-25T21:00:00Z"),
            // Monday after the early close: Friday 13:00 EST.
            ("2026-11-30T14:20:00Z", "2026-11-27T18:00:00Z"),
        ] {
            assert_eq!(previous_close(utc(now)), utc(close), "{now}");
        }
    }

    #[test]
    fn dst_day_only_one_cron_counts() {
        assert!(matches!(
            premarket_decision(utc("2026-11-02T13:30:00Z")),
            SlotDecision::InWindow { .. }
        ));
        assert_eq!(
            premarket_decision(utc("2026-11-02T12:30:00Z")),
            SlotDecision::TooEarly
        );
        assert!(matches!(
            premarket_decision(utc("2026-10-30T12:30:00Z")),
            SlotDecision::InWindow { .. }
        ));
    }

    #[test]
    fn holidays() {
        assert_eq!(trading_day(d("2026-11-26")), DayKind::Closed);
        assert_eq!(trading_day(d("2026-11-27")), DayKind::EarlyClose);
        assert_eq!(trading_day(d("2026-12-25")), DayKind::Closed);
        assert_eq!(trading_day(d("2026-10-31")), DayKind::Closed);
        assert_eq!(
            premarket_decision(utc("2026-11-26T13:30:00Z")),
            SlotDecision::NotToday
        );
    }

    #[test]
    fn holidays_2027() {
        assert_eq!(trading_day(d("2027-03-26")), DayKind::Closed);
        assert_eq!(trading_day(d("2027-11-26")), DayKind::EarlyClose);
        assert_eq!(trading_day(d("2027-06-14")), DayKind::Full);
    }

    #[test]
    fn deadline() {
        assert!(matches!(
            premarket_decision(utc("2026-10-30T13:24:59Z")),
            SlotDecision::InWindow { .. }
        ));
        assert!(matches!(
            premarket_decision(utc("2026-10-30T13:25:00Z")),
            SlotDecision::PastDeadline { .. }
        ));
    }

    #[test]
    fn recap_window_edt_and_est() {
        // Thu 2026-10-29 EDT: 20:45Z = 16:45 ET.
        assert_eq!(
            recap_decision(utc("2026-10-29T20:44:59Z")),
            SlotDecision::TooEarly
        );
        assert_eq!(
            recap_decision(utc("2026-10-29T20:45:00Z")),
            SlotDecision::InWindow {
                trading_date: d("2026-10-29")
            }
        );
        // Mon 2026-11-02 EST: 20:45Z = 15:45 ET, 21:45Z = 16:45 ET.
        assert_eq!(
            recap_decision(utc("2026-11-02T20:45:00Z")),
            SlotDecision::TooEarly
        );
        assert!(matches!(
            recap_decision(utc("2026-11-02T21:45:00Z")),
            SlotDecision::InWindow { .. }
        ));
    }

    #[test]
    fn recap_deadline_crosses_utc_midnight_in_est() {
        // Fri 2026-11-06 18:59:59 ET = 23:59:59Z; 19:00 ET = Sat 00:00Z.
        assert_eq!(
            recap_decision(utc("2026-11-06T23:59:59Z")),
            SlotDecision::InWindow {
                trading_date: d("2026-11-06")
            }
        );
        assert_eq!(
            recap_decision(utc("2026-11-07T00:00:00Z")),
            SlotDecision::PastDeadline {
                trading_date: d("2026-11-06")
            }
        );
        // EDT: 19:00 ET = 23:00Z the same UTC day.
        assert!(matches!(
            recap_decision(utc("2026-10-29T23:00:00Z")),
            SlotDecision::PastDeadline { .. }
        ));
    }

    #[test]
    fn recap_early_close() {
        // Fri 2026-11-27 EST, 1 PM close: the window opens 13:45 ET = 18:45Z.
        assert_eq!(
            recap_decision(utc("2026-11-27T18:44:00Z")),
            SlotDecision::TooEarly
        );
        assert!(matches!(
            recap_decision(utc("2026-11-27T18:45:00Z")),
            SlotDecision::InWindow { .. }
        ));
        // The 7 PM deadline is the same as on full days.
        assert!(matches!(
            recap_decision(utc("2026-11-27T23:55:00Z")),
            SlotDecision::InWindow { .. }
        ));
        assert!(matches!(
            recap_decision(utc("2026-11-28T00:00:00Z")),
            SlotDecision::PastDeadline { .. }
        ));
        // Christmas Eve 2026 is the other early close.
        assert_eq!(
            recap_decision(utc("2026-12-24T18:45:00Z")),
            SlotDecision::InWindow {
                trading_date: d("2026-12-24")
            }
        );
        // Thanksgiving: closed.
        assert_eq!(
            recap_decision(utc("2026-11-26T22:00:00Z")),
            SlotDecision::NotToday
        );
    }

    #[test]
    fn windows_per_day_kind() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert_eq!(
            recap_window(DayKind::Full),
            Window {
                start: t(16, 45),
                end: t(19, 0)
            }
        );
        assert_eq!(recap_window(DayKind::EarlyClose).start, t(13, 45));
        assert_eq!(
            premarket_window(DayKind::EarlyClose),
            premarket_window(DayKind::Full)
        );
    }

    #[test]
    fn early_close_day_is_in_window() {
        assert_eq!(
            premarket_decision(utc("2026-11-27T13:30:00Z")),
            SlotDecision::InWindow {
                trading_date: d("2026-11-27")
            }
        );
    }
}
