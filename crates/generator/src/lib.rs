//! Generator Worker: cron -> queue -> `tick`, backed by D1 + R2.

mod alerts;
mod competitor;
mod http;
mod store;

use chrono::{DateTime, Utc};
use prebell_core::competitor::run_competitor;
use prebell_core::llm::{api_key_var, llm_from_env, unknown_provider};
use prebell_core::run::tick_with_log;
use prebell_core::tavily::Tavily;
use prebell_core::yahoo::Yahoo;
use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::{WorkerHttp, WorkerTimer};
use crate::store::D1Store;

/// Daily competitor watch (wrangler.toml); `core::competitor` picks the
/// firing that is 07:30 ET. Every other cron is the premarket job. Two
/// crons rather than a comma list: the schedule parser can't read comma lists.
const COMPETITOR_CRONS: [&str; 2] = ["31 11 * * *", "31 12 * * *"];

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
    let job = if COMPETITOR_CRONS.contains(&event.cron().as_str()) {
        "competitor"
    } else {
        "premarket"
    };
    let sent = match env.queue("JOBS") {
        Ok(q) => q.send(Job { job: job.into() }).await,
        Err(e) => Err(e),
    };
    match sent {
        Ok(()) => console_log!("cron {}: queued {job} job", event.cron()),
        Err(e) => console_error!("cron {}: queue send failed: {e}", event.cron()),
    }
}

async fn premarket(env: &Env, store: &D1Store, job: &str) -> Result<()> {
    let md = Yahoo::new(WorkerHttp);
    let ns = Tavily::new(WorkerHttp, secret_or_empty(env, "TAVILY_API_KEY"));
    let provider = text(env, "LLM_PROVIDER").unwrap_or_else(|| "claude".into());
    let key_var = api_key_var(&provider)
        .ok_or_else(|| Error::RustError(unknown_provider(&provider).to_string()))?;
    let llm = llm_from_env(
        &provider,
        WorkerHttp,
        secret_or_empty(env, key_var),
        text(env, "LLM_MODEL"),
        text(env, "LLM_EFFORT"),
        |m| console_warn!("{m}"),
    )
    .map_err(|e| Error::RustError(e.to_string()))?;
    let outcome = tick_with_log(now(env), store, &md, &ns, llm.as_ref(), &|m| {
        console_log!("{m}")
    })
    .await;
    console_log!("job {job}: {outcome:?}");
    Ok(())
}

#[event(queue)]
async fn queue(batch: MessageBatch<Job>, env: Env, _ctx: Context) -> Result<()> {
    let store = D1Store {
        db: env.d1("DB")?,
        facts: env.bucket("FACTS")?,
    };

    for msg in batch.messages()? {
        let job = msg.body().job.as_str();
        if job == "competitor" {
            let outcome = run_competitor(now(&env), &WorkerHttp, &WorkerTimer, &store).await;
            console_log!("job {job}: {outcome:?}");
        } else {
            premarket(&env, &store, job).await?;
        }
        msg.ack();
    }

    alerts::deliver(
        &store,
        &WorkerHttp,
        text(&env, "ALERT_WEBHOOK_URL").as_deref(),
    )
    .await;
    Ok(())
}
