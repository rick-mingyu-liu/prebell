//! Competitor watch (internal only): a daily read of AlphaLab's public RSS
//! feed, compared against what we published. Only titles, links, times,
//! categories and title tickers are kept. Nothing here may feed article
//! generation; `tests::generation_never_reads_competitor_data` enforces that.

pub mod feed;
pub mod report;
pub mod tickers;

use std::cell::RefCell;
use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, SecondsFormat, Utc};

use chrono_tz::America::New_York;

use crate::clock::{et, trading_day, DayKind};
use crate::providers::Http;
use feed::parse_feed;
use report::{build_report, digest, OurPost, Report, StoredItem};
use tickers::tickers_in;

pub const FEED_URL: &str = "https://www.alphalab.site/feed.xml";
/// Items stored per run; the feed is far smaller, this bounds a bad response.
const MAX_ITEMS: usize = 200;

pub fn run_key(report_date: NaiveDate) -> String {
    format!("competitor:{}", report_date.format("%Y-%m-%d"))
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompetitorDecision {
    NotNow,
    Due { report_date: NaiveDate },
}

/// Due when the ET time is in [07:30, 08:00), any day of the week. The cron
/// fires at 11:31 and 12:31 UTC; exactly one of those is 07:31 ET.
pub fn competitor_decision(now_utc: DateTime<Utc>) -> CompetitorDecision {
    let local = et(now_utc);
    let start = NaiveTime::from_hms_opt(7, 30, 0).unwrap();
    let end = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
    if (start..end).contains(&local.time()) {
        CompetitorDecision::Due {
            report_date: local.date_naive(),
        }
    } else {
        CompetitorDecision::NotNow
    }
}

/// Our side of the comparison reaches back to 00:00 ET of the previous NYSE
/// trading day (or 24h, whichever is earlier). We don't publish on closed
/// days, so a Monday or weekend report still compares against Friday's posts.
pub fn our_window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let mut d = et(now).date_naive() - Duration::days(1);
    while trading_day(d) == DayKind::Closed {
        d -= Duration::days(1);
    }
    let midnight = d
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(New_York)
        .earliest()
        .map(|t| t.with_timezone(&Utc));
    let day_ago = now - Duration::hours(24);
    midnight.map_or(day_ago, |m| m.min(day_ago))
}

/// A `competitor_items` row to insert or refresh.
#[derive(Debug, Clone, PartialEq)]
pub struct NewItem {
    pub link: String,
    pub title: String,
    /// UTC ISO 8601 with `Z`; `None` when the feed had no usable `pubDate`.
    pub published_at: Option<String>,
    /// Categories joined with `", "`.
    pub category: String,
    pub tickers: Vec<String>,
}

/// Persistence for the competitor watch (`migrations/0004_competitor.sql`).
/// Times are UTC ISO 8601 strings with `Z`; windows are `[start, end)`.
#[async_trait(?Send)]
pub trait CompetitorStore {
    async fn has_report(&self, report_date: &str) -> Result<bool, String>;
    /// Insert by `link`, or refresh an existing row keeping `first_seen_at`.
    async fn upsert_items(&self, items: &[NewItem], seen_at: &str) -> Result<(), String>;
    /// Their items with `published_at` in the window.
    async fn items_between(&self, start: &str, end: &str) -> Result<Vec<StoredItem>, String>;
    /// Our posts whose EN translation is published, with `published_at` in the window.
    async fn our_posts_between(&self, start: &str, end: &str) -> Result<Vec<OurPost>, String>;
    async fn put_report(&self, r: &Report, digest: &str, created_at: &str) -> Result<(), String>;
    /// Same `alerts` table as the run state machine; the generator delivers it.
    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String>;
}

/// In-memory `CompetitorStore` for tests.
#[derive(Default)]
pub struct MemCompetitorStore {
    /// By link: (row, first_seen_at, last_seen_at).
    pub items: RefCell<BTreeMap<String, (NewItem, String, String)>>,
    /// Seeded by tests: (published_at, post).
    pub our_posts: RefCell<Vec<(String, OurPost)>>,
    pub reports: RefCell<Vec<(Report, String)>>,
    /// `(run_key, level, message)` in insertion order.
    pub alerts: RefCell<Vec<(String, String, String)>>,
}

#[async_trait(?Send)]
impl CompetitorStore for MemCompetitorStore {
    async fn has_report(&self, report_date: &str) -> Result<bool, String> {
        Ok(self
            .reports
            .borrow()
            .iter()
            .any(|(r, _)| r.report_date == report_date))
    }
    async fn upsert_items(&self, items: &[NewItem], seen_at: &str) -> Result<(), String> {
        let mut map = self.items.borrow_mut();
        for i in items {
            let first = map
                .get(&i.link)
                .map_or_else(|| seen_at.to_string(), |(_, f, _)| f.clone());
            map.insert(i.link.clone(), (i.clone(), first, seen_at.to_string()));
        }
        Ok(())
    }
    async fn items_between(&self, start: &str, end: &str) -> Result<Vec<StoredItem>, String> {
        Ok(self
            .items
            .borrow()
            .values()
            .filter_map(|(i, _, _)| {
                let at = i.published_at.as_deref()?;
                (start <= at && at < end).then(|| StoredItem {
                    link: i.link.clone(),
                    title: i.title.clone(),
                    published_at: at.to_string(),
                    category: i.category.clone(),
                    tickers: i.tickers.clone(),
                })
            })
            .collect())
    }
    async fn our_posts_between(&self, start: &str, end: &str) -> Result<Vec<OurPost>, String> {
        Ok(self
            .our_posts
            .borrow()
            .iter()
            .filter(|(at, _)| start <= at.as_str() && at.as_str() < end)
            .map(|(_, p)| p.clone())
            .collect())
    }
    async fn put_report(&self, r: &Report, digest: &str, _created_at: &str) -> Result<(), String> {
        let mut reports = self.reports.borrow_mut();
        reports.retain(|(old, _)| old.report_date != r.report_date);
        reports.push((r.clone(), digest.to_string()));
        Ok(())
    }
    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String> {
        self.alerts
            .borrow_mut()
            .push((run_key.into(), level.into(), msg.into()));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompetitorOutcome {
    Nothing,
    AlreadyDone,
    Reported { items: usize, uncovered: usize },
    Failed(String),
}

/// One cron firing. When due and not yet reported today: fetch the feed,
/// store item metadata, compare the last 24h with our posts, store the
/// report and queue its digest as an `info` alert. Any failure becomes an
/// `error` alert and no report; the next day runs normally.
pub async fn run_competitor(
    now: DateTime<Utc>,
    http: &dyn Http,
    store: &dyn CompetitorStore,
    contact: &str,
) -> CompetitorOutcome {
    let CompetitorDecision::Due { report_date } = competitor_decision(now) else {
        return CompetitorOutcome::Nothing;
    };
    let key = run_key(report_date);
    match step(now, report_date, http, store, contact).await {
        Ok(outcome) => outcome,
        Err(e) => {
            let msg = format!("competitor watch failed for {report_date}: {e}");
            // Best effort: if the store is down the alert is lost too.
            let _ = store.add_alert(&key, "error", &msg).await;
            CompetitorOutcome::Failed(e)
        }
    }
}

async fn step(
    now: DateTime<Utc>,
    report_date: NaiveDate,
    http: &dyn Http,
    store: &dyn CompetitorStore,
    contact: &str,
) -> Result<CompetitorOutcome, String> {
    let date = report_date.format("%Y-%m-%d").to_string();
    if store.has_report(&date).await? {
        return Ok(CompetitorOutcome::AlreadyDone);
    }

    let ua = format!("PreBell competitor-watch (+{contact})");
    let resp = http
        .get(
            FEED_URL,
            &[
                ("User-Agent", ua.as_str()),
                ("Accept", "application/rss+xml, application/xml;q=0.9"),
            ],
        )
        .await
        .map_err(|e| format!("feed fetch: {e}"))?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("feed fetch: HTTP {}", resp.status));
    }
    let items = parse_feed(&resp.body).map_err(|e| format!("feed parse: {e}"))?;

    let rows: Vec<NewItem> = items
        .into_iter()
        .take(MAX_ITEMS)
        .map(|i| NewItem {
            tickers: tickers_in(&i.title),
            link: i.link,
            title: i.title,
            published_at: i.published.map(iso),
            category: i.categories.join(", "),
        })
        .collect();
    store.upsert_items(&rows, &iso(now)).await?;

    let start = now - Duration::hours(24);
    let our_start = our_window_start(now);
    let (s, e) = (iso(start), iso(now));
    let theirs = store.items_between(&s, &e).await?;
    let ours = store.our_posts_between(&iso(our_start), &e).await?;
    let report = build_report(report_date, start, now, our_start, &theirs, &ours);
    let text = digest(&report);
    // Queue the digest first: it is the deliverable, and once the report
    // row exists later firings are no-ops.
    store
        .add_alert(&run_key(report_date), "info", &text)
        .await?;
    store.put_report(&report, &text, &iso(now)).await?;
    Ok(CompetitorOutcome::Reported {
        items: report.their_item_count,
        uncovered: report.uncovered.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::report::DIGEST_MAX_CHARS;
    use super::*;
    use crate::providers::{HttpResp, ProviderError};

    const FIXTURE: &str = include_str!("../../tests/fixtures/alphalab_feed.xml");

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    type Seen = Vec<(String, Vec<(String, String)>)>;
    struct FakeHttp {
        status: u16,
        body: String,
        seen: RefCell<Seen>,
    }
    impl FakeHttp {
        fn new(status: u16, body: &str) -> Self {
            FakeHttp {
                status,
                body: body.into(),
                seen: RefCell::new(vec![]),
            }
        }
    }
    #[async_trait(?Send)]
    impl Http for FakeHttp {
        async fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
        ) -> Result<HttpResp, ProviderError> {
            self.seen.borrow_mut().push((
                url.into(),
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            Ok(HttpResp {
                status: self.status,
                body: self.body.clone(),
                set_cookie: None,
            })
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            unreachable!()
        }
    }

    /// Thu 2026-10-29 07:30 EDT; the fixture's dated items fall on 10-27..10-29.
    const NOW: &str = "2026-10-29T11:30:00Z";

    #[test]
    fn decision_is_dst_safe() {
        let due = |s: &str| matches!(competitor_decision(utc(s)), CompetitorDecision::Due { .. });
        assert!(due("2026-10-30T11:30:00Z")); // EDT 07:30
        assert!(due("2026-10-30T11:31:00Z")); // the actual cron minute
        assert!(!due("2026-10-30T12:31:00Z"));
        assert!(due("2026-11-02T12:31:00Z"));
        assert!(!due("2026-11-02T11:31:00Z"));
        assert!(!due("2026-10-30T12:30:00Z")); // EDT 08:30
        assert!(due("2026-11-02T12:30:00Z")); // EST 07:30
        assert!(!due("2026-11-02T11:30:00Z")); // EST 06:30
        assert_eq!(
            competitor_decision(utc("2026-10-31T11:30:00Z")), // Saturday
            CompetitorDecision::Due {
                report_date: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
            }
        );
        // DST ends Sun 2026-11-01 at 02:00; 07:30 that morning is EST.
        assert!(due("2026-11-01T12:30:00Z"));
        assert!(!due("2026-11-01T11:30:00Z"));
    }

    #[test]
    fn our_window_reaches_back_to_last_trading_day() {
        // Tue 07:31 EDT: the previous trading day is Mon; 24h ago is earlier.
        assert_eq!(
            our_window_start(utc("2026-10-27T11:31:00Z")),
            utc("2026-10-26T04:00:00Z")
        );
        // Sat, Sun and Mon reports reach back to Fri 2026-10-30 00:00 EDT.
        for now in [
            "2026-10-31T11:31:00Z",
            "2026-11-01T12:31:00Z",
            "2026-11-02T12:31:00Z",
        ] {
            assert_eq!(
                our_window_start(utc(now)),
                utc("2026-10-30T04:00:00Z"),
                "{now}"
            );
        }
        // Fri after Thanksgiving reaches back to Wed 2026-11-25 (EST).
        assert_eq!(
            our_window_start(utc("2026-11-27T12:31:00Z")),
            utc("2026-11-25T05:00:00Z")
        );
    }

    #[tokio::test]
    async fn weekend_report_compares_with_fridays_posts() {
        let http = FakeHttp::new(200, FIXTURE);
        let store = MemCompetitorStore::default();
        store.our_posts.borrow_mut().push((
            "2026-10-30T12:40:00Z".into(),
            OurPost {
                slug: "premarket-2026-10-30".into(),
                text: "| MSFT | Microsoft |".into(),
            },
        ));
        // Mon 2026-11-02 07:31 EST; Friday's post is 72h old.
        run_competitor(utc("2026-11-02T12:31:00Z"), &http, &store, "x").await;
        let r = store.reports.borrow()[0].0.clone();
        assert_eq!(r.our_slugs, ["premarket-2026-10-30"]);
        assert_eq!(r.our_window_start, "2026-10-30T04:00:00Z");
    }

    #[tokio::test]
    async fn reports_once_per_day() {
        let http = FakeHttp::new(200, FIXTURE);
        let store = MemCompetitorStore::default();
        store.our_posts.borrow_mut().push((
            "2026-10-28T12:40:00Z".into(),
            OurPost {
                slug: "premarket-2026-10-28".into(),
                text: "| KLAC | KLA Corp |".into(),
            },
        ));

        let out = run_competitor(utc(NOW), &http, &store, "ops@prebell.test").await;
        // In the 24h window: MSFT/NVDA (10-29 11:05Z) and KLAC (10-29 02:30Z);
        // the macro item (10-28 23:00Z) has no ticker.
        assert_eq!(
            out,
            CompetitorOutcome::Reported {
                items: 3,
                uncovered: 2
            }
        );

        let (url, headers) = http.seen.borrow()[0].clone();
        assert_eq!(url, FEED_URL);
        assert!(headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.contains("ops@prebell.test")));

        let items = store.items.borrow().clone();
        assert_eq!(items.len(), 5);
        let msft = &items["https://www.alphalab.site/posts/fixture-msft-nvda"].0;
        assert_eq!(msft.tickers, ["MSFT", "NVDA"]);
        assert_eq!(msft.category, "Earnings, Tech");
        assert_eq!(msft.published_at.as_deref(), Some("2026-10-29T11:05:00Z"));
        let tsm = &items["https://www.alphalab.site/posts/fixture-tsm"].0;
        assert_eq!(tsm.published_at, None);
        assert_eq!(tsm.tickers, ["TSM", "MSFT"]);

        let reports = store.reports.borrow().clone();
        assert_eq!(reports.len(), 1);
        let (r, d) = &reports[0];
        assert_eq!(r.covered, ["KLAC"]);
        let gaps: Vec<&str> = r.uncovered.iter().map(|g| g.ticker.as_str()).collect();
        assert_eq!(gaps, ["MSFT", "NVDA"]);
        assert_eq!(r.topics.len(), 1);

        let alerts = store.alerts.borrow().clone();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].0, "competitor:2026-10-29");
        assert_eq!(alerts[0].1, "info");
        assert_eq!(&alerts[0].2, d);
        assert!(alerts[0].2.chars().count() <= DIGEST_MAX_CHARS);

        // A second firing the same ET day does nothing.
        let again = run_competitor(utc("2026-10-29T11:35:00Z"), &http, &store, "x").await;
        assert_eq!(again, CompetitorOutcome::AlreadyDone);
        assert_eq!(http.seen.borrow().len(), 1);
        assert_eq!(store.reports.borrow().len(), 1);
        assert_eq!(store.alerts.borrow().len(), 1);
    }

    #[tokio::test]
    async fn refetch_keeps_first_seen() {
        let http = FakeHttp::new(200, FIXTURE);
        let store = MemCompetitorStore::default();
        run_competitor(utc(NOW), &http, &store, "x").await;
        run_competitor(utc("2026-10-30T11:30:00Z"), &http, &store, "x").await;
        let items = store.items.borrow();
        assert_eq!(items.len(), 5);
        let (_, first, last) = &items["https://www.alphalab.site/posts/fixture-klac"];
        assert_eq!(first, "2026-10-29T11:30:00Z");
        assert_eq!(last, "2026-10-30T11:30:00Z");
    }

    #[tokio::test]
    async fn not_due_does_nothing() {
        let http = FakeHttp::new(200, FIXTURE);
        let store = MemCompetitorStore::default();
        let out = run_competitor(utc("2026-10-29T12:30:00Z"), &http, &store, "x").await;
        assert_eq!(out, CompetitorOutcome::Nothing);
        assert!(http.seen.borrow().is_empty());
    }

    #[tokio::test]
    async fn http_error_alerts_without_report() {
        let http = FakeHttp::new(503, "busy");
        let store = MemCompetitorStore::default();
        let out = run_competitor(utc(NOW), &http, &store, "x").await;
        assert!(
            matches!(out, CompetitorOutcome::Failed(ref e) if e.contains("503")),
            "{out:?}"
        );
        assert!(store.reports.borrow().is_empty());
        assert!(store.items.borrow().is_empty());
        let alerts = store.alerts.borrow();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].1, "error");
    }

    #[tokio::test]
    async fn html_body_fails() {
        let http = FakeHttp::new(200, "<html><body>Just a moment...</body></html>");
        let store = MemCompetitorStore::default();
        let out = run_competitor(utc(NOW), &http, &store, "x").await;
        assert!(matches!(out, CompetitorOutcome::Failed(_)), "{out:?}");
        assert!(store.reports.borrow().is_empty());
        assert_eq!(store.alerts.borrow()[0].1, "error");
    }

    /// Spec §5: competitor data never feeds generation. The generation
    /// modules and prompts must not reference this module or its tables.
    #[test]
    fn generation_never_reads_competitor_data() {
        let sources = [
            ("premarket/mod.rs", include_str!("../premarket/mod.rs")),
            ("premarket/draft.rs", include_str!("../premarket/draft.rs")),
            (
                "premarket/universe.rs",
                include_str!("../premarket/universe.rs"),
            ),
            ("run.rs", include_str!("../run.rs")),
            ("facts.rs", include_str!("../facts.rs")),
            ("store.rs", include_str!("../store.rs")),
            ("anchor.rs", include_str!("../anchor.rs")),
            ("numbers.rs", include_str!("../numbers.rs")),
            (
                "premarket_v1_en.md",
                include_str!("../../prompts/premarket_v1_en.md"),
            ),
            (
                "premarket_v1_zh.md",
                include_str!("../../prompts/premarket_v1_zh.md"),
            ),
        ];
        for (name, src) in sources {
            let lower = src.to_lowercase();
            assert!(
                !lower.contains("competitor") && !lower.contains("alphalab"),
                "{name} references competitor data"
            );
        }
    }
}
