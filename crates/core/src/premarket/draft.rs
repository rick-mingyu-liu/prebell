//! Turns stored facts into one language's article. The LLM writes the prose,
//! and every number in it must trace to the facts (`numbers::check`). Code
//! renders the tables straight from the facts, so table numbers never pass
//! through the LLM.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::cover::fmt_signed_pct;
use crate::events::figures::figures_text;
use crate::events::watch::{WatchItem, WatchKind};
use crate::facts::{numbers_in_facts, Direction, Field, Mover, NewsReason, PremarketFacts, Theme};
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

/// Every section heading the brief can have, in order: the story first
/// (macro, then themes), then the single names.
fn all_headings(lang: Lang) -> [&'static str; 5] {
    match lang {
        Lang::En => [
            "Futures",
            "Macro & overnight",
            "Themes",
            "Biggest premarket movers",
            "What to watch",
        ],
        Lang::Zh => ["期货", "宏观与隔夜", "板块动向", "盘前异动", "今日关注"],
    }
}

/// Where the movers heading sits in `all_headings` and the recap's headings.
pub(crate) const MOVERS: usize = 3;
/// Where the themes heading sits in `all_headings` and the recap's headings.
pub(crate) const THEMES: usize = 2;
/// Where "What to watch" sits in `all_headings`.
const WATCH: usize = 4;

/// The section headings in order: futures and movers always; "Macro &
/// overnight", "Themes" and "What to watch" only when the facts have items
/// for them.
fn headings(lang: Lang, macro_news: bool, themes: bool, watch: bool) -> Vec<&'static str> {
    let [futures, macro_heading, themes_heading, movers, watch_heading] = all_headings(lang);
    [
        (futures, true),
        (macro_heading, macro_news),
        (themes_heading, themes),
        (movers, true),
        (watch_heading, watch),
    ]
    .into_iter()
    .filter_map(|(h, on)| on.then_some(h))
    .collect()
}

/// Whether an optional list has anything to write about.
fn has_items<T>(f: &Field<Vec<T>>) -> bool {
    matches!(f, Field::Value(v) if !v.is_empty())
}

/// The line appended to the user message naming the sections to write, so
/// the LLM never decides which optional sections exist.
pub(crate) fn sections_line(lang: Lang, want: &[&str]) -> String {
    let quoted: Vec<String> = want.iter().map(|h| format!("\"{h}\"")).collect();
    match lang {
        Lang::En => format!(
            "Write exactly these sections, in this order: {}",
            quoted.join(", ")
        ),
        Lang::Zh => format!("只写以下部分，按此顺序：{}", quoted.join("、")),
    }
}

/// `got` without the sections the facts do not call for: a heading in
/// `known` but not in `want` is dropped (never rendered, so never checked),
/// and the rest must then be exactly `want` in order.
pub(crate) fn match_sections(
    mut got: Vec<(String, String)>,
    want: &[&str],
    known: &[&str],
) -> Result<Vec<(String, String)>, String> {
    let seen: Vec<String> = got.iter().map(|(h, _)| h.clone()).collect();
    got.retain(|(h, _)| want.contains(&h.as_str()) || !known.contains(&h.as_str()));
    if got.len() != want.len() || got.iter().zip(want).any(|((h, _), w)| h != w) {
        return Err(format!(
            "sections must be exactly {want:?} in order, got {seen:?}"
        ));
    }
    Ok(got)
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

/// The Why cell of a mover with no verified reason and no fallback. Only the
/// EN tables have a Why column; the ZH text is reserved for a future one.
pub(crate) fn no_news(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "No clear news",
        Lang::Zh => "暂无明确消息",
    }
}

/// A mover's Why: its first verified reason (`summary_en`, in either
/// language), else code's fallback (`why::fill`), else `no_news`.
pub(crate) fn why(m: &Mover, lang: Lang) -> &str {
    let fallback = m.fallback.as_ref().map(|f| match lang {
        Lang::En => f.text_en.as_str(),
        Lang::Zh => f.text_zh.as_str(),
    });
    m.reasons
        .first()
        .map(|r| r.summary_en.as_str())
        .or(fallback)
        .unwrap_or(no_news(lang))
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

/// The code-rendered themes list, one paragraph per theme, so the figures
/// never pass through the LLM: "**Memory & storage** ▼ median −4.10%: MU
/// −4.12%, WDC −3.90%, and 2 more". Negatives carry U+2212, as on the web.
/// Paragraphs, not a table, since the front page's story trim drops tables.
pub fn render_themes(themes: &[Theme], lang: Lang) -> String {
    let lines: Vec<String> = themes
        .iter()
        .map(|t| {
            let arrow = match t.direction {
                Direction::Up => "▲",
                Direction::Down => "▼",
            };
            let median = fmt_signed_pct(t.median_pct);
            let members: Vec<String> = t
                .members
                .iter()
                .map(|m| {
                    format!(
                        "{} {}",
                        one_line_text(&m.ticker),
                        fmt_signed_pct(m.change_pct)
                    )
                })
                .collect();
            match lang {
                Lang::En => {
                    let more = if t.more > 0 {
                        format!(", and {} more", t.more)
                    } else {
                        String::new()
                    };
                    format!(
                        "**{}** {arrow} median {median}: {}{more}",
                        one_line_text(&t.group),
                        members.join(", ")
                    )
                }
                Lang::Zh => {
                    let more = if t.more > 0 {
                        format!("，另有 {} 只", t.more)
                    } else {
                        String::new()
                    };
                    format!(
                        "**{}** {arrow} 中位数 {median}：{}{more}",
                        one_line_text(&t.group_zh),
                        members.join("，")
                    )
                }
            }
        })
        .collect();
    lines.join("\n\n")
}

/// Code's labels on one line, so a stray newline cannot end the paragraph.
fn one_line_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The code-rendered "What to watch" list, one line per item, so times,
/// titles and links never pass through the LLM: "- **8:30 AM ET** October
/// CPI · [Schedule](url)"; once out, "· ✓ Released · [Release](url)", with
/// a CPI or jobs report's figures as "· ✓ Released: +0.4% on the month; core
/// +0.3% on the month · [Release](url)".
pub fn render_watch(items: &[WatchItem], lang: Lang) -> String {
    let zh = lang == Lang::Zh;
    let pick = |en: &'static str, z: &'static str| if zh { z } else { en };
    items
        .iter()
        .map(|i| {
            let (title, time, note) = if zh {
                (&i.title_zh, &i.time_zh, &i.note_zh)
            } else {
                (&i.title, &i.time_et, &i.note)
            };
            let when = match (i.kind, time) {
                (WatchKind::Earnings, _) => Some(pick("After the close", "盘后")),
                (_, Some(t)) => Some(t.as_str()),
                _ => None,
            };
            let mut line = String::from("- ");
            if let Some(w) = when {
                line.push_str(&format!("**{}** ", one_line_text(w)));
            }
            line.push_str(&one_line_text(title));
            if let Some(n) = note {
                line.push_str(&format!(" · {}", one_line_text(n)));
            }
            let released = i.status == "released";
            let moved = i
                .moved_from
                .as_deref()
                .and_then(|d| d.parse::<chrono::NaiveDate>().ok())
                .map(|d| {
                    if zh {
                        format!("由{}改期", d.format("%-m月%-d日"))
                    } else {
                        format!("Moved from {}", d.format("%b %-d"))
                    }
                });
            let flag = match (i.kind, i.status.as_str()) {
                (WatchKind::Event, "released") => Some(match figures_text(&i.figures, lang) {
                    Some(t) => format!(
                        "{}{}",
                        pick("✓ Released: ", "✓ 已发布："),
                        one_line_text(&t)
                    ),
                    None => pick("✓ Released", "✓ 已发布").into(),
                }),
                (WatchKind::Event, "postponed") => Some(pick("Postponed", "已推迟").into()),
                (WatchKind::Event, "moved") => moved,
                (WatchKind::Earnings, "unconfirmed") => {
                    Some(pick("date not confirmed", "日期未确认").into())
                }
                _ => None,
            };
            if let Some(f) = flag {
                line.push_str(&format!(" · {f}"));
            }
            if let Some(url) = &i.link {
                let label = if released {
                    pick("Release", "发布原文")
                } else if i.source_only {
                    pick("Source", "来源")
                } else {
                    pick("Schedule", "官方日程")
                };
                line.push_str(&format!(" · [{label}]({url})"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Theme labels and member names, whose digits ("Phillips 66") are not
/// figures. Theme members are not limited to the movers, so the prose may
/// name companies the movers tables do not.
pub(crate) fn theme_names(themes: &[Theme]) -> Vec<&str> {
    themes
        .iter()
        .flat_map(|t| {
            [t.group.as_str(), t.group_zh.as_str()]
                .into_iter()
                .chain(t.members.iter().map(|m| m.name.as_str()))
        })
        .collect()
}

/// The themes' "and N more" counts the prose may repeat. Counts up to 10
/// are the checker's small counts and pass already.
pub(crate) fn theme_allowed(themes: &[Theme]) -> Vec<f64> {
    themes
        .iter()
        .filter(|t| t.more > 10)
        .map(|t| t.more as f64)
        .collect()
}

/// One `## heading` section: code's block (themes list or movers table)
/// first when there is one, then the LLM body.
pub(crate) fn section(heading: &str, code: Option<&str>, body: &str) -> String {
    let parts: Vec<&str> = [code.map(str::trim_end).unwrap_or(""), body.trim()]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    format!("## {heading}\n\n{}\n\n", parts.join("\n\n"))
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
        row.push_str(&format!(" {} |", cell(why(m, lang))));
    }
    row
}

/// Every reason the brief cites (movers, macro, themes), in that order and
/// possibly repeated; the sources list and the stored sources dedupe it.
pub(crate) fn source_reasons(f: &PremarketFacts) -> impl Iterator<Item = &NewsReason> {
    let macro_items = match &f.macro_items {
        Field::Value(items) => items.as_slice(),
        Field::Missing { .. } => &[],
    };
    f.gainers
        .iter()
        .chain(&f.losers)
        .flat_map(|m| &m.reasons)
        .chain(macro_items)
        .chain(f.themes.iter().flat_map(|t| &t.reasons))
}

/// The movers table, or "" when there are no movers.
fn movers_table(f: &PremarketFacts, lang: Lang) -> String {
    let movers: Vec<&Mover> = f.gainers.iter().chain(&f.losers).collect();
    if movers.is_empty() {
        return String::new();
    }
    let mut out = String::from(match lang {
        Lang::En => "| Ticker | Company | Premarket | Change | Why |\n|---|---|---:|---:|---|\n",
        Lang::Zh => "| 代码 | 公司 | 盘前价 | 涨跌 |\n|---|---|---:|---:|\n",
    });
    for m in movers {
        out.push_str(&mover_row(m, lang));
        out.push('\n');
    }
    out
}

/// The full article: title, summary, futures table, then the sections in
/// order (the themes list heads "Themes", the movers table heads the movers
/// section), sources and disclaimer.
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

    let heads = all_headings(lang);
    let themes = render_themes(&f.themes, lang);
    let movers = movers_table(f, lang);
    let watch = match &f.watch_list {
        Field::Value(items) => render_watch(items, lang),
        Field::Missing { .. } => String::new(),
    };
    for (heading, body) in &d.sections {
        let code = if *heading == heads[THEMES] {
            Some(themes.as_str())
        } else if *heading == heads[MOVERS] {
            Some(movers.as_str())
        } else if *heading == heads[WATCH] {
            Some(watch.as_str())
        } else {
            None
        };
        out.push_str(&section(heading, code, body));
    }

    out.push_str(&render_sources(source_reasons(f), lang));

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

/// Phrases that narrate missing data instead of the market, as in the
/// 2026-10-30 brief. Matched case-insensitively in either language; kept
/// short so real prose passes.
const META: [&str; 15] = [
    "no reason was provided",
    "no reasons were provided",
    "no reason was given",
    "no reasons were given",
    "no news was found",
    "not provided in the facts",
    "not in the facts",
    "the facts do not",
    "the facts don't",
    "事实中",
    "未给出原因",
    "未提供原因",
    "原因不明",
    // The tail of a theme fallback (`why`), which the prose may not repeat.
    "no company news",
    "暂无个股消息",
];

/// Missing-data phrases that are real news in a macro section ("payrolls
/// data was unavailable due to the shutdown"), so they are only checked in
/// the title, the summary and the movers section.
const META_DATA: [&str; 4] = [
    "data is unavailable",
    "data was unavailable",
    "暂无数据",
    "数据缺失",
];

/// The movers section of the premarket brief or the recap, either language.
fn is_movers(heading: &str) -> bool {
    [Lang::En, Lang::Zh].into_iter().any(|l| {
        all_headings(l)[MOVERS] == heading || crate::recap::draft::headings(l)[MOVERS] == heading
    })
}

/// Rejects prose that comments on missing facts or data; `data` adds the
/// phrases that are only meta-commentary outside macro news.
fn reject_meta(what: &str, s: &str, data: bool) -> Result<(), String> {
    let lower = s.to_lowercase();
    let extra: &[&str] = if data { &META_DATA } else { &[] };
    match META.iter().chain(extra).find(|p| lower.contains(*p)) {
        Some(p) => Err(format!(
            "{what} comments on missing facts or data ({p:?}): rephrase without it"
        )),
        None => Ok(()),
    }
}

/// The page text with link targets removed: EDGAR URLs carry accession
/// digits and post slugs carry fiscal years, neither of them figures.
pub(crate) fn without_link_targets(md: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\]\([^)\s]*\)").expect("valid built-in regex"))
        .replace_all(md, "]")
        .into_owned()
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

fn parse(v: &Value, lang: Lang, want: &[&str], known: &[&str]) -> Result<Draft, String> {
    let title = one_line(v, "title")?;
    let summary = one_line(v, "summary")?;
    reject_meta("title", &title, true)?;
    reject_meta("summary", &summary, true)?;
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
    // A known section the facts do not call for is never rendered, so it is
    // dropped rather than retried.
    let got = match_sections(got, want, known)?;
    for (h, m) in &got {
        reject_markup(&format!("section {h:?}"), m)?;
        reject_meta(&format!("section {h:?}"), m, is_movers(h))?;
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
    known: &[&str],
    allowed: &[f64],
    names: &[&str],
    figure_free: (&[&str], &[f64]),
) -> Result<Draft, Problem> {
    let v = v.map_err(Problem::Llm)?;
    let d = parse(&v, lang, want, known).map_err(Problem::Malformed)?;
    let mut text = format!("{}\n{}", d.title, d.summary);
    for (h, m) in &d.sections {
        text.push_str(&format!("\n{h}\n{m}"));
    }
    let numbers = |bad: Vec<String>| {
        let mut seen = HashSet::new();
        Problem::Numbers(bad.into_iter().filter(|n| seen.insert(n.clone())).collect())
    };
    check(&blank_names(&text, names), lang, allowed).map_err(numbers)?;
    // Sections that may cite no figure from the facts (a consensus number in
    // a macro item is allowed elsewhere, never here): only `figure_free.1`;
    // times, dates and month labels are never figures.
    let (free, only) = figure_free;
    for (h, m) in d
        .sections
        .iter()
        .filter(|(h, _)| free.contains(&h.as_str()))
    {
        check(&blank_names(m, names), lang, only)
            .map_err(|bad| numbers(bad.into_iter().map(|n| format!("{h}: {n}")).collect()))?;
    }
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
    // Diagnostics and the theme tuning stats are operational counters, not
    // facts the brief may cite.
    let for_llm = PremarketFacts {
        diagnostics: vec![],
        theme_candidates: vec![],
        ..f.clone()
    };
    let facts_json = serde_json::to_string_pretty(&for_llm)
        .map_err(|e| vec![format!("draft: facts json: {e}")])?;
    let mut allowed = numbers_in_facts(f);
    allowed.extend(theme_allowed(&f.themes));
    // Company names come from the universe file, so their digits ("3M",
    // "Phillips 66") are not figures.
    let names: Vec<&str> = f
        .gainers
        .iter()
        .chain(&f.losers)
        .map(|m| m.name.as_str())
        .chain(theme_names(&f.themes))
        .chain(watch_names(&f.watch_list))
        .collect();
    // "What to watch" states no figure, never a consensus or a result: only
    // the trading year and its neighbours, and a released report's figures
    // and the numbers of their release sentences (the prior month's change).
    let figures = watch_figures(&f.watch_list);
    let mut watch_allowed = trading_years(f).to_vec();
    watch_allowed.extend(&figures);
    if let Field::Value(items) = &f.watch_list {
        for g in items.iter().flat_map(|i| &i.figures) {
            watch_allowed.extend(crate::facts::quote_numbers(&g.quote));
        }
    }
    let guard = |d: &Draft| {
        if figures.is_empty() {
            Ok(())
        } else {
            reject_forecast_talk(d, lang, &figures)
        }
    };
    write_with_guard(
        llm,
        prompt(lang),
        &facts_json,
        &allowed,
        &names,
        &headings(
            lang,
            has_items(&f.macro_items),
            !f.themes.is_empty(),
            has_items(&f.watch_list),
        ),
        &all_headings(lang),
        (&[all_headings(lang)[WATCH]], &watch_allowed),
        lang,
        &guard,
        |d| render_markdown(d, f),
    )
    .await
}

/// The values of the watch list's release figures.
pub(crate) fn watch_figures(w: &Field<Vec<WatchItem>>) -> Vec<f64> {
    match w {
        Field::Value(items) => items
            .iter()
            .flat_map(|i| i.figures.iter().map(|f| f.value))
            .collect(),
        Field::Missing { .. } => vec![],
    }
}

/// Words that put a sentence about the released CPI or jobs report
/// (lowercase).
const REPORT_WORDS: [&str; 18] = [
    "cpi",
    "consumer price",
    "inflation",
    "payrolls",
    "nonfarm",
    "non-farm",
    "jobs report",
    "jobs data",
    "employment report",
    "unemployment rate",
    "jobless rate",
    "消费者物价",
    "物价",
    "通胀",
    "非农",
    "就业报告",
    "就业数据",
    "失业率",
];
/// Market gauges, not forecasts of the figure: removed before the check.
const NOT_FORECASTS: [&str; 2] = ["inflation expectations", "通胀预期"];
/// English word starts that set a figure against forecasts, and words that
/// call it hot or cool.
const FORECAST_STEMS: [&str; 8] = [
    "expect",
    "unexpect",
    "forecast",
    "estimat",
    "anticipat",
    "surpris",
    "consensus",
    "economist",
];
const TEMPERATURE: [&str; 7] = [
    "hot", "hotter", "hottest", "cool", "cooler", "cooled", "cooling",
];
const FORECAST_ZH: [&str; 12] = [
    "预期",
    "预估",
    "预测",
    "共识",
    "经济学家",
    "意外",
    "偏热",
    "偏冷",
    "过热",
    "火热",
    "升温",
    "降温",
];

/// `text` cut into sentences: after `.`, `!` or `?` followed by a space,
/// a CJK character or the end, and at `。！？；` and line breaks.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let next = it.peek().map(|(_, n)| *n);
        let ends = match c {
            '。' | '！' | '？' | '；' | '\n' => true,
            '.' | '!' | '?' => next.is_none_or(|n| n.is_whitespace() || !n.is_ascii()),
            _ => false,
        };
        if ends {
            let end = i + c.len_utf8();
            out.push(&text[from..end]);
            from = end;
        }
    }
    out.push(&text[from..]);
    out.retain(|s| !s.trim().is_empty());
    out
}

/// Whether `sentence` is about the released report: it names the report,
/// or states one of its `figures` (a percent, or a count of thousands).
fn about_the_report(sentence: &str, lower: &str, figures: &[f64]) -> bool {
    REPORT_WORDS.iter().any(|w| lower.contains(w))
        || crate::numbers::extract(sentence, Lang::En)
            .iter()
            .filter(|n| n.is_percent || n.value.abs() >= 1000.0)
            .any(|n| {
                let half = 0.5 * 10f64.powi(-i32::from(n.decimals)) * n.multiplier;
                figures
                    .iter()
                    .any(|f| (n.value.abs() - f.abs()).abs() < half)
            })
}

/// The first forecast or temperature word in `lower`, if any.
fn forecast_word(lower: &str) -> Option<String> {
    let mut text = lower.to_string();
    for p in NOT_FORECASTS {
        text = text.replace(p, " ");
    }
    let en = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| FORECAST_STEMS.iter().any(|s| w.starts_with(s)) || TEMPERATURE.contains(w))
        .map(str::to_string);
    en.or_else(|| {
        FORECAST_ZH
            .iter()
            .find(|w| text.contains(*w))
            .map(|w| w.to_string())
    })
}

/// Rejects a draft whose sentences about a released report set its figures
/// against forecasts or call them hot or cool (the "never consensus" rule),
/// in the title, the summary, the macro section and "What to watch". Other
/// sentences ("revenue below estimates", "Cisco is expected to report") are
/// not checked. Plain string tests: no regex to compile on the Worker.
fn reject_forecast_talk(d: &Draft, lang: Lang, figures: &[f64]) -> Result<(), String> {
    let heads = all_headings(lang);
    let mut texts = vec![("title", d.title.as_str()), ("summary", d.summary.as_str())];
    for (h, m) in &d.sections {
        if *h == heads[1] || *h == heads[WATCH] {
            texts.push((h.as_str(), m.as_str()));
        }
    }
    for (what, text) in texts {
        for sentence in sentences(text) {
            let lower = sentence.to_lowercase();
            if !about_the_report(sentence, &lower, figures) {
                continue;
            }
            if let Some(w) = forecast_word(&lower) {
                return Err(format!(
                    "{what} compares a released figure with forecasts or calls it hot or cool ({w:?} in {:?}): state the figure only",
                    sentence.trim()
                ));
            }
        }
    }
    Ok(())
}

/// The trading year and its neighbours ("2026's last quarter").
fn trading_years(f: &PremarketFacts) -> [f64; 3] {
    let y = f64::from(chrono::Datelike::year(&f.trading_date));
    [y - 1.0, y, y + 1.0]
}

/// Watch-list titles and notes, whose digits ("Apple (AAPL)" aside, "3M
/// (MMM)", "Early close, 1:00 PM ET") are not figures.
fn watch_names(w: &Field<Vec<WatchItem>>) -> impl Iterator<Item = &str> {
    let items: &[WatchItem] = match w {
        Field::Value(v) => v,
        Field::Missing { .. } => &[],
    };
    items.iter().flat_map(|i| {
        [
            Some(&i.title),
            Some(&i.title_zh),
            i.note.as_ref(),
            i.note_zh.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
    })
}

/// The drafting loop shared by every content type: ask `llm` with `system`
/// and the facts JSON plus the `sections_line`, require exactly `headings`
/// (dropping any other section of `known`), reject markup, check every
/// number against `allowed` (ignoring the digits of known `names`; the
/// `figure_free.0` sections' against `figure_free.1` only), and
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
    known: &[&str],
    figure_free: (&[&str], &[f64]),
    lang: Lang,
    render: impl Fn(&Draft) -> String,
) -> Result<(Draft, String), Vec<String>> {
    write_with_guard(
        llm,
        system,
        facts_json,
        allowed,
        names,
        headings,
        known,
        figure_free,
        lang,
        &|_| Ok(()),
        render,
    )
    .await
}

/// `write_with`, also retrying a draft `guard` rejects.
#[allow(clippy::too_many_arguments)]
async fn write_with_guard(
    llm: &dyn LlmProvider,
    system: &str,
    facts_json: &str,
    allowed: &[f64],
    names: &[&str],
    headings: &[&str],
    known: &[&str],
    figure_free: (&[&str], &[f64]),
    lang: Lang,
    guard: &dyn Fn(&Draft) -> Result<(), String>,
    render: impl Fn(&Draft) -> String,
) -> Result<(Draft, String), Vec<String>> {
    let base = format!("{facts_json}\n\n{}", sections_line(lang, headings));
    let mut user = base.clone();
    let mut last = None;
    for _ in 0..ATTEMPTS {
        match attempt(
            llm.complete_json(system, &user).await,
            lang,
            headings,
            known,
            allowed,
            names,
            figure_free,
        )
        .and_then(|d| guard(&d).map(|()| d).map_err(Problem::Malformed))
        {
            Ok(d) => {
                let md = render(&d);
                return Ok((d, md));
            }
            Err(p) => {
                user = format!("{base}{}", retry_note(&p));
                last = Some(p);
            }
        }
    }
    Err(last.map(errors).unwrap_or_default())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::events::figures::{FigureKey, ReleaseFigure};
    use crate::facts::{Direction, Field, Mover, NewsReason, Quote, Theme, ThemeMember};
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
                search_failed: false,
                fallback: None,
            }],
            losers: vec![],
            macro_items: Field::Missing {
                reason: "none".into(),
            },
            watch_list: Field::Missing {
                reason: "not available in v1".into(),
            },
            themes: vec![],
            theme_candidates: vec![],
            diagnostics: vec![],
        }
    }

    fn reply(lang: Lang, body: &str) -> Result<Value, ProviderError> {
        reply_with(&headings(lang, true, false, false), body)
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
        // The facts have no macro news, so its section is dropped unread.
        assert_eq!(d.sections.len(), 2);
        assert_eq!(d.sections[0].0, "Futures");
        assert!(
            !md.contains("What to watch") && !md.contains("\nn/a\n"),
            "{md}"
        );
        assert_eq!(l.prompts.borrow().len(), 1);
        let (sys, user) = &l.prompts.borrow()[0];
        assert_eq!(sys, PROMPT_EN);
        assert_eq!(
            user,
            &format!(
                "{}\n\nWrite exactly these sections, in this order: \"Futures\", \"Biggest premarket movers\"",
                serde_json::to_string_pretty(&facts()).unwrap()
            )
        );
        assert!(md.starts_with("# Futures edge up; Apple jumps 8%"));
        assert!(md.contains(DISCLAIMER_EN));
    }

    #[tokio::test]
    async fn missing_watch_list_drops_its_section_in_both_languages() {
        for (lang, gone) in [(Lang::En, "What to watch"), (Lang::Zh, "今日关注")] {
            // Stray macro, themes and watch sections are dropped, not
            // retried, and the watch body (an unbacked number here) is never
            // checked.
            let all = reply_with(&all_headings(lang), "Up 7.5%.");
            let mut v = all.unwrap();
            for s in v["sections"].as_array_mut().unwrap().iter_mut().take(4) {
                s["markdown"] = json!("Quiet.");
            }
            let l = llm(vec![Ok(v)]);
            let (d, md) = write(&l, &facts(), lang).await.unwrap();
            assert_eq!(l.prompts.borrow().len(), 1);
            assert_eq!(d.sections.len(), 2);
            // No heading and no "n/a" body: missing futures stay as table cells.
            let na_body = format!("\n{}\n", na(lang));
            assert!(!md.contains(gone) && !md.contains(&na_body), "{md}");
        }
    }

    #[test]
    fn only_known_unwanted_sections_are_dropped() {
        let want = headings(Lang::En, true, false, false);
        // The order before macro moved above the movers.
        let swapped = [
            "Futures",
            "Biggest premarket movers",
            "Macro & overnight",
            "What to watch",
        ];
        let twice = [&want[..], &["Macro & overnight"]].concat();
        let unknown = [&want[..], &["Earnings"]].concat();
        for heads in [&swapped[..], &twice[..], &unknown[..]] {
            let v = reply_with(heads, "Quiet.").unwrap();
            let err = parse(&v, Lang::En, &want, &all_headings(Lang::En)).unwrap_err();
            assert!(
                err.starts_with("sections must be exactly"),
                "{heads:?}: {err}"
            );
        }
        // Every unwanted known section goes, wherever it sits.
        let want = headings(Lang::En, false, false, false);
        let six = [&all_headings(Lang::En)[..], &["What to watch"]].concat();
        let v = reply_with(&six, "Quiet.").unwrap();
        let d = parse(&v, Lang::En, &want, &all_headings(Lang::En)).unwrap();
        assert_eq!(d.sections.len(), 2);
        // Recap headings never drop anything.
        let recap = [
            "Index closes",
            "Notable movers",
            "After-hours earnings to watch",
        ];
        let v = reply_with(&[&recap[..], &["What to watch"]].concat(), "Quiet.").unwrap();
        assert!(parse(&v, Lang::En, &recap, &recap).is_err());
    }

    #[test]
    fn meta_commentary_about_missing_data_is_malformed() {
        for (lang, body) in [
            (
                Lang::En,
                "HP Inc. (HPQ) fell 13.26%; no reasons were provided.",
            ),
            (Lang::En, "Equifax rose. No news was found for it."),
            (Lang::En, "The facts do not explain the move."),
            (
                Lang::En,
                "Dell fell; the reason is not provided in the facts.",
            ),
            (Lang::Zh, "Equifax（EFX）涨4.35%，事实中未给出原因。"),
            (Lang::Zh, "Equifax（EFX）涨4.35%，原因不明。"),
            (Lang::Zh, "暂无数据。"),
            (
                Lang::En,
                "Lumentum fell with the networking & optical group; no company news found.",
            ),
            (Lang::Zh, "Lumentum随网络与光通信板块下跌，暂无个股消息。"),
        ] {
            let v = reply(lang, body).unwrap();
            let err = parse(
                &v,
                lang,
                &headings(lang, true, false, false),
                &all_headings(lang),
            )
            .unwrap_err();
            assert!(err.contains("missing facts or data"), "{body:?}: {err}");
            // The note does not steer the model toward any one section.
            assert!(!err.contains("mover"), "{err}");
        }
        let mut v = clean(Lang::En).unwrap();
        v["summary"] = json!("Futures fell; no reason was given for Dell.");
        assert!(parse(
            &v,
            Lang::En,
            &headings(Lang::En, true, false, false),
            &all_headings(Lang::En)
        )
        .is_err());
    }

    #[test]
    fn real_prose_passes_the_meta_commentary_check() {
        for (lang, body) in [
            (Lang::En, "Dell (DELL) and HP Inc. (HPQ) also fell."),
            (
                Lang::En,
                "In fact, the data showed reasons for optimism; no guidance was provided.",
            ),
            (
                Lang::Zh,
                "事实上，戴尔和惠普也下跌。Equifax 涨幅居前，暂无明确消息的个股不作展开。",
            ),
            // A theme fallback without its no-news tail (`why`).
            (
                Lang::En,
                "Lumentum fell with the networking & optical group (median −4.63%).",
            ),
            (
                Lang::Zh,
                "Lumentum随网络与光通信板块下跌（中位数 −4.63%）。",
            ),
        ] {
            let v = reply(lang, body).unwrap();
            assert!(
                parse(
                    &v,
                    lang,
                    &headings(lang, true, false, false),
                    &all_headings(lang)
                )
                .is_ok(),
                "{body:?}"
            );
        }
    }

    #[tokio::test]
    async fn meta_commentary_is_retried() {
        let l = llm(vec![
            reply(Lang::En, "Equifax rose; no reasons were provided."),
            clean(Lang::En),
        ]);
        assert!(write(&l, &facts(), Lang::En).await.is_ok());
        let p = l.prompts.borrow();
        assert!(p[1].1.contains("missing facts or data"), "{}", p[1].1);
    }

    /// A reply with `macro_body` in the macro section and "Quiet." elsewhere.
    fn macro_reply(lang: Lang, macro_body: &str) -> Value {
        let heads = headings(lang, true, false, false);
        let mut v = reply_with(&heads, "Quiet.").unwrap();
        v["sections"][1]["markdown"] = json!(macro_body);
        v
    }

    #[test]
    fn missing_official_data_is_real_macro_news() {
        for (lang, body) in [
            (
                Lang::En,
                "Official jobs data is unavailable because of the government shutdown.",
            ),
            (
                Lang::En,
                "September payrolls data was unavailable due to the shutdown.",
            ),
            (
                Lang::Zh,
                "因政府停摆，官方就业数据缺失，部分经济指标暂无数据。",
            ),
        ] {
            assert!(
                parse(
                    &macro_reply(lang, body),
                    lang,
                    &headings(lang, true, false, false),
                    &all_headings(lang)
                )
                .is_ok(),
                "{body:?}"
            );
        }
    }

    #[test]
    fn data_phrases_still_fail_in_movers_title_and_summary() {
        for (lang, phrase) in [
            (Lang::En, "Data is unavailable for Dell."),
            (Lang::Zh, "戴尔暂无数据。"),
        ] {
            let mut movers = macro_reply(lang, "Quiet.");
            movers["sections"][2]["markdown"] = json!(phrase);
            let mut summary = macro_reply(lang, "Quiet.");
            summary["summary"] = json!(phrase);
            let mut title = macro_reply(lang, "Quiet.");
            title["title"] = json!(phrase);
            for v in [movers, summary, title] {
                assert!(
                    parse(
                        &v,
                        lang,
                        &headings(lang, true, false, false),
                        &all_headings(lang)
                    )
                    .is_err(),
                    "{v}"
                );
            }
        }
        // The recap's movers section is covered too.
        let recap = crate::recap::draft::headings(Lang::En);
        let mut v = reply_with(&recap, "Quiet.").unwrap();
        v["sections"][3]["markdown"] = json!("Data was unavailable for Dell.");
        assert!(parse(&v, Lang::En, &recap, &recap).is_err());
        v["sections"][3]["markdown"] = json!("Quiet.");
        v["sections"][4]["markdown"] = json!("Data was unavailable for Dell.");
        assert!(parse(&v, Lang::En, &recap, &recap).is_ok());
    }

    #[tokio::test]
    async fn present_watch_list_requires_and_renders_its_section() {
        let mut f = facts();
        f.watch_list = Field::Value(vec![cpi_item("scheduled", None)]);
        for (lang, heading, line) in [
            (Lang::En, "What to watch", "- **8:30 AM ET** October CPI"),
            (
                Lang::Zh,
                "今日关注",
                "- **上午8:30（美东）** 10月消费者物价指数（CPI）",
            ),
        ] {
            let l = llm(vec![
                reply_with(&headings(lang, true, false, false), "Quiet."),
                reply_with(&all_headings(lang), "Quiet."),
            ]);
            let (d, md) = write(&l, &f, lang).await.unwrap();
            assert_eq!(l.prompts.borrow().len(), 2);
            // Futures, movers and watch: the facts have no macro news.
            assert_eq!(d.sections.len(), 3);
            // The code list comes first, then the LLM's text.
            assert!(md.contains(&format!("## {heading}\n\n{line}")), "{md}");
            assert!(md.contains("\n\nQuiet.\n\n"), "{md}");
        }
    }

    #[tokio::test]
    async fn what_to_watch_may_not_cite_a_consensus_figure_from_the_facts() {
        let mut f = facts();
        f.macro_items = Field::Value(vec![NewsReason {
            quote: "Economists expect CPI rose 0.3% in October".into(),
            ..reason("https://news.test/cpi", "CPI preview")
        }]);
        f.watch_list = Field::Value(vec![cpi_item("scheduled", None)]);
        let heads = headings(Lang::En, true, false, true);
        let with_watch = |watch: &str| {
            Ok(json!({
                "title": "Futures edge up",
                "summary": "Economists expect CPI rose 0.3%.",
                "sections": heads.iter().map(|h| {
                    let body = if *h == "What to watch" { watch } else { "Quiet." };
                    json!({"heading": h, "markdown": body})
                }).collect::<Vec<_>>(),
            }))
        };
        let l = llm(vec![
            with_watch("Economists expect a 0.3% rise at 8:30 AM ET."),
            with_watch("CPI lands at 8:30 AM ET, before the open in 2026's last quarter."),
        ]);
        let (d, _) = write(&l, &f, Lang::En).await.unwrap();
        // The macro summary may cite the quoted 0.3%; What to watch may not.
        assert_eq!(l.prompts.borrow().len(), 2);
        assert!(
            l.prompts.borrow()[1].1.contains("What to watch: 0.3"),
            "{}",
            l.prompts.borrow()[1].1
        );
        assert!(d
            .sections
            .iter()
            .any(|(h, m)| h == "What to watch" && m.contains("8:30 AM ET")));
    }

    #[test]
    fn render_watch_says_moved_and_links_an_election_to_its_source() {
        let moved = WatchItem {
            status: "moved".into(),
            moved_from: Some("2026-11-05".into()),
            ..cpi_item("moved", None)
        };
        let law = "https://www.govinfo.gov/content/pkg/USCODE-2023-title3/html/USCODE-2023-title3-chap1-sec21.htm";
        let election = WatchItem {
            id: "us:election:2028-11-07".into(),
            time_et: None,
            time_zh: None,
            title: "US general election".into(),
            title_zh: "美国大选".into(),
            link: Some(law.into()),
            source_only: true,
            ..cpi_item("scheduled", None)
        };
        let items = [election, moved];
        let en = render_watch(&items, Lang::En);
        assert!(
            en.starts_with(&format!("- US general election · [Source]({law})")),
            "{en}"
        );
        assert!(
            en.contains("October CPI · Moved from Nov 5 · [Schedule]"),
            "{en}"
        );
        let zh = render_watch(&items, Lang::Zh);
        assert!(zh.contains("美国大选 · [来源]"), "{zh}");
        assert!(zh.contains("由11月5日改期"), "{zh}");
    }

    #[test]
    fn facts_stored_before_the_calendar_still_parse() {
        // The stub every brief before 2026-11-06 stored.
        let mut v = serde_json::to_value(facts()).unwrap();
        v["watch_list"] = json!({"Missing": {"reason": "not available in v1"}});
        let f: PremarketFacts = serde_json::from_value(v).unwrap();
        assert!(matches!(f.watch_list, Field::Missing { .. }));
        // And an item without the optional fields.
        let item = json!({"kind": "event", "id": "bls:cpi:2026-10", "time_et": null, "time_zh": null,
            "title": "October CPI", "title_zh": "10月CPI", "status": "scheduled"});
        let w: WatchItem = serde_json::from_value(item).unwrap();
        assert!(!w.source_only && w.moved_from.is_none() && w.link.is_none());
    }

    #[tokio::test]
    async fn what_to_watch_in_chinese_takes_times_and_months_but_no_figures() {
        let mut f = facts();
        f.macro_items = Field::Value(vec![NewsReason {
            quote: "Economists expect CPI rose 0.3% in October".into(),
            ..reason("https://news.test/cpi", "CPI preview")
        }]);
        f.watch_list = Field::Value(vec![cpi_item("scheduled", None)]);
        let heads = headings(Lang::Zh, true, false, true);
        let with_watch = |watch: &str| {
            Ok(json!({
                "title": "期货走稳",
                "summary": "市场等待CPI。",
                "sections": heads.iter().map(|h| {
                    let body = if *h == "今日关注" { watch } else { "平静。" };
                    json!({"heading": h, "markdown": body})
                }).collect::<Vec<_>>(),
            }))
        };
        let l = llm(vec![
            with_watch("市场预期10月CPI上涨0.3%。"),
            with_watch("10月CPI将于上午8:30（美东）公布，是开盘前的关键数据。"),
        ]);
        let (d, _) = write(&l, &f, Lang::Zh).await.unwrap();
        assert_eq!(l.prompts.borrow().len(), 2);
        assert!(
            l.prompts.borrow()[1].1.contains("今日关注: 0.3"),
            "{}",
            l.prompts.borrow()[1].1
        );
        assert!(d
            .sections
            .iter()
            .any(|(h, m)| h == "今日关注" && m.contains("上午8:30（美东）")));
    }

    #[test]
    fn figure_free_sections_allow_only_the_trading_years() {
        // 2000.0 is a figure in the facts (a level), not a year to allow here.
        let v = json!({"title": "T", "summary": "S", "sections": [
            {"heading": "A", "markdown": "Futures near 2000 points."}]});
        let p = attempt(
            Ok(v),
            Lang::En,
            &["A"],
            &["A"],
            &[2000.0],
            &[],
            (&["A"], &[2025.0, 2026.0, 2027.0]),
        );
        assert!(matches!(p, Err(Problem::Numbers(_))));
    }

    fn cpi_item(status: &str, release: Option<&str>) -> WatchItem {
        WatchItem {
            kind: WatchKind::Event,
            id: "bls:cpi:2026-10".into(),
            time_et: Some("8:30 AM ET".into()),
            time_zh: Some("上午8:30（美东）".into()),
            title: "October CPI".into(),
            title_zh: "10月消费者物价指数（CPI）".into(),
            note: None,
            note_zh: None,
            status: status.into(),
            link: Some(
                release
                    .unwrap_or("https://www.bls.gov/schedule/news_release/cpi.htm")
                    .into(),
            ),
            source_only: false,
            moved_from: None,
            figures: vec![],
        }
    }

    #[test]
    fn render_watch_lists_times_titles_status_and_links() {
        let url = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
        let fomc = WatchItem {
            id: "fed:fomc:2026-12".into(),
            time_et: Some("2:00 PM ET".into()),
            time_zh: Some("下午2:00（美东）".into()),
            title: "FOMC rate decision (with projections)".into(),
            title_zh: "联邦公开市场委员会（FOMC）议息会议（含经济预测）".into(),
            note: Some("Press conference 2:30 PM ET".into()),
            note_zh: Some("美联储主席新闻发布会，下午2:30（美东）".into()),
            link: None,
            ..cpi_item("postponed", None)
        };
        let aapl = WatchItem {
            kind: WatchKind::Earnings,
            id: "earnings:AAPL".into(),
            time_et: None,
            time_zh: None,
            title: "Apple (AAPL)".into(),
            title_zh: "Apple (AAPL)".into(),
            status: "unconfirmed".into(),
            link: None,
            ..cpi_item("x", None)
        };
        let items = [cpi_item("released", Some(url)), fomc, aapl];
        assert_eq!(
            render_watch(&items, Lang::En),
            format!(
                "- **8:30 AM ET** October CPI · ✓ Released · [Release]({url})\n\
                 - **2:00 PM ET** FOMC rate decision (with projections) · Press conference 2:30 PM ET · Postponed\n\
                 - **After the close** Apple (AAPL) · date not confirmed"
            )
        );
        assert_eq!(
            render_watch(&items, Lang::Zh),
            format!(
                "- **上午8:30（美东）** 10月消费者物价指数（CPI） · ✓ 已发布 · [发布原文]({url})\n\
                 - **下午2:00（美东）** 联邦公开市场委员会（FOMC）议息会议（含经济预测） · 美联储主席新闻发布会，下午2:30（美东） · 已推迟\n\
                 - **盘后** Apple (AAPL) · 日期未确认"
            )
        );
    }

    fn figure(key: FigureKey, value: f64) -> ReleaseFigure {
        ReleaseFigure {
            key,
            value,
            quote: "The release's sentence.".into(),
        }
    }

    /// Today's CPI after its release, with the figures the October 2026
    /// page states.
    fn released_cpi() -> WatchItem {
        WatchItem {
            figures: vec![
                ReleaseFigure {
                    quote: "The Consumer Price Index for All Urban Consumers (CPI-U) increased 0.4 \
                            percent on a seasonally adjusted basis in October after rising 0.6 \
                            percent in September, the U.S. Bureau of Labor Statistics reported today."
                        .into(),
                    ..figure(FigureKey::CpiMonth, 0.4)
                },
                figure(FigureKey::CoreMonth, 0.3),
            ],
            ..cpi_item(
                "released",
                Some("https://www.bls.gov/news.release/archives/cpi_11102026.htm"),
            )
        }
    }

    #[test]
    fn render_watch_states_a_released_reports_figures() {
        let url = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
        let full = WatchItem {
            figures: vec![
                figure(FigureKey::CpiMonth, 0.3),
                figure(FigureKey::CpiYear, 3.0),
                figure(FigureKey::CoreMonth, 0.2),
                figure(FigureKey::CoreYear, 3.0),
            ],
            ..released_cpi()
        };
        let jobs = WatchItem {
            id: "bls:jobs:2026-09".into(),
            title: "September jobs report".into(),
            title_zh: "9月非农就业报告".into(),
            figures: vec![
                figure(FigureKey::Payrolls, 119_000.0),
                figure(FigureKey::UnemploymentRate, 4.4),
            ],
            ..released_cpi()
        };
        let cases = [
            (
                released_cpi(),
                Lang::En,
                format!("- **8:30 AM ET** October CPI · ✓ Released: +0.4% on the month; core +0.3% on the month · [Release]({url})"),
            ),
            (
                released_cpi(),
                Lang::Zh,
                format!("- **上午8:30（美东）** 10月消费者物价指数（CPI） · ✓ 已发布：环比 +0.4%；核心环比 +0.3% · [发布原文]({url})"),
            ),
            (
                full.clone(),
                Lang::En,
                format!("- **8:30 AM ET** October CPI · ✓ Released: +0.3% on the month, +3.0% from a year earlier; core +0.2% on the month, +3.0% from a year earlier · [Release]({url})"),
            ),
            (
                full,
                Lang::Zh,
                format!("- **上午8:30（美东）** 10月消费者物价指数（CPI） · ✓ 已发布：环比 +0.3%，同比 +3.0%；核心环比 +0.2%，同比 +3.0% · [发布原文]({url})"),
            ),
            (
                jobs.clone(),
                Lang::En,
                format!("- **8:30 AM ET** September jobs report · ✓ Released: Payrolls +119,000; unemployment rate 4.4% · [Release]({url})"),
            ),
            (
                jobs,
                Lang::Zh,
                format!("- **上午8:30（美东）** 9月非农就业报告 · ✓ 已发布：非农就业 +11.9万人；失业率 4.4% · [发布原文]({url})"),
            ),
        ];
        for (item, lang, want) in cases {
            let line = render_watch(std::slice::from_ref(&item), lang);
            assert_eq!(line, want);
            // Every figure on the page traces to the facts.
            let allowed = watch_figures(&Field::Value(vec![item]));
            assert_eq!(
                check(&without_link_targets(&line), lang, &allowed),
                Ok(()),
                "{line}"
            );
        }
    }

    #[tokio::test]
    async fn released_figures_may_be_stated_but_never_set_against_forecasts() {
        let mut f = facts();
        f.watch_list = Field::Value(vec![released_cpi()]);
        let heads = headings(Lang::En, false, false, true);
        let draft = |summary: &str, watch: &str| {
            Ok(json!({
                "title": "Futures steady after CPI",
                "summary": summary,
                "sections": heads.iter().map(|h| {
                    let body = if *h == "What to watch" { watch } else { "Quiet." };
                    json!({"heading": h, "markdown": body})
                }).collect::<Vec<_>>(),
            }))
        };
        let l = llm(vec![
            draft(
                "Consumer prices rose 0.4% in October, hotter than expected.",
                "Core prices rose 0.3%.",
            ),
            draft(
                "Consumer prices rose 0.4% in October; core prices rose 0.3%.",
                "October CPI rose 0.4% on the month and core 0.3%, setting the tone before the open.",
            ),
        ]);
        let (d, md) = write(&l, &f, Lang::En).await.unwrap();
        let retry = l.prompts.borrow()[1].1.clone();
        assert!(
            retry.contains("summary compares a released figure with forecasts"),
            "{retry}"
        );
        assert!(d.summary.starts_with("Consumer prices rose 0.4%"));
        assert!(md.contains("✓ Released: +0.4% on the month"), "{md}");

        // Each wording is refused, in either language and in "What to watch".
        for (lang, summary, watch) in [
            (Lang::En, "CPI rose 0.4%, above expectations.", "Quiet."),
            (Lang::En, "CPI rose 0.4%.", "Economists had expected 0.3%."),
            (Lang::En, "A hot CPI print: prices rose 0.4%.", "Quiet."),
            (
                Lang::En,
                "CPI rose 0.4%, in line with the consensus.",
                "Quiet.",
            ),
            (Lang::Zh, "10月CPI环比上涨0.4%，高于市场预期。", "平静。"),
            (Lang::Zh, "10月CPI环比上涨0.4%。", "此前市场预期为0.3%。"),
            (Lang::Zh, "通胀明显升温，CPI环比上涨0.4%。", "平静。"),
            (Lang::En, "CPI came in hotter than expected.", "Quiet."),
            (
                Lang::En,
                "CPI rose 0.4%.",
                "Economists had expected CPI to rise 0.3%.",
            ),
            (Lang::Zh, "高于预期的CPI压制期货。", "平静。"),
        ] {
            let heads = headings(lang, false, false, true);
            let watch_head = all_headings(lang)[WATCH];
            let reply = || {
                Ok(json!({
                    "title": "T",
                    "summary": summary,
                    "sections": heads.iter().map(|h| {
                        let body = if *h == watch_head { watch } else { "Quiet." };
                        json!({"heading": h, "markdown": body})
                    }).collect::<Vec<_>>(),
                }))
            };
            let l = llm(vec![reply(), reply()]);
            let err = write(&l, &f, lang).await.unwrap_err();
            assert!(
                err.iter().any(|e| e.contains("forecasts")),
                "{summary} / {watch}: {err:?}"
            );
        }

        // Without figures the guard is off: a preview may say what is expected
        // outside "What to watch", as before.
        let mut f = facts();
        f.watch_list = Field::Value(vec![cpi_item("scheduled", None)]);
        let mut v = reply_with(&headings(Lang::En, false, false, true), "Quiet.").unwrap();
        v["summary"] = json!("Futures edge up as traders await a CPI report expected to cool.");
        assert!(write(&llm(vec![Ok(v)]), &f, Lang::En).await.is_ok());
    }

    #[test]
    fn the_forecast_guard_checks_only_sentences_about_the_report() {
        let draft = |lang: Lang, summary: &str, macro_text: &str, watch: &str| {
            let heads = all_headings(lang);
            Draft {
                lang,
                title: "T".into(),
                summary: summary.into(),
                sections: vec![
                    (heads[1].into(), macro_text.into()),
                    (heads[WATCH].into(), watch.into()),
                ],
            }
        };
        let cpi = [0.4, 0.3];
        // Forecast wording about other things, on a CPI day: the review's cases.
        for (lang, summary, macro_text, watch) in [
            (
                Lang::En,
                "Intel slides 8% after forecasting fourth-quarter revenue below estimates.",
                "China's October exports grew faster than expected.",
                "Cisco is expected to report after the close.",
            ),
            (
                Lang::En,
                "Datadog jumps 12% after results topped Wall Street estimates.",
                "Fed officials have reached no consensus on a December cut.",
                "October CPI rose 0.4% on the month. Cisco is expected to report after the close.",
            ),
            (
                Lang::En,
                "Futures rise as AI stocks stay hot.",
                "Inflation expectations eased as CPI rose 0.4%.",
                "Core prices rose 0.3%.",
            ),
            (
                Lang::Zh,
                "英特尔营收指引低于预期，盘前下跌8%。",
                "中国10月出口好于预期。",
                "10月CPI环比上涨0.4%。思科预计盘后公布财报。",
            ),
        ] {
            let d = draft(lang, summary, macro_text, watch);
            assert_eq!(reject_forecast_talk(&d, lang, &cpi), Ok(()), "{summary}");
        }
        // About the report, by name or by figure.
        for (lang, text, figures) in [
            (Lang::En, "CPI came in hotter than expected.", &cpi[..]),
            (
                Lang::En,
                "Consumer prices rose 0.4%, above forecasts.",
                &cpi,
            ),
            (Lang::En, "Economists had expected CPI to rise 0.3%.", &cpi),
            (Lang::En, "Economists had expected a 0.3% rise.", &cpi),
            (Lang::En, "Prices rose 0.4%, a surprise.", &cpi),
            (
                Lang::En,
                "Payrolls rose by 119,000, above estimates.",
                &[119_000.0, 4.4],
            ),
            (Lang::Zh, "高于预期的CPI压制期货。", &cpi),
            (
                Lang::Zh,
                "非农就业增加11.9万人，好于预期。",
                &[119_000.0, 4.4],
            ),
            (Lang::Zh, "通胀降温。", &cpi),
        ] {
            for d in [
                draft(lang, text, "", ""),
                draft(lang, "S", text, ""),
                draft(lang, "S", "", text),
            ] {
                let err = reject_forecast_talk(&d, lang, figures).unwrap_err();
                assert!(err.contains("compares a released figure"), "{err}");
            }
        }
        assert_eq!(
            sentences("CPI rose 0.4% in the U.S. today. Cisco reports later!下一句。最后"),
            [
                "CPI rose 0.4% in the U.S.",
                " today.",
                " Cisco reports later!",
                "下一句。",
                "最后"
            ]
        );
    }

    #[tokio::test]
    async fn what_to_watch_may_cite_the_prior_month_from_the_release_sentence() {
        let mut f = facts();
        f.watch_list = Field::Value(vec![released_cpi()]);
        let mut v = reply_with(&headings(Lang::En, false, false, true), "Quiet.").unwrap();
        v["sections"][2]["markdown"] =
            json!("October CPI rose 0.4% on the month after 0.6% in September; core rose 0.3%.");
        let l = llm(vec![Ok(v)]);
        let (d, _) = write(&l, &f, Lang::En).await.unwrap();
        assert_eq!(l.prompts.borrow().len(), 1);
        assert!(d.sections[2].1.contains("0.6%"));
    }

    #[tokio::test]
    async fn missing_macro_news_renders_no_macro_section() {
        for (lang, heading) in [(Lang::En, "Macro & overnight"), (Lang::Zh, "宏观与隔夜")] {
            for macro_items in [
                Field::Missing {
                    reason: "none".into(),
                },
                Field::Value(vec![]),
            ] {
                let f = PremarketFacts {
                    macro_items,
                    ..facts()
                };
                // The model still writes the section, as "n/a": dropped unread.
                let mut v = reply_with(&headings(lang, true, false, false), "Quiet.").unwrap();
                v["sections"][1]["markdown"] = json!(na(lang));
                let l = llm(vec![Ok(v)]);
                let (d, md) = write(&l, &f, lang).await.unwrap();
                assert_eq!(l.prompts.borrow().len(), 1);
                assert_eq!(d.sections.len(), 2);
                let na_body = format!("\n{}\n", na(lang));
                assert!(
                    !md.contains(&format!("## {heading}")) && !md.contains(&na_body),
                    "{md}"
                );
                let user = &l.prompts.borrow()[0].1;
                assert!(!user.ends_with(&format!("\"{heading}\"")), "{user}");
            }
        }
    }

    #[tokio::test]
    async fn macro_news_requires_and_renders_its_section() {
        let mut f = facts();
        f.macro_items = Field::Value(vec![reason("https://news.test/m", "Futures slip")]);
        for (lang, line) in [
            (
                Lang::En,
                "Write exactly these sections, in this order: \"Futures\", \"Macro & overnight\", \"Biggest premarket movers\"",
            ),
            (Lang::Zh, "只写以下部分，按此顺序：\"期货\"、\"宏观与隔夜\"、\"盘前异动\""),
        ] {
            let l = llm(vec![
                reply_with(&headings(lang, false, false, false), "Quiet."),
                reply_with(&headings(lang, true, false, false), "Quiet."),
            ]);
            let (d, md) = write(&l, &f, lang).await.unwrap();
            let p = l.prompts.borrow();
            assert_eq!(p.len(), 2);
            assert!(p[0].1.ends_with(line), "{}", p[0].1);
            assert!(p[1].1.contains(&format!("{line}\n\nYour previous reply")), "{}", p[1].1);
            assert_eq!(d.sections.len(), 3);
            let heading = all_headings(lang)[1];
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
            sections: headings(lang, true, false, false)
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
    fn mover_without_reason_reads_no_clear_news() {
        let mut f = facts();
        f.gainers[0].reasons = vec![];
        let en = render_markdown(&draft(Lang::En), &f);
        assert!(
            en.contains("| AAPL | Apple | 230.50 | +8.00% | No clear news |"),
            "{en}"
        );
        // The ZH table has no Why column, so it shows the move only.
        let zh = render_markdown(&draft(Lang::Zh), &f);
        assert!(zh.contains("| AAPL | Apple | 230.50 | +8.00% |\n"), "{zh}");
    }

    #[tokio::test]
    async fn mover_fallback_fills_the_en_why_cell_and_reaches_the_llm() {
        let mut f = facts();
        f.gainers[0].reasons = vec![];
        let group = crate::premarket::group("AAPL").unwrap();
        f.themes = vec![Theme {
            group: group.into(),
            group_zh: crate::premarket::group_zh(group).into(),
            direction: Direction::Up,
            members: vec![ThemeMember {
                ticker: "MSFT".into(),
                name: "Microsoft".into(),
                change_pct: 3.2,
            }],
            more: 2,
            median_pct: 2.95,
            reasons: vec![],
        }];
        crate::why::fill(
            f.gainers.iter_mut(),
            &f.themes,
            &[],
            f.trading_date,
            crate::cover::Session::Premarket,
        );
        let want = format!(
            "Rose with the {} group (median +2.95%); no company news found",
            group.to_lowercase()
        );
        let en = render_markdown(&draft(Lang::En), &f);
        assert!(
            en.contains(&format!("| AAPL | Apple | 230.50 | +8.00% | {want} |")),
            "{en}"
        );
        // The ZH table keeps no Why column.
        let zh = render_markdown(&draft(Lang::Zh), &f);
        assert!(zh.contains("| AAPL | Apple | 230.50 | +8.00% |\n"), "{zh}");
        // The LLM reads it in the facts, and may state it with the median.
        let body = "Apple rose 8%, in line with its group, median 2.95%.";
        let heads = headings(Lang::En, false, true, false);
        let l = llm(vec![reply_with(&heads, body)]);
        write(&l, &f, Lang::En).await.unwrap();
        let user = &l.prompts.borrow()[0].1;
        assert!(
            user.contains(&want) && user.contains("\"kind\": \"theme\""),
            "{user}"
        );
        // A verified reason wins over the fallback.
        f.gainers[0].reasons = vec![reason("https://news.test/aapl", "Apple jumps")];
        let en = render_markdown(&draft(Lang::En), &f);
        assert!(en.contains("| +8.00% | Record iPhone sales. |"), "{en}");
    }

    #[test]
    fn prompts_take_sections_from_the_user_message() {
        assert!(
            PROMPT_EN.contains("listed at the end of the user message")
                && PROMPT_EN
                    .contains("\"Macro & overnight\" exists only when `macro_items` has items")
                && !PROMPT_EN.contains("n/a"),
            "{PROMPT_EN}"
        );
        assert!(
            PROMPT_ZH.contains("列在用户消息末尾")
                && PROMPT_ZH.contains("只有 `macro_items` 有条目时才有“宏观与隔夜”")
                && !PROMPT_ZH.contains("暂缺"),
            "{PROMPT_ZH}"
        );
    }

    #[test]
    fn prompts_forbid_narrating_missing_reasons() {
        assert!(
            PROMPT_EN.contains("no reasons were provided"),
            "{PROMPT_EN}"
        );
        assert!(PROMPT_ZH.contains("事实中未给出原因"), "{PROMPT_ZH}");
    }

    #[test]
    fn prompts_state_a_fallback_without_making_it_a_cause() {
        assert_fallback_rule(PROMPT_EN, PROMPT_ZH);
    }

    /// Both prompts let the prose state a mover's fallback, never as a cause.
    pub(crate) fn assert_fallback_rule(en: &str, zh: &str) {
        assert!(
            en.contains("a `fallback`, which code wrote from the facts")
                && en.contains("Never turn it into a cause")
                && en.contains("do not write \"no company news\""),
            "{en}"
        );
        assert!(
            zh.contains("可能带有 `fallback`") && zh.contains("不得把它写成原因"),
            "{zh}"
        );
    }

    #[test]
    fn prompts_lead_the_summary_with_a_market_wide_story() {
        assert!(
            PROMPT_EN.contains(
                "If the macro items or a theme describe a market-wide story, the first sentence of the summary names it."
            ),
            "{PROMPT_EN}"
        );
        assert!(
            PROMPT_ZH
                .contains("如果宏观条目或某个主题讲的是影响整个市场的事件，摘要的第一句要点明它。"),
            "{PROMPT_ZH}"
        );
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
            search_failed: false,
            fallback: None,
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
            "sections": headings(Lang::En, true, false, false).iter().map(|h| json!({"heading": h, "markdown": "Quiet."})).collect::<Vec<_>>(),
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
                parse(
                    &v,
                    Lang::En,
                    &headings(Lang::En, true, false, false),
                    &all_headings(Lang::En)
                )
                .is_err(),
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
                parse(
                    &v,
                    Lang::En,
                    &headings(Lang::En, true, false, false),
                    &all_headings(Lang::En)
                )
                .is_ok(),
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
                parse(
                    &v,
                    Lang::En,
                    &headings(Lang::En, true, false, false),
                    &all_headings(Lang::En)
                )
                .is_err(),
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
            &["A", "B"],
            (&[], &[]),
            Lang::En,
            |d: &Draft| format!("rendered {}", d.sections.len()),
        )
        .await
        .unwrap();
        assert_eq!(md, "rendered 2");
        assert_eq!(d.sections[1].0, "B");
        let p = l.prompts.borrow();
        assert_eq!(p.len(), 2);
        let user = "{}\n\nWrite exactly these sections, in this order: \"A\", \"B\"";
        assert_eq!(p[0], ("SYS".to_string(), user.to_string()));
        // The retry note follows the section list.
        assert!(p[1].1.starts_with(user) && p[1].1.contains("Your reply was invalid"));
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

    fn member(ticker: &str, name: &str, change_pct: f64) -> ThemeMember {
        ThemeMember {
            ticker: ticker.into(),
            name: name.into(),
            change_pct,
        }
    }
    /// Two themes: chips up with a reason, memory down without one and with
    /// more members than it lists.
    pub(crate) fn two_themes() -> Vec<Theme> {
        vec![
            Theme {
                group: "Semiconductors".into(),
                group_zh: "半导体".into(),
                direction: Direction::Up,
                members: vec![member("NVDA", "Nvidia", 3.03), member("AMD", "AMD", 2.5)],
                more: 0,
                median_pct: 2.77,
                reasons: vec![NewsReason {
                    url: "https://news.test/chips".into(),
                    title: "Chip stocks rally".into(),
                    quote: "Chip stocks rose as much as 3% on AI demand".into(),
                    summary_en: "AI demand.".into(),
                }],
            },
            Theme {
                group: "Memory & storage".into(),
                group_zh: "存储芯片与存储设备".into(),
                direction: Direction::Down,
                members: vec![
                    member("MU", "Micron", -4.12),
                    member("WDC", "Western Digital", -3.9),
                ],
                more: 2,
                median_pct: -4.1,
                reasons: vec![],
            },
        ]
    }

    #[test]
    fn headings_put_macro_then_themes_then_movers() {
        assert_eq!(
            headings(Lang::En, true, true, true),
            [
                "Futures",
                "Macro & overnight",
                "Themes",
                "Biggest premarket movers",
                "What to watch"
            ]
        );
        assert_eq!(
            headings(Lang::Zh, true, true, true),
            ["期货", "宏观与隔夜", "板块动向", "盘前异动", "今日关注"]
        );
        assert_eq!(headings(Lang::Zh, true, true, true), all_headings(Lang::Zh));
        assert_eq!(
            headings(Lang::En, false, true, false),
            ["Futures", "Themes", "Biggest premarket movers"]
        );
        // The movers check follows the movers heading to its new place.
        assert!(is_movers("Biggest premarket movers") && is_movers("盘前异动"));
        assert!(is_movers("Notable movers") && is_movers("个股异动"));
        assert!(!is_movers("Themes") && !is_movers("Macro & overnight"));
    }

    #[tokio::test]
    async fn themes_section_exists_only_with_themes() {
        for (lang, heading) in [(Lang::En, "Themes"), (Lang::Zh, "板块动向")] {
            // No themes: a stray Themes section is dropped unread.
            let l = llm(vec![reply_with(&all_headings(lang), "Quiet.")]);
            let (d, md) = write(&l, &facts(), lang).await.unwrap();
            assert_eq!(d.sections.len(), 2);
            assert!(!md.contains(&format!("## {heading}")), "{md}");
            assert!(!l.prompts.borrow()[0].1.contains(&format!("\"{heading}\"")));
            // With themes: the section is asked for and rendered.
            let f = PremarketFacts {
                themes: two_themes(),
                ..facts()
            };
            let l = llm(vec![
                reply_with(&headings(lang, false, false, false), "Quiet."),
                reply_with(&headings(lang, false, true, false), "Quiet."),
            ]);
            let (d, md) = write(&l, &f, lang).await.unwrap();
            assert_eq!(l.prompts.borrow().len(), 2);
            assert_eq!(d.sections[1].0, heading);
            let user = &l.prompts.borrow()[0].1;
            assert!(user.contains(&format!("\"{heading}\"")), "{user}");
            assert!(
                md.contains(&format!(
                    "## {heading}\n\n{}\n\nQuiet.",
                    render_themes(&f.themes, lang)
                )),
                "{md}"
            );
        }
    }

    #[test]
    fn render_themes_en_and_zh() {
        let mut t = two_themes();
        t[1].more = 12;
        assert_eq!(
            render_themes(&t, Lang::En),
            "**Semiconductors** ▲ median +2.77%: NVDA +3.03%, AMD +2.50%\n\n\
             **Memory & storage** ▼ median −4.10%: MU −4.12%, WDC −3.90%, and 12 more"
        );
        assert_eq!(
            render_themes(&t, Lang::Zh),
            "**半导体** ▲ 中位数 +2.77%：NVDA +3.03%，AMD +2.50%\n\n\
             **存储芯片与存储设备** ▼ 中位数 −4.10%：MU −4.12%，WDC −3.90%，另有 12 只"
        );
        assert_eq!(render_themes(&[], Lang::En), "");
    }

    #[test]
    fn render_markdown_orders_macro_themes_movers() {
        let mut f = facts();
        f.themes = two_themes();
        f.macro_items = Field::Value(vec![reason("https://news.test/m", "Futures slip")]);
        for lang in [Lang::En, Lang::Zh] {
            let d = Draft {
                lang,
                title: "T".into(),
                summary: "S".into(),
                sections: headings(lang, true, true, false)
                    .iter()
                    .map(|h| (h.to_string(), format!("body of {h}")))
                    .collect(),
            };
            let md = render_markdown(&d, &f);
            let [futures, macro_h, themes, movers, _] = all_headings(lang);
            let at = |needle: &str| {
                md.find(needle)
                    .unwrap_or_else(|| panic!("{needle:?} missing from {md}"))
            };
            let order = [
                at(match lang {
                    Lang::En => "| S&P 500 futures |",
                    Lang::Zh => "| 标普500期货 |",
                }),
                at(&format!("## {futures}\n\nbody of {futures}")),
                at(&format!("## {macro_h}\n\nbody of {macro_h}")),
                at(&format!(
                    "## {themes}\n\n{}\n\nbody of {themes}",
                    render_themes(&f.themes, lang)
                )),
                at(&format!("## {movers}\n\n|")),
                at("| AAPL | Apple | 230.50 | +8.00% |"),
                at(&format!("|\n\nbody of {movers}")),
                at(match lang {
                    Lang::En => "## Sources",
                    Lang::Zh => "## 来源",
                }),
            ];
            assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}\n{md}");
            // The tables appear once: under their heading, not above it.
            assert_eq!(md.matches("| AAPL |").count(), 1, "{md}");
            // Theme reasons are cited with the rest.
            assert!(
                md.contains("[Chip stocks rally](https://news.test/chips)"),
                "{md}"
            );
        }
    }

    #[test]
    fn prompts_describe_the_themes_section() {
        assert!(
            PROMPT_EN.contains("one short paragraph per theme in the listed order; cite only the listed moves; a theme without a reason gets one sentence stating the move and gives no cause; do not repeat the list line")
                && PROMPT_EN.contains("\"Themes\" exists only when `themes` is not empty"),
            "{PROMPT_EN}"
        );
        assert!(
            PROMPT_ZH.contains("每个主题写一小段，按列出的顺序；只引用列出的涨跌幅；没有原因的主题用一句话说明涨跌，不写原因；不要重复列表那一行")
                && PROMPT_ZH.contains("只有 `themes` 不为空时才有“板块动向”")
                && PROMPT_ZH.contains("`themes` 列出成分股同向大幅波动的行业板块"),
            "{PROMPT_ZH}"
        );
    }

    #[tokio::test]
    async fn premarket_theme_member_names_with_digits_pass_the_check() {
        let mut f = facts();
        f.themes = two_themes();
        f.themes[1].members[1].name = "Phillips 66".into();
        f.themes[1].group = "Semis 2.0".into();
        let mut v = reply_with(&headings(Lang::En, false, true, false), "Quiet.").unwrap();
        v["sections"][1]["markdown"] =
            json!("Semis 2.0 names fell; Phillips 66 slipped 3.90% with Micron.");
        let l = llm(vec![Ok(v)]);
        let (d, _) = write(&l, &f, Lang::En).await.unwrap();
        assert_eq!(l.prompts.borrow().len(), 1);
        assert!(d.sections[1].1.contains("Phillips 66"));
        // The names' digits back nothing else.
        let mut v = reply_with(&headings(Lang::En, false, true, false), "Quiet.").unwrap();
        v["sections"][1]["markdown"] = json!("Phillips 66 slipped 66%.");
        let bad = || Ok(v.clone());
        let err = write(&llm(vec![bad(), bad()]), &f, Lang::En)
            .await
            .unwrap_err();
        assert_eq!(err, vec!["66%"]);
    }
    #[tokio::test]
    async fn premarket_more_count_above_ten_passes_the_check() {
        let mut f = facts();
        f.themes = two_themes();
        f.themes[1].more = 12;
        let heads = headings(Lang::En, false, true, false);
        let with = |text: &str| {
            let mut v = reply_with(&heads, "Quiet.").unwrap();
            v["sections"][1]["markdown"] = json!(text);
            v
        };
        let l = llm(vec![Ok(with(
            "Micron led storage lower, and 12 more fell.",
        ))]);
        write(&l, &f, Lang::En).await.unwrap();
        assert_eq!(l.prompts.borrow().len(), 1);
        // A count the facts do not hold still fails.
        let bad = || Ok(with("Micron led storage lower, and 18 more fell."));
        let err = write(&llm(vec![bad(), bad()]), &f, Lang::En)
            .await
            .unwrap_err();
        assert_eq!(err, vec!["18"]);
        // Only counts above 10 are added; 0-10 are the checker's small counts.
        f.themes[1].more = 7;
        assert_eq!(theme_allowed(&f.themes), Vec::<f64>::new());
        f.themes[0].more = 11;
        assert_eq!(theme_allowed(&f.themes), vec![11.0]);
    }

    #[tokio::test]
    async fn premarket_mover_names_with_digits_pass_the_check() {
        let mut f = facts();
        f.themes = two_themes();
        f.gainers[0].name = "Phillips 66".into();
        f.losers = vec![Mover {
            quote: quote("MMM", 120.0, -4.0),
            name: "3M".into(),
            reasons: vec![],
            search_failed: false,
            fallback: None,
        }];
        assert!(!theme_names(&f.themes).contains(&"Phillips 66"));
        let heads = headings(Lang::En, false, true, false);
        let with = |text: &str| {
            let mut v = reply_with(&heads, "Quiet.").unwrap();
            v["sections"][2]["markdown"] = json!(text);
            v
        };
        let l = llm(vec![Ok(with("Phillips 66 rose while 3M slipped."))]);
        let (d, _) = write(&l, &f, Lang::En).await.unwrap();
        assert_eq!(l.prompts.borrow().len(), 1);
        assert!(d.sections[2].1.contains("Phillips 66"));
        // The names' digits back nothing else.
        let bad = || Ok(with("Phillips 66 rose 66%."));
        let err = write(&llm(vec![bad(), bad()]), &f, Lang::En)
            .await
            .unwrap_err();
        assert_eq!(err, vec!["66%"]);
    }
}
