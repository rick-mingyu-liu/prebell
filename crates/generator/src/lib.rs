//! Generator Worker: cron -> queue -> `tick`, backed by D1 + R2.

mod alerts;
mod http;
mod store;

use chrono::{DateTime, Utc};
use marketsite_core::deepseek::llm_from_env;
use marketsite_core::run::tick;
use marketsite_core::tavily::Tavily;
use marketsite_core::yahoo::Yahoo;
use serde::{Deserialize, Serialize};
use worker::*;

use crate::http::WorkerHttp;
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
    let sent = match env.queue("JOBS") {
        Ok(q) => {
            q.send(Job {
                job: "premarket".into(),
            })
            .await
        }
        Err(e) => Err(e),
    };
    match sent {
        Ok(()) => console_log!("cron {}: queued premarket job", event.cron()),
        Err(e) => console_error!("cron {}: queue send failed: {e}", event.cron()),
    }
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
        let outcome = tick(now(&env), &store, &md, &ns, llm.as_ref()).await;
        console_log!("job {}: {:?}", msg.body().job, outcome);
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
