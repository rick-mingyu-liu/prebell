//! The run state machine shared by every scheduled content type. Each cron
//! firing calls `tick_content`, which decides from the ET clock and the stored
//! run what to do. Re-running a tick never duplicates a post: the run key is
//! the idempotency key. `Content` supplies what differs per type.

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, SecondsFormat, Timelike, Utc};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::clock::{decide, et, premarket_window, trading_day, DayKind, SlotDecision, Window};
use crate::facts::{missing_required, Field, NewsReason, PremarketFacts};
use crate::numbers::Lang;
use crate::premarket::build_facts;
use crate::premarket::draft::{write, Draft, TEMPLATE_ID};
use crate::providers::{LlmProvider, MarketData, NewsSearch};
use crate::store::{PostRow, RunRow, Store, TranslationRow};

/// One scheduled content type (premarket brief, recap): its window, facts,
/// gate, drafting and sources. Everything else is the shared state machine.
#[async_trait(?Send)]
pub trait Content {
    type Facts: Serialize + DeserializeOwned;
    /// `posts.content_type`; also prefixes the run key, slug and facts key.
    fn content_type(&self) -> &'static str;
    /// `posts.template_id`, e.g. `premarket:v1`.
    fn template_id(&self) -> &'static str;
    /// Human name for alerts, e.g. "premarket brief".
    fn label(&self) -> &'static str;
    /// The ET publishing window on a trading day of this kind.
    fn window(&self, day: DayKind) -> Window;
    /// ET time from which a still-failing ZH draft is marked `failed`.
    fn zh_give_up(&self) -> NaiveTime;
    /// Whether a firing at ET time `t` retries a pending ZH draft. The
    /// default is every firing; the give-up time always retries.
    fn zh_retry_due(&self, _t: NaiveTime) -> bool {
        true
    }
    async fn build_facts(&self, date: NaiveDate, now: DateTime<Utc>) -> Self::Facts;
    /// Names of required sections that are unavailable; empty = publishable.
    fn missing_required(&self, f: &Self::Facts) -> Vec<&'static str>;
    async fn write(&self, f: &Self::Facts, lang: Lang) -> Result<(Draft, String), Vec<String>>;
    /// The post's sources, one per url.
    fn sources(&self, f: &Self::Facts) -> Vec<NewsReason>;
    /// Build counters carried in the facts, for the log and `last_error`.
    fn diagnostics(&self, f: &Self::Facts) -> Vec<String>;

    fn run_key(&self, date: NaiveDate) -> String {
        format!("{}:{}", self.content_type(), date.format("%Y-%m-%d"))
    }
    fn slug(&self, date: NaiveDate) -> String {
        format!("{}-{}", self.content_type(), date.format("%Y-%m-%d"))
    }
    fn facts_key(&self, date: NaiveDate) -> String {
        format!("{}/{}.json", self.content_type(), date.format("%Y-%m-%d"))
    }
}

/// The premarket brief (Plan 1).
pub struct Premarket<'a> {
    pub md: &'a dyn MarketData,
    pub ns: &'a dyn NewsSearch,
    pub llm: &'a dyn LlmProvider,
}

#[async_trait(?Send)]
impl Content for Premarket<'_> {
    type Facts = PremarketFacts;
    fn content_type(&self) -> &'static str {
        "premarket"
    }
    fn template_id(&self) -> &'static str {
        TEMPLATE_ID
    }
    fn label(&self) -> &'static str {
        "premarket brief"
    }
    fn window(&self, day: DayKind) -> Window {
        premarket_window(day)
    }
    fn zh_give_up(&self) -> NaiveTime {
        NaiveTime::from_hms_opt(18, 0, 0).expect("valid built-in time")
    }
    /// Every firing inside the 08:30-09:25 ET window, then hourly on the
    /// hour: the cron fires every 5 minutes all day.
    fn zh_retry_due(&self, t: NaiveTime) -> bool {
        let w = premarket_window(DayKind::Full);
        (w.start..=w.end).contains(&t) || t.minute() == 0
    }
    async fn build_facts(&self, date: NaiveDate, now: DateTime<Utc>) -> PremarketFacts {
        build_facts(self.md, self.ns, self.llm, date, now).await
    }
    fn missing_required(&self, f: &PremarketFacts) -> Vec<&'static str> {
        missing_required(f)
    }
    async fn write(&self, f: &PremarketFacts, lang: Lang) -> Result<(Draft, String), Vec<String>> {
        write(self.llm, f, lang).await
    }
    fn sources(&self, f: &PremarketFacts) -> Vec<NewsReason> {
        sources(f)
    }
    fn diagnostics(&self, f: &PremarketFacts) -> Vec<String> {
        f.diagnostics.clone()
    }
}
/// A `building` run younger than this is assumed to be in flight. A build
/// takes 3-8 minutes and the cron fires every 5, so this must cover several
/// firings or a running build is treated as crashed and rebuilt.
const BUILD_LEASE_MINUTES: i64 = 15;

#[derive(Debug, Clone, PartialEq)]
pub enum TickOutcome {
    Nothing,
    Published,
    PublishedEnOnly,
    Retrying(String),
    Skipped,
}

/// The premarket run key, `premarket:YYYY-MM-DD`.
pub fn run_key(trading_date: NaiveDate) -> String {
    format!("premarket:{}", trading_date.format("%Y-%m-%d"))
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// One premarket cron firing. A store error is reported as `Retrying`.
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
    tick_content(now, store, &Premarket { md, ns, llm }, hooks).await
}

/// What the caller adds to a tick.
pub struct Hooks<'a> {
    /// Receives the build's diagnostics line, once when it builds.
    pub log: &'a dyn Fn(&str),
    /// Lines appended to the build's diagnostics (log and `last_error`, not
    /// the stored facts), read when the facts are built, e.g. the Worker's
    /// subrequest count.
    pub extra: &'a dyn Fn() -> Vec<String>,
}

/// One cron firing for content type `c`. A store error is reported as `Retrying`.
pub async fn tick_content<C: Content>(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &C,
    hooks: &Hooks<'_>,
) -> TickOutcome {
    step(now, store, c, hooks)
        .await
        .unwrap_or_else(TickOutcome::Retrying)
}

async fn step<C: Content>(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &C,
    hooks: &Hooks<'_>,
) -> Result<TickOutcome, String> {
    let decision = decide(now, |d| c.window(d));
    let date = match decision {
        SlotDecision::NotToday => return Ok(TickOutcome::Nothing),
        SlotDecision::TooEarly => et(now).date_naive(),
        SlotDecision::InWindow { trading_date } | SlotDecision::PastDeadline { trading_date } => {
            trading_date
        }
    };
    let key = c.run_key(date);
    // Any read error aborts the tick before a write: a failed read is never "no row".
    let run = store.get_run(&key).await?;

    if let Some(r) = run.as_ref().filter(|r| r.status == "published") {
        return retry_zh(now, store, c, r).await;
    }
    let in_flight = run
        .as_ref()
        .is_some_and(|r| r.status == "building" && !is_stale(r, now));
    let may_act = matches!(
        decision,
        SlotDecision::InWindow { .. } | SlotDecision::PastDeadline { .. }
    );
    if may_act && !in_flight {
        if let Some(r) = reconcile(now, store, c, date, run.clone()).await? {
            return retry_zh(now, store, c, &r).await;
        }
    }

    match decision {
        SlotDecision::InWindow { .. } if in_flight => Ok(TickOutcome::Nothing),
        SlotDecision::InWindow { .. } => build(now, store, c, hooks, date, run).await,
        // Let an in-flight build finish; once its lease expires it is skipped.
        SlotDecision::PastDeadline { .. } if in_flight => Ok(TickOutcome::Nothing),
        SlotDecision::PastDeadline { .. } => skip(now, store, c, date, run).await,
        _ => Ok(TickOutcome::Nothing),
    }
}

/// A store failure part-way through publishing can leave EN live while the
/// run is not `published`. If so, mark the run published (adding a pending
/// ZH row when none exists) without touching EN, and return the run.
async fn reconcile<C: Content>(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &C,
    date: NaiveDate,
    run: Option<RunRow>,
) -> Result<Option<RunRow>, String> {
    let Some(post) = store.get_post_by_run(&c.run_key(date)).await? else {
        return Ok(None);
    };
    let post_id = post.id.ok_or("post has no id")?;
    if !store
        .get_translation(post_id, "en")
        .await?
        .is_some_and(|t| t.status == "published")
    {
        return Ok(None);
    }
    let ts = iso(now);
    if store.get_translation(post_id, "zh").await?.is_none() {
        let pending = translation_zh(
            post_id,
            "pending",
            String::new(),
            String::new(),
            String::new(),
            &ts,
        );
        store.upsert_translation(&pending).await?;
    }
    // The failed write may have been `add_sources`; it replaces, so redo it.
    if let Some(json) = store.get_facts(&post.facts_r2_key).await? {
        let facts: C::Facts =
            serde_json::from_str(&json).map_err(|e| format!("facts json: {e}"))?;
        store.add_sources(post_id, &c.sources(&facts)).await?;
    }
    let mut row = run.unwrap_or_else(|| new_run(now, c, date));
    row.status = "published".into();
    row.updated_at = ts;
    store.upsert_run(&row).await?;
    Ok(Some(row))
}

fn is_stale(r: &RunRow, now: DateTime<Utc>) -> bool {
    match DateTime::parse_from_rfc3339(&r.updated_at) {
        Ok(t) => now - t.with_timezone(&Utc) >= Duration::minutes(BUILD_LEASE_MINUTES),
        Err(_) => true,
    }
}

async fn skip<C: Content>(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &C,
    date: NaiveDate,
    run: Option<RunRow>,
) -> Result<TickOutcome, String> {
    if run.as_ref().is_some_and(|r| r.status == "skipped") {
        return Ok(TickOutcome::Nothing);
    }
    let mut row = run.unwrap_or_else(|| new_run(now, c, date));
    row.status = "skipped".into();
    row.updated_at = iso(now);
    let why = row
        .last_error
        .clone()
        .unwrap_or_else(|| "no attempt succeeded".into());
    store.upsert_run(&row).await?;
    store
        .add_alert(
            &row.run_key,
            "error",
            &format!(
                "{} skipped for {date}: not ready by {} ET ({why})",
                c.label(),
                c.window(trading_day(date)).end.format("%H:%M")
            ),
        )
        .await?;
    Ok(TickOutcome::Skipped)
}

fn new_run<C: Content>(now: DateTime<Utc>, c: &C, date: NaiveDate) -> RunRow {
    RunRow {
        run_key: c.run_key(date),
        content_type: c.content_type().into(),
        trading_date: date.format("%Y-%m-%d").to_string(),
        status: "pending".into(),
        attempts: 0,
        last_error: None,
        created_at: iso(now),
        updated_at: iso(now),
    }
}

async fn build<C: Content>(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &C,
    hooks: &Hooks<'_>,
    date: NaiveDate,
    run: Option<RunRow>,
) -> Result<TickOutcome, String> {
    let ts = iso(now);
    let mut row = run.unwrap_or_else(|| new_run(now, c, date));
    row.status = "building".into();
    row.attempts += 1;
    row.updated_at = ts.clone();
    store.upsert_run(&row).await?;

    let facts = c.build_facts(date, now).await;
    let mut lines = c.diagnostics(&facts);
    lines.extend((hooks.extra)());
    let diagnostics = lines.join("; ");
    (hooks.log)(&format!("diagnostics: {diagnostics}"));
    let missing = c.missing_required(&facts);
    if !missing.is_empty() {
        let mut err = missing.join(", ");
        if !diagnostics.is_empty() {
            err = format!("{err} | {diagnostics}");
        }
        return retry_later(store, row, err).await;
    }

    let (en, en_md) = match c.write(&facts, Lang::En).await {
        Ok(x) => x,
        Err(e) => return retry_later(store, row, format!("en: {}", e.join("; "))).await,
    };
    let zh = c.write(&facts, Lang::Zh).await;

    // The build took minutes; another tick may have finished this run since
    // it was read. If so, keep its post and facts untouched.
    if superseded(store, &row.run_key).await? {
        return Ok(TickOutcome::Nothing);
    }
    let facts_key = c.facts_key(date);
    let json = serde_json::to_string(&facts).map_err(|e| format!("facts json: {e}"))?;
    store.put_facts(&facts_key, &json).await?;

    let post_id = store
        .upsert_post(&PostRow {
            id: None,
            run_key: row.run_key.clone(),
            content_type: c.content_type().into(),
            template_id: c.template_id().into(),
            trading_date: row.trading_date.clone(),
            slug: c.slug(date),
            facts_r2_key: facts_key,
            published_at: Some(ts.clone()),
        })
        .await?;
    store
        .upsert_translation(&TranslationRow {
            post_id,
            lang: "en".into(),
            status: "published".into(),
            title: en.title,
            summary: en.summary,
            body_md: en_md,
            updated_at: ts.clone(),
        })
        .await?;
    let (zh_row, zh_err) = match zh {
        Ok((d, body)) => (
            translation_zh(post_id, "published", d.title, d.summary, body, &ts),
            None,
        ),
        Err(e) => (
            translation_zh(
                post_id,
                "pending",
                String::new(),
                String::new(),
                String::new(),
                &ts,
            ),
            Some(format!("zh: {}", e.join("; "))),
        ),
    };
    store.upsert_translation(&zh_row).await?;
    store.add_sources(post_id, &c.sources(&facts)).await?;

    row.status = "published".into();
    row.last_error = zh_err.clone();
    row.updated_at = ts;
    store.upsert_run(&row).await?;
    Ok(if zh_err.is_some() {
        TickOutcome::PublishedEnOnly
    } else {
        TickOutcome::Published
    })
}

/// True when another tick finished this run (published or skipped) while
/// this one was building; the caller must then write nothing.
async fn superseded(store: &dyn Store, run_key: &str) -> Result<bool, String> {
    Ok(store
        .get_run(run_key)
        .await?
        .is_some_and(|r| r.status == "published" || r.status == "skipped"))
}

async fn retry_later(
    store: &dyn Store,
    mut row: RunRow,
    err: String,
) -> Result<TickOutcome, String> {
    if superseded(store, &row.run_key).await? {
        return Ok(TickOutcome::Nothing);
    }
    row.status = "pending".into();
    row.last_error = Some(err.clone());
    store.upsert_run(&row).await?;
    Ok(TickOutcome::Retrying(err))
}

fn translation_zh(
    post_id: i64,
    status: &str,
    title: String,
    summary: String,
    body_md: String,
    ts: &str,
) -> TranslationRow {
    TranslationRow {
        post_id,
        lang: "zh".into(),
        status: status.into(),
        title,
        summary,
        body_md,
        updated_at: ts.into(),
    }
}

/// Every kept reason (gainers, losers, macro), first occurrence per url.
fn sources(f: &PremarketFacts) -> Vec<NewsReason> {
    let macro_items = match &f.macro_items {
        Field::Value(v) => v.as_slice(),
        Field::Missing { .. } => &[],
    };
    let mut out: Vec<NewsReason> = Vec::new();
    for r in f
        .gainers
        .iter()
        .chain(&f.losers)
        .flat_map(|m| &m.reasons)
        .chain(macro_items)
    {
        if !out.iter().any(|o| o.url == r.url) {
            out.push(r.clone());
        }
    }
    out
}

/// After EN published: retry a pending ZH draft for the rest of the ET day;
/// from `zh_give_up` (18:00 ET for premarket) a failure marks it `failed`
/// with one warn alert.
async fn retry_zh<C: Content>(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &C,
    run: &RunRow,
) -> Result<TickOutcome, String> {
    let post = store
        .get_post_by_run(&run.run_key)
        .await?
        .ok_or("published run has no post")?;
    let post_id = post.id.ok_or("post has no id")?;
    if let Some(t) = store.get_translation(post_id, "zh").await? {
        if t.status != "pending" {
            return Ok(TickOutcome::Nothing);
        }
    }
    let local = et(now).time();
    if !c.zh_retry_due(local) && local < c.zh_give_up() {
        return Ok(TickOutcome::Nothing);
    }
    let json = store
        .get_facts(&post.facts_r2_key)
        .await?
        .ok_or("facts not found")?;
    let facts: C::Facts = serde_json::from_str(&json).map_err(|e| format!("facts json: {e}"))?;
    let ts = iso(now);
    match c.write(&facts, Lang::Zh).await {
        Ok((d, body)) => {
            store
                .upsert_translation(&translation_zh(
                    post_id,
                    "published",
                    d.title,
                    d.summary,
                    body,
                    &ts,
                ))
                .await?;
            Ok(TickOutcome::Published)
        }
        Err(e) => {
            let err = format!("zh: {}", e.join("; "));
            if et(now).time() >= c.zh_give_up() {
                let failed = translation_zh(
                    post_id,
                    "failed",
                    String::new(),
                    String::new(),
                    String::new(),
                    &ts,
                );
                store.upsert_translation(&failed).await?;
                store
                    .add_alert(
                        &run.run_key,
                        "warn",
                        &format!(
                            "Chinese {} failed for {}: {err}",
                            c.label(),
                            run.trading_date
                        ),
                    )
                    .await?;
            }
            Ok(TickOutcome::Retrying(err))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::NewsReason;
    use crate::facts::Quote;
    use crate::providers::{ProviderError, SearchHit};
    use crate::store::{MemStore, PostRow, RunRow, TranslationRow};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::cell::Cell;

    const ARTICLE: &str = "Apple shares jumped 8% in premarket trading after the company reported record iPhone sales for the quarter.";
    const QUOTE: &str = "Apple shares jumped 8% in premarket trading";
    const URL: &str = "https://news.test/aapl";
    const KEY: &str = "premarket:2026-10-29";

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn q(sym: &str, last: f64, pct: f64) -> Quote {
        Quote {
            symbol: sym.into(),
            last,
            prev_close: last / (1.0 + pct / 100.0),
            change_pct: pct,
            volume: Some(2_000_000),
        }
    }

    /// `ok = false` makes every call fail with HTTP 429.
    struct FakeMd {
        ok: bool,
        calls: Cell<usize>,
    }
    #[async_trait(?Send)]
    impl MarketData for FakeMd {
        async fn quote(&self, symbol: &str) -> Result<Quote, ProviderError> {
            self.calls.set(self.calls.get() + 1);
            if self.ok {
                Ok(q(symbol, 5800.0, 0.4))
            } else {
                Err(ProviderError::Http(429))
            }
        }
        async fn premarket_quotes(
            &self,
            symbols: &[String],
            _now: DateTime<Utc>,
        ) -> Vec<(String, Result<Quote, ProviderError>)> {
            self.calls.set(self.calls.get() + 1);
            symbols
                .iter()
                .map(|s| {
                    let r = if !self.ok {
                        Err(ProviderError::Http(429))
                    } else if s == "AAPL" {
                        Ok(q("AAPL", 230.0, 8.0))
                    } else {
                        Err(ProviderError::Http(404))
                    };
                    (s.clone(), r)
                })
                .collect()
        }
    }
    fn md(ok: bool) -> FakeMd {
        FakeMd {
            ok,
            calls: Cell::new(0),
        }
    }

    struct FakeNs;
    #[async_trait(?Send)]
    impl NewsSearch for FakeNs {
        async fn search(&self, query: &str, _max: usize) -> Result<Vec<SearchHit>, ProviderError> {
            if query.contains("AAPL") {
                Ok(vec![SearchHit {
                    url: URL.into(),
                    title: "Apple jumps".into(),
                    content: ARTICLE.into(),
                }])
            } else {
                Err(ProviderError::Empty)
            }
        }
    }

    /// Reason extraction always answers with the anchored Apple reason;
    /// drafts answer clean or bad per language.
    struct FakeLlm {
        en_ok: Cell<bool>,
        zh_ok: Cell<bool>,
        calls: Cell<usize>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, system: &str, _user: &str) -> Result<Value, ProviderError> {
            self.calls.set(self.calls.get() + 1);
            if system.starts_with("You write the US premarket") {
                Ok(draft_reply(false, self.en_ok.get()))
            } else if system.starts_with("你负责撰写美股盘前晨报") {
                Ok(draft_reply(true, self.zh_ok.get()))
            } else {
                Ok(
                    json!({"reasons": [{"url": URL, "title": "x", "quote": QUOTE, "summary_en": "Apple rose on record iPhone sales."}]}),
                )
            }
        }
    }
    fn llm(en_ok: bool, zh_ok: bool) -> FakeLlm {
        FakeLlm {
            en_ok: Cell::new(en_ok),
            zh_ok: Cell::new(zh_ok),
            calls: Cell::new(0),
        }
    }
    fn draft_reply(zh: bool, ok: bool) -> Value {
        // The built facts have no watch list, so there is no watch section.
        let heads: [&str; 3] = if zh {
            ["期货", "盘前异动", "宏观与隔夜"]
        } else {
            ["Futures", "Biggest premarket movers", "Macro & overnight"]
        };
        // 7.5% is not in the facts, so a bad reply fails the number check.
        let body = if ok {
            "Futures were firm."
        } else {
            "Nasdaq futures rose 7.5%."
        };
        json!({
            "title": "Futures firm; Apple jumps",
            "summary": "Futures edged up while Apple rose.",
            "sections": heads.iter().map(|h| json!({"heading": h, "markdown": body})).collect::<Vec<_>>(),
        })
    }

    fn run_status(s: &MemStore) -> String {
        s.runs.borrow()[KEY].status.clone()
    }
    fn tr_status(s: &MemStore, lang: &str) -> String {
        s.translations.borrow()[&(1, lang.to_string())]
            .status
            .clone()
    }

    #[test]
    fn run_key_format() {
        assert_eq!(run_key(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()), KEY);
    }

    #[tokio::test]
    async fn two_ticks_same_time_make_one_post() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        let now = utc("2026-10-29T12:30:00Z");
        assert_eq!(tick(now, &s, &m, &FakeNs, &l).await, TickOutcome::Published);
        assert_eq!(tick(now, &s, &m, &FakeNs, &l).await, TickOutcome::Nothing);
        let posts = s.posts.borrow();
        assert_eq!(posts.len(), 1);
        let p = &posts[0];
        assert_eq!(p.id, Some(1));
        assert_eq!(p.run_key, KEY);
        assert_eq!(p.content_type, "premarket");
        assert_eq!(p.template_id, "premarket:v1");
        assert_eq!(p.trading_date, "2026-10-29");
        assert_eq!(p.slug, "premarket-2026-10-29");
        assert_eq!(p.facts_r2_key, "premarket/2026-10-29.json");
        assert_eq!(p.published_at.as_deref(), Some("2026-10-29T12:30:00Z"));
        drop(posts);
        assert_eq!(run_status(&s), "published");
        assert_eq!(s.runs.borrow()[KEY].attempts, 1);
        assert_eq!(tr_status(&s, "en"), "published");
        assert_eq!(tr_status(&s, "zh"), "published");
        let en = s.translations.borrow()[&(1, "en".to_string())].clone();
        assert_eq!(en.title, "Futures firm; Apple jumps");
        assert!(en.body_md.starts_with("# Futures firm; Apple jumps"));
        assert_eq!(en.updated_at, "2026-10-29T12:30:00Z");
        assert!(s.facts.borrow().contains_key("premarket/2026-10-29.json"));
        let src = s.sources.borrow()[&1].clone();
        assert_eq!(src.len(), 1);
        assert_eq!(src[0].url, URL);
        assert!(s.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn stale_building_rerun_reuses_post() {
        let s = MemStore::default();
        // A crashed build left a post and a `building` run 15 minutes old.
        s.posts.borrow_mut().push(PostRow {
            id: Some(7),
            run_key: KEY.into(),
            content_type: "premarket".into(),
            template_id: "premarket:v1".into(),
            trading_date: "2026-10-29".into(),
            slug: "premarket-2026-10-29".into(),
            facts_r2_key: "premarket/2026-10-29.json".into(),
            published_at: None,
        });
        s.runs
            .borrow_mut()
            .insert(KEY.into(), building_run("2026-10-29T12:30:00Z"));
        let out = tick(
            utc("2026-10-29T12:45:00Z"),
            &s,
            &md(true),
            &FakeNs,
            &llm(true, true),
        )
        .await;
        assert_eq!(out, TickOutcome::Published);
        assert_eq!(s.posts.borrow().len(), 1);
        assert_eq!(s.posts.borrow()[0].id, Some(7));
        assert_eq!(s.runs.borrow()[KEY].attempts, 2);
        assert!(s.translations.borrow().contains_key(&(7, "en".to_string())));
    }

    fn building_run(updated_at: &str) -> RunRow {
        RunRow {
            run_key: KEY.into(),
            content_type: "premarket".into(),
            trading_date: "2026-10-29".into(),
            status: "building".into(),
            attempts: 1,
            last_error: None,
            created_at: "2026-10-29T12:30:00Z".into(),
            updated_at: updated_at.into(),
        }
    }

    #[tokio::test]
    async fn fresh_building_run_is_left_alone() {
        let s = MemStore::default();
        s.runs
            .borrow_mut()
            .insert(KEY.into(), building_run("2026-10-29T12:44:00Z"));
        let (m, l) = (md(true), llm(true, true));
        let out = tick(utc("2026-10-29T12:45:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Nothing);
        assert_eq!(m.calls.get(), 0);
        assert_eq!(l.calls.get(), 0);
        assert_eq!(s.runs.borrow()[KEY], building_run("2026-10-29T12:44:00Z"));
    }

    #[tokio::test]
    async fn all_429_retries_then_skips_once() {
        let s = MemStore::default();
        let (m, l) = (md(false), llm(true, true));
        let n = crate::premarket::universe().len();
        let want = format!(
            "futures, movers | quotes ok=0 http429={n} http_other=0 empty=0 stale=0 parse=0 net=0 \
             skipped=0; \
             movers liquid=0 gainers=0 losers=0; reasons proposed=0 kept=0; \
             search_err=1 llm_err=0; futures ok=0 err=4"
        );
        for t in ["2026-10-29T12:30:00Z", "2026-10-29T12:35:00Z"] {
            match tick(utc(t), &s, &m, &FakeNs, &l).await {
                TickOutcome::Retrying(e) => assert_eq!(e, want),
                o => panic!("{o:?}"),
            }
            assert_eq!(run_status(&s), "pending");
            assert_eq!(s.runs.borrow()[KEY].last_error.as_deref(), Some(&*want));
        }
        assert_eq!(s.runs.borrow()[KEY].attempts, 2);
        assert_eq!(
            tick(utc("2026-10-29T13:25:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Skipped
        );
        assert_eq!(run_status(&s), "skipped");
        assert!(s.posts.borrow().is_empty());
        {
            let alerts = s.alerts.borrow();
            assert_eq!(alerts.len(), 1);
            assert_eq!((alerts[0].0.as_str(), alerts[0].1.as_str()), (KEY, "error"));
        }
        assert_eq!(
            tick(utc("2026-10-29T13:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert_eq!(s.alerts.borrow().len(), 1);
        assert!(s.posts.borrow().is_empty());
    }

    #[tokio::test]
    async fn extra_diagnostics_reach_the_log_and_last_error() {
        let s = MemStore::default();
        let (m, l) = (md(false), llm(true, true));
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let extra = || vec!["subrequests=7".to_string()];
        let hooks = Hooks {
            log: &log,
            extra: &extra,
        };
        let out = tick_with_hooks(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l, &hooks).await;
        match out {
            TickOutcome::Retrying(e) => {
                assert!(e.starts_with("futures, movers | quotes ok=0 "), "{e}");
                assert!(e.ends_with("; futures ok=0 err=4; subrequests=7"), "{e}");
            }
            o => panic!("{o:?}"),
        }
        assert!(
            logs.borrow()[0].ends_with("; subrequests=7"),
            "{:?}",
            logs.borrow()
        );
        // Stored with the run, not with the facts.
        assert!(s.runs.borrow()[KEY]
            .last_error
            .as_deref()
            .unwrap()
            .ends_with("subrequests=7"));
    }

    #[tokio::test]
    async fn build_logs_diagnostics_once() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let now = utc("2026-10-29T12:30:00Z");
        assert_eq!(
            tick_with_log(now, &s, &m, &FakeNs, &l, &log).await,
            TickOutcome::Published
        );
        {
            let logs = logs.borrow();
            assert_eq!(logs.len(), 1, "{logs:?}");
            assert!(logs[0].starts_with("diagnostics: quotes ok=1 "), "{logs:?}");
            assert!(
                logs[0].contains("; movers liquid=1 gainers=1 losers=0"),
                "{logs:?}"
            );
        }
        // No build, no diagnostics line.
        tick_with_log(now, &s, &m, &FakeNs, &l, &log).await;
        assert_eq!(logs.borrow().len(), 1);
    }

    #[tokio::test]
    async fn en_ok_zh_bad_then_zh_retry_publishes() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        assert_eq!(
            tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::PublishedEnOnly
        );
        assert_eq!(run_status(&s), "published");
        assert_eq!(tr_status(&s, "en"), "published");
        let zh = s.translations.borrow()[&(1, "zh".to_string())].clone();
        assert_eq!(zh.status, "pending");
        assert!(zh.title.is_empty() && zh.summary.is_empty() && zh.body_md.is_empty());
        assert!(s.posts.borrow()[0].published_at.is_some());

        // Still bad: stays pending.
        assert!(matches!(
            tick(utc("2026-10-29T12:35:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert_eq!(tr_status(&s, "zh"), "pending");

        // ZH retries continue past 09:25 ET.
        l.zh_ok.set(true);
        assert_eq!(
            tick(utc("2026-10-29T15:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        assert_eq!(tr_status(&s, "zh"), "published");
        assert!(s.translations.borrow()[&(1, "zh".to_string())]
            .body_md
            .contains("## 期货"));
        assert_eq!(s.posts.borrow().len(), 1);
        assert_eq!(
            tick(utc("2026-10-29T15:05:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
    }

    #[tokio::test]
    async fn zh_retries_every_5_min_in_window_then_hourly() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        assert_eq!(
            tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::PublishedEnOnly
        );
        // 08:35 and 09:25 ET: inside the window, every firing retries.
        for t in ["2026-10-29T12:35:00Z", "2026-10-29T13:25:00Z"] {
            let before = l.calls.get();
            assert!(matches!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Retrying(_)
            ));
            assert!(l.calls.get() > before, "{t}");
        }
        // After the window, 5-minute firings off the hour make no LLM call.
        let before = l.calls.get();
        for t in [
            "2026-10-29T13:30:00Z",
            "2026-10-29T14:05:00Z",
            "2026-10-29T21:55:00Z",
        ] {
            assert_eq!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing,
                "{t}"
            );
        }
        assert_eq!(l.calls.get(), before);
        // 10:00 ET retries on the hour.
        assert!(matches!(
            tick(utc("2026-10-29T14:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert!(l.calls.get() > before);
        // The 18:00 ET give-up always runs, on or off the hour.
        assert!(matches!(
            tick(utc("2026-10-29T22:05:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert_eq!(tr_status(&s, "zh"), "failed");
    }

    #[tokio::test]
    async fn zh_failing_at_1805_et_is_failed_with_one_warn() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        assert_eq!(
            tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::PublishedEnOnly
        );
        // 17:00 ET: still pending.
        assert!(matches!(
            tick(utc("2026-10-29T21:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert_eq!(tr_status(&s, "zh"), "pending");
        assert!(s.alerts.borrow().is_empty());
        // 18:05 ET: give up.
        assert!(matches!(
            tick(utc("2026-10-29T22:05:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying(_)
        ));
        assert_eq!(tr_status(&s, "zh"), "failed");
        assert_eq!(run_status(&s), "published");
        {
            let alerts = s.alerts.borrow();
            assert_eq!(alerts.len(), 1);
            assert_eq!((alerts[0].0.as_str(), alerts[0].1.as_str()), (KEY, "warn"));
        }
        assert_eq!(
            tick(utc("2026-10-29T23:00:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert_eq!(s.alerts.borrow().len(), 1);
    }

    #[tokio::test]
    async fn en_error_publishes_nothing() {
        let s = MemStore::default();
        let out = tick(
            utc("2026-10-29T12:30:00Z"),
            &s,
            &md(true),
            &FakeNs,
            &llm(false, true),
        )
        .await;
        assert!(matches!(out, TickOutcome::Retrying(_)), "{out:?}");
        assert_eq!(run_status(&s), "pending");
        assert!(s.runs.borrow()[KEY]
            .last_error
            .as_deref()
            .unwrap()
            .starts_with("en: "));
        assert!(s.posts.borrow().is_empty());
        assert!(s.translations.borrow().is_empty());
    }

    #[tokio::test]
    async fn thanksgiving_is_nothing() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        assert_eq!(
            tick(utc("2026-11-26T13:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert!(s.runs.borrow().is_empty());
        assert_eq!(m.calls.get(), 0);
    }

    #[tokio::test]
    async fn too_early_is_nothing() {
        let s = MemStore::default();
        let out = tick(
            utc("2026-10-29T12:25:00Z"),
            &s,
            &md(true),
            &FakeNs,
            &llm(true, true),
        )
        .await;
        assert_eq!(out, TickOutcome::Nothing);
        assert!(s.runs.borrow().is_empty());
    }
    /// MemStore that fails selected writes once each, and can fail `get_run`.
    #[derive(Default)]
    struct FailStore {
        inner: MemStore,
        fail_published_run: Cell<bool>,
        fail_zh_row: Cell<bool>,
        fail_get_run: Cell<bool>,
        get_run_calls: Cell<usize>,
        /// `(call number, status)`: on that `get_run` call, first set the
        /// stored run to `status`, as a concurrent tick finishing would.
        race_on_get_run: Cell<Option<(usize, &'static str)>>,
    }
    #[async_trait(?Send)]
    impl Store for FailStore {
        async fn get_run(&self, key: &str) -> Result<Option<RunRow>, String> {
            if self.fail_get_run.get() {
                return Err("d1: get_run failed".into());
            }
            let n = self.get_run_calls.get() + 1;
            self.get_run_calls.set(n);
            if let Some((_, status)) = self.race_on_get_run.get().filter(|(at, _)| *at == n) {
                if let Some(r) = self.inner.runs.borrow_mut().get_mut(key) {
                    r.status = status.into();
                }
            }
            self.inner.get_run(key).await
        }
        async fn upsert_run(&self, r: &RunRow) -> Result<(), String> {
            if r.status == "published" && self.fail_published_run.replace(false) {
                return Err("d1: upsert_run failed".into());
            }
            self.inner.upsert_run(r).await
        }
        async fn get_post_by_run(&self, run_key: &str) -> Result<Option<PostRow>, String> {
            self.inner.get_post_by_run(run_key).await
        }
        async fn upsert_post(&self, p: &PostRow) -> Result<i64, String> {
            self.inner.upsert_post(p).await
        }
        async fn get_translation(
            &self,
            post_id: i64,
            lang: &str,
        ) -> Result<Option<TranslationRow>, String> {
            self.inner.get_translation(post_id, lang).await
        }
        async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String> {
            if t.lang == "zh" && self.fail_zh_row.replace(false) {
                return Err("d1: zh row failed".into());
            }
            self.inner.upsert_translation(t).await
        }
        async fn add_sources(&self, post_id: i64, s: &[NewsReason]) -> Result<(), String> {
            self.inner.add_sources(post_id, s).await
        }
        async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String> {
            self.inner.add_alert(run_key, level, msg).await
        }
        async fn has_alert(&self, run_key: &str) -> Result<bool, String> {
            self.inner.has_alert(run_key).await
        }
        async fn put_facts(&self, key: &str, json: &str) -> Result<(), String> {
            self.inner.put_facts(key, json).await
        }
        async fn get_facts(&self, key: &str) -> Result<Option<String>, String> {
            self.inner.get_facts(key).await
        }
    }

    /// First tick publishes EN, then the final run write fails.
    async fn en_written_run_write_failed() -> (FailStore, FakeMd, FakeLlm, TranslationRow, PostRow)
    {
        let s = FailStore::default();
        s.fail_published_run.set(true);
        let (m, l) = (md(true), llm(true, true));
        let out = tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Retrying("d1: upsert_run failed".into()));
        assert_eq!(run_status(&s.inner), "building");
        let en = s.inner.translations.borrow()[&(1, "en".to_string())].clone();
        assert_eq!(en.status, "published");
        let post = s.inner.posts.borrow()[0].clone();
        (s, m, l, en, post)
    }

    #[tokio::test]
    async fn failed_run_write_is_reconciled_without_rebuild() {
        let (s, m, l, en, post) = en_written_run_write_failed().await;
        let (md_calls, llm_calls) = (m.calls.get(), l.calls.get());
        // Within the lease the half-written run still looks in flight.
        let out = tick(utc("2026-10-29T12:35:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Nothing);
        assert_eq!(run_status(&s.inner), "building");
        // Once the lease expires it is reconciled.
        let out = tick(utc("2026-10-29T12:45:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Nothing);
        assert_eq!(run_status(&s.inner), "published");
        assert_eq!((m.calls.get(), l.calls.get()), (md_calls, llm_calls));
        assert_eq!(s.inner.translations.borrow()[&(1, "en".to_string())], en);
        assert_eq!(s.inner.posts.borrow().as_slice(), &[post]);
        assert_eq!(tr_status(&s.inner, "zh"), "published");
        assert_eq!(s.inner.sources.borrow()[&1].len(), 1);
    }

    #[tokio::test]
    async fn failed_run_write_is_not_skipped_past_deadline() {
        let (s, m, l, en, post) = en_written_run_write_failed().await;
        let out = tick(utc("2026-10-29T13:25:00Z"), &s, &m, &FakeNs, &l).await;
        assert_ne!(out, TickOutcome::Skipped);
        assert_eq!(run_status(&s.inner), "published");
        assert!(s.inner.alerts.borrow().iter().all(|a| a.1 != "error"));
        assert_eq!(s.inner.translations.borrow()[&(1, "en".to_string())], en);
        assert_eq!(s.inner.posts.borrow().as_slice(), &[post]);
    }

    #[tokio::test]
    async fn failed_zh_row_write_is_retried_later() {
        let s = FailStore::default();
        s.fail_zh_row.set(true);
        let (m, l) = (md(true), llm(true, true));
        let out = tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Retrying("d1: zh row failed".into()));
        assert!(!s
            .inner
            .translations
            .borrow()
            .contains_key(&(1, "zh".to_string())));
        let (md_calls, llm_calls) = (m.calls.get(), l.calls.get());
        // After the build lease expires.
        let out = tick(utc("2026-10-29T12:45:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Published);
        assert_eq!(run_status(&s.inner), "published");
        assert_eq!(tr_status(&s.inner, "zh"), "published");
        // Only the ZH draft ran: no market data, one LLM call.
        assert_eq!((m.calls.get(), l.calls.get()), (md_calls, llm_calls + 1));
        assert_eq!(s.inner.posts.borrow().len(), 1);
    }

    fn published_run() -> RunRow {
        RunRow {
            status: "published".into(),
            ..building_run("2026-10-29T12:30:00Z")
        }
    }

    #[tokio::test]
    async fn get_run_read_error_retries_without_writes() {
        // Empty store and a store holding a published run: a read error on the
        // run must never look like "no row".
        for seeded in [false, true] {
            let s = FailStore::default();
            if seeded {
                s.inner
                    .runs
                    .borrow_mut()
                    .insert(KEY.into(), published_run());
            }
            s.fail_get_run.set(true);
            let (m, l) = (md(true), llm(true, true));
            for now in ["2026-10-29T12:30:00Z", "2026-10-29T13:30:00Z"] {
                let out = tick(utc(now), &s, &m, &FakeNs, &l).await;
                assert_eq!(out, TickOutcome::Retrying("d1: get_run failed".into()));
            }
            assert_eq!((m.calls.get(), l.calls.get()), (0, 0));
            if seeded {
                assert_eq!(s.inner.runs.borrow()[KEY], published_run());
            } else {
                assert!(s.inner.runs.borrow().is_empty());
            }
            assert!(s.inner.posts.borrow().is_empty());
            assert!(s.inner.translations.borrow().is_empty());
            assert!(s.inner.facts.borrow().is_empty());
            assert!(s.inner.alerts.borrow().is_empty());
        }
    }

    #[tokio::test]
    async fn past_deadline_fresh_building_is_nothing() {
        // 10 minutes old: a realistic build still running past 09:25 ET.
        let s = MemStore::default();
        s.runs
            .borrow_mut()
            .insert(KEY.into(), building_run("2026-10-29T13:20:00Z"));
        let (m, l) = (md(true), llm(true, true));
        let out = tick(utc("2026-10-29T13:30:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Nothing);
        assert_eq!((m.calls.get(), l.calls.get()), (0, 0));
        assert_eq!(s.runs.borrow()[KEY], building_run("2026-10-29T13:20:00Z"));
        assert!(s.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn past_deadline_stale_building_without_en_skips_once() {
        let s = MemStore::default();
        s.runs
            .borrow_mut()
            .insert(KEY.into(), building_run("2026-10-29T13:10:00Z"));
        let (m, l) = (md(true), llm(true, true));
        let out = tick(utc("2026-10-29T13:30:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Skipped);
        assert_eq!(run_status(&s), "skipped");
        let out = tick(utc("2026-10-29T13:35:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Nothing);
        let alerts = s.alerts.borrow();
        assert_eq!(alerts.len(), 1);
        assert_eq!((alerts[0].0.as_str(), alerts[0].1.as_str()), (KEY, "error"));
        assert!(s.posts.borrow().is_empty());
        assert_eq!((m.calls.get(), l.calls.get()), (0, 0));
    }

    #[tokio::test]
    async fn build_superseded_at_reread_writes_nothing() {
        // Call 1 is the tick's own read; call 2 is the re-read just before the
        // post rows, by which time another tick finished the run.
        for status in ["published", "skipped"] {
            let s = FailStore::default();
            s.race_on_get_run.set(Some((2, status)));
            let (m, l) = (md(true), llm(true, true));
            let out = tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await;
            assert_eq!(out, TickOutcome::Nothing, "{status}");
            assert_eq!(s.get_run_calls.get(), 2);
            assert!(l.calls.get() > 0, "the build ran");
            assert_eq!(run_status(&s.inner), status);
            assert!(s.inner.posts.borrow().is_empty());
            assert!(s.inner.translations.borrow().is_empty());
            assert!(s.inner.facts.borrow().is_empty());
            assert!(s.inner.sources.borrow().is_empty());
            assert!(s.inner.alerts.borrow().is_empty());
        }
    }

    #[tokio::test]
    async fn failed_build_does_not_reopen_a_finished_run() {
        // EN fails, but another tick skipped the run meanwhile: stay skipped.
        let s = FailStore::default();
        s.race_on_get_run.set(Some((2, "skipped")));
        let (m, l) = (md(true), llm(false, true));
        let out = tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(out, TickOutcome::Nothing);
        assert_eq!(run_status(&s.inner), "skipped");
    }
}
