//! Turns recap facts into one language's article, with the same drafting
//! loop, markup filter and number check as the premarket brief. Code renders
//! the tables from the facts, and the rendered body is number-checked too.

use crate::facts::{Field, Mover, NewsReason};
use crate::numbers::{check, Lang};
use crate::premarket::draft::{
    blank_names, cell, fmt_num, fmt_pct, na, no_news, render_sources, render_themes, section,
    theme_allowed, theme_names, without_link_targets, write_with, Draft, MOVERS, THEMES,
};
use crate::providers::LlmProvider;
use crate::recap::{numbers_in_facts, RecapFacts, Reported};

pub const TEMPLATE_ID: &str = "recap:v1";
pub const DISCLAIMER_EN: &str =
    "This recap is generated automatically from public data for information only. It is not investment advice.";
pub const DISCLAIMER_ZH: &str = "本回顾根据公开数据自动生成，仅供参考，不构成投资建议。";

const PROMPT_EN: &str = include_str!("../../prompts/recap_v1_en.md");
const PROMPT_ZH: &str = include_str!("../../prompts/recap_v1_zh.md");

/// Every section heading the recap can have, in order, matching the
/// premarket brief's: the story (macro, themes) before the single names.
pub fn headings(lang: Lang) -> [&'static str; 5] {
    match lang {
        Lang::En => [
            "Index closes",
            "Macro & after the close",
            "Themes",
            "Notable movers",
            "After-hours earnings to watch",
        ],
        Lang::Zh => [
            "指数收盘",
            "宏观与盘后",
            "板块动向",
            "个股异动",
            "盘后财报关注",
        ],
    }
}

/// The headings these facts call for: closes and movers always; macro,
/// themes and after-hours only when there is something to say in them.
fn wanted(f: &RecapFacts, lang: Lang) -> Vec<&'static str> {
    let [closes, macro_heading, themes, movers, after] = headings(lang);
    let macro_news = matches!(&f.macro_items, Field::Value(v) if !v.is_empty());
    let ahead = matches!(&f.after_hours, Field::Value(v) if !v.is_empty());
    [
        (closes, true),
        (macro_heading, macro_news),
        (themes, !f.themes.is_empty()),
        (movers, true),
        (after, ahead || !f.reported.is_empty()),
    ]
    .into_iter()
    .filter_map(|(h, on)| on.then_some(h))
    .collect()
}

/// The code-rendered end of the after-hours section, linking each earnings
/// check already published: "Already reported after the close: [Apple
/// (AAPL)](/en/p/aapl-q4-fy2026-earnings)." `None` when there is none.
fn reported_line(reported: &[Reported], lang: Lang) -> Option<String> {
    if reported.is_empty() {
        return None;
    }
    let links: Vec<String> = reported
        .iter()
        .map(|r| {
            let name = cell(&r.company).replace('[', "\\[").replace(']', "\\]");
            let ticker = cell(&r.ticker);
            match lang {
                Lang::En => format!("[{name} ({ticker})](/en/p/{})", r.slug),
                Lang::Zh => format!("[{name}（{ticker}）](/zh/p/{})", r.slug),
            }
        })
        .collect();
    Some(match lang {
        Lang::En => format!("Already reported after the close: {}.", links.join(", ")),
        Lang::Zh => format!("已于盘后发布财报：{}。", links.join("、")),
    })
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
            .map_or(no_news(lang), |r| r.summary_en.as_str());
        row.push_str(&format!(" {} |", cell(why)));
    }
    row
}

/// Everything above the sources list: title, summary, early-close note, the
/// index table, then the sections in order (the themes list heads "Themes",
/// the movers table heads the movers section, the reported line ends the
/// after-hours section).
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

    let mut movers = String::new();
    let listed: Vec<&Mover> = f.gainers.iter().chain(&f.losers).collect();
    if !listed.is_empty() {
        movers.push_str(match lang {
            Lang::En => "| Ticker | Company | Close | Change | Why |\n|---|---|---:|---:|---|\n",
            Lang::Zh => "| 代码 | 公司 | 收盘价 | 涨跌 |\n|---|---|---:|---:|\n",
        });
        for m in listed {
            movers.push_str(&mover_row(m, lang));
            movers.push('\n');
        }
    }

    let heads = headings(lang);
    let after = heads[4];
    let themes = render_themes(&f.themes, lang);
    let reported = reported_line(&f.reported, lang);
    for (heading, body) in &d.sections {
        let body = body.trim();
        let (code, text) = if *heading == heads[THEMES] {
            (Some(themes.as_str()), body.to_string())
        } else if *heading == heads[MOVERS] {
            (Some(movers.as_str()), body.to_string())
        } else {
            let text = match &reported {
                Some(line) if *heading == after && body.is_empty() => line.clone(),
                Some(line) if *heading == after => format!("{body}\n\n{line}"),
                _ => body.to_string(),
            };
            (None, text)
        };
        out.push_str(&section(heading, code, &text));
    }
    out
}

/// Every reason the recap cites (movers, macro, themes, after-hours), in that
/// order and possibly repeated; the sources list and the stored sources
/// dedupe it.
pub(crate) fn sources(f: &RecapFacts) -> impl Iterator<Item = &NewsReason> {
    fn items(field: &Field<Vec<NewsReason>>) -> &[NewsReason] {
        match field {
            Field::Value(items) => items.as_slice(),
            Field::Missing { .. } => &[],
        }
    }
    f.gainers
        .iter()
        .chain(&f.losers)
        .flat_map(|m| &m.reasons)
        .chain(items(&f.macro_items))
        .chain(f.themes.iter().flat_map(|t| &t.reasons))
        .chain(items(&f.after_hours))
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
/// Source link titles are verbatim citations and are not checked, nor are
/// link targets (post slugs). Company names come from the universe file and
/// the earnings checks, so their digits ("3M", "Phillips 66") are not read
/// as figures.
pub async fn write(
    llm: &dyn LlmProvider,
    f: &RecapFacts,
    lang: Lang,
) -> Result<(Draft, String), Vec<String>> {
    // Diagnostics and the theme tuning stats are operational counters, not
    // facts the recap may cite.
    let for_llm = RecapFacts {
        diagnostics: vec![],
        theme_candidates: vec![],
        ..f.clone()
    };
    let facts_json = serde_json::to_string_pretty(&for_llm)
        .map_err(|e| vec![format!("draft: facts json: {e}")])?;
    let mut allowed = numbers_in_facts(f);
    allowed.extend(theme_allowed(&f.themes));
    let names: Vec<&str> = f
        .gainers
        .iter()
        .chain(&f.losers)
        .map(|m| m.name.as_str())
        .chain(f.reported.iter().map(|r| r.company.as_str()))
        .chain(theme_names(&f.themes))
        .collect();
    let (d, md) = write_with(
        llm,
        prompt(lang),
        &facts_json,
        &allowed,
        &names,
        &wanted(f, lang),
        &headings(lang),
        (&[], &[]),
        lang,
        |d| render_markdown(d, f),
    )
    .await?;
    allowed.extend(table_values(f));
    let body = without_link_targets(&render_body(&d, f));
    check(&blank_names(&body, &names), lang, &allowed).map_err(|bad| {
        bad.into_iter()
            .map(|n| format!("rendered: {n}"))
            .collect::<Vec<_>>()
    })?;
    Ok((d, md))
}

/// The table cells and theme-list figures exactly as printed. `format!`
/// rounds ties half-to-even on the binary value while the checker rounds
/// half away from zero, so a fact like 2230.125 prints as "2,230.12"; the
/// printed value is still the fact.
fn table_values(f: &RecapFacts) -> Vec<f64> {
    let quotes = f
        .indexes
        .iter()
        .filter_map(|(_, fq)| match fq {
            Field::Value(q) => Some(q),
            Field::Missing { .. } => None,
        })
        .chain(f.gainers.iter().chain(&f.losers).map(|m| &m.quote))
        .flat_map(|q| [q.last, q.change_pct]);
    let themes = f
        .themes
        .iter()
        .flat_map(|t| std::iter::once(t.median_pct).chain(t.members.iter().map(|m| m.change_pct)));
    quotes
        .chain(themes)
        .map(|v| format!("{v:.2}"))
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Field, Mover, NewsReason, Quote};
    use crate::premarket::draft::{render_themes, tests::two_themes};
    use crate::providers::{LlmProvider, ProviderError};
    use crate::recap::Reported;
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
            macro_items: Field::Missing {
                reason: "empty result".into(),
            },
            after_hours: Field::Missing {
                reason: "empty result".into(),
            },
            reported: vec![],
            themes: vec![],
            theme_candidates: vec![],
            diagnostics: vec![],
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

    #[tokio::test]
    async fn clean_en_draft_renders_tables() {
        let l = llm(vec![clean(Lang::En)]);
        let (d, md) = write(&l, &facts(), Lang::En).await.unwrap();
        // No macro news, no themes, no after-hours news and nothing
        // reported: those sections of the reply are dropped unread, and no
        // "n/a" section is printed.
        assert_eq!(d.sections.len(), 2);
        assert!(!md.contains("After-hours") && !md.contains("n/a"), "{md}");
        let (sys, user) = l.prompts.borrow()[0].clone();
        assert_eq!(sys, PROMPT_EN);
        assert_eq!(
            user,
            format!(
                "{}\n\nWrite exactly these sections, in this order: \"Index closes\", \"Notable movers\"",
                serde_json::to_string_pretty(&facts()).unwrap()
            )
        );
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

    fn apple() -> Reported {
        Reported {
            ticker: "AAPL".into(),
            company: "Apple".into(),
            slug: "aapl-q4-fy2026-earnings".into(),
        }
    }

    #[tokio::test]
    async fn reported_checks_get_the_after_hours_section_and_a_link_line() {
        let mut f = facts();
        f.reported = vec![apple()];
        for (lang, line, heading) in [
            (
                Lang::En,
                "The S&P 500 closed at 5,850.12, up 0.51%.\n\nAlready reported after the close: [Apple (AAPL)](/en/p/aapl-q4-fy2026-earnings).\n\n",
                "After-hours earnings to watch",
            ),
            (
                Lang::Zh,
                "The S&P 500 closed at 5,850.12, up 0.51%.\n\n已于盘后发布财报：[Apple（AAPL）](/zh/p/aapl-q4-fy2026-earnings)。\n\n",
                "盘后财报关注",
            ),
        ] {
            let l = llm(vec![clean(lang)]);
            let (d, md) = write(&l, &f, lang).await.unwrap();
            assert_eq!(d.sections.len(), 3);
            assert!(md.contains(&format!("## {heading}\n\n{line}")), "{md}");
            let user = &l.prompts.borrow()[0].1;
            assert!(user.ends_with(&format!("\"{heading}\"")), "{user}");
            // The link line is code's: the LLM's draft does not carry it.
            assert!(!d.sections[2].1.contains("/p/"));
        }
    }

    #[tokio::test]
    async fn several_reported_join_and_slug_digits_pass_the_check() {
        let mut f = facts();
        f.reported = vec![
            apple(),
            Reported {
                ticker: "MMM".into(),
                company: "3M".into(),
                // A colliding slug carries the accession's last digits.
                slug: "mmm-q3-fy2026-earnings-000070".into(),
            },
        ];
        let (_, md) = write(&llm(vec![clean(Lang::En)]), &f, Lang::En)
            .await
            .unwrap();
        assert!(
            md.contains("Already reported after the close: [Apple (AAPL)](/en/p/aapl-q4-fy2026-earnings), [3M (MMM)](/en/p/mmm-q3-fy2026-earnings-000070)."),
            "{md}"
        );
        let (_, zh) = write(&llm(vec![clean(Lang::Zh)]), &f, Lang::Zh)
            .await
            .unwrap();
        assert!(
            zh.contains("(/zh/p/aapl-q4-fy2026-earnings)、[3M（MMM）]"),
            "{zh}"
        );
    }

    #[tokio::test]
    async fn reported_line_stands_alone_without_llm_text() {
        // No after-hours items: the prompt asks for an empty body there, so
        // the section says "reported" once, in code's line.
        let mut f = facts();
        f.reported = vec![apple()];
        for (lang, want) in [
            (
                Lang::En,
                "## After-hours earnings to watch\n\nAlready reported after the close: [Apple (AAPL)](/en/p/aapl-q4-fy2026-earnings).\n\n## Sources",
            ),
            (
                Lang::Zh,
                "## 盘后财报关注\n\n已于盘后发布财报：[Apple（AAPL）](/zh/p/aapl-q4-fy2026-earnings)。\n\n## 来源",
            ),
        ] {
            let mut v = clean(lang).unwrap();
            v["sections"][4]["markdown"] = json!("");
            let (d, md) = write(&llm(vec![Ok(v)]), &f, lang).await.unwrap();
            assert_eq!(d.sections[2].1, "");
            assert!(md.contains(want), "{md}");
        }
    }

    #[test]
    fn prompts_describe_reported_and_take_sections_from_the_user_message() {
        assert!(
            PROMPT_EN.contains("never say they are scheduled to report")
                && PROMPT_EN.contains("When `after_hours` has no items, leave the \"After-hours earnings to watch\" markdown empty")
                && PROMPT_EN.contains("listed at the end of the user message")
                && !PROMPT_EN.contains("n/a"),
            "{PROMPT_EN}"
        );
        assert!(
            PROMPT_ZH.contains("不得说它们将要发布财报")
                && PROMPT_ZH.contains("`after_hours` 没有条目时，“盘后财报关注”的 `markdown` 留空")
                && PROMPT_ZH.contains("列在用户消息末尾")
                && !PROMPT_ZH.contains("暂缺"),
            "{PROMPT_ZH}"
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
        let mut f = facts();
        f.after_hours = Field::Value(vec![reason("https://news.test/ah", "Intel reports")]);
        let l = llm(vec![clean(Lang::Zh)]);
        let (_, md) = write(&l, &f, Lang::Zh).await.unwrap();
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
            md.contains("99% run") && md.contains("| MMM | 3M | 130.00 | -2.50% | No clear news |"),
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

    #[test]
    fn headings_put_macro_then_themes_then_movers() {
        let mut f = facts();
        f.macro_items = Field::Value(vec![reason("https://news.test/m", "Yields rise")]);
        f.themes = two_themes();
        f.reported = vec![apple()];
        assert_eq!(
            wanted(&f, Lang::En),
            [
                "Index closes",
                "Macro & after the close",
                "Themes",
                "Notable movers",
                "After-hours earnings to watch"
            ]
        );
        assert_eq!(
            wanted(&f, Lang::Zh),
            [
                "指数收盘",
                "宏观与盘后",
                "板块动向",
                "个股异动",
                "盘后财报关注"
            ]
        );
        assert_eq!(wanted(&f, Lang::Zh), headings(Lang::Zh));
        // Each optional section comes and goes on its own facts.
        f.macro_items = Field::Value(vec![]);
        f.reported = vec![];
        assert_eq!(
            wanted(&f, Lang::En),
            ["Index closes", "Themes", "Notable movers"]
        );
        f.themes = vec![];
        assert_eq!(wanted(&f, Lang::En), ["Index closes", "Notable movers"]);
    }

    #[tokio::test]
    async fn recap_with_themes_passes_rendered_number_check() {
        let mut f = facts();
        f.themes = two_themes();
        f.themes[1].more = 12;
        // A member name with digits, as in the movers table.
        f.themes[1].members[1].name = "Phillips 66".into();
        // A half-way move prints as "+2.62%", which only the printed values
        // back (the checker rounds 2.625 away from zero).
        f.themes[0].members[1].change_pct = 2.625;
        f.macro_items = Field::Value(vec![reason("https://news.test/m", "Yields rise")]);
        for lang in [Lang::En, Lang::Zh] {
            let mut v = reply(lang, "Quiet.").unwrap();
            v["sections"][2]["markdown"] = json!(match lang {
                Lang::En => "Chips rose; NVDA gained 3.03%. Phillips 66 slipped 3.90% with memory names, and 12 more fell.",
                Lang::Zh => "芯片股上涨，NVDA涨3.03%。存储股走低，MU跌4.12%。",
            });
            let (d, md) = write(&llm(vec![Ok(v)]), &f, lang).await.unwrap();
            assert_eq!(d.sections.len(), 4);
            let list = render_themes(&f.themes, lang);
            assert!(
                list.contains("MU −4.12%") && list.contains("AMD +2.62%"),
                "{list}"
            );
            assert!(
                list.contains(match lang {
                    Lang::En => "and 12 more",
                    Lang::Zh => "另有 12 只",
                }),
                "{list}"
            );
            let themes = headings(lang)[2];
            assert!(md.contains(&format!("## {themes}\n\n{list}\n\n")), "{md}");
            // The macro section comes before the themes and its sources are cited.
            let macro_h = headings(lang)[1];
            assert!(md.find(&format!("## {macro_h}")) < md.find(&format!("## {themes}")));
            assert!(
                md.contains("(https://news.test/m)") && md.contains("(https://news.test/chips)")
            );
        }
        // A group label's digits are blanked like a company name's.
        f.themes[0].group = "Semis 2.0".into();
        let ok = write(&llm(vec![reply(Lang::En, "Quiet.")]), &f, Lang::En).await;
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn recap_sources_include_macro_and_theme_reasons() {
        let mut f = facts();
        f.macro_items = Field::Value(vec![reason("https://news.test/m", "Yields rise")]);
        f.themes = two_themes();
        let urls: Vec<&str> = sources(&f).map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://news.test/nvda",
                "https://news.test/m",
                "https://news.test/chips"
            ]
        );
    }

    #[test]
    fn prompts_describe_the_themes_section() {
        assert!(
            PROMPT_EN.contains("one short paragraph per theme in the listed order; cite only the listed moves; a theme without a reason gets one sentence stating the move and gives no cause; do not repeat the list line")
                && PROMPT_EN.contains("\"Themes\" exists only when `themes` is not empty")
                && PROMPT_EN.contains("\"Macro & after the close\" exists only when `macro_items` has items"),
            "{PROMPT_EN}"
        );
        assert!(
            PROMPT_ZH.contains("每个主题写一小段，按列出的顺序；只引用列出的涨跌幅；没有原因的主题用一句话说明涨跌，不写原因；不要重复列表那一行")
                && PROMPT_ZH.contains("只有 `themes` 不为空时才有“板块动向”")
                && PROMPT_ZH.contains("`themes` 列出成分股同向大幅波动的行业板块")
                && PROMPT_ZH.contains("只有 `macro_items` 有条目时才有“宏观与盘后”"),
            "{PROMPT_ZH}"
        );
    }
}
