use crate::llm::strip_fence;
use crate::providers::{Http, LlmProvider, ProviderError};
use async_trait::async_trait;
use serde_json::{json, Value};

const URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// Thinking counts toward `max_tokens`; this leaves room for it and a long ZH draft
/// while keeping a non-streaming request well inside HTTP timeouts.
const MAX_TOKENS: u32 = 16000;

pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";

pub struct Claude<H> {
    pub http: H,
    pub api_key: String,
    pub model: String,
}

impl<H> Claude<H> {
    pub fn new(http: H, api_key: String, model: String) -> Self {
        Self {
            http,
            api_key,
            model,
        }
    }
}

#[async_trait(?Send)]
impl<H: Http> LlmProvider for Claude<H> {
    async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
        // No `temperature`/`thinking`/betas: the body stays valid for any configured model.
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": [{"role": "user", "content": user}],
        });
        let resp = self
            .http
            .post_json(
                URL,
                &[
                    ("x-api-key", self.api_key.as_str()),
                    ("anthropic-version", API_VERSION),
                    ("content-type", "application/json"),
                ],
                &body,
            )
            .await?;
        if resp.status >= 400 {
            return Err(ProviderError::Http(resp.status));
        }
        let v: Value =
            serde_json::from_str(&resp.body).map_err(|e| ProviderError::Parse(e.to_string()))?;
        match v.get("stop_reason").and_then(Value::as_str) {
            Some("max_tokens") => return Err(ProviderError::Parse("truncated".into())),
            Some("refusal") => {
                let category = v
                    .pointer("/stop_details/category")
                    .and_then(Value::as_str)
                    .unwrap_or("unspecified");
                return Err(ProviderError::Parse(format!("refusal: {category}")));
            }
            // No tools are sent, so anything else (`tool_use`, `pause_turn`, ...) is an
            // unfinished turn that must not reach the drafter.
            Some("end_turn") | Some("stop_sequence") | None => {}
            Some(other) => return Err(ProviderError::Parse(format!("stop_reason: {other}"))),
        }
        // Responses may start with `thinking` blocks: read by type, not position.
        let text = v
            .get("content")
            .and_then(Value::as_array)
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            })
            .and_then(|b| b.get("text"))
            .and_then(Value::as_str)
            .ok_or(ProviderError::Empty)?;
        let parsed: Value = serde_json::from_str(strip_fence(text))
            .map_err(|e| ProviderError::Parse(e.to_string()))?;
        if !parsed.is_object() {
            return Err(ProviderError::Parse("not a JSON object".into()));
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::HttpResp;
    use std::cell::RefCell;
    use std::rc::Rc;

    const OK: &str = include_str!("../tests/fixtures/claude_ok.json");
    const THINKING_OK: &str = include_str!("../tests/fixtures/claude_thinking_ok.json");
    const BAD: &str = include_str!("../tests/fixtures/claude_bad_json.json");
    const MAX_TOKENS_HIT: &str = include_str!("../tests/fixtures/claude_max_tokens.json");
    const REFUSAL: &str = include_str!("../tests/fixtures/claude_refusal.json");

    type Seen = Vec<(String, Vec<(String, String)>, Value)>;
    #[derive(Clone)]
    struct FakeHttp {
        status: u16,
        body: String,
        seen: Rc<RefCell<Seen>>,
    }
    #[async_trait(?Send)]
    impl Http for FakeHttp {
        async fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            unreachable!()
        }
        async fn post_json(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            body: &Value,
        ) -> Result<HttpResp, ProviderError> {
            self.seen.borrow_mut().push((
                url.into(),
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body.clone(),
            ));
            Ok(HttpResp {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }
    fn fake(status: u16, body: &str) -> FakeHttp {
        FakeHttp {
            status,
            body: body.into(),
            seen: Rc::new(RefCell::new(vec![])),
        }
    }
    fn cl(h: FakeHttp) -> Claude<FakeHttp> {
        Claude::new(h, "ak".into(), DEFAULT_MODEL.into())
    }
    fn wrap(text: &str, stop: &str) -> String {
        json!({
            "id": "msg_01Fixture",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-5-5",
            "content": [{"type": "text", "text": text}],
            "stop_reason": stop,
            "stop_sequence": null,
            "usage": {"input_tokens": 1, "output_tokens": 1},
        })
        .to_string()
    }

    #[tokio::test]
    async fn ok_fixture_returns_object() {
        let v = cl(fake(200, OK)).complete_json("s", "u").await.unwrap();
        assert_eq!(v["headline"], "Futures up");
    }

    #[tokio::test]
    async fn thinking_block_before_text_is_skipped() {
        let v = cl(fake(200, THINKING_OK))
            .complete_json("s", "u")
            .await
            .unwrap();
        assert_eq!(v["n"], 2);
    }

    #[tokio::test]
    async fn non_json_text_is_parse_error() {
        let r = cl(fake(200, BAD)).complete_json("s", "u").await;
        assert!(matches!(r, Err(ProviderError::Parse(_))));
    }

    #[tokio::test]
    async fn non_object_json_is_parse_error() {
        let r = cl(fake(200, &wrap("[1,2]", "end_turn")))
            .complete_json("s", "u")
            .await;
        assert!(matches!(r, Err(ProviderError::Parse(_))));
    }

    #[tokio::test]
    async fn fenced_json_accepted() {
        let b = wrap("```json\n{\"a\":1}\n```", "end_turn");
        assert_eq!(
            cl(fake(200, &b)).complete_json("s", "u").await.unwrap()["a"],
            1
        );
        let b = wrap("```\n{\"a\":2}\n```", "end_turn");
        assert_eq!(
            cl(fake(200, &b)).complete_json("s", "u").await.unwrap()["a"],
            2
        );
    }

    #[tokio::test]
    async fn max_tokens_is_truncated_parse_error() {
        let r = cl(fake(200, MAX_TOKENS_HIT)).complete_json("s", "u").await;
        assert_eq!(r, Err(ProviderError::Parse("truncated".into())));
    }

    #[tokio::test]
    async fn refusal_is_parse_error() {
        let r = cl(fake(200, REFUSAL)).complete_json("s", "u").await;
        assert_eq!(r, Err(ProviderError::Parse("refusal: cyber".into())));
    }

    #[tokio::test]
    async fn refusal_without_category_and_other_stop_reasons() {
        let b = json!({"content": [], "stop_reason": "refusal", "stop_details": null}).to_string();
        let r = cl(fake(200, &b)).complete_json("s", "u").await;
        assert_eq!(r, Err(ProviderError::Parse("refusal: unspecified".into())));
        let r = cl(fake(200, &wrap("{\"a\":1}", "pause_turn")))
            .complete_json("s", "u")
            .await;
        assert_eq!(
            r,
            Err(ProviderError::Parse("stop_reason: pause_turn".into()))
        );
        let r = cl(fake(200, &wrap("{\"a\":1}", "stop_sequence")))
            .complete_json("s", "u")
            .await;
        assert_eq!(r.unwrap()["a"], 1);
    }

    #[tokio::test]
    async fn no_text_block_and_http_error() {
        let r = cl(fake(200, "{\"content\":[],\"stop_reason\":\"end_turn\"}"))
            .complete_json("s", "u")
            .await;
        assert_eq!(r, Err(ProviderError::Empty));
        let err = "{\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}";
        let r = cl(fake(529, err)).complete_json("s", "u").await;
        assert_eq!(r, Err(ProviderError::Http(529)));
        let r = cl(fake(401, "{}")).complete_json("s", "u").await;
        assert_eq!(r, Err(ProviderError::Http(401)));
    }

    #[tokio::test]
    async fn non_json_body_is_parse_error() {
        let r = cl(fake(200, "<html>bad gateway</html>"))
            .complete_json("s", "u")
            .await;
        assert!(matches!(r, Err(ProviderError::Parse(_))));
    }

    #[tokio::test]
    async fn request_shape_and_headers() {
        let h = fake(200, OK);
        cl(h.clone()).complete_json("sys", "usr").await.unwrap();
        let seen = h.seen.borrow();
        let (url, headers, body) = &seen[0];
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert!(headers.contains(&("x-api-key".into(), "ak".into())));
        assert!(headers.contains(&("anthropic-version".into(), "2023-06-01".into())));
        assert!(headers.contains(&("content-type".into(), "application/json".into())));
        assert!(!headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization")));
        assert_eq!(body["model"], "claude-sonnet-5-5");
        assert_eq!(body["max_tokens"], 16000);
        assert_eq!(body["system"], "sys");
        assert_eq!(
            body["messages"],
            json!([{"role": "user", "content": "usr"}])
        );
        let mut keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["max_tokens", "messages", "model", "system"]);
    }
}
