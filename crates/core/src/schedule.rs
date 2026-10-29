//! Which jobs each cron firing enqueues. Cron times are UTC; `clock` decides
//! what a firing means in ET. Day-of-week uses names because Cloudflare
//! numbers days 1-7 from Sunday, unlike most cron dialects.

/// Every job name the queue consumer understands.
pub const JOBS: &[&str] = &["premarket", "recap"];

/// `(cron, jobs)`. Must match `crons` in `crates/generator/wrangler.toml`.
pub const CRONS: &[(&str, &[&str])] = &[
    // Premarket 08:30-09:25 ET under both offsets (12:30Z EDT, 13:30Z EST).
    ("*/5 12-14 * * MON-FRI", &["premarket"]),
    // Hourly premarket ZH retries through the 18:00 ET give-up.
    ("0 15-23 * * MON-FRI", &["premarket"]),
    // Recap: early close opens 17:45Z (EDT) / 18:45Z (EST); full days
    // 20:45Z (EDT) / 21:45Z (EST); EDT deadline 23:00Z.
    ("*/5 17-23 * * MON-FRI", &["recap"]),
    // EST 19:00-19:55 ET is 00:00-00:55Z the next UTC day: the deadline skip
    // and the first hour of ZH retries.
    ("*/5 0 * * TUE-SAT", &["recap"]),
    // Hourly recap ZH retries through the 22:00 ET give-up.
    ("0 1-3 * * TUE-SAT", &["recap"]),
];

/// The jobs for a firing of `cron`, or `None` when the cron is unknown (the
/// Worker then runs every job, which is safe: each tick checks the clock).
pub fn jobs_for_cron(cron: &str) -> Option<&'static [&'static str]> {
    CRONS
        .iter()
        .find(|(c, _)| *c == cron)
        .map(|(_, jobs)| *jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{
        et, premarket_decision, premarket_window, recap_decision, recap_window, trading_day,
        SlotDecision, Window,
    };
    use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Timelike, Utc, Weekday};

    const WRANGLER: &str = include_str!("../../generator/wrangler.toml");

    /// The `crons = [...]` array of wrangler.toml.
    fn wrangler_crons() -> Vec<String> {
        let line = WRANGLER
            .lines()
            .find(|l| l.trim_start().starts_with("crons"))
            .expect("crons line");
        let list = &line[line.find('[').unwrap() + 1..line.rfind(']').unwrap()];
        list.split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn dow(name: &str) -> u32 {
        ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"]
            .iter()
            .position(|d| *d == name)
            .unwrap_or_else(|| panic!("use day names, not {name:?}")) as u32
    }

    /// One field: `*`, `*/n`, `a`, `a-b` (day names in the weekday field).
    fn field(spec: &str, v: u32, weekday: bool) -> bool {
        let num = |s: &str| if weekday { dow(s) } else { s.parse().unwrap() };
        if spec == "*" {
            return true;
        }
        if let Some(step) = spec.strip_prefix("*/") {
            return v.is_multiple_of(step.parse::<u32>().unwrap());
        }
        match spec.split_once('-') {
            Some((a, b)) => num(a) <= v && v <= num(b),
            None => num(spec) == v,
        }
    }

    fn matches(cron: &str, t: DateTime<Utc>) -> bool {
        let f: Vec<&str> = cron.split_whitespace().collect();
        assert_eq!(f.len(), 5, "{cron}");
        field(f[0], t.minute(), false)
            && field(f[1], t.hour(), false)
            && field(f[2], t.day(), false)
            && field(f[3], t.month(), false)
            && field(f[4], t.weekday().num_days_from_sunday(), true)
    }

    fn fires(job: &str, t: DateTime<Utc>) -> bool {
        CRONS
            .iter()
            .any(|(c, jobs)| jobs.contains(&job) && matches(c, t))
    }

    /// Every minute from 00:00 ET on `date` to 06:00 ET the next day.
    fn minutes(date: &str) -> Vec<DateTime<Utc>> {
        let d: NaiveDate = date.parse().unwrap();
        let start = d
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(chrono_tz::America::New_York)
            .unwrap()
            .with_timezone(&Utc);
        (0..30 * 60).map(|m| start + Duration::minutes(m)).collect()
    }

    // Thu/Fri EDT, Mon/Fri EST (DST ends 2026-11-01), early close, Christmas Eve.
    const DAYS: [&str; 6] = [
        "2026-10-29",
        "2026-10-30",
        "2026-11-02",
        "2026-11-06",
        "2026-11-27",
        "2026-12-24",
    ];

    #[test]
    fn table_matches_wrangler_toml() {
        let mine: Vec<&str> = CRONS.iter().map(|(c, _)| *c).collect();
        assert_eq!(wrangler_crons(), mine);
        for (_, jobs) in CRONS {
            assert!(!jobs.is_empty() && jobs.iter().all(|j| JOBS.contains(j)));
        }
        assert_eq!(jobs_for_cron("*/5 0 * * TUE-SAT"), Some(&["recap"][..]));
        // Each cron serves one job, so a recap firing never adds premarket
        // ZH retries and the reverse.
        let premarket: Vec<&str> = CRONS
            .iter()
            .filter(|(_, j)| j.contains(&"premarket"))
            .map(|(c, _)| *c)
            .collect();
        assert_eq!(premarket, ["*/5 12-14 * * MON-FRI", "0 15-23 * * MON-FRI"]);
        assert!(CRONS.iter().all(|(_, j)| j.len() == 1));
        assert_eq!(jobs_for_cron("* * * * *"), None);
    }

    /// Every 5 minutes inside the window, plus the deadline minute itself.
    fn assert_window_covered(
        job: &str,
        window: fn(crate::clock::DayKind) -> Window,
        decision: fn(DateTime<Utc>) -> SlotDecision,
    ) {
        for date in DAYS {
            let d: NaiveDate = date.parse().unwrap();
            let w = window(trading_day(d));
            let mut in_window = 0;
            for t in minutes(date) {
                let local = et(t);
                if local.date_naive() != d || !local.minute().is_multiple_of(5) {
                    continue;
                }
                if matches!(decision(t), SlotDecision::InWindow { .. }) {
                    in_window += 1;
                    assert!(fires(job, t), "{job}: no firing at {t} ({local})");
                }
                if local.time() == w.end {
                    assert!(fires(job, t), "{job}: no deadline firing at {t} ({local})");
                }
            }
            assert!(in_window > 0, "{date}");
        }
    }

    #[test]
    fn premarket_window_and_deadline_covered() {
        assert_window_covered("premarket", premarket_window, premarket_decision);
    }

    #[test]
    fn recap_window_and_deadline_covered() {
        assert_window_covered("recap", recap_window, recap_decision);
    }

    /// At least one firing in each ET hour from `from` through `to`, on the
    /// trading date (ZH retries and the give-up).
    fn assert_hourly(job: &str, from: u32, to: u32) {
        for date in DAYS {
            let d: NaiveDate = date.parse().unwrap();
            for h in from..=to {
                let hit = minutes(date).into_iter().any(|t| {
                    let l = et(t);
                    l.date_naive() == d && l.hour() == h && fires(job, t)
                });
                assert!(hit, "{job}: no firing in {h}:00 ET on {date}");
            }
            // The give-up hour fires on the hour, not later.
            let give_up = minutes(date).into_iter().find(|t| {
                let l = et(*t);
                l.date_naive() == d && l.time() == NaiveTime::from_hms_opt(to, 0, 0).unwrap()
            });
            assert!(fires(job, give_up.unwrap()), "{job}: {to}:00 ET on {date}");
        }
    }

    #[test]
    fn zh_retries_reach_the_give_up() {
        assert_hourly("premarket", 10, 18);
        assert_hourly("recap", 19, 22);
    }

    #[test]
    fn jobs_never_fire_on_utc_sunday() {
        // Sunday ET evening would be UTC Monday 00-03; Saturday UTC 00-03 is
        // Friday ET and must fire.
        let sun: DateTime<Utc> = "2026-11-08T01:00:00Z".parse().unwrap();
        assert_eq!(sun.weekday(), Weekday::Sun);
        let mon_utc: DateTime<Utc> = "2026-11-09T01:00:00Z".parse().unwrap();
        for (c, _) in CRONS {
            assert!(!matches(c, sun), "{c}");
            assert!(!matches(c, mon_utc), "{c}");
        }
    }
}
