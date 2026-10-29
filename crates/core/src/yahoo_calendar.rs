//! `CalendarSource` over Yahoo Finance's earnings calendar page (unofficial,
//! like `yahoo.rs`). Only the symbol and the call-time columns are read; the
//! estimate columns are never parsed or stored (spec §8: no consensus).

use std::sync::OnceLock;

use async_trait::async_trait;
use chrono::NaiveDate;
use regex::Regex;

use crate::earnings::calendar::{CalendarSource, Seed, TimeSlot};
use crate::edgar::html_to_text;
use crate::providers::{Http, ProviderError};

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
const PAGE_SIZE: usize = 100;
/// Busy days list several hundred companies; the watchlist is the largest of them.
const MAX_PAGES: usize = 3;

pub struct YahooCalendar<H: Http> {
    pub http: H,
}

impl<H: Http> YahooCalendar<H> {
    pub fn new(http: H) -> Self {
        Self { http }
    }
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
}

fn slot(text: &str) -> TimeSlot {
    let t = text.trim().to_ascii_lowercase();
    if t == "bmo" || t.contains("before market") {
        TimeSlot::Bmo
    } else if t == "amc" || t.contains("after market") {
        TimeSlot::Amc
    } else {
        TimeSlot::Unknown
    }
}

/// Seeds from one results page, in page order, plus the number of data rows
/// (to know whether another page follows).
pub fn parse_page(html: &str, date: NaiveDate) -> Result<(Vec<Seed>, usize), ProviderError> {
    static DROP: OnceLock<Regex> = OnceLock::new();
    static TABLE: OnceLock<Regex> = OnceLock::new();
    static ROW: OnceLock<Regex> = OnceLock::new();
    static TH: OnceLock<Regex> = OnceLock::new();
    static TD: OnceLock<Regex> = OnceLock::new();
    let drop = DROP.get_or_init(|| re(r"(?is)<script\b.*?</script\s*>|<!--.*?-->"));
    let html = drop.replace_all(html, " ");
    let table = TABLE
        .get_or_init(|| re(r"(?is)<table\b.*?</table\s*>"))
        .find(&html)
        .ok_or_else(|| ProviderError::Parse("no results table".into()))?;
    let th = TH.get_or_init(|| re(r"(?is)<th\b[^>]*>(.*?)</th\s*>"));
    let td = TD.get_or_init(|| re(r"(?is)<td\b[^>]*>(.*?)</td\s*>"));
    let headers: Vec<String> = th
        .captures_iter(table.as_str())
        .map(|c| html_to_text(&c[1]).to_ascii_lowercase())
        .collect();
    let col = |want: &[&str]| {
        headers
            .iter()
            .position(|h| want.iter().any(|w| h.contains(w)))
    };
    let symbol = col(&["symbol"]).ok_or_else(|| ProviderError::Parse("no Symbol column".into()))?;
    let time = col(&["call time", "event start", "time"]);
    let mut out = Vec::new();
    let mut rows = 0;
    for r in ROW
        .get_or_init(|| re(r"(?is)<tr\b[^>]*>(.*?)</tr\s*>"))
        .captures_iter(table.as_str())
    {
        let cells: Vec<String> = td
            .captures_iter(&r[1])
            .map(|c| html_to_text(&c[1]))
            .collect();
        if cells.is_empty() {
            continue;
        }
        rows += 1;
        let Some(sym) = cells
            .get(symbol)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        out.push(Seed {
            // Yahoo writes class shares with a dash; the watchlist with a dot.
            ticker: sym.replace('-', "."),
            date,
            slot: time
                .and_then(|i| cells.get(i))
                .map_or(TimeSlot::Unknown, |t| slot(t)),
        });
    }
    Ok((out, rows))
}

#[async_trait(?Send)]
impl<H: Http> CalendarSource for YahooCalendar<H> {
    async fn day(&self, date: NaiveDate) -> Result<Vec<Seed>, ProviderError> {
        let mut out = Vec::new();
        for page in 0..MAX_PAGES {
            let url = format!(
                "https://finance.yahoo.com/calendar/earnings?day={}&offset={}&size={PAGE_SIZE}",
                date.format("%Y-%m-%d"),
                page * PAGE_SIZE
            );
            let r = self.http.get(&url, &[("User-Agent", UA)]).await?;
            if r.status != 200 {
                return Err(ProviderError::Http(r.status));
            }
            let (seeds, rows) = parse_page(&r.body, date)?;
            out.extend(seeds);
            if rows < PAGE_SIZE {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edgar::tests::FakeHttp;

    const PAGE: &str = include_str!("../tests/fixtures/yahoo_earnings_calendar.html");
    const URL0: &str =
        "https://finance.yahoo.com/calendar/earnings?day=2026-10-30&offset=0&size=100";

    fn d() -> NaiveDate {
        "2026-10-30".parse().unwrap()
    }

    #[test]
    fn parses_symbols_and_slots() {
        let (s, rows) = parse_page(PAGE, d()).unwrap();
        assert_eq!(rows, 5);
        let got: Vec<(&str, TimeSlot)> = s.iter().map(|x| (x.ticker.as_str(), x.slot)).collect();
        assert_eq!(
            got,
            [
                ("AAPL", TimeSlot::Amc),
                ("XOM", TimeSlot::Bmo),
                ("BRK.B", TimeSlot::Unknown),
                ("ZZZQ", TimeSlot::Bmo),
                ("TM", TimeSlot::Unknown),
            ]
        );
        assert!(s.iter().all(|x| x.date == d()));
    }

    #[test]
    fn script_tables_are_ignored() {
        let (s, _) = parse_page(PAGE, d()).unwrap();
        assert!(s.iter().all(|x| x.ticker != "FAKE"));
    }

    #[tokio::test]
    async fn http_errors_and_blocks() {
        let mut http = FakeHttp::default();
        http.routes.insert(URL0.into(), (429, String::new()));
        let y = YahooCalendar::new(http);
        assert_eq!(y.day(d()).await, Err(ProviderError::Http(429)));
        let mut http = FakeHttp::default();
        http.routes
            .insert(URL0.into(), (200, "<html>consent</html>".into()));
        let y = YahooCalendar::new(http);
        assert!(matches!(y.day(d()).await, Err(ProviderError::Parse(_))));
    }

    #[tokio::test]
    async fn short_page_stops_paging() {
        let mut http = FakeHttp::default();
        http.routes.insert(URL0.into(), (200, PAGE.into()));
        let y = YahooCalendar::new(http);
        assert_eq!(y.day(d()).await.unwrap().len(), 5);
        assert_eq!(y.http.seen.borrow().len(), 1);
    }
}
