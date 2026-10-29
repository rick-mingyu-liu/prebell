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
use crate::numbers::{check, Lang};
use crate::premarket::draft::{fmt_num, is_setext_underline, one_line, reject_markup, Draft};
use crate::providers::{LlmProvider, ProviderError};

pub const TEMPLATE_ID: &str = "earnings:v1";
pub const CONTENT_TYPE: &str = "earnings";
pub const DISCLAIMER_EN: &str = "This earnings summary is generated automatically from SEC filings for information only. It reports results and company guidance, not analyst estimates, and it is not investment advice.";
pub const DISCLAIMER_ZH: &str =
    "本财报速评根据美国证监会文件自动生成，仅供参考；只报告业绩和公司指引，不涉及分析师预估，不构成投资建议。";

const PROMPT_EN: &str = include_str!("../../prompts/earnings_v1_en.md");
const PROMPT_ZH: &str = include_str!("../../prompts/earnings_v1_zh.md");
const ATTEMPTS: usize = 2;

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

fn na(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "n/a",
        Lang::Zh => "暂缺",
    }
}

/// Words that would judge results against estimates (spec §8: no consensus,
/// no beat/miss).
fn verdict_re(lang: Lang) -> &'static Regex {
    static EN: OnceLock<Regex> = OnceLock::new();
    static ZH: OnceLock<Regex> = OnceLock::new();
    match lang {
        Lang::En => EN.get_or_init(|| {
            Regex::new(r"(?i)\b(?:beats?|beating|miss(?:es|ed|ing)?|estimates?|estimated|consensus|expectations?|forecasters?|analysts?)\b")
                .expect("valid built-in regex")
        }),
        Lang::Zh => ZH.get_or_init(|| {
            Regex::new(r"预期|预估|共识|超出|不及|分析师").expect("valid built-in regex")
        }),
    }
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
pub(crate) fn fmt_value(v: f64, unit: Unit, lang: Lang) -> String {
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
        (Lang::Zh, Some(q)) => format!("{}财年{}", p.fiscal_year, zh_q[q as usize]),
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
    for m in Metric::ALL {
        let label = match lang {
            Lang::En => m.label_en(),
            Lang::Zh => m.label_zh(),
        };
        let v = f.metrics.value(m).map_or_else(
            || na(lang).to_string(),
            |fig| fmt_value(fig.value, fig.unit, lang),
        );
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

/// The page text with link targets removed (EDGAR URLs carry accession digits).
fn without_link_targets(md: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\]\([^)\s]*\)").expect("valid built-in regex"))
        .replace_all(md, "]")
        .into_owned()
}

enum Problem {
    Llm(ProviderError),
    Malformed(String),
    Numbers(Vec<String>),
}

fn parse(v: &Value, lang: Lang) -> Result<Draft, String> {
    let title = one_line(v, "title")?;
    let summary = one_line(v, "summary")?;
    if summary.starts_with('#') {
        return Err("summary starts with a heading marker".into());
    }
    let want = headings(lang);
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
    allowed: &[f64],
) -> Result<(Draft, String), Problem> {
    let v = v.map_err(Problem::Llm)?;
    let d = parse(&v, lang).map_err(Problem::Malformed)?;
    let md = render_markdown(&d, f);
    // The whole page, code-rendered parts included, is checked.
    check(&without_link_targets(&md), lang, allowed).map_err(|bad| {
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
    let mut user = facts_json.clone();
    let mut last = None;
    for _ in 0..ATTEMPTS {
        match attempt(
            llm.complete_json(prompt(lang), &user).await,
            f,
            lang,
            &allowed,
        ) {
            Ok(x) => return Ok(x),
            Err(p) => {
                user = format!("{facts_json}{}", retry_note(&p));
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
        assert!(md.contains("| Free cash flow (non-GAAP) | n/a |"));
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
        assert!(md.contains("| 经营现金流 | 暂缺 |"));
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
            let d = parse(&reply(lang, "n/a"), lang).unwrap();
            let md = render_markdown(&d, &f);
            assert_eq!(
                check(&without_link_targets(&md), lang, &allowed),
                Ok(()),
                "{md}"
            );
        }
    }
}
