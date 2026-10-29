//! LLM provider selection by config, plus helpers shared by the providers.

use crate::claude::{self, Claude};
use crate::deepseek::DeepSeek;
use crate::providers::{Http, LlmProvider, ProviderError};

const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-chat";

/// Name of the secret holding the API key for `provider` (the `LLM_PROVIDER` value).
pub fn api_key_var(provider: &str) -> Option<&'static str> {
    match provider {
        "deepseek" => Some("DEEPSEEK_API_KEY"),
        "claude" => Some("ANTHROPIC_API_KEY"),
        _ => None,
    }
}

/// Builds the provider named by `LLM_PROVIDER`. `model` (`LLM_MODEL`) overrides the
/// provider's default model; `None` or empty keeps the default.
///
/// `effort` (`LLM_EFFORT`: low | medium | high | none, default low) and `warn` (receives HTTP
/// error diagnostics) apply to Claude only; DeepSeek ignores both. A bad `effort` is an error
/// for every provider, so a typo is caught at queue start.
pub fn llm_from_env<H: Http + 'static>(
    provider: &str,
    http: H,
    api_key: String,
    model: Option<String>,
    effort: Option<String>,
    warn: fn(&str),
) -> Result<Box<dyn LlmProvider>, ProviderError> {
    let effort = claude::parse_effort(effort.as_deref())?;
    let model = model.filter(|m| !m.is_empty());
    match provider {
        "deepseek" => Ok(Box::new(DeepSeek::new(
            http,
            api_key,
            model.unwrap_or_else(|| DEEPSEEK_DEFAULT_MODEL.into()),
        ))),
        "claude" => Ok(Box::new(
            Claude::new(
                http,
                api_key,
                model.unwrap_or_else(|| claude::DEFAULT_MODEL.into()),
            )
            .with_effort(effort)
            .with_warn(warn),
        )),
        other => Err(unknown_provider(other)),
    }
}

/// The error for an `LLM_PROVIDER` value with no implementation.
pub fn unknown_provider(provider: &str) -> ProviderError {
    ProviderError::Parse(format!("unknown LLM provider: {provider}"))
}

/// Strips a Markdown code fence (```` ```json ```` or bare ```` ``` ````) around a JSON reply.
pub(crate) fn strip_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        return rest.trim();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Http, HttpResp, ProviderError};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::cell::RefCell;
    use std::rc::Rc;

    const DEEPSEEK_OK: &str = include_str!("../tests/fixtures/deepseek_ok.json");
    const CLAUDE_OK: &str = include_str!("../tests/fixtures/claude_ok.json");

    type Seen = Vec<(String, Vec<(String, String)>, Value)>;
    #[derive(Clone)]
    struct FakeHttp {
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
                status: 200,
                body: self.body.clone(),
            })
        }
    }
    fn fake(body: &str) -> FakeHttp {
        FakeHttp {
            body: body.into(),
            seen: Rc::new(RefCell::new(vec![])),
        }
    }

    fn no_warn(_: &str) {}

    #[test]
    fn api_key_var_per_provider() {
        assert_eq!(api_key_var("deepseek"), Some("DEEPSEEK_API_KEY"));
        assert_eq!(api_key_var("claude"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(api_key_var("nope"), None);
    }

    #[tokio::test]
    async fn dispatch_deepseek_default_model() {
        let h = fake(DEEPSEEK_OK);
        let llm = llm_from_env("deepseek", h.clone(), "dk".into(), None, None, no_warn).unwrap();
        assert!(llm.complete_json("s", "u").await.is_ok());
        let seen = h.seen.borrow();
        assert_eq!(seen[0].0, "https://api.deepseek.com/chat/completions");
        assert!(seen[0]
            .1
            .contains(&("Authorization".into(), "Bearer dk".into())));
        assert_eq!(seen[0].2["model"], "deepseek-chat");
    }

    #[tokio::test]
    async fn dispatch_claude_default_model() {
        let h = fake(CLAUDE_OK);
        let llm = llm_from_env("claude", h.clone(), "ak".into(), None, None, no_warn).unwrap();
        assert_eq!(llm.complete_json("s", "u").await.unwrap()["n"], 2);
        let seen = h.seen.borrow();
        assert_eq!(seen[0].0, "https://api.anthropic.com/v1/messages");
        assert!(seen[0].1.contains(&("x-api-key".into(), "ak".into())));
        assert_eq!(seen[0].2["model"], "claude-sonnet-5-5");
    }

    #[tokio::test]
    async fn model_override() {
        let h = fake(CLAUDE_OK);
        let llm = llm_from_env(
            "claude",
            h.clone(),
            "ak".into(),
            Some("claude-opus-5-5".into()),
            None,
            no_warn,
        )
        .unwrap();
        llm.complete_json("s", "u").await.unwrap();
        let llm = llm_from_env(
            "claude",
            h.clone(),
            "ak".into(),
            Some(String::new()),
            None,
            no_warn,
        )
        .unwrap();
        llm.complete_json("s", "u").await.unwrap();
        let h2 = fake(DEEPSEEK_OK);
        let llm = llm_from_env(
            "deepseek",
            h2.clone(),
            "dk".into(),
            Some("deepseek-reasoner".into()),
            None,
            no_warn,
        )
        .unwrap();
        llm.complete_json("s", "u").await.unwrap();
        let seen = h.seen.borrow();
        assert_eq!(seen[0].2["model"], "claude-opus-5-5");
        assert_eq!(seen[1].2["model"], "claude-sonnet-5-5");
        assert_eq!(h2.seen.borrow()[0].2["model"], "deepseek-reasoner");
    }

    #[tokio::test]
    async fn effort_from_env_reaches_claude_body() {
        for (raw, want) in [
            (None, Some("low")),
            (Some(""), Some("low")),
            (Some("medium"), Some("medium")),
            (Some("high"), Some("high")),
            (Some("none"), None),
        ] {
            let h = fake(CLAUDE_OK);
            let llm = llm_from_env(
                "claude",
                h.clone(),
                "ak".into(),
                None,
                raw.map(String::from),
                no_warn,
            )
            .unwrap();
            llm.complete_json("s", "u").await.unwrap();
            let body = h.seen.borrow()[0].2.clone();
            assert_eq!(
                body.pointer("/output_config/effort")
                    .and_then(Value::as_str),
                want,
                "LLM_EFFORT={raw:?}"
            );
        }
    }

    #[test]
    fn bad_effort_errors() {
        for bad in ["max", "xhigh", "LOW", "fast"] {
            assert!(matches!(
                llm_from_env("claude", fake("{}"), "k".into(), None, Some(bad.into()), no_warn),
                Err(ProviderError::Parse(m)) if m.starts_with("unknown LLM_EFFORT")
            ));
        }
    }

    #[test]
    fn unknown_provider_errors() {
        assert!(matches!(
            llm_from_env("nope", fake("{}"), "k".into(), None, None, no_warn),
            Err(ProviderError::Parse(m)) if m == "unknown LLM provider: nope"
        ));
        assert_eq!(
            unknown_provider("x").to_string(),
            "parse error: unknown LLM provider: x"
        );
    }
}
