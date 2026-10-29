//! The earnings watchlist (`data/watchlist.tsv`) and the EDGAR polling
//! schedule: which companies one tick polls, and when polling runs.

use chrono::{DateTime, Datelike, NaiveTime, Timelike, Utc, Weekday};

use crate::clock::et;

const DATA: &str = include_str!("../../data/watchlist.tsv");
/// Companies polled per one-minute tick (one submissions request each).
pub const BATCH: usize = 21;

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
    DATA.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(parse)
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

/// The domestic companies this minute's tick polls. Consecutive minutes walk
/// the chunks in order, so every company is polled once per cycle.
pub fn poll_batch(now: DateTime<Utc>) -> Vec<WatchEntry> {
    let domestic: Vec<WatchEntry> = watchlist()
        .into_iter()
        .filter(|e| e.filer == Filer::Domestic)
        .collect();
    let chunks = domestic.len().div_ceil(BATCH).max(1);
    let local = et(now);
    let minute = (local.hour() * 60 + local.minute()) as usize;
    let i = minute % chunks;
    domestic.into_iter().skip(i * BATCH).take(BATCH).collect()
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

    #[test]
    fn five_minutes_cover_every_domestic_company_once() {
        let mut seen = Vec::new();
        for m in 0..5 {
            let t = utc("2026-10-30T14:00:00Z") + chrono::Duration::minutes(m);
            let b = poll_batch(t);
            assert!(b.len() <= BATCH && !b.is_empty());
            assert!(b.iter().all(|e| e.filer == Filer::Domestic));
            seen.extend(b.into_iter().map(|e| e.ticker));
        }
        let domestic = watchlist()
            .into_iter()
            .filter(|e| e.filer == Filer::Domestic)
            .count();
        let unique: HashSet<_> = seen.iter().collect();
        assert_eq!(seen.len(), domestic);
        assert_eq!(unique.len(), domestic);
    }
}
