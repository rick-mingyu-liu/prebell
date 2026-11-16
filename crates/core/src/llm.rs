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

/// A tail longer than this is refused unread: a note is short, and the scan below stays cheap.
const TAIL_MAX_BYTES: usize = 4096;
/// How many `{` in a tail are tried as the start of a JSON object; a tail with more is refused.
const TAIL_SCAN_STARTS: usize = 32;

/// `s` on one line (whitespace runs become one space; control and bidirectional-override
/// characters become `?`, since the text reaches logs and alert messages), cut to
/// `TAIL_SNIPPET_CHARS` characters with its full length noted.
fn snippet(s: &str) -> String {
    let clean: String = s
        .chars()
        .map(|c| match c {
            c if c.is_whitespace() => ' ',
            c if c.is_control() => '?',
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => '?',
            '\u{200E}' | '\u{200F}' | '\u{061C}' => '?',
            c => c,
        })
        .collect();
    let flat = clean.split_whitespace().collect::<Vec<_>>().join(" ");
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

/// Why the text after a reply's object means the reply must be refused, or `None` when it
/// reads as a note that can be dropped. Refused: a tail too long to be a note; one that
/// continues or closes JSON (after any whitespace, invisible characters and an opening
/// fence, it starts with `,` `]` `}` `:` `{` `[`, or with a string that completes an object
/// or array), so the object was a fragment or a second value follows; and one that holds a
/// non-empty JSON object anywhere ("Corrected: {…}", a second fenced block). Arrays and
/// empty objects inside a note are allowed: notes say "articles [2] and [5]" and
/// "returned {} for none".
///
/// Known limit: a second answer written as something other than JSON ("Corrected:
/// {'a': 2}", "{a: 2}", YAML, an object cut off mid-way) reads as a note and is dropped.
/// The kept object is still checked by its caller like any other reply.
fn tail_refusal(tail: &str) -> Option<&'static str> {
    const MORE: &str = "more JSON after the reply's object";
    if tail.len() > TAIL_MAX_BYTES {
        return Some("too much text after the reply's object");
    }
    let invisible = |c: char| {
        c.is_whitespace() || matches!(c, '\u{FEFF}' | '\u{200B}'..='\u{200D}' | '\u{2060}')
    };
    let lead = tail.trim_start_matches(invisible);
    // A fence that opens a second block, with any label, is looked through.
    let lead = match lead
        .strip_prefix("```")
        .or_else(|| lead.strip_prefix("~~~"))
    {
        Some(rest) => rest
            .trim_start_matches(|c: char| {
                c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.' | '_')
            })
            .trim_start_matches(invisible),
        None => lead,
    };
    if lead.starts_with([',', ']', '}', ':', '{', '[']) {
        return Some(MORE);
    }
    // A leading string is the rest of a cut object or array only when an opening brace or
    // bracket in front of it makes JSON; "\"date\": none found." is a note.
    if lead.starts_with('"')
        && ["{", "["].iter().any(|open| {
            let whole = format!("{open}{lead}");
            let mut values = serde_json::Deserializer::from_str(&whole).into_iter::<Value>();
            matches!(values.next(), Some(Ok(_)))
        })
    {
        return Some(MORE);
    }
    for (n, (i, _)) in tail.match_indices('{').enumerate() {
        if n == TAIL_SCAN_STARTS {
            return Some(MORE);
        }
        let mut values = serde_json::Deserializer::from_str(&tail[i..]).into_iter::<Value>();
        if matches!(values.next(), Some(Ok(Value::Object(o))) if !o.is_empty()) {
            return Some(MORE);
        }
    }
    None
}

/// The JSON object a model replied with, and what followed it.
#[derive(Debug, PartialEq)]
pub(crate) struct Reply {
    pub value: Value,
    /// A one-line snippet of the note dropped after the object (text `tail_refusal` let
    /// through), for the provider's `warn` sink. `None` when nothing followed it.
    pub dropped: Option<String>,
}

/// Reads a model's JSON reply: the first JSON value in `text`, which must be an object,
/// with or without a Markdown fence (```` ```json ```` or bare ```` ``` ````) around it.
///
/// A note after the object is tolerated and reported in `Reply::dropped`: models sometimes
/// add one after the answer (seen live on 2026-11-13 and 2026-11-16 as "trailing
/// characters at line 3 column 1", which failed the whole call). A tail that holds or
/// continues JSON is still an error (`tail_refusal`), with a snippet of it in the message:
/// a reply split in two, cut into a fragment, or corrected after the fact must not pass as
/// its first half. Nothing before the object is tolerated, and truncated JSON still fails.
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
    // What follows the object, past the fence that closes it.
    let tail = body[values.byte_offset()..].trim();
    let tail = tail.strip_prefix("```").map_or(tail, str::trim_start);
    if let Some(why) = tail_refusal(tail) {
        return Err(ProviderError::Parse(format!("{why}: {}", snippet(tail))));
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
    }

    /// The review's ways around a rule that only looked at how the tail starts: a second
    /// or corrected answer after prose, a second fence, invisible characters, a list
    /// marker or a rule, and an object that was only a fragment of the answer.
    #[test]
    fn a_tail_that_holds_or_continues_json_is_refused() {
        for text in [
            "{\"a\":1}\n\nCorrected: {\"a\":2}",
            "{\"a\":1}\n\nWait, correction:\n```json\n{\"a\":2}\n```",
            "{\"a\":1}\n```json\n{\"a\":2}",
            "```json\n{\"a\":1}\n```\n```javascript\n{\"a\":2}\n```",
            "```json\n{\"a\":1}\n```\n~~~json\n{\"a\":2}\n~~~",
            "{\"a\":1}\n\u{FEFF}{\"a\":2}",
            "{\"a\":1}\n\u{200B}{\"a\":2}",
            "{\"a\":1}\n\u{FEFF}[1,2]",
            "{\"a\":1},{\"a\":2}",
            "{\"a\":1}\n, {\"a\":2}",
            "{\"a\":1}\n2. {\"a\":2}",
            "{\"a\":1}\n---\n{\"a\":2}",
            "{\"a\":1}\nNote: I left out {\"a\":2}.",
            "{\"a\":1}\nCorrected: [{\"a\":2}]",
            // Fragments: the object closed early and its rest follows.
            "{\"items\":[{\"t\":\"A\"}]}\n,{\"t\":\"B\"}]}",
            "{\"items\":[{\"t\":\"A\"}]}, \"more\": [{\"t\":\"B\"}]}",
            "{\"a\":1}}",
            "{\"a\":1}]",
            "{\"a\":1}\n\"b\": 2}",
            "{\"a\":1}\n\"x\", \"y\"]}",
            "{\"a\":1}\n\"b\"\n: 2}",
            // A second fenced block that holds an array, with any label or fence.
            "```json\n{\"a\":1}\n```\n```json\n[1,2]\n```",
            "```json\n{\"a\":1}\n```\n```jsonc\n[1,2]\n```",
            "{\"a\":1}\n~~~\n[1,2]\n~~~",
        ] {
            let m = parse_err(text);
            assert!(
                m.starts_with("more JSON after the reply's object: "),
                "{text:?}: {m}"
            );
        }
    }

    #[test]
    fn notes_with_brackets_quotes_and_empty_braces_are_still_notes() {
        for note in [
            "Note: articles [2] and [5] were undated.",
            "Note: items is [] because none matched.",
            "\"Reuters\" piece skipped.",
            "Note: returned {} for none.",
            "Note: article id=\"3\" had no date [1,2].",
            "Note: 3 of 7 articles (the {older} ones) were skipped [see above].",
            "\u{201C}Quoted\u{201D} note.",
            "2 articles had no date.",
            // A quoted word first is a note unless it completes JSON.
            "\"date\": none found for article 3.",
            "\"AAPL\": no reason found.",
            "\"Reuters\", \"Bloomberg\" and \"CNBC\" agreed.",
            "Tickers {AAPL, MSFT} had no news.",
            "注：［2］和［5］未注明日期，｛无｝。",
            // The known limit: a second answer that is not JSON reads as a note.
            "Corrected: {'a': 2}",
        ] {
            let r = parsed(&format!("{{\"a\":1}}\n\n{note}"));
            assert_eq!(r.value["a"], 1, "{note:?}");
            assert_eq!(r.dropped.as_deref(), Some(note), "{note:?}");
        }
        // A note inside its own fence is a note too (its opening fence reads as a closing one).
        let r = parsed("{\"a\":1}\n```\nA note inside its own fence.\n```");
        assert_eq!(
            r.dropped.as_deref(),
            Some("A note inside its own fence. ```")
        );
    }

    #[test]
    fn a_tail_too_long_or_too_brace_heavy_to_be_a_note_is_refused() {
        let long = format!("{{\"a\":1}}\n{}", "word ".repeat(TAIL_MAX_BYTES / 5 + 1));
        assert!(parse_err(&long).starts_with("too much text after the reply's object: "));
        let at_cap = format!("{{\"a\":1}}\n{}", "w".repeat(TAIL_MAX_BYTES));
        assert!(parsed(&at_cap).dropped.is_some());
        let braces = format!("{{\"a\":1}}\nsee {}", "{x} ".repeat(TAIL_SCAN_STARTS + 1));
        assert!(parse_err(&braces).starts_with("more JSON after the reply's object: "));
        let few = format!("{{\"a\":1}}\nsee {}", "{x} ".repeat(TAIL_SCAN_STARTS));
        assert!(parsed(&few).dropped.is_some());
    }

    #[test]
    fn control_and_bidi_characters_never_reach_a_log_or_alert() {
        let r = parsed("{\"a\":1}\n\u{1b}[2J\u{202e}gnp.exe \u{7}done\u{2066}x");
        assert_eq!(r.dropped.as_deref(), Some("?[2J?gnp.exe ?done?x"));
        let r = parsed("{\"a\":1}\nnote\u{200E}\u{200F}\u{061C}end");
        assert_eq!(r.dropped.as_deref(), Some("note???end"));
        let m = parse_err("{\"a\":1}\n{\"b\":\"\u{1b}[2J\u{202e}\"}");
        assert!(
            !m.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "{m:?}"
        );
    }

    thread_local! {
        static WARNED: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
    }
    fn record_warn(m: &str) {
        WARNED.with(|w| w.borrow_mut().push(m.to_string()));
    }

    /// `llm_from_env` hands its `warn` to both providers: a dropped note reaches it.
    #[tokio::test]
    async fn the_warn_sink_reaches_both_providers() {
        let note = "{\"a\":1}\n\nNote: checked.";
        let claude = serde_json::json!({"content": [{"type": "text", "text": note}],
            "stop_reason": "end_turn"})
        .to_string();
        let deepseek = serde_json::json!({"choices": [{"finish_reason": "stop",
            "message": {"content": note}}]})
        .to_string();
        for (provider, body) in [("claude", claude), ("deepseek", deepseek)] {
            WARNED.with(|w| w.borrow_mut().clear());
            let llm =
                llm_from_env(provider, fake(&body), "k".into(), None, None, record_warn).unwrap();
            assert_eq!(llm.complete_json("s", "u").await.unwrap()["a"], 1);
            assert_eq!(
                WARNED.with(|w| w.borrow().clone()),
                [format!(
                    "{provider} reply: ignored text after the JSON object: Note: checked."
                )],
                "{provider}"
            );
        }
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
