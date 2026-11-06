//! `CalendarSource` over Yahoo's batched v7 quote session (unofficial, like
//! `yahoo.rs`): each symbol's `earningsTimestamp`, `earningsTimestampStart`,
//! `earningsTimestampEnd` and `isEarningsDateEstimate`, about 3 requests for
//! the whole watchlist. No HTML pages are scraped, and no estimate figures
//! are read (spec §8: no consensus).

use async_trait::async_trait;
use chrono::{NaiveTime, Timelike};

use crate::clock::et;
use crate::earnings::calendar::{CalendarSource, Seed, SeedRead, TimeSlot};
use crate::providers::{Http, MarketData, ProviderError};
use crate::yahoo::{EarningsTimes, Yahoo};

pub struct YahooCalendar<H: Http> {
    pub yahoo: Yahoo<H>,
}

impl<H: Http> YahooCalendar<H> {
    pub fn new(http: H) -> Self {
        Self {
            yahoo: Yahoo::new(http),
        }
    }
}

/// The report's slot from its ET time of day: before the 09:30 open is
/// before market, from the 16:00 close after it. Midnight is Yahoo's "no
/// time", and an estimate's time means nothing.
fn slot(t: &EarningsTimes, local: NaiveTime) -> TimeSlot {
    if t.estimate || local == NaiveTime::MIN {
        TimeSlot::Unknown
    } else if local < NaiveTime::from_hms_opt(9, 30, 0).unwrap() {
        TimeSlot::Bmo
    } else if local.hour() >= 16 {
        TimeSlot::Amc
    } else {
        TimeSlot::Unknown
    }
}

/// The seed for one watchlist ticker: the ET date of the later of
/// `earningsTimestamp` and `earningsTimestampStart`. Until Yahoo rolls it
/// forward, `earningsTimestamp` is the last report while the start of the
/// range is already the next one (an estimate when `isEarningsDateEstimate`):
/// on 2026-11-06, 202 of 561 watchlist symbols, TCOM and PDD among them.
pub fn seed(ticker: &str, t: &EarningsTimes) -> Option<Seed> {
    let local = et([t.at, t.start].into_iter().flatten().max()?);
    Some(Seed {
        ticker: ticker.to_string(),
        date: local.date_naive(),
        slot: slot(t, local.time()),
        estimate: t.estimate,
    })
}

#[async_trait(?Send)]
impl<H: Http> CalendarSource for YahooCalendar<H> {
    async fn upcoming(&self, tickers: &[&str]) -> SeedRead {
        // Yahoo writes class shares with a dash; the watchlist with a dot.
        let symbols: Vec<String> = tickers.iter().map(|t| t.replace('.', "-")).collect();
        let mut out = SeedRead::default();
        for ((_, r), ticker) in self
            .yahoo
            .earnings_times(&symbols)
            .await
            .iter()
            .zip(tickers)
        {
            match r {
                Ok(t) => out.seeds.extend(seed(ticker, t)),
                // No earnings date known for the symbol.
                Err(ProviderError::Empty) => {}
                Err(e) => {
                    out.failed += 1;
                    out.error.get_or_insert_with(|| e.to_string());
                }
            }
        }
        out.diagnostics = self.yahoo.diagnostics();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::HttpResp;
    use chrono::{DateTime, NaiveDate, Utc};
    use std::cell::RefCell;

    fn utc(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse().unwrap())
    }
    fn times(at: &str, estimate: bool) -> EarningsTimes {
        EarningsTimes {
            at: utc(at),
            start: utc(at),
            end: utc(at),
            estimate,
        }
    }
    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn slots_and_et_dates() {
        let s = |at: &str, est: bool| {
            let x = seed("AAPL", &times(at, est)).unwrap();
            (x.date, x.slot)
        };
        // 16:30 EDT; 07:00 EST; 12:00 EDT; midnight ET; an estimate.
        assert_eq!(
            s("2026-10-30T20:30:00Z", false),
            (d("2026-10-30"), TimeSlot::Amc)
        );
        assert_eq!(
            s("2026-11-05T12:00:00Z", false),
            (d("2026-11-05"), TimeSlot::Bmo)
        );
        assert_eq!(
            s("2026-10-30T16:00:00Z", false),
            (d("2026-10-30"), TimeSlot::Unknown)
        );
        assert_eq!(
            s("2026-10-30T04:00:00Z", false),
            (d("2026-10-30"), TimeSlot::Unknown)
        );
        assert_eq!(
            s("2026-10-30T20:30:00Z", true),
            (d("2026-10-30"), TimeSlot::Unknown)
        );
        // After 8 PM ET the UTC date is the next day; the ET date is not.
        assert_eq!(s("2026-10-31T00:30:00Z", false).0, d("2026-10-30"));
        // Only a range: its start.
        let r = EarningsTimes {
            at: None,
            start: utc("2026-10-29T12:00:00Z"),
            end: utc("2026-11-02T12:00:00Z"),
            estimate: true,
        };
        assert_eq!(seed("XOM", &r).unwrap().date, d("2026-10-29"));
        // TCOM on 2026-11-06: `earningsTimestamp` is still the last report
        // (Sep 15); the estimated range is the next one.
        let stale = EarningsTimes {
            at: utc("2026-09-15T20:00:00Z"),
            start: utc("2026-11-16T13:30:00Z"),
            end: utc("2026-11-16T13:30:00Z"),
            estimate: true,
        };
        let x = seed("TCOM", &stale).unwrap();
        assert_eq!(
            (x.date, x.slot, x.estimate),
            (d("2026-11-16"), TimeSlot::Unknown, true)
        );
        // CAT: the last report and a firm next date.
        let firm = EarningsTimes {
            at: utc("2026-10-29T12:30:00Z"),
            start: utc("2027-01-28T12:30:00Z"),
            end: utc("2027-01-28T12:30:00Z"),
            estimate: false,
        };
        let x = seed("CAT", &firm).unwrap();
        assert_eq!(
            (x.date, x.slot, x.estimate),
            (d("2027-01-28"), TimeSlot::Bmo, false)
        );
        let none = EarningsTimes {
            at: None,
            start: None,
            end: None,
            estimate: false,
        };
        assert!(seed("XOM", &none).is_none());
    }

    /// Cookie, crumb, then v7 quotes: AAPL confirmed, BRK-B an estimate,
    /// XOM without a date; `quote_status` answers the quote requests.
    struct Fake {
        quote_status: u16,
        seen: RefCell<Vec<String>>,
    }
    #[async_trait(?Send)]
    impl Http for Fake {
        async fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            self.seen.borrow_mut().push(url.into());
            let r = |status, body: &str, cookie: Option<&str>| HttpResp {
                status,
                body: body.into(),
                set_cookie: cookie.map(String::from),
            };
            if url.starts_with("https://fc.yahoo.com") {
                return Ok(r(404, "", Some("A3=d=x; Path=/")));
            }
            if url.contains("getcrumb") {
                return Ok(r(200, "crumb1", None));
            }
            let body = serde_json::json!({"quoteResponse": {"error": null, "result": [
                {"symbol": "AAPL", "earningsTimestamp": 1793392200_i64, "isEarningsDateEstimate": false},
                {"symbol": "BRK-B", "earningsTimestampStart": 1793361600_i64,
                 "earningsTimestampEnd": 1793448000_i64, "isEarningsDateEstimate": true},
                {"symbol": "XOM"}
            ]}});
            Ok(r(self.quote_status, &body.to_string(), None))
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn reads_the_watchlist_through_v7_quotes() {
        let y = YahooCalendar::new(Fake {
            quote_status: 200,
            seen: RefCell::new(vec![]),
        });
        let r = y.upcoming(&["AAPL", "BRK.B", "XOM"]).await;
        let got: Vec<(&str, NaiveDate, TimeSlot)> = r
            .seeds
            .iter()
            .map(|s| (s.ticker.as_str(), s.date, s.slot))
            .collect();
        assert_eq!(
            got,
            [
                ("AAPL", d("2026-10-30"), TimeSlot::Amc),
                ("BRK.B", d("2026-10-30"), TimeSlot::Unknown),
            ]
        );
        assert_eq!((r.failed, r.error), (0, None));
        let seen = y.yahoo.http.seen.borrow();
        assert_eq!(seen.len(), 3);
        assert!(seen[2].contains("symbols=AAPL,BRK-B,XOM"), "{}", seen[2]);
        assert!(
            !seen.iter().any(|u| u.contains("/calendar/")),
            "no page scraping"
        );
    }

    #[tokio::test]
    async fn a_429_fails_the_read_and_shows_in_diagnostics() {
        let y = YahooCalendar::new(Fake {
            quote_status: 429,
            seen: RefCell::new(vec![]),
        });
        let tickers: Vec<String> = (0..109).map(|i| format!("T{i}")).collect();
        let refs: Vec<&str> = tickers.iter().map(String::as_str).collect();
        let r = y.upcoming(&refs).await;
        assert!(r.seeds.is_empty());
        assert_eq!(r.failed, 109);
        assert_eq!(r.error.as_deref(), Some("http status 429"));
        assert!(
            r.diagnostics.contains(&"yahoo_429=1".to_string()),
            "{:?}",
            r.diagnostics
        );
        // The cookie, the crumb and one quote request: nothing after the 429.
        assert_eq!(y.yahoo.http.seen.borrow().len(), 3);
    }
}
