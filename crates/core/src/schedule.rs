//! Which jobs each cron firing enqueues. Cron times are UTC; `clock` decides
//! what a firing means in ET. Day-of-week uses names because Cloudflare
//! numbers days 1-7 from Sunday, unlike most cron dialects.

/// Every job a cron queues.
pub const JOBS: &[&str] = &["premarket", "premarket-update", "recap", "earnings"];

/// Queue binding for the earnings polls and calendar refresh: one consumer,
/// one message at a time, so a poll never runs twice at once and an
/// earnings step never delays a brief.
pub const EARNINGS_QUEUE: &str = "EARNINGS";
/// Queue binding for earnings builds, one at a time beside the polls, so a
/// burst of builds never delays detection.
pub const EARNINGS_BUILD_QUEUE: &str = "EARNINGS_BUILD";
/// Queue binding for every other job.
pub const JOBS_QUEUE: &str = "JOBS";

/// A content job: a `run::Content` tick that needs the market, news and LLM
/// providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentJob {
    Premarket,
    /// `premarket-update`: the 08:35 ET rebuild of the day's brief, its
    /// own message so it gets its own subrequest budget.
    PremarketUpdate,
    Recap,
}

/// One step of the earnings pipeline, each its own queue message (and so
/// its own invocation and subrequest budget).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarningsStep {
    /// `earnings`, from the cron: read EDGAR's current 8-K feed back to
    /// the last poll.
    Poll,
    /// `earnings:build`, queued by a poll: build at most one filing.
    Build,
    /// `earnings:calendar`, queued by a poll once per ET day.
    Calendar,
}

impl EarningsStep {
    /// The queue message's `job` value.
    pub fn job(self) -> &'static str {
        match self {
            EarningsStep::Poll => "earnings",
            EarningsStep::Build => "earnings:build",
            EarningsStep::Calendar => "earnings:calendar",
        }
    }
}

/// What a queue message's `job` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Content(ContentJob),
    Earnings(EarningsStep),
}

impl JobKind {
    pub fn parse(job: &str) -> Option<JobKind> {
        match job {
            "premarket" => Some(JobKind::Content(ContentJob::Premarket)),
            "premarket-update" => Some(JobKind::Content(ContentJob::PremarketUpdate)),
            "recap" => Some(JobKind::Content(ContentJob::Recap)),
            "earnings" => Some(JobKind::Earnings(EarningsStep::Poll)),
            "earnings:build" => Some(JobKind::Earnings(EarningsStep::Build)),
            "earnings:calendar" => Some(JobKind::Earnings(EarningsStep::Calendar)),
            _ => None,
        }
    }
}

/// The queue binding a job is sent to.
pub fn queue_for(job: &str) -> &'static str {
    match JobKind::parse(job) {
        Some(JobKind::Earnings(EarningsStep::Build)) => EARNINGS_BUILD_QUEUE,
        Some(JobKind::Earnings(_)) => EARNINGS_QUEUE,
        _ => JOBS_QUEUE,
    }
}

/// `(cron, jobs)`. Must match `crons` in `crates/generator/wrangler.toml`.
/// Two crons because the Free Workers plan allows 5 per account. A firing
/// may queue several jobs; each job's clock (and `Content::zh_retry_due`)
/// decides what the firing means for it, so extra firings cost a D1 read.
pub const CRONS: &[(&str, &[&str])] = &[
    // 10:00-23:55Z weekdays. Earnings polling from 06:00 ET under both
    // offsets (10:00Z EDT, 11:00Z EST), on its own queue. Premarket
    // 08:00-09:25 ET (12:00Z EDT, 13:00Z EST) and its ZH retries to the
    // 18:00 ET give-up; the premarket update 08:35-09:25 ET; the recap
    // window from 13:45 ET (early close) or 16:45 ET to 19:00 ET EDT
    // (23:00Z), with its ZH retries. The clock gates the earlier firings:
    // they read D1 and do nothing.
    (
        "*/5 10-23 * * MON-FRI",
        &["premarket", "premarket-update", "recap", "earnings"],
    ),
    // 00:00-03:55Z the next UTC day: the EST recap tail to 19:00 ET
    // (00:00Z) and recap ZH retries to the 22:00 ET give-up; earnings
    // polling to 22:00 ET (02:00Z EDT, 03:00Z EST).
    ("*/5 0-3 * * TUE-SAT", &["recap", "earnings"]),
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
        et, premarket_decision, premarket_update_decision, premarket_update_window,
        premarket_window, recap_decision, recap_window, trading_day, SlotDecision, Window,
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
        // Free Workers plan: at most 5 cron triggers per account; keep 2.
        assert_eq!(CRONS.len(), 2);
        // No comma lists: one cron, one pattern per field.
        assert!(CRONS.iter().all(|(c, _)| !c.contains(',')));
        assert_eq!(jobs_for_cron("* * * * *"), None);
    }

    /// The `[[queues.consumers]]` block of `queue` in wrangler.toml.
    fn consumer(queue: &str) -> String {
        WRANGLER
            .split("[[queues.consumers]]")
            .skip(1)
            .map(|b| b.split("\n[").next().unwrap_or(b))
            .find(|b| b.contains(&format!("queue = \"{queue}\"")))
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn earnings_consumers_are_serial_and_retry_once() {
        for queue in ["prebell-earnings", "prebell-earnings-build"] {
            let c = consumer(queue);
            let setting = |k: &str| {
                c.lines()
                    .find(|l| l.trim_start().starts_with(k))
                    .and_then(|l| l.split('=').nth(1))
                    .map(|v| v.trim().to_string())
            };
            assert_eq!(setting("max_batch_size").as_deref(), Some("1"), "{c}");
            assert_eq!(setting("max_concurrency").as_deref(), Some("1"), "{c}");
            // A poll killed for CPU is redelivered once, not 3 times.
            assert_eq!(setting("max_retries").as_deref(), Some("1"), "{c}");
        }
        assert!(WRANGLER.contains(&format!(
            "binding = \"{EARNINGS_BUILD_QUEUE}\"\nqueue = \"prebell-earnings-build\""
        )));
    }

    #[test]
    fn each_cron_maps_to_its_job() {
        let want: [(&str, &[&str]); 2] = [
            (
                "*/5 10-23 * * MON-FRI",
                &["premarket", "premarket-update", "recap", "earnings"],
            ),
            ("*/5 0-3 * * TUE-SAT", &["recap", "earnings"]),
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
            JobKind::parse("premarket-update"),
            Some(JobKind::Content(ContentJob::PremarketUpdate))
        );
        assert_eq!(
            JobKind::parse("recap"),
            Some(JobKind::Content(ContentJob::Recap))
        );
        assert_eq!(JobKind::parse("nope"), None);
        for step in [
            EarningsStep::Poll,
            EarningsStep::Build,
            EarningsStep::Calendar,
        ] {
            assert_eq!(JobKind::parse(step.job()), Some(JobKind::Earnings(step)));
            let queue = match step {
                EarningsStep::Build => EARNINGS_BUILD_QUEUE,
                _ => EARNINGS_QUEUE,
            };
            assert_eq!(queue_for(step.job()), queue);
        }
        for j in ["premarket", "premarket-update", "recap", "nope"] {
            assert_eq!(queue_for(j), JOBS_QUEUE, "{j}");
        }
        let kinds: Vec<JobKind> = JOBS.iter().map(|j| JobKind::parse(j).unwrap()).collect();
        assert_eq!(kinds.len(), 4);
        for (i, k) in kinds.iter().enumerate() {
            assert!(!kinds[..i].contains(k), "{k:?}");
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
                if fires("premarket-update", t) {
                    assert_eq!(premarket_update_decision(t), SlotDecision::NotToday, "{t}");
                }
                if fires("recap", t) {
                    assert_eq!(recap_decision(t), SlotDecision::NotToday, "{t}");
                }
            }
        }
    }

    /// Every 5-minute ET slot of the earnings window (weekdays 06:00-22:00
    /// ET, holidays included: EDGAR accepts filings on exchange holidays)
    /// has an earnings firing, under EDT and EST; outside it no firing
    /// polls, because `in_window` gates each one.
    #[test]
    fn earnings_window_covered_on_edt_est_dst_and_holidays() {
        use crate::earnings::in_window;
        // EDT Thu/Fri, the DST Sunday, EST Mon/Fri, Thanksgiving, Christmas Eve.
        let days = [
            "2026-10-29",
            "2026-10-30",
            "2026-11-01",
            "2026-11-02",
            "2026-11-06",
            "2026-11-26",
            "2026-12-24",
        ];
        for date in days {
            let d: NaiveDate = date.parse().unwrap();
            let mut slots = 0;
            for t in minutes(date) {
                let local = et(t);
                if local.date_naive() != d || !local.minute().is_multiple_of(5) {
                    continue;
                }
                if in_window(t) {
                    slots += 1;
                    assert!(fires("earnings", t), "earnings: no firing at {t} ({local})");
                }
            }
            // 16 hours of 5-minute slots on a weekday; none on Sunday.
            let want = if d.weekday() == Weekday::Sun {
                0
            } else {
                16 * 12
            };
            assert_eq!(slots, want, "{date}");
        }
        // The first and last slots under each offset.
        for t in [
            "2026-10-30T10:00:00Z", // 06:00 EDT
            "2026-10-31T01:55:00Z", // Fri 21:55 EDT
            "2026-11-02T11:00:00Z", // 06:00 EST
            "2026-11-03T02:55:00Z", // Mon 21:55 EST
        ] {
            let t: DateTime<Utc> = t.parse().unwrap();
            assert!(in_window(t) && fires("earnings", t), "{t}");
        }
        // Before 06:00 ET a firing (EDT 10:00Z is EST 05:00) is a no-op.
        let early: DateTime<Utc> = "2026-11-02T10:00:00Z".parse().unwrap();
        assert!(fires("earnings", early) && !in_window(early));
    }

    /// Premarket, update and recap firings before their windows (the
    /// widened 10:00-11:55Z hours) are clock-gated: nothing builds.
    #[test]
    fn early_firings_never_build_premarket_or_recap() {
        for date in DAYS {
            let d: NaiveDate = date.parse().unwrap();
            for t in minutes(date) {
                let l = et(t);
                if l.date_naive() != d || l.hour() >= 8 {
                    continue;
                }
                if fires("premarket", t) {
                    assert!(
                        !matches!(premarket_decision(t), SlotDecision::InWindow { .. }),
                        "{t}"
                    );
                }
                if fires("premarket-update", t) {
                    assert!(
                        !matches!(premarket_update_decision(t), SlotDecision::InWindow { .. }),
                        "{t}"
                    );
                }
                if fires("recap", t) {
                    assert!(
                        !matches!(recap_decision(t), SlotDecision::InWindow { .. }),
                        "{t}"
                    );
                }
            }
        }
    }

    #[test]
    fn premarket_window_and_deadline_covered() {
        assert_window_covered("premarket", premarket_window, premarket_decision);
    }

    #[test]
    fn premarket_update_window_and_deadline_covered() {
        assert_window_covered(
            "premarket-update",
            premarket_update_window,
            premarket_update_decision,
        );
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
