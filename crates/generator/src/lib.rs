//! Generator Worker: cron -> queue -> `tick`, backed by D1 + R2.

mod alerts;
mod earnings_store;
mod http;
mod store;

use chrono::{DateTime, Utc};
use prebell_core::deepseek::llm_from_env;
use prebell_core::earnings::calendar::refresh_tick;
use prebell_core::earnings::in_window;
use prebell_core::earnings::run::{build_tick, poll_tick};
use prebell_core::edgar::{Edgar, Pause};
use prebell_core::providers::{LlmProvider, NewsSearch};
use prebell_core::run::tick;
use prebell_core::tavily::Tavily;
use prebell_core::yahoo::Yahoo;
use prebell_core::yahoo_calendar::YahooCalendar;
use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::WorkerHttp;
use crate::store::D1Store;

/// The every-minute cron (wrangler.toml) that drives earnings polling.
const EARNINGS_CRON: &str = "* * * * *";
const PLACEHOLDER_CONTACT: &str = "placeholder@example.com";

/// `Pause` on the Workers timer.
struct WorkerPause;

#[async_trait::async_trait(?Send)]
impl Pause for WorkerPause {
    async fn pause(&self, ms: u64) {
        Delay::from(std::time::Duration::from_millis(ms)).await;
    }
}

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
    let job = if event.cron() == EARNINGS_CRON {
        // Earnings jobs run only in the polling window (weekdays 06:00-22:00 ET).
        if !in_window(now(&env)) {
            return;
        }
        "earnings"
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

/// One earnings tick: poll a watchlist batch, refresh the calendar (once a
/// day), then build at most one queued filing.
async fn earnings(
    env: &Env,
    store: &D1Store,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
) -> Result<()> {
    let contact = text(env, "SEC_CONTACT").unwrap_or_default();
    if contact == PLACEHOLDER_CONTACT {
        console_warn!("SEC_CONTACT is the placeholder; set a real contact email before deploying");
    }
    let edgar = Edgar::new(WorkerHttp, WorkerPause, &contact)
        .map_err(|e| Error::RustError(e.to_string()))?;
    let t = now(env);
    match poll_tick(t, store, store, &edgar).await {
        Ok(o) => {
            for e in &o.errors {
                console_warn!("earnings poll: {e}");
            }
            console_log!("earnings poll: checked {}, new {:?}", o.checked, o.detected);
        }
        Err(e) => console_error!("earnings poll aborted: {e}"),
    }
    let cal = YahooCalendar::new(WorkerHttp);
    match refresh_tick(t, store, store, &cal, ns, llm).await {
        Ok(o) if o.ran => console_log!("calendar refresh: {o:?}"),
        Ok(_) => {}
        Err(e) => console_error!("calendar refresh aborted: {e}"),
    }
    match build_tick(now(env), store, store, &edgar, llm).await {
        Ok(o) => {
            for (key, outcome) in o {
                console_log!("earnings {key}: {outcome:?}");
            }
        }
        Err(e) => console_error!("earnings build aborted: {e}"),
    }
    Ok(())
}

#[event(queue)]
async fn queue(batch: MessageBatch<Job>, env: Env, _ctx: Context) -> Result<()> {
    let store = D1Store {
        db: env.d1("DB")?,
        facts: env.bucket("FACTS")?,
    };
    let md = Yahoo::new(WorkerHttp);
    let ns = Tavily::new(WorkerHttp, secret_or_empty(&env, "TAVILY_API_KEY"));
    let provider = text(&env, "LLM_PROVIDER").unwrap_or_else(|| "deepseek".into());
    let llm = llm_from_env(
        &provider,
        WorkerHttp,
        secret_or_empty(&env, "DEEPSEEK_API_KEY"),
    )
    .map_err(|e| Error::RustError(e.to_string()))?;

    for msg in batch.messages()? {
        match msg.body().job.as_str() {
            "earnings" => {
                if let Err(e) = earnings(&env, &store, &ns, llm.as_ref()).await {
                    console_error!("job earnings: {e}");
                }
            }
            job => {
                let outcome = tick(now(&env), &store, &md, &ns, llm.as_ref()).await;
                console_log!("job {job}: {:?}", outcome);
            }
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
