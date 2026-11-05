//! The cover scene maker: Flux Schnell on Workers AI, over the Cloudflare
//! REST API.

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use prebell_core::cover::CoverMaker;
use prebell_core::jobs::redact;
use prebell_core::providers::Http;
use serde_json::{json, Value};

/// The longest API message passed on to the log.
const MAX_MESSAGE: usize = 200;

/// Flux Schnell through the Workers AI REST API. One request per scene, no
/// retry: the caller falls back to a plain cover on error.
pub struct WorkersAiCover<'a, H: Http> {
    http: &'a H,
    account_id: String,
    token: String,
}

impl<'a, H: Http> WorkersAiCover<'a, H> {
    pub fn new(http: &'a H, account_id: String, token: String) -> Self {
        Self {
            http,
            account_id,
            token,
        }
    }
}

#[async_trait(?Send)]
impl<H: Http> CoverMaker for WorkersAiCover<'_, H> {
    async fn scene(&self, prompt: &str) -> Result<Vec<u8>, String> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/ai/run/@cf/black-forest-labs/flux-1-schnell",
            self.account_id
        );
        let auth = format!("Bearer {}", self.token);
        let body = json!({ "prompt": prompt, "steps": 4 });
        let resp = self
            .http
            .post_json(&url, &[("Authorization", auth.as_str())], &body)
            .await
            .map_err(|e| format!("workers ai request failed: {e}"))?;
        let out = if (200..300).contains(&resp.status) {
            parse_image(&resp.body)
        } else {
            Err(api_error(&resp.body).unwrap_or_else(|| format!("workers ai http {}", resp.status)))
        };
        out.map_err(|e| redact(&e, &[&self.token]))
    }
}

/// `errors[0].message` of a Workers AI response body, cut to a log-sized line.
fn api_error(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let msg = v.pointer("/errors/0/message")?.as_str()?.trim();
    (!msg.is_empty()).then(|| {
        let cut: String = msg.chars().take(MAX_MESSAGE).collect();
        format!("workers ai: {cut}")
    })
}

/// The image bytes of a success body
/// (`{"success":true,"result":{"image":"<base64>"}}`). Anything else is an
/// error carrying the API's own message when there is one.
pub fn parse_image(body: &str) -> Result<Vec<u8>, String> {
    let v: Value =
        serde_json::from_str(body).map_err(|_| "workers ai: response is not JSON".to_string())?;
    let image = if v.get("success").and_then(Value::as_bool) == Some(true) {
        v.pointer("/result/image").and_then(Value::as_str)
    } else {
        None
    };
    let Some(image) = image else {
        return Err(api_error(body).unwrap_or_else(|| "workers ai: response has no image".into()));
    };
    let bytes = STANDARD
        .decode(image)
        .map_err(|_| "workers ai: image is not valid base64".to_string())?;
    if bytes.is_empty() {
        return Err("workers ai: image is empty".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prebell_core::providers::{HttpResp, ProviderError};
    use std::cell::RefCell;

    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    #[test]
    fn parse_image_decodes_base64_and_reports_api_errors() {
        let ok = r#"{"success":true,"result":{"image":"iVBORw0KGgo="},"errors":[],"messages":[]}"#;
        assert_eq!(parse_image(ok).unwrap(), PNG_MAGIC);
        let quota = r#"{"success":false,"errors":[{"code":3036,"message":"quota"}]}"#;
        assert!(parse_image(quota).unwrap_err().contains("quota"));
    }

    #[test]
    fn parse_image_rejects_malformed_bodies_without_panicking() {
        for body in [
            "",
            "<html>502</html>",
            r#"{"success":true}"#,
            r#"{"success":true,"result":{"image":"!!!"}}"#,
            r#"{"success":true,"result":{"image":""}}"#,
            r#"{"success":false,"errors":[]}"#,
        ] {
            let e = parse_image(body).unwrap_err();
            assert!(e.starts_with("workers ai"), "{body}: {e}");
            assert!(!e.contains("not configured"), "{e}");
        }
    }

    struct FakeHttp {
        resp: Result<HttpResp, ProviderError>,
        seen: RefCell<Vec<(String, String, serde_json::Value)>>,
    }

    impl FakeHttp {
        fn new(status: u16, body: &str) -> Self {
            Self {
                resp: Ok(HttpResp {
                    status,
                    body: body.into(),
                    set_cookie: None,
                }),
                seen: RefCell::new(vec![]),
            }
        }
    }

    #[async_trait(?Send)]
    impl Http for FakeHttp {
        async fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            unreachable!("the cover maker only posts")
        }
        async fn post_json(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            body: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            let auth = headers
                .iter()
                .find(|(k, _)| *k == "Authorization")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            self.seen
                .borrow_mut()
                .push((url.into(), auth, body.clone()));
            self.resp.clone()
        }
    }

    fn maker(http: &FakeHttp) -> WorkersAiCover<'_, FakeHttp> {
        WorkersAiCover::new(http, "acct1".into(), "sekret-token".into())
    }

    #[tokio::test]
    async fn scene_posts_the_prompt_and_returns_the_bytes() {
        let http = FakeHttp::new(200, r#"{"success":true,"result":{"image":"iVBORw0KGgo="}}"#);
        let bytes = maker(&http).scene("a harbour at dawn").await.unwrap();
        assert_eq!(bytes, PNG_MAGIC);
        let seen = http.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0,
            "https://api.cloudflare.com/client/v4/accounts/acct1/ai/run/@cf/black-forest-labs/flux-1-schnell"
        );
        assert_eq!(seen[0].1, "Bearer sekret-token");
        assert_eq!(
            seen[0].2,
            json!({"prompt": "a harbour at dawn", "steps": 4})
        );
    }

    #[tokio::test]
    async fn scene_errors_name_the_api_message_or_status_and_never_the_token() {
        let cases = [
            (
                429,
                r#"{"success":false,"errors":[{"message":"quota; token sekret-token"}]}"#,
                "quota",
            ),
            (502, "<html>bad gateway</html>", "workers ai http 502"),
            (200, r#"{"success":true,"result":{}}"#, "no image"),
        ];
        for (status, body, want) in cases {
            let http = FakeHttp::new(status, body);
            let e = maker(&http).scene("p").await.unwrap_err();
            assert!(e.contains(want), "{e}");
            assert!(!e.contains("sekret-token"), "{e}");
            assert!(!e.contains("not configured"), "{e}");
        }
        let http = FakeHttp {
            resp: Err(ProviderError::Network("connection reset".into())),
            seen: RefCell::new(vec![]),
        };
        let e = maker(&http).scene("p").await.unwrap_err();
        assert!(e.contains("connection reset"), "{e}");
    }
}
