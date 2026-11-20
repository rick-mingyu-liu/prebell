//! After listing: the recap's batched Yahoo close read also asks for the
//! symbols of tracked IPOs that are priced or listed, as long as they fit
//! in the room its last batch has left (no extra request), and for 30 days
//! after the first session. A symbol already in the universe (ANT since
//! 2026-11-19) is read once, as a universe ticker, and takes no room. Each close is stored with the IPO price from
//! the final prospectus and the change against it, computed by code from
//! those two stored numbers. The first session found confirms the date.

use chrono::{DateTime, Duration, NaiveDate, Utc};

use super::resolve::{changed_rows, current_of, first_session, yahoo_url};
use super::store::{IpoState, IpoStore, IpoWrite, TradingRow};
use super::{is_final, iso, ymd, Tracked};
use crate::facts::{round2, Quote};
use crate::premarket::{in_universe, yahoo_symbol};
use crate::providers::ProviderError;
use crate::yahoo::QUOTE_BATCH;

/// Sessions read after the first one.
pub const TRADING_DAYS: i64 = 30;

/// Symbols that fit in the recap's last v7 batch without another request.
pub fn room(universe_len: usize) -> usize {
    (QUOTE_BATCH - universe_len % QUOTE_BATCH) % QUOTE_BATCH
}

/// `(slug, Yahoo symbol)` to read for `session`: a symbol from the cover
/// and a price range or more (`range_set`, `priced`, `listed`), so the
/// first day is read even when the final prospectus is filed that day;
/// before listing only once a pricing signal is on file (an `EFFECT`
/// notice or a final prospectus filed on or before `session`), so another
/// security that shares the symbol is never read as the IPO; within
/// `TRADING_DAYS` of the first session; at most `room` of them outside the
/// universe (a universe ticker is in the batches anyway and takes none).
pub fn symbols(
    st: &IpoState,
    list: &[Tracked],
    session: NaiveDate,
    room: usize,
) -> Vec<(String, String)> {
    let mut left = room;
    list.iter()
        .filter_map(|t| {
            let r = st.row(&t.slug)?;
            if !matches!(r.status.as_str(), "range_set" | "priced" | "listed") {
                return None;
            }
            let first = first_session(st, &t.slug);
            if first.is_some_and(|f| session - f > Duration::days(TRADING_DAYS)) {
                return None;
            }
            let signal = st.filings_of(&t.slug).iter().any(|f| {
                (f.form == "EFFECT" || is_final(&f.form))
                    && f.filing_date
                        .parse::<NaiveDate>()
                        .is_ok_and(|d| d <= session)
            });
            if first.is_none() && !signal {
                return None;
            }
            Some((t.slug.clone(), r.symbol.clone()?))
        })
        .filter(|(_, sym)| {
            if in_universe(sym) {
                return true;
            }
            let fits = left > 0;
            left = left.saturating_sub(1);
            fits
        })
        .map(|(slug, sym)| (slug, yahoo_symbol(&sym)))
        .collect()
}

/// The final price of the facts source when it is a final prospectus (the
/// IPO's own, frozen at listing; never a follow-on's).
pub fn ipo_price(st: &IpoState, slug: &str) -> Option<f64> {
    let c = current_of(st, slug).filter(|c| is_final(&c.form))?;
    st.facts
        .iter()
        .find(|x| x.accession == c.accession && x.kind == "price_final")
        .and_then(|x| x.value_num)
}

/// Stored sessions of `slug` whose IPO price or change differs from the
/// facts source's final price (it was read after them), recomputed.
pub fn backfill(st: &IpoState, slug: &str, now: DateTime<Utc>) -> Vec<TradingRow> {
    let price = ipo_price(st, slug);
    st.trading_of(slug)
        .into_iter()
        .filter(|t| t.ipo_price != price)
        .map(|t| TradingRow {
            ipo_price: price,
            change_pct: price
                .filter(|p| *p > 0.0)
                .map(|p| round2((t.close - p) / p * 100.0)),
            fetched_at: iso(now),
            ..t.clone()
        })
        .collect()
}

/// The rows for the closes read: one per symbol that has a close for
/// `session`.
pub fn rows(
    st: &IpoState,
    picks: &[(String, String)],
    results: &[(String, Result<Quote, ProviderError>)],
    session: NaiveDate,
    now: DateTime<Utc>,
) -> Vec<TradingRow> {
    picks
        .iter()
        .filter_map(|(slug, sym)| {
            let q = results.iter().find(|(s, _)| s == sym)?.1.as_ref().ok()?;
            let price = ipo_price(st, slug);
            Some(TradingRow {
                slug: slug.clone(),
                session_date: ymd(session),
                symbol: sym.clone(),
                open: None,
                close: q.last,
                volume: q.volume.map(|v| v as i64),
                ipo_price: price,
                change_pct: price
                    .filter(|p| *p > 0.0)
                    .map(|p| round2((q.last - p) / p * 100.0)),
                source_url: yahoo_url(sym),
                fetched_at: iso(now),
            })
        })
        .collect()
}

/// Stores the closes and the rows they change; the log line.
pub async fn record(
    ipo: &dyn IpoStore,
    mut st: IpoState,
    list: &[Tracked],
    picks: &[(String, String)],
    results: &[(String, Result<Quote, ProviderError>)],
    session: NaiveDate,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let mut w = IpoWrite {
        trading: rows(&st, picks, results, session, now),
        ..Default::default()
    };
    let n = w.trading.len();
    if n == 0 {
        return Ok(format!("ipo symbols={} closes=0", picks.len()));
    }
    st.apply(&w);
    w.watch = changed_rows(now, list, &st);
    ipo.ipo_write(&w).await?;
    Ok(format!("ipo symbols={} closes={n}", picks.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipo::parse_list;
    use crate::ipo::store::{FactRow, FilingRow, MemIpoStore, WatchRow};

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }
    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn list() -> Vec<Tracked> {
        parse_list("nw\t9\tNorthwind\t-\nhb\t8\tHarbor\t-\n").unwrap()
    }
    fn state() -> IpoState {
        let row = |slug: &str, status: &str, sym: &str| WatchRow {
            slug: slug.into(),
            status: status.into(),
            symbol: Some(sym.into()),
            ..Default::default()
        };
        IpoState {
            watch: vec![row("nw", "priced", "NWRB"), row("hb", "range_set", "HRBO")],
            filings: vec![FilingRow {
                accession: "b4".into(),
                slug: "nw".into(),
                form: "424B4".into(),
                filing_date: "2026-11-05".into(),
                index_url: "https://www.sec.gov/b4-index.htm".into(),
                read_state: "read".into(),
                ..Default::default()
            }],
            facts: vec![
                FactRow {
                    accession: "b4".into(),
                    kind: "price_final".into(),
                    slug: "nw".into(),
                    value_num: Some(24.0),
                    quote: "The initial public offering price is $24.00 per share.".into(),
                    ..Default::default()
                },
                FactRow {
                    accession: "b4".into(),
                    kind: "shares_offered".into(),
                    slug: "nw".into(),
                    value_num: Some(2e7),
                    quote: "20,000,000 Shares Class A Common Stock".into(),
                    ..Default::default()
                },
                FactRow {
                    accession: "b4".into(),
                    kind: "prospectus_date".into(),
                    slug: "nw".into(),
                    value_text: Some("2026-11-04".into()),
                    quote: "Prospectus dated November 4, 2026".into(),
                    ..Default::default()
                },
                FactRow {
                    accession: "b4".into(),
                    kind: "symbol".into(),
                    slug: "nw".into(),
                    value_text: Some("NWRB".into()),
                    quote: "under the symbol “NWRB.”".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }
    fn quote(last: f64, vol: u64) -> Quote {
        Quote {
            symbol: "NWRB".into(),
            last,
            prev_close: 24.0,
            change_pct: 0.0,
            volume: Some(vol),
        }
    }

    #[test]
    fn room_is_what_the_last_batch_leaves() {
        assert_eq!(room(518), 32);
        assert_eq!(room(500), 0);
        assert_eq!(room(549), 1);
        // 519 tickers since ANT was added (2026-11-19): 31 slots left.
        assert_eq!(room(crate::premarket::universe().len()), 31);
    }

    #[test]
    fn only_priced_or_listed_with_a_symbol_and_within_the_room() {
        let st = state();
        assert_eq!(
            symbols(&st, &list(), d("2026-11-05"), 32),
            [("nw".to_string(), "NWRB".to_string())]
        );
        assert!(symbols(&st, &list(), d("2026-11-05"), 0).is_empty());
        let mut st = st;
        st.trading.push(TradingRow {
            slug: "nw".into(),
            session_date: "2026-11-05".into(),
            ..Default::default()
        });
        assert_eq!(symbols(&st, &list(), d("2026-12-05"), 32).len(), 1);
        assert!(symbols(&st, &list(), d("2026-12-06"), 32).is_empty());
    }

    /// A tracked IPO whose symbol is a universe ticker (ANT since
    /// 2026-11-19) is read with the universe and takes no room: it is picked
    /// even with none left, and the others still get the room there is.
    #[test]
    fn a_universe_symbol_takes_no_room() {
        let mut st = state();
        st.watch[0].symbol = Some("ANT".into());
        assert_eq!(
            symbols(&st, &list(), d("2026-11-05"), 0),
            [("nw".to_string(), "ANT".to_string())]
        );
        st.filings.push(FilingRow {
            accession: "e".into(),
            slug: "hb".into(),
            form: "EFFECT".into(),
            filing_date: "2026-11-05".into(),
            ..Default::default()
        });
        assert_eq!(
            symbols(&st, &list(), d("2026-11-05"), 1),
            [
                ("nw".to_string(), "ANT".to_string()),
                ("hb".to_string(), "HRBO".to_string())
            ]
        );
        assert_eq!(symbols(&st, &list(), d("2026-11-05"), 0).len(), 1);
    }

    #[test]
    fn a_range_set_ipo_is_read_once_a_pricing_signal_is_on_file() {
        let mut st = state();
        st.filings.push(FilingRow {
            accession: "e".into(),
            slug: "hb".into(),
            form: "EFFECT".into(),
            filing_date: "2026-11-05".into(),
            ..Default::default()
        });
        let syms: Vec<String> = symbols(&st, &list(), d("2026-11-05"), 32)
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        assert_eq!(syms, ["NWRB", "HRBO"]);
        // The day before the notice: not yet.
        assert_eq!(symbols(&st, &list(), d("2026-11-04"), 32).len(), 0);
    }

    #[tokio::test]
    async fn a_first_close_lists_it_confirms_the_date_and_keeps_both_inputs() {
        let ipo = MemIpoStore::default();
        *ipo.state.borrow_mut() = state();
        let picks = vec![("nw".to_string(), "NWRB".to_string())];
        let results = vec![
            ("NWRB".to_string(), Ok(quote(31.2, 48_210_000))),
            ("HRBO".to_string(), Err(ProviderError::Empty)),
        ];
        let st = ipo.state.borrow().clone();
        let line = record(
            &ipo,
            st,
            &list(),
            &picks,
            &results,
            d("2026-11-05"),
            utc("2026-11-05T21:00:00Z"),
        )
        .await
        .unwrap();
        assert_eq!(line, "ipo symbols=1 closes=1");
        let st = ipo.state.borrow();
        let t = &st.trading[0];
        assert_eq!(
            (t.close, t.volume, t.ipo_price, t.change_pct),
            (31.2, Some(48_210_000), Some(24.0), Some(30.0))
        );
        assert_eq!(t.open, None);
        assert_eq!(t.source_url, "https://finance.yahoo.com/quote/NWRB");
        let r = st.row("nw").unwrap();
        assert_eq!(r.status, "listed");
        assert_eq!(r.first_trade_date.as_deref(), Some("2026-11-05"));
        assert_eq!(
            (r.date_status.as_deref(), r.date_source.as_deref()),
            (Some("confirmed"), Some("trading"))
        );
    }

    #[tokio::test]
    async fn no_close_writes_nothing() {
        let ipo = MemIpoStore::default();
        let picks = vec![("nw".to_string(), "NWRB".to_string())];
        let results = vec![("NWRB".to_string(), Err(ProviderError::Stale))];
        let line = record(
            &ipo,
            state(),
            &list(),
            &picks,
            &results,
            d("2026-11-05"),
            utc("2026-11-05T21:00:00Z"),
        )
        .await
        .unwrap();
        assert_eq!(line, "ipo symbols=1 closes=0");
        assert_eq!(ipo.calls.get(), 0);
    }

    #[test]
    fn without_a_final_price_there_is_no_change() {
        let mut st = state();
        st.facts.retain(|f| f.kind != "price_final");
        let r = rows(
            &st,
            &[("nw".into(), "NWRB".into())],
            &[("NWRB".into(), Ok(quote(31.2, 1)))],
            d("2026-11-05"),
            utc("2026-11-05T21:00:00Z"),
        );
        assert_eq!((r[0].ipo_price, r[0].change_pct), (None, None));
    }
}
