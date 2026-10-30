use crate::llm::strip_fence;
use crate::providers::{Http, LlmProvider, ProviderError};
use async_trait::async_trait;
use serde_json::{json, Value};

const URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
/// Thinking counts toward `max_tokens`; this leaves room for it and a long ZH draft
/// while keeping a non-streaming request well inside HTTP timeouts.
const MAX_TOKENS: u32 = 16000;

pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
/// `LLM_EFFORT` default. Extraction and drafting are routine work; lower effort keeps a
/// build well inside the stale-build window.
pub const DEFAULT_EFFORT: &str = "low";

/// Parses `LLM_EFFORT` into the `output_config.effort` value to send.
///
/// `None`/empty gives [`DEFAULT_EFFORT`]; `"none"` gives `None` (omit the field, for models
/// without effort support); anything but low / medium / high is a config error.
pub fn parse_effort(raw: Option<&str>) -> Result<Option<&'static str>, ProviderError> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(Some(DEFAULT_EFFORT)),
        Some("none") => Ok(None),
        Some("low") => Ok(Some("low")),
        Some("medium") => Ok(Some("medium")),
        Some("high") => Ok(Some("high")),
        Some(other) => Err(ProviderError::Parse(format!(
            "unknown LLM_EFFORT: {other} (use low, medium, high or none)"
        ))),
    }
}

fn no_warn(_: &str) {}

pub struct Claude<H> {
    pub http: H,
    pub api_key: String,
    pub model: String,
    /// `output_config.effort` to send; `None` omits `output_config`.
    pub effort: Option<&'static str>,
    /// Sink for HTTP error diagnostics (core has no logger; the generator passes `console_warn!`).
    pub warn: fn(&str),
}

impl<H> Claude<H> {
    pub fn new(http: H, api_key: String, model: String) -> Self {
        Self {
            http,
            api_key,
            model,
            effort: Some(DEFAULT_EFFORT),
            warn: no_warn,
        }
    }

    pub fn with_effort(mut self, effort: Option<&'static str>) -> Self {
        self.effort = effort;
        self
    }

    pub fn with_warn(mut self, warn: fn(&str)) -> Self {
        self.warn = warn;
        self
    }
}

/// Keeps only id-like characters and caps the length, so a hostile or malformed error body
/// cannot inject arbitrary text (or a key echoed back) into the logs.
fn log_token(v: Option<&str>) -> String {
    let t: String = v
        .unwrap_or("unknown")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(64)
        .collect();
    if t.is_empty() {
        "unknown".into()
    } else {
        t
    }
}

/// `error.type` and `request_id` from an Anthropic error body; never the message or the rest.
fn error_summary(status: u16, body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let ty = v.pointer("/error/type").and_then(Value::as_str);
    let id = v.get("request_id").and_then(Value::as_str);
    format!(
        "claude http {status}: error.type={} request_id={}",
        log_token(ty),
        log_token(id)
    )
}

#[async_trait(?Send)]
impl<H: Http> LlmProvider for Claude<H> {
    async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
        // No `temperature` (rejected on API version 2026-10-21), `thinking` or betas, so the
        // body stays valid for any configured model. `output_config.effort` is the one
        // model-dependent field: `LLM_EFFORT=none` omits it.
        let mut body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": [{"role": "user", "content": user}],
        });
        if let Some(effort) = self.effort {
            body["output_config"] = json!({"effort": effort});
        }
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
            (self.warn)(&error_summary(resp.status, &resp.body));
            return Err(ProviderError::Http(resp.status));
        }
        let v: Value =
            serde_json::from_str(&resp.body).map_err(|e| ProviderError::Parse(e.to_string()))?;
        match v.get("stop_reason").and_then(Value::as_str) {
            Some("max_tokens") | Some("model_context_window_exceeded") => {
                return Err(ProviderError::Parse("truncated".into()))
            }
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
                set_cookie: None,
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
        assert!(headers.contains(&("anthropic-version".into(), "2026-10-21".into())));
        assert!(headers.contains(&("content-type".into(), "application/json".into())));
        assert!(!headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization")));
        assert_eq!(body["model"], "claude-sonnet-5-5");
        assert_eq!(body["max_tokens"], 16000);
        assert_eq!(body["output_config"], json!({"effort": "low"}));
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
        assert_eq!(
            keys,
            ["max_tokens", "messages", "model", "output_config", "system"]
        );
    }

    #[tokio::test]
    async fn effort_none_omits_output_config() {
        let h = fake(200, OK);
        cl(h.clone())
            .with_effort(None)
            .complete_json("s", "u")
            .await
            .unwrap();
        let body = h.seen.borrow()[0].2.clone();
        assert!(body.get("output_config").is_none());
        let mut keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["max_tokens", "messages", "model", "system"]);
    }

    #[test]
    fn parse_effort_values() {
        assert_eq!(parse_effort(None), Ok(Some("low")));
        assert_eq!(parse_effort(Some("  ")), Ok(Some("low")));
        assert_eq!(parse_effort(Some("medium")), Ok(Some("medium")));
        assert_eq!(parse_effort(Some("high")), Ok(Some("high")));
        assert_eq!(parse_effort(Some("none")), Ok(None));
        assert!(matches!(
            parse_effort(Some("max")),
            Err(ProviderError::Parse(m)) if m == "unknown LLM_EFFORT: max (use low, medium, high or none)"
        ));
    }

    #[tokio::test]
    async fn context_window_exceeded_is_truncated_parse_error() {
        let r = cl(fake(200, &wrap("{\"a\":", "model_context_window_exceeded")))
            .complete_json("s", "u")
            .await;
        assert_eq!(r, Err(ProviderError::Parse("truncated".into())));
    }

    #[tokio::test]
    async fn missing_stop_reason_is_accepted() {
        let b = json!({"content": [{"type": "text", "text": "{\"a\":3}"}]}).to_string();
        let v = cl(fake(200, &b)).complete_json("s", "u").await.unwrap();
        assert_eq!(v["a"], 3);
        let b = json!({"content": [{"type": "text", "text": "{\"a\":4}"}], "stop_reason": null})
            .to_string();
        assert_eq!(
            cl(fake(200, &b)).complete_json("s", "u").await.unwrap()["a"],
            4
        );
    }

    thread_local! {
        static WARNED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
    }
    fn record_warn(m: &str) {
        WARNED.with(|w| w.borrow_mut().push(m.to_string()));
    }

    #[tokio::test]
    async fn http_error_logs_type_and_request_id_only() {
        WARNED.with(|w| w.borrow_mut().clear());
        let body = r#"{"type":"error","error":{"type":"billing_error","message":"secret ak leaked sk-ant-xyz"},"request_id":"req_011CSHoEeqs5C35K2UUqR7Fy"}"#;
        let r = cl(fake(402, body))
            .with_warn(record_warn)
            .complete_json("s", "u")
            .await;
        assert_eq!(r, Err(ProviderError::Http(402)));
        let r = cl(fake(502, "<html>bad gateway</html>"))
            .with_warn(record_warn)
            .complete_json("s", "u")
            .await;
        assert_eq!(r, Err(ProviderError::Http(502)));
        let r = cl(fake(200, OK))
            .with_warn(record_warn)
            .complete_json("s", "u")
            .await;
        assert!(r.is_ok());
        WARNED.with(|w| {
            let w = w.borrow();
            assert_eq!(
                *w,
                [
                    "claude http 402: error.type=billing_error request_id=req_011CSHoEeqs5C35K2UUqR7Fy",
                    "claude http 502: error.type=unknown request_id=unknown",
                ]
            );
        });
    }
}
