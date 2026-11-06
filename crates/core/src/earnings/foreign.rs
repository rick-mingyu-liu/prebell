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
use regex::Regex;

use crate::anchor::{has_word, normalize};
use crate::clock::et;
use crate::earnings::calendar::{apply_to, report_changes, SAME_EVENT_DAYS};
use crate::earnings::detect::{name_mismatches, same_filer};
use crate::earnings::store::{CalendarRow, EarningsStore};
use crate::earnings::{watchlist, Filer, WatchEntry};
use crate::edgar::{exhibit_991, filing_url, FeedEntry, FilingDoc, Filings, FEED_PAGE};
use crate::providers::ProviderError;

/// kv key: the newest 6-K feed acceptance time a poll read (RFC 3339).
pub const KV_6K_WATERMARK: &str = "edgar_6k_watermark";
/// kv key: the 6-K accessions already read (a JSON list, newest last), so a
/// 6-K the feed shows for hours is fetched once.
pub const KV_6K_CHECKED: &str = "edgar_6k_checked";
/// Accessions kept in `KV_6K_CHECKED` (about two weeks of candidates).
pub const CHECKED_KEEP: usize = 300;
/// 6-Ks read per poll at most (2 fetches each); the rest wait for the next
/// firing, still on the feed's first page (100 6-Ks span about 20 hours).
pub const MAX_CANDIDATES: usize = 2;
/// Bytes of a document read: the release's opening, or a financial
/// summary's cover (Novo Nordisk's statement is 17 KB in). About 0.45 ms of
/// CPU per document natively (`timing` below), so two stay well inside the
/// Free plan's 10 ms.
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
const NOT_A_RELEASE: [(&str, &str); 39] = [
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

fn regex(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
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
    static END: OnceLock<Regex> = OnceLock::new();
    static UNDATED: OnceLock<Regex> = OnceLock::new();
    let end = END.get_or_init(|| {
        regex(r"\b(?:ended|ending|through|to)\s+(?:([a-z]{3,9}\.?)\s+(\d{1,2}),?\s+(\d{4})|(\d{1,2})\s+([a-z]{3,9}\.?)\s+(\d{4}))")
    });
    let undated = UNDATED.get_or_init(|| {
        regex(concat!(
            r"\bq([1-4])\s+(20\d\d)\b",
            r"|\b(first|second|third|fourth)\s+quarter(?:\s+and\s+(?:first\s+half|nine\s+months|full\s+year))?(?:\s+of)?\s+(20\d\d)\b",
            r"|\bjanuary\s*(?:-|–|—|to|through)\s*(march|june|september|december)\s+(20\d\d)\b",
            r"|\bfirst\s+(half|nine\s+months)\s+of\s+(20\d\d)\b",
        ))
    });
    let mut out = Vec::new();
    for c in end.captures_iter(lower) {
        let period = match (c.get(1), c.get(4)) {
            (Some(m), _) => period_date(m.as_str(), &c[2], &c[3]),
            (_, Some(d)) => period_date(&c[5], d.as_str(), &c[6]),
            _ => None,
        };
        let (Some(period), Some(m)) = (period, c.get(0)) else {
            continue;
        };
        let at = m.start();
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
                end: m.end(),
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
/// "€11.7 billion", "DKK 31,720"), an amount with a scale word ("5.70bn",
/// "125.8m"), or a table figure in the millions ("24,630,753").
fn has_figure(s: &str) -> bool {
    static FIGURE: OnceLock<Regex> = OnceLock::new();
    FIGURE
        .get_or_init(|| {
            regex(r"(?:rmb|us\$|hk\$|nt\$|s\$|\$|€|£|¥|dkk|eur|usd|jpy)\s?\d|\d[\d,.]*\s?(?:billion|million|bn|mn|bln)\b|\b\d{1,3}(?:,\d{3}){2,}\b")
        })
        .is_match(s)
}

/// The release's dateline: the first date in its opening, before or just
/// after the statement, from a week before the 6-K to the day after it
/// ("BEIJING, China, August 18, 2026 –", "Madrid, 28 October 2026").
fn dateline(lower: &str, upto: usize, accepted: NaiveDate) -> Option<NaiveDate> {
    static DATE: OnceLock<Regex> = OnceLock::new();
    let date = DATE.get_or_init(|| {
        regex(r"\b(?:([a-z]{3,9}\.?)\s+(\d{1,2}),?\s+(\d{4})|(\d{1,2})\s+([a-z]{3,9}\.?)\s+(\d{4}))\b")
    });
    date.captures_iter(&lower[..ceil(lower, upto)])
        .filter_map(|c| match (c.get(1), c.get(4)) {
            (Some(m), _) => period_date(m.as_str(), &c[2], &c[3]),
            (_, Some(d)) => period_date(&c[5], d.as_str(), &c[6]),
            _ => None,
        })
        .find(|d| (accepted - Duration::days(7)..=accepted + Duration::days(1)).contains(d))
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
    Ok(dateline(&lower, st.end + 3000, accepted))
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForeignOutcome {
    /// 6-K feed pages read.
    pub pages: usize,
    /// 6-Ks read (index and document fetched).
    pub read: usize,
    /// Tickers whose calendar date was marked reported.
    pub reported: Vec<String>,
    /// `TICKER accession: reason` for 6-Ks read and not taken as results.
    pub rejected: Vec<String>,
    pub errors: Vec<String>,
    /// SEC answered 403/429 and the 6-K part stopped.
    pub throttled: bool,
    /// Foreign rows whose CIK files under another name, as `(ticker,
    /// message)`: `poll_job` alerts once a day, like the 8-K watchlist.
    pub mismatches: Vec<(&'static str, String)>,
}

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

/// Reads the current 6-K feed back to its watermark (as the 8-K poll does)
/// and checks up to `MAX_CANDIDATES` new 6-Ks of watched foreign companies
/// accepted near their calendar date. A results release marks its release
/// date `reported` (moving the calendar's date when it differs) with the
/// 6-K's index as proof. D1: two kv reads, one ranged
/// calendar read when a candidate is seen, one batched calendar write when
/// a date is marked, and the two kv writes. A store error aborts.
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
        let list = match filings.current_6k_filings(page * FEED_PAGE).await {
            Ok(l) => l,
            Err(e) => {
                out.errors
                    .push(format!("current 6-K filings page {page}: {e}"));
                out.throttled = matches!(e, ProviderError::Http(403 | 429));
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
    // Throttled: no more requests this firing (the watermark stays).
    if out.throttled {
        return Ok(out);
    }

    let watch = foreign_by_cik();
    out.mismatches = name_mismatches(&entries, &watch);
    let mut checked: Vec<String> = es
        .kv_get(KV_6K_CHECKED)
        .await?
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let checked_before = checked.len();
    let mut fresh: Vec<(&FeedEntry, &WatchEntry)> = Vec::new();
    for f in &entries {
        // An amendment repeats a release already counted (or corrects one).
        let Some(w) = watch.get(&f.cik).filter(|_| f.form == "6-K") else {
            continue;
        };
        if !same_filer(&f.company, w.sec)
            || checked.contains(&f.accession)
            || fresh.iter().any(|(g, _)| g.accession == f.accession)
        {
            continue;
        }
        fresh.push((f, w));
    }
    // Oldest first, so a burst is worked through in order.
    fresh.sort_by_key(|(f, _)| f.accepted_at);

    let day = |f: &FeedEntry| et(f.accepted_at).date_naive();
    let mut changes = Vec::new();
    let mut seen = 0;
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
            let rows = by_ticker.entry(w.ticker.to_string()).or_default();
            // Not near a calendar date: not a candidate, and not marked
            // checked (cheap to look at again).
            let Some(d) = target(rows, day(f)) else {
                continue;
            };
            if seen == MAX_CANDIDATES {
                break;
            }
            seen += 1;
            let text = match read(filings, f).await {
                Ok(t) => t,
                Err(e) => {
                    out.errors
                        .push(format!("{} {}: {e}", w.ticker, f.accession));
                    // A 6-K without a readable document reads the same next
                    // time: checked. Network errors and 5xx retry.
                    if matches!(
                        e,
                        ProviderError::Empty
                            | ProviderError::Parse(_)
                            | ProviderError::Http(404 | 410)
                    ) {
                        checked.push(f.accession.clone());
                    }
                    if matches!(e, ProviderError::Http(403 | 429)) {
                        out.throttled = true;
                        break;
                    }
                    continue;
                }
            };
            out.read += 1;
            checked.push(f.accession.clone());
            match classify(&text, w, day(f)) {
                Ok(dated) => {
                    // The release's own date (its dateline), else the 6-K's
                    // when that is later than the calendar's; an earlier 6-K
                    // is the Asian-morning case and keeps the calendar date.
                    let released = dated.unwrap_or_else(|| day(f).max(d));
                    let ch = report_changes(
                        rows,
                        w.ticker,
                        released,
                        filing_url(f.cik, &f.accession),
                        now,
                    );
                    apply_to(rows, &ch);
                    changes.extend(ch);
                    out.reported.push(w.ticker.to_string());
                }
                Err(why) => out
                    .rejected
                    .push(format!("{} {}: {why}", w.ticker, f.accession)),
            }
        }
    }
    if !changes.is_empty() {
        es.write_calendar(&changes).await?;
    }
    if checked.len() != checked_before {
        let keep = checked.len().saturating_sub(CHECKED_KEEP);
        let json = serde_json::to_string(&checked[keep..]).map_err(|e| e.to_string())?;
        es.kv_set(KV_6K_CHECKED, &json, &iso(now)).await?;
    }
    let newest = entries.iter().map(|f| f.accepted_at).max();
    if let (true, Some(n)) = (complete, newest) {
        if watermark.is_none_or(|w| n > w) {
            es.kv_set(KV_6K_WATERMARK, &iso(n), &iso(now)).await?;
        }
    }
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
    fn the_release_date_is_its_dateline() {
        let at = |ticker: &str, name: &str, accepted: &str| {
            classify(&fx(name), &find(ticker).unwrap(), d(accepted)).unwrap()
        };
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
        assert_eq!(o.reported, ["NIO"], "{o:?}");
        assert_eq!(o.rejected.len(), 1, "{o:?}");
        assert!(o.rejected[0].contains("no results"), "{o:?}");
        assert_eq!(
            status(&es, "NIO"),
            [(
                "2026-09-01".into(),
                "reported".into(),
                Some(filing_url(1736541, &res))
            )]
        );
        // The feed page, then 2 fetches for each of the 2 6-Ks.
        assert_eq!((o.pages, o.read, fl.fetches.get()), (1, 2, 5));
        // kv: watermark and checked reads; one calendar read; one write;
        // checked and watermark writes.
        assert_eq!(es.calls.get(), 6);
        // The next firing sees the same feed: nothing is read again.
        let (f0, c0) = (fl.fetches.get(), es.calls.get());
        let o = foreign_tick(now + Duration::minutes(5), &es, &fl)
            .await
            .unwrap();
        assert_eq!((o.read, fl.fetches.get() - f0), (0, 1));
        assert_eq!(es.calls.get() - c0, 2, "only the two kv reads");
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
        let o = foreign_tick(utc("2026-08-20T21:00:00Z"), &es, &fl)
            .await
            .unwrap();
        // The June-quarter release accepted on Aug 19 ET: the Aug 20 date.
        assert_eq!(
            (o.read, o.reported.clone()),
            (1, vec!["BABA".to_string()]),
            "{o:?}"
        );
        assert_eq!(status(&es, "BABA")[0].2, Some(filing_url(1577552, &real)));
        assert_eq!(status(&es, "TM")[0].1, "unconfirmed");
    }

    #[tokio::test]
    async fn at_most_two_6ks_are_read_per_poll_and_the_rest_next_time() {
        let es = stores();
        for t in ["NIO", "XPEV", "LI", "BIDU"] {
            seed(&es, t, "2026-09-01").await;
        }
        let fl = Fake::default();
        for (i, t) in ["NIO", "XPEV", "LI", "BIDU"].iter().enumerate() {
            let at = format!("2026-09-01T1{i}:00:00Z");
            fl.add(t, i, &at, "6-K", Some(fx("nio_august_deliveries.htm")));
        }
        let now = utc("2026-09-01T15:00:00Z");
        let o = foreign_tick(now, &es, &fl).await.unwrap();
        assert_eq!(o.read, MAX_CANDIDATES);
        // Every fetch of this poll: the page and 2 per 6-K.
        assert_eq!(fl.fetches.get(), 1 + 2 * MAX_CANDIDATES);
        let o = foreign_tick(now + Duration::minutes(5), &es, &fl)
            .await
            .unwrap();
        assert_eq!(o.read, 2, "the other two, next firing");
        let o = foreign_tick(now + Duration::minutes(10), &es, &fl)
            .await
            .unwrap();
        assert_eq!(o.read, 0);
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
        let now = utc("2026-09-01T11:20:00Z");
        foreign_tick(now, &es, &fl).await.unwrap();
        // A second 6-K the next day (the GAAP/IFRS reconciliation, say).
        fl.add(
            "NIO",
            2,
            "2026-09-02T11:00:00Z",
            "6-K",
            Some(fx("nio_gaap_ifrs.htm")),
        );
        let before = fl.fetches.get();
        let o = foreign_tick(utc("2026-09-02T11:05:00Z"), &es, &fl)
            .await
            .unwrap();
        assert_eq!((o.read, fl.fetches.get() - before), (0, 1));
    }

    #[tokio::test]
    async fn a_fetch_error_is_retried_next_firing() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        // No document text: the fake answers 503.
        fl.add("NIO", 1, "2026-09-01T11:15:00Z", "6-K", None);
        let now = utc("2026-09-01T11:20:00Z");
        let o = foreign_tick(now, &es, &fl).await.unwrap();
        assert_eq!((o.read, o.errors.len()), (0, 1), "{o:?}");
        fl.docs
            .borrow_mut()
            .insert(fl.feed.borrow()[0].1.clone(), fx("nio_results.htm"));
        let o = foreign_tick(now + Duration::minutes(5), &es, &fl)
            .await
            .unwrap();
        assert_eq!(o.reported, ["NIO"]);
    }
    #[tokio::test]
    async fn a_6k_with_no_readable_document_is_read_once() {
        let es = stores();
        seed(&es, "NIO", "2026-09-01").await;
        let fl = Fake::default();
        let acc = fl.add("NIO", 1, "2026-09-01T11:15:00Z", "6-K", None);
        fl.bare.borrow_mut().push(acc);
        let now = utc("2026-09-01T11:20:00Z");
        let o = foreign_tick(now, &es, &fl).await.unwrap();
        assert_eq!(o.errors.len(), 1, "{o:?}");
        let before = fl.fetches.get();
        foreign_tick(now + Duration::minutes(5), &es, &fl)
            .await
            .unwrap();
        assert_eq!(fl.fetches.get() - before, 1, "the feed page only");
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
        let o = foreign_tick(utc("2026-09-01T11:20:00Z"), &es, &fl)
            .await
            .unwrap();
        assert_eq!(o.reported, ["NIO"]);
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
        let o = foreign_tick(utc("2026-10-26T10:05:00Z"), &es, &fl)
            .await
            .unwrap();
        assert_eq!(o.reported, ["SAP"], "{o:?}");
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
        let o = foreign_tick(utc("2026-09-02T00:00:00Z"), &es, &fl)
            .await
            .unwrap();
        assert_eq!(
            (o.pages, o.reported.clone()),
            (2, vec!["NIO".to_string()]),
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
    async fn a_429_stops_the_6k_part_before_any_read() {
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
            (o.read, fl.fetches.get()),
            (0, 2),
            "two feed pages, no reads"
        );
        // The watermark stays, so the next firing reads the range again.
        assert_eq!(
            es.kv_get(KV_6K_WATERMARK).await.unwrap().as_deref(),
            Some("2026-08-31T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn a_foreign_filer_under_another_name_is_reported() {
        let es = stores();
        let fl = Fake::default();
        fl.add("TM", 1, "2026-09-01T11:15:00Z", "6-K", None);
        // The feed shows Toyota's CIK under another company's name.
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
        // Through the tick: the Fake names rows by their SEC name, so none.
        let o = foreign_tick(utc("2026-09-01T11:20:00Z"), &es, &fl)
            .await
            .unwrap();
        assert!(o.mismatches.is_empty());
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
