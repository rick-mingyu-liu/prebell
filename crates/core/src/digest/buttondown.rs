//! A `Sender` for Buttondown, one newsletter, one email to every subscriber
//! (docs.buttondown.com and api.buttondown.com/v1/openapi.json, checked
//! 2026-11-10): `POST /v1/emails` with `status: "draft"` and no `filters`
//! (tags, which audience filters select by, need a paid plan; with no filter
//! the email goes to every subscriber), and in scheduled mode `PATCH /v1/emails/{id}` with
//! `status: "scheduled"` and `publish_date`. The status is always sent: the
//! API's own default is `about_to_send`, which sends at once. A draft is
//! scheduled only when Buttondown's answer says it saved a draft; a failed
//! schedule call is checked with `GET /v1/emails/{id}` before it counts as
//! failed.
//! `X-Buttondown-Live-Dangerously` is never sent. Only status codes are
//! logged, never a body or the key. The sender's display name is the
//! newsletter's "from name" setting, not part of an email
//! (`docs/runbook.md`, "Weekly digest").

use async_trait::async_trait;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use super::{Email, Saved, Sender};
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

/// Whether email `id` is already scheduled for `at`: `GET /v1/emails/{id}`
/// answers `status: "scheduled"` and that `publish_date`. Anything else,
/// including a failed read, is `false`.
pub async fn scheduled_for(
    http: &dyn Http,
    key: &str,
    id: &str,
    at: DateTime<Utc>,
    log: &dyn Fn(&str),
) -> bool {
    let auth = auth(key);
    let headers = [
        ("Authorization", auth.as_str()),
        ("X-API-Version", API_VERSION),
    ];
    let Ok(resp) = checked(
        http.get(&format!("{API}/emails/{id}"), &headers).await,
        "check",
        &[200],
        log,
    ) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&resp.body) else {
        return false;
    };
    v["status"].as_str() == Some("scheduled")
        && v["publish_date"]
            .as_str()
            .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
            .is_some_and(|d| d.with_timezone(&Utc) == at)
}

#[async_trait(?Send)]
impl Sender for Buttondown<'_> {
    /// A draft for every subscriber; `NeedsOwner` unless Buttondown's
    /// answer shows it saved a draft.
    async fn create(&self, email: &Email) -> Result<Saved, String> {
        let body = draft(&email.subject, &email.body);
        let created = create(self.http, self.key, &email.idempotency_key, &body, self.log)
            .await
            .map_err(|f| f.to_string())?;
        if created.status.as_deref() != Some("draft") {
            let status = created.status.as_deref().unwrap_or("no status");
            return Ok(Saved::NeedsOwner {
                note: format!("email {} saved as {status}, not a draft", created.id),
                alert: format!(
                    "digest: Buttondown answered with {status}, not draft, for email {}; check it in Buttondown now",
                    created.id
                ),
            });
        }
        Ok(Saved::Draft(created.id))
    }

    /// The PATCH; when it fails, a read of the email decides whether an
    /// earlier PATCH already scheduled it for `at` (its answer lost, or a
    /// repeat Buttondown refuses).
    async fn schedule(&self, id: &str, at: DateTime<Utc>) -> Result<(), String> {
        match schedule(self.http, self.key, id, at, self.log).await {
            Ok(()) => Ok(()),
            Err(f) => {
                if scheduled_for(self.http, self.key, id, at, self.log).await {
                    Ok(())
                } else {
                    Err(f.to_string())
                }
            }
        }
    }
}
