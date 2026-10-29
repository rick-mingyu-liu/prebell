//! Deliver unsent `alerts` rows to a Discord-compatible webhook.

use prebell_core::providers::Http;
use worker::{console_error, console_warn};

use crate::store::D1Store;

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

/// Posts each unsent alert; marks it sent only on a 2xx response.
/// Without a webhook URL alerts stay unsent in D1.
pub async fn deliver(store: &D1Store, http: &dyn Http, webhook: Option<&str>) {
    let Some(url) = webhook.filter(|u| !u.is_empty()) else {
        return;
    };
    let alerts = match store.unsent_alerts().await {
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
    use super::{payload, truncate};

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
