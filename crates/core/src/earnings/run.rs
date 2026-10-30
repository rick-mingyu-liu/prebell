//! The earnings run state machine. Every 5-minute firing in the polling
//! window runs `poll_job` (read EDGAR's current 8-K feed back to the last
//! poll and detect watchlist Item 2.02 filings), which asks for follow-up
//! messages: `build_tick` (write at
//! most one queued filing, or retry its Chinese draft) and the daily
//! calendar refresh, each in its own invocation with its own subrequest
//! budget. The run key `earnings:{TICKER}:{accession}` is the idempotency
//! key: re-running never creates a second post.

use chrono::{DateTime, Duration, SecondsFormat, Utc};

use crate::clock::et;
use crate::earnings::charts::{charts, r2_key};
use crate::earnings::detect::{domestic_by_cik, earnings_entries, name_mismatches};
use crate::earnings::draft::{slug, write, CONTENT_TYPE, TEMPLATE_ID};
use crate::earnings::extract::{extract, Extracted};
use crate::earnings::facts::{missing_required, EarningsFacts, Metrics};
use crate::earnings::store::{ChartRow, EarningsStore, FilingRow};
use crate::earnings::xbrl::fetch_trend;
use crate::earnings::{find, in_window, WatchEntry};
use crate::edgar::{exhibit_991, filing_url, FeedEntry, Filings, FEED_PAGE};
use crate::facts::{Field, NewsReason};
use crate::jobs::alert_once_today;
use crate::numbers::Lang;
use crate::providers::{LlmProvider, ProviderError};
use crate::run::TickOutcome;
use crate::schedule::EarningsStep;
use crate::store::{PostRow, RunRow, Store, TranslationRow};

/// A `building` run younger than this is in flight (a build takes 1-5 min).
const BUILD_LEASE_MINUTES: i64 = 15;
/// Pause between attempts of a run whose required data was missing.
const RETRY_MINUTES: i64 = 5;
/// Required figures may arrive late (exhibit amended, EDGAR lag) for this long.
const BUILD_DEADLINE_HOURS: i64 = 6;
/// A filing detected late still gets this long of retries.
const LATE_DETECTION_HOURS: i64 = 1;
/// Chinese retries stop this long after acceptance.
const ZH_DEADLINE_HOURS: i64 = 24;
/// A poll message older than one firing interval is dropped: after a
/// backlog the next firing reads the feed anyway.
pub const MAX_POLL_LAG_MINUTES: i64 = 5;
/// Feed pages one poll reads at most (300 filings, far more than EDGAR
/// accepts in 5 minutes even at the peak of earnings season).
pub const MAX_FEED_PAGES: usize = 3;
/// kv key: the newest feed acceptance time a poll read (RFC 3339).
pub const KV_FEED_WATERMARK: &str = "edgar_feed_watermark";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollOutcome {
    /// Feed entries read.
    pub checked: usize,
    /// Feed pages read.
    pub pages: usize,
    /// Run keys of newly detected filings.
    pub detected: Vec<String>,
    /// Problems: feed errors, a page-cap gap, watchlist name mismatches.
    pub errors: Vec<String>,
    /// SEC answered 403/429 (blocked or rate-limited) and the poll stopped.
    pub throttled: bool,
    /// The message was older than `MAX_POLL_LAG_MINUTES` and did nothing.
    pub stale: bool,
    /// Messages to queue next, each its own invocation.
    pub follow_ups: Vec<EarningsStep>,
}

pub fn run_key(ticker: &str, accession: &str) -> String {
    format!("earnings:{ticker}:{accession}")
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The filing row for a detected feed entry.
fn filing_row(f: &FeedEntry, w: &WatchEntry, now: DateTime<Utc>) -> FilingRow {
    FilingRow {
        accession: f.accession.clone(),
        cik: f.cik,
        ticker: w.ticker.to_string(),
        form: f.form.clone(),
        items: f.items.join(","),
        filing_date: f.filing_date.clone(),
        // The feed has no period of report; nothing reads it.
        report_date: f.filing_date.clone(),
        accepted_at: iso(f.accepted_at),
        primary_doc: String::new(),
        run_key: run_key(w.ticker, &f.accession),
        state: "open".into(),
        detected_at: iso(now),
    }
}

/// Reads EDGAR's current 8-K feed, newest first, back to the watermark (the
/// newest acceptance time the last poll read): further pages only while a
/// full page is still newer than it, at most `MAX_FEED_PAGES`. Without a
/// watermark one page is read. Watchlist Item 2.02 filings are recorded
/// (the accession is the idempotency key). The watermark advances when the
/// read reached it or hit the page cap (logged as a possible gap); after an
/// error (403/429 stops the poll) it stays, so the next poll re-reads the
/// range. A store error aborts the tick.
pub async fn poll_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    filings: &dyn Filings,
) -> Result<PollOutcome, String> {
    let mut out = PollOutcome::default();
    if !in_window(now) {
        return Ok(out);
    }
    let watermark = es
        .kv_get(KV_FEED_WATERMARK)
        .await?
        .and_then(|t| parse_iso(&t));
    let mut entries: Vec<FeedEntry> = Vec::new();
    let mut complete = false;
    for page in 0..MAX_FEED_PAGES {
        let list = match filings.current_filings(page * FEED_PAGE).await {
            Ok(l) => l,
            Err(e) => {
                out.errors.push(format!("current filings page {page}: {e}"));
                out.throttled = matches!(e, ProviderError::Http(403 | 429));
                break;
            }
        };
        out.pages += 1;
        out.checked += list.len();
        let full = list.len() >= FEED_PAGE;
        let oldest = list.iter().map(|f| f.accepted_at).min();
        entries.extend(list);
        let more = match (watermark, oldest) {
            (Some(w), Some(o)) => full && o > w,
            _ => false,
        };
        if !more {
            complete = true;
            break;
        }
        if page + 1 == MAX_FEED_PAGES {
            out.errors.push(format!(
                "current filings: {MAX_FEED_PAGES} pages did not reach the last poll; older filings may be missed"
            ));
            complete = true;
        }
    }
    let watch = domestic_by_cik();
    for (ticker, msg) in name_mismatches(&entries, &watch) {
        out.errors.push(msg.clone());
        alert_once_today(now, &format!("watchlist-cik:{ticker}"), "warn", &msg, store).await?;
    }
    for (f, w) in earnings_entries(&entries, &watch, now) {
        let row = filing_row(f, &w, now);
        let key = row.run_key.clone();
        // Calendar first and on every sighting (idempotent), so a failed
        // write after the insert cannot leave the calendar unmarked.
        crate::earnings::calendar::record_filing(es, &row, now).await?;
        if !es.insert_filing(&row).await? {
            continue;
        }
        if store.get_run(&key).await?.is_none() {
            store.upsert_run(&new_run(&row, now)).await?;
        }
        out.detected.push(key);
    }
    let newest = entries.iter().map(|f| f.accepted_at).max();
    if let (true, Some(n)) = (complete, newest) {
        if watermark.is_none_or(|w| n > w) {
            es.kv_set(KV_FEED_WATERMARK, &iso(n), &iso(now)).await?;
        }
    }
    Ok(out)
}

/// One `earnings` queue message: drops it when stale, else polls the feed,
/// alerts once per ET day when SEC refuses (403/429), and lists the
/// follow-up messages: one build per new filing (or one while any filing is
/// still open, for retries and Chinese drafts) and the calendar refresh
/// until it ran today.
pub async fn poll_job(
    now: DateTime<Utc>,
    fired: Option<DateTime<Utc>>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    filings: &dyn Filings,
) -> Result<PollOutcome, String> {
    let fired = fired.unwrap_or(now);
    if now - fired > Duration::minutes(MAX_POLL_LAG_MINUTES) {
        return Ok(PollOutcome {
            stale: true,
            ..Default::default()
        });
    }
    let mut out = poll_tick(now, es, store, filings).await?;
    if out.throttled {
        let msg = format!(
            "SEC EDGAR refused requests (403/429); polling stopped for this firing: {}",
            out.errors.last().cloned().unwrap_or_default()
        );
        alert_once_today(now, "sec-throttled", "error", &msg, store).await?;
    }
    if !in_window(now) {
        return Ok(out);
    }
    let builds = match out.detected.len() {
        0 => usize::from(!es.open_filings().await?.is_empty()),
        n => n,
    };
    out.follow_ups = vec![EarningsStep::Build; builds];
    if !crate::earnings::calendar::refreshed_today(es, now).await? {
        out.follow_ups.push(EarningsStep::Calendar);
    }
    Ok(out)
}

fn new_run(f: &FilingRow, now: DateTime<Utc>) -> RunRow {
    RunRow {
        run_key: f.run_key.clone(),
        content_type: CONTENT_TYPE.into(),
        trading_date: trading_date(f),
        status: "pending".into(),
        attempts: 0,
        last_error: None,
        created_at: iso(now),
        updated_at: iso(now),
    }
}

/// The ET date the filing was accepted (falls back to EDGAR's filing date).
fn trading_date(f: &FilingRow) -> String {
    parse_iso(&f.accepted_at)
        .map(|t| et(t).date_naive().format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| f.filing_date.clone())
}

fn age(f: &FilingRow, now: DateTime<Utc>) -> Duration {
    parse_iso(&f.accepted_at).map_or(Duration::zero(), |t| now - t)
}

/// `BUILD_DEADLINE_HOURS` after acceptance, but at least `LATE_DETECTION_HOURS`
/// after detection, so a filing found late (weekend, outage) still gets retries.
fn past_deadline(f: &FilingRow, now: DateTime<Utc>) -> bool {
    let detected = parse_iso(&f.detected_at).map_or(Duration::zero(), |t| now - t);
    age(f, now) >= Duration::hours(BUILD_DEADLINE_HOURS)
        && detected >= Duration::hours(LATE_DETECTION_HOURS)
}

fn since_update(r: &RunRow, now: DateTime<Utc>) -> Duration {
    parse_iso(&r.updated_at).map_or(Duration::MAX, |t| now - t)
}

/// Works through the open filings, oldest first: closes finished ones,
/// skips in-flight or recently retried ones, and does at most one piece of
/// LLM work (a build or a Chinese retry) per tick.
pub async fn build_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    filings: &dyn Filings,
    llm: &dyn LlmProvider,
) -> Result<Vec<(String, TickOutcome)>, String> {
    let mut out = Vec::new();
    // One build and one Chinese retry per tick, budgeted separately so a
    // Chinese draft that keeps failing never holds up new posts.
    let (mut built, mut zh_tried) = (false, false);
    for f in es.open_filings().await? {
        let run = match store.get_run(&f.run_key).await? {
            Some(r) => r,
            None => {
                let r = new_run(&f, now);
                store.upsert_run(&r).await?;
                r
            }
        };
        let outcome = match run.status.as_str() {
            "failed" | "skipped" => {
                es.set_filing_state(&f.accession, "done").await?;
                continue;
            }
            "published" if zh_tried => continue,
            "published" => match retry_zh(now, es, store, llm, &f, &run).await? {
                TickOutcome::Nothing => continue,
                o => {
                    zh_tried = true;
                    o
                }
            },
            "building" if since_update(&run, now) < Duration::minutes(BUILD_LEASE_MINUTES) => {
                continue
            }
            // A build that never finishes (the Worker killed mid-build) must
            // not be retried forever.
            "building" if past_deadline(&f, now) => {
                give_up(now, es, store, &f, run, "the build never finished".into()).await?
            }
            _ if built => continue,
            _ if run.attempts > 0
                && run.status == "pending"
                && since_update(&run, now) < Duration::minutes(RETRY_MINUTES) =>
            {
                continue
            }
            _ => {
                built = true;
                if let Some(o) = reconcile(now, es, store, &f, &run).await? {
                    o
                } else {
                    build(now, es, store, filings, llm, &f, run).await?
                }
            }
        };
        out.push((f.run_key.clone(), outcome));
    }
    Ok(out)
}

/// A store failure part-way through publishing can leave EN live with the
/// run not `published`; finish the bookkeeping instead of rebuilding.
async fn reconcile(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    f: &FilingRow,
    run: &RunRow,
) -> Result<Option<TickOutcome>, String> {
    let Some(post) = store.get_post_by_run(&f.run_key).await? else {
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
    // The failed write may have been sources or charts; both replace, so redo them.
    if let Some(json) = store.get_facts(&post.facts_r2_key).await? {
        let facts: EarningsFacts =
            serde_json::from_str(&json).map_err(|e| format!("facts json: {e}"))?;
        store.add_sources(post_id, &sources(&facts)).await?;
        let rows = store_charts(store, &facts, post_id).await?;
        es.set_charts(post_id, &rows).await?;
    }
    let ts = iso(now);
    let zh = store.get_translation(post_id, "zh").await?;
    if zh.is_none() {
        store
            .upsert_translation(&empty_translation(post_id, "zh", "pending", &ts))
            .await?;
    }
    let mut row = run.clone();
    row.status = "published".into();
    row.updated_at = ts;
    store.upsert_run(&row).await?;
    if zh.is_some_and(|t| t.status != "pending") {
        es.set_filing_state(&f.accession, "done").await?;
    }
    Ok(Some(TickOutcome::PublishedEnOnly))
}

fn empty_translation(post_id: i64, lang: &str, status: &str, ts: &str) -> TranslationRow {
    TranslationRow {
        post_id,
        lang: lang.into(),
        status: status.into(),
        title: String::new(),
        summary: String::new(),
        body_md: String::new(),
        updated_at: ts.into(),
    }
}

/// Everything the post is written from. Provider failures become `Missing`
/// fields; the gate decides whether that blocks publishing.
async fn gather(
    filings: &dyn Filings,
    llm: &dyn LlmProvider,
    f: &FilingRow,
) -> Result<EarningsFacts, String> {
    let company = find(&f.ticker)
        .map_or(f.ticker.as_str(), |e| e.name)
        .to_string();
    let docs = filings.filing_docs(f.cik, &f.accession).await;
    let exhibit = match &docs {
        Ok(d) => exhibit_991(d)
            .map(|d| d.url.clone())
            .ok_or_else(|| "no EX-99 exhibit in the filing".to_string()),
        Err(e) => Err(format!("filing index: {e}")),
    };
    let text = match &exhibit {
        Ok(url) => filings
            .document_text(url)
            .await
            .map_err(|e| format!("exhibit: {e}")),
        Err(e) => Err(e.clone()),
    };
    let x = match &text {
        Ok(t) => Some(extract(llm, t, &company, &f.ticker).await),
        Err(_) => None,
    };
    let accepted_at = parse_iso(&f.accepted_at).ok_or("bad accepted_at")?;
    let filing_date = f
        .filing_date
        .parse()
        .map_err(|e| format!("bad filing_date: {e}"))?;
    let trend = match fetch_trend(filings, f.cik, filing_date).await {
        Ok(t) => Field::Value(t),
        Err(e) => Field::Missing {
            reason: format!("xbrl: {e}"),
        },
    };
    let x = x.unwrap_or_else(|| {
        let reason = text.err().unwrap_or_default();
        Extracted {
            fiscal_period: Field::Missing {
                reason: reason.clone(),
            },
            metrics: Metrics::all_missing(&reason),
            guidance: Field::Missing {
                reason: reason.clone(),
            },
            highlights: Field::Missing { reason },
        }
    });
    Ok(EarningsFacts {
        ticker: f.ticker.clone(),
        company,
        cik: f.cik,
        accession: f.accession.clone(),
        filing_date,
        accepted_at,
        filing_url: filing_url(f.cik, &f.accession),
        exhibit_url: match exhibit {
            Ok(u) => Field::Value(u),
            Err(reason) => Field::Missing { reason },
        },
        fiscal_period: x.fiscal_period,
        metrics: x.metrics,
        guidance: x.guidance,
        highlights: x.highlights,
        trend,
    })
}

/// Pending again, or `failed` with one alert once the build deadline passed.
async fn not_ready(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    f: &FilingRow,
    mut row: RunRow,
    err: String,
) -> Result<TickOutcome, String> {
    if superseded(store, &row.run_key).await? {
        return Ok(TickOutcome::Nothing);
    }
    if past_deadline(f, now) {
        return give_up(now, es, store, f, row, err).await;
    }
    row.last_error = Some(err.clone());
    row.updated_at = iso(now);
    row.status = "pending".into();
    store.upsert_run(&row).await?;
    Ok(TickOutcome::Retrying(err))
}

/// The run is `failed` with one error alert and the filing is closed.
async fn give_up(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    f: &FilingRow,
    mut row: RunRow,
    err: String,
) -> Result<TickOutcome, String> {
    row.status = "failed".into();
    row.last_error = Some(err.clone());
    row.updated_at = iso(now);
    store.upsert_run(&row).await?;
    store
        .add_alert(
            &row.run_key,
            "error",
            &format!(
                "earnings check for {} ({}) not published: {err}",
                f.ticker, f.accession
            ),
        )
        .await?;
    es.set_filing_state(&f.accession, "done").await?;
    Ok(TickOutcome::Skipped)
}

async fn superseded(store: &dyn Store, run_key: &str) -> Result<bool, String> {
    Ok(store
        .get_run(run_key)
        .await?
        .is_some_and(|r| r.status == "published" || r.status == "failed"))
}

/// Draws the charts, stores each SVG in R2 and returns their rows.
async fn store_charts(
    store: &dyn Store,
    facts: &EarningsFacts,
    post_id: i64,
) -> Result<Vec<ChartRow>, String> {
    let mut rows = Vec::new();
    for (i, c) in charts(facts).iter().enumerate() {
        let key = r2_key(facts, c.name);
        store.put_facts(&key, &c.svg).await?;
        rows.push(ChartRow {
            post_id,
            name: c.name.into(),
            r2_key: key,
            alt_en: c.alt_en.clone(),
            alt_zh: c.alt_zh.clone(),
            sort: i as i64,
        });
    }
    Ok(rows)
}

fn sources(f: &EarningsFacts) -> Vec<NewsReason> {
    let src = |url: &str, title: &str| NewsReason {
        url: url.into(),
        title: title.into(),
        quote: String::new(),
        summary_en: String::new(),
    };
    let mut out = vec![src(&f.filing_url, &format!("{} Form 8-K", f.ticker))];
    if let Field::Value(u) = &f.exhibit_url {
        out.push(src(
            u,
            &format!("{} press release (Exhibit 99.1)", f.ticker),
        ));
    }
    out
}

async fn build(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    filings: &dyn Filings,
    llm: &dyn LlmProvider,
    f: &FilingRow,
    mut row: RunRow,
) -> Result<TickOutcome, String> {
    let ts = iso(now);
    row.status = "building".into();
    row.attempts += 1;
    row.updated_at = ts.clone();
    store.upsert_run(&row).await?;

    let facts = gather(filings, llm, f).await?;
    let missing = missing_required(&facts);
    if !missing.is_empty() {
        return not_ready(
            now,
            es,
            store,
            f,
            row,
            format!("missing {}", missing.join(", ")),
        )
        .await;
    }
    let (en, en_md) = match write(llm, &facts, Lang::En).await {
        Ok(x) => x,
        Err(e) => return not_ready(now, es, store, f, row, format!("en: {}", e.join("; "))).await,
    };
    let zh = write(llm, &facts, Lang::Zh).await;

    if superseded(store, &row.run_key).await? {
        return Ok(TickOutcome::Nothing);
    }
    let facts_key = format!(
        "earnings/{}/{}.json",
        facts.ticker.replace('.', "-"),
        facts.accession
    );
    let json = serde_json::to_string(&facts).map_err(|e| format!("facts json: {e}"))?;
    store.put_facts(&facts_key, &json).await?;
    let mut chart_rows = store_charts(store, &facts, 0).await?;

    let mut post_slug = slug(&facts);
    if es
        .slug_owner(&post_slug)
        .await?
        .is_some_and(|owner| owner != row.run_key)
    {
        let acc = &facts.accession;
        post_slug = format!("{post_slug}-{}", &acc[acc.len().saturating_sub(6)..]);
    }
    let post_id = store
        .upsert_post(&PostRow {
            id: None,
            run_key: row.run_key.clone(),
            content_type: CONTENT_TYPE.into(),
            template_id: TEMPLATE_ID.into(),
            trading_date: row.trading_date.clone(),
            slug: post_slug,
            facts_r2_key: facts_key,
            published_at: Some(ts.clone()),
        })
        .await?;
    for c in &mut chart_rows {
        c.post_id = post_id;
    }
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
    let zh_err = match zh {
        Ok((d, body)) => {
            store
                .upsert_translation(&TranslationRow {
                    post_id,
                    lang: "zh".into(),
                    status: "published".into(),
                    title: d.title,
                    summary: d.summary,
                    body_md: body,
                    updated_at: ts.clone(),
                })
                .await?;
            None
        }
        Err(e) => {
            store
                .upsert_translation(&empty_translation(post_id, "zh", "pending", &ts))
                .await?;
            Some(format!("zh: {}", e.join("; ")))
        }
    };
    store.add_sources(post_id, &sources(&facts)).await?;
    es.set_charts(post_id, &chart_rows).await?;

    row.status = "published".into();
    row.last_error = zh_err.clone();
    row.updated_at = ts;
    store.upsert_run(&row).await?;
    Ok(match zh_err {
        Some(_) => TickOutcome::PublishedEnOnly,
        None => {
            es.set_filing_state(&f.accession, "done").await?;
            TickOutcome::Published
        }
    })
}

/// After EN published: retry a pending Chinese draft until
/// `ZH_DEADLINE_HOURS` after acceptance, then mark it `failed` with one warn
/// alert. The filing is done once ZH is final.
async fn retry_zh(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    llm: &dyn LlmProvider,
    f: &FilingRow,
    run: &RunRow,
) -> Result<TickOutcome, String> {
    let Some(post) = store.get_post_by_run(&run.run_key).await? else {
        return zh_failed(
            now,
            es,
            store,
            f,
            run,
            None,
            "published run has no post".into(),
        )
        .await;
    };
    let post_id = post.id.ok_or("post has no id")?;
    let zh = store.get_translation(post_id, "zh").await?;
    match &zh {
        Some(t) if t.status != "pending" => {
            es.set_filing_state(&f.accession, "done").await?;
            return Ok(TickOutcome::Nothing);
        }
        Some(t)
            if parse_iso(&t.updated_at)
                .is_some_and(|u| now - u < Duration::minutes(RETRY_MINUTES)) =>
        {
            return Ok(TickOutcome::Nothing);
        }
        _ => {}
    }
    // Missing or unreadable facts cannot be fixed by retrying.
    let facts: EarningsFacts = match store.get_facts(&post.facts_r2_key).await? {
        None => {
            let err = "facts not found".to_string();
            return zh_failed(now, es, store, f, run, Some(post_id), err).await;
        }
        Some(json) => match serde_json::from_str(&json) {
            Ok(x) => x,
            Err(e) => {
                let err = format!("facts json: {e}");
                return zh_failed(now, es, store, f, run, Some(post_id), err).await;
            }
        },
    };
    let ts = iso(now);
    match write(llm, &facts, Lang::Zh).await {
        Ok((d, body)) => {
            store
                .upsert_translation(&TranslationRow {
                    post_id,
                    lang: "zh".into(),
                    status: "published".into(),
                    title: d.title,
                    summary: d.summary,
                    body_md: body,
                    updated_at: ts,
                })
                .await?;
            es.set_filing_state(&f.accession, "done").await?;
            Ok(TickOutcome::Published)
        }
        Err(e) => {
            let err = format!("zh: {}", e.join("; "));
            if age(f, now) >= Duration::hours(ZH_DEADLINE_HOURS) {
                return zh_failed(now, es, store, f, run, Some(post_id), err).await;
            }
            // Restarts the retry pause.
            store
                .upsert_translation(&empty_translation(post_id, "zh", "pending", &ts))
                .await?;
            Ok(TickOutcome::Retrying(err))
        }
    }
}

/// Chinese is final as `failed`, with one warn alert; the filing is closed.
async fn zh_failed(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    f: &FilingRow,
    run: &RunRow,
    post_id: Option<i64>,
    err: String,
) -> Result<TickOutcome, String> {
    if let Some(id) = post_id {
        store
            .upsert_translation(&empty_translation(id, "zh", "failed", &iso(now)))
            .await?;
    }
    store
        .add_alert(
            &run.run_key,
            "warn",
            &format!(
                "Chinese earnings check failed for {} ({}): {err}",
                f.ticker, f.accession
            ),
        )
        .await?;
    es.set_filing_state(&f.accession, "done").await?;
    Ok(TickOutcome::Skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::draft::tests::reply;
    use crate::earnings::store::MemEarningsStore;
    use crate::earnings::xbrl::tests::aapl_concept;
    use crate::edgar::tests::{EX991, FEED, INDEX};
    use crate::edgar::{html_to_text, parse_current_feed, parse_filing_index, Concept, FilingDoc};
    use crate::store::MemStore;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    const EXTRACT_REPLY: &str = include_str!("../../tests/fixtures/llm_earnings_extract_AAPL.json");
    const KEY: &str = "earnings:AAPL:0000320193-26-000070";
    const ACC: &str = "0000320193-26-000070";

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// Serves feed pages by start offset (the fixture at 0 unless set;
    /// later pages empty) and the AAPL filing documents; records each page
    /// requested.
    #[derive(Default)]
    struct FakeFilings {
        pages: RefCell<HashMap<usize, Result<String, ProviderError>>>,
        page_calls: RefCell<Vec<usize>>,
        no_exhibit: Cell<bool>,
    }
    #[async_trait(?Send)]
    impl Filings for FakeFilings {
        async fn current_filings(&self, start: usize) -> Result<Vec<FeedEntry>, ProviderError> {
            self.page_calls.borrow_mut().push(start);
            let body = match self.pages.borrow().get(&start).cloned() {
                Some(r) => r?,
                None if start == 0 => FEED.to_string(),
                None => feed(&[]),
            };
            parse_current_feed(&body)
        }
        async fn filing_docs(&self, _: u64, _: &str) -> Result<Vec<FilingDoc>, ProviderError> {
            if self.no_exhibit.get() {
                return Ok(vec![]);
            }
            Ok(parse_filing_index(INDEX))
        }
        async fn document_text(&self, _: &str) -> Result<String, ProviderError> {
            Ok(html_to_text(EX991))
        }
        async fn company_concept(
            &self,
            _: u64,
            tag: &str,
        ) -> Result<Option<Concept>, ProviderError> {
            Ok(aapl_concept(tag))
        }
    }

    /// An Atom page in the feed's shape: `(cik, accession, accepted, items)`.
    fn feed(entries: &[(u64, String, DateTime<Utc>, &str)]) -> String {
        let mut out = String::from("<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><title>Latest Filings</title>\n");
        for (cik, acc, at, items) in entries {
            let nodash = acc.replace('-', "");
            let lines: String = items
                .split(',')
                .filter(|i| !i.is_empty())
                .map(|i| format!("&lt;br&gt;Item {i}: Something\n"))
                .collect();
            out.push_str(&format!(
                "<entry>\n<title>8-K - {} (00{cik:08}) (Filer)</title>\n<link rel=\"alternate\" type=\"text/html\" href=\"https://www.sec.gov/Archives/edgar/data/{cik}/{nodash}/{acc}-index.htm\"/>\n<summary type=\"html\">\n &lt;b&gt;Filed:&lt;/b&gt; {} &lt;b&gt;AccNo:&lt;/b&gt; {acc}\n{lines}</summary>\n<updated>{}</updated>\n<id>urn:tag:sec.gov,2008:accession-number={acc}</id>\n</entry>\n",
                crate::earnings::by_cik(*cik).map_or("Co", |w| w.name),
                et(*at).format("%Y-%m-%d"),
                at.to_rfc3339()
            ));
        }
        out.push_str("</feed>");
        out
    }

    /// A full page of non-watchlist filings, one a minute back from `newest`.
    fn full_page(newest: DateTime<Utc>, tag: usize) -> String {
        let e: Vec<_> = (0..FEED_PAGE)
            .map(|i| {
                (
                    1_u64,
                    format!("0000000001-26-{tag}{i:05}"),
                    newest - Duration::minutes(i as i64),
                    "8.01",
                )
            })
            .collect();
        feed(&e)
    }

    struct FakeLlm {
        en_ok: Cell<bool>,
        zh_ok: Cell<bool>,
        drafts: Cell<usize>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, system: &str, _user: &str) -> Result<Value, ProviderError> {
            if system.starts_with("You extract reported figures") {
                return Ok(serde_json::from_str(EXTRACT_REPLY).unwrap());
            }
            self.drafts.set(self.drafts.get() + 1);
            let (lang, ok) = if system.starts_with("你负责撰写财报速评") {
                (Lang::Zh, self.zh_ok.get())
            } else {
                (Lang::En, self.en_ok.get())
            };
            // 13% is not in the facts.
            let body = match (lang, ok) {
                (_, false) => "Services grew 13%.",
                (Lang::En, true) => "Revenue was $102.5 billion.",
                (Lang::Zh, true) => "营收1025亿美元。",
            };
            Ok(reply(lang, body))
        }
    }
    fn llm(en_ok: bool, zh_ok: bool) -> FakeLlm {
        FakeLlm {
            en_ok: Cell::new(en_ok),
            zh_ok: Cell::new(zh_ok),
            drafts: Cell::new(0),
        }
    }

    fn stores() -> (Rc<MemStore>, MemEarningsStore) {
        let base = Rc::new(MemStore::default());
        let es = MemEarningsStore::new(base.clone());
        (base, es)
    }

    /// 16:45 ET on Fri 2026-10-30, after the fixture's AAPL filing.
    fn poll_time() -> DateTime<Utc> {
        utc("2026-10-30T20:45:00Z")
    }

    #[tokio::test]
    async fn poll_detects_once() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let t = poll_time();
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert_eq!(o.detected, vec![KEY.to_string()]);
        // No watermark yet: one page.
        assert_eq!((o.pages, o.checked), (1, 4));
        assert_eq!(base.runs.borrow()[KEY].status, "pending");
        assert_eq!(base.runs.borrow()[KEY].trading_date, "2026-10-30");
        let f = es.filings.borrow()[ACC].clone();
        assert_eq!((f.ticker.as_str(), f.items.as_str()), ("AAPL", "2.02,9.01"));
        assert_eq!(f.accepted_at, "2026-10-30T20:30:41Z");
        // The watermark is the newest acceptance read.
        assert_eq!(
            es.kv_get(KV_FEED_WATERMARK).await.unwrap().as_deref(),
            Some("2026-10-30T20:30:41Z")
        );
        // Next firing: the same accession again, nothing new.
        let o = poll_tick(t + Duration::minutes(5), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(o.detected.is_empty());
        assert_eq!(es.filings.borrow().len(), 1);
        assert_eq!(base.runs.borrow().len(), 1);
        // The calendar now shows the report as confirmed by EDGAR.
        let cal = es.calendar_for("AAPL").await.unwrap();
        assert_eq!(cal.len(), 1);
        assert_eq!(
            (cal[0].status.as_str(), cal[0].source.as_str()),
            ("reported", "edgar")
        );
    }

    #[tokio::test]
    async fn an_entry_on_two_pages_is_one_filing() {
        // Entries shift between pages while EDGAR adds filings.
        let (base, es) = stores();
        let t = poll_time();
        es.kv_set(KV_FEED_WATERMARK, "2026-10-30T10:00:00Z", "x")
            .await
            .unwrap();
        let fl = FakeFilings::default();
        let aapl = (
            320193_u64,
            ACC.to_string(),
            utc("2026-10-30T20:30:41Z"),
            "2.02,9.01",
        );
        let mut first: Vec<_> = (0..FEED_PAGE - 1)
            .map(|i| {
                (
                    1_u64,
                    format!("0000000001-26-9{i:05}"),
                    t - Duration::seconds(i as i64),
                    "8.01",
                )
            })
            .collect();
        first.push(aapl.clone());
        fl.pages.borrow_mut().insert(0, Ok(feed(&first)));
        fl.pages.borrow_mut().insert(FEED_PAGE, Ok(feed(&[aapl])));
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert_eq!(o.pages, 2);
        assert_eq!(o.detected, [KEY]);
        assert_eq!(es.filings.borrow().len(), 1);
    }

    #[tokio::test]
    async fn pages_back_to_the_watermark_and_at_most_three() {
        let t = poll_time();
        // The watermark is inside page 2: pages 1 and 2 are read.
        let (base, es) = stores();
        es.kv_set(KV_FEED_WATERMARK, &iso(t - Duration::minutes(150)), "x")
            .await
            .unwrap();
        let fl = FakeFilings::default();
        fl.pages.borrow_mut().insert(0, Ok(full_page(t, 1)));
        fl.pages
            .borrow_mut()
            .insert(FEED_PAGE, Ok(full_page(t - Duration::minutes(100), 2)));
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert_eq!(*fl.page_calls.borrow(), [0, FEED_PAGE]);
        assert_eq!((o.pages, o.checked), (2, 2 * FEED_PAGE));
        assert!(o.errors.is_empty(), "{:?}", o.errors);
        assert_eq!(es.kv_get(KV_FEED_WATERMARK).await.unwrap(), Some(iso(t)));
        // A watermark older than 3 full pages: 3 pages, then a logged gap;
        // the watermark still moves to the newest filing read.
        let (base, es) = stores();
        es.kv_set(KV_FEED_WATERMARK, &iso(t - Duration::hours(12)), "x")
            .await
            .unwrap();
        let fl = FakeFilings::default();
        for p in 0..4 {
            fl.pages.borrow_mut().insert(
                p * FEED_PAGE,
                Ok(full_page(t - Duration::minutes(100 * p as i64), p)),
            );
        }
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert_eq!(*fl.page_calls.borrow(), [0, FEED_PAGE, 2 * FEED_PAGE]);
        assert!(o.errors[0].contains("may be missed"), "{:?}", o.errors);
        assert_eq!(es.kv_get(KV_FEED_WATERMARK).await.unwrap(), Some(iso(t)));
    }

    #[tokio::test]
    async fn throttling_stops_the_poll_keeps_the_watermark_and_alerts_once_a_day() {
        let (base, es) = stores();
        let t = poll_time();
        let wm = iso(t - Duration::hours(2));
        es.kv_set(KV_FEED_WATERMARK, &wm, "x").await.unwrap();
        let fl = FakeFilings::default();
        fl.pages.borrow_mut().insert(0, Ok(full_page(t, 1)));
        fl.pages
            .borrow_mut()
            .insert(FEED_PAGE, Err(ProviderError::Http(403)));
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(o.throttled);
        assert_eq!(
            *fl.page_calls.borrow(),
            [0, FEED_PAGE],
            "no request after the 403"
        );
        // The range is re-read next time.
        assert_eq!(es.kv_get(KV_FEED_WATERMARK).await.unwrap(), Some(wm));
        fl.pages
            .borrow_mut()
            .insert(0, Err(ProviderError::Http(429)));
        let later = t + Duration::minutes(5);
        let o = poll_job(later, Some(later), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(o.throttled);
        let alerts = base.alerts.borrow();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert_eq!(alerts[0].0, "sec-throttled:2026-10-30");
        assert_eq!(alerts[0].1, "error");
    }

    #[tokio::test]
    async fn a_filer_name_mismatch_is_skipped_and_alerted_once_a_day() {
        // The feed says CIK 320193 is someone else: a wrong CIK in the data
        // file must not publish that company's results as Apple's.
        let (base, es) = stores();
        let fl = FakeFilings::default();
        fl.pages.borrow_mut().insert(
            0,
            Ok(FEED.replace(
                "8-K - Apple Inc. (0000320193)",
                "8-K - Estrella Immunopharma, Inc. (0000320193)",
            )),
        );
        let t = poll_time();
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert!(o.detected.is_empty());
        assert!(
            o.errors[0].contains("AAPL") && o.errors[0].contains("Estrella"),
            "{:?}",
            o.errors
        );
        poll_tick(t + Duration::minutes(5), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        let alerts = base.alerts.borrow();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert_eq!(alerts[0].0, "watchlist-cik:AAPL:2026-10-30");
        assert_eq!(alerts[0].1, "warn");
    }

    #[tokio::test]
    async fn a_filing_after_8pm_et_keeps_its_et_date() {
        // 20:45 ET on Fri 2026-10-30 is 00:45Z on Saturday.
        let (base, es) = stores();
        let mut f = parse_current_feed(FEED).unwrap()[0].clone();
        f.accepted_at = utc("2026-10-31T00:45:00Z");
        let row = filing_row(&f, &find("AAPL").unwrap(), utc("2026-10-31T00:50:00Z"));
        assert_eq!(trading_date(&row), "2026-10-30");
        crate::earnings::calendar::record_filing(&es, &row, utc("2026-10-31T00:50:00Z"))
            .await
            .unwrap();
        let cal = es.calendar_for("AAPL").await.unwrap();
        assert_eq!(cal[0].report_date, "2026-10-30");
        assert!(base.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn deadlines_count_from_the_utc_acceptance() {
        // Accepted 20:30:41Z (16:30 ET); the 6 h build deadline is 02:30:41Z.
        let (base, es, fl) = detected().await;
        fl.no_exhibit.set(true);
        let l = llm(true, true);
        let o = build_tick(utc("2026-10-31T02:25:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(matches!(o[0].1, TickOutcome::Retrying(_)), "{o:?}");
        let o = build_tick(utc("2026-10-31T02:31:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped, "{o:?}");
    }

    #[tokio::test]
    async fn a_poll_reads_the_feed_and_queues_one_build_per_filing() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let t = poll_time();
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(fl.page_calls.borrow().len() <= MAX_FEED_PAGES);
        assert_eq!(o.detected, vec![KEY.to_string()]);
        // One build per new filing, plus the day's calendar refresh.
        assert_eq!(o.follow_ups, [EarningsStep::Build, EarningsStep::Calendar]);
        assert!(!o.stale);
        // Next firing: nothing new, but the open filing still gets a build
        // message (retries and the Chinese draft).
        es.kv_set("calendar_refreshed_on", "2026-10-30", "x")
            .await
            .unwrap();
        let t2 = t + Duration::minutes(15);
        let o = poll_job(t2, Some(t2), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(o.detected.is_empty());
        assert_eq!(o.follow_ups, [EarningsStep::Build]);
        // Nothing open, calendar done: no follow-ups.
        es.set_filing_state(ACC, "done").await.unwrap();
        let o = poll_job(t2, Some(t2), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(o.follow_ups.is_empty(), "{o:?}");
    }

    #[tokio::test]
    async fn a_stale_poll_message_is_dropped() {
        // After a backlog the message is older than one firing interval: the
        // next firing reads the feed anyway, so this one does nothing.
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let fired = poll_time();
        let now = fired + Duration::minutes(MAX_POLL_LAG_MINUTES) + Duration::seconds(1);
        let o = poll_job(now, Some(fired), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(o.stale);
        assert!(fl.page_calls.borrow().is_empty());
        assert!(o.follow_ups.is_empty());
        // Within the lag it polls.
        let now = fired + Duration::minutes(MAX_POLL_LAG_MINUTES);
        let o = poll_job(now, Some(fired), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(!o.stale);
        assert_eq!(o.detected.len(), 1);
    }

    /// Counts every LLM call (each is one fetch subrequest).
    struct CountingLlm<'a>(&'a FakeLlm, Cell<usize>);
    #[async_trait(?Send)]
    impl LlmProvider for CountingLlm<'_> {
        async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
            self.1.set(self.1.get() + 1);
            self.0.complete_json(system, user).await
        }
    }

    /// A build's fetch subrequests through the real `Edgar` client: the
    /// filing index, the exhibit, the XBRL concepts, the extraction and
    /// every draft attempt (Chinese failing both, the worst case), plus a
    /// Chinese retry of another filing in the same invocation, stay under 40
    /// of the Free plan's 50.
    #[tokio::test]
    async fn a_build_stays_under_40_subrequests() {
        use crate::earnings::xbrl::tests::{EPS, NET_INCOME, REVENUE};
        use crate::edgar::tests::{FakeHttp, RecTimer};
        use crate::edgar::{concept_url, Edgar};
        use crate::providers::CountingHttp;
        let (base, es, _) = detected().await;
        let mut http = FakeHttp::default();
        let mut route = |u: String, body: &str| {
            http.routes.insert(u, (200, body.to_string()));
        };
        route(filing_url(320193, ACC), INDEX);
        let ex = exhibit_991(&parse_filing_index(INDEX)).unwrap().url.clone();
        route(ex, EX991);
        route(
            concept_url(
                320193,
                "RevenueFromContractWithCustomerExcludingAssessedTax",
            ),
            REVENUE,
        );
        route(concept_url(320193, "EarningsPerShareDiluted"), EPS);
        route(concept_url(320193, "NetIncomeLoss"), NET_INCOME);
        let edgar = Edgar::new(
            CountingHttp::new(http),
            RecTimer::default(),
            "ops@prebell.ai",
        )
        .unwrap();
        let inner = llm(true, false);
        let l = CountingLlm(&inner, Cell::new(0));
        let o = build_tick(poll_time(), &es, base.as_ref(), &edgar, &l)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::PublishedEnOnly, "{o:?}");
        let fetches = edgar.http.count();
        let used = fetches + l.1.get();
        // index + exhibit + 4 concepts (no `Revenues`, then the current tag,
        // EPS, net income) = 6 fetches; extraction + EN + 2 ZH attempts = 4.
        assert_eq!((fetches, l.1.get()), (6, 4));
        // A Chinese retry for another filing in the same invocation adds 2.
        assert!(used + 2 <= 40, "{used}");
    }

    #[tokio::test]
    async fn poll_outside_window_does_nothing() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let o = poll_tick(utc("2026-10-31T15:00:00Z"), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert_eq!(o, PollOutcome::default());
        assert!(fl.page_calls.borrow().is_empty());
    }

    async fn detected() -> (Rc<MemStore>, MemEarningsStore, FakeFilings) {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        poll_tick(poll_time(), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        (base, es, fl)
    }

    #[tokio::test]
    async fn full_build_publishes_one_post() {
        let (base, es, fl) = detected().await;
        let l = llm(true, true);
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l).await.unwrap();
        assert_eq!(o, vec![(KEY.to_string(), TickOutcome::Published)]);
        let posts = base.posts.borrow().clone();
        assert_eq!(posts.len(), 1);
        let p = &posts[0];
        assert_eq!(p.slug, "aapl-q4-fy2026-earnings");
        assert_eq!(p.template_id, "earnings:v1");
        assert_eq!(p.content_type, "earnings");
        assert_eq!(p.trading_date, "2026-10-30");
        let id = p.id.unwrap();
        assert_eq!(
            base.translations.borrow()[&(id, "en".to_string())].status,
            "published"
        );
        assert_eq!(
            base.translations.borrow()[&(id, "zh".to_string())].status,
            "published"
        );
        let src: Vec<String> = base.sources.borrow()[&id]
            .iter()
            .map(|s| s.url.clone())
            .collect();
        assert_eq!(src.len(), 2);
        assert!(src[0].ends_with("0000320193-26-000070-index.htm"));
        assert!(src[1].ends_with("a8-kex991q4202609262026.htm"));
        let charts = es.charts.borrow()[&id].clone();
        assert_eq!(
            charts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["revenue", "eps"]
        );
        assert!(base.facts.borrow()[&charts[0].r2_key].starts_with("<svg"));
        assert!(base
            .facts
            .borrow()
            .contains_key("earnings/AAPL/0000320193-26-000070.json"));
        assert_eq!(base.runs.borrow()[KEY].status, "published");
        assert_eq!(es.filings.borrow()[ACC].state, "done");
        // Nothing left to do.
        let o = build_tick(t + Duration::minutes(1), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(o.is_empty());
    }

    #[tokio::test]
    async fn rebuild_reuses_the_post() {
        let (base, es, fl) = detected().await;
        let l = llm(true, true);
        let t = poll_time() + Duration::minutes(1);
        build_tick(t, &es, base.as_ref(), &fl, &l).await.unwrap();
        // An operator re-runs the filing.
        es.set_filing_state(ACC, "open").await.unwrap();
        let mut r = base.runs.borrow()[KEY].clone();
        r.status = "pending".into();
        base.runs.borrow_mut().insert(KEY.into(), r);
        base.translations.borrow_mut().clear();
        build_tick(t + Duration::minutes(10), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(base.posts.borrow().len(), 1);
        assert_eq!(base.posts.borrow()[0].slug, "aapl-q4-fy2026-earnings");
    }

    #[tokio::test]
    async fn missing_exhibit_retries_then_fails_after_deadline() {
        let (base, es, fl) = detected().await;
        fl.no_exhibit.set(true);
        let l = llm(true, true);
        // Accepted 20:30:41Z.
        let o = build_tick(utc("2026-10-31T01:30:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(matches!(&o[0].1, TickOutcome::Retrying(e) if e.contains("revenue")));
        assert_eq!(base.runs.borrow()[KEY].status, "pending");
        // Retried no sooner than 5 minutes later.
        let o = build_tick(utc("2026-10-31T01:33:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(o.is_empty());
        let o = build_tick(utc("2026-10-31T02:31:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(base.runs.borrow()[KEY].status, "failed");
        assert_eq!(base.alerts.borrow().len(), 1);
        assert_eq!(base.alerts.borrow()[0].1, "error");
        assert!(base.posts.borrow().is_empty());
        assert_eq!(es.filings.borrow()[ACC].state, "done");
    }

    #[tokio::test]
    async fn en_first_then_zh_fills_in() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l).await.unwrap();
        assert_eq!(o[0].1, TickOutcome::PublishedEnOnly);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "pending"
        );
        assert_eq!(es.filings.borrow()[ACC].state, "open");
        l.zh_ok.set(true);
        let o = build_tick(t + Duration::minutes(5), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::Published);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "published"
        );
        assert_eq!(es.filings.borrow()[ACC].state, "done");
    }

    #[tokio::test]
    async fn zh_gives_up_after_a_day() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        build_tick(poll_time(), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        let o = build_tick(utc("2026-10-31T20:31:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "failed"
        );
        assert_eq!(base.alerts.borrow()[0].1, "warn");
        assert_eq!(es.filings.borrow()[ACC].state, "done");
    }

    #[tokio::test]
    async fn en_failure_publishes_nothing() {
        let (base, es, fl) = detected().await;
        let l = llm(false, true);
        let o = build_tick(poll_time(), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(matches!(&o[0].1, TickOutcome::Retrying(e) if e.starts_with("en:")));
        assert!(base.posts.borrow().is_empty());
    }

    #[tokio::test]
    async fn slug_collision_gets_accession_suffix() {
        let (base, es, fl) = detected().await;
        base.posts.borrow_mut().push(PostRow {
            id: Some(99),
            run_key: "earnings:AAPL:0000320193-26-000001".into(),
            content_type: "earnings".into(),
            template_id: "earnings:v1".into(),
            trading_date: "2026-10-29".into(),
            slug: "aapl-q4-fy2026-earnings".into(),
            facts_r2_key: "x".into(),
            published_at: None,
        });
        build_tick(poll_time(), &es, base.as_ref(), &fl, &llm(true, true))
            .await
            .unwrap();
        let posts = base.posts.borrow();
        assert_eq!(posts.last().unwrap().slug, "aapl-q4-fy2026-earnings-000070");
    }

    #[tokio::test]
    async fn at_most_one_build_per_tick_and_leases_respected() {
        let (base, es, fl) = detected().await;
        let mut second = es.filings.borrow()[ACC].clone();
        second.accession = "0000320193-26-000099".into();
        second.run_key = run_key("AAPL", &second.accession);
        second.accepted_at = "2026-10-30T20:40:00Z".into();
        es.insert_filing(&second).await.unwrap();
        let l = llm(true, true);
        let o = build_tick(poll_time(), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].0, KEY, "oldest first");
        // A fresh `building` run is left alone.
        let mut r = base.runs.borrow()[&second.run_key].clone();
        r.status = "building".into();
        r.updated_at = iso(poll_time());
        base.runs.borrow_mut().insert(second.run_key.clone(), r);
        let o = build_tick(
            poll_time() + Duration::minutes(5),
            &es,
            base.as_ref(),
            &fl,
            &l,
        )
        .await
        .unwrap();
        assert!(o.is_empty());
        let o = build_tick(
            poll_time() + Duration::minutes(16),
            &es,
            base.as_ref(),
            &fl,
            &l,
        )
        .await
        .unwrap();
        assert_eq!(o.len(), 1);
    }

    #[tokio::test]
    async fn pending_zh_does_not_hold_up_new_builds() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        build_tick(poll_time(), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        let mut second = es.filings.borrow()[ACC].clone();
        second.accession = "0000320193-26-000099".into();
        second.run_key = run_key("AAPL", &second.accession);
        second.state = "open".into();
        second.accepted_at = "2026-10-30T20:40:00Z".into();
        es.insert_filing(&second).await.unwrap();
        let t = poll_time() + Duration::minutes(6);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l).await.unwrap();
        let keys: Vec<_> = o.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&second.run_key.as_str()), "{o:?}");
        assert!(keys.contains(&KEY), "the ZH retry runs too: {o:?}");
    }

    #[tokio::test]
    async fn zh_retries_wait_between_attempts() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        let t = poll_time();
        build_tick(t, &es, base.as_ref(), &fl, &l).await.unwrap();
        let drafts = l.drafts.get();
        let o = build_tick(t + Duration::minutes(2), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(o.is_empty());
        assert_eq!(l.drafts.get(), drafts);
        let o = build_tick(t + Duration::minutes(6), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(matches!(o[0].1, TickOutcome::Retrying(_)));
    }

    #[tokio::test]
    async fn a_build_that_never_finishes_fails_once() {
        let (base, es, fl) = detected().await;
        let mut r = base.runs.borrow()[KEY].clone();
        r.status = "building".into();
        r.attempts = 3;
        r.updated_at = "2026-10-31T01:00:00Z".into();
        base.runs.borrow_mut().insert(KEY.into(), r);
        let l = llm(true, true);
        // 6h after acceptance (20:30Z) and the lease long expired.
        let o = build_tick(utc("2026-10-31T02:40:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(base.runs.borrow()[KEY].status, "failed");
        assert_eq!(base.alerts.borrow().len(), 1);
        assert_eq!(l.drafts.get(), 0);
        let o = build_tick(utc("2026-10-31T02:50:00Z"), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert!(o.is_empty());
        assert_eq!(base.alerts.borrow().len(), 1);
    }

    #[tokio::test]
    async fn reconcile_restores_sources_and_charts() {
        let (base, es, fl) = detected().await;
        let l = llm(true, true);
        let t = poll_time();
        build_tick(t, &es, base.as_ref(), &fl, &l).await.unwrap();
        // Simulate a crash after EN was written: no sources, no charts, run not published.
        base.sources.borrow_mut().clear();
        es.charts.borrow_mut().clear();
        es.set_filing_state(ACC, "open").await.unwrap();
        let mut r = base.runs.borrow()[KEY].clone();
        r.status = "building".into();
        r.updated_at = iso(t);
        base.runs.borrow_mut().insert(KEY.into(), r);
        let drafts = l.drafts.get();
        build_tick(t + Duration::minutes(20), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        assert_eq!(l.drafts.get(), drafts, "no rebuild");
        assert_eq!(base.runs.borrow()[KEY].status, "published");
        assert_eq!(base.sources.borrow()[&1].len(), 2);
        assert_eq!(es.charts.borrow()[&1].len(), 2);
    }

    #[tokio::test]
    async fn unreadable_facts_end_zh_without_blocking() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        build_tick(poll_time(), &es, base.as_ref(), &fl, &l)
            .await
            .unwrap();
        base.facts
            .borrow_mut()
            .insert("earnings/AAPL/0000320193-26-000070.json".into(), "{".into());
        let o = build_tick(
            poll_time() + Duration::minutes(6),
            &es,
            base.as_ref(),
            &fl,
            &l,
        )
        .await
        .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "failed"
        );
        assert_eq!(es.filings.borrow()[ACC].state, "done");
    }

    #[tokio::test]
    async fn late_detection_still_gets_retries() {
        let (base, es, fl) = detected().await;
        fl.no_exhibit.set(true);
        // Found on Monday after a Friday filing: past 6h from acceptance, but
        // just detected.
        let mut f = es.filings.borrow()[ACC].clone();
        f.detected_at = "2026-11-02T11:00:00Z".into();
        es.filings.borrow_mut().insert(ACC.into(), f);
        let o = build_tick(
            utc("2026-11-02T11:05:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &llm(true, true),
        )
        .await
        .unwrap();
        assert!(matches!(o[0].1, TickOutcome::Retrying(_)), "{o:?}");
    }
}
