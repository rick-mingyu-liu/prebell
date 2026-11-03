//! The after-close recap: index closes (required), notable movers with
//! quote-anchored reasons, and after-hours earnings to watch (optional).

pub mod draft;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::clock::{recap_window, trading_day, DayKind, Window};
use crate::facts::{quote_numbers, Field, Mover, NewsReason, Quote};
use crate::numbers::Lang;
use crate::premarket::draft::Draft;
use crate::premarket::{
    articles_prompt, as_of, company_name, extract, fresh_news, system_prompt, universe,
    yahoo_symbol, Diag, EVENT_RULE,
};
use crate::providers::{LlmProvider, MarketData, NewsSearch};
use crate::run::{tick_content, Content, Hooks, TickOutcome};
use crate::store::Store;

/// `(label, Yahoo symbol)`: the indexes themselves, not ETF proxies, so the
/// levels match what readers see quoted at the close.
const INDEXES: [(&str, &str); 4] = [
    ("S&P 500", "^GSPC"),
    ("Nasdaq 100", "^NDX"),
    ("Dow Jones", "^DJI"),
    ("Russell 2000", "^RUT"),
];
const MIN_PRICE: f64 = 5.0;
/// Regular-session shares; every S&P 500 name clears this on a normal day.
const MIN_VOLUME: u64 = 1_000_000;
const MOVERS_PER_SIDE: usize = 5;
const HITS_PER_MOVER: usize = 3;
const MOVER_REASONS: usize = 2;
const AFTER_HOURS_HITS: usize = 5;
const AFTER_HOURS_ITEMS: usize = 5;
/// After-hours previews may come from earlier in the week.
const AFTER_HOURS_MAX_AGE_DAYS: i64 = 7;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RecapFacts {
    pub trading_date: NaiveDate,
    pub as_of_utc: DateTime<Utc>,
    /// True on NYSE early-close days (1:00 PM ET close).
    pub early_close: bool,
    /// `(label, close)` for the four indexes, in `INDEXES` order.
    pub indexes: Vec<(String, Field<Quote>)>,
    pub gainers: Vec<Mover>,
    pub losers: Vec<Mover>,
    /// Companies the news says report after today's close.
    pub after_hours: Field<Vec<NewsReason>>,
    /// Short counters from the build, for logs and `last_error`. Never sent
    /// to the LLM.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

/// Names of required sections that are unavailable. Empty means publishable.
/// Every index close is required: a recap missing one is thin, so the run
/// retries and then skips instead.
pub fn missing_required(f: &RecapFacts) -> Vec<&'static str> {
    if f.indexes.len() == INDEXES.len()
        && f.indexes.iter().all(|(_, q)| matches!(q, Field::Value(_)))
    {
        vec![]
    } else {
        vec!["indexes"]
    }
}

fn push_quote(out: &mut Vec<f64>, q: &Quote) {
    out.extend([q.last, q.prev_close, q.change_pct]);
    if let Some(v) = q.volume {
        out.push(v as f64);
    }
}

/// Every number a draft may cite: quote fields, numbers inside source
/// quotes, and the trading year and its neighbours.
pub fn numbers_in_facts(f: &RecapFacts) -> Vec<f64> {
    let year = f64::from(f.trading_date.year());
    let mut out = vec![year - 1.0, year, year + 1.0];
    for (_, fq) in &f.indexes {
        if let Field::Value(q) = fq {
            push_quote(&mut out, q);
        }
    }
    for m in f.gainers.iter().chain(&f.losers) {
        push_quote(&mut out, &m.quote);
        for r in &m.reasons {
            out.extend(quote_numbers(&r.quote));
        }
    }
    if let Field::Value(items) = &f.after_hours {
        for r in items {
            out.extend(quote_numbers(&r.quote));
        }
    }
    out
}

/// "October 29, 2026".
fn long_date(d: NaiveDate) -> String {
    d.format("%B %-d, %Y").to_string()
}

async fn mover(
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    quote: Quote,
    now: DateTime<Utc>,
    d: &mut Diag,
) -> Mover {
    let name = company_name(&quote.symbol)
        .unwrap_or(&quote.symbol)
        .to_string();
    let ticker = &quote.symbol;
    let query = format!("{name} ({ticker}) stock");
    let reasons = match fresh_news(ns, &query, HITS_PER_MOVER, now, d).await {
        Ok(hits) => {
            let system = format!(
                "{} {EVENT_RULE}",
                system_prompt(
                    "explain why a US stock moved in today's regular trading session",
                    MOVER_REASONS,
                )
            );
            let header = format!(
                "Ticker: {ticker}\nCompany: {name}\nChange today: {:+.2}%\n{}",
                quote.change_pct,
                as_of(now)
            );
            extract(
                llm,
                &system,
                &articles_prompt(&header, &hits),
                &hits,
                MOVER_REASONS,
                true,
                d,
            )
            .await
            .unwrap_or_default()
        }
        Err(_) => {
            d.search_err += 1;
            vec![]
        }
    };
    Mover {
        quote,
        name,
        reasons,
    }
}

async fn after_hours(
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    date: NaiveDate,
    now: DateTime<Utc>,
    d: &mut Diag,
) -> Field<Vec<NewsReason>> {
    let day = long_date(date);
    // Not held to the news age cutoff: a preview from earlier in the week
    // still names tonight's reports, and the prompt pins the date.
    let since = now - Duration::days(AFTER_HOURS_MAX_AGE_DAYS);
    let hits = match ns
        .search_since(
            &format!("earnings after the bell {day}"),
            AFTER_HOURS_HITS,
            since,
        )
        .await
    {
        Ok(h) => h,
        Err(e) => {
            d.search_err += 1;
            return Field::Missing {
                reason: e.to_string(),
            };
        }
    };
    let system = system_prompt(
        &format!(
            "list the companies that the articles say report earnings after the close on {day}. \
             Skip any company whose article does not clearly say it reports after the close on that date"
        ),
        AFTER_HOURS_ITEMS,
    );
    let header = format!("Topic: companies reporting earnings after the US market close on {day}");
    match extract(
        llm,
        &system,
        &articles_prompt(&header, &hits),
        &hits,
        AFTER_HOURS_ITEMS,
        false,
        d,
    )
    .await
    {
        Ok(v) => Field::Value(v),
        Err(e) => Field::Missing {
            reason: e.to_string(),
        },
    }
}

/// Universe closes passing the price/volume filters, `symbol` set to the universe ticker.
async fn liquid_closes(md: &dyn MarketData, date: NaiveDate, d: &mut Diag) -> Vec<Quote> {
    let tickers = universe();
    let symbols: Vec<String> = tickers.iter().map(|t| yahoo_symbol(t)).collect();
    let quotes = md.close_quotes(&symbols, date).await;
    d.market = md.diagnostics();
    let liquid: Vec<Quote> = quotes
        .into_iter()
        .inspect(|(_, r)| d.quote(r))
        .filter_map(|(sym, r)| {
            let mut q = r.ok()?;
            let i = symbols.iter().position(|s| *s == sym)?;
            q.symbol = tickers[i].clone();
            Some(q)
        })
        .filter(|q| q.last >= MIN_PRICE && q.volume.is_some_and(|v| v >= MIN_VOLUME))
        .collect();
    d.liquid = liquid.len();
    liquid
}

pub async fn build_facts(
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    trading_date: NaiveDate,
    now: DateTime<Utc>,
) -> RecapFacts {
    let mut d = Diag::default();
    // Four chart requests, one per index, kept apart from the batched v7
    // movers scan: the required section then needs no cookie/crumb session,
    // and a v7 failure costs only the optional movers.
    let mut indexes: Vec<(String, Field<Quote>)> = Vec::with_capacity(INDEXES.len());
    for (label, sym) in INDEXES {
        let f = match md.close_quote(sym, trading_date).await {
            Ok(q) => {
                d.futures_ok += 1;
                Field::Value(q)
            }
            Err(e) => {
                d.futures_err += 1;
                Field::Missing {
                    reason: e.to_string(),
                }
            }
        };
        indexes.push((label.to_string(), f));
    }
    let early_close = trading_day(trading_date) == DayKind::EarlyClose;

    // The run retries until every index close is in; skip the costly movers
    // and news fan-out until then.
    if indexes
        .iter()
        .any(|(_, f)| matches!(f, Field::Missing { .. }))
    {
        return RecapFacts {
            trading_date,
            as_of_utc: now,
            early_close,
            indexes,
            gainers: vec![],
            losers: vec![],
            after_hours: Field::Missing {
                reason: "not gathered: index closes unavailable".into(),
            },
            diagnostics: d.lines_as("indexes"),
        };
    }

    let quotes = liquid_closes(md, trading_date, &mut d).await;
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
    d.gainers = up.len();
    d.losers = down.len();

    let mut gainers = Vec::with_capacity(up.len());
    for q in up {
        gainers.push(mover(ns, llm, q, now, &mut d).await);
    }
    let mut losers = Vec::with_capacity(down.len());
    for q in down {
        losers.push(mover(ns, llm, q, now, &mut d).await);
    }

    RecapFacts {
        trading_date,
        as_of_utc: now,
        early_close,
        indexes,
        gainers,
        losers,
        after_hours: after_hours(ns, llm, trading_date, now, &mut d).await,
        diagnostics: d.lines_as("indexes"),
    }
}

/// The after-close recap as a scheduled content type.
pub struct Recap<'a> {
    pub md: &'a dyn MarketData,
    pub ns: &'a dyn NewsSearch,
    pub llm: &'a dyn LlmProvider,
}

#[async_trait(?Send)]
impl Content for Recap<'_> {
    type Facts = RecapFacts;
    fn content_type(&self) -> &'static str {
        "recap"
    }
    fn template_id(&self) -> &'static str {
        draft::TEMPLATE_ID
    }
    fn label(&self) -> &'static str {
        "after-close recap"
    }
    fn window(&self, day: DayKind) -> Window {
        recap_window(day)
    }
    /// Three hours past the 19:00 ET publish deadline.
    fn zh_give_up(&self) -> NaiveTime {
        NaiveTime::from_hms_opt(22, 0, 0).expect("valid built-in time")
    }
    /// Hourly, on the hour: the recap crons fire every 5 minutes until
    /// 19:55 ET, and a ZH redraft each time would only repeat the failure.
    fn zh_retry_due(&self, t: NaiveTime) -> bool {
        t.minute() == 0
    }
    async fn build_facts(&self, date: NaiveDate, now: DateTime<Utc>) -> RecapFacts {
        build_facts(self.md, self.ns, self.llm, date, now).await
    }
    fn missing_required(&self, f: &RecapFacts) -> Vec<&'static str> {
        missing_required(f)
    }
    fn diagnostics(&self, f: &RecapFacts) -> Vec<String> {
        f.diagnostics.clone()
    }
    async fn write(&self, f: &RecapFacts, lang: Lang) -> Result<(Draft, String), Vec<String>> {
        draft::write(self.llm, f, lang).await
    }
    /// Every kept reason (movers, then after-hours), first occurrence per url.
    fn sources(&self, f: &RecapFacts) -> Vec<NewsReason> {
        let after_hours = match &f.after_hours {
            Field::Value(v) => v.as_slice(),
            Field::Missing { .. } => &[],
        };
        let mut out: Vec<NewsReason> = Vec::new();
        for r in f
            .gainers
            .iter()
            .chain(&f.losers)
            .flat_map(|m| &m.reasons)
            .chain(after_hours)
        {
            if !out.iter().any(|o| o.url == r.url) {
                out.push(r.clone());
            }
        }
        out
    }
}

/// One recap cron firing. A store error is reported as `Retrying`.
pub async fn tick(
    now: DateTime<Utc>,
    store: &dyn Store,
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
) -> TickOutcome {
    tick_with_log(now, store, md, ns, llm, &|_| {}).await
}

/// `tick`, passing the build's diagnostics line to `log` once when it builds.
pub async fn tick_with_log(
    now: DateTime<Utc>,
    store: &dyn Store,
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    log: &dyn Fn(&str),
) -> TickOutcome {
    let hooks = Hooks {
        log,
        extra: &Vec::new,
    };
    tick_with_hooks(now, store, md, ns, llm, &hooks).await
}

/// `tick` with the caller's `Hooks`.
pub async fn tick_with_hooks(
    now: DateTime<Utc>,
    store: &dyn Store,
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    hooks: &Hooks<'_>,
) -> TickOutcome {
    tick_content(now, store, &Recap { md, ns, llm }, hooks).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Field, NewsReason, Quote};
    use crate::providers::{LlmProvider, MarketData, NewsSearch, ProviderError, SearchHit};
    use async_trait::async_trait;
    use chrono::{DateTime, NaiveDate, Utc};
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::collections::HashMap;

    const ARTICLE: &str = "Nvidia shares rose 3% on Thursday after the chipmaker unveiled a new data center partnership.";
    const QUOTE: &str = "Nvidia shares rose 3% on Thursday";
    const URL: &str = "https://news.test/nvda";
    const NVDA_Q: &str = "Nvidia (NVDA) stock";
    const AH_Q: &str = "earnings after the bell October 29, 2026";

    fn q(sym: &str, last: f64, pct: f64, vol: Option<u64>) -> Quote {
        Quote {
            symbol: sym.into(),
            last,
            prev_close: last / (1.0 + pct / 100.0),
            change_pct: pct,
            volume: vol,
        }
    }

    /// Close quotes by symbol; an index absent from `indexes` answers 429.
    struct FakeMd {
        indexes: HashMap<String, Quote>,
        stocks: HashMap<String, Quote>,
        sessions: RefCell<Vec<NaiveDate>>,
        /// Answer `close_quotes` in reverse order, as a provider may.
        reverse: bool,
        /// The symbol list of each `close_quotes` call.
        batches: RefCell<Vec<Vec<String>>>,
    }
    #[async_trait(?Send)]
    impl MarketData for FakeMd {
        async fn quote(&self, _symbol: &str) -> Result<Quote, ProviderError> {
            panic!("recap must not use premarket quotes")
        }
        async fn premarket_quotes(
            &self,
            _symbols: &[String],
            _now: DateTime<Utc>,
        ) -> Vec<(String, Result<Quote, ProviderError>)> {
            panic!("recap must not use premarket quotes")
        }
        async fn close_quotes(
            &self,
            symbols: &[String],
            session: NaiveDate,
        ) -> Vec<(String, Result<Quote, ProviderError>)> {
            self.batches.borrow_mut().push(symbols.to_vec());
            let mut out = Vec::new();
            for s in symbols {
                out.push((s.clone(), self.close_quote(s, session).await));
            }
            if self.reverse {
                out.reverse();
            }
            out
        }
        fn diagnostics(&self) -> Vec<String> {
            vec!["closes ok=1 other_day=519".into()]
        }
        async fn close_quote(
            &self,
            symbol: &str,
            session: NaiveDate,
        ) -> Result<Quote, ProviderError> {
            self.sessions.borrow_mut().push(session);
            if symbol.starts_with('^') {
                self.indexes
                    .get(symbol)
                    .cloned()
                    .ok_or(ProviderError::Http(429))
            } else {
                self.stocks
                    .get(symbol)
                    .cloned()
                    .ok_or(ProviderError::Http(404))
            }
        }
    }
    fn md(stocks: &[Quote]) -> FakeMd {
        FakeMd {
            indexes: ["^GSPC", "^NDX", "^DJI", "^RUT"]
                .iter()
                .map(|s| (s.to_string(), q(s, 5850.12, 0.51, Some(1))))
                .collect(),
            stocks: stocks
                .iter()
                .map(|s| (s.symbol.clone(), s.clone()))
                .collect(),
            sessions: RefCell::new(vec![]),
            reverse: false,
            batches: RefCell::new(vec![]),
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
            published: None,
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
        "2026-10-29T20:45:00Z".parse().unwrap()
    }
    fn syms(movers: &[crate::facts::Mover]) -> Vec<&str> {
        movers.iter().map(|m| m.quote.symbol.as_str()).collect()
    }

    #[tokio::test]
    async fn all_indexes_present_is_publishable() {
        let m = md(&[]);
        let f = build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert_eq!(f.trading_date, date());
        assert_eq!(f.as_of_utc, now());
        assert!(!f.early_close);
        let labels: Vec<&str> = f.indexes.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            labels,
            ["S&P 500", "Nasdaq 100", "Dow Jones", "Russell 2000"]
        );
        let symbols: Vec<String> = f
            .indexes
            .iter()
            .map(|(_, fq)| match fq {
                Field::Value(q) => q.symbol.clone(),
                Field::Missing { reason } => panic!("{reason}"),
            })
            .collect();
        assert_eq!(symbols, ["^GSPC", "^NDX", "^DJI", "^RUT"]);
        assert!(missing_required(&f).is_empty());
        // Every close quote was asked for this session.
        assert!(m.sessions.borrow().iter().all(|d| *d == date()));
    }

    #[tokio::test]
    async fn indexes_by_close_quote_universe_in_one_batch() {
        let m = md(&[q("NVDA", 182.35, 3.08, Some(210_000_000))]);
        build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        // The four index closes are single requests (chart); only the
        // universe goes through the batched `close_quotes`.
        let batches = m.batches.borrow();
        assert_eq!(batches.len(), 1, "{batches:?}");
        assert_eq!(batches[0].len(), universe().len());
        assert!(!batches[0].iter().any(|s| s.starts_with('^')));
        assert_eq!(m.sessions.borrow().len(), 4 + universe().len());
    }

    #[tokio::test]
    async fn diagnostics_say_why_movers_collapsed() {
        let m = md(&[q("NVDA", 182.35, 3.08, Some(210_000_000))]);
        let f = build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        let n = universe().len();
        assert_eq!(
            f.diagnostics,
            vec![
                format!(
                    "quotes ok=1 http429=0 http_other={} empty=0 stale=0 parse=0 net=0 skipped=0",
                    n - 1
                ),
                "closes ok=1 other_day=519".to_string(),
                "movers liquid=1 gainers=1 losers=0".to_string(),
                "reasons proposed=0 kept=0 hedged=0".to_string(),
                "search_err=2 llm_err=0 news_stale=0 news_noise=0".to_string(),
                "indexes ok=4 err=0".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn reordered_provider_answers_map_by_symbol() {
        let mut m = md(&[
            q("BRK-B", 480.0, 2.0, Some(3_000_000)),
            q("NVDA", 180.0, 1.0, Some(3_000_000)),
        ]);
        m.reverse = true;
        m.indexes
            .insert("^RUT".into(), q("^RUT", 2230.75, 1.05, Some(1)));
        let f = build_facts(&m, &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert!(
            matches!(&f.indexes[3].1, Field::Value(q) if q.symbol == "^RUT" && q.last == 2230.75)
        );
        assert!(matches!(&f.indexes[0].1, Field::Value(q) if q.symbol == "^GSPC"));
        assert_eq!(syms(&f.gainers), ["BRK.B", "NVDA"]);
        assert_eq!(f.gainers[0].quote.last, 480.0);
    }

    #[tokio::test]
    async fn one_index_missing_gates_indexes() {
        let mut m = md(&[q("NVDA", 182.35, 3.08, Some(210_000_000))]);
        m.indexes.remove("^RUT");
        let l = llm(vec![]);
        let f = build_facts(&m, &ns(vec![]), &l, date(), now()).await;
        // No movers or news work while the required section is missing.
        assert!(f.gainers.is_empty() && l.prompts.borrow().is_empty());
        assert_eq!(m.sessions.borrow().len(), 4);
        assert_eq!(
            f.indexes[3].1,
            Field::Missing {
                reason: ProviderError::Http(429).to_string()
            }
        );
        assert_eq!(missing_required(&f), vec!["indexes"]);
    }

    #[tokio::test]
    async fn movers_filtered_sorted_and_capped() {
        let mut stocks: Vec<Quote> = ["AAPL", "MSFT", "NVDA", "AMZN", "META", "TSLA"]
            .iter()
            .enumerate()
            .map(|(i, s)| q(s, 100.0, i as f64 + 1.0, Some(2_000_000)))
            .collect();
        stocks.extend([
            q("GOOGL", 100.0, -2.0, Some(2_000_000)),
            q("NFLX", 100.0, -9.0, Some(999_999)), // thin volume
            q("F", 4.99, -12.0, Some(90_000_000)), // under $5
            q("AMD", 100.0, -1.0, None),           // unknown volume
            q("BRK-B", 480.0, -0.5, Some(3_000_000)),
        ]);
        let f = build_facts(&md(&stocks), &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert_eq!(syms(&f.gainers), ["TSLA", "META", "AMZN", "NVDA", "MSFT"]);
        assert_eq!(syms(&f.losers), ["GOOGL", "BRK.B"]);
        assert_eq!(f.losers[1].name, "Berkshire Hathaway");
        // Movers are optional.
        assert!(missing_required(&f).is_empty());
    }

    #[tokio::test]
    async fn anchored_reason_kept_unanchored_dropped() {
        let stocks = [
            q("NVDA", 182.35, 3.08, Some(210_000_000)),
            q("AAPL", 230.0, -2.0, Some(50_000_000)),
        ];
        let n = ns(vec![
            (NVDA_Q, Ok(vec![hit(URL, ARTICLE)])),
            (
                "Apple (AAPL) stock",
                Ok(vec![hit("https://news.test/a", ARTICLE)]),
            ),
        ]);
        let l = llm(vec![
            (
                "NVDA",
                reasons(URL, QUOTE, "Nvidia rose 3% on a partnership."),
            ),
            (
                "AAPL",
                reasons(
                    "https://news.test/a",
                    "Apple fell on weak guidance today",
                    "x",
                ),
            ),
        ]);
        let f = build_facts(&md(&stocks), &n, &l, date(), now()).await;
        assert_eq!(f.gainers[0].reasons.len(), 1);
        assert_eq!(f.gainers[0].reasons[0].quote, QUOTE);
        assert_eq!(f.losers[0].reasons, vec![]);
        let (sys, user) = l
            .prompts
            .borrow()
            .iter()
            .find(|(_, u)| u.contains("NVDA"))
            .cloned()
            .unwrap();
        assert!(sys.contains("regular trading session") && sys.contains("verbatim"));
        assert!(user.contains("Change today: +3.08%"), "{user}");
        assert!(user.contains("As of: 2026-10-29 16:45 ET"), "{user}");
    }

    #[tokio::test]
    async fn stale_article_is_never_a_reason_and_movers_need_an_event() {
        let stocks = [q("NVDA", 182.35, 3.08, Some(210_000_000))];
        let stale = SearchHit {
            published: Some(now() - chrono::Duration::days(6)),
            ..hit(URL, ARTICLE)
        };
        let n = ns(vec![(NVDA_Q, Ok(vec![stale]))]);
        let l = llm(vec![("NVDA", reasons(URL, QUOTE, "Nvidia rose."))]);
        let f = build_facts(&md(&stocks), &n, &l, date(), now()).await;
        assert_eq!(f.gainers[0].reasons, vec![]);
        assert!(l.prompts.borrow().iter().all(|(_, u)| !u.contains("NVDA")));
        assert!(
            f.diagnostics
                .iter()
                .any(|l| l.ends_with("news_stale=1 news_noise=0")),
            "{:?}",
            f.diagnostics
        );

        let n = ns(vec![(NVDA_Q, Ok(vec![hit(URL, ARTICLE)]))]);
        let f = build_facts(&md(&stocks), &n, &l, date(), now()).await;
        assert_eq!(f.gainers[0].reasons.len(), 1);
        let p = l.prompts.borrow();
        let (sys, _) = p.iter().find(|(_, u)| u.contains("NVDA")).unwrap();
        assert!(sys.contains("Trailing performance"), "{sys}");
    }

    #[tokio::test]
    async fn after_hours_items_are_anchored() {
        let src = "Intel and Amazon are scheduled to report quarterly results after the bell on Thursday, October 29.";
        let n = ns(vec![(AH_Q, Ok(vec![hit("https://news.test/ah", src)]))]);
        let l = llm(vec![(
            "after the US market close",
            Ok(json!({"reasons": [
                {"url": "https://news.test/ah", "title": "T", "quote": "Intel and Amazon are scheduled to report quarterly results after the bell", "summary_en": "Intel and Amazon report after the close."},
                {"url": "https://news.test/ah", "title": "T", "quote": "Apple will report results after the bell tonight", "summary_en": "Apple reports."}
            ]})),
        )]);
        let f = build_facts(&md(&[]), &n, &l, date(), now()).await;
        match &f.after_hours {
            Field::Value(v) => {
                assert_eq!(v.len(), 1);
                assert!(v[0].quote.starts_with("Intel and Amazon"));
            }
            other => panic!("{other:?}"),
        }
        let (sys, user) = l.prompts.borrow()[0].clone();
        assert!(
            sys.contains("after the close") && sys.contains("October 29, 2026"),
            "{sys}"
        );
        assert!(user.contains("October 29, 2026"), "{user}");
    }

    /// Records each `search_since` call; every search finds nothing.
    struct SinceNs(RefCell<Vec<(String, DateTime<Utc>)>>);
    #[async_trait(?Send)]
    impl NewsSearch for SinceNs {
        async fn search(&self, q: &str, _: usize) -> Result<Vec<SearchHit>, ProviderError> {
            unreachable!("{q}: recap searches are date-bounded")
        }
        async fn search_since(
            &self,
            q: &str,
            _: usize,
            since: DateTime<Utc>,
        ) -> Result<Vec<SearchHit>, ProviderError> {
            self.0.borrow_mut().push((q.into(), since));
            Err(ProviderError::Empty)
        }
    }

    #[tokio::test]
    async fn after_hours_previews_reach_back_a_week() {
        let n = SinceNs(RefCell::new(vec![]));
        build_facts(&md(&[]), &n, &llm(vec![]), date(), now()).await;
        assert_eq!(
            *n.0.borrow(),
            vec![(AH_Q.to_string(), now() - chrono::Duration::days(7))]
        );
    }

    #[tokio::test]
    async fn after_hours_errors_are_missing() {
        let f = build_facts(&md(&[]), &ns(vec![]), &llm(vec![]), date(), now()).await;
        assert_eq!(
            f.after_hours,
            Field::Missing {
                reason: ProviderError::Empty.to_string()
            }
        );
        let n = ns(vec![(AH_Q, Ok(vec![hit("https://news.test/ah", ARTICLE)]))]);
        let l = llm(vec![(
            "after the US market close",
            Err(ProviderError::Http(503)),
        )]);
        let f = build_facts(&md(&[]), &n, &l, date(), now()).await;
        assert_eq!(
            f.after_hours,
            Field::Missing {
                reason: ProviderError::Http(503).to_string()
            }
        );
        // Still publishable: after-hours is optional.
        assert!(missing_required(&f).is_empty());
    }

    #[tokio::test]
    async fn early_close_day_is_flagged() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
        let f = build_facts(&md(&[]), &ns(vec![]), &llm(vec![]), d, now()).await;
        assert!(f.early_close);
    }

    fn reason(quote: &str) -> NewsReason {
        NewsReason {
            url: URL.into(),
            title: "T".into(),
            quote: quote.into(),
            summary_en: "s".into(),
        }
    }

    #[tokio::test]
    async fn allowed_numbers_and_round_trip() {
        let mut f = build_facts(
            &md(&[q("NVDA", 182.35, 3.08, Some(210_000_000))]),
            &ns(vec![]),
            &llm(vec![]),
            date(),
            now(),
        )
        .await;
        f.gainers[0].reasons = vec![reason("Nvidia shares rose 3% on a $5 billion deal")];
        f.after_hours = Field::Value(vec![reason("Intel expects revenue of $13.4 billion")]);
        let n = numbers_in_facts(&f);
        for v in [
            2025.0,
            2026.0,
            2027.0,
            5850.12,
            0.51,
            182.35,
            3.08,
            210_000_000.0,
            3.0,
            -3.0,
            5e9,
            13.4e9,
        ] {
            assert!(
                n.iter().any(|x| (x - v).abs() < 1e-6),
                "{v} missing from {n:?}"
            );
        }
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<RecapFacts>(&s).unwrap(), f);
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;
    use crate::providers::{ProviderError, SearchHit};
    use crate::store::{MemStore, PostRow, RunRow};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::cell::Cell;

    const ARTICLE: &str = "Nvidia shares rose 3% on Thursday after the chipmaker unveiled a new data center partnership.";
    const URL: &str = "https://news.test/nvda";

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn q(sym: &str, last: f64, pct: f64) -> Quote {
        Quote {
            symbol: sym.into(),
            last,
            prev_close: last / (1.0 + pct / 100.0),
            change_pct: pct,
            volume: Some(5_000_000),
        }
    }

    /// `rut_ok = false` makes the Russell 2000 close fail with HTTP 429.
    struct FakeMd {
        rut_ok: Cell<bool>,
    }
    #[async_trait(?Send)]
    impl MarketData for FakeMd {
        async fn quote(&self, _symbol: &str) -> Result<Quote, ProviderError> {
            Err(ProviderError::Http(500))
        }
        async fn premarket_quotes(
            &self,
            _symbols: &[String],
            _now: DateTime<Utc>,
        ) -> Vec<(String, Result<Quote, ProviderError>)> {
            vec![]
        }
        async fn close_quote(
            &self,
            symbol: &str,
            _session: NaiveDate,
        ) -> Result<Quote, ProviderError> {
            match symbol {
                "^RUT" if !self.rut_ok.get() => Err(ProviderError::Http(429)),
                "^GSPC" | "^NDX" | "^DJI" | "^RUT" => Ok(q(symbol, 5850.12, 0.51)),
                "NVDA" => Ok(q("NVDA", 182.35, 3.08)),
                _ => Err(ProviderError::Http(404)),
            }
        }
    }
    fn md(rut_ok: bool) -> FakeMd {
        FakeMd {
            rut_ok: Cell::new(rut_ok),
        }
    }

    struct FakeNs;
    #[async_trait(?Send)]
    impl NewsSearch for FakeNs {
        async fn search(&self, query: &str, _max: usize) -> Result<Vec<SearchHit>, ProviderError> {
            if query.contains("NVDA") {
                Ok(vec![SearchHit {
                    url: URL.into(),
                    title: "Nvidia rises".into(),
                    content: ARTICLE.into(),
                    published: None,
                }])
            } else {
                Err(ProviderError::Empty)
            }
        }
    }

    struct FakeLlm {
        en_ok: Cell<bool>,
        zh_ok: Cell<bool>,
        zh_calls: Cell<usize>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, system: &str, _user: &str) -> Result<Value, ProviderError> {
            if system.starts_with("You write the US after-close") {
                Ok(draft_reply(Lang::En, self.en_ok.get()))
            } else if system.starts_with("你负责撰写美股收盘回顾") {
                self.zh_calls.set(self.zh_calls.get() + 1);
                Ok(draft_reply(Lang::Zh, self.zh_ok.get()))
            } else {
                Ok(
                    json!({"reasons": [{"url": URL, "title": "x", "quote": "Nvidia shares rose 3% on Thursday", "summary_en": "Nvidia rose on a partnership."}]}),
                )
            }
        }
    }
    fn llm(en_ok: bool, zh_ok: bool) -> FakeLlm {
        FakeLlm {
            en_ok: Cell::new(en_ok),
            zh_ok: Cell::new(zh_ok),
            zh_calls: Cell::new(0),
        }
    }
    fn draft_reply(lang: Lang, ok: bool) -> Value {
        let body = if ok {
            "Stocks closed higher."
        } else {
            "The Dow rose 7.5%."
        };
        json!({
            "title": "Stocks close higher",
            "summary": "Indexes rose.",
            "sections": draft::headings(lang).iter().map(|h| json!({"heading": h, "markdown": body})).collect::<Vec<_>>(),
        })
    }

    fn status(s: &MemStore, key: &str) -> String {
        s.runs.borrow()[key].status.clone()
    }
    fn tr_status(s: &MemStore, post: i64, lang: &str) -> String {
        s.translations.borrow()[&(post, lang.to_string())]
            .status
            .clone()
    }
    const KEY: &str = "recap:2026-10-29";

    #[tokio::test]
    async fn publishes_once_in_window() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        assert_eq!(
            tick(utc("2026-10-29T20:44:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert!(s.runs.borrow().is_empty());
        let now = utc("2026-10-29T20:45:00Z");
        assert_eq!(tick(now, &s, &m, &FakeNs, &l).await, TickOutcome::Published);
        assert_eq!(tick(now, &s, &m, &FakeNs, &l).await, TickOutcome::Nothing);
        let posts = s.posts.borrow();
        assert_eq!(posts.len(), 1);
        let p = &posts[0];
        assert_eq!(
            (
                p.run_key.as_str(),
                p.content_type.as_str(),
                p.template_id.as_str()
            ),
            (KEY, "recap", "recap:v1")
        );
        assert_eq!(p.slug, "recap-2026-10-29");
        assert_eq!(p.facts_r2_key, "recap/2026-10-29.json");
        drop(posts);
        assert_eq!(status(&s, KEY), "published");
        assert_eq!(s.runs.borrow()[KEY].content_type, "recap");
        assert_eq!(tr_status(&s, 1, "en"), "published");
        assert_eq!(tr_status(&s, 1, "zh"), "published");
        let facts: RecapFacts =
            serde_json::from_str(&s.facts.borrow()["recap/2026-10-29.json"]).unwrap();
        assert_eq!(facts.gainers[0].quote.symbol, "NVDA");
        assert_eq!(s.sources.borrow()[&1][0].url, URL);
        assert!(s.translations.borrow()[&(1, "en".to_string())]
            .body_md
            .contains("| S&P 500 | 5,850.12 | +0.51% |"));
    }

    #[tokio::test]
    async fn extra_diagnostics_reach_last_error() {
        let s = MemStore::default();
        let (m, l) = (md(false), llm(true, true));
        let extra = || vec!["subrequests=4".to_string()];
        let hooks = crate::run::Hooks {
            log: &|_| {},
            extra: &extra,
        };
        let out = tick_with_hooks(utc("2026-10-29T20:45:00Z"), &s, &m, &FakeNs, &l, &hooks).await;
        assert!(
            matches!(&out, TickOutcome::Retrying(e) if e.starts_with("indexes | ") && e.ends_with("; indexes ok=3 err=1; subrequests=4")),
            "{out:?}"
        );
    }

    #[tokio::test]
    async fn build_logs_diagnostics_once() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let now = utc("2026-10-29T20:45:00Z");
        assert_eq!(
            tick_with_log(now, &s, &m, &FakeNs, &l, &log).await,
            TickOutcome::Published
        );
        {
            let logs = logs.borrow();
            assert_eq!(logs.len(), 1, "{logs:?}");
            let n = crate::premarket::universe().len();
            assert!(
                logs[0].starts_with(&format!(
                    "diagnostics: quotes ok=1 http429=0 http_other={} ",
                    n - 1
                )),
                "{logs:?}"
            );
            assert!(
                logs[0].contains(
                    "; movers liquid=1 gainers=1 losers=0; reasons proposed=1 kept=1 hedged=0;"
                ),
                "{logs:?}"
            );
            assert!(logs[0].ends_with("; indexes ok=4 err=0"), "{logs:?}");
        }
        // No build, no diagnostics line.
        tick_with_log(now, &s, &m, &FakeNs, &l, &log).await;
        assert_eq!(logs.borrow().len(), 1);
        // Diagnostics are stored with the facts.
        let facts: RecapFacts =
            serde_json::from_str(&s.facts.borrow()["recap/2026-10-29.json"]).unwrap();
        assert!(facts
            .diagnostics
            .last()
            .unwrap()
            .starts_with("indexes ok=4"));
    }

    #[tokio::test]
    async fn missing_index_retries_then_skips_at_7pm_est() {
        // Fri 2026-11-06 (EST): 16:45 ET = 21:45Z, 19:00 ET = Sat 00:00Z.
        let key = "recap:2026-11-06";
        let s = MemStore::default();
        let (m, l) = (md(false), llm(true, true));
        let want = "indexes | quotes ok=0 http429=0 http_other=0 empty=0 stale=0 parse=0 net=0 \
                    skipped=0; movers liquid=0 gainers=0 losers=0; reasons proposed=0 kept=0 hedged=0; \
                    search_err=0 llm_err=0 news_stale=0 news_noise=0; indexes ok=3 err=1";
        for t in ["2026-11-06T21:45:00Z", "2026-11-06T23:55:00Z"] {
            assert_eq!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Retrying(want.into())
            );
            assert_eq!(status(&s, key), "pending");
            assert_eq!(s.runs.borrow()[key].last_error.as_deref(), Some(want));
        }
        assert_eq!(
            tick(utc("2026-11-07T00:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Skipped
        );
        assert_eq!(status(&s, key), "skipped");
        assert!(s.posts.borrow().is_empty());
        {
            let alerts = s.alerts.borrow();
            assert_eq!(alerts.len(), 1);
            assert_eq!((alerts[0].0.as_str(), alerts[0].1.as_str()), (key, "error"));
            assert_eq!(
                alerts[0].2,
                format!("after-close recap skipped for 2026-11-06: not ready by 19:00 ET ({want})")
            );
        }
        // Data arriving after the deadline changes nothing.
        m.rut_ok.set(true);
        assert_eq!(
            tick(utc("2026-11-07T00:05:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert!(s.posts.borrow().is_empty());
    }

    #[tokio::test]
    async fn early_close_day_publishes_at_1345() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        assert_eq!(
            tick(utc("2026-11-27T18:40:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert_eq!(
            tick(utc("2026-11-27T18:45:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        let en = s.translations.borrow()[&(1, "en".to_string())]
            .body_md
            .clone();
        assert!(en.contains("closed early at 1:00 PM ET"), "{en}");
    }

    #[tokio::test]
    async fn closed_day_is_nothing() {
        let s = MemStore::default();
        assert_eq!(
            tick(
                utc("2026-11-26T21:45:00Z"),
                &s,
                &md(true),
                &FakeNs,
                &llm(true, true)
            )
            .await,
            TickOutcome::Nothing
        );
        assert!(s.runs.borrow().is_empty());
    }

    #[tokio::test]
    async fn zh_pending_until_22_et_then_failed() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        assert_eq!(
            tick(utc("2026-10-29T20:45:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::PublishedEnOnly
        );
        assert_eq!(tr_status(&s, 1, "zh"), "pending");
        // 21:00 ET (EDT) = 01:00Z the next UTC day: still pending, no alert.
        assert!(matches!(
            tick(utc("2026-10-30T01:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert_eq!(tr_status(&s, 1, "zh"), "pending");
        assert!(s.alerts.borrow().is_empty());
        // 22:00 ET: give up once.
        assert!(matches!(
            tick(utc("2026-10-30T02:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert_eq!(tr_status(&s, 1, "zh"), "failed");
        assert_eq!(status(&s, KEY), "published");
        let alerts = s.alerts.borrow();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].1, "warn");
        assert!(alerts[0]
            .2
            .starts_with("Chinese after-close recap failed for 2026-10-29"));
    }

    #[tokio::test]
    async fn zh_retry_publishes_later() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        tick(utc("2026-10-29T20:45:00Z"), &s, &m, &FakeNs, &l).await;
        l.zh_ok.set(true);
        assert_eq!(
            tick(utc("2026-10-29T23:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        assert!(s.translations.borrow()[&(1, "zh".to_string())]
            .body_md
            .contains("## 指数收盘"));
    }

    #[tokio::test]
    async fn zh_retries_only_on_the_hour_after_en_publishes() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        assert_eq!(
            tick(utc("2026-10-29T20:45:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::PublishedEnOnly
        );
        let zh_calls = || l.zh_calls.get();
        let after_publish = zh_calls();
        // Every 5-minute recap firing between the hours does nothing: no LLM call.
        for t in [
            "2026-10-29T20:50:00Z",
            "2026-10-29T21:05:00Z",
            "2026-10-29T21:55:00Z",
        ] {
            assert_eq!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing,
                "{t}"
            );
        }
        assert_eq!(zh_calls(), after_publish);
        // 18:00 ET retries once.
        assert!(matches!(
            tick(utc("2026-10-29T22:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert!(zh_calls() > after_publish);
        let at_six = zh_calls();
        l.zh_ok.set(true);
        assert_eq!(
            tick(utc("2026-10-29T22:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert_eq!(zh_calls(), at_six);
        assert_eq!(
            tick(utc("2026-10-29T23:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        assert_eq!(tr_status(&s, 1, "zh"), "published");
    }

    #[tokio::test]
    async fn en_failure_publishes_nothing() {
        let s = MemStore::default();
        let out = tick(
            utc("2026-10-29T20:45:00Z"),
            &s,
            &md(true),
            &FakeNs,
            &llm(false, true),
        )
        .await;
        assert!(
            matches!(out, TickOutcome::Retrying(ref e) if e.starts_with("en: ")),
            "{out:?}"
        );
        assert!(s.posts.borrow().is_empty());
    }

    #[tokio::test]
    async fn premarket_and_recap_keep_separate_runs() {
        let s = MemStore::default();
        let pre = RunRow {
            run_key: "premarket:2026-10-29".into(),
            content_type: "premarket".into(),
            trading_date: "2026-10-29".into(),
            status: "published".into(),
            attempts: 1,
            last_error: None,
            created_at: "2026-10-29T12:30:00Z".into(),
            updated_at: "2026-10-29T12:30:00Z".into(),
        };
        let pre_post = PostRow {
            id: None,
            run_key: pre.run_key.clone(),
            content_type: "premarket".into(),
            template_id: "premarket:v1".into(),
            trading_date: "2026-10-29".into(),
            slug: "premarket-2026-10-29".into(),
            facts_r2_key: "premarket/2026-10-29.json".into(),
            published_at: Some("2026-10-29T12:30:00Z".into()),
        };
        s.runs.borrow_mut().insert(pre.run_key.clone(), pre.clone());
        crate::store::Store::upsert_post(&s, &pre_post)
            .await
            .unwrap();
        assert_eq!(
            tick(
                utc("2026-10-29T20:45:00Z"),
                &s,
                &md(true),
                &FakeNs,
                &llm(true, true)
            )
            .await,
            TickOutcome::Published
        );
        assert_eq!(s.runs.borrow()["premarket:2026-10-29"], pre);
        let posts = s.posts.borrow();
        assert_eq!(posts.len(), 2);
        assert_eq!(
            posts[0],
            PostRow {
                id: Some(1),
                ..pre_post
            }
        );
        assert_eq!(
            (posts[1].id, posts[1].slug.as_str()),
            (Some(2), "recap-2026-10-29")
        );
    }
}
