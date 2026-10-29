//! Runs one queue message. A content or earnings job whose providers cannot
//! be built (a bad `LLM_PROVIDER` or `LLM_EFFORT`, a placeholder
//! `SEC_CONTACT`) never reaches its tick, so no run row exists and `skip`
//! never alerts: the config error alerts instead, once per job per ET date.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::clock::et;
use crate::run::TickOutcome;
use crate::schedule::{ContentJob, EarningsStep, JobKind};
use crate::store::Store;

/// What the Worker supplies to run a job.
#[async_trait(?Send)]
pub trait JobRunner {
    /// Builds the providers for `job` (per call, so one bad config fails
    /// only the jobs that need it) and ticks it. `Err` is a config error
    /// whose text must already be free of secret values (see `redact`).
    async fn content(&self, job: ContentJob) -> Result<TickOutcome, String>;
    /// The competitor watch; its outcome as a log line.
    async fn competitor(&self) -> String;
    /// One earnings step. `Ok` is a log line; `Err` is a config error, as
    /// for `content`.
    async fn earnings(&self, step: EarningsStep) -> Result<String, String>;
}

/// Alert text is at most this many characters.
const MAX_ALERT_CHARS: usize = 300;

/// One queue message. `Ok` is a log line, `Err` an error log line.
pub async fn run_job(
    now: DateTime<Utc>,
    job: &str,
    store: &dyn Store,
    runner: &dyn JobRunner,
) -> Result<String, String> {
    match JobKind::parse(job) {
        Some(JobKind::Competitor) => Ok(format!("job {job}: {}", runner.competitor().await)),
        Some(JobKind::Content(c)) => match runner.content(c).await {
            Ok(o) => Ok(format!("job {job}: {o:?}")),
            Err(e) => Err(config_error(now, job, &e, store).await),
        },
        Some(JobKind::Earnings(step)) => match runner.earnings(step).await {
            Ok(line) => Ok(format!("job {job}: {line}")),
            // One alert per day for every earnings step: they share the config.
            Err(e) => Err(config_error(now, "earnings", &e, store).await),
        },
        None => Err(format!("job {job}: unknown job, dropped")),
    }
}

/// Alerts a config error once per ET day; the error log line.
async fn config_error(now: DateTime<Utc>, job: &str, e: &str, store: &dyn Store) -> String {
    let msg = format!("{job}: config error: {e}");
    let note = match alert_once_today(now, &format!("config:{job}"), "error", &msg, store).await {
        Ok(true) => "alerted",
        Ok(false) => "already alerted today",
        Err(se) => return format!("job {job}: config error: {e}; alert failed: {se}"),
    };
    format!("job {job}: config error ({note}): {e}")
}

/// Adds the `<key>:<ET date>` alert unless it exists, as one line of at most
/// `MAX_ALERT_CHARS` characters. Returns whether it added one.
pub async fn alert_once_today(
    now: DateTime<Utc>,
    key: &str,
    level: &str,
    msg: &str,
    store: &dyn Store,
) -> Result<bool, String> {
    let key = format!("{key}:{}", et(now).format("%Y-%m-%d"));
    if store.has_alert(&key).await? {
        return Ok(false);
    }
    let msg: String = msg
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_ALERT_CHARS)
        .collect();
    store.add_alert(&key, level, &msg).await?;
    Ok(true)
}

/// `msg` with every non-empty `secrets` value replaced by `[redacted]`.
pub fn redact(msg: &str, secrets: &[&str]) -> String {
    secrets
        .iter()
        .filter(|s| !s.is_empty())
        .fold(msg.to_string(), |m, s| m.replace(s, "[redacted]"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemStore;
    use std::cell::{Cell, RefCell};

    const KEY: &str = "sk-ant-api03-SECRETVALUE";

    /// Content jobs fail with `content_err`; counts each call.
    struct FakeRunner {
        content_err: Option<String>,
        content_calls: RefCell<Vec<ContentJob>>,
        competitor_calls: Cell<usize>,
        earnings_calls: RefCell<Vec<EarningsStep>>,
    }
    #[async_trait(?Send)]
    impl JobRunner for FakeRunner {
        async fn content(&self, job: ContentJob) -> Result<TickOutcome, String> {
            self.content_calls.borrow_mut().push(job);
            match &self.content_err {
                Some(e) => Err(e.clone()),
                None => Ok(TickOutcome::Nothing),
            }
        }
        async fn competitor(&self) -> String {
            self.competitor_calls.set(self.competitor_calls.get() + 1);
            "AlreadyDone".into()
        }
        async fn earnings(&self, step: EarningsStep) -> Result<String, String> {
            self.earnings_calls.borrow_mut().push(step);
            match &self.content_err {
                Some(e) => Err(e.clone()),
                None => Ok("polled".into()),
            }
        }
    }
    /// A runner whose LLM config has `LLM_EFFORT=hgih`, with the error text
    /// redacted the way the Worker does.
    fn bad_effort() -> FakeRunner {
        let raw = format!(
            "parse error: unknown LLM_EFFORT: hgih (use low, medium, high or none); key {KEY}"
        );
        FakeRunner {
            content_err: Some(redact(&raw, &[KEY, ""])),
            content_calls: RefCell::new(vec![]),
            competitor_calls: Cell::new(0),
            earnings_calls: RefCell::new(vec![]),
        }
    }
    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn bad_effort_alerts_once_per_job_per_et_day() {
        let s = MemStore::default();
        let r = bad_effort();
        // 08:30 and 08:35 ET on 2026-10-29.
        for t in ["2026-10-29T12:30:00Z", "2026-10-29T12:35:00Z"] {
            let out = run_job(utc(t), "premarket", &s, &r).await;
            assert!(
                matches!(&out, Err(e) if e.contains("config error")),
                "{out:?}"
            );
        }
        assert_eq!(r.content_calls.borrow().len(), 2);
        {
            let alerts = s.alerts.borrow();
            assert_eq!(alerts.len(), 1, "{alerts:?}");
            let (key, level, msg) = &alerts[0];
            assert_eq!(key, "config:premarket:2026-10-29");
            assert_eq!(level, "error");
            assert!(msg.starts_with("premarket: config error: "), "{msg}");
            assert!(msg.contains("LLM_EFFORT") && msg.contains("hgih"), "{msg}");
            assert!(!msg.contains(KEY) && !msg.contains("SECRET"), "{msg}");
        }
        // The recap job has its own alert; the next ET day alerts again.
        run_job(utc("2026-10-29T20:45:00Z"), "recap", &s, &r)
            .await
            .ok();
        run_job(utc("2026-10-30T12:30:00Z"), "premarket", &s, &r)
            .await
            .ok();
        let keys: Vec<String> = s.alerts.borrow().iter().map(|a| a.0.clone()).collect();
        assert_eq!(
            keys,
            [
                "config:premarket:2026-10-29",
                "config:recap:2026-10-29",
                "config:premarket:2026-10-30"
            ]
        );
    }

    #[tokio::test]
    async fn the_et_date_keys_the_dedupe() {
        // 23:30 ET on 10-29 is 03:30Z on 10-30: still the 10-29 alert.
        let s = MemStore::default();
        let r = bad_effort();
        run_job(utc("2026-10-29T20:00:00Z"), "recap", &s, &r)
            .await
            .ok();
        run_job(utc("2026-10-30T03:30:00Z"), "recap", &s, &r)
            .await
            .ok();
        assert_eq!(s.alerts.borrow().len(), 1);
    }

    #[tokio::test]
    async fn competitor_runs_despite_a_bad_llm_config() {
        let s = MemStore::default();
        let r = bad_effort();
        run_job(utc("2026-10-29T12:30:00Z"), "premarket", &s, &r)
            .await
            .ok();
        let out = run_job(utc("2026-10-29T11:31:00Z"), "competitor", &s, &r).await;
        assert_eq!(out, Ok("job competitor: AlreadyDone".to_string()));
        assert_eq!(r.competitor_calls.get(), 1);
        // No content build for the competitor job, and no alert for it.
        assert_eq!(r.content_calls.borrow().len(), 1);
        assert_eq!(s.alerts.borrow().len(), 1);
        assert!(s.alerts.borrow()[0].0.starts_with("config:premarket:"));
    }

    #[tokio::test]
    async fn good_config_ticks_without_alerts_and_unknown_job_is_dropped() {
        let s = MemStore::default();
        let r = FakeRunner {
            content_err: None,
            content_calls: RefCell::new(vec![]),
            competitor_calls: Cell::new(0),
            earnings_calls: RefCell::new(vec![]),
        };
        let now = utc("2026-10-29T12:30:00Z");
        assert_eq!(
            run_job(now, "recap", &s, &r).await,
            Ok("job recap: Nothing".to_string())
        );
        assert_eq!(*r.content_calls.borrow(), [ContentJob::Recap]);
        assert_eq!(
            run_job(now, "nope", &s, &r).await,
            Err("job nope: unknown job, dropped".to_string())
        );
        assert!(s.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn earnings_steps_share_one_config_alert_per_day() {
        let s = MemStore::default();
        let r = FakeRunner {
            content_err: Some("SEC_CONTACT is the placeholder".into()),
            content_calls: RefCell::new(vec![]),
            competitor_calls: Cell::new(0),
            earnings_calls: RefCell::new(vec![]),
        };
        for (t, job) in [
            ("2026-10-29T12:30:00Z", "earnings"),
            ("2026-10-29T12:35:00Z", "earnings:build"),
            ("2026-10-29T12:40:00Z", "earnings:calendar"),
        ] {
            let out = run_job(utc(t), job, &s, &r).await;
            assert!(
                matches!(&out, Err(e) if e.contains("config error")),
                "{out:?}"
            );
        }
        assert_eq!(
            *r.earnings_calls.borrow(),
            [
                EarningsStep::Poll,
                EarningsStep::Build,
                EarningsStep::Calendar
            ]
        );
        let alerts = s.alerts.borrow();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert_eq!(alerts[0].0, "config:earnings:2026-10-29");
        assert!(alerts[0].2.contains("SEC_CONTACT"), "{}", alerts[0].2);
    }

    #[tokio::test]
    async fn earnings_ok_is_a_log_line() {
        let s = MemStore::default();
        let r = FakeRunner {
            content_err: None,
            content_calls: RefCell::new(vec![]),
            competitor_calls: Cell::new(0),
            earnings_calls: RefCell::new(vec![]),
        };
        let out = run_job(utc("2026-10-29T12:30:00Z"), "earnings", &s, &r).await;
        assert_eq!(out, Ok("job earnings: polled".to_string()));
        assert!(s.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn alert_once_today_dedupes_by_key_and_et_date() {
        let s = MemStore::default();
        let t = utc("2026-10-29T20:00:00Z");
        assert_eq!(
            alert_once_today(t, "sec-throttled", "warn", "a\nb", &s).await,
            Ok(true)
        );
        assert_eq!(
            alert_once_today(t, "sec-throttled", "warn", "c", &s).await,
            Ok(false)
        );
        // 23:30 ET is still 10-29; the next ET day alerts again.
        let late = utc("2026-10-30T03:30:00Z");
        assert_eq!(
            alert_once_today(late, "sec-throttled", "warn", "c", &s).await,
            Ok(false)
        );
        let next = utc("2026-10-30T12:00:00Z");
        assert_eq!(
            alert_once_today(next, "sec-throttled", "warn", "c", &s).await,
            Ok(true)
        );
        let a = s.alerts.borrow();
        assert_eq!(a[0].0, "sec-throttled:2026-10-29");
        assert_eq!((a[0].1.as_str(), a[0].2.as_str()), ("warn", "a b"));
    }

    #[test]
    fn redact_replaces_every_secret_and_ignores_empty_ones() {
        assert_eq!(
            redact(&format!("bad {KEY} and {KEY}"), &[KEY, ""]),
            "bad [redacted] and [redacted]"
        );
        assert_eq!(redact("LLM_EFFORT='hgih'", &[""]), "LLM_EFFORT='hgih'");
    }

    #[tokio::test]
    async fn alert_text_is_one_short_line() {
        let s = MemStore::default();
        let r = FakeRunner {
            content_err: Some(format!("line one\nline two {}", "x".repeat(1000))),
            content_calls: RefCell::new(vec![]),
            competitor_calls: Cell::new(0),
            earnings_calls: RefCell::new(vec![]),
        };
        run_job(utc("2026-10-29T12:30:00Z"), "premarket", &s, &r)
            .await
            .ok();
        let msg = s.alerts.borrow()[0].2.clone();
        assert!(!msg.contains('\n'), "{msg}");
        assert!(msg.chars().count() <= 300, "{}", msg.len());
    }
}
