//! The earnings watchlist (`data/watchlist.tsv`) and the EDGAR polling
//! window.

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};

use crate::clock::et;

const DATA: &str = include_str!("../../data/watchlist.tsv");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filer {
    /// Files 8-K/10-Q/10-K: polled and written automatically.
    Domestic,
    /// Files 6-K/20-F (no Item 2.02): calendar only.
    Foreign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEntry {
    /// Display form, e.g. `BRK.B`.
    pub ticker: &'static str,
    pub cik: u64,
    pub filer: Filer,
    pub name: &'static str,
    /// The company's own web domains; subdomains match.
    pub domains: Vec<&'static str>,
}

fn data_lines() -> impl Iterator<Item = &'static str> {
    DATA.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
}

fn parse(line: &'static str) -> Option<WatchEntry> {
    let mut cols = line.split('\t').map(str::trim);
    let ticker = cols.next()?;
    let cik = cols.next()?.parse().ok()?;
    let filer = match cols.next()? {
        "domestic" => Filer::Domestic,
        "foreign" => Filer::Foreign,
        _ => return None,
    };
    let name = cols.next()?;
    let domains = cols
        .next()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .collect();
    Some(WatchEntry {
        ticker,
        cik,
        filer,
        name,
        domains,
    })
}

/// Every watchlist entry, in file order.
pub fn watchlist() -> Vec<WatchEntry> {
    data_lines()
        .map(|l| parse(l).unwrap_or_else(|| panic!("bad watchlist.tsv row: {l:?}")))
        .collect()
}

pub fn find(ticker: &str) -> Option<WatchEntry> {
    watchlist().into_iter().find(|e| e.ticker == ticker)
}

pub fn by_cik(cik: u64) -> Option<WatchEntry> {
    watchlist().into_iter().find(|e| e.cik == cik)
}

/// EDGAR's zero-padded 10-digit CIK.
pub fn cik10(cik: u64) -> String {
    format!("{cik:010}")
}

/// Weekdays, 06:00 <= ET < 22:00: when EDGAR is polled and earnings jobs run.
pub fn in_window(now: DateTime<Utc>) -> bool {
    let local = et(now);
    if matches!(local.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    let t = local.time();
    t >= NaiveTime::from_hms_opt(6, 0, 0).unwrap() && t < NaiveTime::from_hms_opt(22, 0, 0).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn watchlist_shape() {
        let w = watchlist();
        let domestic: Vec<_> = w.iter().filter(|e| e.filer == Filer::Domestic).collect();
        assert!(domestic.len() >= 100, "{}", domestic.len());
        let tickers: HashSet<_> = w.iter().map(|e| e.ticker).collect();
        let ciks: HashSet<_> = w.iter().map(|e| e.cik).collect();
        assert_eq!(tickers.len(), w.len(), "duplicate ticker");
        assert_eq!(ciks.len(), w.len(), "duplicate CIK");
        assert!(w
            .iter()
            .all(|e| !e.domains.is_empty() && !e.name.is_empty()));
        assert_eq!(find("AAPL").unwrap().cik, 320193);
        assert_eq!(find("BRK.B").unwrap().name, "Berkshire Hathaway");
        assert_eq!(find("TM").unwrap().filer, Filer::Foreign);
        assert_eq!(by_cik(789019).unwrap().ticker, "MSFT");
    }

    #[test]
    fn ciks_match_sec_company_tickers() {
        // Checked against https://www.sec.gov/files/company_tickers.json on
        // 2026-10-29: BNY trades as BNY (was BK); XOM's filer is the new
        // holding company.
        assert!(find("BK").is_none());
        assert_eq!(find("BNY").unwrap().cik, 1390777);
        let xom = find("XOM").unwrap();
        assert_eq!((xom.cik, xom.name), (2115436, "ExxonMobil"));
        assert!(by_cik(34088).is_none());
    }

    #[test]
    fn every_row_parses() {
        assert_eq!(data_lines().count(), watchlist().len());
    }

    #[test]
    fn cik_padding() {
        assert_eq!(cik10(320193), "0000320193");
        assert_eq!(cik10(1800), "0000001800");
    }

    #[test]
    fn window_edges_and_dst() {
        // Fri 2026-10-30 is EDT (UTC-4).
        assert!(!in_window(utc("2026-10-30T09:59:59Z"))); // 05:59:59 ET
        assert!(in_window(utc("2026-10-30T10:00:00Z"))); // 06:00 ET
        assert!(in_window(utc("2026-10-31T01:59:59Z"))); // Fri 21:59:59 ET
        assert!(!in_window(utc("2026-10-31T02:00:00Z"))); // Fri 22:00 ET

        // Mon 2026-11-02 is EST (UTC-5).
        assert!(!in_window(utc("2026-11-02T10:30:00Z"))); // 05:30 ET
        assert!(in_window(utc("2026-11-02T11:00:00Z"))); // 06:00 ET
                                                         // Saturday.
        assert!(!in_window(utc("2026-10-31T15:00:00Z")));
    }
}
