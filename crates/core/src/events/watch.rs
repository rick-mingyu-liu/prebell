//! "What to watch" for the premarket brief: today's events (tier 1-2) from
//! `macro_events`, with the released check for those already past (so the
//! 8:35 update sees the 8:30 release), and the write-up companies that
//! report after today's close.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::check::{check_releases, new_row};
use super::store::{EventRow, EventStore};
use super::{et_time, has_release, note, planned, title, TIER_NOTABLE};
use crate::budget::allows;
use crate::earnings::store::EarningsStore;
use crate::earnings::{find, Tier};
use crate::facts::Field;
use crate::numbers::Lang;
use crate::providers::Http;

/// Release-page fetches the brief may spend on today's events. Optional
/// work: they run only while `budget::allows` them, and skipping them leaves
/// an event as scheduled with its schedule link.
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
        // Without the table, before its first check fills it, or on a D1
        // error, the snapshot still gives the day's events, as scheduled.
        let mut rows = match self.events.events_between(&day, &day).await {
            Ok(r) if !r.is_empty() => r,
            _ => planned(date, date)
                .iter()
                .map(|p| new_row(p, now))
                .collect(),
        };
        let used = budget();
        let fetches = (0..=BRIEF_RELEASE_CHECKS)
            .rev()
            .find(|n| allows(used, *n))
            .unwrap_or(0);
        let released = check_releases(now, &rows, self.http, fetches).await;
        if !released.is_empty() {
            // Best effort: the brief's facts carry the status either way.
            let _ = self.events.write_events(&released).await;
            for u in released {
                if let Some(r) = rows.iter_mut().find(|r| r.id == u.id) {
                    *r = u;
                }
            }
        }
        // All-day events first, then by time.
        rows.sort_by(|a, b| {
            (a.starts_at.is_some(), &a.starts_at).cmp(&(b.starts_at.is_some(), &b.starts_at))
        });
        let mut items: Vec<WatchItem> = rows
            .iter()
            .filter(|r| r.tier <= i64::from(TIER_NOTABLE) && r.series != "closed")
            .filter_map(event_item)
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
                body: "<title>Consumer Price Index News Release</title>".into(),
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
