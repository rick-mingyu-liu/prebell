//! Which current-filings feed entries are new earnings releases of the
//! watchlist.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

use crate::earnings::{watchlist, Filer, WatchEntry};
use crate::edgar::FeedEntry;

/// A 2.02 8-K accepted longer ago than this is history, not news. Long
/// enough to pick up a Saturday filing on Monday morning.
pub const MAX_AGE_HOURS: i64 = 96;

/// The domestic (8-K filing) watchlist companies by CIK.
pub fn domestic_by_cik() -> HashMap<u64, WatchEntry> {
    watchlist()
        .into_iter()
        .filter(|e| e.filer == Filer::Domestic)
        .map(|e| (e.cik, e))
        .collect()
}

/// Original 8-K filings (not `8-K/A`) with Item 2.02 by a domestic
/// watchlist company, accepted within `MAX_AGE_HOURS`, with their entry.
pub fn earnings_entries<'a>(
    entries: &'a [FeedEntry],
    watch: &HashMap<u64, WatchEntry>,
    now: DateTime<Utc>,
) -> Vec<(&'a FeedEntry, WatchEntry)> {
    entries
        .iter()
        .filter(|f| f.form == "8-K")
        .filter(|f| f.items.iter().any(|i| i == "2.02"))
        .filter(|f| now - f.accepted_at <= Duration::hours(MAX_AGE_HOURS))
        .filter_map(|f| Some((f, watch.get(&f.cik)?.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edgar::{parse_current_feed, tests::FEED};

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn entry(cik: u64, form: &str, items: &[&str], accepted: &str) -> FeedEntry {
        FeedEntry {
            form: form.into(),
            cik,
            company: "X".into(),
            accession: format!("0000000001-26-{:06}", items.len() + form.len()),
            filing_date: "2026-10-30".into(),
            items: items.iter().map(|i| i.to_string()).collect(),
            accepted_at: utc(accepted),
            index_url: "https://www.sec.gov/Archives/edgar/data/1/x-index.htm".into(),
        }
    }

    #[test]
    fn fixture_keeps_only_the_watchlist_202() {
        let f = parse_current_feed(FEED).unwrap();
        let got = earnings_entries(&f, &domestic_by_cik(), utc("2026-10-30T21:00:00Z"));
        // KOSS files a 2.02 but is not on the watchlist; Meta (on it) filed
        // no 2.02; the 8-K/A is not an original.
        let keys: Vec<_> = got
            .iter()
            .map(|(f, w)| (w.ticker, f.accession.as_str()))
            .collect();
        assert_eq!(keys, [("AAPL", "0000320193-26-000070")]);
    }

    #[test]
    fn item_matching_is_exact_and_amendments_are_ignored() {
        let w = domestic_by_cik();
        let t = "2026-10-30T20:00:00Z";
        let now = utc("2026-10-30T21:00:00Z");
        for (items, hit) in [
            (&["2.02"][..], true),
            (&["2.02", "9.01"][..], true),
            (&["7.01", "2.02"][..], true),
            (&["2.020"][..], false),
            (&["12.02"][..], false),
            (&["5.02"][..], false),
            (&[][..], false),
        ] {
            let e = [entry(320193, "8-K", items, t)];
            assert_eq!(
                earnings_entries(&e, &w, now).len(),
                usize::from(hit),
                "{items:?}"
            );
        }
        assert!(earnings_entries(&[entry(320193, "8-K/A", &["2.02"], t)], &w, now).is_empty());
        // A foreign (6-K) filer on the watchlist is calendar only.
        let tm = crate::earnings::find("TM").unwrap().cik;
        assert!(earnings_entries(&[entry(tm, "8-K", &["2.02"], t)], &w, now).is_empty());
    }

    #[test]
    fn age_limit() {
        let w = domestic_by_cik();
        let now = utc("2026-11-02T12:00:00Z");
        let old = [entry(320193, "8-K", &["2.02"], "2026-10-29T11:59:59Z")]; // 96h + 1s
        assert!(earnings_entries(&old, &w, now).is_empty());
        let edge = [entry(320193, "8-K", &["2.02"], "2026-10-29T12:00:00Z")]; // exactly 96h
        assert_eq!(earnings_entries(&edge, &w, now).len(), 1);
    }
}
