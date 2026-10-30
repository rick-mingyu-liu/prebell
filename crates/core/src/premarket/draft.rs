//! Turns stored facts into one language's article. The LLM writes the prose,
//! and every number in it must trace to the facts (`numbers::check`). Code
//! renders the tables straight from the facts, so table numbers never pass
//! through the LLM.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::facts::{numbers_in_facts, Field, Mover, NewsReason, PremarketFacts};
use crate::numbers::{check, Lang};
use crate::providers::{LlmProvider, ProviderError};

pub const TEMPLATE_ID: &str = "premarket:v1";
pub const DISCLAIMER_EN: &str =
    "This brief is generated automatically from public data for information only. It is not investment advice.";
pub const DISCLAIMER_ZH: &str = "本简报根据公开数据自动生成，仅供参考，不构成投资建议。";

const PROMPT_EN: &str = include_str!("../../prompts/premarket_v1_en.md");
const PROMPT_ZH: &str = include_str!("../../prompts/premarket_v1_zh.md");
const ATTEMPTS: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub lang: Lang,
    pub title: String,
    pub summary: String,
    /// `(heading, markdown)` in the order the prompt requires.
    pub sections: Vec<(String, String)>,
}

/// The section headings in order. "What to watch" is last and only there
/// when the facts have a watch list.
fn headings(lang: Lang, watch: bool) -> Vec<&'static str> {
    let all = match lang {
        Lang::En => [
            "Futures",
            "Biggest premarket movers",
            "Macro & overnight",
            "What to watch",
        ],
        Lang::Zh => ["期货", "盘前异动", "宏观与隔夜", "今日关注"],
    };
    all[..if watch { 4 } else { 3 }].to_vec()
}

fn prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::En => PROMPT_EN,
        Lang::Zh => PROMPT_ZH,
    }
}

pub(crate) fn na(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "n/a",
        Lang::Zh => "暂缺",
    }
}

/// Two decimals with thousands separators: 5812.25 -> "5,812.25".
pub(crate) fn fmt_num(v: f64) -> String {
    let s = format!("{:.2}", v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if v < 0.0 && s.bytes().any(|b| b.is_ascii_digit() && b != b'0') {
        "-"
    } else {
        ""
    };
    format!("{sign}{grouped}.{frac}")
}

pub(crate) fn fmt_pct(p: f64) -> String {
    format!("{p:+.2}%")
}

/// Chinese contract names for the ZH table; facts keep the English labels.
fn futures_label(label: &str, lang: Lang) -> &str {
    match (lang, label) {
        (Lang::Zh, "S&P 500 futures") => "标普500期货",
        (Lang::Zh, "Nasdaq 100 futures") => "纳指100期货",
        (Lang::Zh, "Dow futures") => "道指期货",
        (Lang::Zh, "Russell 2000 futures") => "罗素2000期货",
        _ => label,
    }
}

/// Keeps a table cell on one line and its pipes literal.
pub(crate) fn cell(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace('|', "\\|")
}

fn mover_row(m: &Mover, lang: Lang) -> String {
    let q = &m.quote;
    let mut row = format!(
        "| {} | {} | {} | {} |",
        cell(&q.symbol),
        cell(&m.name),
        fmt_num(q.last),
        fmt_pct(q.change_pct)
    );
    if lang == Lang::En {
        let why = m
            .reasons
            .first()
            .map_or(na(lang), |r| r.summary_en.as_str());
        row.push_str(&format!(" {} |", cell(why)));
    }
    row
}

/// The full article: title, summary, code-rendered tables, LLM sections,
/// sources and disclaimer.
pub fn render_markdown(d: &Draft, f: &PremarketFacts) -> String {
    let lang = d.lang;
    let mut out = format!("# {}\n\n{}\n\n", d.title, d.summary);

    out.push_str(match lang {
        Lang::En => "| Contract | Last | Change |\n|---|---:|---:|\n",
        Lang::Zh => "| 合约 | 最新 | 涨跌 |\n|---|---:|---:|\n",
    });
    for (label, fq) in &f.futures {
        let (last, chg) = match fq {
            Field::Value(q) => (fmt_num(q.last), fmt_pct(q.change_pct)),
            Field::Missing { .. } => (na(lang).to_string(), na(lang).to_string()),
        };
        out.push_str(&format!(
            "| {} | {last} | {chg} |\n",
            cell(futures_label(label, lang))
        ));
    }
    out.push('\n');

    let movers: Vec<&Mover> = f.gainers.iter().chain(&f.losers).collect();
    if !movers.is_empty() {
        out.push_str(match lang {
            Lang::En => {
                "| Ticker | Company | Premarket | Change | Why |\n|---|---|---:|---:|---|\n"
            }
            Lang::Zh => "| 代码 | 公司 | 盘前价 | 涨跌 |\n|---|---|---:|---:|\n",
        });
        for m in movers {
            out.push_str(&mover_row(m, lang));
            out.push('\n');
        }
        out.push('\n');
    }

    for (heading, body) in &d.sections {
        out.push_str(&format!("## {heading}\n\n{}\n\n", body.trim()));
    }

    let macro_items = match &f.macro_items {
        Field::Value(items) => items.as_slice(),
        Field::Missing { .. } => &[],
    };
    out.push_str(&render_sources(
        f.gainers
            .iter()
            .chain(&f.losers)
            .flat_map(|m| &m.reasons)
            .chain(macro_items),
        lang,
    ));

    out.push_str(match lang {
        Lang::En => DISCLAIMER_EN,
        Lang::Zh => DISCLAIMER_ZH,
    });
    out.push('\n');
    out
}

/// The "Sources" section (with its trailing blank line) listing each url
/// once, or "" when there are none.
pub(crate) fn render_sources<'a>(
    reasons: impl Iterator<Item = &'a NewsReason>,
    lang: Lang,
) -> String {
    let mut seen = HashSet::new();
    let sources: Vec<String> = reasons
        .filter(|r| seen.insert(r.url.as_str()))
        .map(|r| {
            let title = r.title.split_whitespace().collect::<Vec<_>>().join(" ");
            let title = title.replace('[', "\\[").replace(']', "\\]");
            format!("- [{title}]({})\n", r.url)
        })
        .collect();
    if sources.is_empty() {
        return String::new();
    }
    let mut out = String::from(match lang {
        Lang::En => "## Sources\n\n",
        Lang::Zh => "## 来源\n\n",
    });
    out.extend(sources);
    out.push('\n');
    out
}

enum Problem {
    Llm(ProviderError),
    Malformed(String),
    Numbers(Vec<String>),
}

/// Markup the LLM may not emit: links (inline, reference-style, bare or
/// autolinked), images, HTML and URLs would let it cite sources outside the
/// facts or forge page structure. Matched case-insensitively.
const FORBIDDEN: [&str; 10] = [
    "](",
    "]:",
    "![",
    "<",
    "http://",
    "https://",
    "//",
    "www.",
    "javascript:",
    "mailto:",
];

/// GFM autolinks bare emails as `mailto:` links.
fn email_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\S+@\S+\.\S+").expect("valid built-in regex"))
}

/// Rejects text that markdown could render as a link, image or HTML.
/// Shared with reason extraction, whose `summary_en` lands in the EN table.
pub(crate) fn reject_markup(what: &str, s: &str) -> Result<(), String> {
    let lower = s.to_lowercase();
    if let Some(t) = FORBIDDEN.iter().find(|t| lower.contains(*t)) {
        return Err(format!("{what} contains forbidden markup {t:?}"));
    }
    if email_re().is_match(s) {
        return Err(format!("{what} contains an email address"));
    }
    Ok(())
}

/// A setext heading underline: a line of only `=` or only `-`, length >= 2.
pub(crate) fn is_setext_underline(line: &str) -> bool {
    let l = line.trim();
    l.len() >= 2 && (l.bytes().all(|b| b == b'=') || l.bytes().all(|b| b == b'-'))
}

/// A one-line string: whitespace runs (newlines included) become one space.
pub(crate) fn one_line(v: &Value, key: &str) -> Result<String, String> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if s.is_empty() {
        return Err(format!("missing or empty \"{key}\""));
    }
    reject_markup(key, &s)?;
    Ok(s)
}

fn parse(v: &Value, lang: Lang, want: &[&str]) -> Result<Draft, String> {
    let title = one_line(v, "title")?;
    let summary = one_line(v, "summary")?;
    if summary.starts_with('#') {
        return Err("summary starts with a heading marker".into());
    }
    let items = v
        .get("sections")
        .and_then(Value::as_array)
        .ok_or("missing \"sections\" array")?;
    let got: Vec<(String, String)> = items
        .iter()
        .map(|s| {
            let h = s
                .get("heading")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let m = s
                .get("markdown")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (h, m)
        })
        .collect();
    if got.len() != want.len() || got.iter().zip(want).any(|((h, _), w)| h != w) {
        let seen: Vec<&str> = got.iter().map(|(h, _)| h.as_str()).collect();
        return Err(format!(
            "sections must be exactly {want:?} in order, got {seen:?}"
        ));
    }
    for (h, m) in &got {
        reject_markup(&format!("section {h:?}"), m)?;
        if m.lines()
            .any(|l| l.trim_start().starts_with('#') || is_setext_underline(l))
        {
            return Err(format!("section {h:?} contains a heading line"));
        }
    }
    Ok(Draft {
        lang,
        title,
        summary,
        sections: got,
    })
}

/// `text` with each whole-word occurrence of `names` replaced by a space, so
/// digits that belong to a known name ("3M", "Phillips 66") are not checked
/// as figures, without allowing those digits anywhere else.
pub(crate) fn blank_names(text: &str, names: &[&str]) -> String {
    let mut out = text.to_string();
    for name in names.iter().filter(|n| !n.is_empty()) {
        let re = Regex::new(&format!(r"(^|[^\w.$]){}($|[^\w])", regex::escape(name)))
            .expect("escaped name is a valid regex");
        // Twice, since adjacent matches share their boundary character.
        for _ in 0..2 {
            out = re.replace_all(&out, "${1} ${2}").into_owned();
        }
    }
    out
}

fn attempt(
    v: Result<Value, ProviderError>,
    lang: Lang,
    want: &[&str],
    allowed: &[f64],
    names: &[&str],
) -> Result<Draft, Problem> {
    let v = v.map_err(Problem::Llm)?;
    let d = parse(&v, lang, want).map_err(Problem::Malformed)?;
    let mut text = format!("{}\n{}", d.title, d.summary);
    for (h, m) in &d.sections {
        text.push_str(&format!("\n{h}\n{m}"));
    }
    check(&blank_names(&text, names), lang, allowed).map_err(|bad| {
        let mut seen = HashSet::new();
        Problem::Numbers(bad.into_iter().filter(|n| seen.insert(n.clone())).collect())
    })?;
    Ok(d)
}

fn retry_note(p: &Problem) -> String {
    let line = match p {
        Problem::Llm(e) => format!("The previous request failed: {e}"),
        Problem::Malformed(m) => format!("Your reply was invalid: {m}"),
        Problem::Numbers(n) => format!("These numbers are not in the facts: {}", n.join(", ")),
    };
    format!("\n\nYour previous reply had problems. Fix them and reply again:\n- {line}\n")
}

fn errors(p: Problem) -> Vec<String> {
    match p {
        Problem::Llm(e) => vec![format!("llm: {e}")],
        Problem::Malformed(m) => vec![format!("draft: {m}")],
        Problem::Numbers(n) => n,
    }
}

/// Drafts one language from the facts, retrying once with the problems
/// appended. Returns the draft and its rendered markdown, or the last
/// attempt's errors.
pub async fn write(
    llm: &dyn LlmProvider,
    f: &PremarketFacts,
    lang: Lang,
) -> Result<(Draft, String), Vec<String>> {
    // Diagnostics are operational counters, not facts the brief may cite.
    let for_llm = PremarketFacts {
        diagnostics: vec![],
        ..f.clone()
    };
    let facts_json = serde_json::to_string_pretty(&for_llm)
        .map_err(|e| vec![format!("draft: facts json: {e}")])?;
    write_with(
        llm,
        prompt(lang),
        &facts_json,
        &numbers_in_facts(f),
        &[],
        &headings(lang, matches!(f.watch_list, Field::Value(_))),
        lang,
        |d| render_markdown(d, f),
    )
    .await
}

/// The drafting loop shared by every content type: ask `llm` with `system`
/// and the facts JSON, require exactly `headings`, reject markup, check every
/// number against `allowed` (ignoring the digits of known `names`), and
/// retry once with the problems appended. On success returns the draft and
/// `render(draft)`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn write_with(
    llm: &dyn LlmProvider,
    system: &str,
    facts_json: &str,
    allowed: &[f64],
    names: &[&str],
    headings: &[&str],
    lang: Lang,
    render: impl Fn(&Draft) -> String,
) -> Result<(Draft, String), Vec<String>> {
    let mut user = facts_json.to_string();
    let mut last = None;
    for _ in 0..ATTEMPTS {
        match attempt(
            llm.complete_json(system, &user).await,
            lang,
            headings,
            allowed,
            names,
        ) {
            Ok(d) => {
                let md = render(&d);
                return Ok((d, md));
            }
            Err(p) => {
                user = format!("{facts_json}{}", retry_note(&p));
                last = Some(p);
            }
        }
    }
    Err(last.map(errors).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Field, Mover, NewsReason, Quote};
    use async_trait::async_trait;
    use chrono::NaiveDate;
    use serde_json::{json, Value};
    use std::cell::RefCell;

    /// Replies with scripted responses in order; records every user prompt.
    struct FakeLlm {
        replies: RefCell<Vec<Result<Value, ProviderError>>>,
        prompts: RefCell<Vec<(String, String)>>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
            self.prompts.borrow_mut().push((system.into(), user.into()));
            let mut r = self.replies.borrow_mut();
            assert!(!r.is_empty(), "unexpected extra LLM call");
            r.remove(0)
        }
    }
    fn llm(replies: Vec<Result<Value, ProviderError>>) -> FakeLlm {
        FakeLlm {
            replies: RefCell::new(replies),
            prompts: RefCell::new(vec![]),
        }
    }

    fn quote(sym: &str, last: f64, pct: f64) -> Quote {
        Quote {
            symbol: sym.into(),
            last,
            prev_close: last / (1.0 + pct / 100.0),
            change_pct: pct,
            volume: Some(2_000_000),
        }
    }
    fn reason(url: &str, title: &str) -> NewsReason {
        NewsReason {
            url: url.into(),
            title: title.into(),
            quote: "Apple shares jumped 8% in premarket trading".into(),
            summary_en: "Record iPhone sales.".into(),
        }
    }
    fn facts() -> PremarketFacts {
        PremarketFacts {
            trading_date: NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
            as_of_utc: "2026-10-29T12:45:00Z".parse().unwrap(),
            futures: vec![
                (
                    "S&P 500 futures".into(),
                    Field::Value(quote("ES=F", 5812.25, 0.45)),
                ),
                (
                    "Nasdaq 100 futures".into(),
                    Field::Missing {
                        reason: "feed down".into(),
                    },
                ),
            ],
            gainers: vec![Mover {
                quote: quote("AAPL", 230.5, 8.0),
                name: "Apple".into(),
                reasons: vec![reason("https://news.test/aapl", "Apple jumps")],
            }],
            losers: vec![],
            macro_items: Field::Missing {
                reason: "none".into(),
            },
            watch_list: Field::Missing {
                reason: "not available in v1".into(),
            },
            diagnostics: vec![],
        }
    }

    fn reply(lang: Lang, body: &str) -> Result<Value, ProviderError> {
        reply_with(&headings(lang, false), body)
    }
    fn reply_with(heads: &[&str], body: &str) -> Result<Value, ProviderError> {
        Ok(json!({
            "title": "Futures edge up; Apple jumps 8%",
            "summary": "S&P 500 futures rose 0.45% while Apple gained 8% premarket.",
            "sections": heads.iter().map(|h| json!({"heading": h, "markdown": body})).collect::<Vec<_>>(),
        }))
    }
    fn clean(lang: Lang) -> Result<Value, ProviderError> {
        reply(lang, "S&P 500 futures were at 5,812.25, up 0.45%.")
    }
    fn bad(lang: Lang) -> Result<Value, ProviderError> {
        reply(lang, "Nasdaq futures rose 7.5%.")
    }

    #[tokio::test]
    async fn clean_draft_is_ok() {
        let l = llm(vec![clean(Lang::En)]);
        let (d, md) = write(&l, &facts(), Lang::En).await.unwrap();
        assert_eq!(d.lang, Lang::En);
        assert_eq!(d.title, "Futures edge up; Apple jumps 8%");
        assert_eq!(d.sections.len(), 3);
        assert_eq!(d.sections[0].0, "Futures");
        assert!(
            !md.contains("What to watch") && !md.contains("\nn/a\n"),
            "{md}"
        );
        assert_eq!(l.prompts.borrow().len(), 1);
        let (sys, user) = &l.prompts.borrow()[0];
        assert_eq!(sys, PROMPT_EN);
        assert_eq!(user, &serde_json::to_string_pretty(&facts()).unwrap());
        assert!(md.starts_with("# Futures edge up; Apple jumps 8%"));
        assert!(md.contains(DISCLAIMER_EN));
    }

    #[tokio::test]
    async fn missing_watch_list_drops_its_section_in_both_languages() {
        for (lang, gone) in [(Lang::En, "What to watch"), (Lang::Zh, "今日关注")] {
            let four = reply_with(&headings(lang, true), "Quiet.");
            let l = llm(vec![four, reply(lang, "Quiet.")]);
            let (d, md) = write(&l, &facts(), lang).await.unwrap();
            // A reply that still has the watch section is malformed.
            assert!(l.prompts.borrow()[1].1.contains("Your reply was invalid"));
            assert_eq!(d.sections.len(), 3);
            // No heading and no "n/a" body: missing futures stay as table cells.
            let na_body = format!("\n{}\n", na(lang));
            assert!(!md.contains(gone) && !md.contains(&na_body), "{md}");
        }
    }

    #[tokio::test]
    async fn present_watch_list_requires_and_renders_its_section() {
        let mut f = facts();
        f.watch_list = Field::Value(vec!["Fed minutes at 2 PM ET".into()]);
        for (lang, heading) in [(Lang::En, "What to watch"), (Lang::Zh, "今日关注")] {
            let l = llm(vec![
                reply_with(&headings(lang, false), "Quiet."),
                reply_with(&headings(lang, true), "Quiet."),
            ]);
            let (d, md) = write(&l, &f, lang).await.unwrap();
            assert_eq!(l.prompts.borrow().len(), 2);
            assert_eq!(d.sections.len(), 4);
            assert!(md.contains(&format!("## {heading}\n\nQuiet.")), "{md}");
        }
    }

    #[tokio::test]
    async fn unbacked_number_is_retried_with_errors() {
        let l = llm(vec![bad(Lang::En), clean(Lang::En)]);
        assert!(write(&l, &facts(), Lang::En).await.is_ok());
        let p = l.prompts.borrow();
        assert_eq!(p.len(), 2);
        assert!(!p[0].1.contains("7.5%"));
        assert!(p[1].1.contains("7.5%"));
        assert!(p[1].1.contains("These numbers are not in the facts"));
    }

    #[tokio::test]
    async fn two_bad_replies_fail_with_numbers() {
        let l = llm(vec![bad(Lang::En), bad(Lang::En)]);
        assert_eq!(
            write(&l, &facts(), Lang::En).await.unwrap_err(),
            vec!["7.5%"]
        );
    }

    #[tokio::test]
    async fn malformed_then_good_reply_is_ok() {
        let l = llm(vec![
            Ok(json!({"title": "", "summary": "x", "sections": []})),
            clean(Lang::En),
        ]);
        assert!(write(&l, &facts(), Lang::En).await.is_ok());
        let p = l.prompts.borrow();
        assert_eq!(p.len(), 2);
        assert!(p[1].1.contains("Your reply was invalid"));
    }

    #[tokio::test]
    async fn wrong_headings_are_malformed() {
        let l = llm(vec![clean(Lang::Zh), clean(Lang::Zh)]);
        let err = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(err.len() == 1 && err[0].starts_with("draft: "), "{err:?}");
    }

    #[tokio::test]
    async fn provider_errors_are_reported() {
        let l = llm(vec![
            Err(ProviderError::Http(500)),
            Err(ProviderError::Http(503)),
        ]);
        let err = write(&l, &facts(), Lang::Zh).await.unwrap_err();
        assert_eq!(err, vec!["llm: http status 503"]);
        let p = l.prompts.borrow();
        assert_eq!(p[0].0, PROMPT_ZH);
        assert!(p[1].1.contains("http status 500"));
    }

    #[tokio::test]
    async fn zh_draft_renders_zh_disclaimer() {
        let l = llm(vec![clean(Lang::Zh)]);
        let (d, md) = write(&l, &facts(), Lang::Zh).await.unwrap();
        assert_eq!(d.lang, Lang::Zh);
        assert!(md.contains(DISCLAIMER_ZH) && !md.contains(DISCLAIMER_EN));
        assert!(md.contains("## 期货") && md.contains("来源"));
    }

    fn draft(lang: Lang) -> Draft {
        Draft {
            lang,
            title: "T".into(),
            summary: "S".into(),
            sections: headings(lang, false)
                .iter()
                .map(|h| (h.to_string(), "body".into()))
                .collect(),
        }
    }

    #[test]
    fn render_contains_es_last_from_facts() {
        let md = render_markdown(&draft(Lang::En), &facts());
        assert!(
            md.contains("| S&P 500 futures | 5,812.25 | +0.45% |"),
            "{md}"
        );
        assert!(md.contains("| Nasdaq 100 futures | n/a | n/a |"), "{md}");
        assert!(md.contains("Contract | Last | Change"));
        assert!(md.contains("Ticker | Company | Premarket | Change | Why"));
        assert!(
            md.contains("| AAPL | Apple | 230.50 | +8.00% | Record iPhone sales. |"),
            "{md}"
        );
        assert!(md.contains("## Sources") && md.contains("[Apple jumps](https://news.test/aapl)"));
        assert!(md.trim_end().ends_with(DISCLAIMER_EN));
    }

    #[test]
    fn zh_render_uses_zh_missing_and_headers() {
        let md = render_markdown(&draft(Lang::Zh), &facts());
        assert!(md.contains("| 标普500期货 | 5,812.25 | +0.45% |"), "{md}");
        assert!(md.contains("| 纳指100期货 | 暂缺 | 暂缺 |"), "{md}");
        assert!(!md.contains("futures"), "{md}");
        assert!(md.contains("合约 | 最新 | 涨跌") && md.contains("代码 | 公司 | 盘前价 | 涨跌"));
        assert!(md.contains("| AAPL | Apple | 230.50 | +8.00% |") && !md.contains("Record iPhone"));
        assert!(md.trim_end().ends_with(DISCLAIMER_ZH));
    }

    #[test]
    fn zh_futures_labels_cover_every_contract() {
        let labels: Vec<&str> = [
            "S&P 500 futures",
            "Nasdaq 100 futures",
            "Dow futures",
            "Russell 2000 futures",
        ]
        .iter()
        .map(|l| futures_label(l, Lang::Zh))
        .collect();
        assert_eq!(
            labels,
            ["标普500期货", "纳指100期货", "道指期货", "罗素2000期货"]
        );
        assert_eq!(futures_label("Dow futures", Lang::En), "Dow futures");
    }

    #[tokio::test]
    async fn zh_futures_names_pass_the_number_check() {
        let body = "标普500期货报5,812.25点，上涨0.45%；纳指100期货、道指期货和罗素2000期货暂缺。";
        let l = llm(vec![reply(Lang::Zh, body)]);
        let (_, md) = write(&l, &facts(), Lang::Zh).await.unwrap();
        assert_eq!(l.prompts.borrow().len(), 1);
        // The rendered table's labels carry no stray figures either.
        let table: String = md.lines().filter(|l| l.contains("期货 |")).collect();
        assert_eq!(check(&table, Lang::Zh, &[5812.25, 0.45]), Ok(()), "{table}");
    }

    #[test]
    fn sources_are_unique() {
        let mut f = facts();
        f.losers = vec![Mover {
            quote: quote("MSFT", 410.0, -3.1),
            name: "Microsoft".into(),
            reasons: vec![
                reason("https://news.test/aapl", "Apple jumps"),
                reason("https://news.test/msft", "MSFT"),
            ],
        }];
        f.macro_items = Field::Value(vec![reason("https://news.test/msft", "MSFT")]);
        let md = render_markdown(&draft(Lang::En), &f);
        assert_eq!(md.matches("(https://news.test/aapl)").count(), 1, "{md}");
        assert_eq!(md.matches("(https://news.test/msft)").count(), 1, "{md}");
        assert!(
            md.contains("| MSFT | Microsoft | 410.00 | -3.10% |"),
            "{md}"
        );
    }

    #[tokio::test]
    async fn forged_title_is_rejected_then_retried() {
        let forged = Ok(json!({
            "title": "X\n\n## Sources\n- [Official filing](https://evil.test)",
            "summary": "Futures were mixed.",
            "sections": headings(Lang::En, false).iter().map(|h| json!({"heading": h, "markdown": "Quiet."})).collect::<Vec<_>>(),
        }));
        let l = llm(vec![forged, clean(Lang::En)]);
        let (_, md) = write(&l, &facts(), Lang::En).await.unwrap();
        assert_eq!(l.prompts.borrow().len(), 2);
        assert!(l.prompts.borrow()[1].1.contains("Your reply was invalid"));
        assert!(!md.contains("evil.test"), "{md}");
    }

    #[tokio::test]
    async fn links_in_body_fail_as_malformed() {
        let l = llm(vec![
            reply(Lang::En, "See [x](https://a.test)."),
            reply(Lang::En, "See [x](https://a.test)."),
        ]);
        let err = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(err.iter().any(|e| e.starts_with("draft:")), "{err:?}");
    }

    #[tokio::test]
    async fn heading_line_in_body_is_malformed() {
        let l = llm(vec![
            reply(Lang::En, "ok\n## Sources"),
            reply(Lang::En, "<b>hi</b>"),
        ]);
        let err = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(err.len() == 1 && err[0].starts_with("draft:"), "{err:?}");
        assert!(l.prompts.borrow()[1].1.contains("heading line"));
    }

    #[tokio::test]
    async fn title_newlines_collapse_to_one_line() {
        let mut v = clean(Lang::En).unwrap();
        v["title"] = json!("Futures edge up;\n\n  Apple   jumps 8%");
        v["summary"] = json!("Apple\ngained 8%.");
        let l = llm(vec![Ok(v)]);
        let (d, md) = write(&l, &facts(), Lang::En).await.unwrap();
        assert_eq!(d.title, "Futures edge up; Apple jumps 8%");
        assert_eq!(d.summary, "Apple gained 8%.");
        assert!(
            md.starts_with("# Futures edge up; Apple jumps 8%\n\nApple gained 8%.\n\n"),
            "{md}"
        );
    }

    #[test]
    fn forbidden_markup_in_sections_is_rejected() {
        for body in [
            "See HTTPS://evil.example/x today.",
            "See Http://evil.example/x today.",
            "Visit www.evil.example for more.",
            "Visit WWW.evil.example for more.",
            "Write to ceo@evil.example now.",
            "Read [here][r].\n\n[r]: //evil.example/p.png",
            "Look ![p][r] here.",
            "Proto-relative //evil.example/p.png link.",
            "JavaScript:alert(1)",
            "Forged heading\n===",
            "Forged heading\n---",
            "Forged heading\n  ----  ",
        ] {
            let v = reply(Lang::En, body).unwrap();
            assert!(
                parse(&v, Lang::En, &headings(Lang::En, false)).is_err(),
                "should reject: {body:?}"
            );
        }
    }

    #[test]
    fn ordinary_prose_and_lists_pass_markup_check() {
        for body in [
            "Futures were mixed; 3-5 names led.",
            "- Apple rose\n- Microsoft fell",
            "Rates and/or yields moved.",
            "A single dash line\n-",
        ] {
            let v = reply(Lang::En, body).unwrap();
            assert!(
                parse(&v, Lang::En, &headings(Lang::En, false)).is_ok(),
                "should accept: {body:?}"
            );
        }
    }

    #[test]
    fn forbidden_markup_in_title_and_summary_is_rejected() {
        for (key, val) in [
            ("summary", "# Forged heading"),
            ("summary", "  ## Forged heading"),
            ("summary", "Mail ceo@evil.example"),
            ("title", "See WWW.evil.example"),
        ] {
            let mut v = clean(Lang::En).unwrap();
            v[key] = json!(val);
            assert!(
                parse(&v, Lang::En, &headings(Lang::En, false)).is_err(),
                "should reject {key}: {val:?}"
            );
        }
    }

    #[test]
    fn source_titles_collapse_whitespace() {
        let mut f = facts();
        f.gainers[0].reasons = vec![reason(
            "https://news.test/aapl",
            "Apple\n\n[r]: //evil.example/p.png\n  jumps",
        )];
        let md = render_markdown(&draft(Lang::En), &f);
        assert!(
            md.contains("- [Apple \\[r\\]: //evil.example/p.png jumps](https://news.test/aapl)\n"),
            "{md}"
        );
    }

    #[test]
    fn thousands_separator_formatting() {
        assert_eq!(fmt_num(1234567.891), "1,234,567.89");
        assert_eq!(fmt_num(-42.0), "-42.00");
        assert_eq!(fmt_num(999.999), "1,000.00");
    }

    #[tokio::test]
    async fn write_with_uses_given_headings_and_render() {
        let two = |heads: &[&str]| -> Result<Value, ProviderError> {
            Ok(json!({
                "title": "T",
                "summary": "S",
                "sections": heads.iter().map(|h| json!({"heading": h, "markdown": "Up 0.45%."})).collect::<Vec<_>>(),
            }))
        };
        let l = llm(vec![two(&["A"]), two(&["A", "B"])]);
        let (d, md) = write_with(
            &l,
            "SYS",
            "{}",
            &[0.45],
            &[],
            &["A", "B"],
            Lang::En,
            |d: &Draft| format!("rendered {}", d.sections.len()),
        )
        .await
        .unwrap();
        assert_eq!(md, "rendered 2");
        assert_eq!(d.sections[1].0, "B");
        let p = l.prompts.borrow();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], ("SYS".to_string(), "{}".to_string()));
        assert!(p[1].1.starts_with("{}") && p[1].1.contains("Your reply was invalid"));
    }

    #[test]
    fn blank_names_only_whole_names() {
        let t = "3M fell; 3M's peer rose 33M and $3M; Phillips 66 gained 66%.";
        assert_eq!(
            blank_names(t, &["3M", "Phillips 66"]),
            "  fell;  's peer rose 33M and $3M;   gained 66%."
        );
    }

    #[tokio::test]
    async fn diagnostics_are_not_sent_to_the_llm() {
        let l = llm(vec![clean(Lang::En)]);
        let mut f = facts();
        f.diagnostics = vec!["quotes ok=480 http429=0".into()];
        write(&l, &f, Lang::En).await.unwrap();
        let user = &l.prompts.borrow()[0].1;
        assert!(
            !user.contains("diagnostics") && !user.contains("http429"),
            "{user}"
        );
    }
}
