//! A `Sender` for Buttondown, one newsletter with a `lang:en` / `lang:zh`
//! tag per subscriber (docs.buttondown.com, checked 2026-11-06): `GET
//! /v1/tags` for the tag ids, `POST /v1/emails` with `status: "draft"` and
//! a `subscriber.tags contains <tag id>` filter, and in scheduled mode
//! `PATCH /v1/emails/{id}` with `status: "scheduled"` and `publish_date`.
//! The status is always sent: the API's own default is `about_to_send`,
//! which sends at once. `X-Buttondown-Live-Dangerously` is never sent. Only
//! status codes are logged, never a body or the key.
//!
//! Not wired into the Worker: tag filters need a paid Buttondown plan
//! ("Tagging & segmentation"), and the language split is undecided
//! (`docs/decisions.md`, "Digest language split").

use std::cell::RefCell;
use std::collections::HashMap;

use async_trait::async_trait;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};

use super::{Email, Sender, Sent};
use crate::numbers::Lang;
use crate::providers::{Http, HttpResp, ProviderError};

pub const API: &str = "https://api.buttondown.com/v1";
/// Pinned so a newer Buttondown default never changes a request; filters
/// by tag id since 2024-08-15.
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

/// Tag name -> id, from one page of up to 1,000 tags.
pub async fn tag_ids(
    http: &dyn Http,
    key: &str,
    log: &dyn Fn(&str),
) -> Result<HashMap<String, String>, Failure> {
    let auth = auth(key);
    let headers = [
        ("Authorization", auth.as_str()),
        ("X-API-Version", API_VERSION),
    ];
    let resp = checked(
        http.get(&format!("{API}/tags?page_size=1000"), &headers)
            .await,
        "tags",
        &[200],
        log,
    )?;
    let v: Value = serde_json::from_str(&resp.body).map_err(|_| Failure::Unreadable)?;
    let results = v["results"].as_array().ok_or(Failure::Unreadable)?;
    Ok(results
        .iter()
        .filter_map(|t| Some((t["name"].as_str()?.into(), t["id"].as_str()?.into())))
        .collect())
}

/// Subscribers who have the tag `tag_id`.
pub fn audience(tag_id: &str) -> Value {
    json!({
        "filters": [{"field": "subscriber.tags", "operator": "contains", "value": tag_id}],
        "groups": [],
        "predicate": "and",
    })
}

/// A draft email for the subscribers tagged `tag_id`.
pub fn draft(subject: &str, body: &str, tag_id: &str) -> Value {
    json!({
        "subject": subject,
        "body": format!("{MARKDOWN_MODE}\n{body}"),
        "status": "draft",
        "filters": audience(tag_id),
    })
}

/// A created email.
#[derive(Debug, Clone, PartialEq)]
pub struct Created {
    pub id: String,
    /// The audience as Buttondown stored it.
    pub filters: Value,
}

impl Created {
    /// Whether the stored audience still has the `tag_id` filter.
    pub fn targets(&self, tag_id: &str) -> bool {
        let want = &audience(tag_id)["filters"][0];
        self.filters["filters"]
            .as_array()
            .is_some_and(|fs| fs.iter().any(|f| f == want))
    }
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
        filters: v["filters"].clone(),
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

/// One Buttondown newsletter whose subscribers carry a `lang:` tag.
pub struct Buttondown<'a> {
    pub http: &'a dyn Http,
    pub key: &'a str,
    pub log: &'a dyn Fn(&str),
    /// The tag list, read once per message.
    tags: RefCell<Option<HashMap<String, String>>>,
}

impl<'a> Buttondown<'a> {
    pub fn new(http: &'a dyn Http, key: &'a str, log: &'a dyn Fn(&str)) -> Self {
        Self {
            http,
            key,
            log,
            tags: RefCell::new(None),
        }
    }

    async fn tag_id(&self, name: &str) -> Result<Option<String>, Failure> {
        if self.tags.borrow().is_none() {
            let tags = tag_ids(self.http, self.key, self.log).await?;
            *self.tags.borrow_mut() = Some(tags);
        }
        Ok(self
            .tags
            .borrow()
            .as_ref()
            .and_then(|t| t.get(name).cloned()))
    }
}

#[async_trait(?Send)]
impl Sender for Buttondown<'_> {
    /// A draft for the email's `lang:` tag, scheduled for `send_at` only
    /// once Buttondown's answer shows that filter. No such tag: skipped.
    async fn send(&self, email: &Email) -> Result<Sent, String> {
        let tag = match email.lang {
            Lang::En => "lang:en",
            Lang::Zh => "lang:zh",
        };
        let Some(tag_id) = self.tag_id(tag).await.map_err(|f| format!("tags {f}"))? else {
            return Ok(Sent::Skipped(format!("no {tag} tag")));
        };
        let body = draft(&email.subject, &email.body, &tag_id);
        let created = create(self.http, self.key, &email.idempotency_key, &body, self.log)
            .await
            .map_err(|f| f.to_string())?;
        if !created.targets(&tag_id) {
            return Ok(Sent::NeedsOwner {
                note: format!("draft {} without its audience", created.id),
                alert: format!(
                    "digest: Buttondown dropped the {tag} audience filter on draft {}; left as a draft, set its audience before sending",
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
