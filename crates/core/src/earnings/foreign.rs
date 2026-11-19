//! Reports of foreign private issuers (20-F/6-K filers: Toyota, Alibaba,
//! Novo Nordisk, ...). A 6-K has no Item 2.02, so EDGAR's current 6-K feed is
//! read like the 8-K feed and each 6-K that a watched foreign company files
//! near its calendar date is read: when its press release (or financial
//! summary) states results for a period just ended, with figures, and is not
//! a notice of a coming release, the release date is marked `reported` with
//! the 6-K as its proof. Anything else, or any doubt, leaves the date as it
//! was: a missing tick is better than a wrong one.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::anchor::{has_word, normalize};
use crate::clock::et;
use crate::earnings::calendar::{report_changes, SAME_EVENT_DAYS};
use crate::earnings::detect::{name_mismatches, read_feed_page, same_filer, FeedStats};
use crate::earnings::store::{CalendarRow, EarningsStore};
use crate::earnings::{watchlist, Filer, WatchEntry};
use crate::edgar::{exhibit_991, filing_url, FeedEntry, FilingDoc, Filings, FEED_PAGE};
use crate::jobs::alert_once_today;
use crate::providers::ProviderError;
use crate::store::Store;

/// kv key: the newest 6-K feed acceptance time a poll read (RFC 3339).
pub const KV_6K_WATERMARK: &str = "edgar_6k_watermark";
/// kv key: the 6-K accessions already read (a JSON list, newest last), so a
/// 6-K the feed shows for hours is fetched once.
pub const KV_6K_CHECKED: &str = "edgar_6k_checked";
/// Accessions kept in `KV_6K_CHECKED` (about two weeks of candidates).
pub const CHECKED_KEEP: usize = 300;
/// Bytes of a document read: the release's opening, or a financial
/// summary's cover (Novo Nordisk's statement is 17 KB in). One 6-K message
/// (its index, this much of one document, the check) takes about 2-4 ms of
/// CPU in Wasm in a fresh isolate, well inside the Free plan's 10 ms.
pub const HEAD_BYTES: usize = 32 * 1024;
/// A 6-K accepted from this many days before the calendar date (an Asian
/// morning release is the previous US evening; Yahoo's date may be late) ...
pub const DAYS_BEFORE: i64 = 2;
/// ... to this many after it (SAP filed its Oct 21 release on Oct 26; a
/// stale date) is a candidate. Notices are rejected by `classify`.
pub const DAYS_AFTER: i64 = 7;
/// Feed pages one poll reads at most, as for the 8-K feed.
pub const MAX_PAGES: usize = 3;

/// The 20-F/6-K filers on the watchlist, by CIK.
pub fn foreign_by_cik() -> HashMap<u64, WatchEntry> {
    watchlist()
        .into_iter()
        .filter(|e| e.filer == Filer::Foreign)
        .map(|e| (e.cik, e))
        .collect()
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A period that ended at most this many days before the 6-K is the one it
/// reports (fiscal quarters ending Aug 31, 13-week quarters, late filers);
/// a prior-year comparison is always older.
pub const PERIOD_DAYS: i64 = 100;
/// The reported figures must start within this many characters after the
/// results statement (a notice of a coming release has none; Barclays'
/// first figures are 9,000 characters in).
const FIGURE_WINDOW: usize = 12_000;

/// The date in a period-end match: "September 30, 2026", "Sept. 30, 2026"
/// or "30 September 2026".
fn period_date(month: &str, day: &str, year: &str) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year.parse().ok()?, month_number(month)?, day.parse().ok()?)
}

/// 1-12 for a month name or its abbreviation ("sept.", "oct").
fn month_number(month: &str) -> Option<u32> {
    let m = month.trim_end_matches('.').to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|full| {
            let full = full.to_ascii_lowercase();
            m == full || (m.len() >= 3 && full.starts_with(&m))
        })
        .map(|i| i as u32 + 1)
}

/// The last day of `month` in `year`.
fn month_end(year: i32, month: u32) -> Option<NaiveDate> {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }?;
    next.pred_opt()
}

/// Lowercase phrases that, up to the end of the results statement's
/// sentence, mark a
/// filing that only talks about results: board-meeting and date notices,
/// pre-announcements, earnings-call notices, profit alerts, delivery or
/// monthly reports, meetings, buybacks, GAAP reconciliations, insider
/// dealing. Plain substring checks: the Free plan gives an invocation 10 ms
/// of CPU.
const NOT_A_RELEASE: &[(&str, &str)] = &[
    ("date of board meeting", "board meeting notice"),
    ("date of the board meeting", "board meeting notice"),
    ("notice of board meeting", "board meeting notice"),
    ("meeting of the board", "board meeting notice"),
    ("board meeting", "board meeting notice"),
    ("audit committee meeting", "board meeting notice"),
    ("will be held", "board meeting notice"),
    ("announcement date", "date notice"),
    ("release date", "date notice"),
    ("announces date", "date notice"),
    ("announces the date", "date notice"),
    ("scheduled to", "pre-announcement"),
    ("expected to be released", "pre-announcement"),
    ("will announce", "pre-announcement"),
    ("will report", "pre-announcement"),
    ("will release", "pre-announcement"),
    ("will publish", "pre-announcement"),
    ("will hold", "pre-announcement"),
    ("to announce", "pre-announcement"),
    ("to report", "pre-announcement"),
    ("to release", "pre-announcement"),
    ("to publish", "pre-announcement"),
    ("profit warning", "profit alert"),
    ("profit alert", "profit alert"),
    ("preliminary", "profit alert"),
    ("delivery update", "delivery update"),
    ("delivery results", "delivery update"),
    ("vehicle delivery", "delivery update"),
    ("operating data", "operating update"),
    ("operational update", "operating update"),
    ("business update", "operating update"),
    ("will be released", "pre-announcement"),
    ("will host", "pre-announcement"),
    ("expects", "outlook"),
    ("estimates", "outlook"),
    ("anticipated", "outlook"),
    ("rescheduled", "date notice"),
    ("change of date", "date notice"),
    // TSMC's monthly "Revenue Report" totals January through the month.
    ("revenue report", "monthly revenue"),
    ("monthly revenue", "monthly revenue"),
    ("vehicles were delivered", "delivery update"),
    ("monthly return", "exchange return"),
    ("next day disclosure return", "exchange return"),
    ("annual general meeting", "general meeting"),
    ("extraordinary general meeting", "general meeting"),
    ("voting results", "general meeting"),
    ("poll results", "general meeting"),
    ("share repurchase", "buyback"),
    ("buyback", "buyback"),
    ("reconciliation", "GAAP reconciliation"),
    ("transaction(s) made in", "insider dealing"),
];

/// Lowercase openers of a financial summary, report or announcement.
const SUMMARY: [&str; 9] = [
    "financial summary",
    "financial results",
    "financial report",
    "financial statement",
    "interim results",
    "annual results",
    "quarterly results",
    "quarterly statement",
    "results announcement",
];

/// Words that make an undated period ("Q3 2026", "third quarter of 2026")
/// a results statement when they are near it.
const RESULT_CUES: [&str; 8] = [
    "results",
    "earnings",
    "statement",
    "reports",
    "net income",
    "net sales",
    "revenue",
    "profit",
];

/// The last whole word `w` in `hay`, as a byte offset.
fn last_word(hay: &str, w: &str) -> Option<usize> {
    hay.rmatch_indices(w).map(|(i, _)| i).find(|&i| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + w.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// `i` moved back to a char boundary of `s`.
fn floor(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `i` moved forward to a char boundary of `s` (at most its length).
fn ceil(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// A results statement: where it starts, where it ends, and the period end
/// it states.
#[derive(Debug, Clone, Copy)]
struct Statement {
    start: usize,
    end: usize,
    period: NaiveDate,
}

/// A pattern over ASCII-lowercased text, without Unicode classes: about
/// half the compile time in Wasm (a cold Worker compiles each once).
fn regex(p: &str) -> Regex {
    RegexBuilder::new(p)
        .unicode(false)
        .build()
        .expect("valid built-in regex")
}

/// A written date: "september 30, 2026", "sept. 30, 2026", "30 september
/// 2026", and "august 26 , 2026" (`quick_text` puts a space where a tag
/// ended, as in LI's dateline), lowercase, as `(start, end, date)`.
fn dates(lower: &str) -> impl Iterator<Item = (usize, usize, NaiveDate)> + '_ {
    static DATE: OnceLock<Regex> = OnceLock::new();
    DATE.get_or_init(|| {
        regex(r"\b(?:([a-z]{3,9}\.?)\s+(\d{1,2})\s*,?\s+(\d{4})|(\d{1,2})\s+([a-z]{3,9}\.?)\s+(\d{4}))\b")
    })
    .captures_iter(lower)
    .filter_map(|c| {
        let m = c.get(0)?;
        let d = match (c.get(1), c.get(4)) {
            (Some(mo), _) => period_date(mo.as_str(), &c[2], &c[3]),
            (_, Some(day)) => period_date(&c[5], day.as_str(), &c[6]),
            _ => None,
        }?;
        Some((m.start(), m.end(), d))
    })
}

/// Every results statement in `lower` (the ASCII-lowercased text):
/// - a dated period end after "results ... for the" or a financial summary
///   or report opener: "financial results for the second quarter ended June
///   30, 2026", "Financial report for the period 1 January 2026 to 30
///   September 2026", "FINANCIAL SUMMARY ... (April 1, 2026 through June 30,
///   2026)", "Results Announcement ... relating to the nine months ended 30
///   September 2026";
/// - an undated period near a results word: "net income in Q3 2026",
///   "Second Quarter and First Half of 2026 Financial Results", "results ...
///   January - September 2026".
fn statements(lower: &str) -> Vec<Statement> {
    static UNDATED: OnceLock<Regex> = OnceLock::new();
    let undated = UNDATED.get_or_init(|| {
        regex(concat!(
            r"\bq([1-4])\s+(20\d\d)\b",
            r"|\b(first|second|third|fourth)\s+quarter(?:\s+and\s+(?:first\s+half|nine\s+months|full\s+year))?(?:\s+of)?\s+(20\d\d)\b",
            r"|\bjanuary\s*(?:-|–|—|to|through)\s*(march|june|september|december)\s+(20\d\d)\b",
            r"|\bfirst\s+(half|nine\s+months)\s+of\s+(20\d\d)\b",
        ))
    });
    let mut out = Vec::new();
    for (date_at, date_end, period) in dates(lower) {
        // "ended / ending / through / to <date>"
        let before = lower[..date_at].trim_end();
        let Some(word) = ["ended", "ending", "through", "to"].iter().find(|w| {
            before.ends_with(*w)
                && !before[..before.len() - w.len()].ends_with(|c: char| c.is_ascii_alphanumeric())
        }) else {
            continue;
        };
        let at = before.len() - word.len();
        // "... results ... for the ... ended <date>": "for the" within 100
        // characters, and "results" or "earnings" within 240 before it.
        let near = &lower[floor(lower, at.saturating_sub(100))..at];
        let lead = near.rfind("for the").and_then(|i| {
            let for_at = at - near.len() + i;
            let before = &lower[floor(lower, for_at.saturating_sub(240))..for_at];
            ["results", "earnings"]
                .iter()
                .filter_map(|w| last_word(before, w))
                .min()
                .map(|j| for_at - before.len() + j)
        });
        // "<financial summary> ... ended <date>" within 300 characters.
        let wide = &lower[floor(lower, at.saturating_sub(300))..at];
        let summary = SUMMARY
            .iter()
            .filter_map(|w| wide.find(w))
            .min()
            .map(|j| at - wide.len() + j);
        if let Some(start) = [lead, summary].into_iter().flatten().min() {
            out.push(Statement {
                start,
                end: date_end,
                period,
            });
        }
    }
    for c in undated.captures_iter(lower) {
        let Some(m) = c.get(0) else { continue };
        let year = |i: usize| c.get(i).and_then(|y| y.as_str().parse::<i32>().ok());
        let quarter_end = |q: u32, y: i32| month_end(y, q * 3);
        let period = if let (Some(q), Some(y)) = (c.get(1), year(2)) {
            quarter_end(q.as_str().parse().unwrap_or(0), y)
        } else if let (Some(o), Some(y)) = (c.get(3), year(4)) {
            // A fiscal quarter ("second fiscal quarter") is dated elsewhere.
            let q = ["first", "second", "third", "fourth"]
                .iter()
                .position(|w| *w == o.as_str())
                .map_or(0, |i| i as u32 + 1);
            quarter_end(q, y)
        } else if let (Some(mo), Some(y)) = (c.get(5), year(6)) {
            month_number(mo.as_str()).and_then(|n| month_end(y, n))
        } else if let (Some(h), Some(y)) = (c.get(7), year(8)) {
            month_end(y, if h.as_str() == "half" { 6 } else { 9 })
        } else {
            None
        };
        let Some(period) = period else { continue };
        let before = &lower[floor(lower, m.start().saturating_sub(240))..m.start()];
        let after = &lower[m.end()..ceil(lower, m.end() + 120)];
        let cue_before = RESULT_CUES
            .iter()
            .filter_map(|w| before.rfind(w))
            .max()
            .map(|j| m.start() - before.len() + j);
        let cue_after = RESULT_CUES.iter().any(|w| after.contains(w));
        if cue_before.is_none() && !cue_after {
            continue;
        }
        out.push(Statement {
            start: cue_before.unwrap_or(m.start()).min(m.start()),
            end: m.end(),
            period,
        });
    }
    out
}

/// Reported figures: an amount with a currency ("RMB32,136.9", "US$4.60",
/// "€11.7 billion", "DKK 31,720"), a table figure in the millions
/// ("24,630,753"), or an amount with a scale word next to a financial word
/// ("revenue of 5.70bn"; not XPENG's "4.14 million tons" of CO2).
fn has_figure(s: &str) -> bool {
    static MONEY: OnceLock<Regex> = OnceLock::new();
    static SCALED: OnceLock<Regex> = OnceLock::new();
    let money = MONEY.get_or_init(|| {
        regex(r"(?:rmb|us\$|hk\$|nt\$|s\$|\$|€|£|¥|dkk|eur|usd|jpy)\s?\d|\b\d{1,3}(?:,\d{3}){2,}\b")
    });
    if money.is_match(s) {
        return true;
    }
    let scaled = SCALED.get_or_init(|| regex(r"\d[\d,.]*\s?(?:billion|million|bn|mn|bln)\b"));
    scaled.find_iter(s).any(|m| {
        let near = &s[floor(s, m.start().saturating_sub(60))..m.start()];
        FINANCIAL.iter().any(|w| near.contains(w))
    })
}

/// Words that make "N million" a financial figure.
const FINANCIAL: [&str; 9] = [
    "revenue", "income", "earnings", "sales", "profit", "loss", "ebitda", "margin", "turnover",
];

/// The release's dateline: the first date from a week before the 6-K to
/// the day after it that comes before the statement, or after it right next
/// to a dateline mark ("BEIJING, China, August 18, 2026 –", "Walldorf,
/// Germany – October 21, 2026", "(GLOBE NEWSWIRE)"). A period end stated in
/// the text ("as of September 30, 2026") is not a dateline.
fn dateline(lower: &str, st: &Statement, upto: usize, accepted: NaiveDate) -> Option<NaiveDate> {
    const MARKS: [&str; 9] = [
        "–",
        "—",
        "&ndash;",
        "&mdash;",
        " - ",
        "--",
        "/prnewswire/",
        "(globe newswire)",
        "(business wire)",
    ];
    let window = accepted - Duration::days(7)..=accepted + Duration::days(1);
    dates(&lower[..ceil(lower, upto)])
        .filter(|(_, _, d)| window.contains(d) && *d != st.period)
        .find(|(at, end, _)| {
            let before = &lower[floor(lower, at.saturating_sub(8))..*at];
            let after = &lower[*end..ceil(lower, end + 20)];
            *at < st.start
                || MARKS
                    .iter()
                    .any(|m| before.contains(m) || after.contains(m))
        })
        .map(|(_, _, d)| d)
}

/// The company is named: its watchlist name as a word, or its ticker after
/// an exchange ("NYSE: TSM").
fn names_company(text: &str, lower: &str, e: &WatchEntry) -> bool {
    let t = e.ticker.to_ascii_lowercase();
    has_word(text, e.name)
        || ["nyse", "nasdaq"]
            .iter()
            .any(|x| has_word(lower, &format!("{x}: {t}")) || has_word(lower, &format!("{x}:{t}")))
}

/// The first phrase of `NOT_A_RELEASE` in `s`, with its reason. "to
/// announce" in "pleased to announce" (an HKEX results announcement) does
/// not count.
fn not_a_release(s: &str) -> Option<&'static str> {
    NOT_A_RELEASE
        .iter()
        .find(|(p, _)| {
            if *p == "to announce" {
                s.matches("to announce").count() > s.matches("pleased to announce").count()
            } else {
                s.contains(p)
            }
        })
        .map(|(_, why)| *why)
}

/// Whether `text` (the opening of a 6-K document) is `e`'s release of
/// results for a period that ended within `PERIOD_DAYS` before `accepted`
/// (the 6-K's ET acceptance date): such a statement, nothing before or in it
/// that marks a notice, reported figures after it, and the company named.
/// `Ok` carries the release's dateline when it has one; `Err` says why not.
pub fn classify(
    text: &str,
    e: &WatchEntry,
    accepted: NaiveDate,
) -> Result<Option<NaiveDate>, String> {
    let text = normalize(text);
    let lower = text.to_ascii_lowercase();
    let recent = |p: NaiveDate| (1..=PERIOD_DAYS).contains(&(accepted - p).num_days());
    let Some(st) = statements(&lower)
        .into_iter()
        .filter(|s| recent(s.period))
        .min_by_key(|s| s.start)
    else {
        return Err(format!(
            "no results for a period ended within {PERIOD_DAYS} days"
        ));
    };
    // Up to the end of the statement's sentence: "results for the quarter
    // ended September 30, 2026 are expected to be released on ...".
    let tail = &lower[st.end..ceil(&lower, st.end + 300)];
    let sentence = st.end + tail.find(". ").unwrap_or(0);
    if let Some(why) = not_a_release(&lower[..sentence]) {
        return Err(why.to_string());
    }
    if !has_figure(&lower[st.start..ceil(&lower, st.end + FIGURE_WINDOW)]) {
        return Err("no reported figures".into());
    }
    if !names_company(&text, &lower, e) {
        return Err(format!("{} not named", e.name));
    }
    Ok(dateline(&lower, &st, st.end + 3000, accepted))
}

/// The document to read: an `EX-99` exhibit whose description reads like
/// results ("FY2027 SEMI-ANNUAL FINANCIAL SUMMARY" beside Toyota's other
/// notices), else the press release (`EX-99.1`), else the 6-K itself (Novo
/// Nordisk and TSMC file the announcement as the main document).
pub fn results_doc(docs: &[FilingDoc]) -> Option<&FilingDoc> {
    let results = |desc: &str| {
        let d = desc.to_ascii_lowercase();
        [
            "results",
            "financial summary",
            "financial report",
            "financial statements",
            "earnings",
            "quarter",
        ]
        .iter()
        .any(|w| d.contains(w))
    };
    let html = |d: &&FilingDoc| {
        let n = d.name.to_ascii_lowercase();
        n.ends_with(".htm") || n.ends_with(".html") || n.ends_with(".txt")
    };
    docs.iter()
        .filter(html)
        .find(|d| d.doc_type.to_ascii_uppercase().starts_with("EX-99") && results(&d.description))
        .or_else(|| exhibit_991(docs))
        .or_else(|| {
            docs.iter()
                .filter(html)
                .find(|d| d.doc_type.eq_ignore_ascii_case("6-K"))
        })
}

/// A 6-K waiting to be read by its own `earnings:6k` message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub cik: u64,
    pub ticker: String,
    pub accession: String,
    /// UTC acceptance time (RFC 3339).
    pub accepted_at: String,
    /// Reads that found no document yet (404/410), to retry once.
    #[serde(default)]
    pub missing: u8,
    /// Reads that failed on the network or a 5xx.
    #[serde(default)]
    pub failed: u8,
}

/// What a poll's 6-K part did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForeignOutcome {
    /// 6-K feed pages read.
    pub pages: usize,
    /// 6-Ks newly added to the pending list.
    pub found: usize,
    /// 6-Ks waiting to be read after this poll.
    pub pending: usize,
    /// `earnings:6k` messages to queue (one 6-K each).
    pub reads: usize,
    pub errors: Vec<String>,
    /// SEC answered 403/429 and the 6-K part stopped.
    pub throttled: bool,
    /// The feed error, when the 6-K feed's reply was not a feed (`NotFeed`)
    /// and its one retry failed too, not refused: `poll_job` alerts once a day.
    pub not_feed: Option<String>,
    /// The 6-K feed's non-feed replies and retries (`read_feed_page`).
    pub feed_stats: FeedStats,
    /// Foreign rows whose CIK files under another name, as `(ticker,
    /// message)`: `poll_job` alerts once a day, like the 8-K watchlist.
    pub mismatches: Vec<(&'static str, String)>,
}

/// What one `earnings:6k` message did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadOutcome {
    /// `TICKER accession` read, when one was.
    pub read: Option<String>,
    /// The calendar date marked reported.
    pub reported: Option<String>,
    /// Why the 6-K is not a results release.
    pub rejected: Option<String>,
    pub errors: Vec<String>,
    /// SEC answered 403/429: the 6-K stays pending.
    pub throttled: bool,
    /// A recent 403/429 held this message back: nothing fetched.
    pub held: bool,
}

/// kv key: the 6-Ks found near a calendar date and not yet read (a JSON
/// list, oldest first).
pub const KV_6K_PENDING: &str = "edgar_6k_pending";
/// `earnings:6k` messages a poll queues at most; the rest wait for the next
/// firing.
pub const MAX_READS: usize = 3;
/// Pending 6-Ks kept at most.
const PENDING_KEEP: usize = 50;
/// Network or 5xx failures of one 6-K before it is given up.
const MAX_FAILED: u8 = 5;
/// kv key: no `earnings:6k` message fetches from SEC before this time
/// (RFC 3339), set after a 403/429, so the rest of a poll's messages wait.
pub const KV_6K_BACKOFF: &str = "edgar_6k_backoff";
/// How long a 403/429 holds the 6-K reads back.
const BACKOFF_MINUTES: i64 = 15;

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// The calendar row a 6-K accepted on `day` may report: the ticker's
/// unreported row dated `day - DAYS_AFTER ..= day + DAYS_BEFORE`, nearest
/// first. None when a report of that event is already on the calendar.
fn target(rows: &[CalendarRow], day: NaiveDate) -> Option<NaiveDate> {
    let near = |d: NaiveDate| (day - d).num_days();
    let dated = |r: &CalendarRow| parse_date(&r.report_date);
    let reported = rows
        .iter()
        .filter(|r| r.status == "reported")
        .filter_map(dated);
    let mut open: Vec<NaiveDate> = rows
        .iter()
        .filter(|r| r.status != "reported")
        .filter_map(dated)
        .filter(|d| (-DAYS_BEFORE..=DAYS_AFTER).contains(&near(*d)))
        .collect();
    open.sort_by_key(|d| near(*d).abs());
    let d = *open.first()?;
    let done = reported
        .into_iter()
        .any(|r| (r - d).num_days().abs() <= SAME_EVENT_DAYS);
    (!done).then_some(d)
}

async fn kv_list<T: serde::de::DeserializeOwned>(
    es: &dyn EarningsStore,
    key: &str,
) -> Result<Vec<T>, String> {
    Ok(es
        .kv_get(key)
        .await?
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}

async fn kv_put<T: Serialize>(
    es: &dyn EarningsStore,
    key: &str,
    list: &[T],
    keep: usize,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let from = list.len().saturating_sub(keep);
    let json = serde_json::to_string(&list[from..]).map_err(|e| e.to_string())?;
    es.kv_set(key, &json, &iso(now)).await
}

fn accepted_day(p: &Pending) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(&p.accepted_at)
        .ok()
        .map(|t| et(t.with_timezone(&Utc)).date_naive())
}

/// The poll's 6-K part: reads the current 6-K feed back to its watermark
/// (as the 8-K poll does) and adds each new 6-K that a watched foreign
/// company filed near its calendar date to the pending list. No 6-K is read
/// here: each is read by its own `earnings:6k` message (`foreign_read`), so
/// its CPU never adds to the poll's. D1: the watermark, checked and pending
/// reads, one ranged calendar read when a new 6-K is seen, and the pending
/// and watermark writes. A store error aborts (`poll_job` logs it).
pub async fn foreign_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    filings: &dyn Filings,
) -> Result<ForeignOutcome, String> {
    let mut out = ForeignOutcome::default();
    let watermark = es
        .kv_get(KV_6K_WATERMARK)
        .await?
        .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.with_timezone(&Utc));
    let mut entries: Vec<FeedEntry> = Vec::new();
    let mut complete = false;
    for page in 0..MAX_PAGES {
        let label = format!("current 6-K filings page {page}");
        let read = read_feed_page(
            &label,
            || filings.current_6k_filings(page * FEED_PAGE),
            &mut out.feed_stats,
            &mut out.errors,
        );
        let list = match read.await {
            Ok(l) => l,
            Err((e, alert)) => {
                out.throttled = matches!(e, ProviderError::Http(403 | 429));
                // Still failing after one retry (a refused retry is a throttle).
                out.not_feed = alert;
                break;
            }
        };
        out.pages += 1;
        let full = list.len() >= FEED_PAGE;
        let oldest = list.iter().map(|f| f.accepted_at).min();
        entries.extend(list);
        let more = matches!((watermark, oldest), (Some(w), Some(o)) if full && o > w);
        if !more {
            complete = true;
            break;
        }
        if page + 1 == MAX_PAGES {
            out.errors.push(format!(
                "current 6-K filings: {MAX_PAGES} pages did not reach the last poll; older 6-Ks may be missed"
            ));
            complete = true;
        }
    }

    let watch = foreign_by_cik();
    out.mismatches = name_mismatches(&entries, &watch);
    let checked: Vec<String> = kv_list(es, KV_6K_CHECKED).await?;
    let mut pending: Vec<Pending> = kv_list(es, KV_6K_PENDING).await?;
    let before = pending.clone();
    // A 6-K older than the window is no longer near its date.
    let today = et(now).date_naive();
    let old = |d: NaiveDate| (today - d).num_days() > DAYS_AFTER + 1;
    pending.retain(|p| accepted_day(p).is_some_and(|d| !old(d)));
    let mut fresh: Vec<(&FeedEntry, &WatchEntry)> = Vec::new();
    for f in &entries {
        // An amendment repeats a release already counted (or corrects one).
        let Some(w) = watch.get(&f.cik).filter(|_| f.form == "6-K") else {
            continue;
        };
        if !same_filer(&f.company, w.sec)
            || old(et(f.accepted_at).date_naive())
            || checked.contains(&f.accession)
            || pending.iter().any(|p| p.accession == f.accession)
            || fresh.iter().any(|(g, _)| g.accession == f.accession)
        {
            continue;
        }
        fresh.push((f, w));
    }
    // Oldest first, so a burst is read in order.
    fresh.sort_by_key(|(f, _)| f.accepted_at);
    let day = |f: &FeedEntry| et(f.accepted_at).date_naive();
    if let (Some(first), Some(last)) = (
        fresh.iter().map(|(f, _)| day(f)).min(),
        fresh.iter().map(|(f, _)| day(f)).max(),
    ) {
        let span = Duration::days(SAME_EVENT_DAYS + DAYS_AFTER);
        let mut by_ticker: BTreeMap<String, Vec<CalendarRow>> = BTreeMap::new();
        for r in es
            .calendar_between(&ymd(first - span), &ymd(last + span))
            .await?
        {
            by_ticker.entry(r.ticker.clone()).or_default().push(r);
        }
        for (f, w) in fresh {
            // Not near a calendar date: not a candidate, and not marked
            // checked (cheap to look at again).
            let rows = by_ticker.get(w.ticker).map_or(&[][..], Vec::as_slice);
            if target(rows, day(f)).is_none() {
                continue;
            }
            out.found += 1;
            pending.push(Pending {
                cik: f.cik,
                ticker: w.ticker.to_string(),
                accession: f.accession.clone(),
                accepted_at: iso(f.accepted_at),
                missing: 0,
                failed: 0,
            });
        }
    }
    if pending != before {
        kv_put(es, KV_6K_PENDING, &pending, PENDING_KEEP, now).await?;
    }
    out.pending = pending.len().min(PENDING_KEEP);
    out.reads = out.pending.min(MAX_READS);
    let newest = entries.iter().map(|f| f.accepted_at).max();
    if let (true, Some(n)) = (complete, newest) {
        if watermark.is_none_or(|w| n > w) {
            es.kv_set(KV_6K_WATERMARK, &iso(n), &iso(now)).await?;
        }
    }
    Ok(out)
}

/// One `earnings:6k` message: reads the oldest pending 6-K (its index, then
/// the opening of its results document: 2 fetches) and, when it is a
/// results release, marks its release date reported with the 6-K as proof
/// (moving the calendar date when it differs, as the 8-K path does).
/// Idempotent: a redelivered message finds that 6-K checked and reads the
/// next one, or nothing. A 6-K whose ticker was reported meanwhile is
/// dropped unread. A 403/429 raises the daily `sec-throttled` alert, leaves
/// the 6-K pending and holds every message back for `BACKOFF_MINUTES`; a
/// network failure leaves it pending; a missing document (404/410) is
/// retried once by a later message. D1: the backoff, pending and checked
/// reads, the ticker's calendar rows, at most one calendar write, and the
/// two list writes.
pub async fn foreign_read(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    store: &dyn Store,
    filings: &dyn Filings,
) -> Result<ReadOutcome, String> {
    let mut out = ReadOutcome::default();
    let held = es
        .kv_get(KV_6K_BACKOFF)
        .await?
        .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .is_some_and(|until| now < until.with_timezone(&Utc));
    if held {
        out.held = true;
        return Ok(out);
    }
    let mut pending: Vec<Pending> = kv_list(es, KV_6K_PENDING).await?;
    if pending.is_empty() {
        return Ok(out);
    }
    let mut checked: Vec<String> = kv_list(es, KV_6K_CHECKED).await?;
    let mut p = pending.remove(0);
    let label = format!("{} {}", p.ticker, p.accession);
    let watch = foreign_by_cik();
    let (Some(w), Some(day)) = (watch.get(&p.cik), accepted_day(&p)) else {
        kv_put(es, KV_6K_PENDING, &pending, PENDING_KEEP, now).await?;
        return Ok(out);
    };
    if !checked.contains(&p.accession) {
        let rows = es.calendar_for(w.ticker).await?;
        // Reported meanwhile (an 8-K, another 6-K): nothing to read.
        if let Some(d) = target(&rows, day) {
            let f = FeedEntry {
                form: "6-K".into(),
                cik: p.cik,
                company: w.sec.to_string(),
                accession: p.accession.clone(),
                filing_date: ymd(day),
                items: vec![],
                accepted_at: DateTime::parse_from_rfc3339(&p.accepted_at)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or(now),
                index_url: filing_url(p.cik, &p.accession),
            };
            match read(filings, &f).await {
                Ok(text) => {
                    out.read = Some(label.clone());
                    checked.push(p.accession.clone());
                    match classify(&text, w, day) {
                        Ok(dated) => {
                            // The release's own date (its dateline), else the
                            // 6-K's when that is later than the calendar's; an
                            // earlier 6-K is the Asian-morning case and keeps
                            // the calendar date.
                            let released = dated.unwrap_or_else(|| day.max(d));
                            let ch = report_changes(
                                &rows,
                                w.ticker,
                                released,
                                filing_url(p.cik, &p.accession),
                                now,
                            );
                            if !ch.is_empty() {
                                es.write_calendar(&ch).await?;
                            }
                            out.reported = Some(ymd(released));
                        }
                        Err(why) => out.rejected = Some(format!("{label}: {why}")),
                    }
                }
                Err(e) => {
                    out.errors.push(format!("{label}: {e}"));
                    match e {
                        ProviderError::Http(403 | 429) => {
                            out.throttled = true;
                            pending.insert(0, p);
                            let until = now + Duration::minutes(BACKOFF_MINUTES);
                            es.kv_set(KV_6K_BACKOFF, &iso(until), &iso(now)).await?;
                            let msg = format!(
                                "SEC EDGAR refused requests (403/429) reading a 6-K: {}",
                                out.errors.last().cloned().unwrap_or_default()
                            );
                            alert_once_today(now, "sec-throttled", "error", &msg, store).await?;
                        }
                        // Not there yet, or not at all: one more try later.
                        ProviderError::Http(404 | 410) if p.missing == 0 => {
                            p.missing = 1;
                            pending.push(p);
                        }
                        // No readable document: it reads the same next time.
                        ProviderError::Http(404 | 410)
                        | ProviderError::Empty
                        | ProviderError::EmptyReply(_)
                        | ProviderError::Parse(_) => {
                            checked.push(p.accession.clone());
                        }
                        _ if p.failed + 1 < MAX_FAILED => {
                            p.failed += 1;
                            pending.push(p);
                        }
                        _ => checked.push(p.accession.clone()),
                    }
                }
            }
        } else {
            checked.push(p.accession.clone());
        }
    }
    kv_put(es, KV_6K_CHECKED, &checked, CHECKED_KEEP, now).await?;
    kv_put(es, KV_6K_PENDING, &pending, PENDING_KEEP, now).await?;
    Ok(out)
}

/// The 6-K's index, then the opening of its results document (2 fetches).
async fn read(filings: &dyn Filings, f: &FeedEntry) -> Result<String, ProviderError> {
    let docs = filings.filing_docs(f.cik, &f.accession).await?;
    let doc = results_doc(&docs).ok_or(ProviderError::Empty)?;
    filings.document_head(&doc.url, HEAD_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::find;
    use crate::earnings::store::MemEarningsStore;
    use crate::edgar::{head_text, parse_current_feed, parse_filing_index};
    use crate::store::MemStore;
    use async_trait::async_trait;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }
    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    /// A fixture's opening as `document_head` returns it. The fixtures are
    /// the first 32 KB of real 6-K documents fetched from EDGAR on
    /// 2026-11-06.
    fn fx(name: &str) -> String {
        let path = format!("{}/tests/fixtures/sixk/{name}", env!("CARGO_MANIFEST_DIR"));
        head_text(&String::from_utf8_lossy(&std::fs::read(path).unwrap()))
    }
    fn check(ticker: &str, name: &str, accepted: &str) -> Result<(), String> {
        classify(&fx(name), &find(ticker).unwrap(), d(accepted)).map(|_| ())
    }

    #[test]
    fn real_results_releases_are_recognised() {
        for (ticker, name, accepted) in [
            // "Alibaba Group Announces June Quarter 2026 Results"
            ("BABA", "baba_results.htm", "2026-08-20"),
            // "PDD Holdings Announces Second Quarter 2026 Unaudited Financial Results"
            ("PDD", "pdd_results.htm", "2026-08-25"),
            // "NIO Inc. Reports Unaudited Second Quarter 2026 Financial Results"
            ("NIO", "nio_results.htm", "2026-09-01"),
            // "JD.COM ANNOUNCES SECOND QUARTER AND INTERIM 2026 RESULTS"
            ("JD", "jd_results.htm", "2026-08-13"),
            // "TSMC Reports Third Quarter EPS of NT$29.43" (named by "NYSE: TSM")
            ("TSM", "tsm_results.htm", "2026-10-15"),
            // Toyota's financial summaries (EX-99.1 in August, EX-99.2 in November)
            ("TM", "tm_q1_summary.htm", "2026-08-04"),
            ("TM", "tm_semiannual_summary.htm", "2026-11-05"),
            // Novo Nordisk: "Financial report for the period 1 January 2026 to 30 September 2026"
            ("NVO", "nvo_results.htm", "2026-11-04"),
            // From the review corpus: fiscal quarters ended August 31 ...
            ("TAL", "tal_results.htm", "2026-10-29"),
            ("EDU", "edu_results.htm", "2026-10-28"),
            // ... "net income in Q3 2026" (13-week quarters, no end date) ...
            ("ASML", "asml_results.htm", "2026-10-14"),
            // ... "Second Quarter and First Half of 2026 Financial Results" ...
            ("TCOM", "tcom_results.htm", "2026-09-16"),
            // ... "3RD QUARTER RESULTS ... nine months ended 30 September 2026" ...
            ("BCS", "bcs_results.htm", "2026-10-22"),
            // ... "the first nine months of 2026" ...
            ("SAN", "san_results.htm", "2026-10-28"),
            // ... "Quarterly Statement Q3 2026", filed five days after its date ...
            ("SAP", "sap_results.htm", "2026-10-26"),
            // ... and the HKEX "ANNOUNCEMENT OF ... RESULTS" form.
            ("NTES", "ntes_hkex_results.htm", "2026-08-20"),
        ] {
            assert_eq!(check(ticker, name, accepted), Ok(()), "{name}");
        }
    }

    #[test]
    fn dates_survive_split_tags() {
        let found = |t: &str| dates(t).map(|(_, _, d)| d).collect::<Vec<_>>();
        assert_eq!(
            found("beijing, china, august 26 , 2026 -"),
            [d("2026-08-26")]
        );
        assert_eq!(found("ended 30 september 2026"), [d("2026-09-30")]);
        // LI's September delivery update: its dateline reads, and it is still
        // not a results release.
        let lower = normalize(&fx("li_september_deliveries.htm")).to_ascii_lowercase();
        assert!(found(&lower).contains(&d("2026-10-01")));
        assert!(check("LI", "li_september_deliveries.htm", "2026-10-01").is_err());
    }

    #[test]
    fn a_figure_is_money_or_a_financial_amount() {
        assert!(has_figure("total revenues were rmb32,136.9 million"));
        assert!(has_figure("net sales of €11.7 billion"));
        assert!(has_figure("sales 24,630,753 operating income"));
        assert!(has_figure("revenue reached 5.70bn"));
        // Operating data in millions is not a reported figure.
        assert!(!has_figure("reduced carbon emissions by 4.14 million tons"));
        assert!(!has_figure("125.8 million passengers in 2026"));
        assert!(!has_figure("a meeting on november 16, 2026 at 10:00"));
    }

    #[test]
    fn a_period_end_is_not_a_dateline() {
        let e = find("NIO").unwrap();
        // Filed within a week of the quarter end, with no dateline mark.
        let text = "NIO Inc. (NYSE: NIO) today announced its unaudited financial results for the quarter ended September 30, 2026. Total revenues were RMB32.1 billion.";
        assert_eq!(classify(text, &e, d("2026-10-02")), Ok(None));
        let dated = "SHANGHAI, China, October 2, 2026 (GLOBE NEWSWIRE) -- NIO Inc. (NYSE: NIO) today announced its unaudited financial results for the quarter ended September 30, 2026. Total revenues were RMB32.1 billion.";
        assert_eq!(
            classify(dated, &e, d("2026-10-02")),
            Ok(Some(d("2026-10-02")))
        );
    }

    #[test]
    fn the_release_date_is_its_dateline() {
        let at = |ticker: &str, name: &str, accepted: &str| {
            classify(&fx(name), &find(ticker).unwrap(), d(accepted)).unwrap()
        };
        // "BEIJING, China, August 26 , 2026": a tag ended before the comma.
        assert_eq!(
            at("LI", "li_results.htm", "2026-08-26"),
            Some(d("2026-08-26"))
        );
        // "SHANGHAI, China, September 1, 2026"
        assert_eq!(
            at("NIO", "nio_results.htm", "2026-09-01"),
            Some(d("2026-09-01"))
        );
        // Released September 15 US time, filed the 16th.
        assert_eq!(
            at("TCOM", "tcom_results.htm", "2026-09-16"),
            Some(d("2026-09-15"))
        );
        // "Walldorf, Germany – October 21, 2026", filed the 26th.
        assert_eq!(
            at("SAP", "sap_results.htm", "2026-10-26"),
            Some(d("2026-10-21"))
        );
    }

    #[test]
    fn real_6ks_that_are_not_results_are_rejected() {
        for (ticker, name, accepted, why) in [
            (
                "NIO",
                "nio_august_deliveries.htm",
                "2026-09-01",
                "no results",
            ),
            (
                "NIO",
                "nio_october_deliveries.htm",
                "2026-11-02",
                "no results",
            ),
            // HKEX "DATE OF BOARD MEETING": names the coming results' period.
            (
                "NIO",
                "nio_board_meeting.htm",
                "2026-08-20",
                "board meeting notice",
            ),
            (
                "BABA",
                "baba_board_meeting.htm",
                "2026-11-06",
                "board meeting notice",
            ),
            ("JD", "jd_board_meeting.htm", "2026-07-31", ""),
            ("BABA", "baba_agm_voting.htm", "2026-09-22", ""),
            ("BABA", "baba_buyback_return.htm", "2026-08-26", ""),
            ("TM", "tm_repurchase_notice.htm", "2026-11-04", ""),
            ("TM", "tm_org_changes.htm", "2026-11-05", ""),
            ("NVO", "nvo_insider_trades.htm", "2026-11-05", ""),
            ("PDD", "pdd_director.htm", "2026-08-21", ""),
            ("TSM", "tsm_monthly_revenue.htm", "2026-10-08", ""),
            ("NIO", "nio_gaap_ifrs.htm", "2026-09-01", ""),
            // The review's wrong ticks: board-meeting and results-date
            // notices naming the coming results' period.
            (
                "XPEV",
                "xpev_board_meeting_q3.htm",
                "2026-11-02",
                "board meeting notice",
            ),
            (
                "XPEV",
                "xpev_board_meeting_q2.htm",
                "2026-08-04",
                "board meeting notice",
            ),
            (
                "HTHT",
                "htht_results_date_notice.htm",
                "2026-11-04",
                "board meeting notice",
            ),
            (
                "TCOM",
                "tcom_results_date_notice.htm",
                "2026-09-02",
                "board meeting notice",
            ),
            ("TME", "tme_to_report.htm", "2026-10-13", "pre-announcement"),
            // "XPENG Announces Vehicle Delivery Results for September and
            // Third Quarter 2026", with "4.14 million tons" of CO2.
            (
                "XPEV",
                "xpev_delivery_results.htm",
                "2026-10-02",
                "delivery update",
            ),
        ] {
            let r = check(ticker, name, accepted);
            assert!(r.is_err(), "{name} taken as results");
            assert!(r.unwrap_err().contains(why), "{name}");
        }
    }

    #[test]
    fn the_wrong_quarter_or_company_is_rejected() {
        // Alibaba's June-quarter release read after the September quarter ended.
        assert!(check("BABA", "baba_results.htm", "2026-11-19")
            .unwrap_err()
            .starts_with("no results for a period ended within 100 days"));
        // PDD's release under Alibaba's CIK (a wrong row).
        assert_eq!(
            check("BABA", "pdd_results.htm", "2026-08-25"),
            Err("Alibaba not named".into())
        );
    }

    #[test]
    fn pre_announcements_are_rejected() {
        let e = find("BIDU").unwrap();
        for text in [
            "Baidu, Inc. to Report Third Quarter 2026 Financial Results on November 18, 2026. Baidu, Inc. (NASDAQ: BIDU) will announce its unaudited financial results for the third quarter ended September 30, 2026, before U.S. markets open.",
            "Baidu Announces Date for Third Quarter 2026 Earnings. Baidu (NASDAQ: BIDU) plans to release its earnings for the quarter ended September 30, 2026 on November 18.",
            // The review's synthetic notices.
            "Baidu Announces Third Quarter 2026 Earnings Release Date. Baidu (NASDAQ: BIDU) results for the third quarter ended September 30, 2026 will be released on November 18.",
            "Baidu (NASDAQ: BIDU) management is scheduled to hold an earnings call to discuss results for the quarter ended September 30, 2026. Revenue was RMB32.1 billion last year.",
            "Baidu (NASDAQ: BIDU) results for the quarter ended September 30, 2026 are expected to be released on November 18. Revenue was RMB32.1 billion last year.",
            "POSITIVE PROFIT ALERT. Baidu (NASDAQ: BIDU) is finalising its interim results for the three months ended September 30, 2026; profit is expected to rise to RMB9 billion.",
        ] {
            let why = classify(text, &e, d("2026-11-04")).unwrap_err();
            assert!(
                ["pre-announcement", "date notice", "profit alert", "no reported figures"]
                    .contains(&why.as_str()),
                "{text}: {why}"
            );
        }
        let release = "Baidu Announces Third Quarter 2026 Results. BEIJING, Nov. 18, 2026 -- Baidu, Inc. (NASDAQ: BIDU) today announced its unaudited financial results for the third quarter ended September 30, 2026. Total revenues were RMB32.1 billion. Baidu will hold an earnings conference call.";
        assert_eq!(
            classify(release, &e, d("2026-11-18")),
            Ok(Some(d("2026-11-18")))
        );
    }

    #[test]
    fn the_results_document_is_picked_from_the_index() {
        // Toyota's November 6-K: org changes (99.1), the semi-annual
        // financial summary (99.2), a presentation (99.3), a notice (99.4).
        let docs = parse_filing_index(&fx_raw("tm_semiannual_index.htm"));
        let doc = results_doc(&docs).unwrap();
        assert_eq!(doc.doc_type, "EX-99.2");
        assert!(doc.url.ends_with("/d87505dex992.htm"), "{}", doc.url);
        let doc = |name: &str, t: &str, desc: &str| FilingDoc {
            name: name.into(),
            description: desc.into(),
            doc_type: t.into(),
            url: format!("https://www.sec.gov/Archives/edgar/data/1/2/{name}"),
        };
        // Generic descriptions: the press release; none: the 6-K itself.
        let generic = [
            doc("a.htm", "6-K", "FORM 6-K"),
            doc("b.htm", "EX-99.1", "EXHIBIT 99.1"),
        ];
        assert_eq!(results_doc(&generic).unwrap().name, "b.htm");
        let main = [doc("a.htm", "6-K", "6-K"), doc("g.jpg", "GRAPHIC", "")];
        assert_eq!(results_doc(&main).unwrap().name, "a.htm");
        assert!(results_doc(&[doc("g.jpg", "GRAPHIC", "")]).is_none());
    }

    fn fx_raw(name: &str) -> String {
        let path = format!("{}/tests/fixtures/sixk/{name}", env!("CARGO_MANIFEST_DIR"));
        String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
    }

    #[test]
    fn the_real_6k_feed_parses() {
        let f = parse_current_feed(&fx_raw("current_6k.atom")).unwrap();
        assert_eq!(f.len(), 3);
        assert!(f.iter().all(|e| e.form == "6-K" && e.items.is_empty()));
        assert_eq!(f[0].company, "AKZO NOBEL NV");
    }

    type FeedRow = (u64, String, DateTime<Utc>, &'static str);

    /// Serves a 6-K feed of `(cik, accession, accepted, form)` and, per
    /// accession, an index with one EX-99.1 and that document's text.
    #[derive(Default)]
    struct Fake {
        feed: RefCell<Vec<FeedRow>>,
        docs: RefCell<HashMap<String, String>>,
        fetches: Cell<usize>,
        /// Full pages of other companies' 6-Ks, newer than any in `feed`.
        filler_pages: Cell<usize>,
        /// This page answers 429.
        throttle_page: Cell<Option<usize>>,
        /// Accessions whose index lists no readable document.
        bare: RefCell<Vec<String>>,
        /// Errors a document read answers first, per accession, in order.
        head_errors: RefCell<HashMap<String, Vec<ProviderError>>>,
    }
    impl Fake {
        fn add(
            &self,
            ticker: &str,
            n: usize,
            at: &str,
            form: &'static str,
            text: Option<String>,
        ) -> String {
            let e = find(ticker).unwrap();
            let acc = format!("{:010}-26-{n:06}", e.cik);
            self.feed
                .borrow_mut()
                .push((e.cik, acc.clone(), utc(at), form));
            if let Some(t) = text {
                self.docs.borrow_mut().insert(acc.clone(), t);
            }
            acc
        }
    }
    #[async_trait(?Send)]
    impl Filings for Fake {
        async fn current_filings(&self, _: usize) -> Result<Vec<FeedEntry>, ProviderError> {
            unreachable!()
        }
        async fn current_6k_filings(&self, start: usize) -> Result<Vec<FeedEntry>, ProviderError> {
            self.fetches.set(self.fetches.get() + 1);
            let page = start / FEED_PAGE;
            if self.throttle_page.get() == Some(page) {
                return Err(ProviderError::Http(429));
            }
            if page < self.filler_pages.get() {
                return Ok((0..FEED_PAGE)
                    .map(|i| {
                        let acc = format!("0000000001-26-{page:02}{i:04}");
                        FeedEntry {
                            form: "6-K".into(),
                            cik: 1,
                            company: "Other Co".into(),
                            filing_date: "2026-09-01".into(),
                            items: vec![],
                            accepted_at: utc("2026-09-01T23:00:00Z")
                                - Duration::minutes((page * FEED_PAGE + i) as i64),
                            index_url: filing_url(1, &acc),
                            accession: acc,
                        }
                    })
                    .collect());
            }
            if page > self.filler_pages.get() {
                return Ok(vec![]);
            }
            let mut v: Vec<FeedEntry> = self
                .feed
                .borrow()
                .iter()
                .map(|(cik, acc, at, form)| FeedEntry {
                    form: form.to_string(),
                    cik: *cik,
                    company: crate::earnings::by_cik(*cik)
                        .map_or("Other Co".into(), |w| w.sec.to_string()),
                    accession: acc.clone(),
                    filing_date: ymd(et(*at).date_naive()),
                    items: vec![],
                    accepted_at: *at,
                    index_url: filing_url(*cik, acc),
                })
                .collect();
            v.sort_by_key(|f| std::cmp::Reverse(f.accepted_at));
            Ok(v)
        }
        async fn filing_docs(&self, cik: u64, acc: &str) -> Result<Vec<FilingDoc>, ProviderError> {
            self.fetches.set(self.fetches.get() + 1);
            if self.bare.borrow().iter().any(|a| a == acc) {
                return Ok(vec![FilingDoc {
                    name: "g.jpg".into(),
                    description: String::new(),
                    doc_type: "GRAPHIC".into(),
                    url: filing_url(cik, acc),
                }]);
            }
            Ok(vec![FilingDoc {
                name: "ex99.htm".into(),
                description: "EXHIBIT 99.1".into(),
                doc_type: "EX-99.1".into(),
                url: format!(
                    "https://www.sec.gov/Archives/edgar/data/{cik}/{}/ex99.htm",
                    acc.replace('-', "")
                ),
            }])
        }
        async fn document_text(&self, _: &str) -> Result<String, ProviderError> {
            unreachable!()
        }
        async fn document_head(&self, url: &str, max: usize) -> Result<String, ProviderError> {
            assert_eq!(max, HEAD_BYTES);
            self.fetches.set(self.fetches.get() + 1);
            for (acc, errs) in self.head_errors.borrow_mut().iter_mut() {
                if url.contains(&acc.replace('-', "")) && !errs.is_empty() {
                    return Err(errs.remove(0));
                }
            }
            let docs = self.docs.borrow();
            let (_, text) = docs
                .iter()
                .find(|(acc, _)| url.contains(&acc.replace('-', "")))
                .ok_or(ProviderError::Http(503))?;
            Ok(text.clone())
        }
        async fn company_concept(
            &self,
            _: u64,
            _: &str,
        ) -> Result<Option<crate::edgar::Concept>, ProviderError> {
            unreachable!()
        }
    }

    fn stores() -> MemEarningsStore {
        MemEarningsStore::new(Rc::new(MemStore::default()))
    }
    async fn seed(es: &MemEarningsStore, ticker: &str, date: &str) {
        es.upsert_calendar(&CalendarRow {
            ticker: ticker.into(),
            report_date: date.into(),
            time_slot: "bmo".into(),
            status: "unconfirmed".into(),
            source: "yahoo".into(),
            proof_url: None,
            proof_quote: None,
            conflict_reason: None,
            first_seen_at: "2026-08-01T10:00:00Z".into(),
            checked_at: "2026-08-01T10:00:00Z".into(),
            next_check_at: None,
            updated_at: "2026-08-01T10:00:00Z".into(),
        })
        .await
        .unwrap();
    }
    fn status(es: &MemEarningsStore, ticker: &str) -> Vec<(String, String, Option<String>)> {
        es.calendar
            .borrow()
            .values()
            .filter(|r| r.ticker == ticker)
            .map(|r| (r.report_date.clone(), r.status.clone(), r.proof_url.clone()))
            .collect()
    }

    /// A poll, then the `earnings:6k` messages it queued, one by one.
    async fn run(
        now: DateTime<Utc>,
        es: &MemEarningsStore,
        fl: &Fake,
    ) -> (ForeignOutcome, Vec<ReadOutcome>) {
        let o = foreign_tick(now, es, fl).await.unwrap();
        let mut reads = Vec::new();
        for _ in 0..o.reads {
            reads.push(foreign_read(now, es, es.base.as_ref(), fl).await.unwrap());
        }
        (o, reads)
    }
    fn reported(reads: &[ReadOutcome]) -> Vec<String> {
        reads.iter().filter_map(|r| r.reported.clone()).collect()
    }

    #[tokio::test]
    async fn a_results_6k_near_the_date_marks_it_reported_with_the_6k_as_proof() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        // NIO's results and, the same morning, its delivery update.
        let res = fl.add(
            "NIO",
            1,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        fl.add(
            "NIO",
            2,
            "2026-09-01T10:03:00Z",
            "6-K",
            Some(fx("nio_august_deliveries.htm")),
        );
        let now = utc("2026-09-01T11:20:00Z");
        es.calls.set(0);
        let o = foreign_tick(now, &es, &fl).await.unwrap();
        // The poll reads the feed page only; each 6-K gets its own message.
        assert_eq!(
            (o.pages, o.found, o.reads, fl.fetches.get()),
            (1, 2, 2, 1),
            "{o:?}"
        );
        // kv: watermark, checked and pending reads; one calendar read; the
        // pending and watermark writes.
        assert_eq!(es.calls.get(), 6);
        es.calls.set(0);
        let first = foreign_read(now, &es, es.base.as_ref(), &fl).await.unwrap();
        // The older one first: the delivery update. Backoff, pending and
        // checked reads; the ticker's rows; the two list writes.
        assert!(
            first.rejected.is_some() && first.reported.is_none(),
            "{first:?}"
        );
        assert_eq!((es.calls.get(), fl.fetches.get()), (6, 3));
        es.calls.set(0);
        let second = foreign_read(now, &es, es.base.as_ref(), &fl).await.unwrap();
        assert_eq!(second.reported.as_deref(), Some("2026-09-01"), "{second:?}");
        // Plus the calendar write.
        assert_eq!((es.calls.get(), fl.fetches.get()), (7, 5));
        assert_eq!(
            status(&es, "NIO"),
            [(
                "2026-09-01".into(),
                "reported".into(),
                Some(filing_url(1736541, &res))
            )]
        );
        // A redelivered message finds nothing pending.
        assert_eq!(
            foreign_read(now, &es, es.base.as_ref(), &fl).await.unwrap(),
            ReadOutcome::default()
        );
        // The next firing sees the same feed: nothing is queued again.
        let (f0, c0) = (fl.fetches.get(), es.calls.get());
        let o = foreign_tick(now + Duration::minutes(5), &es, &fl)
            .await
            .unwrap();
        assert_eq!((o.found, o.reads, fl.fetches.get() - f0), (0, 0, 1));
        assert_eq!(es.calls.get() - c0, 3, "only the three kv reads");
    }

    #[tokio::test]
    async fn only_watched_foreign_6ks_near_a_date_are_read() {
        let es = stores();
        seed(&es, "BABA", "2026-08-20").await;
        seed(&es, "TM", "2026-08-04").await;
        let fl = Fake::default();
        // Nine days after Toyota's date: outside the window.
        fl.add(
            "TM",
            1,
            "2026-08-13T10:00:00Z",
            "6-K",
            Some(fx("tm_q1_summary.htm")),
        );
        // An amendment, and a domestic filer's 6-K-shaped entry.
        fl.add(
            "BABA",
            2,
            "2026-08-20T20:05:00Z",
            "6-K/A",
            Some(fx("baba_results.htm")),
        );
        fl.add(
            "AAPL",
            3,
            "2026-08-20T20:05:00Z",
            "6-K",
            Some(fx("baba_results.htm")),
        );
        // Released in Hong Kong's evening, the US afternoon before the date.
        let real = fl.add(
            "BABA",
            4,
            "2026-08-19T20:05:00Z",
            "6-K",
            Some(fx("baba_results.htm")),
        );
        let (o, reads) = run(utc("2026-08-20T21:00:00Z"), &es, &fl).await;
        assert_eq!(o.found, 1, "{o:?}");
        // The June-quarter release dated "August 20, 2026".
        assert_eq!(reported(&reads), ["2026-08-20"], "{reads:?}");
        assert_eq!(status(&es, "BABA")[0].2, Some(filing_url(1577552, &real)));
        assert_eq!(status(&es, "TM")[0].1, "unconfirmed");
    }

    #[tokio::test]
    async fn a_poll_queues_at_most_three_reads_and_the_rest_next_time() {
        let es = stores();
        let tickers = ["NIO", "XPEV", "LI", "BIDU"];
        for t in tickers {
            seed(&es, t, "2026-09-01").await;
        }
        let fl = Fake::default();
        for (i, t) in tickers.iter().enumerate() {
            let at = format!("2026-09-01T1{i}:00:00Z");
            fl.add(t, i, &at, "6-K", Some(fx("nio_august_deliveries.htm")));
        }
        let now = utc("2026-09-01T15:00:00Z");
        let (o, reads) = run(now, &es, &fl).await;
        assert_eq!(
            (o.found, o.pending, o.reads, reads.len()),
            (4, 4, MAX_READS, MAX_READS)
        );
        let (o, reads) = run(now + Duration::minutes(5), &es, &fl).await;
        assert_eq!(
            (o.found, o.reads, reads.len()),
            (0, 1, 1),
            "the fourth, next firing"
        );
        let (o, _) = run(now + Duration::minutes(10), &es, &fl).await;
        assert_eq!((o.pending, o.reads), (0, 0));
    }

    #[tokio::test]
    async fn a_date_already_reported_is_not_read_again() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        fl.add(
            "NIO",
            1,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        run(utc("2026-09-01T11:20:00Z"), &es, &fl).await;
        // A second 6-K the next day (the GAAP/IFRS reconciliation, say).
        fl.add(
            "NIO",
            2,
            "2026-09-02T11:00:00Z",
            "6-K",
            Some(fx("nio_gaap_ifrs.htm")),
        );
        let before = fl.fetches.get();
        let (o, _) = run(utc("2026-09-02T11:05:00Z"), &es, &fl).await;
        assert_eq!((o.found, fl.fetches.get() - before), (0, 1));
    }

    #[tokio::test]
    async fn a_network_error_is_retried_by_a_later_message() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        // No document text yet: the fake answers 503.
        fl.add("NIO", 1, "2026-09-01T11:15:00Z", "6-K", None);
        let now = utc("2026-09-01T11:20:00Z");
        let (_, reads) = run(now, &es, &fl).await;
        assert_eq!(reads[0].errors.len(), 1, "{reads:?}");
        let acc = fl.feed.borrow()[0].1.clone();
        fl.docs.borrow_mut().insert(acc, fx("nio_results.htm"));
        let (o, reads) = run(now + Duration::minutes(5), &es, &fl).await;
        assert_eq!(o.reads, 1);
        assert_eq!(reported(&reads), ["2026-09-01"]);
    }

    #[tokio::test]
    async fn a_missing_document_is_retried_once_then_checked() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        let acc = fl.add(
            "NIO",
            1,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        // EDGAR lists the filing a moment before it serves the document.
        fl.head_errors
            .borrow_mut()
            .insert(acc.clone(), vec![ProviderError::Http(404)]);
        let now = utc("2026-09-01T11:20:00Z");
        let (_, reads) = run(now, &es, &fl).await;
        assert!(reads[0].errors[0].contains("404"), "{reads:?}");
        let (_, reads) = run(now + Duration::minutes(5), &es, &fl).await;
        assert_eq!(reported(&reads), ["2026-09-01"], "the second try reads it");
        // Two 404s: checked, never read again.
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        fl.head_errors.borrow_mut().insert(
            acc.clone(),
            vec![ProviderError::Http(404), ProviderError::Http(410)],
        );
        run(now, &es, &fl).await;
        run(now + Duration::minutes(5), &es, &fl).await;
        let (o, _) = run(now + Duration::minutes(10), &es, &fl).await;
        assert_eq!((o.pending, o.reads), (0, 0));
        assert!(status(&es, "NIO")[0].1 == "unconfirmed");
    }

    #[tokio::test]
    async fn a_429_on_a_read_leaves_the_6k_pending() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        let acc = fl.add(
            "NIO",
            1,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        fl.head_errors
            .borrow_mut()
            .insert(acc, vec![ProviderError::Http(429)]);
        let now = utc("2026-09-01T11:20:00Z");
        let (_, reads) = run(now, &es, &fl).await;
        assert!(reads[0].throttled);
        let (o, reads) = run(now + Duration::minutes(16), &es, &fl).await;
        assert_eq!(o.reads, 1);
        assert_eq!(reported(&reads), ["2026-09-01"]);
    }

    #[tokio::test]
    async fn once_reported_the_tickers_other_pending_6ks_are_not_fetched() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        // The results first, then a reconciliation filed the same morning.
        fl.add(
            "NIO",
            1,
            "2026-09-01T11:00:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        fl.add(
            "NIO",
            2,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_gaap_ifrs.htm")),
        );
        let (o, reads) = run(utc("2026-09-01T11:20:00Z"), &es, &fl).await;
        assert_eq!(o.reads, 2);
        assert_eq!(reported(&reads), ["2026-09-01"]);
        // The feed page and the first 6-K's 2 fetches; the second is dropped unread.
        assert_eq!(fl.fetches.get(), 3);
        assert_eq!(reads[1], ReadOutcome::default());
        let (o, _) = run(utc("2026-09-01T11:25:00Z"), &es, &fl).await;
        assert_eq!((o.pending, o.reads), (0, 0));
    }

    #[tokio::test]
    async fn a_429_alerts_once_and_holds_the_other_reads_back() {
        let es = stores();
        for t in ["NIO", "XPEV"] {
            seed(&es, t, "2026-09-01").await;
        }
        let fl = Fake::default();
        let acc = fl.add(
            "NIO",
            1,
            "2026-09-01T11:00:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        fl.add(
            "XPEV",
            2,
            "2026-09-01T11:05:00Z",
            "6-K",
            Some(fx("nio_august_deliveries.htm")),
        );
        fl.head_errors
            .borrow_mut()
            .insert(acc, vec![ProviderError::Http(429)]);
        let now = utc("2026-09-01T11:20:00Z");
        let (o, reads) = run(now, &es, &fl).await;
        assert_eq!(o.reads, 2);
        assert!(reads[0].throttled, "{reads:?}");
        // The second message of that poll fetches nothing.
        assert!(reads[1].held, "{reads:?}");
        assert_eq!(fl.fetches.get(), 1 + 2);
        let throttled = || {
            es.base
                .alerts
                .borrow()
                .iter()
                .filter(|a| a.0.starts_with("sec-throttled"))
                .count()
        };
        assert_eq!(throttled(), 1);
        // Still held five minutes later; read again after the backoff.
        let (_, reads) = run(now + Duration::minutes(5), &es, &fl).await;
        assert!(reads.iter().all(|r| r.held), "{reads:?}");
        let (_, reads) = run(now + Duration::minutes(16), &es, &fl).await;
        assert_eq!(reported(&reads), ["2026-09-01"], "{reads:?}");
        assert_eq!(throttled(), 1);
    }

    #[tokio::test]
    async fn a_6k_with_no_readable_document_is_read_once() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        let acc = fl.add("NIO", 1, "2026-09-01T11:15:00Z", "6-K", None);
        fl.bare.borrow_mut().push(acc);
        let now = utc("2026-09-01T11:20:00Z");
        let (_, reads) = run(now, &es, &fl).await;
        assert_eq!(reads[0].errors.len(), 1, "{reads:?}");
        let before = fl.fetches.get();
        let (o, _) = run(now + Duration::minutes(5), &es, &fl).await;
        assert_eq!(
            (o.reads, fl.fetches.get() - before),
            (0, 1),
            "the feed page only"
        );
    }

    /// An empty reply now carries what came back (`EmptyReply`), and is
    /// still "no readable document": the 6-K is checked, not retried.
    #[tokio::test]
    async fn a_6k_with_an_empty_reply_is_read_once() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        let acc = fl.add(
            "NIO",
            1,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        fl.head_errors.borrow_mut().insert(
            acc,
            vec![ProviderError::EmptyReply(
                "status 200, content-type \"text/html\", 0 bytes".into(),
            )],
        );
        let now = utc("2026-09-01T11:20:00Z");
        let (_, reads) = run(now, &es, &fl).await;
        assert_eq!(reads.len(), 1);
        assert!(
            reads[0].errors[0]
                .ends_with("empty result (status 200, content-type \"text/html\", 0 bytes)"),
            "{reads:?}"
        );
        let before = fl.fetches.get();
        let (o, _) = run(now + Duration::minutes(5), &es, &fl).await;
        assert_eq!(
            (o.pending, o.reads, fl.fetches.get() - before),
            (0, 0, 1),
            "the feed page only"
        );
    }

    #[tokio::test]
    async fn the_tick_goes_on_the_release_date_and_moves_yahoos() {
        // Yahoo said Aug 29; NIO released (and filed) on Sep 1.
        let es = stores();
        seed(&es, "NIO", "2026-08-29").await;
        let fl = Fake::default();
        let acc = fl.add(
            "NIO",
            1,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        let (_, reads) = run(utc("2026-09-01T11:20:00Z"), &es, &fl).await;
        assert_eq!(reported(&reads), ["2026-09-01"]);
        assert_eq!(
            status(&es, "NIO"),
            [(
                "2026-09-01".into(),
                "reported".into(),
                Some(filing_url(1736541, &acc))
            )]
        );
        // SAP: Yahoo's Oct 21 was right; the 6-K came on the 26th, inside
        // the widened window, and the dateline keeps the 21st.
        let es = stores();
        seed(&es, "SAP", "2026-10-21").await;
        let fl = Fake::default();
        fl.add(
            "SAP",
            1,
            "2026-10-26T10:00:00Z",
            "6-K",
            Some(fx("sap_results.htm")),
        );
        let (_, reads) = run(utc("2026-10-26T10:05:00Z"), &es, &fl).await;
        assert_eq!(reported(&reads), ["2026-10-21"], "{reads:?}");
        assert_eq!(status(&es, "SAP")[0].0, "2026-10-21");
    }

    #[tokio::test]
    async fn pages_back_to_the_watermark_and_warns_at_the_cap() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        es.kv_set(KV_6K_WATERMARK, "2026-08-31T00:00:00Z", "x")
            .await
            .unwrap();
        let fl = Fake::default();
        // A page of other 6-Ks above NIO's, which is on the second page.
        fl.filler_pages.set(1);
        fl.add(
            "NIO",
            1,
            "2026-09-01T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        let (o, reads) = run(utc("2026-09-02T00:00:00Z"), &es, &fl).await;
        assert_eq!(
            (o.pages, reported(&reads)),
            (2, vec!["2026-09-01".to_string()]),
            "{o:?}"
        );
        assert!(o.errors.is_empty(), "{o:?}");
        // Three full pages still newer than the watermark: the gap is logged.
        let es = stores();
        es.kv_set(KV_6K_WATERMARK, "2026-08-31T00:00:00Z", "x")
            .await
            .unwrap();
        let fl = Fake::default();
        fl.filler_pages.set(MAX_PAGES);
        let o = foreign_tick(utc("2026-09-02T00:00:00Z"), &es, &fl)
            .await
            .unwrap();
        assert_eq!(o.pages, MAX_PAGES);
        assert!(o.errors[0].contains("may be missed"), "{o:?}");
    }

    #[tokio::test]
    async fn a_429_stops_the_6k_part_of_the_poll() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        es.kv_set(KV_6K_WATERMARK, "2026-08-31T00:00:00Z", "x")
            .await
            .unwrap();
        let fl = Fake::default();
        fl.filler_pages.set(1);
        fl.throttle_page.set(Some(1));
        fl.add(
            "NIO",
            1,
            "2026-08-31T11:15:00Z",
            "6-K",
            Some(fx("nio_results.htm")),
        );
        let o = foreign_tick(utc("2026-09-01T12:00:00Z"), &es, &fl)
            .await
            .unwrap();
        assert!(o.throttled);
        assert_eq!(
            (o.found, o.reads, fl.fetches.get()),
            (0, 0, 2),
            "two feed pages"
        );
        // The watermark stays, so the next firing reads the range again.
        assert_eq!(
            es.kv_get(KV_6K_WATERMARK).await.unwrap().as_deref(),
            Some("2026-08-31T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn a_pending_6k_past_the_window_is_dropped() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        fl.add("NIO", 1, "2026-09-01T11:15:00Z", "6-K", None);
        let o = foreign_tick(utc("2026-09-01T11:20:00Z"), &es, &fl)
            .await
            .unwrap();
        assert_eq!(o.pending, 1);
        let o = foreign_tick(utc("2026-09-12T11:20:00Z"), &es, &fl)
            .await
            .unwrap();
        assert_eq!((o.pending, o.reads), (0, 0));
    }

    #[test]
    fn a_foreign_filer_under_another_name_is_a_mismatch() {
        let tm = find("TM").unwrap();
        let entries = vec![FeedEntry {
            form: "6-K".into(),
            cik: tm.cik,
            company: "TOYOTA INDUSTRIES CORP".into(),
            accession: "0001094517-26-000001".into(),
            filing_date: "2026-09-01".into(),
            items: vec![],
            accepted_at: utc("2026-09-01T11:15:00Z"),
            index_url: filing_url(tm.cik, "0001094517-26-000001"),
        }];
        assert_eq!(name_mismatches(&entries, &foreign_by_cik())[0].0, "TM");
    }
}

#[cfg(test)]
mod timing {
    /// CPU per 6-K read (release build), over raw 6-K documents in
    /// `$SIXK_RAW`: `cargo test -p prebell-core --release classify_timing --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn classify_timing() {
        let t0 = std::time::Instant::now();
        crate::edgar::html_to_text("<p>x</p>");
        let _ = super::classify(
            "x",
            &crate::earnings::find("NIO").unwrap(),
            "2026-11-05".parse().unwrap(),
        );
        println!("regex compile {:?}", t0.elapsed());
        let dir = std::env::var("SIXK_RAW").unwrap_or_default();
        for name in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = name.path();
            if p.to_string_lossy().ends_with("-index.htm") {
                continue;
            }
            let raw = std::fs::read(&p).unwrap();
            for max in [32 * 1024, 64 * 1024] {
                let html = String::from_utf8_lossy(&raw[..raw.len().min(max)]).into_owned();
                let t = std::time::Instant::now();
                let text = crate::edgar::head_text(&html);
                let _ = super::classify(
                    &text,
                    &crate::earnings::find("NIO").unwrap(),
                    "2026-11-05".parse().unwrap(),
                );
                println!(
                    "{:>6} {:?} {}",
                    max,
                    t.elapsed(),
                    p.file_name().unwrap().to_string_lossy()
                );
            }
        }
    }
}

#[cfg(test)]
mod corpus {
    /// Runs every `<TICKER>_<ACCESSION>/` directory under `$SIXK_CORPUS`
    /// (an `index.htm` and its documents, with `list.tsv` giving the ET
    /// acceptance date) through the pipeline and prints each verdict.
    #[test]
    #[ignore]
    fn corpus_verdicts() {
        let root = std::env::var("SIXK_CORPUS").unwrap();
        let list = std::fs::read_to_string(format!("{root}/list.tsv")).unwrap();
        for line in list.lines() {
            let c: Vec<&str> = line.split('\t').collect();
            let (ticker, acc, day) = (c[0], c[1], c[2]);
            let dir = format!("{root}/docs/{ticker}_{acc}");
            let index = std::fs::read_to_string(format!("{dir}/index.htm")).unwrap();
            let docs = crate::edgar::parse_filing_index(&index);
            let Some(doc) = super::results_doc(&docs) else {
                println!("{ticker}\t{acc}\tNO-DOC");
                continue;
            };
            let file = doc.url.rsplit('/').next().unwrap();
            let raw = std::fs::read(format!("{dir}/{file}")).unwrap_or_default();
            let html =
                String::from_utf8_lossy(&raw[..raw.len().min(super::HEAD_BYTES)]).into_owned();
            let text = crate::edgar::head_text(&html);
            let e = crate::earnings::find(ticker).unwrap();
            let r = super::classify(&text, &e, day.parse().unwrap());
            let snip: String = text
                .chars()
                .take(160)
                .collect::<String>()
                .replace('\n', " ");
            println!("{ticker}\t{acc}\t{day}\t{}\t{r:?}\t{snip}", doc.doc_type);
        }
    }
}
