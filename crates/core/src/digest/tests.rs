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

/// Not a key: a marker the logs and alerts must never contain.
const KEY: &str = "test-only-not-a-key";

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

fn check(slug: &str, date: &str, ticker: &str, title: &str, summary: &str) -> PostLink {
    PostLink {
        slug: slug.into(),
        trading_date: date.into(),
        ticker: Some(ticker.into()),
        title: title.into(),
        summary: summary.into(),
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

/// Buttondown: one answer per email POST (201 echoing the request's status,
/// or `answer_status` instead) and per PATCH. GETs are not expected.
struct Fake {
    seen: RefCell<Vec<Req>>,
    /// Statuses for the next POSTs; 201 once empty.
    post_status: RefCell<VecDeque<u16>>,
    /// The `status` a 201 answers with; `None` echoes the request's.
    answer_status: Option<Value>,
    patch_status: u16,
    network: bool,
}

fn fake() -> Fake {
    Fake {
        seen: RefCell::new(vec![]),
        post_status: RefCell::new(VecDeque::new()),
        answer_status: None,
        patch_status: 200,
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
        resp(404, "{}".into())
    }
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResp, ProviderError> {
        self.record("POST", url, headers, Some(body));
        if self.network {
            return Err(ProviderError::Network("down".into()));
        }
        let n = self.posts().len();
        let status = self.post_status.borrow_mut().pop_front().unwrap_or(201);
        if status != 201 {
            return resp(status, r#"{"detail":"nope","code":"body_invalid"}"#.into());
        }
        let answered = self
            .answer_status
            .clone()
            .unwrap_or_else(|| body["status"].clone());
        resp(
            201,
            json!({"id": format!("em_0{n}"), "status": answered, "subject": body["subject"]})
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

/// Last week's (Nov 2-8) earnings checks: four, so the smallest company
/// (AIG) is left out; and one from the week before, never shown.
fn last_weeks_checks() -> Vec<PostLink> {
    vec![
        check(
            "aig-q3-fy2026-earnings",
            "2026-11-04",
            "AIG",
            "AIG's third quarter 2026: net income of $844 million",
            "Net income was $844 million.",
        ),
        check(
            "amd-q3-fy2026-earnings",
            "2026-11-03",
            "AMD",
            "AMD's third quarter of fiscal 2026: revenue of $9.2 billion",
            "Revenue was $9.2 billion and GAAP diluted EPS was $0.75.",
        ),
        check(
            "uber-q3-fy2026-earnings",
            "2026-11-04",
            "UBER",
            "Uber's third quarter 2026: gross bookings of $49.7 billion",
            "Gross bookings were $49.7 billion; revenue was $13.5 billion.",
        ),
        check(
            "cop-q3-fy2026-earnings",
            "2026-11-05",
            "COP",
            "ConocoPhillips' third quarter 2026: net income of $1.7 billion",
            "Net income was $1.7 billion, or $1.38 per share.",
        ),
        check(
            "aapl-q4-fy2026-earnings",
            "2026-10-29",
            "AAPL",
            "Apple Q4",
            "Not last week.",
        ),
    ]
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
    World {
        base,
        es,
        ev: MemEventStore::default(),
        posts: Posts {
            rows: last_weeks_checks(),
            calls: Cell::new(0),
        },
    }
}

/// `digest_tick` with the Buttondown sender over `http`.
async fn tick(w: &World, http: &Fake, now: &str, mode: Mode) -> String {
    let logs = RefCell::new(Vec::<String>::new());
    let log = |m: &str| logs.borrow_mut().push(m.to_string());
    let bd = Buttondown::new(http, KEY, &log);
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

fn header<'a>(headers: &'a [(String, String)], k: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(k))
        .map(|(_, v)| v.as_str())
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
async fn gather_orders_writeups_counts_the_rest_and_picks_three_checks() {
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
    // Last week (Nov 2-8), largest companies first, three of four: AMD
    // ($662B), COP ($163B), UBER ($140B); AIG ($42B) is left out.
    let slugs: Vec<&str> = wk.picks.iter().map(|p| p.slug.as_str()).collect();
    assert_eq!(
        slugs,
        [
            "amd-q3-fy2026-earnings",
            "cop-q3-fy2026-earnings",
            "uber-q3-fy2026-earnings"
        ]
    );
    // One D1 read each: calendar, events (+ the loaded marker), posts.
    assert_eq!(w.posts.calls.get(), 1);
    assert_eq!(w.ev.calls.get(), 1);
}

#[tokio::test]
async fn picks_are_up_to_three_largest_first_and_the_section_goes_when_there_are_none() {
    // Published in this order; by market cap AMD > COP > UBER > AIG.
    let all = last_weeks_checks();
    let order = ["aig", "amd", "uber", "cop"];
    let by = |t: &str| all.iter().find(|p| p.slug.starts_with(t)).unwrap().clone();
    for (n, want, opening) in [
        (0, vec![], "Hi, it's Rick from PreBell. Here are the dates to know for the week of Nov 9–13."),
        (1, vec!["aig"], "Hi, it's Rick from PreBell. First, one earnings check from last week; then the dates to know for the week of Nov 9–13."),
        (2, vec!["amd", "aig"], "Hi, it's Rick from PreBell. First, two earnings checks from last week; then the dates to know for the week of Nov 9–13."),
        (3, vec!["amd", "uber", "aig"], "Hi, it's Rick from PreBell. First, three earnings checks from last week; then the dates to know for the week of Nov 9–13."),
        (4, vec!["amd", "cop", "uber"], "Hi, it's Rick from PreBell. First, three earnings checks from last week; then the dates to know for the week of Nov 9–13."),
    ] {
        let mut w = world();
        w.posts.rows = order[..n].iter().map(|t| by(t)).collect();
        let wk = week(&w, FRI_EVENING).await;
        let got: Vec<&str> = wk
            .picks
            .iter()
            .map(|p| p.slug.split('-').next().unwrap())
            .collect();
        assert_eq!(got, want, "{n} checks");
        let body = render::body(&wk);
        assert!(body.starts_with(&format!("{opening}\n\n")), "{n}: {body}");
        // Each pick: its title linked to the English page, then its summary.
        for p in &wk.picks {
            let item = format!(
                "**[{}](https://prebell.site/en/p/{})**\n\n{}\n\n",
                p.title, p.slug, p.summary
            );
            assert!(body.contains(&item), "{n}: missing {item:?} in\n{body}");
        }
        let section = body.contains("## From last week's earnings checks");
        assert_eq!(section, n > 0, "{n}: {body}");
        assert_eq!(body.contains("All earnings checks →"), n > 0, "{n}");
        // The week ahead is there whatever the picks.
        assert!(body.contains("## Market calendar"), "{n}");
    }
}

/// The whole email for the fixture week, as Buttondown receives it.
const FIXTURE_EMAIL: &str = "\
Hi, it's Rick from PreBell. First, three earnings checks from last week; then the dates to know for the week of Nov 9–13.

## From last week's earnings checks

**[AMD's third quarter of fiscal 2026: revenue of $9.2 billion](https://prebell.site/en/p/amd-q3-fy2026-earnings)**

Revenue was $9.2 billion and GAAP diluted EPS was $0.75.

**[ConocoPhillips' third quarter 2026: net income of $1.7 billion](https://prebell.site/en/p/cop-q3-fy2026-earnings)**

Net income was $1.7 billion, or $1.38 per share.

**[Uber's third quarter 2026: gross bookings of $49.7 billion](https://prebell.site/en/p/uber-q3-fy2026-earnings)**

Gross bookings were $49.7 billion; revenue was $13.5 billion.

[All earnings checks →](https://prebell.site/en/archive?type=earnings)

## Market calendar

Times are US Eastern.

### Mon, Nov 9

- Before open: 3 companies
- After close: 1 company
- Time n/a: 1 company

### Tue, Nov 10

- 8:30 AM ET: [October CPI](https://www.bls.gov/schedule/news_release/cpi.htm)
- Before open: 1 company

### Wed, Nov 11

- No earnings reports.

### Thu, Nov 12

- Before open: 1 company
- After close: Cisco (CSCO), Applied Materials (AMAT) (unconfirmed), +1 more

### Fri, Nov 13

- No earnings reports.

[Full calendar →](https://prebell.site/en/earnings?week=2026-11-09)

One question: what would make this email more useful to you? Just hit reply.

Rick

---

PreBell is generated automatically from public data and SEC filings, for information only. It is not investment advice.
";

#[tokio::test]
async fn the_fixture_week_renders_exactly() {
    let w = world();
    let wk = week(&w, FRI_EVENING).await;
    assert_eq!(render::subject(wk.monday), "Week ahead: Nov 9–13");
    let body = render::body(&wk);
    assert_eq!(body, FIXTURE_EMAIL, "\n{body}");
    // The question is the last line before the sign-off.
    let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
    let rick = lines.iter().position(|l| *l == copy::SIGN_OFF).unwrap();
    assert_eq!(lines[rick - 1], copy::QUESTION);
    // One question only (the other '?'s are in the links' URLs).
    let asks: Vec<&&str> = lines
        .iter()
        .filter(|l| l.contains('?') && !l.contains("](https://"))
        .collect();
    assert_eq!(asks, [&copy::QUESTION]);
    assert!(!body.contains("aapl") && !body.contains("Walt Disney") && !body.contains("aig"));
}

#[tokio::test]
async fn nothing_in_the_email_compares_with_estimates() {
    // The site never compares with analyst estimates: no "beat" or "miss"
    // in any copy string or in a rendered email.
    let w = world();
    let wk = week(&w, FRI_EVENING).await;
    let texts = [
        copy::SUBJECT,
        copy::OPENING,
        copy::OPENING_NONE,
        copy::PICKS_HEADING,
        copy::CALENDAR_HEADING,
        copy::TIMES_NOTE,
        copy::QUESTION,
        copy::SIGN_OFF,
        copy::FOOTER,
    ]
    .join(" ")
        + &render::body(&wk);
    let lower = texts.to_lowercase();
    for word in ["beat", "miss", "estimate", "consensus", "expectation"] {
        assert!(!lower.contains(word), "{word}");
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
    let en = render::body(&wk);
    assert_eq!(en.matches("- No earnings reports.").count(), 4, "{en}");
    assert!(
        en.contains("### Tue, Dec 15\n\n- After close: 1 company"),
        "{en}"
    );
    assert!(!en.contains("earnings checks"), "{en}");
    assert!(
        en.starts_with(
            "Hi, it's Rick from PreBell. Here are the dates to know for the week of Dec 14–18.\n\n## Market calendar"
        ),
        "{en}"
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
    let en = render::body(&wk);
    assert!(
        en.contains("### Thu, Nov 26\n\n- [US stock market closed](https://www.nyse.com/markets/hours-calendars)\n\n"),
        "{en}"
    );
    assert!(
        en.contains("### Fri, Nov 27\n\n- [Early close, 1:00 PM ET](https://www.nyse.com/markets/hours-calendars)\n- No earnings reports."),
        "{en}"
    );
    assert!(en.contains("- Before open: Deere (DE)\n"), "{en}");
    assert_eq!(render::subject(wk.monday), "Week ahead: Nov 23–27");
    // A week across two months.
    assert_eq!(
        render::subject(day("2026-11-30")),
        "Week ahead: Nov 30–Dec 4"
    );
}

#[tokio::test]
async fn draft_mode_saves_one_draft_for_every_subscriber() {
    let w = world();
    let http = fake();
    let line = tick(&w, &http, FRI_EVENING, Mode::Draft).await;
    assert_eq!(line, "2026-11-09 (draft): draft em_01");
    // One request: no tag list, no second language.
    assert_eq!(http.methods(), ["POST"]);
    let seen = http.seen.borrow().clone();
    let (_, url, headers, body) = &seen[0];
    assert_eq!(url, "https://api.buttondown.com/v1/emails");
    assert_eq!(
        header(headers, "Authorization"),
        Some("Token test-only-not-a-key")
    );
    assert_eq!(header(headers, "X-API-Version"), Some("2026-04-01"));
    assert_eq!(
        header(headers, "X-Idempotency-Key"),
        Some("prebell-digest-2026-11-09")
    );
    assert!(
        headers
            .iter()
            .all(|(n, _)| !n.to_ascii_lowercase().contains("dangerously")),
        "{headers:?}"
    );
    // Exactly subject, body and status: no filters (so no tag), no
    // publish date.
    let body = body.as_ref().unwrap();
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(keys, ["body", "status", "subject"], "{body}");
    assert_eq!(body["status"], "draft");
    assert_eq!(body["subject"], "Week ahead: Nov 9–13");
    assert!(!body.to_string().contains("lang:"), "{body}");
    assert_eq!(
        body["body"].as_str().unwrap(),
        format!("<!-- buttondown-editor-mode: plaintext -->\n{FIXTURE_EMAIL}")
    );
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
    // A second run in the same week sends nothing.
    let line = tick(&w, &http, "2026-11-07T00:00:00Z", Mode::Draft).await;
    assert_eq!(line, "2026-11-09: already built");
    assert_eq!(http.methods().len(), 1);
    assert!(w.base.alerts.borrow().is_empty());
}

#[tokio::test]
async fn scheduled_mode_schedules_the_draft_for_sunday_6pm_et() {
    let w = world();
    let http = fake();
    let line = tick(&w, &http, FRI_EVENING, Mode::Scheduled).await;
    assert_eq!(
        line,
        "2026-11-09 (scheduled 2026-11-08T23:00:00Z): scheduled em_01"
    );
    assert_eq!(http.methods(), ["POST", "PATCH"]);
    let seen = http.seen.borrow();
    // Created as a draft first; scheduled only once the answer says draft.
    assert_eq!(seen[0].3.as_ref().unwrap()["status"], "draft");
    assert!(seen[0].3.as_ref().unwrap().get("publish_date").is_none());
    assert_eq!(seen[1].1, "https://api.buttondown.com/v1/emails/em_01");
    assert_eq!(
        seen[1].3,
        Some(json!({"status": "scheduled", "publish_date": "2026-11-08T23:00:00Z"}))
    );
    assert_eq!(header(&seen[1].2, "X-API-Version"), Some("2026-04-01"));
    assert!(seen[1]
        .2
        .iter()
        .all(|(n, _)| !n.to_ascii_lowercase().contains("dangerously")));
}

#[tokio::test]
async fn an_answer_that_is_not_a_draft_is_never_scheduled_and_alerts() {
    // A status other than draft, and an answer with no status.
    for (answer, said) in [
        (json!("about_to_send"), "about_to_send"),
        (Value::Null, "no status"),
    ] {
        let w = world();
        let mut http = fake();
        http.answer_status = Some(answer.clone());
        let line = tick(&w, &http, FRI_EVENING, Mode::Scheduled).await;
        assert_eq!(
            line,
            format!(
                "2026-11-09 (scheduled 2026-11-08T23:00:00Z): email em_01 saved as {said}, not a draft"
            )
        );
        assert_eq!(http.methods(), ["POST"], "{answer:?}");
        let alerts = w.base.alerts.borrow();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert_eq!(alerts[0].0, "digest:2026-11-06");
        assert!(alerts[0].2.contains("not draft"), "{}", alerts[0].2);
        // Done: a retry would get the same email back.
        assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
    }
}

#[tokio::test]
async fn without_a_sender_it_is_a_dry_run() {
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
    assert_eq!(
        line,
        format!(
            "2026-11-09 (scheduled 2026-11-08T23:00:00Z): dry run, no sender: \"Week ahead: Nov 9–13\" (3 picks, {} bytes)",
            FIXTURE_EMAIL.len()
        )
    );
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    assert!(w.base.alerts.borrow().is_empty());
}

#[test]
fn enabled_only_when_exactly_true() {
    assert!(enabled(Some("true")));
    assert!(enabled(Some(" true ")));
    for off in [
        None,
        Some(""),
        Some("false"),
        Some("TRUE"),
        Some("yes"),
        Some("1"),
    ] {
        assert!(!enabled(off), "{off:?}");
    }
}

#[test]
fn the_gate_drops_the_digest_unless_enabled() {
    use crate::schedule::EarningsStep;
    let polled = [
        EarningsStep::Build,
        EarningsStep::Calendar,
        EarningsStep::Digest,
    ];
    // Disabled by default: unset, and any value but "true". The flag off
    // queues nothing, with or without a key.
    for off in [None, Some("false"), Some("yes")] {
        let mut f = polled.to_vec();
        gate(&mut f, off);
        assert_eq!(f, [EarningsStep::Build, EarningsStep::Calendar], "{off:?}");
    }
    let mut f = polled.to_vec();
    gate(&mut f, Some("true"));
    assert_eq!(f, polled);
}

#[test]
fn the_run_needs_the_flag_and_then_the_key() {
    for key in [None, Some(""), Some(KEY)] {
        for off in [None, Some("false"), Some("TRUE")] {
            assert_eq!(run_kind(off, key), RunKind::Off, "{off:?} {key:?}");
        }
    }
    assert_eq!(run_kind(Some("true"), None), RunKind::DryRun);
    assert_eq!(run_kind(Some("true"), Some("")), RunKind::DryRun);
    assert_eq!(run_kind(Some("true"), Some("  ")), RunKind::DryRun);
    assert_eq!(run_kind(Some("true"), Some(KEY)), RunKind::Send);
}

#[tokio::test]
async fn the_flag_on_without_a_key_sends_nothing() {
    // What the Worker does with RunKind::DryRun: no sender, so no request.
    let w = world();
    let http = fake();
    assert_eq!(run_kind(Some("true"), None), RunKind::DryRun);
    let line = digest_tick(
        utc(FRI_EVENING),
        &w.es,
        &w.ev,
        &w.posts,
        w.base.as_ref(),
        None,
        Mode::Draft,
    )
    .await
    .unwrap();
    assert!(line.contains("dry run"), "{line}");
    assert!(http.seen.borrow().is_empty());
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
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
async fn any_sender_gets_one_email_with_a_stable_idempotency_key() {
    let w = world();
    let s = Scripted {
        answers: RefCell::new(VecDeque::from([
            Err("http 503".into()),
            Ok(Sent::Done("saved".into())),
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
    assert_eq!(line, "2026-11-09 (draft): failed: http 503");
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    let line = run("2026-11-06T23:35:00Z").await.unwrap();
    assert_eq!(line, "2026-11-09 (draft): saved");
    let line = run("2026-11-06T23:40:00Z").await.unwrap();
    assert_eq!(line, "2026-11-09: already built");
    let emails = s.emails.borrow();
    assert_eq!(emails.len(), 2);
    assert!(emails
        .iter()
        .all(|e| e.idempotency_key == "prebell-digest-2026-11-09"
            && e.subject == "Week ahead: Nov 9–13"
            && e.send_at.is_none()));
    assert_eq!(emails[0], emails[1]);
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
}

#[tokio::test]
async fn buttondown_errors_alert_and_leave_the_email_for_the_next_poll() {
    // 500, then 201 on the next poll under the same idempotency key.
    let w = world();
    let http = fake();
    http.post_status.borrow_mut().push_back(500);
    let line = tick(&w, &http, FRI_EVENING, Mode::Draft).await;
    assert_eq!(line, "2026-11-09 (draft): failed: http 500");
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    {
        let alerts = w.base.alerts.borrow();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].0, "digest:2026-11-06");
        assert!(alerts[0].2.contains("http 500") && !alerts[0].2.contains(KEY));
        assert!(!alerts[0].2.contains("nope"), "no response body");
    }
    let line = tick(&w, &http, "2026-11-06T23:35:00Z", Mode::Draft).await;
    assert_eq!(line, "2026-11-09 (draft): draft em_02");
    let keys: Vec<String> = http
        .seen
        .borrow()
        .iter()
        .filter_map(|r| header(&r.2, "X-Idempotency-Key").map(String::from))
        .collect();
    assert_eq!(
        keys,
        ["prebell-digest-2026-11-09", "prebell-digest-2026-11-09"]
    );
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
    // One alert a day, however many polls fail.
    assert_eq!(w.base.alerts.borrow().len(), 1);

    // 401 (a wrong key), and the network down: nothing marked.
    for network in [false, true] {
        let w = world();
        let mut http = fake();
        http.post_status.borrow_mut().push_back(401);
        http.network = network;
        let line = tick(&w, &http, FRI_EVENING, Mode::Draft).await;
        let want = if network { "network error" } else { "http 401" };
        assert_eq!(line, format!("2026-11-09 (draft): failed: {want}"));
        assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
        assert_eq!(w.base.alerts.borrow().len(), 1);
    }

    // Scheduling fails: the draft exists but stays pending, and the retry
    // (same idempotency key, so Buttondown answers with the same draft)
    // schedules it.
    let w = world();
    let mut http = fake();
    http.patch_status = 502;
    let line = tick(&w, &http, FRI_EVENING, Mode::Scheduled).await;
    assert_eq!(
        line,
        "2026-11-09 (scheduled 2026-11-08T23:00:00Z): failed: http 502"
    );
    assert!(w.es.kv.borrow().get(KV_BUILT).is_none());
    http.patch_status = 200;
    let line = tick(&w, &http, "2026-11-06T23:35:00Z", Mode::Scheduled).await;
    assert!(line.ends_with("scheduled em_02"), "{line}");
    assert_eq!(http.methods(), ["POST", "PATCH", "POST", "PATCH"]);
    assert_eq!(w.es.kv.borrow()[KV_BUILT], "2026-11-09");
}

#[test]
fn markdown_in_a_title_or_summary_is_escaped() {
    let wk = Week {
        monday: day("2026-11-09"),
        days: vec![],
        picks: vec![check(
            "x-q3",
            "2026-11-03",
            "X",
            "Q3 [note] *big* _move_",
            "Margin <b>rose</b> to 40%",
        )],
    };
    let en = render::body(&wk);
    assert!(
        en.contains(r"**[Q3 \[note\] \*big\* \_move\_](https://prebell.site/en/p/x-q3)**"),
        "{en}"
    );
    assert!(en.contains(r"Margin \<b\>rose\</b\> to 40%"), "{en}");
    // An empty summary leaves the title alone.
    let mut wk = wk;
    wk.picks[0].summary = " ".into();
    let en = render::body(&wk);
    assert!(
        en.contains("(https://prebell.site/en/p/x-q3)**\n\n[All earnings checks →]"),
        "{en}"
    );
}

#[tokio::test]
async fn a_late_scheduled_run_keeps_the_draft() {
    // Past Sunday 18:00 ET (never in practice: polls stop Friday 22:00 ET),
    // a schedule would send at once; the draft stays a draft.
    let w = world();
    let http = fake();
    w.es.kv_set(KV_BUILT, "2026-11-02", "x").await.unwrap();
    let line = tick(&w, &http, "2026-11-08T23:30:00Z", Mode::Scheduled).await;
    assert_eq!(
        line,
        "2026-11-09 (scheduled, send time passed: draft): draft em_01"
    );
    assert_eq!(http.methods(), ["POST"]);
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
    // 1 create + 1 schedule.
    assert_eq!(http.count(), 2);
    // kv read, calendar, loaded marker, kv write (EarningsStore), one
    // events read, one posts read.
    assert_eq!(w.es.calls.get(), 4);
    assert_eq!(w.ev.calls.get() + w.posts.calls.get(), 2);
}
