//! Runs one queue message. A content job whose providers cannot be built
//! (a bad `LLM_PROVIDER` or `LLM_EFFORT`) never reaches `tick`, so no run row
//! exists and `skip` never alerts: the config error alerts instead, once per
//! job per ET date.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::clock::et;
use crate::run::TickOutcome;
use crate::schedule::{ContentJob, JobKind};
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
            Err(e) => {
                let note = match config_alert(now, job, &e, store).await {
                    Ok(true) => "alerted",
                    Ok(false) => "already alerted today",
                    Err(se) => {
                        return Err(format!("job {job}: config error: {e}; alert failed: {se}"))
                    }
                };
                Err(format!("job {job}: config error ({note}): {e}"))
            }
        },
        None => Err(format!("job {job}: unknown job, dropped")),
    }
}

/// Adds the `config:<job>:<ET date>` error alert unless it exists. Returns
/// whether it added one.
async fn config_alert(
    now: DateTime<Utc>,
    job: &str,
    err: &str,
    store: &dyn Store,
) -> Result<bool, String> {
    let key = format!("config:{job}:{}", et(now).format("%Y-%m-%d"));
    if store.has_alert(&key).await? {
        return Ok(false);
    }
    let one_line: String = err
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let msg: String = format!("{job}: config error: {one_line}")
        .chars()
        .take(MAX_ALERT_CHARS)
        .collect();
    store.add_alert(&key, "error", &msg).await?;
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
        };
        run_job(utc("2026-10-29T12:30:00Z"), "premarket", &s, &r)
            .await
            .ok();
        let msg = s.alerts.borrow()[0].2.clone();
        assert!(!msg.contains('\n'), "{msg}");
        assert!(msg.chars().count() <= 300, "{}", msg.len());
    }
}
