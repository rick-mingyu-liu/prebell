use crate::llm::{dropped_note, parse_reply, strict, Reply};
use crate::providers::{Http, LlmProvider, ProviderError};
use async_trait::async_trait;
use serde_json::{json, Value};

const URL: &str = "https://api.deepseek.com/chat/completions";
/// Thinking is on by default on current DeepSeek models; the drafts and
/// extractions run without it (as the retired `deepseek-chat` did), so the
/// reply stays inside `max_tokens` and `temperature` keeps its effect.
/// <https://api-docs.deepseek.com/guides/thinking_mode>
const THINKING_OFF: &str = "disabled";

pub struct DeepSeek<H> {
    pub http: H,
    pub api_key: String,
    pub model: String,
    /// Sink for text dropped after a reply's JSON (core has no logger; the generator
    /// passes `console_warn!`).
    pub warn: fn(&str),
}

impl<H> DeepSeek<H> {
    pub fn new(http: H, api_key: String, model: String) -> Self {
        Self {
            http,
            api_key,
            model,
            warn: |_| {},
        }
    }

    pub fn with_warn(mut self, warn: fn(&str)) -> Self {
        self.warn = warn;
        self
    }
}

#[async_trait(?Send)]
impl<H: Http> LlmProvider for DeepSeek<H> {
    async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
        let reply = self.reply(system, user).await?;
        if let Some(dropped) = &reply.dropped {
            (self.warn)(&dropped_note("deepseek", dropped));
        }
        Ok(reply.value)
    }

    async fn complete_json_strict(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
        strict(self.reply(system, user).await?)
    }
}

impl<H: Http> DeepSeek<H> {
    /// One call and its parsed reply, with any note after the JSON kept in
    /// `Reply::dropped` for the caller to log or refuse.
    async fn reply(&self, system: &str, user: &str) -> Result<Reply, ProviderError> {
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "response_format": {"type": "json_object"},
            "thinking": {"type": THINKING_OFF},
            "temperature": 0.3,
            "max_tokens": 4096,
            "stream": false,
        });
        let auth = format!("Bearer {}", self.api_key);
        let resp = self
            .http
            .post_json(
                URL,
                &[
                    ("Authorization", auth.as_str()),
                    ("Content-Type", "application/json"),
                ],
                &body,
            )
            .await?;
        if resp.status >= 400 {
            return Err(ProviderError::Http(resp.status));
        }
        let v: Value =
            serde_json::from_str(&resp.body).map_err(|e| ProviderError::Parse(e.to_string()))?;
        let choice = v
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
            .ok_or(ProviderError::Empty)?;
        if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
            return Err(ProviderError::Parse("truncated".into()));
        }
        let content = choice
            .pointer("/message/content")
            .and_then(Value::as_str)
            .ok_or(ProviderError::Empty)?;
        parse_reply(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::HttpResp;
    use std::cell::RefCell;
    use std::rc::Rc;

    const OK: &str = include_str!("../tests/fixtures/deepseek_ok.json");
    const BAD: &str = include_str!("../tests/fixtures/deepseek_bad_json.json");

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
                content_type: None,
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
    fn ds(h: FakeHttp) -> DeepSeek<FakeHttp> {
        DeepSeek::new(h, "dk".into(), "deepseek-flash".into())
    }
    fn wrap(content: &str, finish: &str) -> String {
        json!({"choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":finish}]})
            .to_string()
    }

    #[tokio::test]
    async fn ok_fixture_returns_object() {
        let v = ds(fake(200, OK)).complete_json("s", "u").await.unwrap();
        assert_eq!(v["headline"], "Futures up");
    }

    #[tokio::test]
    async fn non_json_content_is_parse_error() {
        let r = ds(fake(200, BAD)).complete_json("s", "u").await;
        assert!(matches!(r, Err(ProviderError::Parse(_))));
    }

    #[tokio::test]
    async fn non_object_json_is_parse_error() {
        let r = ds(fake(200, &wrap("[1,2]", "stop")))
            .complete_json("s", "u")
            .await;
        assert!(matches!(r, Err(ProviderError::Parse(_))));
    }

    thread_local! {
        static WARNED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    fn record(m: &str) {
        WARNED.with(|w| w.borrow_mut().push(m.to_string()));
    }

    /// The live failure of 2026-11-13 and 2026-11-16: the object, a blank line, a note.
    #[tokio::test]
    async fn a_note_after_the_json_is_ignored_and_warned() {
        WARNED.with(|w| w.borrow_mut().clear());
        let b = wrap("{\"a\":1}\n\nNote: all claims checked.", "stop");
        let r = ds(fake(200, &b))
            .with_warn(record)
            .complete_json("s", "u")
            .await;
        assert_eq!(r.unwrap()["a"], 1);
        assert_eq!(
            WARNED.with(|w| w.borrow().clone()),
            ["deepseek reply: ignored text after the JSON object: Note: all claims checked."]
        );
    }

    /// The judge's case (2026-11-16): a pass, then a change of mind after
    /// the JSON. Strict replies refuse it; `complete_json` still drops it.
    #[tokio::test]
    async fn a_strict_reply_with_a_note_after_its_json_is_an_error() {
        WARNED.with(|w| w.borrow_mut().clear());
        let b = wrap(
            "{\"verdicts\":[]}\n\nOn reflection, claim 2 is not supported.",
            "stop",
        );
        let r = ds(fake(200, &b))
            .with_warn(record)
            .complete_json_strict("s", "u")
            .await;
        assert_eq!(
            r,
            Err(ProviderError::Parse(
                "text after the JSON object: On reflection, claim 2 is not supported.".into()
            ))
        );
        assert!(WARNED.with(|w| w.borrow().is_empty()));
        let r = ds(fake(200, &b)).complete_json("s", "u").await;
        assert_eq!(r.unwrap(), json!({"verdicts": []}));
        let clean = wrap("{\"a\":1}", "stop");
        let r = ds(fake(200, &clean)).complete_json_strict("s", "u").await;
        assert_eq!(r.unwrap()["a"], 1);
    }

    #[tokio::test]
    async fn a_second_json_object_is_a_parse_error() {
        WARNED.with(|w| w.borrow_mut().clear());
        let b = wrap("{\"a\":1}\n\n{\"a\":2}", "stop");
        let r = ds(fake(200, &b))
            .with_warn(record)
            .complete_json("s", "u")
            .await;
        assert_eq!(
            r,
            Err(ProviderError::Parse(
                "more JSON after the reply's object: {\"a\":2}".into()
            ))
        );
        assert!(WARNED.with(|w| w.borrow().is_empty()));
    }

    #[tokio::test]
    async fn a_clean_reply_warns_nothing() {
        WARNED.with(|w| w.borrow_mut().clear());
        let b = wrap("{\"a\":1}", "stop");
        let r = ds(fake(200, &b))
            .with_warn(record)
            .complete_json("s", "u")
            .await;
        assert_eq!(r.unwrap()["a"], 1);
        assert!(WARNED.with(|w| w.borrow().is_empty()));
    }

    #[tokio::test]
    async fn fenced_json_accepted() {
        let b = wrap("```json\n{\"a\":1}\n```", "stop");
        assert_eq!(
            ds(fake(200, &b)).complete_json("s", "u").await.unwrap()["a"],
            1
        );
        let b = wrap("```\n{\"a\":2}\n```", "stop");
        assert_eq!(
            ds(fake(200, &b)).complete_json("s", "u").await.unwrap()["a"],
            2
        );
    }

    #[tokio::test]
    async fn finish_length_is_truncated_parse_error() {
        let r = ds(fake(200, &wrap("{\"a\":", "length")))
            .complete_json("s", "u")
            .await;
        assert_eq!(r, Err(ProviderError::Parse("truncated".into())));
    }

    #[tokio::test]
    async fn empty_choices_and_http_error() {
        let r = ds(fake(200, "{\"choices\":[]}"))
            .complete_json("s", "u")
            .await;
        assert_eq!(r, Err(ProviderError::Empty));
        let r = ds(fake(429, "{}")).complete_json("s", "u").await;
        assert_eq!(r, Err(ProviderError::Http(429)));
    }

    #[tokio::test]
    async fn request_shape_and_auth() {
        let h = fake(200, OK);
        ds(h.clone()).complete_json("sys", "usr").await.unwrap();
        let seen = h.seen.borrow();
        let (url, headers, body) = &seen[0];
        assert_eq!(url, "https://api.deepseek.com/chat/completions");
        assert!(headers.contains(&("Authorization".into(), "Bearer dk".into())));
        assert!(body
            .to_string()
            .contains("\"response_format\":{\"type\":\"json_object\"}"));
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["model"], "deepseek-flash");
        assert_eq!(body["thinking"], json!({"type": "disabled"}));
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "usr");
    }
}
