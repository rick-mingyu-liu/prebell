//! The status and the date line, by code, from what is stored: the forms
//! EDGAR lists, the covers read, the news items and the closes. Every job
//! that writes (`check`, `news`, `trading`) recomputes the rows from the
//! state it is about to write, so the rules live in one place.
//!
//! **The facts source** (`current`): covers read, oldest first; a filing
//! replaces the current one only when it yields everything it must: a final
//! prospectus its price and shares; a registration statement every kind of
//! `KEEP_KINDS` the current one has (so a range, the shares or the symbol is
//! never lost to a thinner amendment), and never a final prospectus. A
//! filing that falls short stays in the timeline and is alerted (`check`).
//! Once listed, filings after the first session are ignored: a follow-on
//! offering or a resale withdrawal never changes the IPO's facts or status.
//!
//! **The date rule** (`docs/decisions.md`, 2026-11-11), first that applies:
//! 1. **confirmed, trading**: the first stored Yahoo session, only when it is
//!    the day the filings point to (rule 2 or 3's date). A later session is
//!    never called the first day.
//! 2. **confirmed, filing**: the current final prospectus states the first
//!    trading day in its own words ("is expected to begin trading ... on
//!    November 5, 2026"; never "on or about").
//! 3. **expected, filing**: the trading day after the current final
//!    prospectus's pricing date ("Prospectus dated ..."); else, with an
//!    `8-A12B` on file, the trading day after the SEC's `EFFECT` notice.
//! 4. **reported, news** (not once listed): the date the newest
//!    `expected_date` item gives, unless that date is before yesterday (ET).
//! 5. Otherwise no date ("date not set"). A withdrawn IPO has no date.

use chrono::{DateTime, Duration, NaiveDate, Utc};

use super::cover::FINAL_KINDS;
use super::store::{FactRow, FilingRow, IpoState, ItemRow, TradingRow, WatchRow};
use super::{is_draft, is_final, is_registration, iso, next_trading_day, ymd, Tracked};
use crate::clock::et;
use crate::premarket::yahoo_symbol;

/// Kinds a registration statement must keep from the cover it replaces.
pub const KEEP_KINDS: [&str; 5] = [
    "shares_offered",
    "price_low",
    "price_high",
    "exchange",
    "symbol",
];

/// The date line's source.
#[derive(Debug, Clone, PartialEq)]
pub struct DateChoice {
    pub date: String,
    /// reported | expected | confirmed
    pub status: &'static str,
    /// news | filing | trading
    pub source: &'static str,
    pub name: String,
    pub url: Option<String>,
    pub quote: Option<String>,
    pub as_of: String,
}

fn order(f: &FilingRow) -> (&str, &str) {
    (f.filing_date.as_str(), f.accession.as_str())
}

fn newest<'a>(fs: impl Iterator<Item = &'a FilingRow>) -> Option<&'a FilingRow> {
    fs.max_by(|a, b| order(a).cmp(&order(b)))
}

fn fact<'a>(st: &'a IpoState, accession: &str, kind: &str) -> Option<&'a FactRow> {
    st.facts
        .iter()
        .find(|f| f.accession == accession && f.kind == kind)
}

fn kinds_of<'a>(st: &'a IpoState, accession: &str) -> Vec<&'a str> {
    st.facts
        .iter()
        .filter(|f| f.accession == accession)
        .map(|f| f.kind.as_str())
        .collect()
}

/// The first stored session of `slug`.
pub fn first_session(st: &IpoState, slug: &str) -> Option<NaiveDate> {
    st.trading_of(slug)
        .iter()
        .filter_map(|t| t.session_date.parse::<NaiveDate>().ok())
        .min()
}

/// The filings that count: once listed, only those filed on or before the
/// first session.
pub fn considered<'a>(filings: &[&'a FilingRow], first: Option<NaiveDate>) -> Vec<&'a FilingRow> {
    filings
        .iter()
        .copied()
        .filter(|f| {
            first.is_none_or(|d| f.filing_date.parse::<NaiveDate>().is_ok_and(|fd| fd <= d))
        })
        .collect()
}

/// What `f`'s cover lacks to replace `cur` as the facts source (empty when
/// it may). A registration statement never replaces a final prospectus.
pub fn missing_to_replace(
    st: &IpoState,
    f: &FilingRow,
    cur: Option<&FilingRow>,
) -> Vec<&'static str> {
    let have = kinds_of(st, &f.accession);
    if is_final(&f.form) {
        return FINAL_KINDS
            .iter()
            .copied()
            .filter(|k| !have.contains(k))
            .collect();
    }
    if !is_registration(&f.form) {
        return vec!["an offering form"];
    }
    if have.is_empty() {
        return vec!["any fact"];
    }
    match cur {
        Some(c) if is_final(&c.form) => vec!["a final prospectus is already the source"],
        Some(c) => {
            let before = kinds_of(st, &c.accession);
            KEEP_KINDS
                .iter()
                .copied()
                .filter(|k| before.contains(k) && !have.contains(k))
                .collect()
        }
        None => vec![],
    }
}

/// The facts source among `filings` (module docs).
pub fn current<'a>(st: &IpoState, filings: &[&'a FilingRow]) -> Option<&'a FilingRow> {
    let mut read: Vec<&FilingRow> = filings
        .iter()
        .copied()
        .filter(|f| f.read_state == "read" && (is_registration(&f.form) || is_final(&f.form)))
        .collect();
    read.sort_by(|a, b| order(a).cmp(&order(b)));
    let mut cur: Option<&FilingRow> = None;
    for f in read {
        if missing_to_replace(st, f, cur).is_empty() {
            cur = Some(f);
        }
    }
    cur
}

/// The status from the forms on file and the closes (contract, "Status").
pub fn status(
    filings: &[&FilingRow],
    cur: Option<&FilingRow>,
    st: &IpoState,
    listed: bool,
) -> Option<&'static str> {
    let offering = newest(
        filings
            .iter()
            .copied()
            .filter(|f| is_registration(&f.form) || is_final(&f.form) || is_draft(&f.form)),
    );
    if listed {
        return Some("listed");
    }
    let rw = newest(filings.iter().copied().filter(|f| f.form == "RW"));
    if let Some(rw) = rw {
        if offering.is_none_or(|o| order(rw) >= order(o)) {
            return Some("withdrawn");
        }
    }
    offering?;
    Some(match cur {
        Some(c) if is_final(&c.form) => "priced",
        Some(c)
            if fact(st, &c.accession, "price_low").is_some()
                && fact(st, &c.accession, "price_high").is_some() =>
        {
            "range_set"
        }
        _ => "filed",
    })
}

pub fn yahoo_url(symbol: &str) -> String {
    format!("https://finance.yahoo.com/quote/{}", yahoo_symbol(symbol))
}

/// The first trading day the filings point to (rules 2 and 3).
pub fn filing_date(
    filings: &[&FilingRow],
    cur: Option<&FilingRow>,
    st: &IpoState,
) -> Option<DateChoice> {
    let from_fact = |f: &FilingRow, x: &FactRow, date: String, status: &'static str| DateChoice {
        date,
        status,
        source: "filing",
        name: f.form.clone(),
        url: Some(f.index_url.clone()),
        quote: Some(x.quote.clone()),
        as_of: f.filing_date.clone(),
    };
    if let Some(c) = cur.filter(|c| is_final(&c.form)) {
        if let Some(x) = fact(st, &c.accession, "trading_date") {
            if let Some(d) = x.value_text.clone() {
                return Some(from_fact(c, x, d, "confirmed"));
            }
        }
        if let Some(x) = fact(st, &c.accession, "prospectus_date") {
            if let Some(d) = x
                .value_text
                .as_deref()
                .and_then(|d| d.parse::<NaiveDate>().ok())
            {
                return Some(from_fact(c, x, ymd(next_trading_day(d)), "expected"));
            }
        }
    }
    let effect = newest(filings.iter().copied().filter(|f| f.form == "EFFECT"))?;
    if !filings.iter().any(|f| f.form == "8-A12B") {
        return None;
    }
    let d: NaiveDate = effect.filing_date.parse().ok()?;
    Some(DateChoice {
        date: ymd(next_trading_day(d)),
        status: "expected",
        source: "filing",
        name: effect.form.clone(),
        url: Some(effect.index_url.clone()),
        quote: None,
        as_of: effect.filing_date.clone(),
    })
}

/// What `date_for` reads besides the filings.
pub struct DateInputs<'a> {
    pub today: NaiveDate,
    pub status: &'a str,
    pub items: &'a [&'a ItemRow],
    pub trading: &'a [&'a TradingRow],
    pub symbol: Option<&'a str>,
}

/// The date line (module docs). `today` is the ET date.
pub fn date_for(
    filings: &[&FilingRow],
    cur: Option<&FilingRow>,
    st: &IpoState,
    x: &DateInputs,
) -> Option<DateChoice> {
    if x.status == "withdrawn" {
        return None;
    }
    let by_filing = filing_date(filings, cur, st);
    if let Some(t) = x
        .trading
        .iter()
        .min_by(|a, b| a.session_date.cmp(&b.session_date))
    {
        if by_filing.as_ref().is_some_and(|d| d.date == t.session_date) {
            return Some(DateChoice {
                date: t.session_date.clone(),
                status: "confirmed",
                source: "trading",
                name: "Yahoo Finance".into(),
                url: Some(yahoo_url(x.symbol.unwrap_or(&t.symbol))),
                quote: None,
                as_of: t.session_date.clone(),
            });
        }
    }
    if by_filing.is_some() || x.status == "listed" {
        return by_filing;
    }
    let newest_item = x
        .items
        .iter()
        .filter(|i| i.kind == "expected_date" && i.value_date.is_some())
        .max_by(|a, b| (&a.published_on, &a.found_at).cmp(&(&b.published_on, &b.found_at)))?;
    let d: NaiveDate = newest_item.value_date.as_deref()?.parse().ok()?;
    (d >= x.today - Duration::days(1)).then(|| DateChoice {
        date: ymd(d),
        status: "reported",
        source: "news",
        name: newest_item.outlet.clone(),
        url: Some(newest_item.url.clone()),
        quote: Some(newest_item.quote.clone()),
        as_of: newest_item.published_on.clone(),
    })
}

fn same(a: &WatchRow, b: &WatchRow) -> bool {
    let strip = |r: &WatchRow| WatchRow {
        checked_at: String::new(),
        updated_at: String::new(),
        ..r.clone()
    };
    strip(a) == strip(b)
}

/// The facts source of `slug` in `st` (once listed, as of the first
/// session).
pub fn current_of<'a>(st: &'a IpoState, slug: &str) -> Option<&'a FilingRow> {
    let all = st.filings_of(slug);
    let filings = considered(&all, first_session(st, slug));
    current(st, &filings)
}

/// The `ipo_watch` row of `t` from `st` (already holding the job's
/// writes), or `None` while EDGAR lists no offering filing for it.
pub fn row_for(now: DateTime<Utc>, t: &Tracked, st: &IpoState) -> Option<WatchRow> {
    let first = first_session(st, &t.slug);
    let all = st.filings_of(&t.slug);
    let filings = considered(&all, first);
    let trading = st.trading_of(&t.slug);
    let items = st.items_of(&t.slug);
    let cur = current(st, &filings);
    let status = status(&filings, cur, st, first.is_some())?;
    let text_fact = |kind: &str| {
        cur.and_then(|c| fact(st, &c.accession, kind))
            .and_then(|x| x.value_text.clone())
    };
    let symbol = text_fact("symbol");
    let date = date_for(
        &filings,
        cur,
        st,
        &DateInputs {
            today: et(now).date_naive(),
            status,
            items: &items,
            trading: &trading,
            symbol: symbol.as_deref(),
        },
    );
    let prev = st.row(&t.slug);
    Some(WatchRow {
        slug: t.slug.clone(),
        cik: t.cik as i64,
        name: t.name.clone(),
        name_zh: t.name_zh.clone(),
        status: status.into(),
        exchange: text_fact("exchange"),
        symbol,
        facts_accession: cur.map(|c| c.accession.clone()),
        first_trade_date: date.as_ref().map(|d| d.date.clone()),
        date_status: date.as_ref().map(|d| d.status.to_string()),
        date_source: date.as_ref().map(|d| d.source.to_string()),
        date_source_name: date.as_ref().map(|d| d.name.clone()),
        date_source_url: date.as_ref().and_then(|d| d.url.clone()),
        date_quote: date.as_ref().and_then(|d| d.quote.clone()),
        date_as_of: date.as_ref().map(|d| d.as_of.clone()),
        checked_at: iso(now),
        updated_at: prev.map_or_else(|| iso(now), |p| p.updated_at.clone()),
    })
}

/// Rows that changed (or are new) for every tracked IPO, with `updated_at`
/// set to `now`.
pub fn changed_rows(now: DateTime<Utc>, list: &[Tracked], st: &IpoState) -> Vec<WatchRow> {
    list.iter()
        .filter_map(|t| {
            let mut r = row_for(now, t, st)?;
            if st.row(&t.slug).is_some_and(|p| same(p, &r)) {
                return None;
            }
            r.updated_at = iso(now);
            Some(r)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipo::parse_list;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn filing(acc: &str, form: &str, date: &str, read: &str) -> FilingRow {
        FilingRow {
            accession: acc.into(),
            slug: "x".into(),
            form: form.into(),
            filing_date: date.into(),
            index_url: format!("https://www.sec.gov/{acc}-index.htm"),
            doc_url: None,
            read_state: read.into(),
            attempts: 0,
            first_seen_at: String::new(),
        }
    }
    fn fact(acc: &str, kind: &str, num: Option<f64>, text: Option<&str>) -> FactRow {
        FactRow {
            accession: acc.into(),
            kind: kind.into(),
            slug: "x".into(),
            value_num: num,
            value_text: text.map(String::from),
            quote: format!("quote of {kind}"),
        }
    }
    fn item(date: &str, published: &str, outlet: &str) -> ItemRow {
        ItemRow {
            slug: "x".into(),
            kind: "expected_date".into(),
            url: format!("https://{outlet}.test/{published}"),
            outlet: outlet.into(),
            published_on: published.into(),
            quote: "q".into(),
            value_date: Some(date.into()),
            found_at: format!("{published}T12:00:00Z"),
            ..Default::default()
        }
    }
    fn trade(session: &str) -> TradingRow {
        TradingRow {
            slug: "x".into(),
            session_date: session.into(),
            symbol: "XX".into(),
            close: 10.0,
            source_url: "u".into(),
            ..Default::default()
        }
    }
    fn tracked() -> Vec<Tracked> {
        parse_list("x\t9\tX Corp\t-\t-\n").unwrap()
    }
    const NOW: &str = "2026-11-11T15:00:00Z";

    fn row(st: &IpoState) -> Option<WatchRow> {
        row_for(utc(NOW), &tracked()[0], st)
    }
    /// An S-1/A with shares, a range and a symbol.
    fn ranged() -> IpoState {
        let mut st = IpoState::default();
        st.filings.push(filing("a3", "S-1/A", "2026-11-10", "read"));
        for (k, n, t) in [
            ("shares_offered", Some(825e6), None),
            ("price_low", Some(70.0), None),
            ("price_high", Some(78.0), None),
            ("symbol", None, Some("XX")),
        ] {
            st.facts.push(fact("a3", k, n, t));
        }
        st
    }
    /// A complete 424B4 priced on `prospectus_date`.
    fn priced(st: &mut IpoState, prospectus_date: &str) {
        st.filings.push(filing("a4", "424B4", "2026-11-13", "read"));
        st.facts.push(fact("a4", "price_final", Some(75.0), None));
        st.facts
            .push(fact("a4", "shares_offered", Some(825e6), None));
        st.facts.push(fact("a4", "symbol", None, Some("XX")));
        st.facts
            .push(fact("a4", "prospectus_date", None, Some(prospectus_date)));
    }

    #[test]
    fn status_moves_filed_range_set_priced_listed() {
        let mut st = IpoState::default();
        assert_eq!(row(&st), None, "no filing: no row");
        st.filings.push(filing("a1", "DRS", "2026-06-01", "none"));
        assert_eq!(row(&st).unwrap().status, "filed");
        st.filings.push(filing("a2", "S-1", "2026-10-26", "read"));
        st.facts.push(fact("a2", "symbol", None, Some("XX")));
        let r = row(&st).unwrap();
        assert_eq!(
            (r.status.as_str(), r.symbol.as_deref()),
            ("filed", Some("XX"))
        );
        assert_eq!(r.facts_accession.as_deref(), Some("a2"));
        let mut st = ranged();
        assert_eq!(row(&st).unwrap().status, "range_set");
        // A 424B4 on file but not yet read is not "priced".
        st.filings
            .push(filing("a4", "424B4", "2026-11-13", "pending"));
        assert_eq!(row(&st).unwrap().status, "range_set");
        st.filings.retain(|f| f.accession != "a4");
        priced(&mut st, "2026-11-12");
        let r = row(&st).unwrap();
        assert_eq!(
            (r.status.as_str(), r.facts_accession.as_deref()),
            ("priced", Some("a4"))
        );
        st.trading.push(trade("2026-11-13"));
        assert_eq!(row(&st).unwrap().status, "listed");
    }

    #[test]
    fn a_424b4_without_its_price_or_shares_replaces_nothing() {
        let mut st = ranged();
        st.filings.push(filing("a4", "424B4", "2026-11-13", "read"));
        st.facts
            .push(fact("a4", "prospectus_date", None, Some("2026-11-12")));
        st.facts.push(fact("a4", "symbol", None, Some("XX")));
        let r = row(&st).unwrap();
        // The S-1/A stays the source: still range_set, the range kept.
        assert_eq!(
            (r.status.as_str(), r.facts_accession.as_deref()),
            ("range_set", Some("a3"))
        );
        let f4 = st.filings.iter().find(|f| f.accession == "a4").unwrap();
        let cur = st.filings.iter().find(|f| f.accession == "a3");
        assert_eq!(
            missing_to_replace(&st, f4, cur),
            ["price_final", "shares_offered"]
        );
        // Its pricing date gives no date either: it is not the source.
        assert_eq!(r.first_trade_date, None);
    }

    #[test]
    fn a_thinner_amendment_never_loses_the_range() {
        let mut st = ranged();
        st.filings.push(filing("a5", "S-1/A", "2026-11-12", "read"));
        st.facts
            .push(fact("a5", "cover_date", None, Some("2026-11-12")));
        st.facts.push(fact("a5", "symbol", None, Some("XX")));
        let r = row(&st).unwrap();
        assert_eq!(
            (r.status.as_str(), r.facts_accession.as_deref()),
            ("range_set", Some("a3"))
        );
        let f5 = st.filings.iter().find(|f| f.accession == "a5").unwrap();
        let cur = st.filings.iter().find(|f| f.accession == "a3");
        assert_eq!(
            missing_to_replace(&st, f5, cur),
            ["shares_offered", "price_low", "price_high"]
        );
        // A full one (a new range) replaces it.
        for (k, n) in [
            ("shares_offered", 825e6),
            ("price_low", 72.0),
            ("price_high", 80.0),
        ] {
            st.facts.push(fact("a5", k, Some(n), None));
        }
        assert_eq!(row(&st).unwrap().facts_accession.as_deref(), Some("a5"));
    }

    #[test]
    fn a_withdrawal_after_the_last_registration_is_withdrawn_with_no_date() {
        let mut st = IpoState::default();
        st.filings.push(filing("a2", "S-1", "2026-10-26", "none"));
        st.items.push(item("2026-11-16", "2026-11-10", "news"));
        assert_eq!(
            row(&st).unwrap().first_trade_date.as_deref(),
            Some("2026-11-16")
        );
        st.filings.push(filing("a5", "RW", "2026-11-11", "none"));
        let r = row(&st).unwrap();
        assert_eq!(r.status, "withdrawn");
        assert_eq!((r.first_trade_date, r.date_status), (None, None));
        // Refiled after the withdrawal: back to filed.
        st.filings.push(filing("a6", "S-1", "2026-12-01", "none"));
        assert_eq!(row(&st).unwrap().status, "filed");
    }

    #[test]
    fn listed_is_terminal_for_filings() {
        let mut st = ranged();
        priced(&mut st, "2026-11-12");
        st.trading.push(trade("2026-11-13"));
        let before = row(&st).unwrap();
        assert_eq!(
            (before.status.as_str(), before.facts_accession.as_deref()),
            ("listed", Some("a4"))
        );
        // A resale RW and a follow-on 424B4 after the first session.
        st.filings.push(filing("a7", "RW", "2026-12-01", "none"));
        st.filings.push(filing("a8", "424B4", "2027-03-01", "read"));
        st.facts.push(fact("a8", "price_final", Some(120.0), None));
        st.facts.push(fact("a8", "shares_offered", Some(1e7), None));
        let after = row(&st).unwrap();
        assert_eq!(after.status, "listed");
        assert_eq!(after.facts_accession.as_deref(), Some("a4"));
        assert_eq!(after.first_trade_date, before.first_trade_date);
    }

    #[test]
    fn the_newest_news_date_is_reported_until_it_passes() {
        let mut st = IpoState::default();
        st.filings.push(filing("a2", "S-1/A", "2026-11-10", "none"));
        st.items.push(item("2026-11-20", "2026-10-12", "older"));
        st.items.push(item("2026-11-16", "2026-11-10", "newer"));
        let r = row(&st).unwrap();
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-16"));
        assert_eq!(r.date_status.as_deref(), Some("reported"));
        assert_eq!(r.date_source.as_deref(), Some("news"));
        assert_eq!(r.date_source_name.as_deref(), Some("newer"));
        assert_eq!(r.date_as_of.as_deref(), Some("2026-11-10"));
        // Yesterday still shows; the day before is dropped, not replaced by
        // the older report.
        let later = |now: &str| row_for(utc(now), &tracked()[0], &st).unwrap();
        assert_eq!(
            later("2026-11-17T15:00:00Z").first_trade_date.as_deref(),
            Some("2026-11-16")
        );
        // The day turns at midnight in New York, not UTC: 23:59 ET on the
        // 17th still shows it, 00:00 ET on the 18th does not.
        assert_eq!(
            later("2026-11-18T04:59:00Z").first_trade_date.as_deref(),
            Some("2026-11-16")
        );
        assert_eq!(later("2026-11-18T05:00:00Z").first_trade_date, None);
        let gone = later("2026-11-18T15:00:00Z");
        assert_eq!((gone.first_trade_date, gone.date_status), (None, None));
    }

    #[test]
    fn a_pricing_date_gives_an_expected_date_and_the_first_session_confirms_it() {
        let mut st = ranged();
        st.items.push(item("2026-11-20", "2026-11-10", "news"));
        priced(&mut st, "2026-11-13");
        let r = row(&st).unwrap();
        // Friday pricing: Monday, expected, from the filing over the news.
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-16"));
        assert_eq!(r.date_status.as_deref(), Some("expected"));
        assert_eq!(r.date_source_name.as_deref(), Some("424B4"));
        assert_eq!(r.date_quote.as_deref(), Some("quote of prospectus_date"));
        // The first session on that day confirms it by trading.
        st.trading.push(trade("2026-11-16"));
        st.trading.push(trade("2026-11-17"));
        let r = row(&st).unwrap();
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-16"));
        assert_eq!(
            (r.date_status.as_deref(), r.date_source.as_deref()),
            (Some("confirmed"), Some("trading"))
        );
        assert_eq!(r.date_quote, None);
    }

    #[test]
    fn a_missed_first_day_never_makes_a_later_session_the_first() {
        let mut st = ranged();
        priced(&mut st, "2026-11-13");
        // Day one (Nov 16) was not read; Nov 17 was.
        st.trading.push(trade("2026-11-17"));
        let r = row(&st).unwrap();
        assert_eq!(r.status, "listed");
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-16"));
        assert_eq!(
            (r.date_status.as_deref(), r.date_source.as_deref()),
            (Some("expected"), Some("filing"))
        );
    }

    #[test]
    fn a_stated_trading_day_counts_only_on_the_current_final_prospectus() {
        let mut st = ranged();
        // An older preliminary cover's trading day is never read as one.
        st.facts
            .push(fact("a3", "trading_date", None, Some("2026-11-14")));
        assert_eq!(row(&st).unwrap().first_trade_date, None);
        priced(&mut st, "2026-11-13");
        st.facts
            .push(fact("a4", "trading_date", None, Some("2026-11-16")));
        let r = row(&st).unwrap();
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-16"));
        assert_eq!(r.date_status.as_deref(), Some("confirmed"));
        assert_eq!(r.date_quote.as_deref(), Some("quote of trading_date"));
    }

    #[test]
    fn effect_with_an_exchange_registration_is_expected() {
        let mut st = IpoState::default();
        st.filings.push(filing("a3", "S-1/A", "2026-11-10", "none"));
        st.filings
            .push(filing("e1", "EFFECT", "2026-11-12", "none"));
        // EFFECT alone is not enough.
        assert_eq!(row(&st).unwrap().first_trade_date, None);
        st.filings
            .push(filing("e2", "8-A12B", "2026-11-11", "none"));
        let r = row(&st).unwrap();
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-13"));
        assert_eq!(r.date_status.as_deref(), Some("expected"));
        assert_eq!(r.date_source_name.as_deref(), Some("EFFECT"));
    }

    #[test]
    fn unchanged_rows_are_not_rewritten() {
        let mut st = IpoState::default();
        st.filings.push(filing("a2", "S-1", "2026-10-26", "none"));
        let rows = changed_rows(utc(NOW), &tracked(), &st);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].updated_at, "2026-11-11T15:00:00Z");
        st.watch = rows;
        assert!(changed_rows(utc("2026-11-11T16:00:00Z"), &tracked(), &st).is_empty());
    }
}
