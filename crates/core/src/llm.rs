//! LLM provider selection by config, plus helpers shared by the providers.

use crate::claude::{self, Claude};
use crate::deepseek::DeepSeek;
use crate::providers::{Http, LlmProvider, ProviderError};
use serde_json::Value;

/// `deepseek-chat` was retired; `deepseek-flash` without thinking is what it served.
const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-flash";

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
/// `effort` (`LLM_EFFORT`: low | medium | high | none, default low) applies to Claude only;
/// DeepSeek ignores it. A bad `effort` is an error for every provider, so a typo is caught at
/// queue start. `warn` receives diagnostics from both providers: text dropped after a reply's
/// JSON, and for Claude HTTP error summaries.
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
        "deepseek" => Ok(Box::new(
            DeepSeek::new(
                http,
                api_key,
                model.unwrap_or_else(|| DEEPSEEK_DEFAULT_MODEL.into()),
            )
            .with_warn(warn),
        )),
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
    ProviderError::Parse(format!(
        "unknown LLM_PROVIDER: {provider} (use claude or deepseek)"
    ))
}

/// How much of a reply's dropped or offending tail goes into a log line or an error.
const TAIL_SNIPPET_CHARS: usize = 80;

/// `s` on one line (whitespace runs become one space), cut to `TAIL_SNIPPET_CHARS`
/// characters with its full length noted, for a log line or an error message.
fn snippet(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let total = flat.chars().count();
    if total <= TAIL_SNIPPET_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(TAIL_SNIPPET_CHARS).collect();
    format!("{head}… ({total} characters)")
}

/// `s` without a Markdown fence marker at its start (```` ``` ````, with or without a
/// `json` label) and the whitespace after it.
fn after_fence(s: &str) -> &str {
    let s = s.trim_start();
    match s.strip_prefix("```") {
        Some(rest) => {
            let label = rest
                .get(..4)
                .is_some_and(|l| l.eq_ignore_ascii_case("json"));
            rest[if label { 4 } else { 0 }..].trim_start()
        }
        None => s,
    }
}

/// The JSON object a model replied with, and what followed it.
#[derive(Debug, PartialEq)]
pub(crate) struct Reply {
    pub value: Value,
    /// A one-line snippet of the text dropped after the object (never JSON), for the
    /// provider's `warn` sink. `None` when nothing followed it.
    pub dropped: Option<String>,
}

/// Reads a model's JSON reply: the first JSON value in `text`, which must be an object,
/// with or without a Markdown fence (```` ```json ```` or bare ```` ``` ````) around it.
///
/// Text after the object is tolerated and reported in `Reply::dropped`: models sometimes
/// add a note after the answer (seen live on 2026-11-13 and 2026-11-16 as "trailing
/// characters at line 3 column 1", which failed the whole call). A second JSON value after
/// the first is still an error, with a snippet of it in the message: a reply split in two,
/// or corrected after the fact, must not pass as its first half. Nothing before the object
/// is tolerated, and truncated JSON still fails.
pub(crate) fn parse_reply(text: &str) -> Result<Reply, ProviderError> {
    let body = after_fence(text);
    let mut values = serde_json::Deserializer::from_str(body).into_iter::<Value>();
    let value = match values.next() {
        Some(Ok(v)) => v,
        Some(Err(e)) => return Err(ProviderError::Parse(e.to_string())),
        None => return Err(ProviderError::Parse("empty reply".into())),
    };
    if !value.is_object() {
        return Err(ProviderError::Parse("not a JSON object".into()));
    }
    // What follows the object, past a closing fence and any fence that opens a second block.
    let tail = body[values.byte_offset()..].trim();
    let tail = after_fence(tail.strip_prefix("```").map_or(tail, str::trim_start));
    if tail.starts_with(['{', '[']) {
        return Err(ProviderError::Parse(format!(
            "more JSON after the reply's object: {}",
            snippet(tail)
        )));
    }
    Ok(Reply {
        value,
        dropped: (!tail.is_empty()).then(|| snippet(tail)),
    })
}

/// The line a provider sends to its `warn` sink when `parse_reply` dropped text.
pub(crate) fn dropped_note(provider: &str, dropped: &str) -> String {
    format!("{provider} reply: ignored text after the JSON object: {dropped}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{Http, HttpResp, ProviderError};
    use async_trait::async_trait;
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
                set_cookie: None,
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
        assert_eq!(seen[0].2["model"], "deepseek-flash");
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
            Some("deepseek-v4-pro".into()),
            None,
            no_warn,
        )
        .unwrap();
        llm.complete_json("s", "u").await.unwrap();
        let seen = h.seen.borrow();
        assert_eq!(seen[0].2["model"], "claude-opus-5-5");
        assert_eq!(seen[1].2["model"], "claude-sonnet-5-5");
        assert_eq!(h2.seen.borrow()[0].2["model"], "deepseek-v4-pro");
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
            Err(ProviderError::Parse(m)) if m == "unknown LLM_PROVIDER: nope (use claude or deepseek)"
        ));
        assert_eq!(
            unknown_provider("x").to_string(),
            "parse error: unknown LLM_PROVIDER: x (use claude or deepseek)"
        );
    }

    fn parsed(text: &str) -> Reply {
        parse_reply(text).unwrap_or_else(|e| panic!("{text:?} did not parse: {e}"))
    }

    fn parse_err(text: &str) -> String {
        match parse_reply(text) {
            Err(ProviderError::Parse(m)) => m,
            other => panic!("{text:?} should be a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_plain_or_fenced_object_parses_with_nothing_dropped() {
        for text in [
            "{\"a\":1}",
            "  {\"a\":1}\n",
            "```json\n{\"a\":1}\n```",
            "```JSON\n{\"a\":1}\n```",
            "```\n{\"a\":1}\n```",
            "```json\n{\"a\":1}",
        ] {
            let r = parsed(text);
            assert_eq!(r.value["a"], 1, "{text:?}");
            assert_eq!(r.dropped, None, "{text:?}");
        }
    }

    /// The live failure: serde's "trailing characters at line 3 column 1" is a reply of
    /// the object, a blank line, then more text.
    #[test]
    fn a_note_after_the_object_is_dropped_and_reported() {
        let text = "{\"items\":[]}\n\nNote: none of the articles names a date.";
        // The old parser's error, to pin the shape this fixes.
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap_err().to_string(),
            "trailing characters at line 3 column 1"
        );
        let r = parsed(text);
        assert_eq!(r.value["items"], serde_json::json!([]));
        assert_eq!(
            r.dropped.as_deref(),
            Some("Note: none of the articles names a date.")
        );
    }

    #[test]
    fn prose_after_a_fenced_object_is_dropped_and_reported() {
        let r = parsed("```json\n{\"a\":1}\n```\nI checked every claim.");
        assert_eq!(r.value["a"], 1);
        assert_eq!(r.dropped.as_deref(), Some("I checked every claim."));
        // A stray closing fence alone is not a tail.
        assert_eq!(parsed("{\"a\":1}\n```").dropped, None);
    }

    #[test]
    fn braces_and_fences_inside_strings_do_not_end_the_object() {
        let r = parsed("{\"a\":\"} ``` {\\\"b\\\":2}\",\"c\":[\"]\"]}\n\nDone.");
        assert_eq!(r.value["a"], "} ``` {\"b\":2}");
        assert_eq!(r.value["c"][0], "]");
        assert_eq!(r.dropped.as_deref(), Some("Done."));
    }

    #[test]
    fn a_second_json_value_after_the_object_is_an_error_that_shows_it() {
        for text in [
            "{\"a\":1}\n{\"a\":2}",
            "{\"a\":1}\n\n[1,2]",
            "```json\n{\"a\":1}\n```\n```json\n{\"a\":2}\n```",
            "{\"a\":1}```{\"a\":2}",
        ] {
            let m = parse_err(text);
            assert!(
                m.starts_with("more JSON after the reply's object: "),
                "{text:?}: {m}"
            );
            assert!(m.contains("2"), "{text:?}: {m}");
        }
        // Prose that mentions JSON later is still only a note.
        let r = parsed("{\"a\":1}\nNote: I left out {\"a\":2}.");
        assert_eq!(r.dropped.as_deref(), Some("Note: I left out {\"a\":2}."));
    }

    #[test]
    fn truncated_empty_non_object_and_prose_replies_still_fail() {
        assert!(parse_err("{\"a\":1").contains("EOF"));
        assert!(parse_err("{\"a\":[1,2").contains("EOF"));
        assert_eq!(parse_err(""), "empty reply");
        assert_eq!(parse_err("   \n"), "empty reply");
        assert!(parse_err("```json\n```").contains("expected value"));
        assert_eq!(parse_err("[1,2]"), "not a JSON object");
        assert_eq!(parse_err("\"text\"\n\nand a note"), "not a JSON object");
        assert_eq!(parse_err("true story: {\"a\":1}"), "not a JSON object");
        assert!(parse_err("Sorry, here is prose, not JSON.").contains("expected value"));
        // Nothing before the object is tolerated.
        assert!(parse_err("Here is the JSON:\n{\"a\":1}").contains("expected value"));
    }

    #[test]
    fn a_long_or_multiline_tail_is_one_capped_line() {
        let tail = "line one\n\n  line two\t".to_string() + &"é".repeat(200);
        let r = parsed(&format!("{{\"a\":1}}\n{tail}"));
        let d = r.dropped.unwrap();
        assert!(d.starts_with("line one line two "), "{d}");
        assert!(!d.contains('\n'));
        assert!(d.ends_with("… (218 characters)"), "{d}");
        assert_eq!(
            d.chars().take_while(|c| *c != '…').count(),
            TAIL_SNIPPET_CHARS
        );
        let m = parse_err(&format!("{{\"a\":1}}\n{{\"b\":\"{}\"}}", "x".repeat(300)));
        assert!(m.ends_with("characters)") && m.chars().count() < 160, "{m}");
    }

    #[test]
    fn the_dropped_note_names_the_provider() {
        assert_eq!(
            dropped_note("claude", "Note: x"),
            "claude reply: ignored text after the JSON object: Note: x"
        );
    }
}
