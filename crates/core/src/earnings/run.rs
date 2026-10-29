//! The earnings run state machine. Every one-minute tick in the polling
//! window calls `poll_tick` (detect new 8-K Item 2.02 filings for one batch
//! of the watchlist) and `build_tick` (write at most one queued filing, or
//! retry its Chinese draft). The run key `earnings:{TICKER}:{accession}` is
//! the idempotency key: re-running never creates a second post.

use chrono::{DateTime, Duration, SecondsFormat, Utc};

use crate::clock::et;
use crate::earnings::charts::{charts, r2_key};
use crate::earnings::detect::{earnings_filings, ticker_matches};
use crate::earnings::draft::{slug, write, CONTENT_TYPE, TEMPLATE_ID};
use crate::earnings::extract::{extract, Extracted};
use crate::earnings::facts::{missing_required, EarningsFacts, Metrics};
use crate::earnings::store::{ChartRow, EarningsStore, FilingRow};
use crate::earnings::xbrl::trend;
use crate::earnings::{find, in_window, poll_batch};
use crate::edgar::{exhibit_991, filing_url, Filings};
use crate::facts::{Field, NewsReason};
use crate::numbers::Lang;
use crate::providers::{LlmProvider, ProviderError};
use crate::run::TickOutcome;
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollOutcome {
    /// Companies whose submissions were read.
    pub checked: usize,
    /// Run keys of newly detected filings.
    pub detected: Vec<String>,
    /// Per-company problems (provider errors, ticker mismatches).
    pub errors: Vec<String>,
    /// SEC answered 403/429 (blocked or rate-limited) and the batch stopped.
    pub throttled: bool,
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

/// Polls this minute's watchlist batch for new earnings filings. A provider
/// error skips that company; 403/429 (SEC throttling) stops the batch. A
/// store error aborts the tick.
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
    for entry in poll_batch(now) {
        let sub = match filings.submissions(entry.cik).await {
            Ok(s) => s,
            Err(e) => {
                out.errors.push(format!("{}: {e}", entry.ticker));
                if matches!(e, ProviderError::Http(403 | 429)) {
                    out.throttled = true;
                    break;
                }
                continue;
            }
        };
        out.checked += 1;
        if !ticker_matches(&sub, &entry) {
            out.errors.push(format!(
                "{}: CIK {} lists tickers {:?}; fix data/watchlist.tsv",
                entry.ticker, entry.cik, sub.tickers
            ));
            continue;
        }
        for f in earnings_filings(&sub, now) {
            let key = run_key(entry.ticker, &f.accession);
            let row = FilingRow {
                accession: f.accession.clone(),
                cik: entry.cik,
                ticker: entry.ticker.to_string(),
                form: f.form.clone(),
                items: f.items.clone(),
                filing_date: f.filing_date.clone(),
                report_date: f.report_date.clone(),
                accepted_at: iso(f.accepted_at),
                primary_doc: f.primary_doc.clone(),
                run_key: key.clone(),
                state: "open".into(),
                detected_at: iso(now),
            };
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
    let trend = match filings.company_facts(f.cik).await {
        Ok(cf) => Field::Value(trend(&cf)),
        Err(e) => Field::Missing {
            reason: format!("companyfacts: {e}"),
        },
    };
    let accepted_at = parse_iso(&f.accepted_at).ok_or("bad accepted_at")?;
    let filing_date = f
        .filing_date
        .parse()
        .map_err(|e| format!("bad filing_date: {e}"))?;
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
    use crate::edgar::tests::{COMPANYFACTS, EX991, INDEX, SUBMISSIONS};
    use crate::edgar::{
        html_to_text, parse_filing_index, parse_submissions, CompanyFacts, FilingDoc, Submissions,
    };
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

    /// Serves the AAPL fixtures for CIK 320193; every other CIK has no
    /// filings, or the configured error.
    #[derive(Default)]
    struct FakeFilings {
        sub_calls: RefCell<Vec<u64>>,
        errors: HashMap<u64, ProviderError>,
        no_exhibit: Cell<bool>,
    }
    #[async_trait(?Send)]
    impl Filings for FakeFilings {
        async fn submissions(&self, cik: u64) -> Result<Submissions, ProviderError> {
            self.sub_calls.borrow_mut().push(cik);
            if let Some(e) = self.errors.get(&cik) {
                return Err(e.clone());
            }
            if cik == 320193 {
                return parse_submissions(SUBMISSIONS);
            }
            let e = crate::earnings::by_cik(cik).unwrap();
            Ok(Submissions {
                cik,
                name: e.name.into(),
                tickers: vec![e.ticker.replace('.', "-")],
                recent: vec![],
            })
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
        async fn company_facts(&self, _: u64) -> Result<CompanyFacts, ProviderError> {
            Ok(serde_json::from_str(COMPANYFACTS).unwrap())
        }
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

    /// A minute (ET 16:45 on Fri 2026-10-30) whose batch contains AAPL.
    fn poll_time() -> DateTime<Utc> {
        let mut t = utc("2026-10-30T20:45:00Z");
        while !crate::earnings::poll_batch(t)
            .iter()
            .any(|e| e.ticker == "AAPL")
        {
            t += Duration::minutes(1);
        }
        t
    }

    #[tokio::test]
    async fn poll_detects_once() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let t = poll_time();
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert_eq!(o.detected, vec![KEY.to_string()]);
        assert!(o.checked <= crate::earnings::watchlist::BATCH);
        assert_eq!(fl.sub_calls.borrow().len(), o.checked);
        assert_eq!(base.runs.borrow()[KEY].status, "pending");
        assert_eq!(base.runs.borrow()[KEY].trading_date, "2026-10-30");
        // Next cycle: same accession, nothing new.
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
    async fn poll_outside_window_does_nothing() {
        let (base, es) = stores();
        let fl = FakeFilings::default();
        let o = poll_tick(utc("2026-10-31T15:00:00Z"), &es, base.as_ref(), &fl)
            .await
            .unwrap();
        assert_eq!(o, PollOutcome::default());
        assert!(fl.sub_calls.borrow().is_empty());
    }

    #[tokio::test]
    async fn one_company_error_does_not_stop_the_batch_but_throttling_does() {
        let (base, es) = stores();
        let t = poll_time();
        let batch = crate::earnings::poll_batch(t);
        let other = batch.iter().find(|e| e.ticker != "AAPL").unwrap().cik;
        let mut fl = FakeFilings::default();
        fl.errors
            .insert(other, ProviderError::Network("reset".into()));
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert_eq!(o.errors.len(), 1);
        assert_eq!(o.checked, batch.len() - 1);
        assert_eq!(o.detected.len(), 1);

        let (base, es) = stores();
        let first = batch[0].cik;
        let mut fl = FakeFilings::default();
        fl.errors.insert(first, ProviderError::Http(429));
        let o = poll_tick(t, &es, base.as_ref(), &fl).await.unwrap();
        assert_eq!(fl.sub_calls.borrow().len(), 1, "stop after SEC throttles");
        assert!(o.throttled);
        assert_eq!(o.checked, 0);
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
