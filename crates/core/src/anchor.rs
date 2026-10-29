//! Proves an LLM-proposed reason is anchored in the fetched article text.

use crate::facts::{quote_numbers, NewsReason};
use crate::numbers::{check, Lang};

const MIN_QUOTE_CHARS: usize = 20;

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Decode entities, unify curly quotes, collapse whitespace, trim.
fn normalize(s: &str) -> String {
    let mut decoded = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        decoded.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let hit = tail
            .find(';')
            .filter(|&e| e <= 10)
            .and_then(|e| decode_entity(&tail[..e]).map(|c| (c, e)));
        match hit {
            Some((c, e)) => {
                decoded.push(c);
                rest = &tail[e + 1..];
            }
            None => {
                decoded.push('&');
                rest = tail;
            }
        }
    }
    decoded.push_str(rest);
    let mapped: String = decoded
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            c => c,
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True if `quote` (>= 20 chars) appears verbatim in `source_text` after normalization.
pub fn verify_quote(source_text: &str, quote: &str) -> bool {
    let q = normalize(quote);
    q.chars().count() >= MIN_QUOTE_CHARS && normalize(source_text).contains(&q)
}

/// Quote is anchored in the source AND the summary only uses numbers in the quote.
pub fn verify_reason(source_text: &str, r: &NewsReason) -> bool {
    verify_quote(source_text, &r.quote)
        && check(&r.summary_en, Lang::En, &quote_numbers(&r.quote)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(quote: &str, summary: &str) -> NewsReason {
        NewsReason { url: "u".into(), title: "t".into(), quote: quote.into(), summary_en: summary.into() }
    }

    #[test]
    fn whitespace_and_amp() {
        let src = "Profits at AT&amp;T\n  rose   sharply this quarter.";
        assert!(verify_quote(src, "Profits at AT&T rose sharply this quarter."));
    }
    #[test]
    fn short_quote_rejected() {
        let q = "a".repeat(19);
        assert!(!verify_quote(&q, &q));
        let q = "a".repeat(20);
        assert!(verify_quote(&q, &q));
    }
    #[test]
    fn absent_quote() {
        assert!(!verify_quote("Shares rose after earnings beat estimates.", "Shares fell after earnings missed estimates."));
    }
    #[test]
    fn curly_quotes() {
        let src = "The CEO said \"we're very confident\" on the call.";
        assert!(verify_quote(src, "The CEO said \u{201c}we\u{2019}re very confident\u{201d} on the call."));
    }
    #[test]
    fn numeric_entity() {
        assert!(verify_quote("The firm&#39;s revenue grew strongly in Q3.", "The firm's revenue grew strongly in Q3."));
        assert!(verify_quote("The firm&#x27;s revenue grew strongly in Q3.", "The firm's revenue grew strongly in Q3."));
    }
    #[test]
    fn summary_number_mismatch() {
        let q = "Shares jumped 10% after the earnings report.";
        assert!(!verify_reason(q, &reason(q, "Stock rose 12% on earnings.")));
        assert!(verify_reason(q, &reason(q, "Stock rose 10% on earnings.")));
    }
}
