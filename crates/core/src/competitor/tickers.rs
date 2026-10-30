//! Tickers named in a title, e.g. `MSFT`, `(KLAC)`, `$NVDA`, `（TSM）`.
//! A bare word counts only when it is in our universe (S&P 500 ∪
//! Nasdaq-100); `$` and parenthesized forms are explicit enough without.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::premarket::universe;

/// Uppercase words that look like tickers but, in a headline, almost never
/// are. Applied to bare words and parenthesized words, not to `$` prefixes.
const STOPWORDS: &[&str] = &[
    "AI", "AM", "PM", "ET", "EST", "EDT", "PT", "GMT", "UTC", "US", "USA", "UK", "EU", "UN", "CEO",
    "CFO", "COO", "CTO", "CIO", "GDP", "CPI", "PPI", "PCE", "PMI", "ISM", "NFP", "FOMC", "FED",
    "ECB", "BOJ", "BOE", "PBOC", "IMF", "OPEC", "WTI", "ETF", "ETFS", "EPS", "IPO", "IPOS", "SEC",
    "FTC", "DOJ", "FDA", "NYSE", "NASDAQ", "AMEX", "OTC", "NYSEARCA", "DJIA", "YOY", "QOQ", "YTD",
    "MOM", "ATH", "FY", "TTM", "USD", "EUR", "JPY", "CNY", "RMB", "TWD", "HKD", "GBP", "EV", "EVS",
    "GPU", "GPUS", "CPU", "HBM", "LLM", "API", "AR", "VR", "IT", "TV", "OK", "VS", "NEW", "TOP",
    "WHY", "HOW", "WHAT", "BIG", "HOT", "LIVE", "NEWS", "ALERT", "UPDATE", "FAQ", "II", "III",
    "IV", "ESG", "ROE", "ROI", "PE", "EBIT", "GAAP", "REIT", "ADR", "AND", "THE", "FOR", "OF",
    "TO", "IN", "ON", "AT", "BY", "IS", "ARE", "WITH", "FROM", "NOT", "BUT", "OR", "INTO", "OVER",
    "AFTER", "AHEAD", "AS", "DRAM", "NAND", "CHIPS", "CHIP", "TSMC", "SOX", "SPX", "NDX", "VIX",
    // `(A)`, `(B)`: list labels and share classes far more often than tickers.
    "A", "B",
];
const EXCHANGES: &[&str] = &["NASDAQ", "NYSE", "AMEX", "NYSEARCA", "OTC"];

/// `[A-Z]{min,5}` with an optional one-letter class suffix (`BRK.B`).
fn is_symbol(tok: &str, min: usize) -> bool {
    let (base, class) = match tok.split_once('.') {
        Some((b, c)) => (b, Some(c)),
        None => (tok, None),
    };
    let upper = |s: &str| s.bytes().all(|b| b.is_ascii_uppercase());
    (min..=5).contains(&base.len()) && upper(base) && class.is_none_or(|c| c.len() == 1 && upper(c))
}

fn in_universe(tok: &str) -> bool {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| universe().into_iter().collect())
        .contains(tok)
}

fn is_open(c: Option<char>) -> bool {
    matches!(c, Some('(' | '（'))
}
fn is_close(c: Option<char>) -> bool {
    matches!(c, Some(')' | '）'))
}

/// Unique tickers in first-seen order.
pub fn tickers_in(text: &str) -> Vec<String> {
    let is_tok = |c: char| c.is_ascii_alphanumeric() || c == '.';
    // Maximal runs of [A-Za-z0-9.] (trailing dots dropped), as (start, end).
    let mut toks: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (is_tok(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                let end = s + text[s..i].trim_end_matches('.').len();
                if end > s {
                    toks.push((s, end));
                }
                start = None;
            }
            _ => {}
        }
    }
    let before = |at: usize| text[..at].chars().rev().find(|c| !c.is_whitespace());
    let after = |at: usize| text[at..].chars().find(|c| !c.is_whitespace());

    let mut out: Vec<String> = Vec::new();
    for (k, &(s, e)) in toks.iter().enumerate() {
        let tok = &text[s..e];
        let stop = STOPWORDS.contains(&tok);
        let ticker = if text[..s].ends_with('$') {
            is_symbol(tok, 1)
        } else if is_close(after(e)) {
            // `(KLAC)`, or `(NASDAQ: AAPL)` with the exchange as the previous token.
            let opened = is_open(before(s))
                || (before(s) == Some(':')
                    && k > 0
                    && EXCHANGES.contains(&&text[toks[k - 1].0..toks[k - 1].1])
                    && is_open(before(toks[k - 1].0)));
            if opened {
                is_symbol(tok, 1) && !stop
            } else {
                is_symbol(tok, 2) && !stop && in_universe(tok)
            }
        } else {
            // A bare word right before a parenthesized ticker, as in
            // `KLA (KLAC)`, is the company name.
            // Also `KLA (NASDAQ: KLAC)`. A stopword in parentheses, as in
            // `MSFT (AI)` or `AAPL (NASDAQ)`, is not a ticker, so it names nothing.
            let paren_ticker = |t: &str| is_symbol(t, 1) && !STOPWORDS.contains(&t);
            let named = is_open(after(e))
                && toks.get(k + 1).is_some_and(|&(ns, ne)| {
                    let next = &text[ns..ne];
                    let direct = is_open(before(ns)) && is_close(after(ne)) && paren_ticker(next);
                    let listed = EXCHANGES.contains(&next)
                        && after(ne) == Some(':')
                        && toks.get(k + 2).is_some_and(|&(ts, te)| {
                            is_close(after(te)) && paren_ticker(&text[ts..te])
                        });
                    direct || listed
                });
            is_symbol(tok, 2) && !stop && !named && in_universe(tok)
        };
        if ticker && !out.iter().any(|t| t == tok) {
            out.push(tok.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<String> {
        tickers_in(s)
    }

    #[test]
    fn bare_dollar_and_parens() {
        assert_eq!(t("Why MSFT and $NVDA moved"), ["MSFT", "NVDA"]);
        assert_eq!(t("KLA (KLAC) beats"), ["KLAC"]);
        assert_eq!(t("AMD (+5%) and INTC"), ["AMD", "INTC"]);
        assert_eq!(t("KLA (NASDAQ: KLAC) AND THE CHIPS"), ["KLAC"]);
        assert_eq!(t("(NASDAQ: AAPL) and (NYSE:IBM)"), ["AAPL", "IBM"]);
        assert_eq!(t("Ford ($F) and (T)"), ["F", "T"]);
    }

    #[test]
    fn cjk_text_bounds_tokens() {
        assert_eq!(t("台积电（TSM）与微软MSFT展望"), ["TSM", "MSFT"]);
        assert_eq!(t("輝達NVDA財報"), ["NVDA"]);
    }

    #[test]
    fn share_classes() {
        assert_eq!(t("BRK.B rises."), ["BRK.B"]);
        assert_eq!(t("$BRK.B and (BF.B)"), ["BRK.B", "BF.B"]);
    }

    #[test]
    fn not_tickers() {
        assert!(t("AI capex, CEO says GDP Q3 FY2027 ETF FOMC").is_empty());
        assert!(t("F").is_empty());
        assert!(t("US CPI and the FED, 8:30 AM ET").is_empty());
        assert!(t("Nvidia rallies").is_empty());
        assert!(t("($1.2B deal) (Q3) (AI)").is_empty());
        assert!(t("ABCDEF is too long").is_empty());
    }

    #[test]
    fn bare_words_must_be_in_the_universe() {
        for s in [
            "AWS",
            "HPC",
            "BTC",
            "TW",
            "(A)",
            "(B)",
            "AWS HPC BTC TW (A) (B)",
        ] {
            assert!(t(s).is_empty(), "{s}: {:?}", t(s));
        }
        assert_eq!(t("MSFT (AI) capex"), ["MSFT"]);
        assert_eq!(t("AAPL (NASDAQ)"), ["AAPL"]);
        assert_eq!(t("Apple (AAPL)"), ["AAPL"]);
        assert_eq!(t("$NVDA"), ["NVDA"]);
        // `$` and parenthesized forms need no universe entry.
        assert_eq!(t("$BTC and Taiwan Semi (TSM)"), ["BTC", "TSM"]);
    }

    #[test]
    fn duplicates_collapse() {
        assert_eq!(t("MSFT vs $MSFT (MSFT) and AMD"), ["MSFT", "AMD"]);
    }
}
