use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::{json, Value};

use super::buttondown::Buttondown;
use super::*;
use crate::earnings::store::{CalendarRow, MemEarningsStore};
use crate::events::store::MemEventStore;
use crate::providers::{Http, HttpResp, ProviderError};
use crate::store::MemStore;

const KEY: &str = "bd-SECRETKEY";

fn utc(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn day(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn cal(ticker: &str, date: &str, slot: &str, status: &str) -> CalendarRow {
    CalendarRow {
        ticker: ticker.into(),
        report_date: date.into(),
        time_slot: slot.into(),
        status: status.into(),
        source: "yahoo".into(),
        proof_url: None,
        proof_quote: None,
        conflict_reason: None,
        first_seen_at: "2026-11-01T00:00:00Z".into(),
        checked_at: "2026-11-06T11:00:00Z".into(),
        next_check_at: None,
        updated_at: "2026-11-06T11:00:00Z".into(),
    }
}

fn check(slug: &str, date: &str, ticker: &str, en: Option<&str>, zh: Option<&str>) -> PostLink {
    PostLink {
        slug: slug.into(),
        trading_date: date.into(),
        ticker: Some(ticker.into()),
        title_en: en.map(String::from),
        title_zh: zh.map(String::from),
    }
}

/// Earnings posts by trading date; counts its calls.
#[derive(Default)]
struct Posts {
    rows: Vec<PostLink>,
    calls: Cell<usize>,
}

#[async_trait(?Send)]
impl DigestStore for Posts {
    async fn published_posts(
        &self,
        content_type: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<PostLink>, String> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(content_type, "earnings");
        Ok(self
            .rows
            .iter()
            .filter(|p| p.trading_date.as_str() >= from && p.trading_date.as_str() <= to)
            .cloned()
            .collect())
    }
}

/// `(method, url, headers, body)`.
type Req = (String, String, Vec<(String, String)>, Option<Value>);

/// Buttondown: the tag list, then one answer per email POST (201 with the
/// request's filters echoed, unless `drop_filters`) and per PATCH.
struct Fake {
    seen: RefCell<Vec<Req>>,
    tags_status: u16,
    tags: Vec<(&'static str, &'static str)>,
    /// Statuses for the next POSTs; 201 once empty.
    post_status: RefCell<VecDeque<u16>>,
    patch_status: u16,
    drop_filters: bool,
    network: bool,
}

fn fake() -> Fake {
    Fake {
        seen: RefCell::new(vec![]),
        tags_status: 200,
        tags: vec![
            ("prebell", "tag_0prebell"),
            ("lang:en", "tag_0en"),
            ("lang:zh", "tag_0zh"),
            ("source:home", "tag_0home"),
        ],
        post_status: RefCell::new(VecDeque::new()),
        patch_status: 200,
        drop_filters: false,
        network: false,
    }
}

impl Fake {
    fn record(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&Value>) {
        self.seen.borrow_mut().push((
            method.into(),
            url.into(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body.cloned(),
        ));
    }
    fn posts(&self) -> Vec<Value> {
        self.seen
            .borrow()
            .iter()
            .filter(|r| r.0 == "POST")
            .map(|r| r.3.clone().unwrap())
            .collect()
    }
    fn methods(&self) -> Vec<String> {
        self.seen.borrow().iter().map(|r| r.0.clone()).collect()
    }
}

fn resp(status: u16, body: String) -> Result<HttpResp, ProviderError> {
    Ok(HttpResp {
        status,
        body,
        set_cookie: None,
    })
}

#[async_trait(?Send)]
impl Http for Fake {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
        self.record("GET", url, headers, None);
        if self.network {
            return Err(ProviderError::Network("down".into()));
        }
        let results: Vec<Value> = self
            .tags
            .iter()
            .map(|(name, id)| json!({"id": id, "name": name, "color": "#000000"}))
            .collect();
        resp(
            self.tags_status,
            json!({"results": results, "next": null, "previous": null, "count": results.len()})
                .to_string(),
        )
    }
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResp, ProviderError> {
        self.record("POST", url, headers, Some(body));
        let n = self.posts().len();
        let status = self.post_status.borrow_mut().pop_front().unwrap_or(201);
        if status != 201 {
            return resp(status, r#"{"detail":"nope","code":"body_invalid"}"#.into());
        }
        let filters = if self.drop_filters {
            json!({"filters": [], "groups": [], "predicate": "and"})
        } else {
            body["filters"].clone()
        };
        resp(
            201,
            json!({"id": format!("em_0{n}"), "status": body["status"], "filters": filters})
                .to_string(),
        )
    }
    async fn patch_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResp, ProviderError> {
        self.record("PATCH", url, headers, Some(body));
        resp(self.patch_status, "{}".into())
    }
}

/// The week ahead of Fri 2026-11-06: Monday 2026-11-09.
const FRI_EVENING: &str = "2026-11-06T23:30:00Z"; // 18:30 EST

struct World {
    base: Rc<MemStore>,
    es: MemEarningsStore,
    ev: MemEventStore,
    posts: Posts,
}

/// Next week's calendar from D1's rows on 2026-11-06 (plus SNPS, so a slot
/// has write-ups and others), the events table not yet loaded (so the
/// snapshot: CPI on Tue Nov 10), and last week's checks.
fn world() -> World {
    let base = Rc::new(MemStore::default());
    let es = MemEarningsStore::new(base.clone());
    for (t, d, slot, status) in [
        ("CEG", "2026-11-09", "bmo", "unconfirmed"),
        ("KKR", "2026-11-09", "bmo", "unconfirmed"),
        ("NRG", "2026-11-09", "bmo", "unconfirmed"),
        ("TTWO", "2026-11-09", "amc", "unconfirmed"),
        ("TECH", "2026-11-09", "unknown", "unconfirmed"),
        ("TDG", "2026-11-10", "bmo", "unconfirmed"),
        ("CSCO", "2026-11-12", "amc", "confirmed"),
        ("AMAT", "2026-11-12", "amc", "unconfirmed"),
        ("SNPS", "2026-11-12", "amc", "unconfirmed"),
        ("NBIS", "2026-11-12", "bmo", "unconfirmed"),
        // Not shown: quarantined, and outside the week.
        ("DIS", "2026-11-12", "bmo", "quarantined"),
        ("WMT", "2026-11-19", "bmo", "confirmed"),
    ] {
        es.calendar
            .borrow_mut()
            .insert((t.into(), d.into()), cal(t, d, slot, status));
    }
    let posts = Posts {
        rows: vec![
            check(
                "aig-q3-fy2026-earnings",
                "2026-11-04",
                "AIG",
                Some("AIG's third quarter 2026: net income of $844 million"),
                Some("AIG 2026财年第三季度业绩速览"),
            ),
            check(
                "amd-q3-fy2026-earnings",
                "2026-11-03",
                "AMD",
                Some("AMD's third quarter of fiscal 2026"),
                Some("AMD 2026财年第三季度业绩速览"),
            ),
            // Chinese still pending: the ZH email links the English page.
            check(
                "cop-q3-fy2026-earnings",
                "2026-11-05",
                "COP",
                Some("ConocoPhillips third-quarter 2026 results"),
                None,
            ),
            // The week before: not shown.
            check(
                "aapl-q4-fy2026-earnings",
                "2026-10-29",
                "AAPL",
                Some("Apple Q4"),
                Some("苹果"),
            ),
        ],
        calls: Cell::new(0),
    };
    World {
        base,
        es,
        ev: MemEventStore::default(),
        posts,
    }
}

/// `digest_tick` with the Buttondown sender over `http`.
async fn tick(w: &World, http: &Fake, now: &str, mode: Mode, key: &str) -> String {
    let logs = RefCell::new(Vec::<String>::new());
    let log = |m: &str| logs.borrow_mut().push(m.to_string());
    let bd = Buttondown::new(http, key, &log);
    let line = digest_tick(
        utc(now),
        &w.es,
        &w.ev,
        &w.posts,
        w.base.as_ref(),
        Some(&bd),
        mode,
    )
    .await
    .unwrap();
    // Status codes only: never a key or a body.
    for l in logs.borrow().iter() {
        assert!(
            !l.contains(KEY) && !l.contains("detail") && !l.contains("Week"),
            "{l}"
        );
        assert!(l.starts_with("digest: buttondown "), "{l}");
    }
    line
}

async fn week(w: &World, now: &str) -> Week {
    gather(utc(now), week_ahead(utc(now)), &w.es, &w.ev, &w.posts)
        .await
        .unwrap()
}

#[test]
fn mode_parses_and_rejects_typos() {
    assert_eq!(parse_mode(None), Ok(Mode::Draft));
    assert_eq!(parse_mode(Some("")), Ok(Mode::Draft));
    assert_eq!(parse_mode(Some("draft")), Ok(Mode::Draft));
    assert_eq!(parse_mode(Some("scheduled")), Ok(Mode::Scheduled));
    let e = parse_mode(Some("schedule")).unwrap_err();
    assert!(e.contains("DIGEST_MODE") && e.contains("schedule"), "{e}");
}

#[test]
fn due_on_friday_evenings_et_for_the_next_monday() {
    // 18:00 EST Friday is 23:00Z; 18:00 EDT Friday is 22:00Z.
    assert!(due(utc("2026-11-06T23:00:00Z")));
    assert!(!due(utc("2026-11-06T22:55:00Z")));
    assert!(due(utc("2026-10-30T22:00:00Z")));
    assert!(!due(utc("2026-10-30T21:55:00Z")));
    // 21:55 ET Friday is Saturday in UTC: still due.
    assert!(due(utc("2026-11-07T02:55:00Z")));
    assert!(!due(utc("2026-11-05T23:00:00Z"))); // Thursday
    assert_eq!(week_ahead(utc("2026-11-07T02:55:00Z")), day("2026-11-09"));
    assert_eq!(week_ahead(utc("2026-10-30T22:00:00Z")), day("2026-11-02"));
    // Year end: the Friday before New Year's.
    assert_eq!(week_ahead(utc("2026-12-25T23:00:00Z")), day("2026-12-28"));
}

#[test]
fn the_scheduled_time_is_sunday_6pm_new_york_across_dst() {
    // A regular EST Sunday: 18:00 EST = 23:00Z.
    assert_eq!(
        send_at(day("2026-11-09")),
        Some(utc("2026-11-08T23:00:00Z"))
    );
    // DST ends Sun 2026-11-01 at 02:00: the digest built Fri Oct 30 (EDT,
    // UTC-4) sends at 18:00 EST, 23:00Z, not 22:00Z.
    assert_eq!(
        send_at(day("2026-11-02")),
        Some(utc("2026-11-01T23:00:00Z"))
    );
    // DST starts Sun 2027-03-14: built Fri Mar 12 (EST), sends 18:00 EDT = 22:00Z.
    assert_eq!(
        send_at(day("2027-03-15")),
        Some(utc("2027-03-14T22:00:00Z"))
    );
    // A regular EDT Sunday.
    assert_eq!(
        send_at(day("2026-10-26")),
        Some(utc("2026-10-25T22:00:00Z"))
    );
}

#[tokio::test]
async fn gather_orders_writeups_counts_the_rest_and_keeps_last_weeks_checks() {
    let w = world();
    let wk = week(&w, FRI_EVENING).await;
    assert_eq!(wk.monday, day("2026-11-09"));
    assert_eq!(wk.days.len(), 5);
    let mon = &wk.days[0];
    // Calendar-only companies are counted, not named.
    assert!(
        mon.bmo.writeups.is_empty() && mon.bmo.others == 3,
        "{mon:?}"
    );
    assert_eq!((mon.amc.writeups.len(), mon.amc.others), (0, 1));
    assert_eq!(mon.unknown.others, 1);
    let thu = &wk.days[3];
    // Write-up companies largest first (Cisco is the larger); SNPS is
    // calendar only.
    let names: Vec<&str> = thu.amc.writeups.iter().map(|c| c.ticker.as_str()).collect();
    assert_eq!(names, ["CSCO", "AMAT"]);
    assert_eq!(thu.amc.others, 1);
    assert!(thu.amc.writeups[0].confirmed && !thu.amc.writeups[1].confirmed);
    // The quarantined DIS row is not shown.
    assert!(
        thu.bmo.writeups.is_empty() && thu.bmo.others == 1,
        "{thu:?}"
    );
    // The events table is not loaded yet: the snapshot's CPI on Tuesday.
    let tue = &wk.days[1];
    assert_eq!(tue.events.len(), 1);
    assert_eq!(tue.events[0].series, "cpi");
    assert_eq!(tue.events[0].starts_at, Some(utc("2026-11-10T13:30:00Z")));
    // Last week (Nov 2-8), largest companies first: AMD, COP, AIG.
    let slugs: Vec<&str> = wk.checks.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(
        slugs,
        [
            "amd-q3-fy2026-earnings",
            "cop-q3-fy2026-earnings",
            "aig-q3-fy2026-earnings"
        ]
    );
    // One D1 read each: calendar, events (+ the loaded marker), posts.
    assert_eq!(w.posts.calls.get(), 1);
    assert_eq!(w.ev.calls.get(), 1);
}

#[tokio::test]
async fn renders_both_languages() {
    let w = world();
    let wk = week(&w, FRI_EVENING).await;
    assert_eq!(render::subject(wk.monday, Lang::En), "Week ahead: Nov 9–13");
    assert_eq!(
        render::subject(wk.monday, Lang::Zh),
        "一周前瞻：11月9日–13日"
    );

    let en = render::body(&wk, Lang::En);
    for want in [
        "Earnings and economic dates for the week of Nov 9–13. Times are US Eastern.",
        "## Market calendar",
        "### Mon, Nov 9\n\n- Before open: 3 companies\n- After close: 1 company\n- Time n/a: 1 company",
        "### Tue, Nov 10\n\n- 8:30 AM ET: [October CPI](https://www.bls.gov/schedule/news_release/cpi.htm)\n- Before open: 1 company\n\n",
        "### Wed, Nov 11\n\n- No earnings reports.",
        "- After close: Cisco (CSCO), Applied Materials (AMAT) (unconfirmed), +1 more",
        "[Full calendar →](https://prebell.site/en/earnings?week=2026-11-09)",
        "## Last week's earnings checks\n\n- [AMD's third quarter of fiscal 2026](https://prebell.site/en/p/amd-q3-fy2026-earnings)",
        "- [ConocoPhillips third-quarter 2026 results](https://prebell.site/en/p/cop-q3-fy2026-earnings)",
        "[AIG's third quarter 2026: net income of $844 million](https://prebell.site/en/p/aig-q3-fy2026-earnings)",
        "[All earnings checks →](https://prebell.site/en/archive?type=earnings)",
    ] {
        assert!(en.contains(want), "missing {want:?} in\n{en}");
    }
    assert!(!en.contains("aapl") && !en.contains("Walt Disney"), "{en}");
    assert!(!en.starts_with("---"));

    let zh = render::body(&wk, Lang::Zh);
    for want in [
        "11月9日–13日当周的财报与经济日程。时间均为美东时间。",
        "## 市场日历",
        "### 11月9日 周一\n\n- 盘前：3家公司\n- 盘后：1家公司\n- 时间暂缺：1家公司",
        "- 盘后：Cisco (CSCO)、Applied Materials (AMAT)（未确认）、另有1家",
        "- 上午8:30（美东）：[10月消费者物价指数（CPI）](https://www.bls.gov/schedule/news_release/cpi.htm)",
        "### 11月11日 周三\n\n- 无财报。",
        "[完整日历 →](https://prebell.site/zh/earnings?week=2026-11-09)",
        "## 上周财报速评",
        "- [AMD 2026财年第三季度业绩速览](https://prebell.site/zh/p/amd-q3-fy2026-earnings)",
        // ZH pending: the English page, with its English title.
        "- [ConocoPhillips third-quarter 2026 results](https://prebell.site/en/p/cop-q3-fy2026-earnings)",
        "[全部财报速评 →](https://prebell.site/zh/archive?type=earnings)",
    ] {
        assert!(zh.contains(want), "missing {want:?} in\n{zh}");
    }
}

#[tokio::test]
async fn a_quiet_week_lists_its_days_and_drops_the_empty_section() {
    // Week of Dec 14-18, 2026: the events table is loaded and empty for
    // it, one company reports and no check was published the week before.
    let base = Rc::new(MemStore::default());
    let es = MemEarningsStore::new(base.clone());
    es.kv_set("events_loaded_on", "2026-12-11", "x")
        .await
        .unwrap();
    es.calendar.borrow_mut().insert(
        ("ROST".into(), "2026-12-15".into()),
        cal("ROST", "2026-12-15", "amc", "unconfirmed"),
    );
    let w = World {
        base,
        es,
        ev: MemEventStore::default(),
        posts: Posts::default(),
    };
    let wk = week(&w, "2026-12-11T23:30:00Z").await;
    assert_eq!(wk.monday, day("2026-12-14"));
    assert!(wk.days.iter().all(|d| d.events.is_empty()), "{wk:?}");
    let en = render::body(&wk, Lang::En);
    assert_eq!(en.matches("- No earnings reports.").count(), 4, "{en}");
    assert!(
        en.contains("### Tue, Dec 15\n\n- After close: 1 company"),
        "{en}"
    );
    assert!(!en.contains("## Last week's earnings checks"), "{en}");
    assert!(!en.contains("All earnings checks"), "{en}");
    let zh = render::body(&wk, Lang::Zh);
    assert!(
        zh.contains("- 盘后：1家公司") && !zh.contains("上周财报速评"),
        "{zh}"
    );
}

#[tokio::test]
async fn a_holiday_week_shows_the_closure_and_the_early_close() {
    // Thanksgiving 2026: closed Thu Nov 26, early close Fri Nov 27, even
    // with the events table loaded and holding no closure rows.
    let base = Rc::new(MemStore::default());
    let es = MemEarningsStore::new(base.clone());
    es.kv_set("events_loaded_on", "2026-11-20", "x")
        .await
        .unwrap();
    es.calendar.borrow_mut().insert(
        ("DE".into(), "2026-11-25".into()),
        cal("DE", "2026-11-25", "bmo", "confirmed"),
    );
    let w = World {
        base,
        es,
        ev: MemEventStore::default(),
        posts: Posts::default(),
    };
    let wk = week(&w, "2026-11-20T23:30:00Z").await;
    assert_eq!(wk.monday, day("2026-11-23"));
    assert!(wk.days[3].closed() && !wk.days[4].closed());
    let en = render::body(&wk, Lang::En);
    assert!(
        en.contains("### Thu, Nov 26\n\n- [US stock market closed](https://www.nyse.com/markets/hours-calendars)\n\n"),
        "{en}"
    );
    assert!(
        en.contains("### Fri, Nov 27\n\n- [Early close, 1:00 PM ET](https://www.nyse.com/markets/hours-calendars)\n- No earnings reports."),
        "{en}"
    );
    assert!(en.contains("- Before open: Deere (DE)\n"), "{en}");
    assert_eq!(
        render::subject(wk.monday, Lang::En),
        "Week ahead: Nov 23–27"
    );
    let zh = render::body(&wk, Lang::Zh);
    assert!(zh.contains("### 11月26日 周四\n\n- [美股休市]("), "{zh}");
    assert!(zh.contains("- [美股提前收盘，下午1:00（美东）]("), "{zh}");
    // A week across two months.
    assert_eq!(
        render::subject(day("2026-11-30"), Lang::En),
        "Week ahead: Nov 30–Dec 4"
    );
    assert_eq!(
        render::subject(day("2026-11-30"), Lang::Zh),
        "一周前瞻：11月30日–12月4日"
    );
}

#[tokio::test]
async fn draft_mode_saves_two_drafts_with_their_language_tag_and_no_live_header() {
    let w = world();
    let http = fake();
    let line = tick(&w, &http, FRI_EVENING, Mode::Draft, KEY).await;
    assert_eq!(line, "2026-11-09 (draft): en draft em_01, zh draft em_02");
    assert_eq!(http.methods(), ["GET", "POST", "POST"]);
    let seen = http.seen.borrow().clone();
    assert_eq!(
        seen[0].1,
        "https://api.buttondown.com/v1/tags?page_size=1000"
    );
    for (i, (lang, tag, subject)) in [
        ("en", "tag_0en", "Week ahead: Nov 9–13"),
        ("zh", "tag_0zh", "一周前瞻：11月9日–13日"),
    ]
    .iter()
    .enumerate()
    {
        let (method, url, headers, body) = &seen[i + 1];
        assert_eq!(
            (method.as_str(), url.as_str()),
            ("POST", "https://api.buttondown.com/v1/emails")
        );
        let h = |k: &str| {
            headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(k))
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(h("Authorization"), Some("Token bd-SECRETKEY"));
        assert_eq!(h("X-API-Version"), Some("2026-04-01"));
        assert_eq!(
            h("X-Idempotency-Key"),
            Some(format!("prebell-digest-2026-11-09-{lang}").as_str())
        );
        assert!(
            headers
                .iter()
                .all(|(n, _)| !n.to_ascii_lowercase().contains("dangerously")),
            "{headers:?}"
        );
        let body = body.as_ref().unwrap();
        assert_eq!(body["status"], "draft");
        assert_eq!(body["subject"], *subject);
        assert!(body.get("publish_date").is_none(), "{body}");
        assert_eq!(
            body["filters"],
            json!({
                "filters": [{"field": "subscriber.tags", "operator": "contains", "value": tag}],
                "groups": [],
                "predicate": "and"
            })
        );
        let text = body["body"].as_str().unwrap();
        assert!(
            text.starts_with("<!-- buttondown-editor-mode: plaintext -->\n"),
            "{text}"
        );
    }
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
    // A second run in the same week sends nothing.
    let line = tick(&w, &http, "2026-11-07T00:00:00Z", Mode::Draft, KEY).await;
    assert_eq!(line, "2026-11-09: already built");
    assert_eq!(http.methods().len(), 3);
    assert!(w.base.alerts.borrow().is_empty());
}

#[tokio::test]
async fn scheduled_mode_schedules_each_draft_for_sunday_6pm_et() {
    let w = world();
    let http = fake();
    let line = tick(&w, &http, FRI_EVENING, Mode::Scheduled, KEY).await;
    assert_eq!(
        line,
        "2026-11-09 (scheduled 2026-11-08T23:00:00Z): en scheduled em_01, zh scheduled em_02"
    );
    assert_eq!(http.methods(), ["GET", "POST", "PATCH", "POST", "PATCH"]);
    let seen = http.seen.borrow();
    // Created as drafts first; scheduled only once the audience shows.
    assert_eq!(seen[1].3.as_ref().unwrap()["status"], "draft");
    assert_eq!(seen[2].1, "https://api.buttondown.com/v1/emails/em_01");
    assert_eq!(
        seen[2].3,
        Some(json!({"status": "scheduled", "publish_date": "2026-11-08T23:00:00Z"}))
    );
    assert!(seen[2]
        .2
        .iter()
        .all(|(n, _)| !n.to_ascii_lowercase().contains("dangerously")));
    assert_eq!(seen[4].1, "https://api.buttondown.com/v1/emails/em_02");
}

#[tokio::test]
async fn a_dropped_audience_is_never_scheduled_and_alerts() {
    let w = world();
    let mut http = fake();
    http.drop_filters = true;
    let line = tick(&w, &http, FRI_EVENING, Mode::Scheduled, KEY).await;
    assert!(
        line.contains("en draft em_01 without its audience"),
        "{line}"
    );
    assert_eq!(http.methods(), ["GET", "POST", "POST"]);
    let alerts = w.base.alerts.borrow();
    assert_eq!(alerts.len(), 2, "{alerts:?}");
    assert_eq!(alerts[0].0, "digest:en:2026-11-06");
    assert!(
        alerts[0].2.contains("lang:en audience filter"),
        "{}",
        alerts[0].2
    );
    // Done: a retry would get the same draft back.
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
}

#[tokio::test]
async fn without_a_sender_it_is_a_dry_run() {
    // The Worker today: it gathers and renders, sends and marks nothing.
    let w = world();
    let line = digest_tick(
        utc(FRI_EVENING),
        &w.es,
        &w.ev,
        &w.posts,
        w.base.as_ref(),
        None,
        Mode::Scheduled,
    )
    .await
    .unwrap();
    assert!(
        line.starts_with(
            "2026-11-09 (scheduled 2026-11-08T23:00:00Z): dry run, no sender: en \"Week ahead: Nov 9–13\" ("
        ),
        "{line}"
    );
    assert!(line.contains(", zh \"一周前瞻：11月9日–13日\" ("), "{line}");
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    assert!(w.base.alerts.borrow().is_empty());
}

#[test]
fn enabled_only_when_true() {
    assert_eq!(parse_enabled(None), Ok(false));
    assert_eq!(parse_enabled(Some("")), Ok(false));
    assert_eq!(parse_enabled(Some("false")), Ok(false));
    assert_eq!(parse_enabled(Some("true")), Ok(true));
    let e = parse_enabled(Some("yes")).unwrap_err();
    assert!(e.contains("DIGEST_ENABLED") && e.contains("yes"), "{e}");
}

/// A sender that answers from a script, for the core loop alone.
struct Scripted {
    answers: RefCell<VecDeque<Result<Sent, String>>>,
    emails: RefCell<Vec<Email>>,
}

#[async_trait(?Send)]
impl Sender for Scripted {
    async fn send(&self, email: &Email) -> Result<Sent, String> {
        self.emails.borrow_mut().push(email.clone());
        self.answers.borrow_mut().pop_front().unwrap()
    }
}

#[tokio::test]
async fn any_sender_gets_both_languages_with_stable_idempotency_keys() {
    let w = world();
    let s = Scripted {
        answers: RefCell::new(VecDeque::from([
            Err("http 503".into()),
            Ok(Sent::Done("saved".into())),
            Ok(Sent::Done("saved again".into())),
        ])),
        emails: RefCell::new(vec![]),
    };
    let run = |at: &'static str| {
        digest_tick(
            utc(at),
            &w.es,
            &w.ev,
            &w.posts,
            w.base.as_ref(),
            Some(&s),
            Mode::Draft,
        )
    };
    let line = run(FRI_EVENING).await.unwrap();
    assert_eq!(line, "2026-11-09 (draft): en failed: http 503, zh saved");
    let line = run("2026-11-06T23:35:00Z").await.unwrap();
    assert_eq!(line, "2026-11-09 (draft): en saved again");
    let emails = s.emails.borrow();
    let keys: Vec<&str> = emails.iter().map(|e| e.idempotency_key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "prebell-digest-2026-11-09-en",
            "prebell-digest-2026-11-09-zh",
            "prebell-digest-2026-11-09-en"
        ]
    );
    assert_eq!(emails[1].lang, Lang::Zh);
    assert_eq!(emails[1].subject, "一周前瞻：11月9日–13日");
    assert!(emails.iter().all(|e| e.send_at.is_none()));
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
}

#[tokio::test]
async fn a_missing_language_tag_skips_that_language_never_sends_unfiltered() {
    let w = world();
    let mut http = fake();
    http.tags.retain(|(n, _)| *n != "lang:zh");
    let line = tick(&w, &http, FRI_EVENING, Mode::Draft, KEY).await;
    assert_eq!(
        line,
        "2026-11-09 (draft): en draft em_01, zh skipped: no lang:zh tag"
    );
    assert_eq!(http.posts().len(), 1);
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
}

#[tokio::test]
async fn buttondown_errors_alert_and_leave_the_language_for_the_next_poll() {
    // 500 on English, 201 on Chinese.
    let w = world();
    let http = fake();
    http.post_status.borrow_mut().push_back(500);
    let line = tick(&w, &http, FRI_EVENING, Mode::Draft, KEY).await;
    assert_eq!(
        line,
        "2026-11-09 (draft): en failed: http 500, zh draft em_02"
    );
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09:zh");
    {
        let alerts = w.base.alerts.borrow();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].0, "digest:en:2026-11-06");
        assert!(alerts[0].2.contains("http 500") && !alerts[0].2.contains(KEY));
    }
    // The next poll sends only English, under the same idempotency key.
    let line = tick(&w, &http, "2026-11-06T23:35:00Z", Mode::Draft, KEY).await;
    assert_eq!(line, "2026-11-09 (draft): en draft em_03");
    let posts = http.posts();
    assert_eq!(posts.len(), 3);
    assert_eq!(posts[2]["subject"], "Week ahead: Nov 9–13");
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");

    // 401 (a wrong key) on both: nothing marked, one alert per language.
    let w = world();
    let http = fake();
    http.post_status.borrow_mut().extend([401, 401]);
    let line = tick(&w, &http, FRI_EVENING, Mode::Draft, KEY).await;
    assert_eq!(
        line,
        "2026-11-09 (draft): en failed: http 401, zh failed: http 401"
    );
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    assert_eq!(w.base.alerts.borrow().len(), 2);

    // The tag list fails (403, then the network): no email is created.
    for network in [false, true] {
        let w = world();
        let mut http = fake();
        http.tags_status = 403;
        http.network = network;
        let line = tick(&w, &http, FRI_EVENING, Mode::Draft, KEY).await;
        let want = if network { "network error" } else { "http 403" };
        assert_eq!(
            line,
            format!("2026-11-09 (draft): en failed: tags {want}, zh failed: tags {want}")
        );
        assert_eq!(http.methods(), ["GET", "GET"]);
        let keys: Vec<String> = w.base.alerts.borrow().iter().map(|a| a.0.clone()).collect();
        assert_eq!(keys, ["digest:en:2026-11-06", "digest:zh:2026-11-06"]);
        assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    }

    // Scheduling fails: the draft exists but stays pending, and the retry
    // (same idempotency key, so Buttondown answers with the same draft)
    // schedules it.
    let w = world();
    let mut http = fake();
    http.patch_status = 502;
    let line = tick(&w, &http, FRI_EVENING, Mode::Scheduled, KEY).await;
    assert!(line.contains("en failed: http 502"), "{line}");
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    http.patch_status = 200;
    let line = tick(&w, &http, "2026-11-06T23:35:00Z", Mode::Scheduled, KEY).await;
    assert!(
        line.contains("en scheduled") && line.contains("zh scheduled"),
        "{line}"
    );
    let keys: Vec<String> = http
        .seen
        .borrow()
        .iter()
        .filter(|r| r.0 == "POST")
        .filter_map(|r| {
            r.2.iter()
                .find(|(k, _)| k == "X-Idempotency-Key")
                .map(|(_, v)| v.clone())
        })
        .collect();
    assert_eq!(
        keys,
        [
            "prebell-digest-2026-11-09-en",
            "prebell-digest-2026-11-09-zh",
            "prebell-digest-2026-11-09-en",
            "prebell-digest-2026-11-09-zh"
        ]
    );
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
}

#[test]
fn markdown_in_a_title_is_escaped() {
    let mut wk = Week {
        monday: day("2026-11-09"),
        days: vec![],
        checks: vec![check(
            "x-q3",
            "2026-11-03",
            "X",
            Some("Q3 [beat] *big* _move_"),
            None,
        )],
    };
    let en = render::body(&wk, Lang::En);
    assert!(
        en.contains(r"- [Q3 \[beat\] \*big\* \_move\_](https://prebell.site/en/p/x-q3)"),
        "{en}"
    );
    // A check with no published title in either language is not listed.
    wk.checks[0].title_en = None;
    assert!(!render::body(&wk, Lang::Zh).contains("x-q3"));
}

#[tokio::test]
async fn a_late_scheduled_run_keeps_drafts() {
    // Past Sunday 18:00 ET (never in practice: polls stop Friday 22:00 ET),
    // a schedule would send at once; the drafts stay drafts.
    let w = world();
    let http = fake();
    w.es.kv_set(KV_BUILT, "2026-11-02", "x").await.unwrap();
    let line = tick(&w, &http, "2026-11-08T23:30:00Z", Mode::Scheduled, KEY).await;
    assert!(
        line.starts_with("2026-11-09 (scheduled, send time passed: draft)"),
        "{line}"
    );
    assert_eq!(http.methods(), ["GET", "POST", "POST"]);
}

#[tokio::test]
async fn the_job_stays_within_the_free_plan_budget() {
    let w = world();
    let http = crate::providers::CountingHttp::new(fake());
    let log = |_: &str| {};
    let bd = Buttondown::new(&http, KEY, &log);
    digest_tick(
        utc(FRI_EVENING),
        &w.es,
        &w.ev,
        &w.posts,
        w.base.as_ref(),
        Some(&bd),
        Mode::Scheduled,
    )
    .await
    .unwrap();
    // 1 tag list + 2 creates + 2 schedules.
    assert_eq!(http.count(), 5);
    // kv read, calendar, loaded marker, 2 kv writes (EarningsStore), one
    // events read, one posts read.
    assert_eq!(w.es.calls.get(), 5);
    assert_eq!(w.ev.calls.get() + w.posts.calls.get(), 2);
}
