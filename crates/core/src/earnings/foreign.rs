//! Reports of foreign private issuers (20-F/6-K filers: Toyota, Alibaba,
//! Novo Nordisk, ...). A 6-K has no Item 2.02, so EDGAR's current 6-K feed is
//! read like the 8-K feed and each 6-K that a watched foreign company files
//! near its calendar date is read: when its press release (or financial
//! summary) states results for the quarter just ended, the calendar date is
//! marked `reported` with the 6-K as its proof. Anything else, or any doubt,
//! leaves the date as it was: a missing tick is better than a wrong one.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use regex::Regex;

use crate::anchor::{has_word, normalize};
use crate::clock::et;
use crate::earnings::calendar::{apply_to, report_changes, SAME_EVENT_DAYS};
use crate::earnings::detect::same_filer;
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
pub const MAX_CANDIDATES: usize = 3;
/// Bytes of a document read: the release's opening, or a financial
/// summary's cover (Novo Nordisk's statement is 17 KB in). About 0.45 ms of
/// CPU per document natively (`timing` below), so three stay well inside
/// the Free plan's 10 ms.
pub const HEAD_BYTES: usize = 32 * 1024;
/// A 6-K accepted from this many days before the calendar date (an Asian
/// morning release is the previous US evening) ...
pub const DAYS_BEFORE: i64 = 1;
/// ... to this many after it (a 6-K filed the next day) is a candidate.
pub const DAYS_AFTER: i64 = 2;
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

/// The last calendar-quarter end before `day`: the period a release made
/// on `day` reports (Toyota's and Alibaba's fiscal quarters end on calendar
/// quarter ends too).
pub fn quarter_end_before(day: NaiveDate) -> NaiveDate {
    let (y, m) = (day.year(), day.month());
    let end = |y: i32, m: u32, d: u32| NaiveDate::from_ymd_opt(y, m, d).expect("quarter end");
    match m {
        1..=3 => end(y - 1, 12, 31),
        4..=6 => end(y, 3, 31),
        7..=9 => end(y, 6, 30),
        _ => end(y, 9, 30),
    }
}

/// The date in a period-end match: "September 30, 2026", "Sept. 30, 2026"
/// or "30 September 2026".
fn period_date(month: &str, day: &str, year: &str) -> Option<NaiveDate> {
    let m = month.trim_end_matches('.').to_ascii_lowercase();
    let m = MONTHS.iter().position(|full| {
        let full = full.to_ascii_lowercase();
        m == full || (m.len() >= 3 && full.starts_with(&m))
    })? as u32;
    NaiveDate::from_ymd_opt(year.parse().ok()?, m + 1, day.parse().ok()?)
}

/// Lowercase phrases that, before the results statement, mark a filing that
/// only talks about results: a date notice, a pre-announcement, a delivery
/// or monthly report, a meeting, a buyback, a GAAP reconciliation, insider
/// dealing. Plain substring checks: the Free plan gives an invocation 10 ms
/// of CPU.
const NOT_A_RELEASE: [(&str, &str); 26] = [
    ("date of board meeting", "board meeting notice"),
    ("date of the board meeting", "board meeting notice"),
    ("announces date", "pre-announcement"),
    ("announces the date", "pre-announcement"),
    ("will announce", "pre-announcement"),
    ("will report", "pre-announcement"),
    ("will release", "pre-announcement"),
    ("will publish", "pre-announcement"),
    ("will hold", "pre-announcement"),
    ("to announce", "pre-announcement"),
    ("to report", "pre-announcement"),
    ("to release", "pre-announcement"),
    ("to publish", "pre-announcement"),
    ("delivery update", "delivery update"),
    ("vehicles were delivered", "delivery update"),
    ("monthly return", "exchange return"),
    ("next day disclosure return", "exchange return"),
    ("annual general meeting", "general meeting"),
    ("extraordinary general meeting", "general meeting"),
    ("voting results", "general meeting"),
    ("poll results", "general meeting"),
    ("repurchase of shares", "buyback"),
    ("share repurchase", "buyback"),
    ("buyback", "buyback"),
    ("reconciliation between", "GAAP reconciliation"),
    ("made in", "insider dealing"),
];

/// Lowercase openers of a financial summary or report.
const SUMMARY: [&str; 7] = [
    "financial summary",
    "financial results",
    "financial report",
    "financial statement",
    "interim results",
    "annual results",
    "quarterly results",
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

/// The release states results for the period ended `quarter`: "announced its
/// unaudited financial results for the second quarter ended June 30, 2026",
/// "earnings per share ... for the third quarter ended September 30, 2026",
/// "Financial report for the period 1 January 2026 to 30 September 2026",
/// "FINANCIAL SUMMARY ... (April 1, 2026 through June 30, 2026)". `lower` is
/// the ASCII-lowercased text. Returns where the first such statement starts.
fn results_statement(lower: &str, quarter: NaiveDate) -> Option<usize> {
    static END: OnceLock<Regex> = OnceLock::new();
    let end = END.get_or_init(|| {
        Regex::new(
            r"\b(?:ended|ending|through|to)\s+(?:([a-z]{3,9}\.?)\s+(\d{1,2}),?\s+(\d{4})|(\d{1,2})\s+([a-z]{3,9}\.?)\s+(\d{4}))",
        )
        .expect("valid built-in regex")
    });
    for c in end.captures_iter(lower) {
        let date = match (c.get(1), c.get(4)) {
            (Some(m), _) => period_date(m.as_str(), &c[2], &c[3]),
            (_, Some(d)) => period_date(&c[5], d.as_str(), &c[6]),
            _ => None,
        };
        if date != Some(quarter) {
            continue;
        }
        let at = c.get(0)?.start();
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
            return Some(start);
        }
    }
    None
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

/// Whether `text` (the opening of a 6-K document) is `e`'s release of
/// results for the quarter before `accepted` (its ET acceptance date).
/// `Err` says why not.
pub fn classify(text: &str, e: &WatchEntry, accepted: NaiveDate) -> Result<(), String> {
    let text = normalize(text);
    let lower = text.to_ascii_lowercase();
    let quarter = quarter_end_before(accepted);
    let Some(at) = results_statement(&lower, quarter) else {
        return Err(format!("no results for the period ended {quarter}"));
    };
    if let Some((_, why)) = NOT_A_RELEASE.iter().find(|(p, _)| lower[..at].contains(p)) {
        return Err((*why).to_string());
    }
    if !names_company(&text, &lower, e) {
        return Err(format!("{} not named", e.name));
    }
    Ok(())
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
/// accepted near their calendar date. A results release marks that date
/// `reported` with the 6-K's index as proof. D1: two kv reads, one ranged
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
        if !more || page + 1 == MAX_PAGES {
            complete = true;
            break;
        }
    }

    let watch = foreign_by_cik();
    let mut checked: Vec<String> = es
        .kv_get(KV_6K_CHECKED)
        .await?
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
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
                Ok(()) => {
                    let ch =
                        report_changes(rows, w.ticker, d, filing_url(f.cik, &f.accession), now);
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
    if out.read > 0 {
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
        classify(&fx(name), &find(ticker).unwrap(), d(accepted))
    }

    #[test]
    fn quarter_ends() {
        assert_eq!(quarter_end_before(d("2026-11-05")), d("2026-09-30"));
        assert_eq!(quarter_end_before(d("2026-10-01")), d("2026-09-30"));
        assert_eq!(quarter_end_before(d("2026-09-30")), d("2026-06-30"));
        assert_eq!(quarter_end_before(d("2027-02-20")), d("2026-12-31"));
        assert_eq!(quarter_end_before(d("2026-05-12")), d("2026-03-31"));
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
        ] {
            assert_eq!(check(ticker, name, accepted), Ok(()), "{name}");
        }
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
            .starts_with("no results for the period ended 2026-09-30"));
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
        ] {
            assert_eq!(
                classify(text, &e, d("2026-11-04")),
                Err("pre-announcement".into()),
                "{text}"
            );
        }
        let release = "Baidu Announces Third Quarter 2026 Results. BEIJING, Nov. 18, 2026 -- Baidu, Inc. (NASDAQ: BIDU) today announced its unaudited financial results for the third quarter ended September 30, 2026. Baidu will hold an earnings conference call.";
        assert_eq!(classify(release, &e, d("2026-11-18")), Ok(()));
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
            if start > 0 {
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
                .ok_or(ProviderError::Http(404))?;
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
        // Four days after Toyota's date: outside the window.
        fl.add(
            "TM",
            1,
            "2026-08-08T10:00:00Z",
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
    async fn at_most_three_6ks_are_read_per_poll_and_the_rest_next_time() {
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
        assert_eq!(o.read, 1, "the fourth one, next firing");
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
        // No document text: the fake answers 404.
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
