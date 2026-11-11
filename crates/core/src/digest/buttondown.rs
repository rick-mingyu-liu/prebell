//! A `Sender` for Buttondown, one newsletter, one email to every subscriber
//! (docs.buttondown.com and api.buttondown.com/v1/openapi.json, checked
//! 2026-11-10): `POST /v1/emails` with `status: "draft"` and no `filters`
//! (tags, which audience filters select by, need a paid plan; with no filter
//! the email goes to every subscriber), and in scheduled mode `PATCH /v1/emails/{id}` with
//! `status: "scheduled"` and `publish_date`. The status is always sent: the
//! API's own default is `about_to_send`, which sends at once. A draft is
//! scheduled only when Buttondown's answer says it saved a draft.
//! `X-Buttondown-Live-Dangerously` is never sent. Only status codes are
//! logged, never a body or the key. The sender's display name is the
//! newsletter's "from name" setting, not part of an email
//! (`docs/runbook.md`, "Weekly digest").

use async_trait::async_trait;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use super::{Email, Sender, Sent};
use crate::providers::{Http, HttpResp, ProviderError};

pub const API: &str = "https://api.buttondown.com/v1";
/// Pinned so a newer Buttondown default never changes a request (the
/// current version on 2026-11-10, docs.buttondown.com/api-versioning).
pub const API_VERSION: &str = "2026-04-01";
/// Markdown, stated rather than detected.
pub const MARKDOWN_MODE: &str = "<!-- buttondown-editor-mode: plaintext -->";

/// Why a call failed, for the log line and the alert.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    Status(u16),
    Network,
    /// A success status with a body we could not read.
    Unreadable,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Status(s) => write!(f, "http {s}"),
            Failure::Network => write!(f, "network error"),
            Failure::Unreadable => write!(f, "unreadable response"),
        }
    }
}

fn auth(key: &str) -> String {
    format!("Token {key}")
}

/// The response, or `Failure` for anything but `ok`, logging the status
/// (or `network error`) as `digest: buttondown <what> status <code>`.
fn checked(
    r: Result<HttpResp, ProviderError>,
    what: &str,
    ok: &[u16],
    log: &dyn Fn(&str),
) -> Result<HttpResp, Failure> {
    match r {
        Ok(resp) => {
            log(&format!("digest: buttondown {what} status {}", resp.status));
            if ok.contains(&resp.status) {
                Ok(resp)
            } else {
                Err(Failure::Status(resp.status))
            }
        }
        Err(_) => {
            log(&format!("digest: buttondown {what} network error"));
            Err(Failure::Network)
        }
    }
}

/// A draft email for every subscriber: no `filters`, so Buttondown's
/// default audience (all subscribers) applies.
pub fn draft(subject: &str, body: &str) -> Value {
    json!({
        "subject": subject,
        "body": format!("{MARKDOWN_MODE}\n{body}"),
        "status": "draft",
    })
}

/// A created email.
#[derive(Debug, Clone, PartialEq)]
pub struct Created {
    pub id: String,
    /// The status as Buttondown stored it; `None` when the answer has none.
    pub status: Option<String>,
}

/// `POST /v1/emails` with `email`, under `idempotency` (a retry with the same
/// key gets the first answer back, not a second email).
pub async fn create(
    http: &dyn Http,
    key: &str,
    idempotency: &str,
    email: &Value,
    log: &dyn Fn(&str),
) -> Result<Created, Failure> {
    let auth = auth(key);
    let headers = [
        ("Authorization", auth.as_str()),
        ("X-API-Version", API_VERSION),
        ("X-Idempotency-Key", idempotency),
    ];
    let resp = checked(
        http.post_json(&format!("{API}/emails"), &headers, email)
            .await,
        "create",
        &[200, 201],
        log,
    )?;
    let v: Value = serde_json::from_str(&resp.body).map_err(|_| Failure::Unreadable)?;
    let id = v["id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        })
        .ok_or(Failure::Unreadable)?;
    Ok(Created {
        id: id.into(),
        status: v["status"].as_str().map(String::from),
    })
}

/// `PATCH /v1/emails/{id}`: scheduled to send at `at`.
pub async fn schedule(
    http: &dyn Http,
    key: &str,
    id: &str,
    at: DateTime<Utc>,
    log: &dyn Fn(&str),
) -> Result<(), Failure> {
    let auth = auth(key);
    let headers = [
        ("Authorization", auth.as_str()),
        ("X-API-Version", API_VERSION),
    ];
    let body = json!({
        "status": "scheduled",
        "publish_date": at.to_rfc3339_opts(SecondsFormat::Secs, true),
    });
    checked(
        http.patch_json(&format!("{API}/emails/{id}"), &headers, &body)
            .await,
        "schedule",
        &[200],
        log,
    )
    .map(|_| ())
}

/// One Buttondown newsletter.
pub struct Buttondown<'a> {
    pub http: &'a dyn Http,
    pub key: &'a str,
    pub log: &'a dyn Fn(&str),
}

impl<'a> Buttondown<'a> {
    pub fn new(http: &'a dyn Http, key: &'a str, log: &'a dyn Fn(&str)) -> Self {
        Self { http, key, log }
    }
}

#[async_trait(?Send)]
impl Sender for Buttondown<'_> {
    /// A draft for every subscriber, scheduled for `send_at` only once
    /// Buttondown's answer shows it saved a draft.
    async fn send(&self, email: &Email) -> Result<Sent, String> {
        let body = draft(&email.subject, &email.body);
        let created = create(self.http, self.key, &email.idempotency_key, &body, self.log)
            .await
            .map_err(|f| f.to_string())?;
        if created.status.as_deref() != Some("draft") {
            let status = created.status.as_deref().unwrap_or("no status");
            return Ok(Sent::NeedsOwner {
                note: format!("email {} saved as {status}, not a draft", created.id),
                alert: format!(
                    "digest: Buttondown answered with {status}, not draft, for email {}; check it in Buttondown now",
                    created.id
                ),
            });
        }
        match email.send_at {
            None => Ok(Sent::Done(format!("draft {}", created.id))),
            Some(t) => {
                schedule(self.http, self.key, &created.id, t, self.log)
                    .await
                    .map_err(|f| f.to_string())?;
                Ok(Sent::Done(format!("scheduled {}", created.id)))
            }
        }
    }
}
