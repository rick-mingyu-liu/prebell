//! One language's earnings health check from the stored facts. The LLM
//! writes the prose; code renders the key-figures table, the filing links
//! and the disclaimer. Every number in the final page must trace to the
//! facts (`numbers::check`), and the text may not judge results against
//! estimates.

use std::collections::HashSet;
use std::sync::OnceLock;

use chrono::Datelike;
use regex::Regex;
use serde_json::Value;

use crate::earnings::facts::{numbers_in_facts, EarningsFacts, Metric, Unit};
use crate::facts::Field;
use crate::numbers::{check, extract as extract_numbers, Lang};
use crate::premarket::draft::{
    fmt_num, is_setext_underline, match_sections, one_line, reject_markup, sections_line,
    without_link_targets, Draft,
};
use crate::providers::{LlmProvider, ProviderError};

pub const TEMPLATE_ID: &str = "earnings:v1";
pub const CONTENT_TYPE: &str = "earnings";
pub const DISCLAIMER_EN: &str = "This earnings summary is generated automatically from SEC filings for information only. It reports results and company guidance, not analyst estimates, and it is not investment advice.";
pub const DISCLAIMER_ZH: &str =
    "本财报速评根据美国证监会文件自动生成，仅供参考；只报告业绩和公司指引，不涉及分析师预估，不构成投资建议。";

const PROMPT_EN: &str = include_str!("../../prompts/earnings_v1_en.md");
const PROMPT_ZH: &str = include_str!("../../prompts/earnings_v1_zh.md");
const ATTEMPTS: usize = 2;

/// Every section heading the check can have, in order.
fn headings(lang: Lang) -> [&'static str; 4] {
    match lang {
        Lang::En => [
            "Results",
            "Margins and cash flow",
            "Guidance",
            "What else stood out",
        ],
        Lang::Zh => ["业绩概览", "利润率与现金流", "业绩指引", "其他要点"],
    }
}

/// The headings these facts call for: Results always, every other section
/// only when it has a figure or note to discuss ("What else stood out" may
/// discuss the trend, when it has any quarter).
fn wanted(f: &EarningsFacts, lang: Lang) -> Vec<&'static str> {
    let [results, margins, guidance, other] = headings(lang);
    let notes = |n: &Field<Vec<_>>| matches!(n, Field::Value(v) if !v.is_empty());
    let mut out = vec![results];
    if [
        Metric::GrossMargin,
        Metric::OperatingMargin,
        Metric::OperatingCashFlow,
        Metric::FreeCashFlow,
    ]
    .into_iter()
    .any(|m| f.metrics.value(m).is_some())
    {
        out.push(margins);
    }
    if notes(&f.guidance) {
        out.push(guidance);
    }
    let trend = matches!(&f.trend, Field::Value(t)
        if !(t.revenue.is_empty() && t.eps_diluted.is_empty() && t.net_income.is_empty()));
    if notes(&f.highlights) || trend {
        out.push(other);
    }
    out
}

/// Phrases that narrate a missing figure, which the page simply leaves out
/// (the Apple fiscal Q4 2026 rehearsal said "Operating cash flow: n/a", then cited
/// $28 billion of it). Matched case-insensitively.
const MISSING: [&str; 6] = [
    "n/a",
    "暂缺",
    "not disclosed",
    "not provided",
    "未披露",
    "未提供",
];

/// Words that would judge results against estimates (spec §8: no consensus,
/// no beat/miss).
fn verdict_re(lang: Lang) -> &'static Regex {
    static EN: OnceLock<Regex> = OnceLock::new();
    static ZH: OnceLock<Regex> = OnceLock::new();
    match lang {
        Lang::En => EN.get_or_init(|| {
            Regex::new(r"(?i)\b(?:beat|beating|miss|misses|missed|estimates?|consensus|expectations?|wall street|topped|fell short|analysts?)\b|\b(?:better|worse|stronger|weaker|higher|lower)\s+than\s+(?:expected|anticipated|forecast)")
                .expect("valid built-in regex")
        }),
        Lang::Zh => ZH.get_or_init(|| {
            Regex::new(r"预期|预估|共识|預期|預估|共識|华尔街|分析师|估计").expect("valid built-in regex")
        }),
    }
}

/// Longest title, in characters. The front page shows earnings checks in a
/// three-column grid at 16px (about 26 Latin or 13 CJK characters a line):
/// 90 English or 40 Chinese characters is at most four lines there, and two
/// in the 32px post heading.
fn title_max(lang: Lang) -> usize {
    match lang {
        Lang::En => 90,
        Lang::Zh => 40,
    }
}

/// The press release's own kind of headline: "Charter Communications
/// Reports Third Quarter 2026 Results", "Duke Energy reports third-quarter
/// 2026 results", "Simon Property Group: third-quarter 2026 results",
/// "苹果公布2026财年第四季度业绩".
fn boilerplate_re(lang: Lang) -> &'static Regex {
    static EN: OnceLock<Regex> = OnceLock::new();
    static ZH: OnceLock<Regex> = OnceLock::new();
    match lang {
        Lang::En => EN.get_or_init(|| {
            // Period words that may sit between the verb and "results".
            const P: &str = r"(?:first|second|third|fourth|q[1-4]|quarter(?:ly)?|full|year(?:-end)?|fiscal|annual|fy\s?\d{2,4}|\d{4}|and|its|their|financial|operating)";
            // "earnings" alone is a takeaway's subject ("fourth-quarter
            // earnings rise 91%"); only a headline that ends on it is boilerplate.
            const END: &str = r"(?:results\b|earnings\s*(?:[:;,.!–—-]|$))";
            Regex::new(&format!(
                r"(?i)\b(?:report(?:s|ed)?|announce(?:s|d)?|post(?:s|ed)?|release(?:s|d)?|deliver(?:s|ed)?|unveil(?:s|ed)?)\s+(?:{P}[-\s]+)*{END}|\b(?:first|second|third|fourth|q[1-4]|fiscal|full|annual|fy\s?\d{{2,4}}|\d{{4}})(?:[-\s]+{P})*[-\s]+{END}"
            ))
            .expect("valid built-in regex")
        }),
        Lang::Zh => ZH.get_or_init(|| {
            Regex::new(r"(?:公布|发布|披露|宣布|发表|交出)[^，。；：,;:]{0,20}?(?:财报|业绩|财务结果)(?:报告)?(?:[，。；：,;:\s]|$)|(?:季度|财年|年度|Q[1-4])\s*(?:财报|业绩)\s*$")
                .expect("valid built-in regex")
        }),
    }
}

/// A figure the title states: a percent, a dollar or scaled amount, a
/// decimal, or a whole number that is neither a small count nor a year.
fn has_figure(text: &str, lang: Lang) -> bool {
    extract_numbers(text, lang).iter().any(|n| {
        n.is_percent
            || n.raw.contains('$')
            || n.multiplier != 1.0
            || n.decimals > 0
            || (n.value.abs() > 10.0 && !(1900.0..=2100.0).contains(&n.value.abs()))
    })
}

/// The headline states the quarter's takeaway: short, with a figure, and
/// not the press release's title. Its numbers are checked with the page's.
fn check_title(title: &str, lang: Lang, f: &EarningsFacts) -> Result<(), String> {
    let n = title.chars().count();
    if n > title_max(lang) {
        return Err(format!(
            "the title is {n} characters; keep it to {} or fewer",
            title_max(lang)
        ));
    }
    if let Some(m) = boilerplate_re(lang).find(title) {
        return Err(format!(
            "the title must state the quarter's most important result, not repeat the press release's headline (found {:?})",
            m.as_str().trim()
        ));
    }
    // "3M" is a name, not three million.
    if !has_figure(&mask_names(title, f), lang) {
        return Err(
            "the title must state the quarter's most important result with at least one figure from the facts"
                .into(),
        );
    }
    Ok(())
}

/// Up to `max` decimals, trailing zeros dropped: 47.20 -> "47.2".
fn trim_dec(v: f64, max: usize) -> String {
    let s = format!("{v:.max$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// A figure value for the key-figures table.
pub fn fmt_value(v: f64, unit: Unit, lang: Lang) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    match (unit, lang) {
        (Unit::Percent, _) => format!("{}%", trim_dec(v, 2)),
        (Unit::UsdPerShare, Lang::En) => format!("{sign}${a:.2}"),
        (Unit::UsdPerShare, Lang::Zh) => format!("{sign}{a:.2}美元"),
        (Unit::Usd, Lang::En) if a >= 1e9 => format!("{sign}${} billion", trim_dec(a / 1e9, 2)),
        (Unit::Usd, Lang::En) if a >= 1e6 => format!("{sign}${} million", trim_dec(a / 1e6, 1)),
        (Unit::Usd, Lang::En) => format!("{sign}${}", fmt_num(a)),
        (Unit::Usd, Lang::Zh) if a >= 1e8 => format!("{sign}{}亿美元", trim_dec(a / 1e8, 1)),
        (Unit::Usd, Lang::Zh) if a >= 1e4 => format!("{sign}{}万美元", trim_dec(a / 1e4, 1)),
        (Unit::Usd, Lang::Zh) => format!("{sign}{}美元", fmt_num(a)),
    }
}

/// "Fiscal Q4 2026" / "2026财年第四季度"; full-year releases drop the quarter.
pub fn period_label(f: &EarningsFacts, lang: Lang) -> Option<String> {
    let Field::Value(p) = &f.fiscal_period else {
        return None;
    };
    let zh_q = ["", "第一季度", "第二季度", "第三季度", "第四季度"];
    Some(match (lang, p.quarter) {
        (Lang::En, Some(q)) => format!("Fiscal Q{q} {}", p.fiscal_year),
        (Lang::En, None) => format!("Fiscal year {}", p.fiscal_year),
        (Lang::Zh, Some(q)) => format!(
            "{}财年{}",
            p.fiscal_year,
            zh_q.get(q as usize).copied().unwrap_or("")
        ),
        (Lang::Zh, None) => format!("{}财年", p.fiscal_year),
    })
}

/// `{ticker}-{period}-earnings`: `aapl-q4-fy2026-earnings`,
/// `brk-b-fy2026-earnings`, or the ET filing date when the period is unverified.
pub fn slug(f: &EarningsFacts) -> String {
    let t = f.ticker.to_ascii_lowercase().replace('.', "-");
    let period = match &f.fiscal_period {
        Field::Value(p) => match p.quarter {
            Some(q) => format!("q{q}-fy{}", p.fiscal_year),
            None => format!("fy{}", p.fiscal_year),
        },
        Field::Missing { .. } => f.filing_date.format("%Y-%m-%d").to_string(),
    };
    format!("{t}-{period}-earnings")
}

fn cell(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace('|', "\\|")
}

/// The full article: title, filing line, key-figures table, LLM sections,
/// sources and disclaimer.
pub fn render_markdown(d: &Draft, f: &EarningsFacts) -> String {
    let lang = d.lang;
    let mut out = format!("# {}\n\n{}\n\n", d.title, d.summary);

    let filed = f.filing_date;
    let mut meta = vec![format!("**{} ({})**", cell(&f.company), cell(&f.ticker))];
    if let Some(p) = period_label(f, lang) {
        meta.push(p);
    }
    meta.push(match lang {
        Lang::En => format!("Form 8-K filed {}", filed.format("%b %-d, %Y")),
        Lang::Zh => format!(
            "8-K 提交于{}年{}月{}日",
            filed.year(),
            filed.month(),
            filed.day()
        ),
    });
    out.push_str(&meta.join(" · "));
    out.push_str("\n\n");

    out.push_str(match lang {
        Lang::En => "| Key figure | This quarter |\n|---|---:|\n",
        Lang::Zh => "| 关键数据 | 本季度 |\n|---|---:|\n",
    });
    // Only the figures the release gave; an EPS is required, and revenue is unless the release has none.
    for m in Metric::ALL {
        let Some(fig) = f.metrics.value(m) else {
            continue;
        };
        let label = match lang {
            Lang::En => m.label_en(),
            Lang::Zh => m.label_zh(),
        };
        let v = fmt_value(fig.value, fig.unit, lang);
        out.push_str(&format!("| {label} | {v} |\n"));
    }
    out.push('\n');

    for (heading, body) in &d.sections {
        out.push_str(&format!("## {heading}\n\n{}\n\n", body.trim()));
    }

    out.push_str(match lang {
        Lang::En => "## Sources\n\n",
        Lang::Zh => "## 来源\n\n",
    });
    let (filing, release) = match lang {
        Lang::En => ("Form 8-K filing (SEC EDGAR)", "Press release"),
        Lang::Zh => ("8-K 文件（SEC EDGAR）", "新闻稿"),
    };
    out.push_str(&format!("- [{filing}]({})\n", f.filing_url));
    if let Field::Value(url) = &f.exhibit_url {
        out.push_str(&format!("- [{release}]({url})\n"));
    }
    out.push('\n');

    out.push_str(match lang {
        Lang::En => DISCLAIMER_EN,
        Lang::Zh => DISCLAIMER_ZH,
    });
    out.push('\n');
    out
}

/// The company name and ticker replaced by placeholders before the number
/// check, as whole words only (no ASCII letter or digit on either side), so
/// a one-letter ticker such as C, T, V or F never eats letters of "CEO" or
/// "FY2026". CJK text next to a name still counts as a boundary.
fn mask_names(text: &str, f: &EarningsFacts) -> String {
    let mut out = text.to_string();
    for name in [f.company.as_str(), f.ticker.as_str()] {
        if name.is_empty() {
            continue;
        }
        let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
        let mut masked = String::with_capacity(out.len());
        let mut rest = 0;
        for (i, _) in out.match_indices(name) {
            if i < rest
                || word(out[..i].chars().next_back())
                || word(out[i + name.len()..].chars().next())
            {
                continue;
            }
            masked.push_str(&out[rest..i]);
            masked.push_str("NAME");
            rest = i + name.len();
        }
        masked.push_str(&out[rest..]);
        out = masked;
    }
    out
}

enum Problem {
    Llm(ProviderError),
    Malformed(String),
    Numbers(Vec<String>),
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
            let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            (text("heading").trim().to_string(), text("markdown"))
        })
        .collect();
    // A known section the facts do not call for is dropped unread.
    let got = match_sections(got, want, &headings(lang))?;
    for (h, m) in &got {
        reject_markup(&format!("section {h:?}"), m)?;
        if m.lines()
            .any(|l| l.trim_start().starts_with('#') || is_setext_underline(l))
        {
            return Err(format!("section {h:?} contains a heading line"));
        }
    }
    let all = std::iter::once(title.as_str())
        .chain([summary.as_str()])
        .chain(got.iter().map(|(_, m)| m.as_str()));
    for text in all {
        if let Some(w) = verdict_re(lang).find(text) {
            return Err(format!(
                "do not compare results with estimates or expectations (found {:?})",
                w.as_str()
            ));
        }
        let lower = text.to_lowercase();
        if let Some(p) = MISSING.iter().find(|p| lower.contains(*p)) {
            return Err(format!(
                "do not mention missing figures ({p:?}): leave them out entirely"
            ));
        }
    }
    Ok(Draft {
        lang,
        title,
        summary,
        sections: got,
    })
}

fn attempt(
    v: Result<Value, ProviderError>,
    f: &EarningsFacts,
    lang: Lang,
    want: &[&str],
    allowed: &[f64],
) -> Result<(Draft, String), Problem> {
    let v = v.map_err(Problem::Llm)?;
    let d = parse(&v, lang, want).map_err(Problem::Malformed)?;
    check_title(&d.title, lang, f).map_err(Problem::Malformed)?;
    let md = render_markdown(&d, f);
    // The whole page, code-rendered parts included, is checked. Names are
    // not figures: "3M" must not read as three million.
    check(&mask_names(&without_link_targets(&md), f), lang, allowed).map_err(|bad| {
        let mut seen = HashSet::new();
        Problem::Numbers(bad.into_iter().filter(|n| seen.insert(n.clone())).collect())
    })?;
    Ok((d, md))
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

fn prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::En => PROMPT_EN,
        Lang::Zh => PROMPT_ZH,
    }
}

/// Drafts one language, retrying once with the problems appended. Returns
/// the draft and its rendered markdown, or the last attempt's errors.
pub async fn write(
    llm: &dyn LlmProvider,
    f: &EarningsFacts,
    lang: Lang,
) -> Result<(Draft, String), Vec<String>> {
    let facts_json =
        serde_json::to_string_pretty(f).map_err(|e| vec![format!("draft: facts json: {e}")])?;
    let allowed = numbers_in_facts(f);
    let want = wanted(f, lang);
    let base = format!("{facts_json}\n\n{}", sections_line(lang, &want));
    let mut user = base.clone();
    let mut last = None;
    for _ in 0..ATTEMPTS {
        match attempt(
            llm.complete_json(prompt(lang), &user).await,
            f,
            lang,
            &want,
            &allowed,
        ) {
            Ok(x) => return Ok(x),
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
    use crate::earnings::facts::tests::facts;
    use crate::earnings::facts::FiscalPeriod;
    use async_trait::async_trait;
    use serde_json::json;
    use std::cell::RefCell;

    /// Replies with scripted responses in order; records every user prompt.
    pub(crate) struct FakeLlm {
        pub replies: RefCell<Vec<Result<Value, ProviderError>>>,
        pub prompts: RefCell<Vec<String>>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, _system: &str, user: &str) -> Result<Value, ProviderError> {
            self.prompts.borrow_mut().push(user.into());
            let mut r = self.replies.borrow_mut();
            assert!(!r.is_empty(), "unexpected extra LLM call");
            r.remove(0)
        }
    }
    fn llm(replies: Vec<Value>) -> FakeLlm {
        FakeLlm {
            replies: RefCell::new(replies.into_iter().map(Ok).collect()),
            prompts: RefCell::new(vec![]),
        }
    }

    pub(crate) fn reply(lang: Lang, body: &str) -> Value {
        let (title, summary) = match lang {
            Lang::En => (
                "Apple fiscal Q4 2026: revenue $102.5 billion",
                "Revenue rose 8% and GAAP diluted EPS was $1.85.",
            ),
            Lang::Zh => (
                "苹果2026财年第四季度营收1025亿美元",
                "营收增长8%，GAAP摊薄每股收益1.85美元。",
            ),
        };
        json!({
            "title": title,
            "summary": summary,
            "sections": headings(lang).iter().map(|h| json!({"heading": h, "markdown": body})).collect::<Vec<_>>(),
        })
    }

    #[tokio::test]
    async fn clean_draft_renders_facts_table_and_sources() {
        let l = llm(vec![reply(Lang::En, "Revenue was $102.5 billion, up 8%.")]);
        let (_, md) = write(&l, &facts(), Lang::En).await.unwrap();
        assert!(md.contains("| Revenue | $102.5 billion |"), "{md}");
        assert!(md.contains("| Diluted EPS (GAAP) | $1.85 |"));
        assert!(md.contains("| Gross margin | 47.2% |"));
        // Missing figures get no row at all.
        assert!(
            !md.contains("Free cash flow") && !md.contains("n/a"),
            "{md}"
        );
        assert!(md.contains("Fiscal Q4 2026 · Form 8-K filed Oct 30, 2026"));
        assert!(md.contains("(https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm)"));
        assert!(md.contains("a8-kex991q4202609262026.htm)"));
        assert!(md.trim_end().ends_with(DISCLAIMER_EN));
    }

    #[tokio::test]
    async fn zh_draft() {
        let l = llm(vec![reply(Lang::Zh, "营收1025亿美元，毛利率47.2%。")]);
        let (_, md) = write(&l, &facts(), Lang::Zh).await.unwrap();
        assert!(md.contains("| 营收 | 1025亿美元 |"), "{md}");
        assert!(md.contains("| 摊薄每股收益（GAAP） | 1.85美元 |"));
        assert!(md.contains("2026财年第四季度 · 8-K 提交于2026年10月30日"));
        assert!(!md.contains("经营现金流") && !md.contains("暂缺"), "{md}");
    }

    #[tokio::test]
    async fn full_facts_keep_all_four_sections() {
        for lang in [Lang::En, Lang::Zh] {
            let l = llm(vec![reply(lang, "Quiet.")]);
            let (d, md) = write(&l, &facts(), lang).await.unwrap();
            let got: Vec<&str> = d.sections.iter().map(|(h, _)| h.as_str()).collect();
            assert_eq!(got, headings(lang));
            for h in headings(lang) {
                assert!(md.contains(&format!("## {h}\n\nQuiet.")), "{md}");
            }
        }
    }

    #[tokio::test]
    async fn thin_release_prints_only_given_figures_and_results() {
        let mut f = facts();
        f.metrics.gross_margin = Field::Missing { reason: "x".into() };
        f.metrics.net_income = Field::Value(crate::earnings::facts::Figure {
            value: 27.5e9,
            unit: Unit::Usd,
            amount: "$27.5 billion".into(),
            scale: 1.0,
            quote: "Net income was $27.5 billion for the quarter.".into(),
        });
        f.guidance = Field::Value(vec![]);
        f.highlights = Field::Missing {
            reason: "none".into(),
        };
        f.trend = Field::Missing {
            reason: "no XBRL".into(),
        };
        for (lang, rows, line) in [
            (
                Lang::En,
                "| Key figure | This quarter |\n|---|---:|\n| Revenue | $102.5 billion |\n| Net income (GAAP) | $27.5 billion |\n| Diluted EPS (GAAP) | $1.85 |\n\n",
                "Write exactly these sections, in this order: \"Results\"",
            ),
            (
                Lang::Zh,
                "| 关键数据 | 本季度 |\n|---|---:|\n| 营收 | 1025亿美元 |\n| 净利润（GAAP） | 275亿美元 |\n| 摊薄每股收益（GAAP） | 1.85美元 |\n\n",
                "只写以下部分，按此顺序：\"业绩概览\"",
            ),
        ] {
            // The model writes all four; the three without facts are dropped.
            let l = llm(vec![reply(lang, "Quiet.")]);
            let (d, md) = write(&l, &f, lang).await.unwrap();
            assert!(md.contains(rows), "{md}");
            assert_eq!(d.sections.len(), 1);
            assert_eq!(d.sections[0].0, headings(lang)[0]);
            assert_eq!(md.matches("\n## ").count(), 2, "Results and Sources only: {md}");
            assert!(l.prompts.borrow()[0].ends_with(line), "{}", l.prompts.borrow()[0]);
        }
    }

    #[test]
    fn an_empty_trend_is_nothing_to_discuss() {
        let mut f = facts();
        f.highlights = Field::Value(vec![]);
        f.trend = Field::Value(crate::earnings::facts::Trend {
            revenue: vec![],
            eps_diluted: vec![],
            net_income: vec![],
        });
        assert_eq!(
            wanted(&f, Lang::En),
            ["Results", "Margins and cash flow", "Guidance"]
        );
        // Any quarter in the trend is enough.
        f.trend = facts().trend;
        assert_eq!(wanted(&f, Lang::En).len(), 4);
    }

    #[tokio::test]
    async fn narrating_a_missing_figure_is_retried() {
        for (lang, body) in [
            (Lang::En, "Operating cash flow: n/a."),
            (Lang::En, "Free cash flow was not disclosed."),
            (Lang::En, "Guidance was Not Provided."),
            (Lang::Zh, "经营现金流暂缺。"),
            (Lang::Zh, "公司未披露自由现金流。"),
            (Lang::Zh, "公司未提供指引。"),
        ] {
            let l = llm(vec![reply(lang, body), reply(lang, "Quiet.")]);
            assert!(write(&l, &facts(), lang).await.is_ok(), "{body}");
            let p = l.prompts.borrow();
            assert_eq!(p.len(), 2);
            assert!(p[1].contains("do not mention missing figures"), "{}", p[1]);
        }
        let mut r = reply(Lang::En, "Quiet.");
        r["summary"] = "Revenue was $102.5 billion; cash flow n/a.".into();
        let l = llm(vec![r.clone(), r]);
        let e = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(e[0].contains("missing figures"), "{e:?}");
    }

    #[test]
    fn prompts_leave_missing_figures_out() {
        assert!(
            PROMPT_EN.contains("do not mention it at all")
                && PROMPT_EN.contains("listed at the end of the user message")
                && !PROMPT_EN.contains("write \"n/a\" for it")
                && !PROMPT_EN.contains("says \"n/a\""),
            "{PROMPT_EN}"
        );
        assert!(
            PROMPT_ZH.contains("完全不要提及")
                && PROMPT_ZH.contains("列在用户消息末尾")
                && !PROMPT_ZH.contains("时写\"暂缺\""),
            "{PROMPT_ZH}"
        );
    }

    #[tokio::test]
    async fn unbacked_number_retried_with_errors() {
        let l = llm(vec![
            reply(Lang::En, "Services revenue grew 13%."),
            reply(Lang::En, "Revenue was $102.5 billion."),
        ]);
        assert!(write(&l, &facts(), Lang::En).await.is_ok());
        let p = l.prompts.borrow();
        assert_eq!(p.len(), 2);
        assert!(p[1].contains("These numbers are not in the facts: 13%"));
    }

    #[tokio::test]
    async fn estimate_talk_rejected() {
        let bad = reply(Lang::En, "Results beat estimates.");
        let l = llm(vec![bad.clone(), bad]);
        let e = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(e[0].contains("estimates") || e[0].contains("beat"), "{e:?}");
        let bad = reply(Lang::Zh, "业绩超出市场预期。");
        let l = llm(vec![bad.clone(), bad]);
        assert!(write(&l, &facts(), Lang::Zh).await.is_err());
    }

    #[tokio::test]
    async fn links_rejected() {
        let bad = reply(Lang::En, "See [the release](https://evil.test).");
        let l = llm(vec![bad.clone(), bad]);
        let e = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(e[0].contains("forbidden markup"), "{e:?}");
    }

    #[test]
    fn one_letter_tickers_mask_only_whole_words() {
        let mut f = facts();
        f.company = "Citigroup".into();
        f.ticker = "C".into();
        assert_eq!(
            mask_names(
                "Citigroup (C) said CEO pay rose; C's EPS was $1.85 in FY2026 (花旗C公司).",
                &f
            ),
            "NAME (NAME) said CEO pay rose; NAME's EPS was $1.85 in FY2026 (花旗NAME公司)."
        );
        f.ticker = "F".into();
        f.company = "Ford Motor".into();
        assert_eq!(
            mask_names("Ford Motor (F): FCF and FY2026, per F.", &f),
            "NAME (NAME): FCF and FY2026, per NAME."
        );
    }

    #[tokio::test]
    async fn names_with_digits_are_not_figures() {
        let mut f = facts();
        f.company = "3M".into();
        f.ticker = "MMM".into();
        let mut r = reply(Lang::En, "3M reported revenue of $102.5 billion.");
        r["title"] = "3M revenue reaches $102.5 billion".into();
        let l = llm(vec![r]);
        let (_, md) = write(&l, &f, Lang::En).await.unwrap();
        assert!(md.contains("**3M (MMM)**"));
        // The name is no figure: a title with only "3M" in it has none.
        assert!(check_title("3M posts a strong fourth quarter", Lang::En, &f).is_err());
    }

    #[test]
    fn titles_state_the_takeaway() {
        let f = facts();
        for (lang, t) in [
            (
                Lang::En,
                "Apple's EPS jumps 91% to $1.85 as revenue reaches $102.5 billion",
            ),
            (Lang::En, "Apple fiscal Q4 2026: revenue $102.5 billion"),
            (
                Lang::En,
                "Apple's fourth-quarter earnings rise 91% to $1.85 a share",
            ),
            (
                Lang::En,
                "Charter swings to a $20.50-a-share loss as revenue rises to $14.9 billion",
            ),
            (Lang::Zh, "苹果第四季度每股收益增长91%至1.85美元"),
            (Lang::Zh, "苹果2026财年第四季度营收1025亿美元"),
            (Lang::Zh, "特许通讯由盈转亏，每股亏损20.50美元"),
        ] {
            assert_eq!(check_title(t, lang, &f), Ok(()), "{t}");
        }
    }

    #[test]
    fn press_release_titles_rejected() {
        let f = facts();
        for (lang, t) in [
            (
                Lang::En,
                "Charter Communications Reports Third Quarter 2026 Results",
            ),
            (Lang::En, "Simon Property Group: third-quarter 2026 results"),
            (Lang::En, "ConocoPhillips third-quarter 2026 results"),
            (Lang::En, "Duke Energy reports third-quarter 2026 results"),
            (
                Lang::En,
                "Qualcomm Announces Fourth Quarter and Fiscal 2026 Results",
            ),
            (
                Lang::En,
                "Southern Company reports third-quarter 2026 earnings",
            ),
            // A figure tacked on does not make it a takeaway.
            (
                Lang::En,
                "Apple Reports Fourth Quarter Results: Revenue $102.5 Billion",
            ),
            (Lang::Zh, "苹果公布2026财年第四季度业绩"),
            (Lang::Zh, "苹果发布第四季度财报"),
            (Lang::Zh, "苹果：2026年第三季度业绩"),
            (Lang::Zh, "苹果公布第四季度业绩，营收1025亿美元"),
        ] {
            let e = check_title(t, lang, &f).unwrap_err();
            assert!(e.contains("press release's headline"), "{t}: {e}");
        }
        // No figure at all.
        for (lang, t) in [
            (Lang::En, "Apple closes a record fiscal 2026"),
            (Lang::Zh, "苹果2026财年收官"),
        ] {
            let e = check_title(t, lang, &f).unwrap_err();
            assert!(e.contains("at least one figure"), "{t}: {e}");
        }
    }

    #[test]
    fn title_length_cap() {
        let f = facts();
        let en = format!(
            "Apple revenue reaches $102.5 billion{}",
            " and more".repeat(6)
        );
        assert_eq!(en.chars().count(), 90);
        assert_eq!(check_title(&en, Lang::En, &f), Ok(()));
        let e = check_title(&format!("{en}!"), Lang::En, &f).unwrap_err();
        assert!(
            e.contains("91 characters") && e.contains("90 or fewer"),
            "{e}"
        );
        let zh: String = format!("苹果营收1025亿美元{}", "，同比增长".repeat(6))
            .chars()
            .take(40)
            .collect();
        assert_eq!(zh.chars().count(), 40);
        assert_eq!(check_title(&zh, Lang::Zh, &f), Ok(()));
        assert!(check_title(&format!("{zh}。"), Lang::Zh, &f).is_err());
    }

    #[tokio::test]
    async fn press_release_title_is_retried_with_the_reason() {
        for lang in [Lang::En, Lang::Zh] {
            let mut bad = reply(lang, "Quiet.");
            bad["title"] = match lang {
                Lang::En => "Apple Reports Fourth Quarter Results".into(),
                Lang::Zh => "苹果公布第四季度业绩".into(),
            };
            let l = llm(vec![bad.clone(), reply(lang, "Quiet.")]);
            assert!(write(&l, &facts(), lang).await.is_ok());
            let p = l.prompts.borrow();
            assert!(
                p[1].contains("not repeat the press release's headline"),
                "{}",
                p[1]
            );
            let l = llm(vec![bad.clone(), bad]);
            let e = write(&l, &facts(), lang).await.unwrap_err();
            assert!(e[0].starts_with("draft: the title must"), "{e:?}");
        }
    }

    #[tokio::test]
    async fn title_numbers_are_checked_with_the_page() {
        let mut r = reply(Lang::En, "Quiet.");
        r["title"] = "Apple revenue jumps 15% to $102.5 billion".into();
        let l = llm(vec![r.clone(), r]);
        let e = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert_eq!(e, vec!["15%".to_string()]);
        let mut r = reply(Lang::Zh, "Quiet.");
        r["title"] = "苹果营收增长13%".into();
        let l = llm(vec![r.clone(), r]);
        assert_eq!(
            write(&l, &facts(), Lang::Zh).await.unwrap_err(),
            vec!["13%".to_string()]
        );
    }

    #[tokio::test]
    async fn estimate_talk_in_the_title_rejected() {
        let mut r = reply(Lang::En, "Quiet.");
        r["title"] = "Apple tops estimates with $102.5 billion in revenue".into();
        let l = llm(vec![r.clone(), r]);
        let e = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(e[0].contains("estimates"), "{e:?}");
    }

    #[test]
    fn prompts_ask_for_the_takeaway_title() {
        assert!(
            PROMPT_EN.contains("most important result")
                && PROMPT_EN.contains("at most 90 characters")
        );
        assert!(PROMPT_ZH.contains("最重要的结果") && PROMPT_ZH.contains("不超过 40 个字"));
    }

    #[test]
    fn verdict_words() {
        let ok = |t: &str, lang| verdict_re(lang).find(t).is_none();
        assert!(ok("Free cash flow is missing from the release.", Lang::En));
        assert!(ok(
            "Revenue is expected to grow 10 to 12 percent.",
            Lang::En
        ));
        assert!(ok("Beats headphones sales rose.", Lang::En));
        assert!(ok("营收超出1000亿美元。", Lang::Zh));
        assert!(!ok("Revenue was better than expected.", Lang::En));
        assert!(!ok("EPS topped Wall Street forecasts.", Lang::En));
        assert!(!ok("Results missed estimates.", Lang::En));
        assert!(!ok("业绩不及预期。", Lang::Zh));
        assert!(!ok("華爾街預期", Lang::Zh));
        assert!(!ok("Revenue came in ahead of analyst forecasts.", Lang::En));
        assert!(!ok("营收高于分析师估计。", Lang::Zh));
    }

    #[tokio::test]
    async fn zh_rounding_and_loss_through_full_page_check() {
        let mut f = facts();
        if let Field::Value(fig) = &mut f.metrics.revenue {
            fig.value = 102.466e9;
            fig.amount = "102,466".into();
            fig.scale = 1e6;
            fig.quote = "Total net sales $102,466 $94,930".into();
        }
        f.metrics.net_income = Field::Value(crate::earnings::facts::Figure {
            value: -25e6,
            unit: Unit::Usd,
            amount: "$25 million".into(),
            scale: 1.0,
            quote: "Net loss was $25 million in the quarter.".into(),
        });
        // The revenue quote is now a table row, so "up 8 percent" is gone.
        let mut r = reply(Lang::Zh, "营收1024.7亿美元，净亏损2500万美元。");
        r["summary"] = "营收1024.7亿美元。".into();
        let l = llm(vec![r]);
        let (_, md) = write(&l, &f, Lang::Zh).await.unwrap();
        assert!(md.contains("| 营收 | 1024.7亿美元 |"), "{md}");
        assert!(md.contains("| 净利润（GAAP） | -2500万美元 |"), "{md}");
    }

    #[test]
    fn slugs() {
        let mut f = facts();
        assert_eq!(slug(&f), "aapl-q4-fy2026-earnings");
        f.ticker = "BRK.B".into();
        f.fiscal_period = Field::Value(FiscalPeriod {
            quarter: None,
            fiscal_year: 2026,
            quote: "x".into(),
        });
        assert_eq!(slug(&f), "brk-b-fy2026-earnings");
        f.fiscal_period = Field::Missing { reason: "x".into() };
        assert_eq!(slug(&f), "brk-b-2026-10-30-earnings");
    }

    #[test]
    fn value_formats() {
        assert_eq!(fmt_value(102.466e9, Unit::Usd, Lang::En), "$102.47 billion");
        assert_eq!(fmt_value(-25e6, Unit::Usd, Lang::En), "-$25 million");
        assert_eq!(fmt_value(102.466e9, Unit::Usd, Lang::Zh), "1024.7亿美元");
        assert_eq!(fmt_value(-0.25, Unit::UsdPerShare, Lang::En), "-$0.25");
        assert_eq!(fmt_value(47.2, Unit::Percent, Lang::Zh), "47.2%");
    }

    #[test]
    fn every_rendered_number_is_in_the_facts() {
        let f = facts();
        let allowed = numbers_in_facts(&f);
        for lang in [Lang::En, Lang::Zh] {
            let d = parse(&reply(lang, "Quiet."), lang, &wanted(&f, lang)).unwrap();
            let md = render_markdown(&d, &f);
            assert_eq!(
                check(&without_link_targets(&md), lang, &allowed),
                Ok(()),
                "{md}"
            );
        }
    }
}
