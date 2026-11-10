//! The daily events check (`events:check`, queued by an earnings poll):
//! loads the snapshot and the closures into `macro_events`, re-reads the
//! FOMC calendar and the BLS jobs and CPI schedules (3 fetches) for moved
//! or postponed dates, and marks events released once their release page
//! answers 200. Between daily checks, a poll queues the step again just
//! after an event's time, for the released check alone.
//!
//! Free plan per run: 3 schedule fetches plus at most `MAX_RELEASE_CHECKS`
//! release fetches; 5 D1 queries (marker read and write, one ranged read,
//! one batch write, the loaded marker).

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, Utc};

use super::parse::{parse_bls, parse_fomc, FomcPage, BLS_CPI_URL, BLS_JOBS_URL, FOMC_URL};
use super::store::{EventRow, EventStore};
use super::{planned, release_url_for, Planned};
use crate::clock::{et, trading_day, DayKind};
use crate::earnings::store::EarningsStore;
use crate::providers::Http;

/// BLS answers 403 to browser and default User-Agents and serves one that
/// names a contact.
pub const USER_AGENT: &str = "PreBell (https://prebell.site; support@prebell.site)";
/// A schedule page larger than this is refused (the FOMC calendar is ~170 KB).
pub const MAX_PAGE_BYTES: usize = 1 << 20;
/// Bytes of a release page read for the released check: the status, the
/// page title and, for CPI and jobs, the opening summary the headline
/// figures come from (`figures`). BLS release pages are over 1 MB; on the
/// 2026-11 pages the summary ends at byte 36,066 (CPI) and the jobs
/// headline sentence at 36,100, after ~34 KB of site navigation.
pub const RELEASE_PREFIX_BYTES: usize = 48 * 1024;
/// An event this many trading days ahead or nearer, re-seen on its page, is
/// `confirmed`.
pub const CONFIRM_TRADING_DAYS: usize = 10;
/// Consecutive checks an event must be missing from a page that otherwise
/// parsed before it is `postponed`: one bad parse never cancels an event.
pub const POSTPONE_AFTER_MISSES: i64 = 2;
/// Rows kept from this many days back (the calendar's current week and the
/// one before) to this many ahead.
pub const PAST_DAYS: i64 = 31;
pub const AHEAD_DAYS: i64 = 400;
/// A poll queues the released check while an event started within this.
pub const RELEASE_WINDOW_MINUTES: i64 = 120;
/// The daily check also catches up releases this many days old (an event
/// moved to a time the poll did not know).
pub const RELEASE_LOOKBACK_DAYS: i64 = 3;
/// Release-page fetches per run at most.
pub const MAX_RELEASE_CHECKS: usize = 4;

const KV_CHECKED: &str = "events_checked_on";
/// Set once a daily check has loaded the table (not by the pre-0008 skip).
const KV_LOADED: &str = "events_loaded_on";

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// What the live check read: each page, or `None` when it failed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    pub fomc: Option<FomcPage>,
    pub jobs: Option<Vec<Planned>>,
    pub cpi: Option<Vec<Planned>>,
}

/// The page lists a date on or after `date`.
fn listed_through(page: Option<&[Planned]>, date: NaiveDate) -> bool {
    page.is_some_and(|rows| rows.iter().any(|p| p.et_date >= date))
}

impl Live {
    fn sighting(&self, id: &str) -> Option<&Planned> {
        let fomc = self.fomc.as_ref().map(|p| p.meetings.as_slice());
        [fomc, self.jobs.as_deref(), self.cpi.as_deref()]
            .into_iter()
            .flatten()
            .flatten()
            .find(|p| p.id == id)
    }

    /// Whether a page that parsed would list `series` on `date` if it were
    /// still scheduled: only future days (BLS drops past releases), for FOMC
    /// only the years the page has panels for, for BLS only up to the last
    /// date the page lists.
    fn covers(&self, series: &str, date: NaiveDate, today: NaiveDate) -> bool {
        date > today
            && match series {
                "fomc" => self
                    .fomc
                    .as_ref()
                    .is_some_and(|p| p.years.contains(&date.year())),
                "jobs" => listed_through(self.jobs.as_deref(), date),
                "cpi" => listed_through(self.cpi.as_deref(), date),
                _ => false,
            }
    }

    fn all(&self) -> impl Iterator<Item = &Planned> {
        let fomc = self.fomc.as_ref().map(|p| p.meetings.as_slice());
        [fomc, self.jobs.as_deref(), self.cpi.as_deref()]
            .into_iter()
            .flatten()
            .flatten()
    }

    /// `fomc=ok jobs=ok cpi=err`, for the log.
    pub fn summary(&self) -> String {
        let s = |ok: bool| if ok { "ok" } else { "err" };
        format!(
            "fomc={} jobs={} cpi={}",
            s(self.fomc.is_some()),
            s(self.jobs.is_some()),
            s(self.cpi.is_some())
        )
    }
}

/// Kinds whose dates the daily check re-reads from an official page.
fn read_live(series: &str) -> bool {
    matches!(series, "fomc" | "jobs" | "cpi")
}

/// The `n`th trading day after `today`.
fn trading_days_after(today: NaiveDate, n: usize) -> NaiveDate {
    let mut d = today;
    let mut left = n;
    while left > 0 {
        d += Duration::days(1);
        if trading_day(d) != DayKind::Closed {
            left -= 1;
        }
    }
    d
}

pub fn new_row(p: &Planned, now: DateTime<Utc>) -> EventRow {
    EventRow {
        id: p.id.clone(),
        series: p.series.clone(),
        ref_period: p.ref_period.clone(),
        variant: p.variant.clone(),
        starts_at: p.starts_at.map(iso),
        et_date: ymd(p.et_date),
        tier: i64::from(p.tier),
        status: "scheduled".into(),
        proof_url: p.proof_url.clone(),
        release_url: None,
        moved_from: None,
        missing_count: 0,
        first_seen_at: iso(now),
        checked_at: iso(now),
        updated_at: iso(now),
    }
}

/// Same event, ignoring the bookkeeping times.
fn same(a: &EventRow, b: &EventRow) -> bool {
    let strip = |r: &EventRow| EventRow {
        checked_at: String::new(),
        updated_at: String::new(),
        ..r.clone()
    };
    strip(a) == strip(b)
}

/// The rows to write after a live check: `planned` (snapshot and closures)
/// and `live` (the pages; `None` = failed) against `existing`. A live date
/// wins over the snapshot's. A row whose date changes becomes `moved`
/// (`moved_from` = the old date); one missing on two checks of a page that
/// parsed becomes `postponed`; one re-seen within `CONFIRM_TRADING_DAYS`
/// becomes `confirmed` (FOMC: only the next meeting, as the Fed confirms a
/// date at the meeting before it). Released rows never change.
pub fn reconcile(
    now: DateTime<Utc>,
    planned: &[Planned],
    live: &Live,
    existing: &[EventRow],
) -> Vec<EventRow> {
    let today = et(now).date_naive();
    let horizon = trading_days_after(today, CONFIRM_TRADING_DAYS);
    let (from, to) = (
        today - Duration::days(PAST_DAYS),
        today + Duration::days(AHEAD_DAYS),
    );
    let old: HashMap<&str, &EventRow> = existing.iter().map(|r| (r.id.as_str(), r)).collect();

    // Planned first, then what only the pages list (a release added since
    // the snapshot), within the window.
    let mut ids: Vec<&str> = planned.iter().map(|p| p.id.as_str()).collect();
    for p in live.all().filter(|p| p.et_date >= from && p.et_date <= to) {
        if !ids.contains(&p.id.as_str()) {
            ids.push(&p.id);
        }
    }
    let plan: HashMap<&str, &Planned> = planned.iter().map(|p| (p.id.as_str(), p)).collect();
    let next_fomc = ids
        .iter()
        .filter_map(|id| {
            live.sighting(id)
                .map(|p| (p.series.as_str(), p.et_date))
                .or_else(|| {
                    let r = old.get(id)?;
                    Some((r.series.as_str(), r.et_date.parse().ok()?))
                })
                .or_else(|| plan.get(id).map(|p| (p.series.as_str(), p.et_date)))
        })
        .filter(|(series, date)| *series == "fomc" && *date >= today)
        .map(|(_, date)| date)
        .min();

    let mut out = Vec::new();
    for id in ids {
        let sight = live.sighting(id);
        let prev = old.get(id).copied();
        if prev.is_some_and(|r| r.status == "released") {
            continue;
        }
        // Where the date comes from: a sighting on its page; else the
        // snapshot, for kinds no page is re-read for (minutes, elections,
        // closures) and for a page-read row a sighting never moved (so a
        // snapshot rerun still moves it while its page is unreachable). A
        // row a sighting moved keeps its date when unsighted (its page
        // failed, or BLS dropped it once past), never the snapshot's.
        let src = match (sight, prev) {
            (Some(s), _) => Some(s),
            (None, None) => plan.get(id).copied(),
            (None, Some(p)) => plan.get(id).copied().filter(|pl| {
                !read_live(&pl.series) || (p.status != "moved" && p.moved_from.is_none())
            }),
        };
        let mut r = match (prev, src) {
            (Some(p), _) => p.clone(),
            (None, Some(s)) => new_row(s, now),
            (None, None) => continue,
        };
        if let Some(src) = src {
            let new_date = ymd(src.et_date);
            if let Some(p) = prev.filter(|p| p.et_date != new_date) {
                r.status = "moved".into();
                r.moved_from = Some(p.et_date.clone());
            }
            r.series = src.series.clone();
            r.ref_period = src.ref_period.clone();
            r.variant = src.variant.clone();
            r.starts_at = src.starts_at.map(iso);
            r.et_date = new_date;
            r.tier = i64::from(src.tier);
            r.proof_url = src.proof_url.clone();
        }
        let Ok(date) = r.et_date.parse::<NaiveDate>() else {
            continue;
        };
        if sight.is_some() {
            r.missing_count = 0;
            if matches!(r.status.as_str(), "scheduled" | "confirmed" | "postponed") {
                let next = r.series != "fomc" || next_fomc == Some(date);
                r.status = if date <= horizon && next {
                    "confirmed".into()
                } else {
                    "scheduled".into()
                };
            }
        } else if live.covers(&r.series, date, today) {
            r.missing_count += 1;
            if r.missing_count >= POSTPONE_AFTER_MISSES {
                r.status = "postponed".into();
            }
        }
        if prev.is_none_or(|p| !same(p, &r)) {
            r.checked_at = iso(now);
            r.updated_at = iso(now);
            out.push(r);
        }
    }
    out
}

fn ok<T>(r: Result<T, String>, errors: &mut Vec<String>) -> Option<T> {
    r.map_err(|e| errors.push(e)).ok()
}

/// Fetches each schedule page with the contact User-Agent; a failed or
/// unparsable page is `None` and an entry in `errors`.
pub async fn fetch_live(http: &dyn Http, errors: &mut Vec<String>) -> Live {
    async fn page(http: &dyn Http, url: &str, errors: &mut Vec<String>) -> Option<String> {
        match http
            .get_capped(url, &[("User-Agent", USER_AGENT)], MAX_PAGE_BYTES)
            .await
        {
            Ok(r) if r.status == 200 => Some(r.body),
            Ok(r) => {
                errors.push(format!("{url}: http {}", r.status));
                None
            }
            Err(e) => {
                errors.push(format!("{url}: {e}"));
                None
            }
        }
    }
    let fomc = page(http, FOMC_URL, errors).await.map(|h| parse_fomc(&h));
    let jobs = page(http, BLS_JOBS_URL, errors)
        .await
        .map(|h| parse_bls(&h, "jobs", BLS_JOBS_URL));
    let cpi = page(http, BLS_CPI_URL, errors)
        .await
        .map(|h| parse_bls(&h, "cpi", BLS_CPI_URL));
    Live {
        fomc: fomc.and_then(|r| ok(r, errors)),
        jobs: jobs.and_then(|r| ok(r, errors)),
        cpi: cpi.and_then(|r| ok(r, errors)),
    }
}

/// Text the first `RELEASE_PREFIX_BYTES` of a real release page carry (its
/// title or `og:title`).
fn release_marker(series: &str) -> Option<&'static str> {
    match series {
        // Title forms: "Consumer Price Index" alone is also in the BLS site
        // navigation (its 404 page too).
        "jobs" => Some("Employment Situation News Release"),
        "cpi" => Some("Consumer Price Index News Release"),
        "fomc" => Some("issues FOMC statement"),
        "fomc_minutes" => Some("FOMC Minutes, "),
        _ => None,
    }
}

/// The first `RELEASE_PREFIX_BYTES` of `url` (one GET with the contact
/// User-Agent) when it answers 200 and carries `series`' release title. A
/// 200 page that is not the release (a bot wall) does not count.
pub(crate) async fn release_page(http: &dyn Http, url: &str, series: &str) -> Option<String> {
    let marker = release_marker(series)?;
    http.get_prefix(url, &[("User-Agent", USER_AGENT)], RELEASE_PREFIX_BYTES)
        .await
        .ok()
        .filter(|resp| resp.status == 200 && resp.body.contains(marker))
        .map(|resp| resp.body)
}

/// Rows whose release page is up: for each row not yet released that
/// started at most `RELEASE_LOOKBACK_DAYS` ago and has a release URL, one
/// GET of the page's first `RELEASE_PREFIX_BYTES` (at most `max`); a 200
/// whose prefix carries the release's title marks it released with that URL.
pub async fn check_releases(
    now: DateTime<Utc>,
    rows: &[EventRow],
    http: &dyn Http,
    max: usize,
) -> Vec<EventRow> {
    check_release_pages(now, rows, http, max)
        .await
        .0
        .into_iter()
        .map(|(r, _)| r)
        .collect()
}

/// `check_releases` with each released row's page prefix, and the number
/// of fetches made.
pub(crate) async fn check_release_pages(
    now: DateTime<Utc>,
    rows: &[EventRow],
    http: &dyn Http,
    max: usize,
) -> (Vec<(EventRow, String)>, usize) {
    let mut out = Vec::new();
    let mut tries = 0;
    for r in rows {
        if r.status == "released" || tries >= max {
            continue;
        }
        let Some(start) = r
            .starts_at
            .as_deref()
            .and_then(|t| t.parse::<DateTime<Utc>>().ok())
        else {
            continue;
        };
        if start > now || now - start > Duration::days(RELEASE_LOOKBACK_DAYS) {
            continue;
        }
        let Ok(date) = r.et_date.parse::<NaiveDate>() else {
            continue;
        };
        let Some(url) = release_url_for(&r.series, date, r.ref_period.as_deref()) else {
            continue;
        };
        if release_marker(&r.series).is_none() {
            continue;
        }
        tries += 1;
        if let Some(page) = release_page(http, &url, &r.series).await {
            let mut r = r.clone();
            r.status = "released".into();
            r.release_url = Some(url);
            r.moved_from = None;
            r.checked_at = iso(now);
            r.updated_at = iso(now);
            out.push((r, page));
        }
    }
    (out, tries)
}

/// The error `check_tick` returns before migration 0008 is applied.
pub fn missing_table(err: &str) -> bool {
    err.contains("no such table: macro_events")
}

/// Records the daily check as done for `now`'s ET date (the Worker also
/// calls it when the table is missing, so polls stop queuing the check).
pub async fn mark_checked(es: &dyn EarningsStore, now: DateTime<Utc>) -> Result<(), String> {
    es.kv_set(KV_CHECKED, &ymd(et(now).date_naive()), &iso(now))
        .await
}

/// The ET date a daily check last loaded the table, `None` if none ever did.
pub async fn last_loaded(es: &dyn EarningsStore) -> Result<Option<String>, String> {
    es.kv_get(KV_LOADED).await
}

/// Whether the daily check already ran on `now`'s ET date.
pub async fn checked_today(es: &dyn EarningsStore, now: DateTime<Utc>) -> Result<bool, String> {
    let today = ymd(et(now).date_naive());
    Ok(es.kv_get(KV_CHECKED).await?.as_deref() == Some(today.as_str()))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckOutcome {
    /// The daily part (pages and snapshot) ran; otherwise releases only.
    pub daily: bool,
    /// `fomc=ok jobs=ok cpi=ok` for a daily run.
    pub pages: String,
    pub written: usize,
    pub released: Vec<String>,
    pub moved: Vec<String>,
    pub postponed: Vec<String>,
    pub errors: Vec<String>,
}

/// One `events:check` message: the daily part once per ET day, then the
/// released check.
pub async fn check_tick(
    now: DateTime<Utc>,
    ev: &dyn EventStore,
    es: &dyn EarningsStore,
    http: &dyn Http,
) -> Result<CheckOutcome, String> {
    let mut out = CheckOutcome::default();
    let today = et(now).date_naive();
    let from = today - Duration::days(PAST_DAYS);
    out.daily = !checked_today(es, now).await?;
    let existing = ev.events_between(&ymd(from), "9999-12-31").await?;
    if out.daily {
        // Marked before the pages: a failing page must not be fetched again
        // every poll. After the read, so a D1 failure retries the day.
        mark_checked(es, now).await?;
    }
    let mut changed: BTreeMap<String, EventRow> = BTreeMap::new();
    if out.daily {
        let live = fetch_live(http, &mut out.errors).await;
        out.pages = live.summary();
        // Elections are listed years ahead; the official schedules for the window.
        let last = super::snapshot()
            .iter()
            .map(|p| p.et_date)
            .max()
            .unwrap_or(today);
        let to = last.max(today + Duration::days(AHEAD_DAYS));
        for r in reconcile(now, &planned(from, to), &live, &existing) {
            changed.insert(r.id.clone(), r);
        }
    }
    let current: Vec<EventRow> = existing
        .iter()
        .map(|r| changed.get(&r.id).cloned().unwrap_or_else(|| r.clone()))
        .chain(
            changed
                .values()
                .filter(|c| !existing.iter().any(|r| r.id == c.id))
                .cloned(),
        )
        .collect();
    for r in check_releases(now, &current, http, MAX_RELEASE_CHECKS).await {
        changed.insert(r.id.clone(), r);
    }
    for r in changed.values() {
        match r.status.as_str() {
            "released" => out.released.push(r.id.clone()),
            "moved" => out.moved.push(r.id.clone()),
            "postponed" => out.postponed.push(r.id.clone()),
            _ => {}
        }
    }
    out.written = changed.len();
    if !changed.is_empty() {
        let rows: Vec<EventRow> = changed.into_values().collect();
        ev.write_events(&rows).await?;
    }
    if out.daily {
        es.kv_set(KV_LOADED, &ymd(today), &iso(now)).await?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::store::MemEarningsStore;
    use crate::events::store::MemEventStore;
    use crate::providers::{HttpResp, ProviderError};
    use async_trait::async_trait;
    use std::cell::RefCell;

    const FOMC: &str = include_str!("../../tests/fixtures/fed_fomccalendars.htm");
    const JOBS: &str = include_str!("../../tests/fixtures/bls_empsit_schedule.htm");
    const CPI: &str = include_str!("../../tests/fixtures/bls_cpi_schedule.htm");

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    /// Answers each url from `pages` (200) or 404; records every request
    /// with its User-Agent.
    #[derive(Default)]
    struct FakeHttp {
        pages: HashMap<String, (u16, String)>,
        seen: RefCell<Vec<(String, String)>>,
    }
    impl FakeHttp {
        fn with(pages: &[(&str, u16, &str)]) -> Self {
            Self {
                pages: pages
                    .iter()
                    .map(|(u, s, b)| (u.to_string(), (*s, b.to_string())))
                    .collect(),
                ..Default::default()
            }
        }
        fn official() -> Self {
            Self::with(&[
                (FOMC_URL, 200, FOMC),
                (BLS_JOBS_URL, 200, JOBS),
                (BLS_CPI_URL, 200, CPI),
            ])
        }
        fn urls(&self) -> Vec<String> {
            self.seen.borrow().iter().map(|(u, _)| u.clone()).collect()
        }
    }
    #[async_trait(?Send)]
    impl Http for FakeHttp {
        async fn get(&self, url: &str, h: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            let ua = h
                .iter()
                .find(|(k, _)| *k == "User-Agent")
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            self.seen.borrow_mut().push((url.to_string(), ua));
            let (status, body) = self.pages.get(url).cloned().unwrap_or((404, String::new()));
            Ok(HttpResp {
                status,
                body,
                set_cookie: None,
            })
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            Err(ProviderError::Http(405))
        }
    }

    fn plan(id: &str, series: &str, date: &str, start: Option<&str>) -> Planned {
        Planned {
            id: id.into(),
            series: series.into(),
            ref_period: Some(date[..7].to_string()),
            variant: None,
            starts_at: start.map(utc),
            et_date: d(date),
            tier: 1,
            proof_url: "https://www.bls.gov/schedule/news_release/cpi.htm".into(),
        }
    }
    fn live_cpi(rows: Vec<Planned>) -> Live {
        Live {
            cpi: Some(rows),
            ..Default::default()
        }
    }
    const NOW: &str = "2026-11-06T11:00:00Z";

    #[test]
    fn first_run_inserts_scheduled_rows_and_confirms_near_sighted_ones() {
        let p = vec![
            plan(
                "bls:cpi:2026-10",
                "cpi",
                "2026-11-10",
                Some("2026-11-10T13:30:00Z"),
            ),
            plan(
                "bls:cpi:2027-04",
                "cpi",
                "2027-05-12",
                Some("2027-05-12T12:30:00Z"),
            ),
        ];
        let out = reconcile(utc(NOW), &p, &live_cpi(p.clone()), &[]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].status, "confirmed");
        assert_eq!(out[1].status, "scheduled");
        // Nothing changed: nothing to write.
        assert!(reconcile(utc(NOW), &p, &live_cpi(p.clone()), &out).is_empty());
    }

    #[test]
    fn a_live_date_moves_the_row_and_keeps_where_it_was() {
        let p = vec![plan(
            "bls:cpi:2026-10",
            "cpi",
            "2026-11-10",
            Some("2026-11-10T13:30:00Z"),
        )];
        let first = reconcile(utc(NOW), &p, &live_cpi(p.clone()), &[]);
        let moved = vec![plan(
            "bls:cpi:2026-10",
            "cpi",
            "2026-11-13",
            Some("2026-11-13T13:30:00Z"),
        )];
        let out = reconcile(utc(NOW), &p, &live_cpi(moved), &first);
        assert_eq!(out[0].status, "moved");
        assert_eq!(out[0].et_date, "2026-11-13");
        assert_eq!(out[0].moved_from.as_deref(), Some("2026-11-10"));
        assert_eq!(out[0].starts_at.as_deref(), Some("2026-11-13T13:30:00Z"));
    }

    #[test]
    fn missing_twice_from_a_parsed_page_is_postponed_and_a_failed_page_changes_nothing() {
        let p = vec![
            plan(
                "bls:cpi:2026-10",
                "cpi",
                "2026-11-10",
                Some("2026-11-10T13:30:00Z"),
            ),
            plan(
                "bls:cpi:2026-11",
                "cpi",
                "2026-12-10",
                Some("2026-12-10T13:30:00Z"),
            ),
        ];
        let rows = reconcile(utc(NOW), &p, &live_cpi(p.clone()), &[]);
        // The page parsed but lists only December (a shutdown pulled November).
        let only_dec = live_cpi(vec![p[1].clone()]);
        let once = reconcile(utc(NOW), &p, &only_dec, &rows);
        assert_eq!(once.len(), 1);
        assert_eq!(
            (once[0].missing_count, once[0].status.as_str()),
            (1, "confirmed")
        );
        let rows: Vec<EventRow> = vec![once[0].clone(), rows[1].clone()];
        let twice = reconcile(utc(NOW), &p, &only_dec, &rows);
        assert_eq!(
            (twice[0].missing_count, twice[0].status.as_str()),
            (2, "postponed")
        );
        // A failed page (None) counts nothing.
        assert!(reconcile(utc(NOW), &p, &Live::default(), &rows).is_empty());
        // Seen again on its day: back to confirmed.
        let rows = vec![twice[0].clone(), rows[1].clone()];
        let back = reconcile(utc(NOW), &p, &live_cpi(p.clone()), &rows);
        assert_eq!(
            (back[0].missing_count, back[0].status.as_str()),
            (0, "confirmed")
        );
    }

    #[test]
    fn release_day_and_past_events_are_never_counted_missing() {
        let p = vec![plan(
            "bls:cpi:2026-10",
            "cpi",
            "2026-11-06",
            Some("2026-11-06T13:30:00Z"),
        )];
        let rows = reconcile(utc(NOW), &p, &live_cpi(p.clone()), &[]);
        let empty_page = live_cpi(vec![plan("bls:cpi:2026-11", "cpi", "2026-12-10", None)]);
        let out = reconcile(utc(NOW), &p, &empty_page, &rows);
        assert!(out.iter().all(|r| r.id != "bls:cpi:2026-10"), "{out:?}");
    }

    #[test]
    fn released_rows_never_change() {
        let p = vec![plan(
            "bls:cpi:2026-10",
            "cpi",
            "2026-11-10",
            Some("2026-11-10T13:30:00Z"),
        )];
        let mut rows = reconcile(utc(NOW), &p, &live_cpi(p.clone()), &[]);
        rows[0].status = "released".into();
        let moved = vec![plan("bls:cpi:2026-10", "cpi", "2026-11-13", None)];
        assert!(reconcile(utc(NOW), &p, &live_cpi(moved), &rows).is_empty());
    }

    #[test]
    fn only_the_next_fomc_meeting_is_confirmed() {
        let page = parse_fomc(FOMC).unwrap();
        let p = page.meetings.clone();
        let live = Live {
            fomc: Some(page),
            ..Default::default()
        };
        // 2026-12-02: the December meeting is within 10 trading days and next.
        let out = reconcile(utc("2026-12-02T15:00:00Z"), &p, &live, &[]);
        let st = |id: &str| out.iter().find(|r| r.id == id).unwrap().status.clone();
        assert_eq!(st("fed:fomc:2026-12"), "confirmed");
        assert_eq!(st("fed:fomc:2027-01"), "scheduled");
    }

    #[test]
    fn pages_list_a_release_the_snapshot_lacks() {
        let live = live_cpi(vec![plan("bls:cpi:2026-10", "cpi", "2026-11-10", None)]);
        let out = reconcile(utc(NOW), &[], &live, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "bls:cpi:2026-10");
    }

    #[tokio::test]
    async fn releases_are_marked_from_their_page() {
        let p = [
            plan(
                "bls:cpi:2026-10",
                "cpi",
                "2026-11-10",
                Some("2026-11-10T13:30:00Z"),
            ),
            plan(
                "bls:jobs:2026-11",
                "jobs",
                "2026-12-04",
                Some("2026-12-04T13:30:00Z"),
            ),
        ];
        let rows: Vec<EventRow> = p.iter().map(|p| new_row(p, utc(NOW))).collect();
        let cpi_release = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
        // The real page is 1.37 MB: only its first bytes are read.
        let page = format!(
            "<title> Consumer Price Index News Release - 2026 M10 Results</title>{}",
            "x".repeat(1_400_000)
        );
        let http = FakeHttp::with(&[(cpi_release, 200, &page)]);
        // 8:31 ET on CPI day: the CPI page is up; jobs is in the future.
        let out = check_releases(utc("2026-11-10T13:31:00Z"), &rows, &http, 4).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, "released");
        assert_eq!(out[0].release_url.as_deref(), Some(cpi_release));
        assert_eq!(http.urls(), [cpi_release]);
        assert_eq!(http.seen.borrow()[0].1, USER_AGENT);
        // Before 8:30 nothing is fetched; a 404 marks nothing.
        let http = FakeHttp::default();
        assert!(check_releases(utc("2026-11-10T13:29:00Z"), &rows, &http, 4)
            .await
            .is_empty());
        assert!(http.urls().is_empty());
        assert!(check_releases(utc("2026-11-10T13:35:00Z"), &rows, &http, 4)
            .await
            .is_empty());
        assert_eq!(http.urls().len(), 1);
        // A soft error page that only names the index in its navigation.
        let nav = FakeHttp::with(&[(
            cpi_release,
            200,
            "<nav>Consumer Price Index</nav> Page Not Found",
        )]);
        assert!(check_releases(utc("2026-11-10T13:31:00Z"), &rows, &nav, 4)
            .await
            .is_empty());
        // A 200 that is not the release (a bot wall) marks nothing.
        let wall = FakeHttp::with(&[(cpi_release, 200, "<title>Access Denied</title>")]);
        assert!(check_releases(utc("2026-11-10T13:31:00Z"), &rows, &wall, 4)
            .await
            .is_empty());
    }

    #[test]
    fn a_moved_row_keeps_its_date_when_the_page_fails_or_drops_it() {
        let snap = vec![plan(
            "bls:cpi:2026-11",
            "cpi",
            "2026-12-10",
            Some("2026-12-10T13:30:00Z"),
        )];
        let first = reconcile(utc(NOW), &snap, &live_cpi(snap.clone()), &[]);
        let moved_to = vec![plan(
            "bls:cpi:2026-11",
            "cpi",
            "2026-12-15",
            Some("2026-12-15T13:30:00Z"),
        )];
        let moved = reconcile(utc(NOW), &snap, &live_cpi(moved_to), &first);
        assert_eq!(
            (moved[0].status.as_str(), moved[0].et_date.as_str()),
            ("moved", "2026-12-15")
        );
        // The page fails the next day: the row stays as it is.
        assert!(reconcile(utc(NOW), &snap, &Live::default(), &moved).is_empty());
        // After the release day BLS drops the row from its page: still as it is.
        let later = live_cpi(vec![plan("bls:cpi:2026-12", "cpi", "2027-01-13", None)]);
        let out = reconcile(utc("2026-12-16T11:00:00Z"), &snap, &later, &moved);
        assert!(out.iter().all(|r| r.id != "bls:cpi:2026-11"), "{out:?}");
    }

    #[test]
    fn an_unreachable_page_still_lets_a_snapshot_rerun_move_a_row() {
        let old_snap = vec![plan(
            "bls:cpi:2026-11",
            "cpi",
            "2026-12-10",
            Some("2026-12-10T13:30:00Z"),
        )];
        let rows = reconcile(utc(NOW), &old_snap, &Live::default(), &[]);
        // A shutdown: the rerun snapshot says Dec 17; BLS still answers 403.
        let new_snap = vec![plan(
            "bls:cpi:2026-11",
            "cpi",
            "2026-12-17",
            Some("2026-12-17T13:30:00Z"),
        )];
        let out = reconcile(utc(NOW), &new_snap, &Live::default(), &rows);
        assert_eq!(
            (
                out[0].status.as_str(),
                out[0].et_date.as_str(),
                out[0].moved_from.as_deref()
            ),
            ("moved", "2026-12-17", Some("2026-12-10"))
        );
    }

    #[test]
    fn kinds_no_page_lists_follow_the_snapshot() {
        let mut minutes = plan(
            "fed:fomc_minutes:2026-10-28",
            "fomc_minutes",
            "2026-11-18",
            Some("2026-11-18T19:00:00Z"),
        );
        minutes.ref_period = Some("2026-10-28".into());
        let rows = reconcile(
            utc(NOW),
            std::slice::from_ref(&minutes),
            &Live::default(),
            &[],
        );
        minutes.et_date = d("2026-11-19");
        minutes.starts_at = Some(utc("2026-11-19T19:00:00Z"));
        let out = reconcile(utc(NOW), &[minutes], &Live::default(), &rows);
        assert_eq!(out[0].status, "moved");
        assert_eq!(out[0].moved_from.as_deref(), Some("2026-11-18"));
    }

    #[test]
    fn bls_rows_after_the_last_listed_date_are_not_missing() {
        let p = vec![
            plan("bls:cpi:2026-10", "cpi", "2026-11-10", None),
            plan("bls:cpi:2027-11", "cpi", "2027-12-10", None),
        ];
        let rows = reconcile(utc(NOW), &p, &live_cpi(p.clone()), &[]);
        // A page that lists only November so far.
        let short = live_cpi(vec![p[0].clone()]);
        let out = reconcile(utc(NOW), &p, &short, &rows);
        assert!(out.iter().all(|r| r.id != "bls:cpi:2027-11"), "{out:?}");
    }

    #[test]
    fn the_missing_table_error_is_recognised() {
        assert!(missing_table(
            "D1_ERROR: no such table: macro_events: SQLITE_ERROR"
        ));
        assert!(!missing_table("D1_ERROR: network"));
    }

    #[tokio::test]
    async fn daily_check_loads_the_snapshot_reads_three_pages_and_marks_the_day() {
        let ev = MemEventStore::default();
        let es = MemEarningsStore::default();
        let http = FakeHttp::official();
        let out = check_tick(utc(NOW), &ev, &es, &http).await.unwrap();
        assert!(out.daily, "{out:?}");
        assert_eq!(out.pages, "fomc=ok jobs=ok cpi=ok");
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(http.urls()[..3], [FOMC_URL, BLS_JOBS_URL, BLS_CPI_URL]);
        assert!(http.seen.borrow().iter().all(|(_, ua)| ua == USER_AGENT));
        {
            let rows = ev.rows.borrow();
            assert!(rows.contains_key("bls:cpi:2026-10"));
            assert!(rows.contains_key("nyse:closed:2026-11-26"));
            assert!(rows.contains_key("us:election:2028-11-07"));
            assert_eq!(rows["bls:cpi:2026-10"].status, "confirmed");
        }
        // One ranged read and one batch write; the brief now trusts the table.
        assert_eq!(ev.calls.get(), 2);
        assert_eq!(
            last_loaded(&es).await.unwrap().as_deref(),
            Some("2026-11-06")
        );

        // Later the same day: no pages, releases only.
        let http = FakeHttp::default();
        let out = check_tick(utc("2026-11-06T15:00:00Z"), &ev, &es, &http)
            .await
            .unwrap();
        assert!(!out.daily);
        assert!(
            http.urls().iter().all(|u| u.contains("news.release")),
            "{:?}",
            http.urls()
        );
    }

    #[tokio::test]
    async fn daily_check_survives_failed_pages() {
        let ev = MemEventStore::default();
        let es = MemEarningsStore::default();
        let http = FakeHttp::with(&[
            (FOMC_URL, 503, ""),
            (BLS_JOBS_URL, 403, ""),
            (BLS_CPI_URL, 200, "<p>new layout</p>"),
        ]);
        let out = check_tick(utc(NOW), &ev, &es, &http).await.unwrap();
        assert_eq!(out.pages, "fomc=err jobs=err cpi=err");
        assert_eq!(out.errors.len(), 3, "{:?}", out.errors);
        // The snapshot still loads, as scheduled.
        let rows = ev.rows.borrow();
        assert_eq!(rows["bls:cpi:2026-10"].status, "scheduled");
        assert!(out.postponed.is_empty());
    }
}
