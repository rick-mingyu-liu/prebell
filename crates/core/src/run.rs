//! The run state machine shared by every scheduled content type. Each cron
//! firing calls `tick_content`, which decides from the ET clock and the stored
//! run what to do. Re-running a tick never duplicates a post: the run key is
//! the idempotency key. `Content` supplies what differs per type.

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, SecondsFormat, Timelike, Utc};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::clock::{decide, et, premarket_window, trading_day, DayKind, SlotDecision, Window};
use crate::cover::{
    brief_graphic, brief_lines, cover_movers, index_snapshot, store_graphic, CoverLines,
    IndexSnapshot, Session, INDEX_LINE_KEY,
};
use crate::events::watch::WatchSource;
use crate::facts::{missing_required, NewsReason, PremarketFacts};
use crate::numbers::Lang;
use crate::premarket::build_facts;
use crate::premarket::draft::{source_reasons, write, Draft, TEMPLATE_ID};
use crate::providers::{LlmProvider, MarketData, NewsSearch};
use crate::store::{PostRow, RunRow, Store, TranslationRow};
use crate::why;

pub mod update;

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
    /// `budget` returns the subrequests used so far; optional work checks it
    /// with `crate::budget::allows` before spending more.
    async fn build_facts(
        &self,
        date: NaiveDate,
        now: DateTime<Utc>,
        budget: &dyn Fn() -> usize,
    ) -> Self::Facts;
    /// Names of required sections that are unavailable; empty = publishable.
    fn missing_required(&self, f: &Self::Facts) -> Vec<&'static str>;
    async fn write(&self, f: &Self::Facts, lang: Lang) -> Result<(Draft, String), Vec<String>>;
    /// The post's sources, one per url.
    fn sources(&self, f: &Self::Facts) -> Vec<NewsReason>;
    /// Build counters carried in the facts, for the log and `last_error`.
    fn diagnostics(&self, f: &Self::Facts) -> Vec<String>;
    /// The two lines printed on the post's cover.
    fn cover_lines(&self, f: &Self::Facts) -> CoverLines;
    /// The market snapshot the front page's index line is built from.
    fn index_snapshot(&self, f: &Self::Facts) -> IndexSnapshot;
    /// The cover's data graphic (an SVG), or `None` when the facts give no
    /// graphic and the post keeps the flat tile.
    fn cover_graphic(&self, f: &Self::Facts) -> Option<String>;

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
    /// Today's events and after-close earnings for "What to watch"; `None`
    /// leaves the section out.
    pub watch: Option<&'a dyn WatchSource>,
    /// Read once for the reported results that explain a mover without a
    /// news reason (`why`); `None` leaves only the theme fallback.
    pub reports: Option<&'a dyn Store>,
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
    /// Every firing inside the 08:00-09:25 ET window, then hourly on the
    /// hour: the cron fires every 5 minutes all day.
    fn zh_retry_due(&self, t: NaiveTime) -> bool {
        let w = premarket_window(DayKind::Full);
        (w.start..=w.end).contains(&t) || t.minute() == 0
    }
    async fn build_facts(
        &self,
        date: NaiveDate,
        now: DateTime<Utc>,
        budget: &dyn Fn() -> usize,
    ) -> PremarketFacts {
        let mut f = build_facts(self.md, self.ns, self.llm, date, now, budget).await;
        if let Some(w) = self.watch {
            f.watch_list = w.watch_list(date, now, budget).await;
        }
        let needed = f
            .gainers
            .iter()
            .chain(&f.losers)
            .any(|m| m.reasons.is_empty());
        let (reports, reports_err) = match self.reports {
            Some(s) => why::read_reports(s, date, needed).await,
            None => (vec![], 0),
        };
        let counts = why::fill(
            f.gainers.iter_mut().chain(f.losers.iter_mut()),
            &f.themes,
            &reports,
            date,
            Session::Premarket,
        );
        f.diagnostics.extend(why::diagnostics(counts, reports_err));
        f
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
    fn cover_lines(&self, f: &PremarketFacts) -> CoverLines {
        brief_lines(&f.futures, &f.gainers, &f.losers, true)
    }
    fn index_snapshot(&self, f: &PremarketFacts) -> IndexSnapshot {
        index_snapshot("premarket", f.as_of_utc, &f.futures)
    }
    fn cover_graphic(&self, f: &PremarketFacts) -> Option<String> {
        brief_graphic(&cover_movers(&f.gainers, &f.losers), Session::Premarket)
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

/// `tick`, passing the build's diagnostics line to `log` once when it builds
/// (and the cover outcome, `cover=ok|err|none`, when it gets that far).
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
        budget: &|| 0,
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
    tick_with_watch(now, store, md, ns, llm, None, hooks).await
}

/// `tick_with_hooks` with "What to watch" from `watch`.
pub async fn tick_with_watch(
    now: DateTime<Utc>,
    store: &dyn Store,
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    watch: Option<&dyn WatchSource>,
    hooks: &Hooks<'_>,
) -> TickOutcome {
    let c = Premarket {
        md,
        ns,
        llm,
        watch,
        reports: Some(store),
    };
    tick_content(now, store, &c, hooks).await
}

/// What the caller adds to a tick.
pub struct Hooks<'a> {
    /// Receives the build's diagnostics line, once when the facts are built
    /// (before the drafts), then `cover=ok|err|none` after the cover write,
    /// and a failed index-line write.
    pub log: &'a dyn Fn(&str),
    /// Lines appended to the build's diagnostics (log and `last_error`, not
    /// the stored facts), read when the facts are built, e.g. the Worker's
    /// subrequest count.
    pub extra: &'a dyn Fn() -> Vec<String>,
    /// Subrequests used so far (the Worker's fetch count); read by
    /// `build_facts` to skip optional work near the Free plan's limit.
    pub budget: &'a dyn Fn() -> usize,
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
    let facts: Option<C::Facts> = match store.get_facts(&post.facts_r2_key).await? {
        Some(json) => Some(serde_json::from_str(&json).map_err(|e| format!("facts json: {e}"))?),
        None => None,
    };
    if store.get_translation(post_id, "zh").await?.is_none() {
        let pending = translation_zh(
            post_id,
            "pending",
            String::new(),
            String::new(),
            String::new(),
            facts.as_ref().map(|f| c.cover_lines(f).zh),
            &ts,
        );
        store.upsert_translation(&pending).await?;
    }
    // The failed write may have been `add_sources`; it replaces, so redo it.
    if let Some(facts) = &facts {
        store.add_sources(post_id, &c.sources(facts)).await?;
    }
    let mut row = run.unwrap_or_else(|| new_run(now, c.run_key(date), c.content_type(), date));
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
    let mut row = run.unwrap_or_else(|| new_run(now, c.run_key(date), c.content_type(), date));
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

/// A `pending` run with no attempts yet.
fn new_run(now: DateTime<Utc>, run_key: String, content_type: &str, date: NaiveDate) -> RunRow {
    RunRow {
        run_key,
        content_type: content_type.into(),
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
    let mut row = run.unwrap_or_else(|| new_run(now, c.run_key(date), c.content_type(), date));
    row.status = "building".into();
    row.attempts += 1;
    row.updated_at = ts;
    store.upsert_run(&row).await?;

    let facts = c.build_facts(date, now, hooks.budget).await;
    let mut lines = c.diagnostics(&facts);
    lines.extend((hooks.extra)());
    // Logged now, before the drafts, so the line survives a build that dies
    // in them (CPU or subrequest limits).
    (hooks.log)(&format!("diagnostics: {}", lines.join("; ")));
    publish(now, store, c, hooks, date, row, &facts, &lines).await
}

/// The rest of `build` once the facts are in: gate, drafts, cover, rows.
/// `lines` (the diagnostics) go into `last_error` when the gate fails; the
/// cover outcome is logged on its own line.
#[allow(clippy::too_many_arguments)]
async fn publish<C: Content>(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &C,
    hooks: &Hooks<'_>,
    date: NaiveDate,
    mut row: RunRow,
    facts: &C::Facts,
    lines: &[String],
) -> Result<TickOutcome, String> {
    let ts = iso(now);
    if let Some(err) = gate_error(&c.missing_required(facts), lines) {
        return retry_later(store, row, err).await;
    }

    let (en, en_md) = match c.write(facts, Lang::En).await {
        Ok(x) => x,
        Err(e) => return retry_later(store, row, format!("en: {}", e.join("; "))).await,
    };
    let zh = c.write(facts, Lang::Zh).await;

    // The build took minutes; another tick may have finished this run since
    // it was read. If so, keep its post and facts untouched.
    if superseded(store, &row.run_key).await? {
        return Ok(TickOutcome::Nothing);
    }
    let slug = c.slug(date);
    let (cover_key, outcome, cover_err) = store_graphic(store, &slug, c.cover_graphic(facts)).await;
    (hooks.log)(&format!("cover={outcome}"));
    if let Some(e) = cover_err {
        (hooks.log)(&format!("cover write failed: {e}"));
    }
    let facts_key = c.facts_key(date);
    let json = serde_json::to_string(facts).map_err(|e| format!("facts json: {e}"))?;
    store.put_facts(&facts_key, &json).await?;

    let post_id = store
        .upsert_post(&PostRow {
            id: None,
            run_key: row.run_key.clone(),
            content_type: c.content_type().into(),
            template_id: c.template_id().into(),
            trading_date: row.trading_date.clone(),
            slug,
            facts_r2_key: facts_key,
            published_at: Some(ts.clone()),
            cover_key,
            revised_at: None,
        })
        .await?;
    // The front page's index line; a failed write keeps the previous one.
    let index_line = match serde_json::to_string(&c.index_snapshot(facts)) {
        Ok(json) => store.kv_set(INDEX_LINE_KEY, &json, &ts).await,
        Err(e) => Err(format!("index snapshot json: {e}")),
    };
    if let Err(e) = index_line {
        (hooks.log)(&format!("index line not written: {e}"));
    }
    let cover = c.cover_lines(facts);
    let [en_line1, en_line2] = cover.en;
    store
        .upsert_translation(&TranslationRow {
            post_id,
            lang: "en".into(),
            status: "published".into(),
            title: en.title,
            summary: en.summary,
            body_md: en_md,
            updated_at: ts.clone(),
            cover_line1: Some(en_line1),
            cover_line2: Some(en_line2),
        })
        .await?;
    let (zh_row, zh_err) = match zh {
        Ok((d, body)) => (
            translation_zh(
                post_id,
                "published",
                d.title,
                d.summary,
                body,
                Some(cover.zh),
                &ts,
            ),
            None,
        ),
        Err(e) => (
            translation_zh(
                post_id,
                "pending",
                String::new(),
                String::new(),
                String::new(),
                Some(cover.zh),
                &ts,
            ),
            Some(format!("zh: {}", e.join("; "))),
        ),
    };
    store.upsert_translation(&zh_row).await?;
    store.add_sources(post_id, &c.sources(facts)).await?;

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

/// The `last_error` of a build whose facts lack the `missing` required
/// sections, with the diagnostics `lines`; `None` when nothing is missing.
fn gate_error(missing: &[&str], lines: &[String]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    let diagnostics = lines.join("; ");
    let err = missing.join(", ");
    Some(if diagnostics.is_empty() {
        err
    } else {
        format!("{err} | {diagnostics}")
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

/// A ZH row. Every write carries the cover `lines`: the upsert replaces the
/// whole row, so a write without them would erase them.
fn translation_zh(
    post_id: i64,
    status: &str,
    title: String,
    summary: String,
    body_md: String,
    lines: Option<[String; 2]>,
    ts: &str,
) -> TranslationRow {
    let [cover_line1, cover_line2] = lines.map_or([None, None], |l| l.map(Some));
    TranslationRow {
        post_id,
        lang: "zh".into(),
        status: status.into(),
        title,
        summary,
        body_md,
        updated_at: ts.into(),
        cover_line1,
        cover_line2,
    }
}

/// Every kept reason (gainers, losers, macro, themes), first occurrence per
/// url: the same list the page's sources section shows.
fn sources(f: &PremarketFacts) -> Vec<NewsReason> {
    let mut out: Vec<NewsReason> = Vec::new();
    for r in source_reasons(f) {
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
    let lines = c.cover_lines(&facts).zh;
    // The premarket update may rewrite the post, ZH included, while this
    // draft runs from the older facts: each write below lands only at the
    // revision read above, and otherwise writes nothing.
    let revision = post.revised_at.as_deref();
    match c.write(&facts, Lang::Zh).await {
        Ok((d, body)) => {
            let published = translation_zh(
                post_id,
                "published",
                d.title,
                d.summary,
                body,
                Some(lines),
                &ts,
            );
            if !store
                .upsert_translation_at_revision(&published, revision)
                .await?
            {
                return Ok(TickOutcome::Nothing);
            }
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
                    Some(lines),
                    &ts,
                );
                if !store
                    .upsert_translation_at_revision(&failed, revision)
                    .await?
                {
                    return Ok(TickOutcome::Nothing);
                }
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
    use crate::facts::Quote;
    use crate::facts::{Field, NewsReason};
    use crate::providers::{ProviderError, SearchHit};
    use crate::store::{MemStore, PostRow, RunRow, TranslationRow};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::cell::Cell;

    const ARTICLE: &str = "Apple shares jumped 8% in premarket trading after the company reported record iPhone sales for the quarter.";
    const QUOTE: &str = "Apple shares jumped 8% in premarket trading";
    pub(super) const URL: &str = "https://news.test/aapl";
    pub(super) const KEY: &str = "premarket:2026-10-29";

    pub(super) fn utc(s: &str) -> DateTime<Utc> {
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
    pub(super) struct FakeMd {
        ok: bool,
        pub(super) calls: Cell<usize>,
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
    pub(super) fn md(ok: bool) -> FakeMd {
        FakeMd {
            ok,
            calls: Cell::new(0),
        }
    }

    pub(super) struct FakeNs;
    #[async_trait(?Send)]
    impl NewsSearch for FakeNs {
        async fn search(&self, query: &str, _max: usize) -> Result<Vec<SearchHit>, ProviderError> {
            if query.contains("AAPL") {
                Ok(vec![SearchHit {
                    url: URL.into(),
                    title: "Apple jumps".into(),
                    content: ARTICLE.into(),
                    published: None,
                }])
            } else {
                Err(ProviderError::Empty)
            }
        }
    }

    /// Reason extraction always answers with the anchored Apple reason;
    /// drafts answer clean or bad per language.
    pub(super) struct FakeLlm {
        pub(super) en_ok: Cell<bool>,
        pub(super) zh_ok: Cell<bool>,
        pub(super) calls: Cell<usize>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
            self.calls.set(self.calls.get() + 1);
            if system.starts_with("You write the US premarket") {
                Ok(draft_reply(false, self.en_ok.get(), user))
            } else if system.starts_with("你负责撰写美股盘前晨报") {
                Ok(draft_reply(true, self.zh_ok.get(), user))
            } else {
                Ok(
                    json!({"reasons": [{"url": URL, "title": "x", "quote": QUOTE, "summary_en": "Apple rose on record iPhone sales."}]}),
                )
            }
        }
    }
    pub(super) fn llm(en_ok: bool, zh_ok: bool) -> FakeLlm {
        FakeLlm {
            en_ok: Cell::new(en_ok),
            zh_ok: Cell::new(zh_ok),
            calls: Cell::new(0),
        }
    }
    fn draft_reply(zh: bool, ok: bool, user: &str) -> Value {
        let mut heads: Vec<&str> = if zh {
            vec!["期货", "宏观与隔夜", "盘前异动"]
        } else {
            vec!["Futures", "Macro & overnight", "Biggest premarket movers"]
        };
        // A watch section only when the facts have a watch list (the
        // sections line at the end of the user message asks for it).
        let watch = if zh { "今日关注" } else { "What to watch" };
        if user.contains(&format!("\"{watch}\"")) {
            heads.push(watch);
        }
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

    /// A watch source that records each call and lists one item.
    #[derive(Default)]
    pub(crate) struct FakeWatch {
        pub calls: std::cell::RefCell<Vec<(NaiveDate, DateTime<Utc>)>>,
    }
    #[async_trait(?Send)]
    impl WatchSource for FakeWatch {
        async fn watch_list(
            &self,
            date: NaiveDate,
            now: DateTime<Utc>,
            _budget: &dyn Fn() -> usize,
        ) -> Field<Vec<crate::events::watch::WatchItem>> {
            self.calls.borrow_mut().push((date, now));
            Field::Missing {
                reason: "fake".into(),
            }
        }
    }

    #[tokio::test]
    async fn premarket_facts_take_the_watch_list_from_the_source() {
        let (md, llm) = (md(true), llm(true, true));
        let w = FakeWatch::default();
        let c = Premarket {
            md: &md,
            ns: &FakeNs,
            llm: &llm,
            watch: Some(&w),
            reports: None,
        };
        let date = "2026-10-29".parse().unwrap();
        let f = c
            .build_facts(date, utc("2026-10-29T12:00:00Z"), &|| 0)
            .await;
        assert_eq!(*w.calls.borrow(), [(date, utc("2026-10-29T12:00:00Z"))]);
        assert_eq!(
            f.watch_list,
            Field::Missing {
                reason: "fake".into()
            }
        );
    }

    #[tokio::test]
    async fn premarket_facts_fall_back_to_reported_results_then_themes() {
        use crate::facts::FallbackKind;
        use crate::store::ReportRow;
        /// AAPL up 8% (its news is found), and three optical names down
        /// without news.
        struct OpticalMd;
        #[async_trait(?Send)]
        impl MarketData for OpticalMd {
            async fn quote(&self, symbol: &str) -> Result<Quote, ProviderError> {
                Ok(q(symbol, 5800.0, 0.4))
            }
            async fn premarket_quotes(
                &self,
                symbols: &[String],
                _now: DateTime<Utc>,
            ) -> Vec<(String, Result<Quote, ProviderError>)> {
                let moves = [
                    ("AAPL", 8.0),
                    ("LITE", -5.55),
                    ("COHR", -5.45),
                    ("CIEN", -4.63),
                ];
                symbols
                    .iter()
                    .map(|s| {
                        let r = moves
                            .iter()
                            .find(|(t, _)| t == s)
                            .map(|(t, p)| q(t, 100.0, *p))
                            .ok_or(ProviderError::Http(404));
                        (s.clone(), r)
                    })
                    .collect()
            }
        }
        let s = MemStore::default();
        *s.reports.borrow_mut() = vec![ReportRow {
            ticker: "LITE".into(),
            report_date: "2026-10-28".into(),
            time_slot: "amc".into(),
            // 16:05 ET.
            accepted_at: vec!["2026-10-28T20:05:00Z".into()],
            proof_url: Some(
                "https://www.sec.gov/Archives/edgar/data/1633978/000163397826000041/0001633978-26-000041-index.htm"
                    .into(),
            ),
            post_slug: None,
        }];
        /// FakeNs, except that COHR's search completes and finds nothing;
        /// LITE's and CIEN's fail.
        struct OpticalNs;
        #[async_trait(?Send)]
        impl NewsSearch for OpticalNs {
            async fn search(
                &self,
                query: &str,
                max: usize,
            ) -> Result<Vec<SearchHit>, ProviderError> {
                if query.contains("COHR") {
                    Ok(vec![])
                } else {
                    FakeNs.search(query, max).await
                }
            }
        }
        let llm = llm(true, true);
        let c = Premarket {
            md: &OpticalMd,
            ns: &OpticalNs,
            llm: &llm,
            watch: None,
            reports: Some(&s),
        };
        let date = "2026-10-29".parse().unwrap();
        let f = c
            .build_facts(date, utc("2026-10-29T12:00:00Z"), &|| 0)
            .await;
        assert_eq!(f.themes[0].group, "Networking & optical");
        assert_eq!(f.gainers[0].fallback, None);
        let fb = |sym: &str| {
            f.losers
                .iter()
                .find(|m| m.quote.symbol == sym)
                .and_then(|m| m.fallback.clone())
                .unwrap()
        };
        assert_eq!(fb("LITE").kind, FallbackKind::Results);
        assert_eq!(
            fb("LITE").text_en,
            "Reported results after Wednesday's close"
        );
        assert!(fb("LITE").filing_url.is_some());
        assert_eq!(fb("COHR").kind, FallbackKind::Theme);
        assert_eq!(
            fb("COHR").text_en,
            "Fell with the networking & optical group (median −5.45%); no company news found"
        );
        // CIEN's search failed: the page claims no search.
        assert_eq!(
            fb("CIEN").text_en,
            "Fell with the networking & optical group (median −5.45%)"
        );
        assert_eq!(
            fb("CIEN").text_zh,
            "随网络与光通信板块下跌（中位数 −5.45%）"
        );
        let failed = |sym: &str| {
            f.losers
                .iter()
                .any(|m| m.quote.symbol == sym && m.search_failed)
        };
        assert!(failed("CIEN") && failed("LITE") && !failed("COHR"));
        assert!(!f.gainers[0].search_failed);
        assert_eq!(
            f.diagnostics.last().unwrap(),
            "why results=1 theme=2 reports_err=0"
        );
    }

    #[test]
    fn premarket_cover_and_snapshot_come_from_the_futures() {
        let (md, llm) = (md(true), llm(true, true));
        let c = Premarket {
            md: &md,
            ns: &FakeNs,
            llm: &llm,
            watch: None,
            reports: None,
        };
        let f = PremarketFacts {
            trading_date: "2026-10-29".parse().unwrap(),
            as_of_utc: utc("2026-10-29T12:30:09Z"),
            futures: vec![
                (
                    "S&P 500 futures".into(),
                    Field::Value(q("ES=F", 5800.0, -1.14)),
                ),
                (
                    "Nasdaq 100 futures".into(),
                    Field::Missing {
                        reason: "HTTP 429".into(),
                    },
                ),
            ],
            gainers: vec![crate::facts::Mover {
                quote: q("AAPL", 230.0, 8.0),
                name: "Apple".into(),
                reasons: vec![],
                search_failed: false,
                fallback: None,
            }],
            losers: vec![],
            macro_items: Field::Missing {
                reason: "n/a".into(),
            },
            watch_list: Field::Missing {
                reason: "n/a".into(),
            },
            themes: vec![],
            theme_candidates: vec![],
            diagnostics: vec![],
        };
        let lines = c.cover_lines(&f);
        assert_eq!(lines.en, ["S&P futures −1.14%", "AAPL +8.00%"]);
        assert_eq!(lines.zh[0], "标普500期货 −1.14%");
        let snap = c.index_snapshot(&f);
        assert_eq!(snap.kind, "premarket");
        assert_eq!(snap.as_of_utc, "2026-10-29T12:30:09Z");
        assert_eq!(snap.entries.len(), 1);
        assert!(c.cover_graphic(&f).unwrap().contains(">AAPL<"));
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
            cover_key: None,
            revised_at: None,
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
             movers liquid=0 gainers=0 losers=0; reasons proposed=0 kept=0 hedged=0; \
             search_err=3 llm_err=0 news_stale=0 news_noise=0; \
             macro queries=3 hits=0 items=0; themes found=0 kept=0 searched=0; \
             futures ok=0 err=4"
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

    /// A `Content` that only records what its `budget` closure returns and
    /// then fails the gate, so a tick stops right after the facts stage.
    struct BudgetProbe {
        seen: Cell<Option<usize>>,
    }

    #[async_trait(?Send)]
    impl Content for BudgetProbe {
        type Facts = u32;
        fn content_type(&self) -> &'static str {
            "premarket"
        }
        fn template_id(&self) -> &'static str {
            "probe:v1"
        }
        fn label(&self) -> &'static str {
            "probe"
        }
        fn window(&self, day: DayKind) -> Window {
            premarket_window(day)
        }
        fn zh_give_up(&self) -> NaiveTime {
            NaiveTime::from_hms_opt(18, 0, 0).expect("valid time")
        }
        async fn build_facts(
            &self,
            _date: NaiveDate,
            _now: DateTime<Utc>,
            budget: &dyn Fn() -> usize,
        ) -> u32 {
            self.seen.set(Some(budget()));
            0
        }
        fn missing_required(&self, _f: &u32) -> Vec<&'static str> {
            vec!["probe"]
        }
        async fn write(&self, _f: &u32, _lang: Lang) -> Result<(Draft, String), Vec<String>> {
            Err(vec!["probe writes nothing".into()])
        }
        fn sources(&self, _f: &u32) -> Vec<NewsReason> {
            Vec::new()
        }
        fn diagnostics(&self, _f: &u32) -> Vec<String> {
            Vec::new()
        }
        fn cover_lines(&self, _f: &u32) -> CoverLines {
            CoverLines {
                en: [String::new(), String::new()],
                zh: [String::new(), String::new()],
            }
        }
        fn index_snapshot(&self, _f: &u32) -> IndexSnapshot {
            IndexSnapshot {
                kind: "premarket".into(),
                as_of_utc: String::new(),
                entries: Vec::new(),
            }
        }
        fn cover_graphic(&self, _f: &u32) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn build_facts_receives_the_budget() {
        let s = MemStore::default();
        let probe = BudgetProbe {
            seen: Cell::new(None),
        };
        let used = Cell::new(37);
        let budget = || used.get();
        let hooks = Hooks {
            log: &|_| {},
            extra: &Vec::new,
            budget: &budget,
        };
        tick_content(utc("2026-10-29T12:30:00Z"), &s, &probe, &hooks).await;
        assert_eq!(probe.seen.get(), Some(37));
    }

    #[tokio::test]
    async fn near_the_subrequest_limit_the_brief_skips_release_fetches_and_publishes() {
        use crate::earnings::store::MemEarningsStore;
        use crate::events::check::new_row;
        use crate::events::store::MemEventStore;
        use crate::events::watch::StoreWatch;
        use crate::providers::{Http, HttpResp};

        /// Answers every release page as out; counts requests.
        #[derive(Default)]
        struct Released(Cell<usize>);
        #[async_trait(?Send)]
        impl Http for Released {
            async fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
                self.0.set(self.0.get() + 1);
                Ok(HttpResp {
                    status: 200,
                    body: "<title>Consumer Price Index News Release</title>".into(),
                    set_cookie: None,
                })
            }
            async fn post_json(
                &self,
                _: &str,
                _: &[(&str, &str)],
                _: &Value,
            ) -> Result<HttpResp, ProviderError> {
                Err(ProviderError::Http(405))
            }
        }

        // A CPI release at 8:30 ET on the brief's day, just past at 8:35.
        let cpi = crate::events::Planned {
            id: "bls:cpi:2026-09".into(),
            series: "cpi".into(),
            ref_period: Some("2026-09".into()),
            variant: None,
            starts_at: Some(utc("2026-10-29T12:30:00Z")),
            et_date: "2026-10-29".parse().unwrap(),
            tier: 1,
            proof_url: "https://www.bls.gov/schedule/news_release/cpi.htm".into(),
        };
        let ev = MemEventStore::default();
        ev.rows
            .borrow_mut()
            .insert(cpi.id.clone(), new_row(&cpi, utc("2026-10-29T10:00:00Z")));
        let es = MemEarningsStore::default();
        let http = Released::default();
        let watch = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        let hooks = Hooks {
            log: &|_| {},
            extra: &Vec::new,
            budget: &|| crate::budget::BUDGET_SOFT,
        };
        let out = tick_with_watch(
            utc("2026-10-29T12:35:00Z"),
            &s,
            &m,
            &FakeNs,
            &l,
            Some(&watch),
            &hooks,
        )
        .await;
        assert_eq!(out, TickOutcome::Published);
        // No release fetch at the limit: the event stays scheduled, linked to its schedule.
        assert_eq!(http.0.get(), 0);
        assert_eq!(ev.rows.borrow()["bls:cpi:2026-09"].status, "scheduled");
        let en = s.translations.borrow()[&(1, "en".to_string())]
            .body_md
            .clone();
        assert!(
            en.contains("- **8:30 AM ET** September CPI · [Schedule](https://www.bls.gov/schedule/news_release/cpi.htm)"),
            "{en}"
        );
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
            budget: &|| 0,
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
            // The facts line, then the cover outcome on its own line.
            let logs = logs.borrow();
            assert_eq!(logs.len(), 2, "{logs:?}");
            assert!(logs[0].starts_with("diagnostics: quotes ok=1 "), "{logs:?}");
            assert!(
                logs[0].contains("; movers liquid=1 gainers=1 losers=0"),
                "{logs:?}"
            );
            assert!(!logs[0].contains("cover="), "{logs:?}");
            assert_eq!(logs[1], "cover=ok");
        }
        // No build, no diagnostics line.
        tick_with_log(now, &s, &m, &FakeNs, &l, &log).await;
        assert_eq!(logs.borrow().len(), 2);
        // The cover outcome is not part of the stored facts.
        assert!(!s.facts.borrow()["premarket/2026-10-29.json"].contains("cover="));
    }

    /// An LLM that records what had been logged when the first draft was asked for.
    struct DraftSpy<'a> {
        inner: FakeLlm,
        logs: &'a std::cell::RefCell<Vec<String>>,
        at_draft: std::cell::RefCell<Option<Vec<String>>>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for DraftSpy<'_> {
        async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
            if system.starts_with("You write the US premarket") && self.at_draft.borrow().is_none()
            {
                *self.at_draft.borrow_mut() = Some(self.logs.borrow().clone());
            }
            self.inner.complete_json(system, user).await
        }
    }

    #[tokio::test]
    async fn diagnostics_are_logged_before_the_drafts_and_cover_after_the_write() {
        let s = MemStore::default();
        let m = md(true);
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let spy = DraftSpy {
            inner: llm(true, true),
            logs: &logs,
            at_draft: std::cell::RefCell::new(None),
        };
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let extra = || vec!["subrequests=7".to_string()];
        let hooks = Hooks {
            log: &log,
            extra: &extra,
            budget: &|| 0,
        };
        let out = tick_with_hooks(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &spy, &hooks).await;
        assert_eq!(out, TickOutcome::Published);
        let at_draft = spy.at_draft.borrow().clone().expect("no draft was written");
        assert_eq!(at_draft.len(), 1, "{at_draft:?}");
        assert!(at_draft[0].starts_with("diagnostics: "), "{at_draft:?}");
        assert!(at_draft[0].ends_with("; subrequests=7"), "{at_draft:?}");
        assert_eq!(
            *logs.borrow(),
            [at_draft[0].clone(), "cover=ok".to_string()]
        );
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
            utc("2026-10-29T11:55:00Z"),
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
        fail_put_bytes: Cell<bool>,
        fail_kv_set: Cell<bool>,
        get_run_calls: Cell<usize>,
        /// `(call number, status)`: on that `get_run` call, first set the
        /// stored run to `status`, as a concurrent tick finishing would.
        race_on_get_run: Cell<Option<(usize, &'static str)>>,
        /// Sets post 1's `revised_at` right before a revision-checked
        /// translation write, as the premarket update landing then would.
        revise_before_zh_write: Cell<Option<&'static str>>,
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
        async fn posts_on(&self, content_type: &str, date: &str) -> Result<Vec<PostRow>, String> {
            self.inner.posts_on(content_type, date).await
        }
        async fn upsert_post(&self, p: &PostRow) -> Result<i64, String> {
            self.inner.upsert_post(p).await
        }
        async fn revise_post(
            &self,
            p: &PostRow,
            t: &[TranslationRow],
            s: &[NewsReason],
        ) -> Result<(), String> {
            self.inner.revise_post(p, t, s).await
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
        async fn upsert_translation_at_revision(
            &self,
            t: &TranslationRow,
            revised_at: Option<&str>,
        ) -> Result<bool, String> {
            if let Some(at) = self.revise_before_zh_write.take() {
                if let Some(p) = self
                    .inner
                    .posts
                    .borrow_mut()
                    .iter_mut()
                    .find(|p| p.id == Some(1))
                {
                    p.revised_at = Some(at.into());
                }
            }
            if t.lang == "zh" && self.fail_zh_row.replace(false) {
                return Err("d1: zh row failed".into());
            }
            self.inner
                .upsert_translation_at_revision(t, revised_at)
                .await
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
        async fn put_bytes(
            &self,
            key: &str,
            bytes: &[u8],
            content_type: &str,
        ) -> Result<(), String> {
            if self.fail_put_bytes.get() {
                return Err("r2: put_bytes failed".into());
            }
            self.inner.put_bytes(key, bytes, content_type).await
        }
        async fn kv_set(&self, key: &str, value: &str, now: &str) -> Result<(), String> {
            if self.fail_kv_set.get() {
                return Err("d1: kv_set failed".into());
            }
            self.inner.kv_set(key, value, now).await
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
        let zh = s.inner.translations.borrow()[&(1, "zh".to_string())].clone();
        let want = stored_lines(&s.inner, "premarket/2026-10-29.json").zh;
        assert_eq!([zh.cover_line1.unwrap(), zh.cover_line2.unwrap()], want);
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
    async fn zh_retry_writes_nothing_once_the_post_was_revised() {
        // The update lands between the ZH draft and its write: neither the
        // published nor the 18:00 failed row overwrites it, and no alert.
        for (zh_ok, at) in [
            (true, "2026-10-29T12:35:00Z"),
            (false, "2026-10-29T22:05:00Z"),
        ] {
            let s = FailStore::default();
            let (m, l) = (md(true), llm(true, false));
            let out = tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await;
            assert_eq!(out, TickOutcome::PublishedEnOnly);
            let before = s.inner.translations.borrow()[&(1, "zh".to_string())].clone();
            l.zh_ok.set(zh_ok);
            s.revise_before_zh_write.set(Some("2026-10-29T12:40:00Z"));
            let calls = l.calls.get();
            assert_eq!(
                tick(utc(at), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing,
                "{at}"
            );
            assert!(l.calls.get() > calls, "{at}: the draft ran");
            assert_eq!(
                s.inner.translations.borrow()[&(1, "zh".to_string())],
                before,
                "{at}"
            );
            assert!(s.inner.alerts.borrow().is_empty(), "{at}");
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

    const NOV4: &str = "2026-11-04T13:30:00Z";
    const NOV4_KEY: &str = "premarket:2026-11-04";
    const NOV4_FACTS: &str = "premarket/2026-11-04.json";

    /// The cover lines of the facts stored under `key`.
    fn stored_lines(s: &MemStore, key: &str) -> CoverLines {
        let f: PremarketFacts = serde_json::from_str(&s.facts.borrow()[key]).unwrap();
        let (m, l) = (md(true), llm(true, true));
        let c = Premarket {
            md: &m,
            ns: &FakeNs,
            llm: &l,
            watch: None,
            reports: None,
        };
        c.cover_lines(&f)
    }
    fn lines_of(s: &MemStore, lang: &str) -> [Option<String>; 2] {
        let t = s.translations.borrow()[&(1, lang.to_string())].clone();
        [t.cover_line1, t.cover_line2]
    }

    #[tokio::test]
    async fn build_stores_cover_lines_graphic_and_index_snapshot() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let out = tick_with_log(utc(NOV4), &s, &m, &FakeNs, &l, &log).await;
        assert_eq!(out, TickOutcome::Published);

        let key = "covers/premarket-2026-11-04.svg";
        assert_eq!(s.posts.borrow()[0].cover_key.as_deref(), Some(key));
        let (bytes, mime) = s.bytes.borrow()[key].clone();
        assert_eq!(mime, "image/svg+xml");
        let svg = String::from_utf8(bytes).unwrap();
        assert!(svg.starts_with("<svg"), "{svg}");
        assert!(svg.contains(">AAPL<"), "{svg}");
        assert!(svg.contains(">+8.00%<"), "{svg}");

        let lines = stored_lines(&s, NOV4_FACTS);
        assert_eq!(lines.en[1], "AAPL +8.00%");
        assert_eq!(lines_of(&s, "en"), lines.en.map(Some));
        assert_eq!(lines_of(&s, "zh"), lines.zh.map(Some));

        let snap: IndexSnapshot =
            serde_json::from_str(&s.kv.borrow()[crate::cover::INDEX_LINE_KEY]).unwrap();
        assert_eq!(snap.kind, "premarket");
        assert_eq!(snap.as_of_utc, NOV4);
        assert_eq!(snap.entries.len(), 4);

        let logs = logs.borrow();
        assert_eq!(logs.len(), 2, "{logs:?}");
        assert!(logs[0].starts_with("diagnostics: "), "{logs:?}");
        assert_eq!(logs[1], "cover=ok");
    }

    #[tokio::test]
    async fn build_publishes_without_cover_when_the_write_fails() {
        let s = FailStore::default();
        s.fail_put_bytes.set(true);
        let (m, l) = (md(true), llm(true, true));
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let out = tick_with_log(utc(NOV4), &s, &m, &FakeNs, &l, &log).await;
        assert_eq!(out, TickOutcome::Published);
        assert_eq!(s.inner.runs.borrow()[NOV4_KEY].status, "published");
        assert_eq!(s.inner.posts.borrow()[0].cover_key, None);
        assert!(s.inner.bytes.borrow().is_empty());
        // The lines and the snapshot do not depend on the cover graphic.
        assert!(lines_of(&s.inner, "en")[1].is_some());
        assert!(s.inner.kv.borrow().contains_key("index_line"));
        // The write error reaches the log on its own line, never the run or the facts.
        assert_eq!(
            logs.borrow()[1..],
            ["cover=err", "cover write failed: r2: put_bytes failed"],
            "{:?}",
            logs.borrow()
        );
        assert_eq!(s.inner.runs.borrow()[NOV4_KEY].last_error, None);
        assert!(s
            .inner
            .facts
            .borrow()
            .values()
            .all(|j| !j.contains("put_bytes")));
    }

    #[tokio::test]
    async fn index_line_write_error_is_logged_and_the_build_publishes() {
        let s = FailStore::default();
        s.fail_kv_set.set(true);
        let (m, l) = (md(true), llm(true, true));
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let out = tick_with_log(utc(NOV4), &s, &m, &FakeNs, &l, &log).await;
        assert_eq!(out, TickOutcome::Published);
        assert!(
            logs.borrow()
                .iter()
                .any(|m| m.contains("d1: kv_set failed")),
            "{:?}",
            logs.borrow()
        );
    }

    #[tokio::test]
    async fn retry_zh_keeps_cover_lines() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        let now = utc("2026-10-29T12:30:00Z");
        assert_eq!(
            tick(now, &s, &m, &FakeNs, &l).await,
            TickOutcome::PublishedEnOnly
        );
        let lines = stored_lines(&s, "premarket/2026-10-29.json");
        // The pending row already carries the Chinese lines.
        assert_eq!(lines_of(&s, "zh"), lines.zh.clone().map(Some));
        l.zh_ok.set(true);
        assert_eq!(
            tick(utc("2026-10-29T12:35:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        assert_eq!(tr_status(&s, "zh"), "published");
        assert_eq!(lines_of(&s, "zh"), lines.zh.map(Some));
    }

    #[tokio::test]
    async fn failed_zh_keeps_cover_lines() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, false));
        tick(utc("2026-10-29T12:30:00Z"), &s, &m, &FakeNs, &l).await;
        tick(utc("2026-10-29T22:05:00Z"), &s, &m, &FakeNs, &l).await;
        assert_eq!(tr_status(&s, "zh"), "failed");
        let lines = stored_lines(&s, "premarket/2026-10-29.json");
        assert_eq!(lines_of(&s, "zh"), lines.zh.map(Some));
    }
}
