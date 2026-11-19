//! Deliver unsent `alerts` rows to a Discord-compatible webhook.

use async_trait::async_trait;
use prebell_core::providers::Http;
use worker::{console_error, console_warn};

pub use crate::store::AlertRec;

/// Fetch subrequests an invocation may use before alert delivery stops:
/// under the Free plan's 50, with room for redirects.
pub const SUBREQUEST_CEILING: usize = 45;
/// D1 queries per invocation on the Free plan (a batch is one).
pub const D1_LIMIT: usize = 50;
/// D1 queries one delivery makes, however many alerts it posts: the read and
/// one update marking them all sent.
pub const DELIVERY_D1: usize = 2;
/// Alerts posted per invocation at most; the rest wait for the next one
/// (every 5 minutes in the polling window). Also keeps the update's
/// `IN (...)` list under D1's 100 bound values.
pub const MAX_ALERTS: usize = 20;

/// Webhook posts left for this invocation after the jobs used `fetches`
/// fetch subrequests and `d1` D1 queries: at most `MAX_ALERTS`, and none
/// when delivery's own `DELIVERY_D1` queries would pass `D1_LIMIT`.
pub fn delivery_budget(fetches: usize, d1: usize) -> usize {
    if d1 + DELIVERY_D1 > D1_LIMIT {
        return 0;
    }
    SUBREQUEST_CEILING.saturating_sub(fetches).min(MAX_ALERTS)
}

/// The unsent alert rows (D1 in production).
#[async_trait(?Send)]
pub trait AlertQueue {
    /// At most `limit` unsent alerts, oldest first.
    async fn unsent_alerts(&self, limit: usize) -> Result<Vec<AlertRec>, String>;
    /// Marks all of `ids` sent in one query.
    async fn mark_alerts_sent(&self, ids: &[i64]) -> Result<(), String>;
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

/// Posts up to `max` unsent alerts (see `delivery_budget`), oldest first,
/// then marks the ones that got a 2xx response sent, in one query. The rest
/// stay unsent for the next invocation, as do all of them without a webhook
/// URL.
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
    let mut sent = Vec::new();
    for a in alerts.into_iter().take(max.min(MAX_ALERTS)) {
        let content = format!(
            "{}: {}: {}",
            a.level,
            a.run_key.as_deref().unwrap_or(""),
            a.message
        );
        let body = payload(&content);
        match http.post_json(url, &[], &body).await {
            Ok(r) if (200..300).contains(&r.status) => sent.push(a.id),
            Ok(r) => console_warn!("alert {} webhook returned {}", a.id, r.status),
            Err(e) => console_warn!("alert {} webhook failed: {e}", a.id),
        }
    }
    if sent.is_empty() {
        return;
    }
    // A failure here posts these alerts again next time: a duplicate, never a loss.
    if let Err(e) = store.mark_alerts_sent(&sent).await {
        console_error!("marking alerts {sent:?} sent failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use prebell_core::providers::{HttpResp, ProviderError};
    use std::cell::RefCell;

    /// `n` unsent alerts; records the limit asked for, the ids marked sent
    /// and each marking call.
    #[derive(Default)]
    struct Queue {
        n: i64,
        limits: RefCell<Vec<usize>>,
        sent: RefCell<Vec<i64>>,
        marks: RefCell<usize>,
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
        async fn mark_alerts_sent(&self, ids: &[i64]) -> Result<(), String> {
            *self.marks.borrow_mut() += 1;
            self.sent.borrow_mut().extend(ids);
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
                content_type: None,
            })
        }
    }

    fn queue(n: i64) -> Queue {
        Queue {
            n,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn delivery_fits_the_invocation_budget() {
        let q = queue(10);
        let hook = Hook(RefCell::new(0));
        // A poll used 40 fetches: 5 alerts go now, 5 wait for the next invocation.
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(40, 8)).await;
        assert_eq!(*hook.0.borrow(), 5);
        assert_eq!(*q.limits.borrow(), [5]);
        assert_eq!(*q.sent.borrow(), [1, 2, 3, 4, 5]);
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(0, 0)).await;
        assert_eq!(*q.sent.borrow(), (1..=10).collect::<Vec<_>>());
        // At or past 45 nothing is read or posted.
        let q = queue(3);
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(45, 0)).await;
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(60, 0)).await;
        assert!(q.limits.borrow().is_empty() && q.sent.borrow().is_empty());
        assert_eq!(delivery_budget(46, 0), 0);
    }

    #[tokio::test]
    async fn a_backlog_goes_out_twenty_at_a_time_with_one_update_each() {
        // 60 unsent alerts (a webhook outage) after a poll of 1 fetch and 8
        // D1 queries: one-update-per-alert would have been 8 + 1 + 44 = 53 D1
        // queries, over the Free plan's 50.
        let q = queue(60);
        let hook = Hook(RefCell::new(0));
        for round in 1..=3 {
            deliver(&q, &hook, Some("https://hook.test"), delivery_budget(1, 8)).await;
            assert_eq!(q.sent.borrow().len(), 20 * round);
            assert_eq!(*q.marks.borrow(), round, "one update per delivery");
        }
        assert_eq!(*q.limits.borrow(), [MAX_ALERTS; 3]);
        assert_eq!(*q.sent.borrow(), (1..=60).collect::<Vec<_>>());
        // With nothing posted, nothing is marked.
        deliver(&q, &hook, Some("https://hook.test"), delivery_budget(1, 8)).await;
        assert_eq!(*q.marks.borrow(), 3);
    }

    #[test]
    fn delivery_never_passes_the_d1_limit() {
        // The jobs' queries plus delivery's 2 stay at or under 50.
        assert_eq!(delivery_budget(0, D1_LIMIT - DELIVERY_D1), MAX_ALERTS);
        assert_eq!(delivery_budget(0, D1_LIMIT - DELIVERY_D1 + 1), 0);
        assert_eq!(delivery_budget(0, 60), 0);
        for d1 in 0..=60 {
            if delivery_budget(0, d1) > 0 {
                assert!(d1 + DELIVERY_D1 <= D1_LIMIT);
            }
        }
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
