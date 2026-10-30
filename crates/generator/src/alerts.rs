//! Deliver unsent `alerts` rows to a Discord-compatible webhook.

use async_trait::async_trait;
use prebell_core::providers::Http;
use worker::{console_error, console_warn};

pub use crate::store::AlertRec;

/// Fetch subrequests an invocation may use before alert delivery stops:
/// under the Free plan's 50, with room for redirects.
pub const SUBREQUEST_CEILING: usize = 45;

/// Webhook posts left for this invocation after `used` fetch subrequests.
pub fn delivery_budget(used: usize) -> usize {
    SUBREQUEST_CEILING.saturating_sub(used)
}

/// The unsent alert rows (D1 in production).
#[async_trait(?Send)]
pub trait AlertQueue {
    /// At most `limit` unsent alerts, oldest first.
    async fn unsent_alerts(&self, limit: usize) -> Result<Vec<AlertRec>, String>;
    async fn mark_alert_sent(&self, id: i64) -> Result<(), String>;
}

/// Discord rejects `content` over 2000 characters; keep a margin.
const MAX_CONTENT_CHARS: usize = 1900;

/// At most `max` characters (not bytes), so multi-byte text never splits.
fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

/// Webhook body. `allowed_mentions` with an empty `parse` list means no
/// alert text (feed titles included) can ping @everyone, @here, a user or
/// a role.
fn payload(content: &str) -> serde_json::Value {
    serde_json::json!({
        "content": truncate(content, MAX_CONTENT_CHARS),
        "allowed_mentions": { "parse": [] },
    })
}

/// Posts up to `max` unsent alerts (see `delivery_budget`), oldest first;
/// marks each sent only on a 2xx response. The rest stay unsent for the
/// next invocation, as do all of them without a webhook URL.
pub async fn deliver(store: &dyn AlertQueue, http: &dyn Http, webhook: Option<&str>, max: usize) {
    let Some(url) = webhook.filter(|u| !u.is_empty()) else {
        return;
    };
    if max == 0 {
        return;
    }
    let alerts = match store.unsent_alerts(max).await {
        Ok(a) => a,
        Err(e) => return console_error!("reading alerts failed: {e}"),
    };
    for a in alerts {
        let content = format!(
            "{}: {}: {}",
            a.level,
            a.run_key.as_deref().unwrap_or(""),
            a.message
        );
        let body = payload(&content);
        match http.post_json(url, &[], &body).await {
            Ok(r) if (200..300).contains(&r.status) => {
                if let Err(e) = store.mark_alert_sent(a.id).await {
                    console_error!("marking alert {} sent failed: {e}", a.id);
                }
            }
            Ok(r) => console_warn!("alert {} webhook returned {}", a.id, r.status),
            Err(e) => console_warn!("alert {} webhook failed: {e}", a.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use prebell_core::providers::{HttpResp, ProviderError};
    use std::cell::RefCell;

    /// `n` unsent alerts; records the limit asked for and the ids marked sent.
    struct Queue {
        n: i64,
        limits: RefCell<Vec<usize>>,
        sent: RefCell<Vec<i64>>,
    }
    #[async_trait(?Send)]
    impl AlertQueue for Queue {
        async fn unsent_alerts(&self, limit: usize) -> Result<Vec<AlertRec>, String> {
            self.limits.borrow_mut().push(limit);
            Ok((1..=self.n)
                .filter(|id| !self.sent.borrow().contains(id))
                .take(limit)
                .map(|id| AlertRec {
                    id,
                    run_key: None,
                    level: "warn".into(),
                    message: format!("alert {id}"),
                })
                .collect())
        }
        async fn mark_alert_sent(&self, id: i64) -> Result<(), String> {
            self.sent.borrow_mut().push(id);
            Ok(())
        }
    }
    struct Hook(RefCell<usize>);
    #[async_trait(?Send)]
    impl Http for Hook {
        async fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            unreachable!()
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            *self.0.borrow_mut() += 1;
            Ok(HttpResp {
                status: 204,
                body: String::new(),
                set_cookie: None,
            })
        }
    }

    #[tokio::test]
    async fn delivery_fits_the_invocation_budget() {
        let q = Queue {
            n: 10,
            limits: RefCell::new(vec![]),
            sent: RefCell::new(vec![]),
        };
        let hook = Hook(RefCell::new(0));
        // A poll used 40 fetches: 5 alerts go now, 5 wait for the next invocation.
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(40)).await;
        assert_eq!(*hook.0.borrow(), 5);
        assert_eq!(*q.limits.borrow(), [5]);
        assert_eq!(*q.sent.borrow(), [1, 2, 3, 4, 5]);
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(0)).await;
        assert_eq!(*q.sent.borrow(), (1..=10).collect::<Vec<_>>());
        // At or past 45 nothing is read or posted.
        let q = Queue {
            n: 3,
            limits: RefCell::new(vec![]),
            sent: RefCell::new(vec![]),
        };
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(45)).await;
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(60)).await;
        assert!(q.limits.borrow().is_empty() && q.sent.borrow().is_empty());
        assert_eq!(delivery_budget(46), 0);
    }

    #[test]
    fn payload_never_pings() {
        let body = payload("error: x: @everyone @here <@123> <@&456>");
        assert_eq!(body["allowed_mentions"], serde_json::json!({ "parse": [] }));
        assert_eq!(body["content"], "error: x: @everyone @here <@123> <@&456>");
        let long = payload(&"x".repeat(5000));
        assert_eq!(long["content"].as_str().unwrap().chars().count(), 1900);
    }

    #[test]
    fn truncates_by_chars() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("盘前简报失败", 2), "盘前");
        assert_eq!(truncate(&"x".repeat(5000), 1900).chars().count(), 1900);
    }
}
