//! Generator Worker: cron -> queue -> `tick`, backed by D1 + R2.

mod alerts;
mod earnings_store;
mod http;
mod store;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use prebell_core::earnings::calendar::{refresh_tick, yahoo_seeding, CalendarSource};
use prebell_core::earnings::run::{build_tick, poll_job};
use prebell_core::edgar::Edgar;
use prebell_core::jobs::{redact, run_job, JobRunner};
use prebell_core::llm::{api_key_var, llm_from_env, unknown_provider};
use prebell_core::providers::CountingHttp;
use prebell_core::providers::LlmProvider;
use prebell_core::recap;
use prebell_core::run::{tick_with_hooks, update, Hooks, TickOutcome};
use prebell_core::schedule::{
    jobs_for_cron, queue_for, ContentJob, EarningsStep, EARNINGS_QUEUE, JOBS,
};
use prebell_core::tavily::Tavily;
use prebell_core::yahoo::Yahoo;
use prebell_core::yahoo_calendar::YahooCalendar;
use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::{WorkerHttp, WorkerTimer};
use crate::store::D1Store;

const PLACEHOLDER_CONTACT: &str = "placeholder@example.com";

/// Queue message body. A struct (not `serde_json::Value`) so it crosses the
/// queue as a plain JS object rather than a JS `Map`.
#[derive(Serialize, Deserialize, Debug)]
struct Job {
    job: String,
    /// When the cron fired (RFC 3339): an earnings poll older than one
    /// firing interval is dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<String>,
}

fn text(env: &Env, name: &str) -> Option<String> {
    env.var(name)
        .ok()
        .map(|v| v.to_string())
        .filter(|v| !v.is_empty())
}

fn secret_or_empty(env: &Env, name: &str) -> String {
    let v = env
        .secret(name)
        .ok()
        .map(|v| v.to_string())
        .unwrap_or_default();
    if v.is_empty() {
        console_warn!("secret {name} is not set; provider calls will fail");
    }
    v
}

/// Wall clock from the JS runtime; `FORCE_NOW_UTC` (RFC 3339) overrides it for local testing.
fn now(env: &Env) -> DateTime<Utc> {
    if let Some(forced) = text(env, "FORCE_NOW_UTC") {
        match DateTime::parse_from_rfc3339(&forced) {
            Ok(t) => {
                console_warn!("FORCE_NOW_UTC is set: using {forced} instead of the wall clock");
                return t.with_timezone(&Utc);
            }
            Err(e) => console_warn!("ignoring FORCE_NOW_UTC={forced}: {e}"),
        }
    }
    DateTime::from_timestamp_millis(Date::now().as_millis() as i64).unwrap_or_default()
}

#[event(fetch)]
async fn fetch(req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    if req.method() == Method::Get && req.path() == "/health" {
        Response::ok("ok")
    } else {
        Response::error("not found", 404)
    }
}

#[event(scheduled)]
async fn scheduled(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let cron = event.cron();
    let jobs = jobs_for_cron(&cron).unwrap_or_else(|| {
        console_error!("cron {cron}: not in schedule.rs; queueing every job");
        JOBS
    });
    let at = now(&env).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for job in jobs {
        // Earnings go to their own queue (one consumer, one message at a
        // time) so EDGAR is never hit in parallel and a build never delays
        // the premarket brief.
        let binding = queue_for(job);
        let queue = match env.queue(binding) {
            Ok(q) => q,
            Err(e) => {
                console_error!("cron {cron}: queue binding {binding} failed: {e}");
                continue;
            }
        };
        match queue
            .send(Job {
                job: job.to_string(),
                at: Some(at.clone()),
            })
            .await
        {
            Ok(()) => console_log!("cron {cron}: queued {job} job"),
            Err(e) => console_error!("cron {cron}: queue send for {job} failed: {e}"),
        }
    }
}

/// The Worker side of `jobs::run_job`, for one queue message.
struct Runner<'a> {
    env: &'a Env,
    store: &'a D1Store,
    /// Every outgoing fetch goes through this, so `http.count()` is the
    /// invocation's fetch subrequests (D1 and R2 binding calls not included).
    http: CountingHttp<WorkerHttp>,
    /// When the message's cron fired.
    fired: Option<DateTime<Utc>>,
}

impl Runner<'_> {
    /// The LLM, built per message so a bad config fails only the jobs that
    /// need it. The error names the variable and its value, never a secret.
    fn llm(&self, other_secrets: &[&str]) -> std::result::Result<Box<dyn LlmProvider>, String> {
        let env = self.env;
        let provider = text(env, "LLM_PROVIDER").unwrap_or_else(|| "claude".into());
        let key_var =
            api_key_var(&provider).ok_or_else(|| unknown_provider(&provider).to_string())?;
        let llm_key = secret_or_empty(env, key_var);
        llm_from_env(
            &provider,
            self.http.clone(),
            llm_key.clone(),
            text(env, "LLM_MODEL"),
            text(env, "LLM_EFFORT"),
            |m| console_warn!("{m}"),
        )
        .map_err(|e| {
            let mut secrets = vec![llm_key.as_str()];
            secrets.extend_from_slice(other_secrets);
            redact(&e.to_string(), &secrets)
        })
    }

    /// EDGAR as `SEC_CONTACT`; SEC blocks anonymous clients, so the
    /// placeholder is a config error.
    fn edgar(&self) -> std::result::Result<Edgar<CountingHttp<WorkerHttp>, WorkerTimer>, String> {
        let contact = text(self.env, "SEC_CONTACT").unwrap_or_default();
        if contact == PLACEHOLDER_CONTACT {
            return Err(
                "SEC_CONTACT is the placeholder; set a real contact email to turn earnings on"
                    .into(),
            );
        }
        Edgar::new(self.http.clone(), WorkerTimer, &contact).map_err(|e| e.to_string())
    }
}

#[async_trait(?Send)]
impl JobRunner for Runner<'_> {
    /// A premarket, premarket update or recap tick.
    async fn content(&self, job: ContentJob) -> std::result::Result<TickOutcome, String> {
        let env = self.env;
        let md = Yahoo::new(self.http.clone());
        let news_key = secret_or_empty(env, "TAVILY_API_KEY");
        let ns = Tavily::new(self.http.clone(), news_key.clone());
        let llm = self.llm(&[&news_key])?;
        let log = |m: &str| console_log!("{m}");
        // Read when the facts are built, so a retrying run's last_error says
        // how close the build came to the Free plan's 50.
        let extra = || vec![format!("subrequests={}", self.http.count())];
        let hooks = Hooks {
            log: &log,
            extra: &extra,
            budget: &|| self.http.count(),
        };
        let at = now(env);
        let store = self.store;
        Ok(match job {
            ContentJob::Premarket => {
                tick_with_hooks(at, store, &md, &ns, llm.as_ref(), &hooks).await
            }
            ContentJob::PremarketUpdate => {
                update::tick_with_hooks(at, store, &md, &ns, llm.as_ref(), &hooks).await
            }
            ContentJob::Recap => {
                recap::tick_with_hooks(at, store, &md, &ns, llm.as_ref(), &hooks).await
            }
        })
    }

    /// One earnings step, each its own message and so its own subrequest
    /// budget: a poll (no LLM) queues the builds and the calendar refresh.
    async fn earnings(&self, step: EarningsStep) -> std::result::Result<String, String> {
        let env = self.env;
        let store = self.store;
        let t = now(env);
        let edgar = self.edgar()?;
        match step {
            EarningsStep::Poll => {
                let o = poll_job(t, self.fired, store, store, &edgar).await?;
                if o.stale {
                    return Ok("stale message, dropped".into());
                }
                for e in &o.errors {
                    console_warn!("earnings poll: {e}");
                }
                let queue = env.queue(EARNINGS_QUEUE).map_err(|e| e.to_string());
                for next in &o.follow_ups {
                    let sent = match &queue {
                        Ok(q) => q
                            .send(Job {
                                job: next.job().into(),
                                at: None,
                            })
                            .await
                            .map_err(|e| e.to_string()),
                        Err(e) => Err(e.clone()),
                    };
                    if let Err(e) = sent {
                        console_error!("earnings: queue send for {} failed: {e}", next.job());
                    }
                }
                Ok(format!(
                    "pages {}, entries {}, new {:?}, queued {:?}",
                    o.pages,
                    o.checked,
                    o.detected,
                    o.follow_ups.iter().map(|s| s.job()).collect::<Vec<_>>()
                ))
            }
            EarningsStep::Build => {
                let llm = self.llm(&[])?;
                let log = |m: &str| console_log!("{m}");
                let o = build_tick(t, store, store, &edgar, llm.as_ref(), &log).await?;
                Ok(format!("{o:?}"))
            }
            EarningsStep::Calendar => {
                let news_key = secret_or_empty(env, "TAVILY_API_KEY");
                let ns = Tavily::new(self.http.clone(), news_key.clone());
                let llm = self.llm(&[&news_key])?;
                let yahoo = yahoo_seeding(text(env, "EARNINGS_CALENDAR_SEED").as_deref())?;
                let cal = YahooCalendar::new(self.http.clone());
                let src: Option<&dyn CalendarSource> = if yahoo { Some(&cal) } else { None };
                let o = refresh_tick(t, store, store, src, &ns, llm.as_ref()).await?;
                Ok(format!("{o:?}"))
            }
        }
    }
}

#[event(queue)]
async fn queue(batch: MessageBatch<Job>, env: Env, _ctx: Context) -> Result<()> {
    let store = D1Store {
        db: env.d1("DB")?,
        facts: env.bucket("FACTS")?,
    };
    let http = CountingHttp::new(WorkerHttp);
    // Fetch subrequests this invocation has used (Free plan: 50).
    let mut used = 0;

    for msg in batch.messages()? {
        let body = msg.body();
        let job = body.job.as_str();
        http.reset();
        let runner = Runner {
            env: &env,
            store: &store,
            http: http.clone(),
            fired: body
                .at
                .as_deref()
                .and_then(|a| DateTime::parse_from_rfc3339(a).ok())
                .map(|a| a.with_timezone(&Utc)),
        };
        // A config error is alerted (once per job per ET day) and acked, not
        // retried: a retry would fail the same way, and the other messages
        // and the alert delivery below must still run.
        match run_job(now(&env), job, &store, &runner).await {
            Ok(line) => console_log!("{line}"),
            Err(line) => console_error!("{line}"),
        }
        // Free plan: fetch subrequests this job used (limit 50 per invocation).
        console_log!("subrequests={} job={job}", http.count());
        used += http.count();
        msg.ack();
    }

    http.reset();
    // Whatever the jobs left of the budget; the rest wait for the next invocation.
    let budget = alerts::delivery_budget(used);
    alerts::deliver(
        &store,
        &http,
        text(&env, "ALERT_WEBHOOK_URL").as_deref(),
        budget,
    )
    .await;
    console_log!("subrequests={} job=alerts", http.count());
    Ok(())
}
