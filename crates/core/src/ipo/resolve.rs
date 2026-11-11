//! The status and the date line, by code, from what is stored: the forms
//! EDGAR lists, the covers read, the news items and the closes. Every job
//! that writes (`check`, `news`, `trading`) recomputes the rows from the
//! state it is about to write, so the rules live in one place.
//!
//! The date rule (`docs/decisions.md`, 2026-11-11), first that applies:
//! 1. **confirmed, trading**: the symbol's first regular session Yahoo
//!    reported (the recap's close read).
//! 2. **confirmed, filing**: a filing that states the first trading day in
//!    its own words ("is expected to begin trading ... on November 5, 2026";
//!    never "on or about").
//! 3. **expected, filing**: the trading day after the pricing date a final
//!    prospectus states ("Prospectus dated November 4, 2026"); else, with an
//!    `8-A12B` on file, the trading day after the SEC's `EFFECT` notice. A
//!    rule, so never "confirmed".
//! 4. **reported, news**: the date the newest `expected_date` item gives,
//!    unless that date is before yesterday (ET).
//! 5. Otherwise no date ("date not set"). A withdrawn IPO has no date.

use chrono::{DateTime, Duration, NaiveDate, Utc};

use super::store::{FactRow, FilingRow, IpoState, ItemRow, TradingRow, WatchRow};
use super::{is_draft, is_final, is_registration, iso, next_trading_day, ymd, Tracked};
use crate::clock::et;
use crate::premarket::yahoo_symbol;

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

/// Filings with facts read, newest first.
fn read_newest_first<'a>(filings: &[&'a FilingRow], st: &IpoState) -> Vec<&'a FilingRow> {
    let mut v: Vec<&FilingRow> = filings
        .iter()
        .copied()
        .filter(|f| f.read_state == "read" && !st.facts_of(&f.accession).is_empty())
        .collect();
    v.sort_by(|a, b| order(b).cmp(&order(a)));
    v
}

fn fact<'a>(st: &'a IpoState, accession: &str, kind: &str) -> Option<&'a FactRow> {
    st.facts
        .iter()
        .find(|f| f.accession == accession && f.kind == kind)
}

/// The status from the forms on file (contract, "Status").
pub fn status(
    filings: &[&FilingRow],
    current: Option<&str>,
    st: &IpoState,
    trading: &[&TradingRow],
) -> Option<&'static str> {
    let offering = newest(
        filings
            .iter()
            .copied()
            .filter(|f| is_registration(&f.form) || is_final(&f.form) || is_draft(&f.form)),
    );
    let rw = newest(filings.iter().copied().filter(|f| f.form == "RW"));
    if let (Some(rw), last) = (rw, offering) {
        if last.is_none_or(|o| order(rw) >= order(o)) {
            return Some("withdrawn");
        }
    }
    offering?;
    if !trading.is_empty() {
        return Some("listed");
    }
    if filings.iter().any(|f| is_final(&f.form)) {
        return Some("priced");
    }
    let ranged = current
        .is_some_and(|a| fact(st, a, "price_low").is_some() && fact(st, a, "price_high").is_some());
    Some(if ranged { "range_set" } else { "filed" })
}

pub fn yahoo_url(symbol: &str) -> String {
    format!("https://finance.yahoo.com/quote/{}", yahoo_symbol(symbol))
}

/// The date line (module docs). `today` is the ET date.
pub fn date_for(
    today: NaiveDate,
    status: &str,
    filings: &[&FilingRow],
    st: &IpoState,
    items: &[&ItemRow],
    trading: &[&TradingRow],
    symbol: Option<&str>,
) -> Option<DateChoice> {
    if status == "withdrawn" {
        return None;
    }
    if let Some(t) = trading
        .iter()
        .min_by(|a, b| a.session_date.cmp(&b.session_date))
    {
        return Some(DateChoice {
            date: t.session_date.clone(),
            status: "confirmed",
            source: "trading",
            name: "Yahoo Finance".into(),
            url: Some(yahoo_url(symbol.unwrap_or(&t.symbol))),
            quote: None,
            as_of: t.session_date.clone(),
        });
    }
    let read = read_newest_first(filings, st);
    let from_filing =
        |f: &FilingRow, fact: &FactRow, date: String, status: &'static str| DateChoice {
            date,
            status,
            source: "filing",
            name: f.form.clone(),
            url: Some(f.index_url.clone()),
            quote: Some(fact.quote.clone()),
            as_of: f.filing_date.clone(),
        };
    for f in &read {
        if let Some(x) = fact(st, &f.accession, "trading_date") {
            if let Some(d) = x.value_text.clone() {
                return Some(from_filing(f, x, d, "confirmed"));
            }
        }
    }
    for f in read.iter().filter(|f| is_final(&f.form)) {
        if let Some(x) = fact(st, &f.accession, "prospectus_date") {
            if let Some(d) = x
                .value_text
                .as_deref()
                .and_then(|d| d.parse::<NaiveDate>().ok())
            {
                return Some(from_filing(f, x, ymd(next_trading_day(d)), "expected"));
            }
        }
    }
    let effect = newest(filings.iter().copied().filter(|f| f.form == "EFFECT"));
    let registered = filings.iter().any(|f| f.form == "8-A12B");
    if let (Some(e), true) = (effect, registered) {
        if let Ok(d) = e.filing_date.parse::<NaiveDate>() {
            return Some(DateChoice {
                date: ymd(next_trading_day(d)),
                status: "expected",
                source: "filing",
                name: e.form.clone(),
                url: Some(e.index_url.clone()),
                quote: None,
                as_of: e.filing_date.clone(),
            });
        }
    }
    let newest_item = items
        .iter()
        .filter(|i| i.kind == "expected_date" && i.value_date.is_some())
        .max_by(|a, b| (&a.published_on, &a.found_at).cmp(&(&b.published_on, &b.found_at)))?;
    let d: NaiveDate = newest_item.value_date.as_deref()?.parse().ok()?;
    (d >= today - Duration::days(1)).then(|| DateChoice {
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

/// The `ipo_watch` row of `t` from `st` (already holding the job's
/// writes), or `None` while EDGAR lists no offering filing for it.
pub fn row_for(now: DateTime<Utc>, t: &Tracked, st: &IpoState) -> Option<WatchRow> {
    let filings = st.filings_of(&t.slug);
    let trading = st.trading_of(&t.slug);
    let items = st.items_of(&t.slug);
    let read = read_newest_first(&filings, st);
    let current = read.first().map(|f| f.accession.clone());
    let status = status(&filings, current.as_deref(), st, &trading)?;
    // The newest stated exchange and symbol, from any cover read.
    let text_fact = |kind: &str| {
        read.iter()
            .find_map(|f| fact(st, &f.accession, kind).and_then(|x| x.value_text.clone()))
    };
    let symbol = text_fact("symbol");
    let today = et(now).date_naive();
    let date = date_for(
        today,
        status,
        &filings,
        st,
        &items,
        &trading,
        symbol.as_deref(),
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
        facts_accession: current,
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
        parse_list("x\t9\tX Corp\t-\n").unwrap()
    }
    const NOW: &str = "2026-11-11T15:00:00Z";

    fn row(st: &IpoState) -> Option<WatchRow> {
        row_for(utc(NOW), &tracked()[0], st)
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
        st.filings.push(filing("a3", "S-1/A", "2026-11-10", "read"));
        st.facts.push(fact("a3", "price_low", Some(70.0), None));
        st.facts.push(fact("a3", "price_high", Some(78.0), None));
        let r = row(&st).unwrap();
        assert_eq!(r.status, "range_set");
        // The symbol is kept from the older cover.
        assert_eq!(r.symbol.as_deref(), Some("XX"));
        st.filings
            .push(filing("a4", "424B4", "2026-11-13", "pending"));
        assert_eq!(row(&st).unwrap().status, "priced");
        st.trading.push(trade("2026-11-13"));
        assert_eq!(row(&st).unwrap().status, "listed");
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
        let gone = later("2026-11-18T15:00:00Z");
        assert_eq!((gone.first_trade_date, gone.date_status), (None, None));
    }

    #[test]
    fn a_pricing_date_gives_an_expected_date_never_a_confirmed_one() {
        let mut st = IpoState::default();
        st.filings.push(filing("a3", "S-1/A", "2026-11-10", "read"));
        st.facts.push(fact("a3", "price_low", Some(70.0), None));
        st.items.push(item("2026-11-16", "2026-11-10", "news"));
        st.filings.push(filing("a4", "424B4", "2026-11-13", "read"));
        st.facts
            .push(fact("a4", "prospectus_date", None, Some("2026-11-13")));
        let r = row(&st).unwrap();
        // Friday pricing: Monday.
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-16"));
        assert_eq!(r.date_status.as_deref(), Some("expected"));
        assert_eq!(r.date_source.as_deref(), Some("filing"));
        assert_eq!(r.date_source_name.as_deref(), Some("424B4"));
        assert_eq!(r.date_quote.as_deref(), Some("quote of prospectus_date"));
        // A stated trading day confirms it.
        st.facts
            .push(fact("a4", "trading_date", None, Some("2026-11-16")));
        let r = row(&st).unwrap();
        assert_eq!(r.date_status.as_deref(), Some("confirmed"));
        assert_eq!(r.date_quote.as_deref(), Some("quote of trading_date"));
        // The first session confirms it by trading, whatever came before.
        st.trading.push(trade("2026-11-17"));
        st.trading.push(trade("2026-11-18"));
        let r = row(&st).unwrap();
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-17"));
        assert_eq!(r.date_source.as_deref(), Some("trading"));
        assert_eq!(r.date_source_name.as_deref(), Some("Yahoo Finance"));
        assert_eq!(r.date_quote, None);
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
