//! The ticker universe: S&P 500 ∪ Nasdaq-100, from `data/universe.txt`,
//! with each ticker's GICS sector and industry group.

use std::collections::HashMap;
use std::sync::OnceLock;

const DATA: &str = include_str!("../../data/universe.txt");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    name: &'static str,
    sector: Option<&'static str>,
    group: Option<&'static str>,
}

struct Index {
    order: Vec<&'static str>,
    by_ticker: HashMap<&'static str, Entry>,
    groups: Vec<&'static str>,
}

/// One data line: `TICKER[<TAB>Name[<TAB>Sector[<TAB>Group]]]`. Missing or
/// empty columns are `""` (name) or `None`.
fn parse_line(line: &'static str) -> Option<(&'static str, Entry)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut cols = line.split('\t').map(str::trim);
    let ticker = cols.next()?;
    let name = cols.next().unwrap_or("");
    let mut opt = || cols.next().filter(|c| !c.is_empty());
    let sector = opt();
    let group = opt();
    Some((
        ticker,
        Entry {
            name,
            sector,
            group,
        },
    ))
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut order = Vec::new();
        let mut by_ticker = HashMap::new();
        for (t, e) in DATA.lines().filter_map(parse_line) {
            order.push(t);
            by_ticker.insert(t, e);
        }
        let mut groups: Vec<&'static str> = by_ticker.values().filter_map(|e| e.group).collect();
        groups.sort_unstable();
        groups.dedup();
        Index {
            order,
            by_ticker,
            groups,
        }
    })
}

/// Every universe ticker, in file order (display form, e.g. `BRK.B`).
pub fn universe() -> Vec<String> {
    index().order.iter().map(|t| t.to_string()).collect()
}

/// Company name for a universe ticker.
pub fn company_name(ticker: &str) -> Option<&'static str> {
    index()
        .by_ticker
        .get(ticker)
        .map(|e| e.name)
        .filter(|n| !n.is_empty())
}

/// GICS sector for a universe ticker.
pub fn sector(ticker: &str) -> Option<&'static str> {
    index().by_ticker.get(ticker).and_then(|e| e.sector)
}

/// Whether `ticker` (display form, e.g. `BRK.B`) is in the universe.
pub fn in_universe(ticker: &str) -> bool {
    index().by_ticker.contains_key(ticker)
}

/// Industry group (e.g. "Memory & storage") for a universe ticker.
pub fn group(ticker: &str) -> Option<&'static str> {
    index().by_ticker.get(ticker).and_then(|e| e.group)
}

/// Every distinct industry group, sorted.
pub fn groups() -> impl Iterator<Item = &'static str> {
    index().groups.iter().copied()
}

/// Yahoo spells class shares with a dash: `BRK.B` -> `BRK-B`.
pub fn yahoo_symbol(t: &str) -> String {
    t.replace('.', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::premarket::group_zh;

    #[test]
    fn yahoo_symbol_uses_dash() {
        assert_eq!(yahoo_symbol("BRK.B"), "BRK-B");
        assert_eq!(yahoo_symbol("AAPL"), "AAPL");
    }

    #[test]
    fn universe_is_large_and_unique() {
        let u = universe();
        assert!(u.len() > 500, "{}", u.len());
        let mut d = u.clone();
        d.sort();
        d.dedup();
        assert_eq!(d.len(), u.len());
        assert!(u.contains(&"BRK.B".to_string()) && u.contains(&"ASML".to_string()));
        assert!(in_universe("BRK.B") && !in_universe("BRK-B") && !in_universe("NWRB"));
    }

    #[test]
    fn company_names() {
        assert_eq!(company_name("AAPL"), Some("Apple"));
        assert_eq!(company_name("ZZZZ"), None);
    }

    #[test]
    fn universe_contains_recent_additions() {
        let u = universe();
        for t in ["SNDK", "VRT", "CRWV"] {
            assert!(u.contains(&t.to_string()), "{t} missing from the universe");
        }
        assert_eq!(company_name("SNDK"), Some("Sandisk"));
    }

    #[test]
    fn universe_is_at_most_550() {
        let n = universe().len();
        assert!(n <= 550, "{n}");
    }

    const GICS: [&str; 11] = [
        "Communication Services",
        "Consumer Discretionary",
        "Consumer Staples",
        "Energy",
        "Financials",
        "Health Care",
        "Industrials",
        "Information Technology",
        "Materials",
        "Real Estate",
        "Utilities",
    ];

    #[test]
    fn every_ticker_has_sector_and_group() {
        for t in universe() {
            assert!(sector(&t).is_some(), "{t} has no sector");
            assert!(group(&t).is_some(), "{t} has no group");
        }
        assert_eq!(sector("ZZZZ"), None);
        assert_eq!(group("ZZZZ"), None);
    }

    #[test]
    fn sectors_are_the_gics_eleven() {
        let got: std::collections::BTreeSet<&str> =
            universe().iter().filter_map(|t| sector(t)).collect();
        let want: std::collections::BTreeSet<&str> = GICS.into_iter().collect();
        assert_eq!(got, want);
    }

    #[test]
    fn group_count_in_range() {
        let n = groups().count();
        assert!((25..=60).contains(&n), "{n} groups");
        let mut g: Vec<&str> = groups().collect();
        g.sort_unstable();
        g.dedup();
        assert_eq!(g.len(), n, "groups() repeats a label");
    }

    #[test]
    fn no_singleton_groups() {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for t in universe() {
            *count.entry(group(&t).unwrap_or("")).or_default() += 1;
        }
        for g in groups() {
            let n = count.get(g).copied().unwrap_or(0);
            assert!(n >= 2, "{g} has {n} member(s)");
        }
    }

    #[test]
    fn group_zh_has_a_label_for_every_group() {
        for g in groups() {
            let zh = group_zh(g);
            assert!(!zh.is_empty() && zh != g, "{g} has no Chinese label");
            assert!(
                zh.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{g}: {zh}"
            );
        }
        assert_eq!(group_zh("Not a group"), "Not a group");
    }

    #[test]
    fn lookups_use_the_map_once() {
        let _ = group("AAPL");
        let start = std::time::Instant::now();
        for i in 0..10_000 {
            let t = if i % 2 == 0 { "TRI" } else { "ZZZZ" };
            std::hint::black_box(group(std::hint::black_box(t)));
        }
        let ms = start.elapsed().as_millis();
        assert!(ms < 50, "10k lookups took {ms} ms");
    }

    #[test]
    fn known_groups() {
        assert_eq!(group("MU"), Some("Memory & storage"));
        assert_eq!(group("NVDA"), Some("Semiconductors"));
        assert_eq!(group("ZION"), None);
        assert_eq!(group("RKLB"), Some("Aerospace & defense"));
        assert_eq!(group("SPCX"), Some("Aerospace & defense"));
        assert_eq!(group("URI"), Some("Industrial distributors & rental"));
        assert_eq!(group("FER"), Some("Electrical & grid infrastructure"));
        assert!(groups().all(|g| g != "Space & launch"));
        assert_eq!(sector("GOOGL"), Some("Communication Services"));
        assert_eq!(sector("V"), Some("Financials"));
        assert_eq!(company_name("HON"), Some("Honeywell Technologies"));
    }

    #[test]
    fn parser_accepts_two_three_or_four_columns() {
        let (t, e) = parse_line("AAPL\tApple").unwrap();
        assert_eq!(
            (t, e.name, e.sector, e.group),
            ("AAPL", "Apple", None, None)
        );
        let (_, e) = parse_line("AAPL\tApple\tInformation Technology").unwrap();
        assert_eq!((e.sector, e.group), (Some("Information Technology"), None));
        let (_, e) =
            parse_line("AAPL\tApple\tInformation Technology\tMega-cap tech platforms").unwrap();
        assert_eq!(e.group, Some("Mega-cap tech platforms"));
        let (t, e) = parse_line("ZZZZ").unwrap();
        assert_eq!((t, e.name), ("ZZZZ", ""));
        assert!(parse_line("# comment").is_none() && parse_line("  ").is_none());
    }

    #[test]
    fn universe_contains_renamed_tickers() {
        let u = universe();
        for t in ["BNY", "FISV", "MRSH", "SKYD", "VMRK", "ECHO"] {
            assert!(u.contains(&t.to_string()), "{t} missing from the universe");
        }
    }
}
