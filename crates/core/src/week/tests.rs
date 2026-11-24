//! The week post's tests: the bundle (task 1), the page, guard and job
//! (task 2), and the dry run on a read-only snapshot of live D1 rows.

use std::cell::Cell;
use std::rc::Rc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::Value;

use super::guard::verify_page;
use super::job::{follow_up, render_both, week_tick, BACKOFF_MINUTES, MAX_BUILDS};
use super::render::{body, cover_lines, summary_of, title_of, write, LEGEND_EN, LEGEND_ZH};
use super::store::{CheckRow, WeekRun, WeekStore, Written};
use super::*;
use crate::earnings::store::{CalendarRow, MemEarningsStore};
use crate::events::store::{EventRow, MemEventStore};
use crate::numbers::Lang;
use crate::schedule::EarningsStep;
use crate::store::{MemStore, PostRow, RunRow, TranslationRow};

pub(crate) fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// "Copart (CPRT)", by the watchlist's name.
fn lbl(t: &str) -> String {
    let name = crate::earnings::find(t).map_or(t.to_string(), |w| w.name.to_string());
    format!("{name} ({t})")
}

// ---------------------------------------------------------------------------
// Worlds: the in-memory stores, with checks, calendar rows and events.

pub(crate) struct World {
    pub base: Rc<MemStore>,
    pub es: MemEarningsStore,
    pub ev: MemEventStore,
}

pub(crate) fn world() -> World {
    let base = Rc::new(MemStore::default());
    let es = MemEarningsStore::new(base.clone());
    // The events table is loaded, so its rows (not the snapshot) are used.
    es.kv
        .borrow_mut()
        .insert("events_loaded_on".into(), "2026-11-01".into());
    World {
        base,
        es,
        ev: MemEventStore::default(),
    }
}

impl World {
    /// An earnings check: its post and both translations.
    #[allow(clippy::too_many_arguments)]
    pub fn check(
        &self,
        slug: &str,
        ticker: &str,
        date: &str,
        published_at: &str,
        en: [&str; 2],
        zh: [&str; 2],
        zh_status: &str,
    ) {
        let mut posts = self.base.posts.borrow_mut();
        let id = posts.len() as i64 + 1;
        posts.push(PostRow {
            id: Some(id),
            run_key: format!("earnings:{ticker}:acc-{id}"),
            content_type: "earnings".into(),
            template_id: "earnings:v1".into(),
            trading_date: date.into(),
            slug: slug.into(),
            facts_r2_key: format!("facts/{slug}.json"),
            published_at: Some(published_at.into()),
            cover_key: None,
            revised_at: None,
        });
        let mut t = self.base.translations.borrow_mut();
        for (lang, lines, status) in [("en", en, "published"), ("zh", zh, zh_status)] {
            t.insert(
                (id, lang.into()),
                TranslationRow {
                    post_id: id,
                    lang: lang.into(),
                    status: status.into(),
                    title: format!("{slug} {lang}"),
                    summary: String::new(),
                    body_md: String::new(),
                    updated_at: published_at.into(),
                    cover_line1: Some(lines[0].into()),
                    cover_line2: Some(lines[1].into()),
                },
            );
        }
    }

    pub fn cal(&self, ticker: &str, date: &str, slot: &str, status: &str) {
        self.es.calendar.borrow_mut().insert(
            (ticker.into(), date.into()),
            CalendarRow {
                ticker: ticker.into(),
                report_date: date.into(),
                time_slot: slot.into(),
                status: status.into(),
                source: if status == "unconfirmed" {
                    "yahoo"
                } else {
                    "ir"
                }
                .into(),
                proof_url: None,
                proof_quote: None,
                conflict_reason: None,
                first_seen_at: "2026-11-01T00:00:00Z".into(),
                checked_at: "2026-11-20T11:00:00Z".into(),
                next_check_at: None,
                updated_at: "2026-11-20T11:00:00Z".into(),
            },
        );
    }

    pub fn event(&self, id: &str, series: &str, date: &str, starts_at: Option<&str>, link: &str) {
        self.ev.rows.borrow_mut().insert(
            id.into(),
            EventRow {
                id: id.into(),
                series: series.into(),
                ref_period: Some("2026-11".into()),
                variant: None,
                starts_at: starts_at.map(String::from),
                et_date: date.into(),
                tier: 1,
                status: "scheduled".into(),
                proof_url: link.into(),
                release_url: None,
                moved_from: None,
                missing_count: 0,
                first_seen_at: "2026-11-01T00:00:00Z".into(),
                checked_at: "2026-11-20T11:00:00Z".into(),
                updated_at: "2026-11-20T11:00:00Z".into(),
            },
        );
    }

    /// The day's calendar refresh has finished.
    pub fn refreshed(&self, day: &str) {
        self.es
            .kv
            .borrow_mut()
            .insert("calendar_refreshed_on".into(), day.into());
    }

    pub async fn try_tick(&self, now: &str) -> Result<String, String> {
        week_tick(
            utc(now),
            &self.es,
            &self.ev,
            self.base.as_ref(),
            self.base.as_ref(),
        )
        .await
    }

    pub async fn tick(&self, now: &str) -> String {
        self.try_tick(now).await.unwrap()
    }

    pub async fn queued(&self, now: &str) -> bool {
        follow_up(Some("true"), utc(now), &self.es, self.base.as_ref())
            .await
            .unwrap()
            == Some(EarningsStep::Week)
    }

    pub fn run(&self, friday: &str) -> Option<RunRow> {
        self.base
            .runs
            .borrow()
            .get(&format!("week:{friday}"))
            .cloned()
    }

    pub fn post(&self, friday: &str) -> Option<PostRow> {
        self.base
            .posts
            .borrow()
            .iter()
            .find(|p| p.run_key == format!("week:{friday}"))
            .cloned()
    }

    pub fn page(&self, friday: &str, lang: &str) -> Option<TranslationRow> {
        let id = self.post(friday)?.id?;
        self.base
            .translations
            .borrow()
            .get(&(id, lang.into()))
            .cloned()
    }

    pub fn bundle(&self, friday: &str) -> Bundle {
        let json = self
            .base
            .facts
            .borrow()
            .get(&format!("week/{friday}.json"))
            .cloned()
            .unwrap_or_else(|| panic!("no bundle for {friday}"));
        serde_json::from_str(&json).unwrap()
    }

    pub fn alerts(&self) -> Vec<(String, String, String)> {
        self.base.alerts.borrow().clone()
    }

    pub fn set_run(&self, friday: &str, f: impl FnOnce(&mut RunRow)) {
        let mut runs = self.base.runs.borrow_mut();
        let r = runs
            .entry(format!("week:{friday}"))
            .or_insert_with(|| job_run(friday));
        f(r);
    }
}

fn job_run(friday: &str) -> RunRow {
    RunRow {
        run_key: format!("week:{friday}"),
        content_type: "week".into(),
        trading_date: friday.into(),
        status: "pending".into(),
        attempts: 0,
        last_error: None,
        created_at: "2026-11-01T00:00:00Z".into(),
        updated_at: "2026-11-01T00:00:00Z".into(),
    }
}

/// The Friday 2026-12-04 world: two write-up checks this week (Chinese
/// published and pending), eight companies next week (three write-up, two
/// confirmed), a quarantined row and a stale row of this week, an event, and
/// the day's calendar refresh done.
fn dec4() -> World {
    let w = world();
    w.check(
        "crm-q3-fy2027-earnings",
        "CRM",
        "2026-12-02",
        "2026-12-02T21:31:00Z",
        ["Revenue $10.4B", "GAAP EPS $1.26"],
        ["营收 103.6亿美元", "GAAP每股收益 1.26美元"],
        "published",
    );
    w.check(
        "mrvl-q3-fy2027-earnings",
        "MRVL",
        "2026-12-02",
        "2026-12-02T21:46:00Z",
        ["Revenue $2.1B", "GAAP EPS $0.21"],
        ["营收 21.2亿美元", "GAAP每股收益 0.21美元"],
        "pending",
    );
    for (t, day, slot, status) in [
        ("ADBE", "2026-12-08", "amc", "confirmed"),
        ("AVGO", "2026-12-10", "amc", "unconfirmed"),
        ("COST", "2026-12-10", "amc", "unconfirmed"),
        ("CPRT", "2026-12-08", "amc", "unconfirmed"),
        ("NIO", "2026-12-08", "amc", "unconfirmed"),
        ("CIEN", "2026-12-10", "bmo", "confirmed"),
        ("LULU", "2026-12-10", "amc", "unconfirmed"),
        ("KR", "2026-12-08", "unknown", "unconfirmed"),
        // Left out: quarantined, and a stale row of this week.
        ("ORCL", "2026-12-09", "amc", "quarantined"),
        ("PFE", "2026-12-03", "bmo", "unconfirmed"),
    ] {
        w.cal(t, day, slot, status);
    }
    w.event(
        "bls:cpi:2026-11",
        "cpi",
        "2026-12-09",
        Some("2026-12-09T13:30:00Z"),
        "https://www.bls.gov/schedule/news_release/cpi.htm",
    );
    w.refreshed("2026-12-04");
    w
}

const DEC4_BUILD: &str = "2026-12-04T23:05:00Z";

/// An English cover line the guard cannot place (it holds a line break), so
/// only the English page fails.
fn broken_check(w: &World) {
    w.check(
        "acn-q1-fy2027-earnings",
        "ACN",
        "2026-12-03",
        "2026-12-03T12:00:00Z",
        ["Revenue $18.7B\nper quarter", "GAAP EPS $3.60"],
        ["营收 187亿美元", "GAAP每股收益 3.60美元"],
        "published",
    );
}

// ---------------------------------------------------------------------------
// Fixtures: a rendered page must equal its file. `UPDATE_WEEK_FIXTURES=1`
// rewrites the files (then read every diff).

fn fixture(name: &str, got: &str) {
    let path = format!("{}/tests/fixtures/week/{name}", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("UPDATE_WEEK_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing fixture {path}"));
    assert_eq!(got, want, "fixture {name} differs:\n{got}");
}

/// Both languages' cover lines and pages, as stored.
fn rendered(w: &World, friday: &str) -> String {
    let mut out = String::new();
    for lang in ["en", "zh"] {
        let t = w.page(friday, lang).unwrap();
        out.push_str(&format!(
            "<!-- {lang} cover lines: {} / {} -->\n",
            t.cover_line1.unwrap_or_default(),
            t.cover_line2.unwrap_or_default()
        ));
        out.push_str(&t.body_md);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Task 1: the rows.

#[tokio::test]
async fn the_first_post_lists_this_weeks_checks_and_next_weeks_calendar() {
    let w = dec4();
    let line = w.tick(DEC4_BUILD).await;
    assert!(
        line.contains("published earnings-week-2026-12-04 (2 checks, 8 expected)"),
        "{line}"
    );
    let b = w.bundle("2026-12-04");
    assert_eq!(
        (b.friday.as_str(), b.next_monday.as_str()),
        ("2026-12-04", "2026-12-07")
    );
    assert_eq!(b.since, "2026-11-30");
    assert_eq!(b.cutoff, DEC4_BUILD);
    assert_eq!(b.as_of, DEC4_BUILD);
    assert_eq!(
        b.listed,
        ["crm-q3-fy2027-earnings", "mrvl-q3-fy2027-earnings"]
    );
    assert_eq!(b.window, Some(["2026-12-02".into(), "2026-12-02".into()]));
    assert!(!b
        .expected
        .iter()
        .any(|e| e.ticker == "ORCL" || e.ticker == "PFE"));
    assert_eq!(
        b.counts,
        Counts {
            checks: 2,
            expected: 8,
            confirmed: 2,
            writeups: 3,
            per_day: [0, 4, 0, 4, 0],
        }
    );
    // Every row keeps where its date came from.
    let adbe = b.expected.iter().find(|e| e.ticker == "ADBE").unwrap();
    assert_eq!(
        (adbe.source.as_str(), adbe.checked_at.as_str()),
        ("ir", "2026-11-20T11:00:00Z")
    );
    let post = w.post("2026-12-04").unwrap();
    assert_eq!(
        (
            post.content_type.as_str(),
            post.template_id.as_str(),
            post.slug.as_str()
        ),
        ("week", "week-v1", "earnings-week-2026-12-04")
    );
    assert_eq!(
        (post.trading_date.as_str(), post.cover_key.as_deref()),
        ("2026-12-04", None)
    );
    assert_eq!(post.published_at.as_deref(), Some(DEC4_BUILD));
    assert_eq!(post.revised_at, None);
    // The event's official page is the post's source, so the site links it.
    let sources = w
        .base
        .sources
        .borrow()
        .get(&post.id.unwrap())
        .cloned()
        .unwrap();
    let urls: Vec<&str> = sources.iter().map(|s| s.url.as_str()).collect();
    assert_eq!(urls, ["https://www.bls.gov/schedule/news_release/cpi.htm"]);
    fixture("2026-12-04.md", &rendered(&w, "2026-12-04"));
    // The site renders these two pages in its own tests: keep them equal
    // (checked once the site's fixtures are in the tree).
    for lang in ["en", "zh"] {
        site_fixture(
            &format!("week-2026-12-04-{lang}.md"),
            &w.page("2026-12-04", lang).unwrap().body_md,
        );
    }
}

/// A page the site's tests render (`apps/web/tests/fixtures/<name>`): equal
/// to the generator's output, or rewritten with `UPDATE_WEEK_FIXTURES=1`.
fn site_fixture(name: &str, got: &str) {
    let path = format!(
        "{}/../../apps/web/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var("UPDATE_WEEK_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&path, got).unwrap();
    } else if let Ok(s) = std::fs::read_to_string(&path) {
        assert_eq!(s, got, "{path} is stale");
    }
}

#[tokio::test]
async fn a_check_published_after_the_build_appears_in_the_next_post_only() {
    let w = dec4();
    w.tick(DEC4_BUILD).await;
    // Reported Dec 4, published at 18:10 ET, after the 18:05 build.
    w.check(
        "dri-q2-fy2027-earnings",
        "DRI",
        "2026-12-04",
        "2026-12-04T23:10:00Z",
        ["Revenue $3.1B", "GAAP EPS $2.10"],
        ["营收 31亿美元", "GAAP每股收益 2.10美元"],
        "published",
    );
    assert!(!w
        .bundle("2026-12-04")
        .listed
        .iter()
        .any(|s| s.starts_with("dri-")));
    for (slug, t, day, at) in [
        (
            "adbe-q4-fy2026-earnings",
            "ADBE",
            "2026-12-08",
            "2026-12-08T21:10:00Z",
        ),
        (
            "avgo-q4-fy2026-earnings",
            "AVGO",
            "2026-12-10",
            "2026-12-10T21:20:00Z",
        ),
    ] {
        w.check(
            slug,
            t,
            day,
            at,
            ["Revenue $6.2B", "GAAP EPS $4.10"],
            ["营收 62亿美元", "GAAP每股收益 4.10美元"],
            "published",
        );
    }
    w.refreshed("2026-12-11");
    w.tick("2026-12-11T23:05:00Z").await;
    let b = w.bundle("2026-12-11");
    assert_eq!(
        b.listed,
        [
            "avgo-q4-fy2026-earnings",
            "adbe-q4-fy2026-earnings",
            "dri-q2-fy2027-earnings"
        ]
    );
    // From the previous post's Monday; Darden's report of Dec 4 names its date.
    assert_eq!(b.since, "2026-11-30");
    assert_eq!(b.range(), (d("2026-12-04"), d("2026-12-11")));
    let en = w.page("2026-12-11", "en").unwrap();
    assert!(
        en.body_md
            .contains("## Earnings checks published Dec 4–Dec 11"),
        "{}",
        en.body_md
    );
}

/// A check published at 18:02 ET is in an 18:05 build: builds share the
/// serial queue, so one in flight finishes first.
#[tokio::test]
async fn a_check_published_at_18_02_is_in_the_18_05_build() {
    let w = dec4();
    w.check(
        "orcl-q2-fy2027-earnings",
        "ORCL",
        "2026-12-04",
        "2026-12-04T23:02:00Z",
        ["Revenue $16.1B", "GAAP EPS $1.47"],
        ["营收 161亿美元", "GAAP每股收益 1.47美元"],
        "published",
    );
    w.tick(DEC4_BUILD).await;
    assert!(w
        .bundle("2026-12-04")
        .listed
        .contains(&"orcl-q2-fy2027-earnings".to_string()));
}

#[tokio::test]
async fn a_check_rebuilt_or_revised_after_it_was_listed_is_never_listed_twice() {
    let w = dec4();
    w.tick(DEC4_BUILD).await;
    // CRM's check is rebuilt (a new published_at), MRVL's revised.
    {
        let mut posts = w.base.posts.borrow_mut();
        let crm = posts
            .iter_mut()
            .find(|p| p.slug == "crm-q3-fy2027-earnings")
            .unwrap();
        crm.published_at = Some("2026-12-07T15:00:00Z".into());
        let mrvl = posts
            .iter_mut()
            .find(|p| p.slug == "mrvl-q3-fy2027-earnings")
            .unwrap();
        mrvl.revised_at = Some("2026-12-08T15:00:00Z".into());
    }
    // Two write-up companies the week after, so Dec 11 publishes.
    w.cal("ORCL", "2026-12-15", "amc", "confirmed");
    w.cal("MU", "2026-12-17", "amc", "unconfirmed");
    w.refreshed("2026-12-11");
    w.tick("2026-12-11T23:05:00Z").await;
    let b = w.bundle("2026-12-11");
    assert!(b.listed.is_empty(), "{:?}", b.listed);
    // ...and the week that listed them is flagged once.
    let stale: Vec<_> = w
        .alerts()
        .into_iter()
        .filter(|a| a.0.starts_with("week-stale"))
        .collect();
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert_eq!(
        (stale[0].0.as_str(), stale[0].1.as_str()),
        ("week-stale:2026-12-04", "warn")
    );
    assert!(
        stale[0]
            .2
            .contains("crm-q3-fy2027-earnings, mrvl-q3-fy2027-earnings"),
        "{stale:?}"
    );
    // A later Friday does not raise it again.
    w.refreshed("2026-12-18");
    w.tick("2026-12-18T23:05:00Z").await;
    assert_eq!(
        w.alerts()
            .iter()
            .filter(|a| a.0.starts_with("week-stale"))
            .count(),
        1
    );
}

#[tokio::test]
async fn a_failed_week_is_covered_by_the_next_post_under_its_dates() {
    let w = dec4();
    for (slug, t, day) in [
        ("dell-q3-fy2027-earnings", "DELL", "2026-11-24"),
        ("nke-q2-fy2027-earnings", "NKE", "2026-11-25"),
    ] {
        w.check(
            slug,
            t,
            day,
            &format!("{day}T21:10:00Z"),
            ["Revenue $24.4B", "GAAP EPS $1.70"],
            ["营收 244亿美元", "GAAP每股收益 1.70美元"],
            "published",
        );
    }
    w.refreshed("2026-11-27");
    w.tick("2026-11-27T23:05:00Z").await;
    assert_eq!(w.run("2026-11-27").unwrap().status, "published");
    // Dec 4 failed: no post.
    w.set_run("2026-12-04", |r| r.status = "failed".into());
    w.cal("ORCL", "2026-12-15", "amc", "confirmed");
    w.refreshed("2026-12-11");
    w.tick("2026-12-11T23:05:00Z").await;
    let b = w.bundle("2026-12-11");
    assert_eq!(b.since, "2026-11-23");
    assert_eq!(
        b.listed,
        ["crm-q3-fy2027-earnings", "mrvl-q3-fy2027-earnings"]
    );
    let en = w.page("2026-12-11", "en").unwrap().body_md;
    assert!(
        en.contains("## Earnings checks published Dec 2–Dec 11"),
        "{en}"
    );
    let zh = w.page("2026-12-11", "zh").unwrap().body_md;
    assert!(zh.contains("## 12月2日至12月11日发布的财报速评"), "{zh}");
}

#[tokio::test]
async fn failed_and_pending_chinese_translations() {
    let w = dec4();
    w.check(
        "emr-q1-fy2027-earnings",
        "EMR",
        "2026-12-03",
        "2026-12-03T12:10:00Z",
        ["Revenue $4.5B", "Non-GAAP EPS $1.40"],
        ["营收 45亿美元", "非GAAP每股收益 1.40美元"],
        "failed",
    );
    w.tick(DEC4_BUILD).await;
    let zh = w.page("2026-12-04", "zh").unwrap().body_md;
    // Failed: its Chinese lines, linked to the English check, marked.
    assert!(
        zh.contains(&format!(
            "- [{}](/en/p/emr-q1-fy2027-earnings)（英文） · 2027财年第一季度 · 营收 45亿美元 · 非GAAP每股收益 1.40美元",
            lbl("EMR")
        )),
        "{zh}"
    );
    // Pending: the Chinese page, which shows the coming-soon notice.
    assert!(zh.contains("](/zh/p/mrvl-q3-fy2027-earnings) · "), "{zh}");
    let en = w.page("2026-12-04", "en").unwrap().body_md;
    assert!(
        en.contains("](/en/p/emr-q1-fy2027-earnings) · Q1 fiscal 2027 · "),
        "{en}"
    );
    assert!(!en.contains("（英文）"));
}

#[tokio::test]
async fn a_cover_line_without_a_digit_is_left_out_and_two_checks_on_one_date_both_show() {
    let w = dec4();
    w.check(
        "aig-q4-fy2026-earnings",
        "AIG",
        "2026-12-02",
        "2026-12-02T21:50:00Z",
        ["American International Group results", "GAAP EPS $1.60"],
        ["American International Group业绩", "GAAP每股收益 1.60美元"],
        "published",
    );
    w.tick(DEC4_BUILD).await;
    let en = w.page("2026-12-04", "en").unwrap().body_md;
    assert!(
        en.contains("](/en/p/aig-q4-fy2026-earnings) · Q4 fiscal 2026 · GAAP EPS $1.60\n"),
        "{en}"
    );
    assert!(!en.contains("American International Group results"));
    let b = w.bundle("2026-12-04");
    assert_eq!(
        b.checks
            .iter()
            .filter(|c| c.trading_date == "2026-12-02")
            .count(),
        3
    );
    assert_eq!(en.matches("](/en/p/").count(), 3);
}

#[tokio::test]
async fn checks_go_largest_first_and_ant_with_no_figure_sorts_last() {
    let w = dec4();
    w.check(
        "ant-q4-fy2026-earnings",
        "ANT",
        "2026-12-03",
        "2026-12-03T21:10:00Z",
        ["Revenue $9.0B", "GAAP EPS $0.10"],
        ["营收 90亿美元", "GAAP每股收益 0.10美元"],
        "published",
    );
    w.check(
        "nvda-q4-fy2027-earnings",
        "NVDA",
        "2026-12-03",
        "2026-12-03T21:20:00Z",
        ["Revenue $120.0B", "GAAP EPS $2.80"],
        ["营收 1200亿美元", "GAAP每股收益 2.80美元"],
        "published",
    );
    w.tick(DEC4_BUILD).await;
    let b = w.bundle("2026-12-04");
    let order: Vec<(&str, usize)> = b
        .checks
        .iter()
        .map(|c| (c.ticker.as_str(), c.rank))
        .collect();
    assert_eq!(order, [("NVDA", 1), ("CRM", 2), ("MRVL", 3), ("ANT", 4)]);
    assert_eq!(b.checks[3].cap, 0);
    assert_eq!(
        w.page("2026-12-04", "en").unwrap().title,
        "Earnings this week: Nvidia, Salesforce, Marvell Technology; 8 expected next week"
    );
    assert_eq!(
        w.page("2026-12-04", "zh").unwrap().title,
        "本周财报：Nvidia、Salesforce、Marvell Technology；下周预计8家"
    );
}

#[tokio::test]
async fn mixed_statuses_in_one_slot_and_a_reported_row_next_week() {
    let w = dec4();
    w.cal("CASY", "2026-12-08", "amc", "confirmed");
    w.cal("DRI", "2026-12-08", "amc", "reported");
    w.cal("ORCL", "2026-12-08", "amc", "reported");
    w.tick(DEC4_BUILD).await;
    let en = w.page("2026-12-04", "en").unwrap().body_md;
    let part = |head: &str| {
        en.split(head)
            .nth(1)
            .unwrap()
            .split("### ")
            .next()
            .unwrap()
            .to_string()
    };
    assert!(part("### Mon, Dec 7").contains("No reports expected."));
    let tue = part("### Tue, Dec 8");
    assert!(
        tue.contains(&format!("- [{}](/en/t/ADBE): confirmed\n", lbl("ADBE"))),
        "{tue}"
    );
    assert!(
        tue.contains(&format!("- [{}](/en/t/ORCL): reported\n", lbl("ORCL"))),
        "{tue}"
    );
    assert!(
        tue.contains(&format!("- Confirmed: [{}](/en/t/CASY)\n", lbl("CASY"))),
        "{tue}"
    );
    assert!(
        tue.contains(&format!(
            "- Not confirmed: [{}](/en/t/CPRT), [{}](/en/t/NIO)\n",
            lbl("CPRT"),
            lbl("NIO")
        )),
        "{tue}"
    );
    assert!(
        tue.contains(&format!("- Reported: [{}](/en/t/DRI)", lbl("DRI"))),
        "{tue}"
    );
    assert!(
        tue.contains("**Time n/a**\n\n- Not confirmed: [Kroger (KR)](/en/t/KR)"),
        "{tue}"
    );
    let zh = w.page("2026-12-04", "zh").unwrap().body_md;
    assert!(
        zh.contains(&format!("- [{}](/zh/t/ORCL)：已发布", lbl("ORCL"))),
        "{zh}"
    );
    assert!(
        zh.contains(&format!(
            "- 未确认：[{}](/zh/t/CPRT)、[{}](/zh/t/NIO)",
            lbl("CPRT"),
            lbl("NIO")
        )),
        "{zh}"
    );
}

// ---------------------------------------------------------------------------
// Task 2: the page.

/// A bundle from parts, for the template variants.
fn bundle_with(
    checks: usize,
    writeups: usize,
    others: usize,
    confirmed: usize,
    as_of: &str,
) -> Bundle {
    let names = [
        ("NVDA", "nvda-q3-fy2027-earnings"),
        ("ADBE", "adbe-q4-fy2026-earnings"),
        ("AVGO", "avgo-q4-fy2026-earnings"),
        ("COST", "cost-q1-fy2027-earnings"),
    ];
    let rows: Vec<CheckRow> = names
        .iter()
        .take(checks)
        .map(|(t, s)| CheckRow {
            slug: s.to_string(),
            run_key: format!("earnings:{t}:x"),
            trading_date: "2026-12-10".into(),
            published_at: "2026-12-10T21:00:00Z".into(),
            revised_at: None,
            en: [Some("Revenue $1.2B".into()), Some("GAAP EPS $0.34".into())],
            zh: [
                Some("营收 12亿美元".into()),
                Some("GAAP每股收益 0.34美元".into()),
            ],
            zh_status: Some("published".into()),
        })
        .collect();
    let co = |t: &str, status: &str| crate::digest::Company {
        ticker: t.into(),
        name: crate::earnings::find(t).map_or(t.to_string(), |w| w.name.to_string()),
        confirmed: status != "unconfirmed",
        status: status.into(),
        source: "yahoo".into(),
        proof_url: None,
        checked_at: "2026-12-11T11:00:00Z".into(),
    };
    let mut days: Vec<crate::digest::Day> = (0..5)
        .map(|i| crate::digest::Day {
            date: d("2026-12-14") + Duration::days(i),
            events: vec![],
            bmo: Default::default(),
            amc: Default::default(),
            unknown: Default::default(),
        })
        .collect();
    let mut left = confirmed;
    let mut status = move || {
        if left > 0 {
            left -= 1;
            "confirmed"
        } else {
            "unconfirmed"
        }
    };
    for t in ["ORCL", "CRM", "MRVL"].iter().take(writeups) {
        let s = status();
        days[1].amc.writeups.push(co(t, s));
    }
    for t in ["CPRT", "NIO", "LULU", "KR", "CIEN"].iter().take(others) {
        let s = status();
        days[2].bmo.rest.push(co(t, s));
        days[2].bmo.others += 1;
    }
    assemble(
        &Stamp {
            friday: d("2026-12-11"),
            since: d("2026-12-07"),
            cutoff: utc(as_of),
            as_of: utc(as_of),
            revision: 0,
        },
        super::checks(rows, &[]),
        &days,
    )
}

#[test]
fn every_title_summary_and_cover_variant() {
    let at = "2026-12-11T23:05:00Z";
    let mut out = String::new();
    for (label, b) in [
        (
            "3 or more checks, with companies expected",
            bundle_with(4, 2, 5, 1, at),
        ),
        (
            "2 checks, 1 expected, none confirmed",
            bundle_with(2, 1, 0, 0, at),
        ),
        (
            "1 check, 2 expected, both confirmed",
            bundle_with(1, 2, 0, 2, at),
        ),
        (
            "checks, none expected next week",
            bundle_with(3, 0, 0, 0, at),
        ),
        ("no check, 8 expected", bundle_with(0, 3, 5, 0, at)),
        ("no check, 1 expected", bundle_with(0, 1, 0, 0, at)),
    ] {
        out.push_str(&format!("## {label}\n"));
        for lang in [Lang::En, Lang::Zh] {
            let [c1, c2] = cover_lines(&b, lang);
            out.push_str(&format!(
                "{}\n{}\n{c1} | {c2}\n",
                title_of(&b, lang),
                summary_of(&b, lang)
            ));
            // Every variant passes the guard.
            let (_, md) = write(&b, lang);
            assert_eq!(
                verify_page(&b, lang, &md, &cover_lines(&b, lang)),
                Ok(()),
                "{label} {lang:?}\n{md}"
            );
        }
        out.push('\n');
    }
    fixture("variants.txt", &out);
}

/// The as-of time in EST and EDT, and a correction's long form.
#[test]
fn the_as_of_time_in_est_edt_and_after_a_correction() {
    let est = bundle_with(2, 2, 0, 0, "2026-12-04T23:05:00Z");
    assert!(
        body(&est, Lang::En, "t", "s").contains("## Expected next week, as of Fri Dec 4, 18:05 ET")
    );
    assert!(
        body(&est, Lang::Zh, "t", "s").contains("## 下周预计公布（截至美东时间12月4日周五18:05）")
    );
    let mut edt = bundle_with(2, 2, 0, 0, "2027-03-19T22:05:00Z");
    edt.friday = "2027-03-19".into();
    assert!(body(&edt, Lang::En, "t", "s").contains("as of Fri Mar 19, 18:05 ET"));
    assert!(summary_of(&edt, Lang::En).contains("as of Friday 18:05 ET"));
    assert!(summary_of(&edt, Lang::Zh).contains("截至美东时间周五18:05"));
    let mut fix = bundle_with(2, 2, 0, 0, "2026-12-11T23:05:00Z");
    fix.as_of = "2026-12-15T15:05:00Z".into();
    assert!(summary_of(&fix, Lang::En).contains("as of Tue Dec 15, 10:05 ET"));
    assert!(summary_of(&fix, Lang::Zh).contains("截至美东时间12月15日周二10:05"));
}

#[test]
fn the_source_sentence_is_the_calendars_in_both_languages() {
    let en: Value =
        serde_json::from_str(include_str!("../../../../apps/web/messages/en.json")).unwrap();
    let zh: Value =
        serde_json::from_str(include_str!("../../../../apps/web/messages/zh.json")).unwrap();
    let legend = |v: &Value| v["earnings"]["legend"].as_str().unwrap().to_string();
    // The web legend is this sentence, then the tier sentence only.
    let (en_l, zh_l) = (legend(&en), legend(&zh));
    let en_tier = en_l.strip_prefix(&format!("{LEGEND_EN} ")).unwrap();
    let zh_tier = zh_l.strip_prefix(LEGEND_ZH).unwrap();
    assert_eq!(
        en_tier.matches(". ").count() + usize::from(en_tier.ends_with('.')),
        1,
        "{en_tier}"
    );
    assert_eq!(zh_tier.matches('。').count(), 1, "{zh_tier}");
    let b = bundle_with(2, 2, 0, 0, "2026-12-11T23:05:00Z");
    assert!(body(&b, Lang::En, "t", "s").contains(LEGEND_EN));
    assert!(body(&b, Lang::Zh, "t", "s").contains(LEGEND_ZH));
}

#[test]
fn nothing_compares_with_estimates() {
    let words = [
        "beat",
        "beats",
        "miss",
        "missed",
        "misses",
        "estimate",
        "estimates",
        "consensus",
    ];
    let whole = |text: &str, w: &str| {
        text.match_indices(w).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = text[i + w.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    };
    let src = include_str!("render.rs").to_lowercase();
    let mut pages = String::new();
    for (c, wu, o, k) in [(4, 3, 5, 2), (0, 2, 0, 0), (1, 1, 1, 1)] {
        let b = bundle_with(c, wu, o, k, "2026-12-11T23:05:00Z");
        for lang in [Lang::En, Lang::Zh] {
            pages.push_str(&write(&b, lang).1.to_lowercase());
        }
    }
    for text in [&src, &pages] {
        for w in words {
            assert!(!whole(text, w), "{w}");
        }
        assert!(!text.contains("预期"));
    }
    // Word boundaries: "missing" and "commission" are not "miss".
    assert!(!whole("missing commission", "miss"));
}

// ---------------------------------------------------------------------------
// The guard.

fn guarded(b: &Bundle, lang: Lang, edit: impl Fn(&str) -> String) -> Result<(), Vec<String>> {
    let (_, md) = write(b, lang);
    verify_page(b, lang, &edit(&md), &cover_lines(b, lang))
}

/// Dec 4 with 3M's check too: a write-up company with a digit in its name.
async fn dec4_bundle() -> Bundle {
    let w = dec4();
    w.check(
        "mmm-q4-fy2026-earnings",
        "MMM",
        "2026-12-03",
        "2026-12-03T12:00:00Z",
        ["Revenue $6.1B", "GAAP EPS $2.10"],
        ["营收 61亿美元", "GAAP每股收益 2.10美元"],
        "published",
    );
    w.tick(DEC4_BUILD).await;
    w.bundle("2026-12-04")
}

#[tokio::test]
async fn the_guard_accepts_the_page_as_written_with_3m_as_a_name() {
    let b = dec4_bundle().await;
    assert!(b.checks.iter().any(|c| c.name == "3M"), "{:?}", b.checks);
    for lang in [Lang::En, Lang::Zh] {
        assert_eq!(guarded(&b, lang, str::to_string), Ok(()), "{lang:?}");
    }
    assert!(title_of(&b, Lang::En).contains("3M"));
    assert!(title_of(&b, Lang::Zh).contains("3M"));
}

#[tokio::test]
async fn the_guard_refuses_a_page_that_is_not_the_bundle() {
    let b = dec4_bundle().await;
    let refused = |lang, from: &str, to: &str| -> String {
        let r = guarded(&b, lang, |md| {
            assert!(md.contains(from), "{from:?} not on the page:\n{md}");
            md.replacen(from, to, 1)
        });
        match r {
            Err(e) => e.join("; "),
            Ok(()) => panic!("{from:?} -> {to:?} passed"),
        }
    };
    // A cover line with a changed digit, or changed by any one character.
    assert!(refused(Lang::En, "Revenue $10.4B", "Revenue $10.5B").contains("check line"));
    assert!(refused(Lang::En, "Revenue $10.4B", "Revenue $10.4b").contains("check line"));
    assert!(refused(Lang::En, "GAAP EPS $1.26", "GAAP EPS $1.26 ").contains("check line"));
    assert!(refused(Lang::Zh, "营收 103.6亿美元", "营收 103.7亿美元").contains("check line"));
    // A stored line under the wrong company.
    assert!(refused(Lang::En, "Revenue $2.1B", "Revenue $10.4B").contains("check line"));
    // A planted number, a date not in the bundle, a time.
    assert!(refused(
        Lang::En,
        "No reports expected.",
        "No reports expected, 12 later."
    )
    .contains("number not in the bundle"));
    assert!(
        refused(Lang::En, "### Tue, Dec 8", "### Tue, Dec 18").contains("date not in the bundle")
    );
    assert!(
        refused(Lang::Zh, "### 12月8日（周二）", "### 12月18日（周二）")
            .contains("date not in the bundle")
    );
    assert!(refused(Lang::En, "18:05 ET", "18:06 ET").contains("time not in the bundle"));
    assert!(refused(Lang::Zh, "周五18:05", "周五18:06").contains("time not in the bundle"));
    // A wrong fiscal year or quarter, in each language's wording.
    assert!(refused(Lang::En, "Q3 fiscal 2027", "Q3 fiscal 2028").contains("check line"));
    assert!(refused(Lang::En, "Q3 fiscal 2027", "Q4 fiscal 2027").contains("check line"));
    assert!(refused(Lang::Zh, "2027财年第三季度", "2028财年第三季度").contains("check line"));
    assert!(refused(Lang::Zh, "2027财年第三季度", "2027财年第四季度").contains("check line"));
    // A wrong date in the summary's range, in each language.
    assert!(refused(
        Lang::En,
        "published Nov 30–Dec 4.",
        "published Nov 30–Dec 5."
    )
    .contains("date not in the bundle"));
    assert!(refused(
        Lang::Zh,
        "11月30日至12月4日发布）",
        "11月30日至12月5日发布）"
    )
    .contains("date not in the bundle"));
    // A cover line under the wrong company, in Chinese too.
    assert!(refused(Lang::Zh, "营收 21.2亿美元", "营收 103.6亿美元").contains("check line"));
    // A count off by one, in each phrase that states one.
    assert!(
        refused(Lang::En, "; 8 expected next week", "; 9 expected next week")
            .contains("count not in the bundle")
    );
    assert!(refused(
        Lang::En,
        "checked 3 earnings reports",
        "checked 4 earnings reports"
    )
    .contains("count"));
    assert!(refused(
        Lang::En,
        "8 companies are expected",
        "7 companies are expected"
    )
    .contains("count"));
    assert!(refused(Lang::En, "2 dates are confirmed", "3 dates are confirmed").contains("count"));
    assert!(refused(Lang::Zh, "下周预计8家", "下周预计9家").contains("count"));
    assert!(refused(Lang::Zh, "核对了3份财报", "核对了2份财报").contains("count"));
    assert!(refused(Lang::Zh, "其中2家日期已确认", "其中1家日期已确认").contains("count"));
    // Links: a target and a label not in the bundle; a check listed twice.
    assert!(refused(Lang::En, "(/en/t/CPRT)", "(/en/t/CPRX)").contains("link not in the bundle"));
    assert!(refused(Lang::En, "[Copart (CPRT)]", "[Copart (CPRX)]").contains("link label"));
    let crm = b.checks.iter().find(|c| c.ticker == "CRM").unwrap();
    let line = format!("- [{}](/en/p/{}) · ", lbl("CRM"), crm.slug);
    let doubled = guarded(&b, Lang::En, |md| {
        let l = md
            .lines()
            .find(|l| l.starts_with(&line))
            .unwrap()
            .to_string();
        md.replacen(&l, &format!("{l}\n{l}"), 1)
    });
    assert!(doubled.unwrap_err().join("; ").contains("listed 2 times"));
    // The cover lines are checked too: a small count off by one ("9" for 8).
    let mut cover = cover_lines(&b, Lang::En);
    assert_eq!(cover[1], "8 expected next week");
    cover[1] = "9 expected next week".into();
    let e = verify_page(&b, Lang::En, &write(&b, Lang::En).1, &cover).unwrap_err();
    assert!(
        e.join("; ")
            .contains("count not in the bundle: 9 expected next week"),
        "{e:?}"
    );
    let mut cover = cover_lines(&b, Lang::Zh);
    assert_eq!(cover[1], "下周预计 8 家");
    cover[1] = "下周预计 9 家".into();
    assert!(verify_page(&b, Lang::Zh, &write(&b, Lang::Zh).1, &cover).is_err());
}

/// Neither language publishes unless both pass: an English cover line the
/// guard cannot place fails the English page only, and nothing is written.
#[tokio::test]
async fn neither_language_publishes_unless_both_pass() {
    let w = dec4();
    broken_check(&w);
    let line = w.tick(DEC4_BUILD).await;
    assert!(
        line.contains("retrying: en: check line not as stored"),
        "{line}"
    );
    assert!(!line.contains("zh:"), "{line}");
    assert!(w.post("2026-12-04").is_none());
    assert!(w.base.facts.borrow().is_empty());
    let run = w.run("2026-12-04").unwrap();
    assert_eq!((run.status.as_str(), run.attempts), ("pending", 1));
}

// ---------------------------------------------------------------------------
// The job.

#[tokio::test]
async fn friday_evening_after_the_refresh_only() {
    let w = dec4();
    // Before 18:00 ET, and on a Thursday: nothing.
    assert!(!w.queued("2026-12-04T22:55:00Z").await);
    assert!(!w.queued("2026-12-03T23:05:00Z").await);
    // From 18:00 ET once the day's refresh finished.
    w.refreshed("2026-12-03");
    assert!(!w.queued(DEC4_BUILD).await);
    assert_eq!(
        w.tick(DEC4_BUILD).await,
        "week:2026-12-04: waiting for today's calendar refresh to finish"
    );
    // From 21:45 ET whatever the refresh.
    assert!(w.queued("2026-12-05T02:45:00Z").await);
    w.refreshed("2026-12-04");
    assert!(w.queued(DEC4_BUILD).await);
    // A stray message on a Thursday builds nothing.
    assert_eq!(w.tick("2026-12-03T23:05:00Z").await, "nothing to build");
}

#[tokio::test]
async fn holiday_fridays_follow_the_minimum() {
    let w = world();
    for (slug, t, day) in [
        ("orcl-q2-fy2027-earnings", "ORCL", "2026-12-15"),
        ("mu-q1-fy2027-earnings", "MU", "2026-12-16"),
    ] {
        w.check(
            slug,
            t,
            day,
            &format!("{day}T21:10:00Z"),
            ["Revenue $9.3B", "GAAP EPS $1.91"],
            ["营收 93亿美元", "GAAP每股收益 1.91美元"],
            "published",
        );
    }
    w.refreshed("2026-12-18");
    w.tick("2026-12-18T23:05:00Z").await;
    assert_eq!(w.run("2026-12-18").unwrap().status, "published");
    // Dec 25 (market closed): polls still run, and one check is below the
    // minimum. Skipped, logged, no alert.
    w.check(
        "fdx-q2-fy2027-earnings",
        "FDX",
        "2026-12-23",
        "2026-12-23T21:31:00Z",
        ["Revenue $22.2B", "GAAP EPS $4.03"],
        ["营收 222亿美元", "GAAP每股收益 4.03美元"],
        "published",
    );
    w.cal("NKE", "2026-12-29", "amc", "unconfirmed");
    w.refreshed("2026-12-25");
    assert!(w.queued("2026-12-25T23:05:00Z").await);
    let line = w.tick("2026-12-25T23:05:00Z").await;
    assert!(
        line.contains(
            "skipped: below the minimum: 1 checks, 1 write-up companies expected next week"
        ),
        "{line}"
    );
    assert_eq!(w.run("2026-12-25").unwrap().status, "skipped");
    assert!(w.post("2026-12-25").is_none());
    assert!(w.alerts().is_empty(), "{:?}", w.alerts());
    // Skipped is never queued again.
    assert!(!w.queued("2026-12-25T23:10:00Z").await);
    // Jan 1, 2027 (closed too): FedEx's check rides along into the next post.
    w.check(
        "stz-q3-fy2027-earnings",
        "STZ",
        "2026-12-30",
        "2026-12-30T21:31:00Z",
        ["Revenue $2.5B", "GAAP EPS $3.20"],
        ["营收 25亿美元", "GAAP每股收益 3.20美元"],
        "published",
    );
    w.refreshed("2027-01-01");
    let line = w.tick("2027-01-01T23:05:00Z").await;
    assert!(
        line.contains("published earnings-week-2027-01-01 (2 checks"),
        "{line}"
    );
    let b = w.bundle("2027-01-01");
    assert_eq!(b.since, "2026-12-14");
    assert_eq!(b.listed.len(), 2);
    assert!(b.listed.contains(&"fdx-q2-fy2027-earnings".to_string()));
    fixture("2027-01-01.md", &rendered(&w, "2027-01-01"));
}

#[tokio::test]
async fn a_failure_retries_then_fails_with_an_alert_and_is_never_queued_again() {
    let w = dec4();
    broken_check(&w);
    let mut now = utc(DEC4_BUILD);
    for i in 0..MAX_BUILDS {
        assert!(w.queued(&iso(now)).await, "build {i}");
        let line = w.tick(&iso(now)).await;
        let run = w.run("2026-12-04").unwrap();
        if i + 1 < MAX_BUILDS {
            assert_eq!(
                (run.status.as_str(), run.attempts),
                ("pending", i + 1),
                "{line}"
            );
            // Waiting out its pause: not queued meanwhile.
            assert!(!w.queued(&iso(now + Duration::minutes(1))).await);
            now += Duration::minutes(BACKOFF_MINUTES[(i as usize).min(3)]);
        } else {
            assert_eq!(run.status, "failed", "{line}");
        }
    }
    let warns = w.alerts().into_iter().filter(|a| a.1 == "warn").count();
    assert_eq!(warns, 1, "one retry warning a day: {:?}", w.alerts());
    let errors: Vec<_> = w.alerts().into_iter().filter(|a| a.1 == "error").collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0]
            .2
            .starts_with("week post for 2026-12-04 not published: en: check line"),
        "{errors:?}"
    );
    // Failed is never queued again, even by a later poll that evening.
    assert!(!w.queued(&iso(now + Duration::minutes(5))).await);
    assert!(w.post("2026-12-04").is_none());
}

/// A build fails at 21:50 ET: its retry would wait past 22:00, so the 21:55
/// poll queues the message anyway, and the week alerts once.
#[tokio::test]
async fn the_last_try_alerts_once() {
    let w = dec4();
    broken_check(&w);
    w.tick("2026-12-05T02:50:00Z").await;
    assert_eq!(w.run("2026-12-04").unwrap().status, "pending");
    assert!(w.queued("2026-12-05T02:55:00Z").await);
    let line = w.tick("2026-12-05T02:55:00Z").await;
    assert!(
        line.contains("alerted, not published by the last try"),
        "{line}"
    );
    w.tick("2026-12-05T02:58:00Z").await;
    let missed: Vec<_> = w
        .alerts()
        .into_iter()
        .filter(|a| a.0.starts_with("week-missed"))
        .collect();
    assert_eq!(missed.len(), 1, "{missed:?}");
    assert_eq!(missed[0].1, "error");
    assert_eq!(missed[0].2, "week post for 2026-12-04 not published");
    // A published week raises nothing at 21:55.
    let w = dec4();
    w.tick(DEC4_BUILD).await;
    w.tick("2026-12-05T02:56:00Z").await;
    assert!(!w.alerts().iter().any(|a| a.0.starts_with("week-missed")));
}

/// A Tuesday correction rebuilds last Friday's post with last Friday's
/// dates, through `revise_post`: `revised_at` set, `published_at` kept, the
/// revision counted (the card task names the new cover stem from it).
#[tokio::test]
async fn a_tuesday_correction_rebuilds_last_fridays_post() {
    let w = dec4();
    w.tick(DEC4_BUILD).await;
    let first = w.post("2026-12-04").unwrap();
    let before = w.bundle("2026-12-04");
    assert!(!w.queued("2026-12-08T15:00:00Z").await);
    // CRM's check is revised (a new figure), the owner fixes a calendar row
    // (Costco confirmed) and sets the run back.
    {
        let mut posts = w.base.posts.borrow_mut();
        let crm = posts
            .iter_mut()
            .find(|p| p.slug == "crm-q3-fy2027-earnings")
            .unwrap();
        crm.revised_at = Some("2026-12-07T12:00:00Z".into());
        let id = crm.id.unwrap();
        let mut t = w.base.translations.borrow_mut();
        t.get_mut(&(id, "en".into())).unwrap().cover_line1 = Some("Revenue $10.5B".into());
    }
    w.cal("COST", "2026-12-10", "amc", "confirmed");
    w.set_run("2026-12-04", |r| {
        r.status = "pending".into();
        r.attempts = 0;
    });
    // A check published since must not join the corrected week.
    w.check(
        "adbe-q4-fy2026-earnings",
        "ADBE",
        "2026-12-07",
        "2026-12-07T21:10:00Z",
        ["Revenue $6.2B", "GAAP EPS $4.10"],
        ["营收 62亿美元", "GAAP每股收益 4.10美元"],
        "published",
    );
    assert!(w.queued("2026-12-08T15:05:00Z").await);
    let line = w.tick("2026-12-08T15:05:00Z").await;
    assert!(
        line.contains("week:2026-12-04: revised earnings-week-2026-12-04 (revision 1)"),
        "{line}"
    );
    let post = w.post("2026-12-04").unwrap();
    assert_eq!(
        (post.id, &post.published_at),
        (first.id, &first.published_at)
    );
    assert_eq!(post.revised_at.as_deref(), Some("2026-12-08T15:05:00Z"));
    let after = w.bundle("2026-12-04");
    assert_eq!(
        (after.friday.as_str(), after.next_monday.as_str()),
        ("2026-12-04", "2026-12-07")
    );
    assert_eq!(
        (&after.cutoff, &after.since, &after.listed),
        (&before.cutoff, &before.since, &before.listed)
    );
    assert_eq!(
        (after.as_of.as_str(), after.revision),
        ("2026-12-08T15:05:00Z", 1)
    );
    assert_eq!(
        cover_stem(after.friday(), after.revision),
        "earnings-week-2026-12-04-r1"
    );
    assert_eq!(after.counts.confirmed, before.counts.confirmed + 1);
    let en = w.page("2026-12-04", "en").unwrap().body_md;
    assert!(en.contains("as of Tue Dec 8, 10:05 ET"), "{en}");
    // The page says the list was read again, and when the post first went out.
    assert!(
        en.contains("## Expected next week, as of Tue Dec 8, 10:05 ET\n\nThis list was read again on Tue Dec 8, 10:05 ET, after the post was first published on Fri Dec 4, 18:05 ET. The checks above are the ones listed then."),
        "{en}"
    );
    let zh = w.page("2026-12-04", "zh").unwrap().body_md;
    assert!(
        zh.contains("## 下周预计公布（截至美东时间12月8日周二10:05）\n\n本列表于美东时间12月8日周二10:05重新读取；本文首发于美东时间12月4日周五18:05。上方的速评仍为首发时所列。"),
        "{zh}"
    );
    assert!(w
        .page("2026-12-04", "en")
        .unwrap()
        .summary
        .contains("as of Tue Dec 8, 10:05 ET"));
    // The first build's page has no such note.
    assert!(!rendered(&w, "2026-12-04").is_empty());
    assert!(!std::fs::read_to_string(format!(
        "{}/tests/fixtures/week/2026-12-04.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .contains("read again"));
    assert!(!en.contains("adbe-q4"));
    assert!(en.contains("Revenue $10.5B"), "{en}");
    assert_eq!(w.run("2026-12-04").unwrap().status, "published");
    assert!(!w.queued("2026-12-08T15:10:00Z").await);
    // The corrected post shows the revised figure, so the next Friday raises
    // no week-stale for it (the check changed before the rebuild read it).
    w.cal("ORCL", "2026-12-15", "amc", "confirmed");
    w.cal("MU", "2026-12-17", "amc", "unconfirmed");
    w.refreshed("2026-12-11");
    w.tick("2026-12-11T23:05:00Z").await;
    assert_eq!(w.run("2026-12-11").unwrap().status, "published");
    assert!(
        !w.alerts().iter().any(|a| a.0.starts_with("week-stale")),
        "{:?}",
        w.alerts()
    );
}

/// A store failure at the batch leaves nothing visible: the post row
/// without text. The message fails, the run waits out its 15-minute lease
/// and is built again into the same post; the fifth failure fails the run
/// with one error alert.
#[tokio::test]
async fn a_store_failure_at_the_batch_leaves_nothing_visible_and_retries_on_the_lease() {
    let w = dec4();
    w.base.fail_revise_post.set(true);
    let mut now = utc(DEC4_BUILD);
    let mut id = None;
    for i in 0..MAX_BUILDS {
        assert!(w.try_tick(&iso(now)).await.is_err(), "build {i}");
        let post = w.post("2026-12-04").unwrap();
        assert!(w.page("2026-12-04", "en").is_none() && w.page("2026-12-04", "zh").is_none());
        assert!(id.is_none() || id == post.id, "one post");
        id = post.id;
        let run = w.run("2026-12-04").unwrap();
        assert_eq!((run.status.as_str(), run.attempts), ("building", i + 1));
        assert!(
            !w.queued(&iso(now + Duration::minutes(10))).await,
            "in its lease"
        );
        now += Duration::minutes(16);
        assert!(w.queued(&iso(now)).await, "after its lease");
    }
    let line = w.tick(&iso(now)).await;
    assert!(line.contains("failed: "), "{line}");
    assert_eq!(w.run("2026-12-04").unwrap().status, "failed");
    let errors: Vec<_> = w.alerts().into_iter().filter(|a| a.1 == "error").collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(
        w.base
            .posts
            .borrow()
            .iter()
            .filter(|p| p.content_type == "week")
            .count(),
        1
    );
}

/// A correction that keeps hitting a store error is bounded the same way,
/// and the post stays as it was published.
#[tokio::test]
async fn a_correction_that_keeps_failing_gives_up_and_leaves_the_post() {
    let w = dec4();
    w.tick(DEC4_BUILD).await;
    let before = (
        w.post("2026-12-04").unwrap(),
        w.page("2026-12-04", "en").unwrap(),
    );
    w.set_run("2026-12-04", |r| {
        r.status = "pending".into();
        r.attempts = 0;
    });
    w.base.fail_revise_post.set(true);
    let mut now = utc("2026-12-08T15:05:00Z");
    for i in 0..MAX_BUILDS {
        assert!(w.queued(&iso(now)).await, "build {i}");
        assert!(w.try_tick(&iso(now)).await.is_err(), "build {i}");
        let run = w.run("2026-12-04").unwrap();
        assert_eq!((run.status.as_str(), run.attempts), ("building", i + 1));
        assert!(
            !w.queued(&iso(now + Duration::minutes(10))).await,
            "in its lease"
        );
        now += Duration::minutes(16);
    }
    assert!(w.queued(&iso(now)).await);
    let line = w.tick(&iso(now)).await;
    assert!(
        line.contains("failed: the correction did not finish in 5 builds"),
        "{line}"
    );
    assert_eq!(w.run("2026-12-04").unwrap().status, "failed");
    assert!(!w.queued(&iso(now + Duration::minutes(5))).await);
    assert_eq!(
        (
            w.post("2026-12-04").unwrap(),
            w.page("2026-12-04", "en").unwrap()
        ),
        before
    );
    assert_eq!(w.alerts().iter().filter(|a| a.1 == "error").count(), 1);
}

/// A first build that died after its batch is closed, not rebuilt.
#[tokio::test]
async fn a_crash_after_the_batch_closes_the_run() {
    let w = dec4();
    w.tick(DEC4_BUILD).await;
    let published = w.post("2026-12-04").unwrap().published_at;
    w.set_run("2026-12-04", |r| {
        r.status = "building".into();
        r.updated_at = DEC4_BUILD.into();
    });
    // In flight for its 15-minute lease, then picked up.
    assert!(!w.queued("2026-12-04T23:10:00Z").await);
    assert!(w.queued("2026-12-04T23:25:00Z").await);
    let line = w.tick("2026-12-04T23:25:00Z").await;
    assert!(line.contains("published (run closed)"), "{line}");
    let post = w.post("2026-12-04").unwrap();
    assert_eq!((post.published_at, post.revised_at), (published, None));
}

// ---------------------------------------------------------------------------
// The switch: off is nothing, without a read.

/// `WeekStore` that counts its calls.
struct Counted<'a>(&'a MemStore, Cell<usize>);

#[async_trait(?Send)]
impl WeekStore for Counted<'_> {
    async fn week_runs(&self, this: &str) -> Result<Vec<WeekRun>, String> {
        self.1.set(self.1.get() + 1);
        self.0.week_runs(this).await
    }
    async fn week_posts(&self, f: &str, l: usize) -> Result<Vec<PostRow>, String> {
        self.1.set(self.1.get() + 1);
        self.0.week_posts(f, l).await
    }
    async fn week_checks(&self, s: &str, c: &str) -> Result<Vec<CheckRow>, String> {
        self.1.set(self.1.get() + 1);
        self.0.week_checks(s, c).await
    }
    async fn checks_by_slug(&self, s: &[String]) -> Result<Vec<CheckRow>, String> {
        self.1.set(self.1.get() + 1);
        self.0.checks_by_slug(s).await
    }
    async fn written(&self, s: &[String]) -> Result<Vec<Written>, String> {
        self.1.set(self.1.get() + 1);
        self.0.written(s).await
    }
}

#[tokio::test]
async fn off_reads_nothing_and_on_reads_one_row_range_per_poll() {
    let w = dec4();
    let ws = Counted(&w.base, Cell::new(0));
    let before = w.es.calls.get();
    for raw in [None, Some("false"), Some("TRUE"), Some("true ")] {
        assert_eq!(
            follow_up(raw, utc(DEC4_BUILD), &w.es, &ws).await.unwrap(),
            None
        );
    }
    assert_eq!((ws.1.get(), w.es.calls.get() - before), (0, 0));
    assert!(w.base.runs.borrow().is_empty() && w.base.facts.borrow().is_empty());
    // On, on a Tuesday: the one read of the week runs.
    follow_up(Some("true"), utc("2026-12-08T15:00:00Z"), &w.es, &ws)
        .await
        .unwrap();
    assert_eq!((ws.1.get(), w.es.calls.get() - before), (1, 0));
    // On, on Friday evening: that read and the refresh marker.
    follow_up(Some("true"), utc(DEC4_BUILD), &w.es, &ws)
        .await
        .unwrap();
    assert_eq!((ws.1.get(), w.es.calls.get() - before), (2, 1));
}

/// A first build's reads: the runs, the earlier posts and the checks (no
/// stale read without an earlier post), the refresh marker, and next week
/// through `week_days` (the events and the calendar; no picks query).
#[tokio::test]
async fn a_friday_build_reads_what_the_spec_counts() {
    let w = dec4();
    let ws = Counted(&w.base, Cell::new(0));
    let (ev, es) = (w.ev.calls.get(), w.es.calls.get());
    week_tick(utc(DEC4_BUILD), &w.es, &w.ev, w.base.as_ref(), &ws)
        .await
        .unwrap();
    assert_eq!(ws.1.get(), 3);
    assert_eq!((w.ev.calls.get() - ev, w.es.calls.get() - es), (1, 2));
}

// ---------------------------------------------------------------------------
// A peak week: 60 checks and 200 companies.

pub(crate) fn peak() -> World {
    let w = world();
    let tickers: Vec<&'static str> = crate::earnings::watchlist()
        .iter()
        .map(|e| e.ticker)
        .collect();
    for (i, t) in tickers.iter().take(60).enumerate() {
        let slug = format!(
            "{}-q4-fy2026-earnings",
            t.to_ascii_lowercase().replace('.', "-")
        );
        let day = format!("2027-01-{:02}", 4 + (i % 5));
        w.check(
            &slug,
            t,
            &day,
            &format!("{day}T21:{:02}:00Z", i % 60),
            [
                &format!("Revenue ${}.{}B", 10 + i, i % 10),
                &format!("GAAP EPS ${}.{:02}", i % 7, i),
            ],
            [
                &format!("营收 {}.{}亿美元", 100 + i, i % 10),
                &format!("GAAP每股收益 {}.{:02}美元", i % 7, i),
            ],
            "published",
        );
    }
    let slots = ["bmo", "amc", "unknown"];
    let statuses = ["unconfirmed", "confirmed", "unconfirmed", "reported"];
    for (i, t) in tickers.iter().skip(60).take(200).enumerate() {
        w.cal(
            t,
            &format!("2027-01-{:02}", 11 + (i % 5)),
            slots[i % 3],
            statuses[i % 4],
        );
    }
    w.event(
        "bls:cpi:2026-12",
        "cpi",
        "2027-01-13",
        Some("2027-01-13T13:30:00Z"),
        "https://www.bls.gov/schedule/news_release/cpi.htm",
    );
    w.refreshed("2027-01-08");
    w
}

#[tokio::test]
async fn a_peak_week_of_60_checks_and_200_companies_renders_whole() {
    let w = peak();
    let line = w.tick("2027-01-08T23:05:00Z").await;
    assert!(
        line.contains("published earnings-week-2027-01-08 (60 checks, 200 expected)"),
        "{line}"
    );
    let en = w.page("2027-01-08", "en").unwrap();
    assert_eq!(en.body_md.matches("](/en/p/").count(), 60);
    assert_eq!(en.body_md.matches("](/en/t/").count(), 200);
    assert!(en.body_md.len() < 40_000, "{}", en.body_md.len());
    fixture("2027-01-08.md", &rendered(&w, "2027-01-08"));
    // The bundle too, for the Wasm CPU probe (examples/week-cpu.rs).
    fixture(
        "2027-01-08.bundle.json",
        &serde_json::to_string(&w.bundle("2027-01-08")).unwrap(),
    );
    // The site renders this page in its own tests too.
    site_fixture("week-2027-01-08-en.md", &en.body_md);
}

/// The peak week's render and guard, timed natively to catch regressions
/// (the Wasm measurement is in the PR).
#[tokio::test]
async fn the_peak_week_renders_and_checks_in_milliseconds() {
    let w = peak();
    w.tick("2027-01-08T23:05:00Z").await;
    let b = w.bundle("2027-01-08");
    render_both(&b).unwrap();
    let t = std::time::Instant::now();
    for _ in 0..10 {
        render_both(&b).unwrap();
    }
    let per = t.elapsed() / 10;
    assert!(per < std::time::Duration::from_millis(200), "{per:?}");
}

// ---------------------------------------------------------------------------
// The dry run: the posts for Fri 2026-11-20 and Fri 2026-11-27 from a
// read-only snapshot of live D1 rows taken 2026-11-24 (SELECT only; public
// rows: the checks with their cover lines, calendar rows, events). The
// statuses are those of 2026-11-24, so the Nov 20 page shows what a build
// from today's rows writes, not byte for byte what Nov 20 would have.

fn snapshot(name: &str) -> Vec<Value> {
    let path = format!(
        "{}/tests/fixtures/week/d1-2026-11-24-{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn s(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().map(String::from)
}

pub(crate) fn live_world() -> World {
    let w = world();
    for (i, r) in snapshot("checks").iter().enumerate() {
        let id = i as i64 + 1;
        w.base.posts.borrow_mut().push(PostRow {
            id: Some(id),
            run_key: s(r, "run_key").unwrap(),
            content_type: "earnings".into(),
            template_id: "earnings:v1".into(),
            trading_date: s(r, "trading_date").unwrap(),
            slug: s(r, "slug").unwrap(),
            facts_r2_key: String::new(),
            published_at: s(r, "published_at"),
            cover_key: None,
            revised_at: s(r, "revised_at"),
        });
        for (lang, status, l1, l2) in [
            ("en", "en_status", "en1", "en2"),
            ("zh", "zh_status", "zh1", "zh2"),
        ] {
            let Some(status) = s(r, status) else { continue };
            w.base.translations.borrow_mut().insert(
                (id, lang.into()),
                TranslationRow {
                    post_id: id,
                    lang: lang.into(),
                    status,
                    title: String::new(),
                    summary: String::new(),
                    body_md: String::new(),
                    updated_at: String::new(),
                    cover_line1: s(r, l1),
                    cover_line2: s(r, l2),
                },
            );
        }
    }
    for r in snapshot("calendar") {
        w.es.calendar.borrow_mut().insert(
            (s(&r, "ticker").unwrap(), s(&r, "report_date").unwrap()),
            CalendarRow {
                ticker: s(&r, "ticker").unwrap(),
                report_date: s(&r, "report_date").unwrap(),
                time_slot: s(&r, "time_slot").unwrap(),
                status: s(&r, "status").unwrap(),
                source: s(&r, "source").unwrap(),
                proof_url: s(&r, "proof_url"),
                proof_quote: None,
                conflict_reason: None,
                first_seen_at: String::new(),
                checked_at: s(&r, "checked_at").unwrap(),
                next_check_at: None,
                updated_at: String::new(),
            },
        );
    }
    for r in snapshot("events") {
        let id = s(&r, "id").unwrap();
        w.ev.rows.borrow_mut().insert(
            id.clone(),
            EventRow {
                id,
                series: s(&r, "series").unwrap(),
                ref_period: s(&r, "ref_period"),
                variant: s(&r, "variant"),
                starts_at: s(&r, "starts_at"),
                et_date: s(&r, "et_date").unwrap(),
                tier: r["tier"].as_i64().unwrap(),
                status: s(&r, "status").unwrap(),
                proof_url: s(&r, "proof_url").unwrap(),
                release_url: s(&r, "release_url"),
                moved_from: s(&r, "moved_from"),
                missing_count: 0,
                first_seen_at: String::new(),
                checked_at: String::new(),
                updated_at: String::new(),
            },
        );
    }
    w
}

#[tokio::test]
async fn the_dry_run_on_live_rows_for_nov_20_and_nov_27() {
    let w = live_world();
    w.refreshed("2026-11-20");
    let line = w.tick("2026-11-20T23:05:00Z").await;
    assert!(
        line.contains("published earnings-week-2026-11-20"),
        "{line}"
    );
    let b = w.bundle("2026-11-20");
    assert_eq!(b.since, "2026-11-16");
    assert_eq!(b.counts.checks, 9, "{:?}", b.listed);
    fixture("dry-run-2026-11-20.md", &rendered(&w, "2026-11-20"));
    w.refreshed("2026-11-27");
    let line = w.tick("2026-11-27T23:05:00Z").await;
    assert!(line.contains("week:2026-11-27"), "{line}");
    let out = match w.post("2026-11-27") {
        Some(_) => {
            let b27 = w.bundle("2026-11-27");
            assert!(b27.listed.iter().all(|s| !b.listed.contains(s)));
            rendered(&w, "2026-11-27")
        }
        None => format!("{line}\n"),
    };
    fixture("dry-run-2026-11-27.md", &out);
}

/// The `week` message rides the serial build queue, as `ipo:article` does.
#[test]
fn the_week_job_goes_to_the_build_queue() {
    use crate::schedule::{queue_for, JobKind, EARNINGS_BUILD_QUEUE};
    assert_eq!(EarningsStep::Week.job(), "week");
    assert_eq!(
        JobKind::parse("week"),
        Some(JobKind::Earnings(EarningsStep::Week))
    );
    assert_eq!(queue_for("week"), EARNINGS_BUILD_QUEUE);
    assert_eq!(queue_for("week"), queue_for("ipo:article"));
}
