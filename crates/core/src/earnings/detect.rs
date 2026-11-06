//! Which current-filings feed entries are new earnings releases of the
//! watchlist.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

use crate::earnings::{watchlist, Filer, WatchEntry};
use crate::edgar::FeedEntry;

/// A 2.02 8-K accepted longer ago than this is history, not news. Long
/// enough to pick up a Saturday filing on Monday morning.
pub const MAX_AGE_HOURS: i64 = 96;

/// The domestic (8-K filing) watchlist companies by CIK, both tiers.
pub fn domestic_by_cik() -> HashMap<u64, WatchEntry> {
    watchlist()
        .into_iter()
        .filter(|e| e.filer == Filer::Domestic)
        .map(|e| (e.cik, e))
        .collect()
}

/// Words that do not identify a company.
const NAME_NOISE: [&str; 19] = [
    "inc",
    "incorporated",
    "corp",
    "corporation",
    "co",
    "company",
    "companies",
    "ltd",
    "limited",
    "plc",
    "holdings",
    "holding",
    "the",
    "group",
    "sa",
    "nv",
    "ag",
    "lp",
    "llc",
];

/// Lowercase tokens of a company name: "&", ".", and apostrophes are dropped
/// inside words ("AT&T" = "ATT", "Lowe's" = "LOWES", "Amazon.com" =
/// "AMAZON COM"), other punctuation separates, and legal suffixes go.
fn name_tokens(name: &str) -> Vec<String> {
    let joined: String = name
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '&' | '.' | '\'' | '\u{2019}'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    joined
        .split_whitespace()
        .filter(|t| !NAME_NOISE.contains(t))
        .map(String::from)
        .collect()
}

/// The filer's name is the row's SEC conformed name: the same significant
/// words, ignoring case, punctuation, spacing and legal suffixes. Matching
/// only the first word would, at about 560 rows, let a wrong CIK through
/// between names that share it (American Express and American Tower). A
/// company renamed on EDGAR fails until the watchlist is regenerated
/// (`--refresh`), and the poller alerts about it.
pub fn same_filer(filer: &str, sec: &str) -> bool {
    let (a, b) = (name_tokens(filer).concat(), name_tokens(sec).concat());
    !a.is_empty() && a == b
}

/// The watchlist entry for a feed entry, unless the filer's name is not
/// the row's SEC name (a wrong CIK in the data file).
fn watched<'w>(f: &FeedEntry, watch: &'w HashMap<u64, WatchEntry>) -> Option<&'w WatchEntry> {
    watch.get(&f.cik).filter(|w| same_filer(&f.company, w.sec))
}

/// Watchlist rows whose CIK files under another company's name, as
/// `(ticker, message)`, one per ticker.
pub fn name_mismatches(
    entries: &[FeedEntry],
    watch: &HashMap<u64, WatchEntry>,
) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for f in entries {
        let Some(w) = watch.get(&f.cik) else { continue };
        if watched(f, watch).is_none() && !out.iter().any(|(t, _)| *t == w.ticker) {
            out.push((
                w.ticker,
                format!(
                    "watchlist {} has CIK {}, but EDGAR files it as {:?}; its filings are skipped until data/watchlist.tsv is fixed",
                    w.ticker, f.cik, f.company
                ),
            ));
        }
    }
    out
}

/// Original 8-K filings (not `8-K/A`) with Item 2.02 by a domestic
/// watchlist company (its filer name matching the row), accepted within
/// `MAX_AGE_HOURS`, with their entry.
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
        .filter_map(|f| Some((f, watched(f, watch)?.clone())))
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
            company: "Apple Inc.".into(),
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
    fn filer_names_match_across_spelling() {
        for (filer, sec) in [
            ("APPLE INC", "Apple Inc."),
            ("META PLATFORMS, INC.", "Meta Platforms, Inc."),
            ("EXXON MOBIL HOLDINGS CORP", "ExxonMobil Holdings Corp"),
            ("AT&T INC.", "AT&T INC."),
            ("COCA COLA CO", "COCA COLA CO"),
            ("LOWES COMPANIES INC", "LOWE'S COMPANIES INC"),
            ("The Walt Disney Company", "Walt Disney Co"),
        ] {
            assert!(same_filer(filer, sec), "{filer} vs {sec}");
        }
        for (filer, sec) in [
            ("Estrella Immunopharma, Inc.", "Apple Inc."),
            ("Apple Hospitality REIT, Inc.", "Apple Inc."),
            ("COCA COLA CONSOLIDATED, INC.", "COCA COLA CO"),
            ("INC", "INC"),
        ] {
            assert!(!same_filer(filer, sec), "{filer} vs {sec}");
        }
    }

    #[test]
    fn feed_names_match_the_sec_name() {
        // Feed spellings of the conformed names in watchlist.tsv.
        for (ticker, feed) in [
            ("AAPL", "Apple Inc."),
            ("AAPL", "APPLE INC"),
            ("BNY", "Bank of New York Mellon Corp"),
            ("BRK.B", "BERKSHIRE HATHAWAY INC"),
            ("GOOGL", "Alphabet Inc."),
            ("XOM", "ExxonMobil Holdings Corp"),
            ("XOM", "EXXON MOBIL HOLDINGS CORP"),
            ("USB", "US BANCORP \\DE\\"),
            ("KO", "COCA COLA CO"),
        ] {
            let w = crate::earnings::find(ticker).unwrap();
            assert!(same_filer(feed, w.sec), "{ticker}: {feed} vs {}", w.sec);
        }
        for (ticker, feed) in [
            ("AAPL", "Apple Hospitality REIT, Inc."),
            ("AXP", "AMERICAN TOWER CORP /MA/"),
            ("GM", "GENERAL MILLS INC"),
            ("AAPL", "INC"),
        ] {
            let w = crate::earnings::find(ticker).unwrap();
            assert!(!same_filer(feed, w.sec), "{ticker}: {feed} vs {}", w.sec);
        }
    }

    #[test]
    fn no_two_rows_share_an_sec_name() {
        let w = crate::earnings::watchlist();
        let mut seen = HashMap::new();
        for e in &w {
            let key = name_tokens(e.sec).concat();
            assert!(!key.is_empty(), "{}", e.ticker);
            if let Some(other) = seen.insert(key, e.ticker) {
                panic!("{other} and {} have the same SEC name", e.ticker);
            }
        }
    }

    #[test]
    fn a_wrong_cik_is_not_assigned_to_the_ticker() {
        let mut w = domestic_by_cik();
        // Suppose the data file gave AAPL's CIK to another ticker.
        let mut wrong = w.remove(&320193).unwrap();
        wrong.ticker = "INTU";
        wrong.name = "Intuit";
        wrong.sec = "INTUIT INC.";
        w.insert(320193, wrong);
        let f = parse_current_feed(FEED).unwrap();
        let now = utc("2026-10-30T21:00:00Z");
        assert!(earnings_entries(&f, &w, now).is_empty());
        let bad = name_mismatches(&f, &w);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, "INTU");
        assert!(bad[0].1.contains("Apple Inc."), "{}", bad[0].1);
        assert!(name_mismatches(&f, &domestic_by_cik()).is_empty());
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
