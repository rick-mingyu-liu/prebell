//! Turns recap facts into one language's article, with the same drafting
//! loop, markup filter and number check as the premarket brief. Code renders
//! the tables from the facts, and the rendered body is number-checked too.

use crate::facts::{Field, Mover, NewsReason};
use crate::numbers::{check, Lang};
use crate::premarket::draft::{
    blank_names, cell, fmt_num, fmt_pct, na, render_sources, write_with, Draft,
};
use crate::providers::LlmProvider;
use crate::recap::{numbers_in_facts, RecapFacts};

pub const TEMPLATE_ID: &str = "recap:v1";
pub const DISCLAIMER_EN: &str =
    "This recap is generated automatically from public data for information only. It is not investment advice.";
pub const DISCLAIMER_ZH: &str = "本回顾根据公开数据自动生成，仅供参考，不构成投资建议。";

const PROMPT_EN: &str = include_str!("../../prompts/recap_v1_en.md");
const PROMPT_ZH: &str = include_str!("../../prompts/recap_v1_zh.md");

pub fn headings(lang: Lang) -> [&'static str; 3] {
    match lang {
        Lang::En => [
            "Index closes",
            "Notable movers",
            "After-hours earnings to watch",
        ],
        Lang::Zh => ["指数收盘", "个股异动", "盘后财报关注"],
    }
}

fn prompt(lang: Lang) -> &'static str {
    match lang {
        Lang::En => PROMPT_EN,
        Lang::Zh => PROMPT_ZH,
    }
}

/// Chinese index names for the ZH table; facts keep the English labels.
fn index_label(label: &str, lang: Lang) -> &str {
    match (lang, label) {
        (Lang::Zh, "S&P 500") => "标普500",
        (Lang::Zh, "Nasdaq 100") => "纳斯达克100",
        (Lang::Zh, "Dow Jones") => "道琼斯",
        (Lang::Zh, "Russell 2000") => "罗素2000",
        _ => label,
    }
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

/// Everything above the sources list: title, summary, early-close note,
/// code-rendered tables and the LLM sections.
fn render_body(d: &Draft, f: &RecapFacts) -> String {
    let lang = d.lang;
    let mut out = format!("# {}\n\n{}\n\n", d.title, d.summary);
    if f.early_close {
        out.push_str(match lang {
            Lang::En => "_Shortened session: US markets closed early at 1:00 PM ET._\n\n",
            Lang::Zh => "_提前收盘：美股今日于美东时间下午1:00收盘。_\n\n",
        });
    }

    out.push_str(match lang {
        Lang::En => "| Index | Close | Change |\n|---|---:|---:|\n",
        Lang::Zh => "| 指数 | 收盘 | 涨跌 |\n|---|---:|---:|\n",
    });
    for (label, fq) in &f.indexes {
        let (last, chg) = match fq {
            Field::Value(q) => (fmt_num(q.last), fmt_pct(q.change_pct)),
            Field::Missing { .. } => (na(lang).to_string(), na(lang).to_string()),
        };
        out.push_str(&format!(
            "| {} | {last} | {chg} |\n",
            cell(index_label(label, lang))
        ));
    }
    out.push('\n');

    let movers: Vec<&Mover> = f.gainers.iter().chain(&f.losers).collect();
    if !movers.is_empty() {
        out.push_str(match lang {
            Lang::En => "| Ticker | Company | Close | Change | Why |\n|---|---|---:|---:|---|\n",
            Lang::Zh => "| 代码 | 公司 | 收盘价 | 涨跌 |\n|---|---|---:|---:|\n",
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
    out
}

fn sources(f: &RecapFacts) -> impl Iterator<Item = &NewsReason> {
    let after_hours = match &f.after_hours {
        Field::Value(items) => items.as_slice(),
        Field::Missing { .. } => &[],
    };
    f.gainers
        .iter()
        .chain(&f.losers)
        .flat_map(|m| &m.reasons)
        .chain(after_hours)
}

/// The full article: body, sources and disclaimer.
pub fn render_markdown(d: &Draft, f: &RecapFacts) -> String {
    let mut out = render_body(d, f);
    out.push_str(&render_sources(sources(f), d.lang));
    out.push_str(match d.lang {
        Lang::En => DISCLAIMER_EN,
        Lang::Zh => DISCLAIMER_ZH,
    });
    out.push('\n');
    out
}

/// Drafts one language from the facts (one retry, as for the premarket
/// brief), then checks every number in the rendered body, tables included.
/// Source link titles are verbatim citations and are not checked. Company
/// names come from the universe file, so their digits ("3M", "Phillips 66")
/// are not read as figures.
pub async fn write(
    llm: &dyn LlmProvider,
    f: &RecapFacts,
    lang: Lang,
) -> Result<(Draft, String), Vec<String>> {
    let facts_json =
        serde_json::to_string_pretty(f).map_err(|e| vec![format!("draft: facts json: {e}")])?;
    let mut allowed = numbers_in_facts(f);
    let names: Vec<&str> = f
        .gainers
        .iter()
        .chain(&f.losers)
        .map(|m| m.name.as_str())
        .collect();
    let (d, md) = write_with(
        llm,
        prompt(lang),
        &facts_json,
        &allowed,
        &names,
        &headings(lang),
        lang,
        |d| render_markdown(d, f),
    )
    .await?;
    allowed.extend(table_values(f));
    check(&blank_names(&render_body(&d, f), &names), lang, &allowed).map_err(|bad| {
        bad.into_iter()
            .map(|n| format!("rendered: {n}"))
            .collect::<Vec<_>>()
    })?;
    Ok((d, md))
}

/// The table cells exactly as printed. `format!` rounds ties half-to-even on
/// the binary value while the checker rounds half away from zero, so a fact
/// like 2230.125 prints as "2,230.12"; the printed value is still the fact.
fn table_values(f: &RecapFacts) -> Vec<f64> {
    let quotes = f
        .indexes
        .iter()
        .filter_map(|(_, fq)| match fq {
            Field::Value(q) => Some(q),
            Field::Missing { .. } => None,
        })
        .chain(f.gainers.iter().chain(&f.losers).map(|m| &m.quote));
    quotes
        .flat_map(|q| [format!("{:.2}", q.last), format!("{:.2}", q.change_pct)])
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Field, Mover, NewsReason, Quote};
    use crate::providers::{LlmProvider, ProviderError};
    use async_trait::async_trait;
    use chrono::NaiveDate;
    use serde_json::{json, Value};
    use std::cell::RefCell;

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
            volume: Some(210_000_000),
        }
    }
    fn reason(url: &str, title: &str) -> NewsReason {
        NewsReason {
            url: url.into(),
            title: title.into(),
            quote: "Nvidia shares rose 3% on Thursday".into(),
            summary_en: "New data center partnership.".into(),
        }
    }
    fn facts() -> RecapFacts {
        let idx = |l: &str, s: &str, last: f64, pct: f64| {
            (l.to_string(), Field::Value(quote(s, last, pct)))
        };
        RecapFacts {
            trading_date: NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
            as_of_utc: "2026-10-29T20:45:00Z".parse().unwrap(),
            early_close: false,
            indexes: vec![
                idx("S&P 500", "^GSPC", 5850.12, 0.5106),
                idx("Nasdaq 100", "^NDX", 20500.5, 0.82),
                idx("Dow Jones", "^DJI", 42100.0, -0.12),
                idx("Russell 2000", "^RUT", 2230.75, 1.05),
            ],
            gainers: vec![Mover {
                quote: quote("NVDA", 182.35, 3.08),
                name: "Nvidia".into(),
                reasons: vec![reason("https://news.test/nvda", "Nvidia rises")],
            }],
            losers: vec![],
            after_hours: Field::Missing {
                reason: "empty result".into(),
            },
        }
    }

    fn reply(lang: Lang, body: &str) -> Result<Value, ProviderError> {
        Ok(json!({
            "title": "Stocks close higher; Nvidia gains 3%",
            "summary": "The S&P 500 rose 0.51% as Nvidia climbed.",
            "sections": headings(lang).iter().map(|h| json!({"heading": h, "markdown": body})).collect::<Vec<_>>(),
        }))
    }
    fn clean(lang: Lang) -> Result<Value, ProviderError> {
        reply(lang, "The S&P 500 closed at 5,850.12, up 0.51%.")
    }
    fn bad(lang: Lang) -> Result<Value, ProviderError> {
        reply(lang, "The Nasdaq 100 rose 7.5%.")
    }

    #[tokio::test]
    async fn clean_en_draft_renders_tables() {
        let l = llm(vec![clean(Lang::En)]);
        let (d, md) = write(&l, &facts(), Lang::En).await.unwrap();
        assert_eq!(d.sections.len(), 3);
        assert_eq!(d.sections[2].0, "After-hours earnings to watch");
        let (sys, user) = l.prompts.borrow()[0].clone();
        assert_eq!(sys, PROMPT_EN);
        assert_eq!(user, serde_json::to_string_pretty(&facts()).unwrap());
        assert!(
            md.starts_with("# Stocks close higher; Nvidia gains 3%\n\n"),
            "{md}"
        );
        assert!(md.contains("| Index | Close | Change |"), "{md}");
        assert!(md.contains("| S&P 500 | 5,850.12 | +0.51% |"), "{md}");
        assert!(md.contains("| Dow Jones | 42,100.00 | -0.12% |"), "{md}");
        assert!(
            md.contains("| NVDA | Nvidia | 182.35 | +3.08% | New data center partnership. |"),
            "{md}"
        );
        assert!(md.contains("## Index closes") && md.contains("## Notable movers"));
        assert!(md.contains("## Sources\n\n- [Nvidia rises](https://news.test/nvda)"));
        assert!(!md.contains("early"), "{md}");
        assert!(md.trim_end().ends_with(DISCLAIMER_EN));
    }

    #[tokio::test]
    async fn unbacked_number_retried_then_fails() {
        let l = llm(vec![bad(Lang::En), clean(Lang::En)]);
        assert!(write(&l, &facts(), Lang::En).await.is_ok());
        assert!(l.prompts.borrow()[1].1.contains("7.5%"));
        let l = llm(vec![bad(Lang::En), bad(Lang::En)]);
        assert_eq!(
            write(&l, &facts(), Lang::En).await.unwrap_err(),
            vec!["7.5%"]
        );
    }

    #[tokio::test]
    async fn zh_draft_uses_zh_headings_and_disclaimer() {
        let l = llm(vec![clean(Lang::Zh)]);
        let (_, md) = write(&l, &facts(), Lang::Zh).await.unwrap();
        assert_eq!(l.prompts.borrow()[0].0, PROMPT_ZH);
        assert!(
            md.contains("## 指数收盘") && md.contains("## 盘后财报关注"),
            "{md}"
        );
        assert!(
            md.contains("| 指数 | 收盘 | 涨跌 |") && md.contains("| 代码 | 公司 | 收盘价 | 涨跌 |")
        );
        assert!(md.contains("| NVDA | Nvidia | 182.35 | +3.08% |") && !md.contains("partnership"));
        assert!(md.contains("## 来源"));
        assert!(md.trim_end().ends_with(DISCLAIMER_ZH) && !md.contains(DISCLAIMER_EN));
    }

    #[tokio::test]
    async fn en_headings_rejected_for_zh() {
        let l = llm(vec![clean(Lang::En), clean(Lang::En)]);
        let err = write(&l, &facts(), Lang::Zh).await.unwrap_err();
        assert!(err.len() == 1 && err[0].starts_with("draft: "), "{err:?}");
    }

    #[tokio::test]
    async fn early_close_note_in_both_languages() {
        let mut f = facts();
        f.early_close = true;
        let (_, en) = write(&llm(vec![clean(Lang::En)]), &f, Lang::En)
            .await
            .unwrap();
        assert!(en.contains("closed early at 1:00 PM ET"), "{en}");
        let (_, zh) = write(&llm(vec![clean(Lang::Zh)]), &f, Lang::Zh)
            .await
            .unwrap();
        assert!(zh.contains("提前收盘"), "{zh}");
    }

    #[test]
    fn missing_index_renders_na() {
        let mut f = facts();
        f.indexes[1].1 = Field::Missing { reason: "x".into() };
        let d = Draft {
            lang: Lang::Zh,
            title: "T".into(),
            summary: "S".into(),
            sections: vec![],
        };
        let md = render_markdown(&d, &f);
        assert!(md.contains("| 纳斯达克100 | 暂缺 | 暂缺 |"), "{md}");
        assert!(
            md.contains("| 标普500 | 5,850.12 | +0.51% |") && md.contains("| 罗素2000 |"),
            "{md}"
        );
    }

    #[tokio::test]
    async fn source_titles_are_not_checked_but_names_are_allowed() {
        let mut f = facts();
        f.gainers[0].reasons[0].title = "5 things to know before Nvidia's 99% run".into();
        f.losers = vec![Mover {
            quote: quote("MMM", 130.0, -2.5),
            name: "3M".into(),
            reasons: vec![],
        }];
        let (_, md) = write(&llm(vec![clean(Lang::En)]), &f, Lang::En)
            .await
            .unwrap();
        assert!(
            md.contains("99% run") && md.contains("| MMM | 3M | 130.00 | -2.50% | n/a |"),
            "{md}"
        );
    }

    #[tokio::test]
    async fn rendered_tables_are_checked() {
        // A summary_en number that is not in its quote never passes anchoring,
        // but if one slipped into the facts the rendered table must fail.
        let mut f = facts();
        f.gainers[0].reasons[0].summary_en = "Shares rose 44% on a deal.".into();
        let err = write(&llm(vec![clean(Lang::En)]), &f, Lang::En)
            .await
            .unwrap_err();
        assert_eq!(err, vec!["rendered: 44%"]);
    }

    #[tokio::test]
    async fn names_with_digits_pass_in_prose() {
        let mut f = facts();
        f.losers = vec![Mover {
            quote: quote("PSX", 130.0, -2.5),
            name: "Phillips 66".into(),
            reasons: vec![],
        }];
        let l = llm(vec![reply(Lang::En, "Phillips 66 fell 2.5%.")]);
        assert!(write(&l, &f, Lang::En).await.is_ok());
    }

    #[tokio::test]
    async fn half_way_values_render_and_pass() {
        let mut f = facts();
        if let Field::Value(q) = &mut f.indexes[3].1 {
            q.last = 2230.125;
            q.change_pct = 0.015;
        }
        let (_, md) = write(&llm(vec![clean(Lang::En)]), &f, Lang::En)
            .await
            .unwrap();
        assert!(md.contains("| Russell 2000 | 2,230.12 | +0.01% |"), "{md}");
    }

    #[tokio::test]
    async fn name_digits_do_not_back_other_numbers() {
        let mut f = facts();
        f.losers = vec![Mover {
            quote: quote("MMM", 130.0, -2.5),
            name: "3M".into(),
            reasons: vec![],
        }];
        let bad = || reply(Lang::En, "3M fell; Apple rose 9%.");
        let l = llm(vec![bad(), bad()]);
        assert_eq!(write(&l, &f, Lang::En).await.unwrap_err(), vec!["9%"]);
    }
}
