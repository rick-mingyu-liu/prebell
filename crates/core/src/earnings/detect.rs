//! Which filings in a submissions document are new earnings releases.

use chrono::{DateTime, Duration, Utc};

use crate::earnings::WatchEntry;
use crate::edgar::{RecentFiling, Submissions};

/// A 2.02 8-K accepted longer ago than this is history, not news. Long
/// enough to pick up a Saturday filing on Monday morning.
pub const MAX_AGE_HOURS: i64 = 96;

/// Original 8-K filings with Item 2.02, accepted within `MAX_AGE_HOURS`.
pub fn earnings_filings(sub: &Submissions, now: DateTime<Utc>) -> Vec<&RecentFiling> {
    sub.recent
        .iter()
        .filter(|f| f.form == "8-K")
        .filter(|f| f.items.split(',').any(|i| i.trim() == "2.02"))
        .filter(|f| now - f.accepted_at <= Duration::hours(MAX_AGE_HOURS))
        .collect()
}

/// The submissions document lists the watchlist ticker (`BRK.B` = `BRK-B`),
/// guarding against a wrong CIK in the data file.
pub fn ticker_matches(sub: &Submissions, entry: &WatchEntry) -> bool {
    let want = entry.ticker.replace('.', "-");
    sub.tickers
        .iter()
        .any(|t| t.replace('.', "-").eq_ignore_ascii_case(&want))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::find;
    use crate::edgar::{parse_submissions, tests::SUBMISSIONS};

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    fn filing(form: &str, items: &str, accepted: &str) -> RecentFiling {
        RecentFiling {
            accession: format!("0000000001-26-{:06}", items.len() + form.len()),
            form: form.into(),
            items: items.into(),
            filing_date: "2026-10-30".into(),
            report_date: "2026-10-30".into(),
            accepted_at: utc(accepted),
            primary_doc: "x.htm".into(),
        }
    }
    fn sub(recent: Vec<RecentFiling>) -> Submissions {
        Submissions {
            cik: 1,
            name: "X".into(),
            tickers: vec!["BRK-B".into(), "BRK-A".into()],
            recent,
        }
    }

    #[test]
    fn fixture_detects_only_the_new_original_202() {
        let s = parse_submissions(SUBMISSIONS).unwrap();
        let got = earnings_filings(&s, utc("2026-10-30T21:00:00Z"));
        let accs: Vec<_> = got.iter().map(|f| f.accession.as_str()).collect();
        // 8-K/A (…069), 5.07 and 8.01 8-Ks, Form 4, 10-Q and the July 2.02 are ignored.
        assert_eq!(accs, vec!["0000320193-26-000070"]);
    }

    #[test]
    fn item_matching_is_exact() {
        let t = "2026-10-30T20:00:00Z";
        let now = utc("2026-10-30T21:00:00Z");
        for (items, hit) in [
            ("2.02", true),
            ("2.02,9.01", true),
            ("7.01, 2.02", true),
            ("2.020", false),
            ("12.02", false),
            ("5.02", false),
            ("", false),
        ] {
            let s = sub(vec![filing("8-K", items, t)]);
            assert_eq!(
                earnings_filings(&s, now).len(),
                usize::from(hit),
                "{items:?}"
            );
        }
        let s = sub(vec![filing("8-K/A", "2.02", t)]);
        assert!(earnings_filings(&s, now).is_empty());
    }

    #[test]
    fn age_limit() {
        let now = utc("2026-11-02T12:00:00Z");
        let s = sub(vec![filing("8-K", "2.02", "2026-10-29T11:59:59Z")]); // 96h + 1s
        assert!(earnings_filings(&s, now).is_empty());
        let s = sub(vec![filing("8-K", "2.02", "2026-10-29T12:00:00Z")]); // exactly 96h
        assert_eq!(earnings_filings(&s, now).len(), 1);
    }

    #[test]
    fn ticker_guard() {
        let s = sub(vec![]);
        assert!(ticker_matches(&s, &find("BRK.B").unwrap()));
        assert!(!ticker_matches(&s, &find("AAPL").unwrap()));
        let apple = parse_submissions(SUBMISSIONS).unwrap();
        assert!(ticker_matches(&apple, &find("AAPL").unwrap()));
    }
}
