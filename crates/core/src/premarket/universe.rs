//! The ticker universe: S&P 500 ∪ Nasdaq-100, from `data/universe.txt`.

const DATA: &str = include_str!("../../data/universe.txt");

fn entries() -> impl Iterator<Item = (&'static str, &'static str)> {
    DATA.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once('\t') {
            Some((t, n)) => (t.trim(), n.trim()),
            None => (l, ""),
        })
}

/// Every universe ticker, in file order (display form, e.g. `BRK.B`).
pub fn universe() -> Vec<String> {
    entries().map(|(t, _)| t.to_string()).collect()
}

/// Company name for a universe ticker.
pub fn company_name(ticker: &str) -> Option<&'static str> {
    entries()
        .find(|(t, _)| *t == ticker)
        .map(|(_, n)| n)
        .filter(|n| !n.is_empty())
}

/// Yahoo spells class shares with a dash: `BRK.B` -> `BRK-B`.
pub fn yahoo_symbol(t: &str) -> String {
    t.replace('.', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn company_names() {
        assert_eq!(company_name("AAPL"), Some("Apple"));
        assert_eq!(company_name("ZZZZ"), None);
    }
}
