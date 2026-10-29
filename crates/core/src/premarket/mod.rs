//! Gathers everything the morning brief may say. Every news reason must carry
//! a verbatim quote that code verifies against the fetched article.

pub mod draft;
mod universe;

pub use universe::{company_name, universe, yahoo_symbol};

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;

use crate::anchor::verify_reason;
use crate::facts::{Field, Mover, NewsReason, PremarketFacts, Quote};
use crate::providers::{LlmProvider, MarketData, NewsSearch, ProviderError, SearchHit};

const FUTURES: [(&str, &str); 4] = [
    ("S&P 500 futures", "ES=F"),
    ("Nasdaq 100 futures", "NQ=F"),
    ("Dow futures", "YM=F"),
    ("Russell 2000 futures", "RTY=F"),
];
const MIN_PRICE: f64 = 5.0;
const MIN_VOLUME: u64 = 50_000;
const MOVERS_PER_SIDE: usize = 5;
const HITS_PER_MOVER: usize = 3;
const MOVER_REASONS: usize = 2;
const MACRO_QUERY: &str = "stock market futures today";
const MACRO_HITS: usize = 5;
const MACRO_ITEMS: usize = 3;
const MAX_CONTENT_CHARS: usize = 6_000;

fn system_prompt(task: &str, max: usize) -> String {
    format!(
        "You {task}. Use ONLY the provided articles; never use outside knowledge. \
         Return JSON of the form {{\"reasons\":[{{\"url\",\"title\",\"quote\",\"summary_en\"}}]}} \
         with at most {max} reasons. `url` and `title` must be copied from the article the reason comes from. \
         `quote` must be copied verbatim (at least 20 characters) from that article's text. \
         `summary_en` is at most 30 words and must not contain any number that is not in the quote. \
         If no article explains it, return {{\"reasons\":[]}}."
    )
}

fn articles_prompt(header: &str, hits: &[SearchHit]) -> String {
    let mut out = format!("{header}\n\nArticles:\n");
    for (i, h) in hits.iter().enumerate() {
        let content: String = h.content.chars().take(MAX_CONTENT_CHARS).collect();
        out.push_str(&format!(
            "\n[{}]\nurl: {}\ntitle: {}\ncontent: {}\n",
            i + 1,
            h.url,
            h.title,
            content
        ));
    }
    out
}

/// Ask the LLM for reasons and keep only those anchored in a fetched hit.
async fn extract(
    llm: &dyn LlmProvider,
    system: &str,
    user: &str,
    hits: &[SearchHit],
    max: usize,
) -> Result<Vec<NewsReason>, ProviderError> {
    let v = llm.complete_json(system, user).await?;
    let items = v
        .get("reasons")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(items
        .into_iter()
        .filter_map(|it| serde_json::from_value::<NewsReason>(it).ok())
        .filter_map(|r| {
            let h = hits
                .iter()
                .find(|h| h.url == r.url && verify_reason(&h.content, &r))?;
            // Title and url come from the fetched hit, never from the LLM.
            Some(NewsReason {
                url: h.url.clone(),
                title: h.title.clone(),
                ..r
            })
        })
        .take(max)
        .collect())
}

async fn mover_reasons(
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    ticker: &str,
    name: &str,
    change_pct: f64,
) -> Vec<NewsReason> {
    let Ok(hits) = ns
        .search(&format!("{name} ({ticker}) stock"), HITS_PER_MOVER)
        .await
    else {
        return vec![];
    };
    let system = system_prompt(
        "explain why a US stock is moving in premarket trading",
        MOVER_REASONS,
    );
    let header = format!("Ticker: {ticker}\nCompany: {name}\nPremarket change: {change_pct:+.2}%");
    extract(
        llm,
        &system,
        &articles_prompt(&header, &hits),
        &hits,
        MOVER_REASONS,
    )
    .await
    .unwrap_or_default()
}

async fn macro_items(ns: &dyn NewsSearch, llm: &dyn LlmProvider) -> Field<Vec<NewsReason>> {
    let hits = match ns.search(MACRO_QUERY, MACRO_HITS).await {
        Ok(h) => h,
        Err(e) => {
            return Field::Missing {
                reason: e.to_string(),
            }
        }
    };
    let system = system_prompt(
        "list the overnight and macro news items that matter for today's US stock market open",
        MACRO_ITEMS,
    );
    let header = "Topic: overnight and macro news ahead of the US stock market open";
    match extract(
        llm,
        &system,
        &articles_prompt(header, &hits),
        &hits,
        MACRO_ITEMS,
    )
    .await
    {
        Ok(v) => Field::Value(v),
        Err(e) => Field::Missing {
            reason: e.to_string(),
        },
    }
}

/// Universe quotes passing the price/volume filters, with `symbol` set to the universe ticker.
async fn liquid_quotes(md: &dyn MarketData) -> Vec<Quote> {
    let tickers = universe();
    let symbols: Vec<String> = tickers.iter().map(|t| yahoo_symbol(t)).collect();
    md.premarket_quotes(&symbols)
        .await
        .into_iter()
        .filter_map(|(sym, r)| {
            let mut q = r.ok()?;
            let i = symbols.iter().position(|s| *s == sym)?;
            q.symbol = tickers[i].clone();
            Some(q)
        })
        .filter(|q| q.last >= MIN_PRICE && q.volume.is_some_and(|v| v >= MIN_VOLUME))
        .collect()
}

pub async fn build_facts(
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    trading_date: NaiveDate,
    now: DateTime<Utc>,
) -> PremarketFacts {
    let mut futures = Vec::with_capacity(FUTURES.len());
    for (label, sym) in FUTURES {
        let f = match md.quote(sym).await {
            Ok(q) => Field::Value(q),
            Err(e) => Field::Missing {
                reason: e.to_string(),
            },
        };
        futures.push((label.to_string(), f));
    }

    let quotes = liquid_quotes(md).await;
    let mut up: Vec<Quote> = quotes
        .iter()
        .filter(|q| q.change_pct > 0.0)
        .cloned()
        .collect();
    up.sort_by(|a, b| b.change_pct.total_cmp(&a.change_pct));
    up.truncate(MOVERS_PER_SIDE);
    let mut down: Vec<Quote> = quotes.into_iter().filter(|q| q.change_pct < 0.0).collect();
    down.sort_by(|a, b| a.change_pct.total_cmp(&b.change_pct));
    down.truncate(MOVERS_PER_SIDE);

    let mut gainers = Vec::with_capacity(up.len());
    for q in up {
        gainers.push(to_mover(ns, llm, q).await);
    }
    let mut losers = Vec::with_capacity(down.len());
    for q in down {
        losers.push(to_mover(ns, llm, q).await);
    }

    PremarketFacts {
        trading_date,
        as_of_utc: now,
        futures,
        gainers,
        losers,
        macro_items: macro_items(ns, llm).await,
        watch_list: Field::Missing {
            reason: "not available in v1".into(),
        },
    }
}

async fn to_mover(ns: &dyn NewsSearch, llm: &dyn LlmProvider, quote: Quote) -> Mover {
    let name = company_name(&quote.symbol)
        .unwrap_or(&quote.symbol)
        .to_string();
    let reasons = mover_reasons(ns, llm, &quote.symbol, &name, quote.change_pct).await;
    Mover {
        quote,
        name,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{missing_required, Field, Quote};
    use crate::providers::{ProviderError, SearchHit};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::collections::HashMap;

    const ARTICLE: &str = "Apple shares jumped 8% in premarket trading after the company reported record iPhone sales for the quarter.";
    const QUOTE: &str = "Apple shares jumped 8% in premarket trading";
    const URL: &str = "https://news.test/aapl";

    fn q(sym: &str, last: f64, pct: f64, vol: Option<u64>) -> Quote {
        Quote {
            symbol: sym.into(),
            last,
            prev_close: last / (1.0 + pct / 100.0),
            change_pct: pct,
            volume: vol,
        }
    }

    struct FakeMd {
        futures: Option<Quote>,
        stocks: HashMap<String, Quote>,
    }
    #[async_trait(?Send)]
    impl MarketData for FakeMd {
        async fn quote(&self, symbol: &str) -> Result<Quote, ProviderError> {
            match &self.futures {
                Some(f) => Ok(Quote {
                    symbol: symbol.into(),
                    ..f.clone()
                }),
                None => Err(ProviderError::Http(429)),
            }
        }
        async fn premarket_quotes(
            &self,
            symbols: &[String],
        ) -> Vec<(String, Result<Quote, ProviderError>)> {
            symbols
                .iter()
                .map(|s| {
                    (
                        s.clone(),
                        self.stocks.get(s).cloned().ok_or(ProviderError::Http(404)),
                    )
                })
                .collect()
        }
    }
    fn md(stocks: &[Quote]) -> FakeMd {
        FakeMd {
            futures: Some(q("ES=F", 5800.0, 0.4, Some(1000))),
            stocks: stocks
                .iter()
                .map(|s| (s.symbol.clone(), s.clone()))
                .collect(),
        }
    }

    struct FakeNs(HashMap<String, Result<Vec<SearchHit>, ProviderError>>);
    #[async_trait(?Send)]
    impl NewsSearch for FakeNs {
        async fn search(&self, query: &str, _max: usize) -> Result<Vec<SearchHit>, ProviderError> {
            self.0
                .get(query)
                .cloned()
                .unwrap_or(Err(ProviderError::Empty))
        }
    }
    fn hit(url: &str, content: &str) -> SearchHit {
        SearchHit {
            url: url.into(),
            title: "Title".into(),
            content: content.into(),
        }
    }
    fn ns(items: Vec<(&str, Result<Vec<SearchHit>, ProviderError>)>) -> FakeNs {
        FakeNs(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// Answers with the first response whose needle occurs in the user prompt.
    struct FakeLlm {
        rules: Vec<(String, Result<Value, ProviderError>)>,
        prompts: RefCell<Vec<(String, String)>>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
            self.prompts.borrow_mut().push((system.into(), user.into()));
            self.rules
                .iter()
                .find(|(n, _)| user.contains(n.as_str()))
                .map(|(_, r)| r.clone())
                .unwrap_or(Ok(json!({"reasons": []})))
        }
    }
    fn llm(rules: Vec<(&str, Result<Value, ProviderError>)>) -> FakeLlm {
        FakeLlm {
            rules: rules.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            prompts: RefCell::new(vec![]),
        }
    }
    fn reasons(url: &str, quote: &str, summary: &str) -> Result<Value, ProviderError> {
        Ok(
            json!({"reasons": [{"url": url, "title": "Title", "quote": quote, "summary_en": summary}]}),
        )
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
    }
    fn now() -> DateTime<Utc> {
        "2026-10-29T12:45:00Z".parse().unwrap()
    }
    const AAPL_Q: &str = "Apple (AAPL) stock";

    #[tokio::test]
    async fn verified_reason_is_kept() {
        let m = md(&[q("AAPL", 230.0, 8.0, Some(2_000_000))]);
        let n = ns(vec![(AAPL_Q, Ok(vec![hit(URL, ARTICLE)]))]);
        let l = llm(vec![(
            "AAPL",
            reasons(URL, QUOTE, "Apple rose 8% on record iPhone sales."),
        )]);
        let f = build_facts(&m, &n, &l, date(), now()).await;
        assert_eq!(f.trading_date, date());
        assert_eq!(f.as_of_utc, now());
        assert_eq!(f.gainers.len(), 1);
        assert_eq!(f.gainers[0].name, "Apple");
        assert_eq!(f.gainers[0].reasons.len(), 1);
        assert_eq!(f.gainers[0].reasons[0].quote, QUOTE);
        assert_eq!(
            f.watch_list,
            Field::Missing {
                reason: "not available in v1".into()
            }
        );
        assert!(missing_required(&f).is_empty());
        let (sys, user) = &l.prompts.borrow()[0];
        assert!(sys.contains("verbatim") && sys.contains("ONLY"));
        assert!(user.contains(URL) && user.contains(ARTICLE) && user.contains("Apple"));
    }

    #[tokio::test]
    async fn unanchored_quote_is_dropped() {
        let m = md(&[q("AAPL", 230.0, 8.0, Some(2_000_000))]);
        let n = ns(vec![(AAPL_Q, Ok(vec![hit(URL, ARTICLE)]))]);
        let l = llm(vec![(
            "AAPL",
            reasons(
                URL,
                "Apple announced a surprise buyback of shares",
                "Buyback.",
            ),
        )]);
        let f = build_facts(&m, &n, &l, date(), now()).await;
        assert_eq!(f.gainers.len(), 1);
        assert_eq!(f.gainers[0].reasons, vec![]);
    }

    #[tokio::test]
    async fn kept_reason_carries_hit_title_not_llm_title() {
        let m = md(&[q("AAPL", 230.0, 8.0, Some(2_000_000))]);
        let n = ns(vec![(AAPL_Q, Ok(vec![hit(URL, ARTICLE)]))]);
        let l = llm(vec![(
            "AAPL",
            Ok(
                json!({"reasons": [{"url": URL, "title": "Apple to be acquired, sources say", "quote": QUOTE, "summary_en": "Apple rose."}]}),
            ),
        )]);
        let f = build_facts(&m, &n, &l, date(), now()).await;
        assert_eq!(f.gainers[0].reasons.len(), 1);
        assert_eq!(f.gainers[0].reasons[0].title, "Title");
        assert_eq!(f.gainers[0].reasons[0].url, URL);
    }

    #[tokio::test]
    async fn reason_with_foreign_url_is_dropped() {
        let m = md(&[q("AAPL", 230.0, 8.0, Some(2_000_000))]);
        let n = ns(vec![(AAPL_Q, Ok(vec![hit(URL, ARTICLE)]))]);
        let l = llm(vec![(
            "AAPL",
            reasons("https://elsewhere.test/x", QUOTE, "Apple rose."),
        )]);
        let f = build_facts(&m, &n, &l, date(), now()).await;
        assert_eq!(f.gainers[0].reasons, vec![]);
    }

    #[tokio::test]
    async fn llm_error_keeps_mover_with_no_reasons() {
        let m = md(&[
            q("AAPL", 230.0, 8.0, Some(2_000_000)),
            q("MSFT", 400.0, -6.0, Some(900_000)),
        ]);
        let n = ns(vec![(AAPL_Q, Ok(vec![hit(URL, ARTICLE)]))]);
        let l = llm(vec![("AAPL", Err(ProviderError::Http(500)))]);
        let f = build_facts(&m, &n, &l, date(), now()).await;
        assert_eq!(f.gainers.len(), 1);
        assert_eq!(f.gainers[0].quote.symbol, "AAPL");
        assert_eq!(f.gainers[0].reasons, vec![]);
        // MSFT search errors (no hits) -> still listed, no reasons.
        assert_eq!(f.losers.len(), 1);
        assert_eq!(f.losers[0].reasons, vec![]);
    }

    #[tokio::test]
    async fn penny_stock_is_never_a_mover() {
        let m = md(&[
            q("F", 3.10, 40.0, Some(5_000_000)),
            q("AAPL", 230.0, 2.0, Some(2_000_000)),
        ]);
        let f = build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert_eq!(
            f.gainers
                .iter()
                .map(|m| m.quote.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["AAPL"]
        );
    }

    #[tokio::test]
    async fn low_or_unknown_volume_is_excluded() {
        let m = md(&[
            q("AAPL", 230.0, 9.0, Some(49_999)),
            q("MSFT", 400.0, 9.0, None),
            q("NVDA", 180.0, 1.0, Some(50_000)),
        ]);
        let f = build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert_eq!(
            f.gainers
                .iter()
                .map(|m| m.quote.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["NVDA"]
        );
    }

    #[tokio::test]
    async fn gainers_only_positive_losers_only_negative_top_five() {
        let stocks: Vec<Quote> = ["AAPL", "MSFT", "NVDA", "AMZN", "META", "TSLA", "GOOGL"]
            .iter()
            .enumerate()
            .map(|(i, s)| q(s, 100.0, -(i as f64 + 1.0), Some(100_000)))
            .chain([q("NFLX", 100.0, 0.0, Some(100_000))])
            .collect();
        let f = build_facts(&md(&stocks), &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert!(f.gainers.is_empty());
        let pcts: Vec<f64> = f.losers.iter().map(|m| m.quote.change_pct).collect();
        assert_eq!(pcts, vec![-7.0, -6.0, -5.0, -4.0, -3.0]);
        assert_eq!(missing_required(&f), vec!["movers"]);
    }

    #[tokio::test]
    async fn gainers_sorted_desc_and_capped() {
        let stocks: Vec<Quote> = ["AAPL", "MSFT", "NVDA", "AMZN", "META", "TSLA"]
            .iter()
            .enumerate()
            .map(|(i, s)| q(s, 100.0, i as f64 + 1.0, Some(100_000)))
            .collect();
        let f = build_facts(&md(&stocks), &ns(vec![]), &llm(vec![]), date(), now()).await;
        let syms: Vec<&str> = f.gainers.iter().map(|m| m.quote.symbol.as_str()).collect();
        assert_eq!(syms, vec!["TSLA", "META", "AMZN", "NVDA", "MSFT"]);
        assert!(f.losers.is_empty());
    }

    #[tokio::test]
    async fn class_shares_map_back_to_universe_ticker() {
        let m = md(&[q("BRK-B", 480.0, 5.5, Some(300_000))]);
        let f = build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert_eq!(f.gainers[0].quote.symbol, "BRK.B");
        assert_eq!(f.gainers[0].name, "Berkshire Hathaway");
    }

    #[tokio::test]
    async fn all_futures_429_are_missing_and_gate() {
        let mut m = md(&[]);
        m.futures = None;
        let f = build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        let labels: Vec<&str> = f.futures.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "S&P 500 futures",
                "Nasdaq 100 futures",
                "Dow futures",
                "Russell 2000 futures"
            ]
        );
        for (_, fq) in &f.futures {
            assert_eq!(
                fq,
                &Field::Missing {
                    reason: ProviderError::Http(429).to_string()
                }
            );
        }
        assert!(missing_required(&f).contains(&"futures"));
    }

    #[tokio::test]
    async fn futures_use_expected_symbols() {
        let f = build_facts(&md(&[]), &ns(vec![]), &llm(vec![]), date(), now()).await;
        let syms: Vec<String> = f
            .futures
            .iter()
            .map(|(_, fq)| match fq {
                Field::Value(q) => q.symbol.clone(),
                Field::Missing { reason } => panic!("{reason}"),
            })
            .collect();
        assert_eq!(syms, vec!["ES=F", "NQ=F", "YM=F", "RTY=F"]);
    }

    #[tokio::test]
    async fn macro_items_verified() {
        let src =
            "Stock futures rose overnight as Treasury yields fell after the Fed signaled patience.";
        let n = ns(vec![(
            "stock market futures today",
            Ok(vec![hit("https://news.test/macro", src)]),
        )]);
        let l = llm(vec![(
            "overnight",
            Ok(json!({"reasons": [
                {"url": "https://news.test/macro", "title": "T", "quote": "Stock futures rose overnight as Treasury yields fell", "summary_en": "Futures rose as yields fell."},
                {"url": "https://news.test/macro", "title": "T", "quote": "Oil prices surged on supply worries overnight", "summary_en": "Oil up."}
            ]})),
        )]);
        let f = build_facts(&md(&[]), &n, &l, date(), now()).await;
        match f.macro_items {
            Field::Value(v) => {
                assert_eq!(v.len(), 1);
                assert!(v[0].quote.starts_with("Stock futures rose"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn macro_search_error_is_missing() {
        let f = build_facts(&md(&[]), &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert_eq!(
            f.macro_items,
            Field::Missing {
                reason: ProviderError::Empty.to_string()
            }
        );
    }

    #[tokio::test]
    async fn macro_llm_error_is_missing_and_zero_verified_is_empty() {
        let n = ns(vec![(
            "stock market futures today",
            Ok(vec![hit(
                "https://news.test/m",
                "Some macro text here, long enough.",
            )]),
        )]);
        let f = build_facts(
            &md(&[]),
            &n,
            &llm(vec![("overnight", Err(ProviderError::Http(503)))]),
            date(),
            now(),
        )
        .await;
        assert_eq!(
            f.macro_items,
            Field::Missing {
                reason: ProviderError::Http(503).to_string()
            }
        );
        let f = build_facts(&md(&[]), &n, &llm(vec![]), date(), now()).await;
        assert_eq!(f.macro_items, Field::Value(vec![]));
    }
}
