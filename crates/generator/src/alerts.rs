//! Deliver unsent `alerts` rows to a Discord-compatible webhook.

use marketsite_core::providers::Http;
use worker::{console_error, console_warn};

use crate::http::WorkerHttp;
use crate::store::D1Store;

/// Posts each unsent alert; marks it sent only on a 2xx response.
/// Without a webhook URL alerts stay unsent in D1.
pub async fn deliver(store: &D1Store, http: &WorkerHttp, webhook: Option<&str>) {
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
        let body = serde_json::json!({ "content": content });
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
