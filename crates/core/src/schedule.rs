//! Which jobs each cron firing enqueues. Cron times are UTC; `clock` decides
//! what a firing means in ET. Day-of-week uses names because Cloudflare
//! numbers days 1-7 from Sunday, unlike most cron dialects.

/// Every job name the queue consumer understands.
pub const JOBS: &[&str] = &["premarket", "recap", "competitor"];

/// A content job: a `run::Content` tick that needs the market, news and LLM
/// providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentJob {
    Premarket,
    Recap,
}

/// What a queue message's `job` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Content(ContentJob),
    /// The competitor watch; it needs no LLM.
    Competitor,
}

impl JobKind {
    pub fn parse(job: &str) -> Option<JobKind> {
        match job {
            "premarket" => Some(JobKind::Content(ContentJob::Premarket)),
            "recap" => Some(JobKind::Content(ContentJob::Recap)),
            "competitor" => Some(JobKind::Competitor),
            _ => None,
        }
    }
}

/// `(cron, jobs)`. Must match `crons` in `crates/generator/wrangler.toml`.
/// Three crons because the Free Workers plan allows 5 per account. A firing
/// may queue several jobs; each job's clock (and `Content::zh_retry_due`)
/// decides what the firing means for it, so extra firings cost a D1 read.
pub const CRONS: &[(&str, &[&str])] = &[
    // 12:00-23:55Z weekdays. Premarket 08:30-09:25 ET under both offsets
    // (12:30Z EDT, 13:30Z EST) and its ZH retries to the 18:00 ET give-up;
    // the recap window from 13:45 ET (early close) or 16:45 ET to 19:00 ET
    // EDT (23:00Z), with its ZH retries.
    ("*/5 12-23 * * MON-FRI", &["premarket", "recap"]),
    // 00:00-03:55Z the next UTC day: the EST recap tail to 19:00 ET
    // (00:00Z) and recap ZH retries to the 22:00 ET give-up.
    ("*/5 0-3 * * TUE-SAT", &["recap"]),
    // Competitor watch, daily: 07:31 ET under either offset (11:31Z EDT,
    // 12:31Z EST); `competitor_decision` picks the firing that counts.
    // Minute 31 so no other job fires with it.
    ("31 11-12 * * *", &["competitor"]),
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
        // Quoted strings, so a comma inside a cron does not split it.
        list.split('"')
            .skip(1)
            .step_by(2)
            .map(String::from)
            .collect()
    }

    const DAYS_OF_WEEK: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

    fn dow(name: &str) -> Option<u32> {
        DAYS_OF_WEEK
            .iter()
            .position(|d| *d == name)
            .map(|i| i as u32)
    }

    /// One field: a comma list of `*`, `*/n`, `a` or `a-b` (day names in the
    /// weekday field). Anything else never matches.
    fn field(spec: &str, v: u32, weekday: bool) -> bool {
        let num = |s: &str| if weekday { dow(s) } else { s.parse().ok() };
        spec.split(',').any(|part| {
            if part == "*" {
                return true;
            }
            if let Some(step) = part.strip_prefix("*/") {
                return step
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > 0)
                    .is_some_and(|n| v.is_multiple_of(n));
            }
            match part.split_once('-') {
                Some((a, b)) => matches!((num(a), num(b)), (Some(a), Some(b)) if a <= v && v <= b),
                None => num(part) == Some(v),
            }
        })
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
        // Day names, not numbers: Cloudflare numbers 1 = Sunday.
        for (c, _) in CRONS {
            let dow_field = c.split_whitespace().nth(4).unwrap();
            for part in dow_field.split([',', '-']).filter(|p| *p != "*") {
                assert!(dow(part).is_some(), "{c}: use day names, not {part:?}");
            }
        }
        for (_, jobs) in CRONS {
            assert!(!jobs.is_empty() && jobs.iter().all(|j| JOBS.contains(j)));
        }
        // Free Workers plan: at most 5 cron triggers per account; keep 3.
        assert_eq!(CRONS.len(), 3);
        // No comma lists: one cron, one pattern per field.
        assert!(CRONS.iter().all(|(c, _)| !c.contains(',')));
        assert_eq!(jobs_for_cron("* * * * *"), None);
    }

    #[test]
    fn each_cron_maps_to_its_job() {
        let want: [(&str, &[&str]); 3] = [
            ("*/5 12-23 * * MON-FRI", &["premarket", "recap"]),
            ("*/5 0-3 * * TUE-SAT", &["recap"]),
            ("31 11-12 * * *", &["competitor"]),
        ];
        for (cron, jobs) in want {
            assert_eq!(jobs_for_cron(cron), Some(jobs), "{cron}");
        }
        assert_eq!(CRONS.len(), want.len());
        // Every job has a cron.
        for j in JOBS {
            assert!(CRONS.iter().any(|(_, jobs)| jobs.contains(j)), "{j}");
        }
    }

    #[test]
    fn job_kind_parses_every_job() {
        assert_eq!(
            JobKind::parse("premarket"),
            Some(JobKind::Content(ContentJob::Premarket))
        );
        assert_eq!(
            JobKind::parse("recap"),
            Some(JobKind::Content(ContentJob::Recap))
        );
        assert_eq!(JobKind::parse("competitor"), Some(JobKind::Competitor));
        assert_eq!(JobKind::parse("nope"), None);
        let kinds: Vec<JobKind> = JOBS.iter().map(|j| JobKind::parse(j).unwrap()).collect();
        assert_eq!(kinds.len(), 3);
        for (i, k) in kinds.iter().enumerate() {
            assert!(!kinds[..i].contains(k), "{k:?}");
        }
    }

    #[test]
    fn competitor_fires_once_at_0731_et_every_day() {
        // The competitor job is due in [07:30, 08:00) ET; exactly one of its
        // firings lands there each day. Includes a Saturday, a Sunday and the
        // 2026-11-01 DST change.
        let (start, end) = (
            NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        );
        for date in [
            "2026-10-29",
            "2026-10-31",
            "2026-11-01",
            "2026-11-02",
            "2026-11-26",
        ] {
            let d: NaiveDate = date.parse().unwrap();
            let due: Vec<DateTime<Utc>> = minutes(date)
                .into_iter()
                .filter(|t| et(*t).date_naive() == d && fires("competitor", *t))
                .filter(|t| (start..end).contains(&et(*t).time()))
                .collect();
            assert_eq!(due.len(), 1, "{date}: {due:?}");
            assert_eq!(
                et(due[0]).time(),
                NaiveTime::from_hms_opt(7, 31, 0).unwrap()
            );
            // Minute 31: no other job fires with it.
            assert!(!fires("premarket", due[0]) && !fires("recap", due[0]));
        }
    }

    #[test]
    fn field_reads_comma_lists_and_never_panics() {
        let t: DateTime<Utc> = "2026-10-28T11:30:00Z".parse().unwrap(); // Wed
        assert!(matches("0,30 11 * * MON,WED", t));
        assert!(matches("15,30-35 9-10,11 * * MON-TUE,WED", t));
        assert!(!matches("0,45 11 * * *", t));
        assert!(!matches("30 11 * * MON,FRI", t));
        // Malformed fields never match and never panic.
        for bad in [
            "x 11 * * *",
            "30 11 * * FUNDAY",
            "*/0 * * * *",
            ", * * * *",
            "1-x * * * *",
        ] {
            assert!(!matches(bad, t), "{bad}");
        }
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

    /// Closed days, including the 2026-11-01 DST change (a Sunday) and
    /// weekday holidays the weekday cron still fires on: every firing's
    /// clock decision is `NotToday`, so nothing builds.
    #[test]
    fn closed_days_never_build() {
        for date in ["2026-11-01", "2026-11-26", "2026-12-25", "2026-10-31"] {
            let d: NaiveDate = date.parse().unwrap();
            assert_eq!(trading_day(d), crate::clock::DayKind::Closed, "{date}");
            for t in minutes(date) {
                if et(t).date_naive() != d {
                    continue;
                }
                if fires("premarket", t) {
                    assert_eq!(premarket_decision(t), SlotDecision::NotToday, "{t}");
                }
                if fires("recap", t) {
                    assert_eq!(recap_decision(t), SlotDecision::NotToday, "{t}");
                }
            }
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
