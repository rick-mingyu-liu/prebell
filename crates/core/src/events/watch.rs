//! "What to watch" for the premarket brief: today's events (tier 1-2) from
//! `macro_events`, with the released check for those already past (so the
//! 8:35 update sees the 8:30 release) and, for a released CPI or jobs
//! report, its headline figures read from that same page; and the write-up
//! companies that report after today's close.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::check::{check_release_pages, last_loaded, new_row, release_page};
use super::figures::{has_figures, parse_figures, ReleaseFigure};
use super::store::{EventRow, EventStore};
use super::{et_time, has_release, note, planned, title, TIER_NOTABLE};
use crate::budget::allows;
use crate::clock::et;
use crate::earnings::store::EarningsStore;
use crate::earnings::{find, Tier};
use crate::facts::Field;
use crate::numbers::Lang;
use crate::providers::Http;

/// Release-page fetches the brief may spend on today's events. Optional
/// work: they run only while `budget::allows` them, and skipping them leaves
/// an event as scheduled with its schedule link (or a released one without
/// its figures).
pub const BRIEF_RELEASE_CHECKS: usize = 2;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WatchKind {
    Event,
    /// A write-up company reporting after today's close.
    Earnings,
}

/// One "What to watch" line. Titles and times come in both languages, from
/// the glossary (events) or the watchlist (companies), never from an LLM.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WatchItem {
    pub kind: WatchKind,
    /// The event id, or `earnings:TICKER`.
    pub id: String,
    /// "8:30 AM ET" / "上午8:30（美东）"; `None` for all-day events and earnings.
    pub time_et: Option<String>,
    pub time_zh: Option<String>,
    pub title: String,
    pub title_zh: String,
    /// The FOMC press conference line, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_zh: Option<String>,
    /// Events: scheduled | confirmed | moved | postponed | released.
    /// Earnings: confirmed | unconfirmed (an aggregator's date).
    pub status: String,
    /// The release once out, else the official schedule (events only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Elections and closures: the link is the law or the NYSE page, not a
    /// release schedule.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub source_only: bool,
    /// The previous ET date of an event moved onto today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<String>,
    /// A released CPI or jobs report's headline figures, each with the
    /// release's sentence; empty for every other item and in facts stored
    /// before figures existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub figures: Vec<ReleaseFigure>,
}

/// Gives the brief its watch list. The Worker's is `StoreWatch`.
#[async_trait(?Send)]
pub trait WatchSource {
    /// `budget` returns the subrequests used so far (see `Content::build_facts`).
    async fn watch_list(
        &self,
        date: NaiveDate,
        now: DateTime<Utc>,
        budget: &dyn Fn() -> usize,
    ) -> Field<Vec<WatchItem>>;
}

pub struct StoreWatch<'a> {
    pub events: &'a dyn EventStore,
    pub earnings: &'a dyn EarningsStore,
    pub http: &'a dyn Http,
}

fn event_item(r: &EventRow) -> Option<WatchItem> {
    let (rp, v) = (r.ref_period.as_deref(), r.variant.as_deref());
    let start = r
        .starts_at
        .as_deref()
        .and_then(|t| t.parse::<DateTime<Utc>>().ok());
    Some(WatchItem {
        kind: WatchKind::Event,
        id: r.id.clone(),
        time_et: start.map(|t| et_time(t, Lang::En)),
        time_zh: start.map(|t| et_time(t, Lang::Zh)),
        title: title(&r.series, rp, v, Lang::En)?,
        title_zh: title(&r.series, rp, v, Lang::Zh)?,
        note: note(&r.series, Lang::En).map(str::to_string),
        note_zh: note(&r.series, Lang::Zh).map(str::to_string),
        status: r.status.clone(),
        link: Some(r.release_url.clone().unwrap_or_else(|| r.proof_url.clone())),
        source_only: !has_release(&r.series),
        moved_from: r.moved_from.clone().filter(|_| r.status == "moved"),
        figures: vec![],
    })
}

#[async_trait(?Send)]
impl WatchSource for StoreWatch<'_> {
    async fn watch_list(
        &self,
        date: NaiveDate,
        now: DateTime<Utc>,
        budget: &dyn Fn() -> usize,
    ) -> Field<Vec<WatchItem>> {
        let day = date.format("%Y-%m-%d").to_string();
        // Without the table, on a D1 error, or before the first check filled
        // it, the snapshot still gives the day's events, as scheduled. Once a
        // check has run, an empty day is empty (an event may have moved away).
        let snapshot = || {
            planned(date, date)
                .iter()
                .map(|p| new_row(p, now))
                .collect()
        };
        let mut rows = match self.events.events_between(&day, &day).await {
            Ok(r) if !r.is_empty() => r,
            Ok(r) => match last_loaded(self.earnings).await {
                Ok(None) => snapshot(),
                _ => r,
            },
            Err(_) => snapshot(),
        };
        let used = budget();
        let fetches = (0..=BRIEF_RELEASE_CHECKS)
            .rev()
            .find(|n| allows(used, *n))
            .unwrap_or(0);
        let (released, mut tried) = check_release_pages(now, &rows, self.http, fetches).await;
        let mut figures: HashMap<String, Vec<ReleaseFigure>> = HashMap::new();
        if !released.is_empty() {
            let updated: Vec<EventRow> = released.iter().map(|(r, _)| r.clone()).collect();
            // Best effort: the brief's facts carry the status either way.
            let _ = self.events.write_events(&updated).await;
            for (u, page) in released {
                figures.insert(u.id.clone(), parse_figures(&u.series, &page));
                if let Some(r) = rows.iter_mut().find(|r| r.id == u.id) {
                    *r = u;
                }
            }
        }
        // A CPI or jobs report of today released before this run (by the
        // poll's check, or for an earlier brief) has no figures in the
        // table: its page is read again with the fetches left.
        let today = et(now).date_naive().format("%Y-%m-%d").to_string();
        let again: Vec<&EventRow> = rows
            .iter()
            .filter(|r| {
                r.status == "released"
                    && has_figures(&r.series)
                    && r.et_date == today
                    && !figures.contains_key(&r.id)
            })
            .collect();
        for r in again {
            let Some(url) = r.release_url.as_deref() else {
                continue;
            };
            if tried >= fetches {
                break;
            }
            tried += 1;
            if let Some(page) = release_page(self.http, url, &r.series).await {
                figures.insert(r.id.clone(), parse_figures(&r.series, &page));
            }
        }
        // All-day events first, then by time.
        rows.sort_by(|a, b| {
            (a.starts_at.is_some(), &a.starts_at).cmp(&(b.starts_at.is_some(), &b.starts_at))
        });
        let mut items: Vec<WatchItem> = rows
            .iter()
            .filter(|r| r.tier <= i64::from(TIER_NOTABLE) && r.series != "closed")
            .filter_map(|r| {
                Some(WatchItem {
                    figures: figures.remove(&r.id).unwrap_or_default(),
                    ..event_item(r)?
                })
            })
            .collect();

        if let Ok(cal) = self.earnings.calendar_between(&day, &day).await {
            for c in cal.iter().filter(|c| {
                c.time_slot == "amc" && matches!(c.status.as_str(), "confirmed" | "unconfirmed")
            }) {
                let Some(w) = find(&c.ticker).filter(|w| w.tier == Tier::Full) else {
                    continue;
                };
                let name = format!("{} ({})", w.name, w.ticker);
                items.push(WatchItem {
                    kind: WatchKind::Earnings,
                    id: format!("earnings:{}", w.ticker),
                    time_et: None,
                    time_zh: None,
                    title: name.clone(),
                    title_zh: name,
                    note: None,
                    note_zh: None,
                    status: c.status.clone(),
                    link: None,
                    source_only: false,
                    moved_from: None,
                    figures: vec![],
                });
            }
        }
        if items.is_empty() {
            Field::Missing {
                reason: "no events or after-close earnings today".into(),
            }
        } else {
            Field::Value(items)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::store::{CalendarRow, MemEarningsStore};
    use crate::events::figures::FigureKey;
    use crate::events::store::MemEventStore;
    use crate::providers::{HttpResp, ProviderError};
    use std::cell::RefCell;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[derive(Default)]
    struct Http200 {
        ok: Vec<String>,
        /// The body every url answers; a bare CPI title when `None`.
        page: Option<String>,
        seen: RefCell<Vec<String>>,
    }
    #[async_trait(?Send)]
    impl Http for Http200 {
        async fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            self.seen.borrow_mut().push(url.into());
            let status = if self.ok.iter().any(|u| u == url) {
                200
            } else {
                404
            };
            Ok(HttpResp {
                status,
                body: self
                    .page
                    .clone()
                    .unwrap_or_else(|| "<title>Consumer Price Index News Release</title>".into()),
                set_cookie: None,
                content_type: None,
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

    fn cal(ticker: &str, slot: &str, status: &str) -> CalendarRow {
        CalendarRow {
            ticker: ticker.into(),
            report_date: "2026-11-10".into(),
            time_slot: slot.into(),
            status: status.into(),
            source: "yahoo".into(),
            proof_url: None,
            proof_quote: None,
            conflict_reason: None,
            first_seen_at: String::new(),
            checked_at: String::new(),
            next_check_at: None,
            updated_at: String::new(),
        }
    }

    fn seeded() -> MemEventStore {
        let ev = MemEventStore::default();
        for p in planned(d("2026-11-10"), d("2026-11-10")) {
            ev.rows
                .borrow_mut()
                .insert(p.id.clone(), new_row(&p, utc("2026-11-06T11:00:00Z")));
        }
        ev
    }

    #[tokio::test]
    async fn cpi_day_lists_cpi_then_after_close_write_ups() {
        let ev = seeded();
        let es = MemEarningsStore::default();
        for c in [
            cal("AAPL", "amc", "confirmed"),
            cal("CSCO", "amc", "unconfirmed"),
            cal("WMT", "bmo", "confirmed"),
            cal("AIG", "amc", "reported"),
        ] {
            es.calendar
                .borrow_mut()
                .insert((c.ticker.clone(), c.report_date.clone()), c);
        }
        let http = Http200::default();
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        // 8:00 ET: before the release, nothing is fetched.
        let Field::Value(items) = w
            .watch_list(d("2026-11-10"), utc("2026-11-10T13:00:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert!(http.seen.borrow().is_empty());
        assert_eq!(items[0].title, "October CPI");
        assert_eq!(items[0].title_zh, "10月消费者物价指数（CPI）");
        assert_eq!(items[0].time_et.as_deref(), Some("8:30 AM ET"));
        assert_eq!(items[0].time_zh.as_deref(), Some("上午8:30（美东）"));
        assert_eq!(
            items[0].link.as_deref(),
            Some("https://www.bls.gov/schedule/news_release/cpi.htm")
        );
        let names: Vec<&str> = items[1..].iter().map(|i| i.title.as_str()).collect();
        // Only write-up companies, after the close, not yet reported.
        assert!(names.iter().any(|n| n.contains("(AAPL)")), "{names:?}");
        assert!(
            !names.iter().any(|n| n.contains("WMT") || n.contains("AIG")),
            "{names:?}"
        );
        assert!(items[1..].iter().all(|i| i.kind == WatchKind::Earnings));
    }

    #[tokio::test]
    async fn the_835_update_sees_the_release_and_records_it() {
        let ev = seeded();
        let es = MemEarningsStore::default();
        let url = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
        let http = Http200 {
            ok: vec![url.into()],
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w
            .watch_list(d("2026-11-10"), utc("2026-11-10T13:35:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert_eq!(items[0].status, "released");
        assert_eq!(items[0].link.as_deref(), Some(url));
        assert_eq!(ev.rows.borrow()["bls:cpi:2026-10"].status, "released");
        // A release page whose opening states no figure gives none.
        assert!(items[0].figures.is_empty());
    }

    const CPI_RELEASE: &str = include_str!("../../tests/fixtures/bls_cpi_release.htm");
    const JOBS_RELEASE: &str = include_str!("../../tests/fixtures/bls_empsit_release.htm");

    /// A release page as BLS serves it: the opening summary after ~34 KB of
    /// site navigation (its offset on the 2026-11-10 CPI page), then 1.3 MB.
    fn real_page(fixture: &str) -> String {
        format!("{}{fixture}{}", " ".repeat(33_942), "x".repeat(1_300_000))
    }

    #[tokio::test]
    async fn the_835_update_states_the_cpi_figures_from_the_same_fetch() {
        let ev = seeded();
        let es = MemEarningsStore::default();
        let url = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
        let http = Http200 {
            ok: vec![url.into()],
            page: Some(real_page(CPI_RELEASE)),
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w
            .watch_list(d("2026-11-10"), utc("2026-11-10T13:35:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        // One fetch marks it released and gives the figures.
        assert_eq!(http.seen.borrow().len(), 1);
        assert_eq!(items[0].status, "released");
        let got: Vec<(FigureKey, f64)> =
            items[0].figures.iter().map(|f| (f.key, f.value)).collect();
        assert_eq!(
            got,
            [(FigureKey::CpiMonth, 0.4), (FigureKey::CoreMonth, 0.3)]
        );
        assert!(items[0].figures[0].quote.starts_with(
            "The Consumer Price Index for All Urban Consumers (CPI-U) increased 0.4 percent"
        ));
        // The summary ends within the bytes read.
        let end = 33_942 + CPI_RELEASE.find("</PRE>").unwrap();
        assert!(end < crate::events::check::RELEASE_PREFIX_BYTES, "{end}");
    }

    #[tokio::test]
    async fn jobs_day_states_payrolls_and_the_rate() {
        let ev = MemEventStore::default();
        for p in planned(d("2026-11-06"), d("2026-11-06")) {
            ev.rows
                .borrow_mut()
                .insert(p.id.clone(), new_row(&p, utc("2026-11-01T11:00:00Z")));
        }
        let es = MemEarningsStore::default();
        let url = "https://www.bls.gov/news.release/archives/empsit_11062026.htm";
        let http = Http200 {
            ok: vec![url.into()],
            page: Some(real_page(JOBS_RELEASE)),
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w
            .watch_list(d("2026-11-06"), utc("2026-11-06T13:35:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        let jobs = items.iter().find(|i| i.id == "bls:jobs:2026-10").unwrap();
        let got: Vec<(FigureKey, f64)> = jobs.figures.iter().map(|f| (f.key, f.value)).collect();
        assert_eq!(
            got,
            [
                (FigureKey::Payrolls, 165_000.0),
                (FigureKey::UnemploymentRate, 4.1)
            ]
        );
    }

    /// The store as the poll's check leaves it: the CPI row released, with
    /// its release link and no figures.
    fn released_by_the_poll() -> (MemEventStore, &'static str) {
        let ev = seeded();
        let url = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
        {
            let mut rows = ev.rows.borrow_mut();
            let r = rows.get_mut("bls:cpi:2026-10").unwrap();
            r.status = "released".into();
            r.release_url = Some(url.into());
        }
        (ev, url)
    }

    #[tokio::test]
    async fn a_report_released_before_the_brief_is_read_again_for_its_figures() {
        let (ev, url) = released_by_the_poll();
        let es = MemEarningsStore::default();
        let http = Http200 {
            ok: vec![url.into()],
            page: Some(real_page(CPI_RELEASE)),
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let at = utc("2026-11-10T13:40:00Z");
        let calls = ev.calls.get();
        let Field::Value(items) = w.watch_list(d("2026-11-10"), at, &|| 0).await else {
            panic!("missing");
        };
        assert_eq!(*http.seen.borrow(), [url]);
        assert_eq!(items[0].figures.len(), 2);
        // Read only: one ranged read, nothing written.
        assert_eq!(ev.calls.get(), calls + 1);

        // 39 used: the one fetch still fits.
        let http = Http200 {
            ok: vec![url.into()],
            page: Some(real_page(CPI_RELEASE)),
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w.watch_list(d("2026-11-10"), at, &|| 39).await else {
            panic!("missing");
        };
        assert_eq!(http.seen.borrow().len(), 1);
        assert_eq!(items[0].figures.len(), 2);

        // At the limit: no fetch, released without figures (today's line).
        let http = Http200 {
            ok: vec![url.into()],
            page: Some(real_page(CPI_RELEASE)),
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w.watch_list(d("2026-11-10"), at, &|| 40).await else {
            panic!("missing");
        };
        assert!(http.seen.borrow().is_empty());
        assert_eq!(
            (items[0].status.as_str(), items[0].link.as_deref()),
            ("released", Some(url))
        );
        assert!(items[0].figures.is_empty());

        // A brief built the next day for the release day reads nothing.
        let Field::Value(_) = w
            .watch_list(d("2026-11-10"), utc("2026-11-11T13:40:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert!(http.seen.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_bot_wall_on_the_second_read_leaves_the_released_line() {
        let (ev, url) = released_by_the_poll();
        let es = MemEarningsStore::default();
        for page in [
            "<title>Access Denied</title>",
            "<title>Consumer Price Index News Release</title>",
        ] {
            let http = Http200 {
                ok: vec![url.into()],
                page: Some(page.into()),
                ..Default::default()
            };
            let w = StoreWatch {
                events: &ev,
                earnings: &es,
                http: &http,
            };
            let Field::Value(items) = w
                .watch_list(d("2026-11-10"), utc("2026-11-10T13:40:00Z"), &|| 0)
                .await
            else {
                panic!("missing");
            };
            assert_eq!(http.seen.borrow().len(), 1);
            assert_eq!(items[0].status, "released");
            assert!(items[0].figures.is_empty(), "{page}");
        }
    }

    #[test]
    fn items_stored_before_figures_load_and_items_without_figures_store_as_before() {
        let old = serde_json::json!({"kind": "event", "id": "bls:cpi:2026-10",
            "time_et": "8:30 AM ET", "time_zh": "上午8:30（美东）", "title": "October CPI",
            "title_zh": "10月消费者物价指数（CPI）", "status": "released",
            "link": "https://www.bls.gov/news.release/archives/cpi_11102026.htm"});
        let w: WatchItem = serde_json::from_value(old.clone()).unwrap();
        assert!(w.figures.is_empty());
        assert_eq!(serde_json::to_value(&w).unwrap(), old);
    }

    #[tokio::test]
    async fn near_the_subrequest_limit_the_release_fetch_is_skipped() {
        let ev = seeded();
        let es = MemEarningsStore::default();
        let url = "https://www.bls.gov/news.release/archives/cpi_11102026.htm";
        let http = Http200 {
            ok: vec![url.into()],
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let at = utc("2026-11-10T13:35:00Z");
        // 39 used: one fetch fits under BUDGET_SOFT (40), not two.
        let Field::Value(items) = w.watch_list(d("2026-11-10"), at, &|| 39).await else {
            panic!("missing");
        };
        assert_eq!(http.seen.borrow().len(), 1);
        assert_eq!(items[0].status, "released");
        // At the limit: no fetch, and the event stays scheduled with its schedule link.
        let ev = seeded();
        let http = Http200 {
            ok: vec![url.into()],
            ..Default::default()
        };
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w.watch_list(d("2026-11-10"), at, &|| 40).await else {
            panic!("missing");
        };
        assert!(http.seen.borrow().is_empty());
        assert_eq!(items[0].status, "scheduled");
        assert_eq!(
            items[0].link.as_deref(),
            Some("https://www.bls.gov/schedule/news_release/cpi.htm")
        );
    }

    #[tokio::test]
    async fn an_event_moved_away_is_not_listed_on_its_old_day() {
        // CPI moved from Nov 10 to Nov 13; the check has run.
        let ev = seeded();
        {
            let mut rows = ev.rows.borrow_mut();
            let r = rows.get_mut("bls:cpi:2026-10").unwrap();
            r.et_date = "2026-11-13".into();
            r.starts_at = Some("2026-11-13T13:30:00Z".into());
            r.status = "moved".into();
            r.moved_from = Some("2026-11-10".into());
        }
        let es = MemEarningsStore::default();
        es.kv_set("events_loaded_on", "2026-11-09", "x")
            .await
            .unwrap();
        let http = Http200::default();
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        assert!(matches!(
            w.watch_list(d("2026-11-10"), utc("2026-11-10T13:00:00Z"), &|| 0)
                .await,
            Field::Missing { .. }
        ));
        let Field::Value(items) = w
            .watch_list(d("2026-11-13"), utc("2026-11-13T13:00:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert_eq!(items[0].moved_from.as_deref(), Some("2026-11-10"));
    }

    #[tokio::test]
    async fn a_quiet_day_is_missing_and_a_missing_table_falls_back_to_the_snapshot() {
        let es = MemEarningsStore::default();
        let http = Http200::default();
        let ev = MemEventStore::default();
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        assert!(matches!(
            w.watch_list(d("2026-11-12"), utc("2026-11-12T13:00:00Z"), &|| 0)
                .await,
            Field::Missing { .. }
        ));
        // An empty table (0008 applied, no check yet) also reads the snapshot.
        let Field::Value(items) = w
            .watch_list(d("2026-11-10"), utc("2026-11-10T13:00:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert_eq!(items[0].id, "bls:cpi:2026-10");
        let broken = MemEventStore {
            fail: Some("no such table: macro_events".into()),
            ..Default::default()
        };
        let w = StoreWatch {
            events: &broken,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w
            .watch_list(d("2026-11-10"), utc("2026-11-10T13:00:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert_eq!(items[0].id, "bls:cpi:2026-10");
    }

    #[tokio::test]
    async fn fomc_day_carries_the_press_conference_and_early_close_is_listed() {
        let ev = MemEventStore::default();
        for p in planned(d("2026-11-27"), d("2026-12-09")) {
            ev.rows
                .borrow_mut()
                .insert(p.id.clone(), new_row(&p, utc("2026-11-06T11:00:00Z")));
        }
        let es = MemEarningsStore::default();
        let http = Http200::default();
        let w = StoreWatch {
            events: &ev,
            earnings: &es,
            http: &http,
        };
        let Field::Value(items) = w
            .watch_list(d("2026-12-09"), utc("2026-12-09T13:00:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert_eq!(items[0].title, "FOMC rate decision (with projections)");
        assert_eq!(
            items[0].note.as_deref(),
            Some("Press conference 2:30 PM ET")
        );
        assert_eq!(items[0].time_et.as_deref(), Some("2:00 PM ET"));
        let Field::Value(items) = w
            .watch_list(d("2026-11-27"), utc("2026-11-27T13:00:00Z"), &|| 0)
            .await
        else {
            panic!("missing");
        };
        assert_eq!(items[0].title, "Early close, 1:00 PM ET");
        assert_eq!(items[0].time_et, None);
    }
}
