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
use crate::cover::{earnings_graphic, earnings_lines, store_graphic};
use crate::earnings::charts::{charts, r2_key};
use crate::earnings::detect::{domestic_by_cik, earnings_entries, name_mismatches};
use crate::earnings::draft::{slug, write, CONTENT_TYPE, TEMPLATE_ID};
use crate::earnings::extract::{extract, Extracted};
use crate::earnings::facts::{missing_required, EarningsFacts, Metric, Metrics, NOT_STATED};
use crate::earnings::foreign::{foreign_tick, ForeignOutcome};
use crate::earnings::store::{ChartRow, EarningsStore, FilingRow};
use crate::earnings::xbrl::fetch_trend;
use crate::earnings::{find, in_window, Tier, WatchEntry};
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
/// No run gives up before this many builds, so a backlog (one build per
/// tick) cannot spend a filing's whole window before its first try.
const MIN_ATTEMPTS: i64 = 3;
/// Chinese retries stop this long after acceptance.
const ZH_DEADLINE_HOURS: i64 = 24;
/// A poll message older than one firing interval is dropped: after a
/// backlog the next firing reads the feed anyway.
pub const MAX_POLL_LAG_MINUTES: i64 = 5;
/// Feed pages one poll reads at most (300 filings, far more than EDGAR
/// accepts in 5 minutes even at the peak of earnings season).
pub const MAX_FEED_PAGES: usize = 3;
/// Build messages a poll sends for open filings when it detected nothing
/// new, so a burst's retries drain three at a time.
pub const MAX_RETRY_BUILDS: usize = 3;
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
    /// The 6-K part: foreign filers' results releases.
    pub foreign: ForeignOutcome,
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
    // Every page re-reads hours of filings, so each step is one D1 query
    // for all of them (Free: 50 per invocation), whatever the count.
    let mut rows: Vec<(FilingRow, Tier)> = Vec::new();
    for (f, w) in earnings_entries(&entries, &watch, now) {
        // An entry on two pages is one filing.
        if !rows.iter().any(|(r, _)| r.accession == f.accession) {
            rows.push((filing_row(f, &w, now), w.tier));
        }
    }
    let filings: Vec<FilingRow> = rows.iter().map(|(r, _)| r.clone()).collect();
    // Calendar first and on every sighting (idempotent), so a failed write
    // after the insert cannot leave the calendar unmarked.
    crate::earnings::calendar::record_filings(es, &filings, now).await?;
    // A calendar-tier report only marks its date reported.
    let full: Vec<String> = rows
        .iter()
        .filter(|(_, t)| *t == Tier::Full)
        .map(|(r, _)| r.accession.clone())
        .collect();
    if !full.is_empty() {
        let known = es.known_accessions(&full).await?;
        let new: Vec<(FilingRow, RunRow)> = filings
            .into_iter()
            .filter(|r| full.contains(&r.accession) && !known.contains(&r.accession))
            .map(|r| {
                let run = new_run(&r, now);
                (r, run)
            })
            .collect();
        if !new.is_empty() {
            es.insert_new_filings(&new).await?;
        }
        out.detected = new.into_iter().map(|(r, _)| r.run_key).collect();
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
/// follow-up messages: one build per new filing (or, with none, see
/// `retry_builds`: for retries and Chinese drafts), the calendar refresh
/// until it ran today, and the macro events check until it ran today or
/// while an event's release may have just come out.
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
    // Foreign filers report on 6-Ks; skipped while SEC refuses requests.
    // An error here (a D1 failure included) is logged, and the builds,
    // calendar refresh and events check below are still queued.
    if !out.throttled {
        match foreign_tick(now, es, filings).await {
            Ok(f) => {
                if f.throttled {
                    let msg = format!(
                        "SEC EDGAR refused requests (403/429) on the 6-K feed: {}",
                        f.errors.last().cloned().unwrap_or_default()
                    );
                    alert_once_today(now, "sec-throttled", "error", &msg, store).await?;
                }
                for (ticker, msg) in &f.mismatches {
                    out.errors.push(msg.clone());
                    alert_once_today(now, &format!("watchlist-cik:{ticker}"), "warn", msg, store)
                        .await?;
                }
                out.errors.extend(f.errors.iter().cloned());
                out.foreign = f;
            }
            Err(e) => out.errors.push(format!("6-K: {e}")),
        }
    }
    let builds = match out.detected.len() {
        0 => retry_builds(&es.open_runs().await?, now),
        n => n,
    };
    out.follow_ups = vec![EarningsStep::Build; builds];
    // One message per pending 6-K, each read in its own invocation.
    out.follow_ups.extend(std::iter::repeat_n(
        EarningsStep::Foreign,
        out.foreign.reads,
    ));
    if !crate::earnings::calendar::refreshed_today(es, now).await? {
        out.follow_ups.push(EarningsStep::Calendar);
    }
    if !crate::events::check::checked_today(es, now).await?
        || crate::events::release_due(now, crate::events::check::RELEASE_WINDOW_MINUTES)
    {
        out.follow_ups.push(EarningsStep::Events);
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
/// after detection, so a filing found late (weekend, outage) still gets
/// retries, and never before `MIN_ATTEMPTS` builds.
fn past_deadline(f: &FilingRow, run: &RunRow, now: DateTime<Utc>) -> bool {
    let detected = parse_iso(&f.detected_at).map_or(Duration::zero(), |t| now - t);
    run.attempts >= MIN_ATTEMPTS
        && age(f, now) >= Duration::hours(BUILD_DEADLINE_HOURS)
        && detected >= Duration::hours(LATE_DETECTION_HOURS)
}

fn since_update(r: &RunRow, now: DateTime<Utc>) -> Duration {
    parse_iso(&r.updated_at).map_or(Duration::MAX, |t| now - t)
}

/// A build is running: its lease has not run out.
fn leased(run: &RunRow, now: DateTime<Utc>) -> bool {
    run.status == "building" && since_update(run, now) < Duration::minutes(BUILD_LEASE_MINUTES)
}

/// `build_tick` would act on the filing now: close it, retry its Chinese
/// draft, or (re)build it. Not while its build is leased or a failed attempt
/// waits out `RETRY_MINUTES`.
fn due(run: &RunRow, now: DateTime<Utc>) -> bool {
    let waiting = run.attempts > 0
        && run.status == "pending"
        && since_update(run, now) < Duration::minutes(RETRY_MINUTES);
    !leased(run, now) && !waiting
}

/// Build messages for open filings when a poll found nothing new: one per
/// due filing, at most `MAX_RETRY_BUILDS`, and none while a build is leased
/// (the serial build queue would only stack messages behind it that find
/// nothing to do).
fn retry_builds(open: &[(FilingRow, Option<RunRow>)], now: DateTime<Utc>) -> usize {
    if open
        .iter()
        .any(|(_, r)| r.as_ref().is_some_and(|r| leased(r, now)))
    {
        return 0;
    }
    open.iter()
        .filter(|(_, r)| r.as_ref().is_none_or(|r| due(r, now)))
        .count()
        .min(MAX_RETRY_BUILDS)
}

/// Works through the open filings, oldest first: closes finished ones,
/// skips in-flight or recently retried ones, and does at most one piece of
/// LLM work (a build or a Chinese retry) per tick. `log` receives the
/// cover outcome, `cover=ok|err|none`, once per build that publishes.
pub async fn build_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    filings: &dyn Filings,
    llm: &dyn LlmProvider,
    log: &dyn Fn(&str),
) -> Result<Vec<(String, TickOutcome)>, String> {
    let mut out = Vec::new();
    // One build and one Chinese retry per tick, budgeted separately so a
    // Chinese draft that keeps failing never holds up new posts.
    let (mut built, mut zh_tried) = (false, false);
    for (f, run) in es.open_runs().await? {
        let run = match run {
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
            _ if !due(&run, now) => continue,
            "published" if zh_tried => continue,
            "published" => match retry_zh(now, es, store, llm, &f, &run).await? {
                TickOutcome::Nothing => continue,
                o => {
                    zh_tried = true;
                    o
                }
            },
            // A build that never finishes (the Worker killed mid-build) must
            // not be retried forever.
            "building" if past_deadline(&f, &run, now) => {
                give_up(now, es, store, &f, run, "the build never finished".into()).await?
            }
            _ if built => continue,
            _ => {
                built = true;
                if let Some(o) = reconcile(now, es, store, &f, &run).await? {
                    o
                } else {
                    build(now, es, store, filings, llm, log, &f, run).await?
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
    let mut lines = None;
    if let Some(json) = store.get_facts(&post.facts_r2_key).await? {
        let facts: EarningsFacts =
            serde_json::from_str(&json).map_err(|e| format!("facts json: {e}"))?;
        store.add_sources(post_id, &sources(&facts)).await?;
        let rows = store_charts(store, &facts, post_id).await?;
        es.set_charts(post_id, &rows).await?;
        lines = Some(earnings_lines(&facts).zh);
    }
    let ts = iso(now);
    let zh = store.get_translation(post_id, "zh").await?;
    if zh.is_none() {
        store
            .upsert_translation(&empty_translation(post_id, "zh", "pending", lines, &ts))
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

/// A row without a draft. It still carries the cover `lines`: the upsert
/// replaces the whole row, so a write without them would erase them.
fn empty_translation(
    post_id: i64,
    lang: &str,
    status: &str,
    lines: Option<[String; 2]>,
    ts: &str,
) -> TranslationRow {
    let [cover_line1, cover_line2] = lines.map_or([None, None], |l| l.map(Some));
    TranslationRow {
        post_id,
        lang: lang.into(),
        status: status.into(),
        title: String::new(),
        summary: String::new(),
        body_md: String::new(),
        updated_at: ts.into(),
        cover_line1,
        cover_line2,
    }
}

/// The cover lines a stored row carries, when it has both.
fn row_lines(t: &TranslationRow) -> Option<[String; 2]> {
    Some([t.cover_line1.clone()?, t.cover_line2.clone()?])
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
    if past_deadline(f, &row, now) {
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

/// Why a field `missing_required` names is missing: the extraction, EDGAR
/// or verifier reason (for "eps", GAAP EPS's unless it is simply not
/// stated, then non-GAAP EPS's), so the run error says which step failed.
fn missing_reason<'a>(f: &'a EarningsFacts, field: &str) -> Option<&'a str> {
    let reason = |m: Metric| match f.metrics.get(m) {
        Field::Missing { reason } => Some(reason.as_str()),
        Field::Value(_) => None,
    };
    match field {
        "revenue" => reason(Metric::Revenue),
        "eps" => match reason(Metric::EpsGaap) {
            Some(NOT_STATED) => reason(Metric::EpsNonGaap).or(Some(NOT_STATED)),
            r => r,
        },
        _ => None,
    }
}

/// The run error for missing required fields: each one's reason (with the
/// rejected quote, from `extract`), a reason already given not repeated.
fn missing_error(f: &EarningsFacts, missing: &[&str]) -> String {
    let mut reasons: Vec<String> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for field in missing {
        if let Some(reason) = missing_reason(f, field).filter(|r| !seen.contains(r)) {
            seen.push(reason);
            reasons.push(format!("{field}: {reason}"));
        }
    }
    let mut err = format!("missing {}", missing.join(", "));
    if !reasons.is_empty() {
        err.push_str(&format!(" ({})", reasons.join("; ")));
    }
    err
}

#[allow(clippy::too_many_arguments)]
async fn build(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    filings: &dyn Filings,
    llm: &dyn LlmProvider,
    log: &dyn Fn(&str),
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
        let err = missing_error(&facts, &missing);
        return not_ready(now, es, store, f, row, err).await;
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
    // A failed cover write leaves the post on the flat tile; it never blocks
    // publishing.
    let (cover_key, outcome, cover_err) =
        store_graphic(store, &post_slug, earnings_graphic(&facts)).await;
    log(&format!("cover={outcome}"));
    if let Some(e) = cover_err {
        log(&format!("cover write failed: {e}"));
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
            cover_key,
            revised_at: None,
        })
        .await?;
    for c in &mut chart_rows {
        c.post_id = post_id;
    }
    let lines = earnings_lines(&facts);
    let [en_line1, en_line2] = lines.en;
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
    let zh_err = match zh {
        Ok((d, body)) => {
            let [zh_line1, zh_line2] = lines.zh;
            store
                .upsert_translation(&TranslationRow {
                    post_id,
                    lang: "zh".into(),
                    status: "published".into(),
                    title: d.title,
                    summary: d.summary,
                    body_md: body,
                    updated_at: ts.clone(),
                    cover_line1: Some(zh_line1),
                    cover_line2: Some(zh_line2),
                })
                .await?;
            None
        }
        Err(e) => {
            store
                .upsert_translation(&empty_translation(
                    post_id,
                    "zh",
                    "pending",
                    Some(lines.zh),
                    &ts,
                ))
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
    // Missing or unreadable facts cannot be fixed by retrying. Without
    // facts, the lines the build wrote are kept from the existing row.
    let kept = zh.as_ref().and_then(row_lines);
    let facts: EarningsFacts = match store.get_facts(&post.facts_r2_key).await? {
        None => {
            let err = "facts not found".to_string();
            return zh_failed(now, es, store, f, run, Some(post_id), kept, err).await;
        }
        Some(json) => match serde_json::from_str(&json) {
            Ok(x) => x,
            Err(e) => {
                let err = format!("facts json: {e}");
                return zh_failed(now, es, store, f, run, Some(post_id), kept, err).await;
            }
        },
    };
    let ts = iso(now);
    let lines = earnings_lines(&facts).zh;
    match write(llm, &facts, Lang::Zh).await {
        Ok((d, body)) => {
            let [zh_line1, zh_line2] = lines;
            store
                .upsert_translation(&TranslationRow {
                    post_id,
                    lang: "zh".into(),
                    status: "published".into(),
                    title: d.title,
                    summary: d.summary,
                    body_md: body,
                    updated_at: ts,
                    cover_line1: Some(zh_line1),
                    cover_line2: Some(zh_line2),
                })
                .await?;
            es.set_filing_state(&f.accession, "done").await?;
            Ok(TickOutcome::Published)
        }
        Err(e) => {
            let err = format!("zh: {}", e.join("; "));
            if age(f, now) >= Duration::hours(ZH_DEADLINE_HOURS) {
                return zh_failed(now, es, store, f, run, Some(post_id), Some(lines), err).await;
            }
            // Restarts the retry pause.
            store
                .upsert_translation(&empty_translation(
                    post_id,
                    "zh",
                    "pending",
                    Some(lines),
                    &ts,
                ))
                .await?;
            Ok(TickOutcome::Retrying(err))
        }
    }
}

/// Chinese is final as `failed` (keeping the cover `lines`), with one warn
/// alert; the filing is closed.
#[allow(clippy::too_many_arguments)]
async fn zh_failed(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    f: &FilingRow,
    run: &RunRow,
    post_id: Option<i64>,
    lines: Option<[String; 2]>,
    err: String,
) -> Result<TickOutcome, String> {
    if let Some(id) = post_id {
        store
            .upsert_translation(&empty_translation(id, "zh", "failed", lines, &iso(now)))
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
    use crate::cover::{earnings_graphic, earnings_lines};
    use crate::earnings::draft::tests::reply;
    use crate::earnings::store::MemEarningsStore;
    use crate::earnings::xbrl::tests::aapl_concept;
    use crate::earnings::Filer;
    use crate::edgar::tests::{EX991, FEED, INDEX};
    use crate::edgar::{html_to_text, parse_current_feed, parse_filing_index, Concept, FilingDoc};
    use crate::store::{MemStore, PostRow, TranslationRow};
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
        /// No XBRL concepts: the facts carry no quarterly trend.
        no_trend: Cell<bool>,
        /// The exhibit's text, when not the AAPL fixture's.
        exhibit_text: RefCell<Option<String>>,
        /// The current 6-K feed's first page.
        six_k: RefCell<Vec<FeedEntry>>,
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
        async fn current_6k_filings(&self, start: usize) -> Result<Vec<FeedEntry>, ProviderError> {
            Ok(if start == 0 {
                self.six_k.borrow().clone()
            } else {
                vec![]
            })
        }
        async fn filing_docs(&self, _: u64, _: &str) -> Result<Vec<FilingDoc>, ProviderError> {
            if self.no_exhibit.get() {
                return Ok(vec![]);
            }
            Ok(parse_filing_index(INDEX))
        }
        async fn document_text(&self, _: &str) -> Result<String, ProviderError> {
            Ok(self
                .exhibit_text
                .borrow()
                .clone()
                .unwrap_or_else(|| html_to_text(EX991)))
        }
        async fn company_concept(
            &self,
            _: u64,
            tag: &str,
        ) -> Result<Option<Concept>, ProviderError> {
            if self.no_trend.get() {
                return Ok(None);
            }
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
                crate::earnings::by_cik(*cik).map_or("Co", |w| w.sec),
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
        /// The release states no revenue (the extraction leaves it out).
        no_revenue: Cell<bool>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, system: &str, _user: &str) -> Result<Value, ProviderError> {
            if system.starts_with("You extract reported figures") {
                let mut v: Value = serde_json::from_str(EXTRACT_REPLY).unwrap();
                if self.no_revenue.get() {
                    v["metrics"].as_object_mut().unwrap().remove("revenue");
                }
                return Ok(v);
            }
            self.drafts.set(self.drafts.get() + 1);
            let (lang, ok) = if system.starts_with("你负责撰写财报速评") {
                (Lang::Zh, self.zh_ok.get())
            } else {
                (Lang::En, self.en_ok.get())
            };
            // 13% is not in the facts.
            let body = match (lang, ok, self.no_revenue.get()) {
                (_, false, _) => "Services grew 13%.",
                (Lang::En, true, false) => "Revenue was $102.5 billion.",
                (Lang::Zh, true, false) => "营收1025亿美元。",
                (Lang::En, true, true) => "GAAP diluted EPS was $1.85.",
                (Lang::Zh, true, true) => "GAAP摊薄每股收益为1.85美元。",
            };
            let mut r = reply(lang, body);
            if self.no_revenue.get() {
                // The headline and summary cannot cite a revenue figure either.
                r["title"] = match lang {
                    Lang::En => "Apple fiscal Q4 2026: GAAP EPS $1.85".into(),
                    Lang::Zh => "苹果2026财年第四季度GAAP每股收益1.85美元".into(),
                };
                r["summary"] = r["sections"][0]["markdown"].clone();
            }
            Ok(r)
        }
    }
    fn llm(en_ok: bool, zh_ok: bool) -> FakeLlm {
        FakeLlm {
            en_ok: Cell::new(en_ok),
            zh_ok: Cell::new(zh_ok),
            drafts: Cell::new(0),
            no_revenue: Cell::new(false),
        }
    }

    /// The stores, with the macro events check already done on
    /// `poll_time()`'s day (`polls_queue_the_events_check` covers it).
    fn stores() -> (Rc<MemStore>, MemEarningsStore) {
        let base = Rc::new(MemStore::default());
        let es = MemEarningsStore::new(base.clone());
        es.kv
            .borrow_mut()
            .insert("events_checked_on".into(), "2026-10-30".into());
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
        crate::earnings::calendar::record_filings(&es, &[row], utc("2026-10-31T00:50:00Z"))
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
        // One earlier build, so the 02:31 build is the third (`MIN_ATTEMPTS`).
        let mut r = base.runs.borrow()[KEY].clone();
        r.attempts = 1;
        base.runs.borrow_mut().insert(KEY.into(), r);
        let l = llm(true, true);
        let o = build_tick(
            utc("2026-10-31T02:25:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert!(matches!(o[0].1, TickOutcome::Retrying(_)), "{o:?}");
        let o = build_tick(
            utc("2026-10-31T02:31:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
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
        // The 6-K feed is read too (empty here).
        assert_eq!(o.foreign.pages, 1);
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
        // A burst's retries: one build per open filing, at most
        // `MAX_RETRY_BUILDS` per firing.
        for i in 0..5 {
            let mut f = es.filings.borrow()[ACC].clone();
            f.accession = format!("0000320193-26-00010{i}");
            f.run_key = run_key("AAPL", &f.accession);
            es.insert_filing(&f).await.unwrap();
        }
        let o = poll_job(t2, Some(t2), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert_eq!(o.follow_ups, [EarningsStep::Build; MAX_RETRY_BUILDS]);
        for i in 0..5 {
            es.set_filing_state(&format!("0000320193-26-00010{i}"), "done")
                .await
                .unwrap();
        }
        // Nothing open, calendar done: no follow-ups.
        es.set_filing_state(ACC, "done").await.unwrap();
        let o = poll_job(t2, Some(t2), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(o.follow_ups.is_empty(), "{o:?}");
    }

    #[tokio::test]
    async fn polls_queue_the_events_check_daily_and_after_a_release_time() {
        let (base, es) = stores();
        es.kv.borrow_mut().clear();
        es.kv_set("calendar_refreshed_on", "2026-11-10", "x")
            .await
            .unwrap();
        let fl = FakeFilings::default();
        // 06:00 ET on CPI day, not yet checked: queued.
        let t = utc("2026-11-10T11:00:00Z");
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(o.follow_ups.contains(&EarningsStep::Events), "{o:?}");
        es.kv_set("events_checked_on", "2026-11-10", "x")
            .await
            .unwrap();
        // Checked, and no event just out: nothing.
        let t = utc("2026-11-10T13:00:00Z");
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(!o.follow_ups.contains(&EarningsStep::Events), "{o:?}");
        // 8:35 ET, CPI was due at 8:30: queued for the released check.
        let t = utc("2026-11-10T13:35:00Z");
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(o.follow_ups.contains(&EarningsStep::Events), "{o:?}");
    }

    #[tokio::test]
    async fn foreign_and_chinese_filers_never_queue_a_build() {
        // An 8-K with Item 2.02 under Yum China's CIK (a domestic, calendar
        // tier filer) and under every 20-F/6-K filer's. Foreign rows are not
        // watched at all; this guards that they stay so.
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let t = poll_time();
        let rows: Vec<_> = crate::earnings::watchlist()
            .into_iter()
            .filter(|e| {
                e.filer == Filer::Foreign
                    || crate::earnings::watchlist::tests::CHINA.contains(&e.ticker)
            })
            .collect();
        assert!(rows.len() >= 50, "{}", rows.len());
        let page: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(i, e)| {
                (
                    e.cik,
                    format!("{:010}-26-{:06}", e.cik, i),
                    t - Duration::minutes(i as i64 % 60),
                    "2.02,9.01",
                )
            })
            .collect();
        fl.pages.borrow_mut().insert(0, Ok(feed(&page)));
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(o.detected.is_empty(), "{:?}", o.detected);
        assert!(
            !o.follow_ups.contains(&EarningsStep::Build),
            "{:?}",
            o.follow_ups
        );
        assert!(es.filings.borrow().is_empty() && base.runs.borrow().is_empty());
        // Foreign filers are not watched for 8-Ks at all; Yum China's marks
        // its calendar date only.
        let cal = es.calendar.borrow();
        assert_eq!(cal.len(), 1, "{cal:?}");
        assert_eq!(cal.keys().next().unwrap().0, "YUMC");
    }

    #[tokio::test]
    async fn a_calendar_tier_report_marks_the_calendar_and_queues_no_build() {
        // Agilent: domestic filer, calendar tier.
        let a = crate::earnings::find("A").unwrap();
        assert_eq!((a.filer, a.tier), (Filer::Domestic, Tier::Calendar));
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let t = poll_time();
        let acc = "0001090872-26-000041".to_string();
        fl.pages.borrow_mut().insert(
            0,
            Ok(feed(&[(
                a.cik,
                acc.clone(),
                t - Duration::minutes(10),
                "2.02,9.01",
            )])),
        );
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(o.detected.is_empty());
        assert!(es.filings.borrow().is_empty());
        assert!(base.runs.borrow().is_empty());
        assert_eq!(o.follow_ups, [EarningsStep::Calendar]);
        let cal = es.calendar_for("A").await.unwrap();
        assert_eq!(cal.len(), 1);
        assert_eq!(
            (cal[0].status.as_str(), cal[0].report_date.as_str()),
            ("reported", "2026-10-30")
        );
        assert_eq!(
            cal[0].proof_url.as_deref(),
            Some(filing_url(a.cik, &acc).as_str())
        );
    }

    #[tokio::test]
    async fn a_busy_poll_uses_a_fixed_number_of_store_queries() {
        // 40 watchlist reports on one page (a 16:00 ET burst at about 560
        // rows): per-filing reads and writes would pass the Free plan's 50
        // D1 queries per invocation.
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let t = poll_time();
        let picks: Vec<_> = crate::earnings::watchlist()
            .into_iter()
            .filter(|e| e.filer == Filer::Domestic)
            .take(40)
            .collect();
        let page: Vec<_> = picks
            .iter()
            .enumerate()
            .map(|(i, e)| {
                (
                    e.cik,
                    format!("{:010}-26-{:06}", e.cik, i),
                    t - Duration::minutes(i as i64),
                    "2.02,9.01",
                )
            })
            .collect();
        fl.pages.borrow_mut().insert(0, Ok(feed(&page)));
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        let full = picks.iter().filter(|e| e.tier == Tier::Full).count();
        assert!(full > 0 && full < 40, "{full}");
        assert_eq!(o.detected.len(), full);
        assert_eq!(es.calendar.borrow().len(), 40);
        // Watermark read, calendar read, calendar write, existence check,
        // filings and runs insert, watermark write.
        assert_eq!(es.calls.get(), 6);
        // The next firing sees the same page: nothing to write.
        es.calls.set(0);
        let o = poll_tick(t + Duration::minutes(5), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert!(o.detected.is_empty());
        assert_eq!(
            es.calls.get(),
            3,
            "watermark read, calendar read, existence check"
        );
    }

    #[tokio::test]
    async fn a_poll_never_resets_a_run_a_build_already_took() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let mut r = new_run(
            &filing_row(
                &parse_current_feed(FEED).unwrap()[0],
                &find("AAPL").unwrap(),
                poll_time(),
            ),
            poll_time(),
        );
        r.status = "building".into();
        base.runs.borrow_mut().insert(KEY.into(), r);
        let o = poll_tick(poll_time(), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert_eq!(o.detected, [KEY]);
        assert_eq!(base.runs.borrow()[KEY].status, "building");
    }

    #[test]
    fn retries_go_only_to_due_filings_and_wait_for_a_leased_build() {
        let t = poll_time();
        let f = filing_row(
            &parse_current_feed(FEED).unwrap()[0],
            &find("AAPL").unwrap(),
            t,
        );
        let run = |status: &str, attempts: i64, mins_ago: i64| {
            let mut r = new_run(&f, t);
            r.status = status.into();
            r.attempts = attempts;
            r.updated_at = iso(t - Duration::minutes(mins_ago));
            Some(r)
        };
        let open = |runs: Vec<Option<RunRow>>| -> Vec<(FilingRow, Option<RunRow>)> {
            runs.into_iter().map(|r| (f.clone(), r)).collect()
        };
        // Just retried, so waiting; a Chinese draft to retry; no run yet.
        let o = open(vec![run("pending", 1, 2), run("published", 1, 60), None]);
        assert_eq!(retry_builds(&o, t), 2);
        // A due pending one and an expired lease: both due, capped at 3.
        let o = open(vec![
            run("pending", 1, 6),
            run("building", 1, 16),
            None,
            None,
        ]);
        assert_eq!(retry_builds(&o, t), MAX_RETRY_BUILDS);
        // While a build holds its lease, nothing is sent.
        let o = open(vec![run("building", 1, 3), run("pending", 0, 0)]);
        assert_eq!(retry_builds(&o, t), 0);
    }

    #[tokio::test]
    async fn a_failing_6k_part_never_holds_up_the_poll() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        *es.fail_kv.borrow_mut() = Some(crate::earnings::foreign::KV_6K_WATERMARK.into());
        let t = poll_time();
        let o = poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        assert!(
            o.errors.iter().any(|e| e.starts_with("6-K: ")),
            "{:?}",
            o.errors
        );
        // The new filing's build and the day's calendar refresh still go.
        assert_eq!(o.detected, [KEY]);
        assert!(o.follow_ups.contains(&EarningsStep::Build));
        assert!(o.follow_ups.contains(&EarningsStep::Calendar));
    }

    #[tokio::test]
    async fn a_foreign_filer_under_another_name_alerts_once_a_day() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let tm = find("TM").unwrap();
        let t = poll_time();
        let acc = "0001094517-26-000077".to_string();
        let mut entry = parse_current_feed(&feed(&[(tm.cik, acc, t - Duration::minutes(5), "")]))
            .unwrap()
            .remove(0);
        entry.form = "6-K".into();
        entry.company = "TOYOTA INDUSTRIES CORP".into();
        fl.six_k.borrow_mut().push(entry);
        poll_job(t, Some(t), &es, base.as_ref(), &fl).await.unwrap();
        poll_job(
            t + Duration::minutes(5),
            Some(t + Duration::minutes(5)),
            &es,
            base.as_ref(),
            &fl,
        )
        .await
        .unwrap();
        let alerts: Vec<_> = base
            .alerts
            .borrow()
            .iter()
            .filter(|a| a.0.starts_with("watchlist-cik:TM"))
            .cloned()
            .collect();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
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
            "ops@prebell.site",
        )
        .unwrap();
        let inner = llm(true, false);
        let l = CountingLlm(&inner, Cell::new(0));
        let o = build_tick(poll_time(), &es, base.as_ref(), &edgar, &l, &|_| {})
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
    async fn an_open_filing_still_builds_after_its_company_moves_to_the_calendar_tier() {
        // A company dropped below the write-up line mid-season: its filing
        // was already detected and finishes.
        let (base, es, fl) = detected().await;
        let mut f = es.filings.borrow()[ACC].clone();
        f.ticker = "A".into();
        es.filings.borrow_mut().insert(ACC.into(), f);
        assert_eq!(find("A").unwrap().tier, Tier::Calendar);
        let l = llm(true, true);
        let o = build_tick(poll_time(), &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        assert_eq!(o.len(), 1, "{o:?}");
        assert_eq!(base.runs.borrow()[KEY].attempts, 1);
    }

    /// `Store` that counts the calls D1 serves (R2 reads and writes are not
    /// D1 queries).
    struct D1Count<'a>(&'a MemStore, Cell<usize>);
    impl D1Count<'_> {
        fn tick(&self) {
            self.1.set(self.1.get() + 1);
        }
    }
    #[async_trait(?Send)]
    impl Store for D1Count<'_> {
        async fn get_run(&self, key: &str) -> Result<Option<RunRow>, String> {
            self.tick();
            self.0.get_run(key).await
        }
        async fn upsert_run(&self, r: &RunRow) -> Result<(), String> {
            self.tick();
            self.0.upsert_run(r).await
        }
        async fn get_post_by_run(&self, k: &str) -> Result<Option<PostRow>, String> {
            self.tick();
            self.0.get_post_by_run(k).await
        }
        async fn revise_post(
            &self,
            p: &PostRow,
            t: &[TranslationRow],
            s: &[NewsReason],
        ) -> Result<(), String> {
            self.tick();
            self.0.revise_post(p, t, s).await
        }
        async fn upsert_translation_at_revision(
            &self,
            t: &TranslationRow,
            revised_at: Option<&str>,
        ) -> Result<bool, String> {
            self.tick();
            self.0.upsert_translation_at_revision(t, revised_at).await
        }
        async fn posts_on(&self, c: &str, d: &str) -> Result<Vec<PostRow>, String> {
            self.tick();
            self.0.posts_on(c, d).await
        }
        async fn upsert_post(&self, p: &PostRow) -> Result<i64, String> {
            self.tick();
            self.0.upsert_post(p).await
        }
        async fn get_translation(
            &self,
            id: i64,
            l: &str,
        ) -> Result<Option<TranslationRow>, String> {
            self.tick();
            self.0.get_translation(id, l).await
        }
        async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String> {
            self.tick();
            self.0.upsert_translation(t).await
        }
        async fn add_sources(&self, id: i64, s: &[crate::facts::NewsReason]) -> Result<(), String> {
            self.tick();
            self.0.add_sources(id, s).await
        }
        async fn add_alert(&self, k: &str, l: &str, m: &str) -> Result<(), String> {
            self.tick();
            self.0.add_alert(k, l, m).await
        }
        async fn has_alert(&self, k: &str) -> Result<bool, String> {
            self.tick();
            self.0.has_alert(k).await
        }
        async fn put_facts(&self, k: &str, j: &str) -> Result<(), String> {
            self.0.put_facts(k, j).await
        }
        async fn get_facts(&self, k: &str) -> Result<Option<String>, String> {
            self.0.get_facts(k).await
        }
        async fn put_bytes(&self, k: &str, b: &[u8], c: &str) -> Result<(), String> {
            self.0.put_bytes(k, b, c).await
        }
        async fn kv_set(&self, k: &str, v: &str, n: &str) -> Result<(), String> {
            self.tick();
            self.0.kv_set(k, v, n).await
        }
    }

    #[tokio::test]
    async fn a_build_stays_far_under_50_d1_queries() {
        // A build that publishes EN with ZH pending, then the next tick: a
        // second filing's build plus the first one's Chinese retry, the
        // busiest build invocation. Free allows 50 D1 queries per invocation.
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        let d1 = D1Count(base.as_ref(), Cell::new(0));
        es.calls.set(0);
        build_tick(poll_time(), &es, &d1, &fl, &l, &|_| {})
            .await
            .unwrap();
        let first = d1.1.get() + es.calls.get();
        let mut second = es.filings.borrow()[ACC].clone();
        second.accession = "0000320193-26-000099".into();
        second.run_key = run_key("AAPL", &second.accession);
        es.insert_filing(&second).await.unwrap();
        let (d1, l) = (D1Count(base.as_ref(), Cell::new(0)), llm(true, true));
        es.calls.set(0);
        let t = poll_time() + Duration::minutes(10);
        let o = build_tick(t, &es, &d1, &fl, &l, &|_| {}).await.unwrap();
        assert_eq!(o.len(), 2, "{o:?}");
        let both = d1.1.get() + es.calls.get();
        println!("d1 queries: build {first}, build + zh retry {both}");
        assert!(first <= 15 && both <= 25, "{first} {both}");
    }

    #[tokio::test]
    async fn full_build_publishes_one_post() {
        let (base, es, fl) = detected().await;
        let l = llm(true, true);
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
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
        let o = build_tick(
            t + Duration::minutes(1),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert!(o.is_empty());
    }

    #[tokio::test]
    async fn a_release_that_states_no_revenue_still_publishes() {
        let (base, es, fl) = detected().await;
        // The AAPL release with every revenue line renamed: none left to state.
        let text = regex::Regex::new(r"(?i)revenues?|net sales")
            .unwrap()
            .replace_all(&html_to_text(EX991), "turnover")
            .into_owned();
        *fl.exhibit_text.borrow_mut() = Some(text);
        let l = llm(true, true);
        l.no_revenue.set(true);
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        assert_eq!(o, vec![(KEY.to_string(), TickOutcome::Published)]);
        let f = stored_facts(&base);
        assert!(matches!(&f.metrics.revenue, Field::Missing { reason } if reason == NOT_STATED));
        let en = base.translations.borrow()[&(1, "en".to_string())].clone();
        assert!(!en.body_md.contains("| Revenue |"), "{}", en.body_md);
    }

    #[tokio::test]
    async fn a_model_that_skips_a_stated_revenue_retries() {
        // The AAPL release states revenue; a reply without it is not "not stated".
        let (base, es, fl) = detected().await;
        let l = llm(true, true);
        l.no_revenue.set(true);
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        assert!(
            matches!(&o[0].1, TickOutcome::Retrying(e) if e.starts_with("missing revenue")),
            "{o:?}"
        );
        assert!(base.posts.borrow().is_empty());
    }

    #[tokio::test]
    async fn rebuild_reuses_the_post() {
        let (base, es, fl) = detected().await;
        let l = llm(true, true);
        let t = poll_time() + Duration::minutes(1);
        build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        // An operator re-runs the filing.
        es.set_filing_state(ACC, "open").await.unwrap();
        let mut r = base.runs.borrow()[KEY].clone();
        r.status = "pending".into();
        base.runs.borrow_mut().insert(KEY.into(), r);
        base.translations.borrow_mut().clear();
        build_tick(
            t + Duration::minutes(10),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(base.posts.borrow().len(), 1);
        assert_eq!(base.posts.borrow()[0].slug, "aapl-q4-fy2026-earnings");
    }

    #[test]
    fn missing_error_names_each_field_once() {
        let mut f = crate::earnings::facts::tests::facts();
        let unverified = |r: &str| Field::Missing { reason: r.into() };
        f.metrics.revenue = unverified("unverified: a [quote: \"Total revenues $1 $2\"]");
        f.metrics.eps_gaap = unverified(NOT_STATED);
        f.metrics.eps_non_gaap = unverified("unverified: b [quote: \"Diluted $1.54\"]");
        assert_eq!(
            missing_error(&f, &missing_required(&f)),
            "missing revenue, eps (revenue: unverified: a [quote: \"Total revenues $1 $2\"]; eps: unverified: b [quote: \"Diluted $1.54\"])"
        );
        // GAAP EPS's reason when it has one; a reason the revenue gave is
        // not repeated.
        f.metrics.eps_gaap = unverified("unverified: c");
        assert!(missing_error(&f, &missing_required(&f)).ends_with("; eps: unverified: c)"));
        f.metrics.eps_gaap = unverified("extraction failed: x");
        f.metrics.eps_non_gaap = unverified("extraction failed: x");
        f.metrics.revenue = unverified("extraction failed: x");
        assert_eq!(
            missing_error(&f, &missing_required(&f)),
            "missing revenue, eps (revenue: extraction failed: x)"
        );
    }

    #[tokio::test]
    async fn missing_exhibit_retries_then_fails_after_deadline() {
        let (base, es, fl) = detected().await;
        fl.no_exhibit.set(true);
        let l = llm(true, true);
        // Accepted 20:30:41Z.
        let o = build_tick(
            utc("2026-10-31T01:30:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        let err = "missing revenue, eps (revenue: no EX-99 exhibit in the filing)";
        assert_eq!(o[0].1, TickOutcome::Retrying(err.into()));
        assert_eq!(base.runs.borrow()[KEY].status, "pending");
        assert_eq!(base.runs.borrow()[KEY].last_error.as_deref(), Some(err));
        // Retried no sooner than 5 minutes later.
        let o = build_tick(
            utc("2026-10-31T01:33:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert!(o.is_empty());
        // Past the deadline but only the second build: it keeps retrying.
        let o = build_tick(
            utc("2026-10-31T02:31:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert!(matches!(o[0].1, TickOutcome::Retrying(_)), "{o:?}");
        assert!(base.alerts.borrow().is_empty());
        let o = build_tick(
            utc("2026-10-31T02:37:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(base.runs.borrow()[KEY].status, "failed");
        assert_eq!(base.runs.borrow()[KEY].attempts, 3);
        assert_eq!(base.alerts.borrow().len(), 1);
        assert_eq!(base.alerts.borrow()[0].1, "error");
        assert!(
            base.alerts.borrow()[0].2.ends_with(err),
            "{:?}",
            base.alerts.borrow()[0]
        );
        assert!(base.posts.borrow().is_empty());
        assert_eq!(es.filings.borrow()[ACC].state, "done");
    }

    struct DeniedLlm;
    #[async_trait(?Send)]
    impl LlmProvider for DeniedLlm {
        async fn complete_json(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
            Err(ProviderError::Http(401))
        }
    }

    #[tokio::test]
    async fn a_failed_extraction_names_its_reason() {
        let (base, es, fl) = detected().await;
        let o = build_tick(
            poll_time() + Duration::minutes(1),
            &es,
            base.as_ref(),
            &fl,
            &DeniedLlm,
            &|_| {},
        )
        .await
        .unwrap();
        let err = "missing revenue, eps (revenue: extraction failed: http status 401)";
        assert_eq!(o[0].1, TickOutcome::Retrying(err.into()));
        assert_eq!(base.runs.borrow()[KEY].last_error.as_deref(), Some(err));
    }

    #[tokio::test]
    async fn en_first_then_zh_fills_in() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::PublishedEnOnly);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "pending"
        );
        assert_eq!(es.filings.borrow()[ACC].state, "open");
        l.zh_ok.set(true);
        let o = build_tick(
            t + Duration::minutes(5),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
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
        build_tick(poll_time(), &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        let o = build_tick(
            utc("2026-10-31T20:31:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "failed"
        );
        assert_eq!(zh_lines(&base), stored_lines(&base).zh.map(Some));
        assert_eq!(base.alerts.borrow()[0].1, "warn");
        assert_eq!(es.filings.borrow()[ACC].state, "done");
    }

    #[tokio::test]
    async fn en_failure_publishes_nothing() {
        let (base, es, fl) = detected().await;
        let l = llm(false, true);
        let o = build_tick(poll_time(), &es, base.as_ref(), &fl, &l, &|_| {})
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
            cover_key: None,
            revised_at: None,
        });
        build_tick(
            poll_time(),
            &es,
            base.as_ref(),
            &fl,
            &llm(true, true),
            &|_| {},
        )
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
        let o = build_tick(poll_time(), &es, base.as_ref(), &fl, &l, &|_| {})
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
            &|_| {},
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
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(o.len(), 1);
    }

    #[tokio::test]
    async fn pending_zh_does_not_hold_up_new_builds() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        build_tick(poll_time(), &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        let mut second = es.filings.borrow()[ACC].clone();
        second.accession = "0000320193-26-000099".into();
        second.run_key = run_key("AAPL", &second.accession);
        second.state = "open".into();
        second.accepted_at = "2026-10-30T20:40:00Z".into();
        es.insert_filing(&second).await.unwrap();
        let t = poll_time() + Duration::minutes(6);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        let keys: Vec<_> = o.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&second.run_key.as_str()), "{o:?}");
        assert!(keys.contains(&KEY), "the ZH retry runs too: {o:?}");
    }

    #[tokio::test]
    async fn zh_retries_wait_between_attempts() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        let t = poll_time();
        build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        let drafts = l.drafts.get();
        let o = build_tick(
            t + Duration::minutes(2),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert!(o.is_empty());
        assert_eq!(l.drafts.get(), drafts);
        let o = build_tick(
            t + Duration::minutes(6),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
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
        let o = build_tick(
            utc("2026-10-31T02:40:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(base.runs.borrow()[KEY].status, "failed");
        assert_eq!(base.alerts.borrow().len(), 1);
        assert_eq!(l.drafts.get(), 0);
        let o = build_tick(
            utc("2026-10-31T02:50:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
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
        build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        // Simulate a crash after EN was written: no sources, no charts, run not published.
        base.sources.borrow_mut().clear();
        es.charts.borrow_mut().clear();
        base.translations
            .borrow_mut()
            .remove(&(1, "zh".to_string()));
        es.set_filing_state(ACC, "open").await.unwrap();
        let mut r = base.runs.borrow()[KEY].clone();
        r.status = "building".into();
        r.updated_at = iso(t);
        base.runs.borrow_mut().insert(KEY.into(), r);
        let drafts = l.drafts.get();
        build_tick(
            t + Duration::minutes(20),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(l.drafts.get(), drafts, "no rebuild");
        assert_eq!(base.runs.borrow()[KEY].status, "published");
        assert_eq!(base.sources.borrow()[&1].len(), 2);
        assert_eq!(es.charts.borrow()[&1].len(), 2);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "pending"
        );
        assert_eq!(zh_lines(&base), stored_lines(&base).zh.map(Some));
    }

    #[tokio::test]
    async fn unreadable_facts_end_zh_without_blocking() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        build_tick(poll_time(), &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        let want = stored_lines(&base).zh.map(Some);
        base.facts
            .borrow_mut()
            .insert("earnings/AAPL/0000320193-26-000070.json".into(), "{".into());
        let o = build_tick(
            poll_time() + Duration::minutes(6),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(
            base.translations.borrow()[&(1, "zh".to_string())].status,
            "failed"
        );
        // The facts are unreadable: the lines come from the pending row.
        assert_eq!(zh_lines(&base), want);
        assert_eq!(es.filings.borrow()[ACC].state, "done");
    }

    #[tokio::test]
    async fn a_backlog_still_gets_three_builds() {
        // Detected promptly, but the first build only ran 7 h later (one
        // build per tick behind a queue): past every window, yet it retries
        // until its third build.
        let (base, es, fl) = detected().await;
        fl.no_exhibit.set(true);
        let l = llm(true, true);
        for (i, at) in ["2026-10-31T03:30:00Z", "2026-10-31T03:36:00Z"]
            .iter()
            .enumerate()
        {
            let o = build_tick(utc(at), &es, base.as_ref(), &fl, &l, &|_| {})
                .await
                .unwrap();
            assert!(
                matches!(o[0].1, TickOutcome::Retrying(_)),
                "build {}: {o:?}",
                i + 1
            );
        }
        let o = build_tick(
            utc("2026-10-31T03:42:00Z"),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(o[0].1, TickOutcome::Skipped);
        assert_eq!(base.runs.borrow()[KEY].attempts, 3);
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
            &|_| {},
        )
        .await
        .unwrap();
        assert!(matches!(o[0].1, TickOutcome::Retrying(_)), "{o:?}");
    }

    const FACTS_KEY: &str = "earnings/AAPL/0000320193-26-000070.json";

    fn stored_facts(base: &MemStore) -> EarningsFacts {
        serde_json::from_str(&base.facts.borrow()[FACTS_KEY]).unwrap()
    }
    fn stored_lines(base: &MemStore) -> crate::cover::CoverLines {
        earnings_lines(&stored_facts(base))
    }
    fn lines_of(base: &MemStore, lang: &str) -> [Option<String>; 2] {
        let t = base.translations.borrow()[&(1, lang.to_string())].clone();
        [t.cover_line1, t.cover_line2]
    }
    fn zh_lines(base: &MemStore) -> [Option<String>; 2] {
        lines_of(base, "zh")
    }

    #[tokio::test]
    async fn earnings_build_writes_cover_key_and_lines() {
        let (base, es, fl) = detected().await;
        let l = llm(true, true);
        let logs = RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &log)
            .await
            .unwrap();
        assert_eq!(o, vec![(KEY.to_string(), TickOutcome::Published)]);
        let key = "covers/aapl-q4-fy2026-earnings.svg";
        assert_eq!(base.posts.borrow()[0].cover_key.as_deref(), Some(key));
        let facts = stored_facts(&base);
        assert_eq!(
            base.bytes.borrow()[key],
            (
                earnings_graphic(&facts).unwrap().into_bytes(),
                "image/svg+xml".to_string()
            )
        );
        assert_eq!(*logs.borrow(), ["cover=ok"]);
        let lines = earnings_lines(&facts);
        assert!(lines.en[0].starts_with("Revenue $"), "{lines:?}");
        assert_eq!(lines_of(&base, "en"), lines.en.map(Some));
        assert_eq!(lines_of(&base, "zh"), lines.zh.map(Some));
        // Earnings posts never move the index line.
        assert!(base.kv.borrow().is_empty());
    }

    #[tokio::test]
    async fn earnings_zh_retry_keeps_cover_lines() {
        let (base, es, fl) = detected().await;
        let l = llm(true, false);
        let t = poll_time() + Duration::minutes(1);
        build_tick(t, &es, base.as_ref(), &fl, &l, &|_| {})
            .await
            .unwrap();
        let want = stored_lines(&base).zh.map(Some);
        assert_eq!(zh_lines(&base), want);
        // A failed retry rewrites the pending row; it keeps the lines.
        let o = build_tick(
            t + Duration::minutes(5),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert!(matches!(o[0].1, TickOutcome::Retrying(_)), "{o:?}");
        assert_eq!(zh_lines(&base), want);
        l.zh_ok.set(true);
        let o = build_tick(
            t + Duration::minutes(11),
            &es,
            base.as_ref(),
            &fl,
            &l,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(o[0].1, TickOutcome::Published);
        assert_eq!(zh_lines(&base), want);
    }

    #[tokio::test]
    async fn earnings_build_publishes_without_cover_when_the_facts_give_no_graphic() {
        let (base, es, fl) = detected().await;
        fl.no_trend.set(true);
        let l = llm(true, true);
        let logs = RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &log)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::Published);
        assert_eq!(earnings_graphic(&stored_facts(&base)), None);
        assert_eq!(base.posts.borrow()[0].cover_key, None);
        assert!(base
            .bytes
            .borrow()
            .keys()
            .all(|k| !k.starts_with("covers/")));
        assert_eq!(*logs.borrow(), ["cover=none"]);
    }

    #[tokio::test]
    async fn earnings_build_publishes_when_the_cover_write_fails() {
        let (base, es, fl) = detected().await;
        base.fail_put_bytes.set(true);
        let l = llm(true, true);
        let logs = RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let t = poll_time() + Duration::minutes(1);
        let o = build_tick(t, &es, base.as_ref(), &fl, &l, &log)
            .await
            .unwrap();
        assert_eq!(o[0].1, TickOutcome::Published);
        assert_eq!(base.posts.borrow()[0].cover_key, None);
        assert!(base.bytes.borrow().is_empty());
        assert_eq!(lines_of(&base, "en"), stored_lines(&base).en.map(Some));
        // The write error reaches the log on its own line, never the run or the facts.
        assert_eq!(
            *logs.borrow(),
            ["cover=err", "cover write failed: r2: put_bytes failed"]
        );
        assert!(base.runs.borrow().values().all(|r| r.last_error.is_none()));
        assert!(base
            .facts
            .borrow()
            .values()
            .all(|j| !j.contains("put_bytes")));
    }
}
