//! Generator Worker: cron -> queue -> `tick`, backed by D1 + R2.

mod alerts;
mod competitor;
mod http;
mod store;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use prebell_core::competitor::run_competitor;
use prebell_core::jobs::{redact, run_job, JobRunner};
use prebell_core::llm::{api_key_var, llm_from_env, unknown_provider};
use prebell_core::providers::CountingHttp;
use prebell_core::recap;
use prebell_core::run::{tick_with_hooks, Hooks, TickOutcome};
use prebell_core::schedule::{jobs_for_cron, ContentJob, JOBS};
use prebell_core::tavily::Tavily;
use prebell_core::yahoo::Yahoo;
use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::{WorkerHttp, WorkerTimer};
use crate::store::D1Store;

/// Queue message body. A struct (not `serde_json::Value`) so it crosses the
/// queue as a plain JS object rather than a JS `Map`.
#[derive(Serialize, Deserialize, Debug)]
struct Job {
    job: String,
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
    let queue = match env.queue("JOBS") {
        Ok(q) => q,
        Err(e) => {
            console_error!("cron {cron}: queue binding failed: {e}");
            return;
        }
    };
    for job in jobs {
        match queue
            .send(Job {
                job: job.to_string(),
            })
            .await
        {
            Ok(()) => console_log!("cron {cron}: queued {job} job"),
            Err(e) => console_error!("cron {cron}: queue send for {job} failed: {e}"),
        }
    }
}

/// The Worker side of `jobs::run_job`.
struct Runner<'a> {
    env: &'a Env,
    store: &'a D1Store,
    /// Every outgoing fetch goes through this, so `http.count()` is the
    /// invocation's fetch subrequests (D1 and R2 binding calls not included).
    http: CountingHttp<WorkerHttp>,
}

#[async_trait(?Send)]
impl JobRunner for Runner<'_> {
    /// A premarket or recap tick. Providers are built here, per message, so
    /// a bad LLM config fails only the jobs that need the LLM. The error
    /// names the variable and its value, never a secret.
    async fn content(&self, job: ContentJob) -> std::result::Result<TickOutcome, String> {
        let env = self.env;
        let md = Yahoo::new(self.http.clone());
        let news_key = secret_or_empty(env, "TAVILY_API_KEY");
        let ns = Tavily::new(self.http.clone(), news_key.clone());
        let provider = text(env, "LLM_PROVIDER").unwrap_or_else(|| "claude".into());
        let key_var =
            api_key_var(&provider).ok_or_else(|| unknown_provider(&provider).to_string())?;
        let llm_key = secret_or_empty(env, key_var);
        let llm = llm_from_env(
            &provider,
            self.http.clone(),
            llm_key.clone(),
            text(env, "LLM_MODEL"),
            text(env, "LLM_EFFORT"),
            |m| console_warn!("{m}"),
        )
        .map_err(|e| redact(&e.to_string(), &[&llm_key, &news_key]))?;
        let log = |m: &str| console_log!("{m}");
        // Read when the facts are built, so a retrying run's last_error says
        // how close the build came to the Free plan's 50.
        let extra = || vec![format!("subrequests={}", self.http.count())];
        let hooks = Hooks {
            log: &log,
            extra: &extra,
        };
        let at = now(env);
        let store = self.store;
        Ok(match job {
            ContentJob::Premarket => {
                tick_with_hooks(at, store, &md, &ns, llm.as_ref(), &hooks).await
            }
            ContentJob::Recap => {
                recap::tick_with_hooks(at, store, &md, &ns, llm.as_ref(), &hooks).await
            }
        })
    }

    async fn competitor(&self) -> String {
        let outcome = run_competitor(now(self.env), &self.http, &WorkerTimer, self.store).await;
        format!("{outcome:?}")
    }
}

#[event(queue)]
async fn queue(batch: MessageBatch<Job>, env: Env, _ctx: Context) -> Result<()> {
    let store = D1Store {
        db: env.d1("DB")?,
        facts: env.bucket("FACTS")?,
    };
    let runner = Runner {
        env: &env,
        store: &store,
        http: CountingHttp::new(WorkerHttp),
    };

    for msg in batch.messages()? {
        let job = msg.body().job.as_str();
        runner.http.reset();
        // A config error is alerted (once per job per ET day) and acked, not
        // retried: a retry would fail the same way, and the other messages
        // and the alert delivery below must still run.
        match run_job(now(&env), job, &store, &runner).await {
            Ok(line) => console_log!("{line}"),
            Err(line) => console_error!("{line}"),
        }
        // Free plan trial: fetch subrequests this job used (limit 50 per invocation).
        console_log!("subrequests={} job={job}", runner.http.count());
        msg.ack();
    }

    runner.http.reset();
    alerts::deliver(
        &store,
        &runner.http,
        text(&env, "ALERT_WEBHOOK_URL").as_deref(),
    )
    .await;
    console_log!("subrequests={} job=alerts", runner.http.count());
    Ok(())
}
