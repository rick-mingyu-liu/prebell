//! The offering's facts from a filing's cover page, by code patterns (no
//! model): exchange and symbol, shares offered and their split, the
//! underwriters' option, the price range or the final price, the cover and
//! pricing dates, and a first trading day when the filing states one. Each
//! fact keeps the cover's own words; a fact whose words are not in the
//! document, or whose number is not in its words, is dropped.
//!
//! The text is the plain text of the document's first bytes
//! (`edgar::head_text`), one line, so a sentence is found by its
//! punctuation.

use std::sync::OnceLock;

use chrono::NaiveDate;
use regex::{Captures, Regex};

use crate::anchor::{normalize, verify_quote};
use crate::facts::quote_numbers;

/// Bytes of the primary document read: Anthropic's S-1/A cover ends near
/// byte 85,000 (its underwriters' option sentence) and its
/// "Prospectus dated" line is at 114,000; the full document is 14.8 MB.
pub const COVER_PREFIX_BYTES: usize = 160 * 1024;
/// A sentence longer than this is not a cover sentence; the match alone is kept.
const MAX_SENTENCE_CHARS: usize = 600;

/// One cover fact.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverFact {
    pub kind: &'static str,
    pub num: Option<f64>,
    pub text: Option<String>,
    pub quote: String,
}

const MONTH: &str =
    "(January|February|March|April|May|June|July|August|September|October|November|December)";
const EXCHANGES: &str = "Nasdaq Global Select Market|Nasdaq Global Market|Nasdaq Capital Market|Nasdaq Stock Market|New York Stock Exchange|NYSE American|NYSE Arca|NYSE Texas|NYSE|Cboe BZX Exchange";
const SHARES_NUM: &str = r"(\d{1,3}(?:,\d{3})+)";

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
}

struct Patterns {
    cover_date: Regex,
    prospectus_date: Regex,
    total: Regex,
    company: Regex,
    selling: Regex,
    option: Regex,
    range: Regex,
    final_price: Regex,
    listing: Regex,
    trading: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        cover_date: re(&format!(
            r"Subject to Completion[.,]?\s+(?:Preliminary Prospectus\s+)?[Dd]ated\s+{MONTH}\s+(\d{{1,2}}),\s+(\d{{4}})\.?"
        )),
        prospectus_date: re(&format!(
            r"(?:Prospectus dated|PROSPECTUS DATED|The date of this prospectus is)\s+{MONTH}\s+(\d{{1,2}}),\s+(\d{{4}})"
        )),
        total: re(&format!(
            r"(?:Class [A-Z] )?(?:Common Stock|Ordinary Shares|Common Shares)\s+{SHARES_NUM}\s+Shares\b|\b{SHARES_NUM}\s+Shares\s+(?:of\s+)?(?:Class [A-Z]\s+)?(?:Common Stock|Ordinary Shares|Common Shares)|\b{SHARES_NUM}\s+American Depositary Shares\b"
        )),
        company: re(&format!(
            r"\b(?:We are|is) (?:offering|selling) {SHARES_NUM} (?:shares|American Depositary Shares|ADSs)\b"
        )),
        selling: re(&format!(
            r"selling (?:stockholders|shareholders)\b[^.]{{0,200}}?\b(?:are )?(?:offering|selling) (?:an additional |a total of )?{SHARES_NUM} (?:shares|American Depositary Shares|ADSs)\b"
        )),
        option: re(&format!(
            r"option to purchase (?:up to )?(?:an additional |a maximum of )?{SHARES_NUM} (?:additional )?(?:shares|American Depositary Shares|ADSs)\b"
        )),
        range: re(r"between \$\s?(\d{1,4}(?:\.\d{2})?) and \$\s?(\d{1,4}(?:\.\d{2})?)"),
        final_price: re(
            r"(?i)\binitial public offering price(?: per (?:share|ADS))? (?:is|of) \$\s?(\d{1,4}(?:\.\d{2})?)",
        ),
        listing: re(&format!(
            r"\b(?:list|listing|listed)\b[^.]{{0,160}}? on (?:the |The )?({EXCHANGES})(?:\s*\(the “[A-Za-z]+”\)|\s*\(“[A-Za-z]+”\))?,?\s+under the (?:ticker |trading )?symbol\s+[“\x22]([A-Z]{{1,5}}(?:\.[A-Z])?)\.?[”\x22]"
        )),
        trading: re(&format!(
            r"(?:expected to|will) (?:begin|commence) trading\b[^.]{{0,160}}? on {MONTH} (\d{{1,2}}), (\d{{4}})"
        )),
    })
}

fn month_num(name: &str) -> Option<u32> {
    const M: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let n = name.to_ascii_lowercase();
    M.iter().position(|m| *m == n).map(|i| i as u32 + 1)
}

/// `YYYY-MM-DD` from (month name, day, year) captures at `i`, `i+1`, `i+2`.
fn date_at(c: &Captures, i: usize) -> Option<String> {
    let m = month_num(c.get(i)?.as_str())?;
    let d: u32 = c.get(i + 1)?.as_str().parse().ok()?;
    let y: i32 = c.get(i + 2)?.as_str().parse().ok()?;
    NaiveDate::from_ymd_opt(y, m, d).map(|d| d.format("%Y-%m-%d").to_string())
}

fn num(s: &str) -> Option<f64> {
    s.replace(',', "").parse().ok()
}

/// Words that never end a sentence before their period.
const ABBREV: [&str; 16] = [
    "Inc", "Corp", "Co", "Ltd", "Jr", "Sr", "Mr", "Ms", "Mrs", "Dr", "St", "No", "Nos", "vs",
    "approx", "Messrs",
];

/// Whether the character at byte `i` ends a sentence: `!`, `?`, or a period
/// that is not a decimal point, not after an abbreviation and not after a
/// single capital (U.S.).
fn ends_at(text: &str, i: usize) -> bool {
    match text[i..].chars().next() {
        Some('!' | '?') => true,
        Some('.') => {
            if text[i + 1..].starts_with(|c: char| c.is_ascii_digit()) {
                return false;
            }
            let word: String = text[..i]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let single_cap = word.len() == 1 && word.chars().all(|c| c.is_ascii_uppercase());
            !(single_cap || ABBREV.contains(&word.as_str()))
        }
        _ => false,
    }
}

/// The sentence holding byte `from` (a match's start): from just after the
/// previous sentence end to the first end at or after `from` (a symbol
/// match ends with its sentence, "“ANT.”"), with the closing quotes after
/// its period. `None` when it runs past `MAX_SENTENCE_CHARS`.
fn sentence(text: &str, from: usize) -> Option<String> {
    let start = text[..from]
        .char_indices()
        .rev()
        .find(|&(i, _)| ends_at(text, i))
        .map_or(0, |(i, c)| i + c.len_utf8());
    // The previous sentence's closing quotes belong to it.
    let rest = &text[start..];
    let start = start + rest.len()
        - rest
            .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '”' | '"' | '’' | ')'))
            .len();
    let mut end = text[from..]
        .char_indices()
        .find(|&(i, _)| ends_at(text, from + i))
        .map_or(text.len(), |(i, c)| from + i + c.len_utf8());
    while let Some(c) = text[end..]
        .chars()
        .next()
        .filter(|c| matches!(c, '”' | '"' | '’' | ')'))
    {
        end += c.len_utf8();
    }
    let s = text[start..end].trim();
    // "...between $70.00 and $78.00 ." reads "... and $78.00".
    let s = s.strip_suffix(" .").unwrap_or(s);
    (s.chars().count() <= MAX_SENTENCE_CHARS).then(|| s.to_string())
}

/// The sentence holding capture group 0, or the match itself when the
/// sentence is too long.
fn quote_of(text: &str, c: &Captures) -> String {
    let m = c.get(0).expect("group 0");
    sentence(text, m.start()).unwrap_or_else(|| m.as_str().trim().to_string())
}

fn fact_num(kind: &'static str, v: f64, quote: String) -> CoverFact {
    CoverFact {
        kind,
        num: Some(v),
        text: None,
        quote,
    }
}

fn fact_text(kind: &'static str, v: String, quote: String) -> CoverFact {
    CoverFact {
        kind,
        num: None,
        text: Some(v),
        quote,
    }
}

/// The fact's words are in the document and carry its value: the number
/// among the quote's numbers, the text or the date (as the cover writes
/// it) in the quote.
fn verified(text: &str, f: &CoverFact) -> bool {
    if !verify_quote(text, &f.quote) {
        return false;
    }
    match (&f.num, &f.text) {
        (Some(v), _) => quote_numbers(&f.quote)
            .iter()
            .any(|n| (n - v).abs() <= 1e-9 * v.abs().max(1.0)),
        (None, Some(t)) => match NaiveDate::parse_from_str(t, "%Y-%m-%d") {
            Ok(d) => normalize(&f.quote).contains(&d.format("%B %-d, %Y").to_string()),
            Err(_) => f.quote.contains(t.as_str()),
        },
        (None, None) => false,
    }
}

/// The cover facts of `text` (a filing of `form`), each verified. Price
/// range and dates other than the final prospectus's are read from
/// registration statements; the final price and the pricing date only from
/// a final prospectus (a preliminary prospectus is also "dated").
pub fn extract(text: &str, form: &str) -> Vec<CoverFact> {
    let p = patterns();
    let is_final = super::is_final(form);
    let mut out: Vec<CoverFact> = Vec::new();

    if let Some(c) = p.cover_date.captures(text) {
        if let Some(d) = date_at(&c, 1) {
            out.push(fact_text("cover_date", d, c[0].trim().to_string()));
        }
    }
    if is_final {
        if let Some(c) = p.prospectus_date.captures(text) {
            if let Some(d) = date_at(&c, 1) {
                out.push(fact_text("prospectus_date", d, c[0].trim().to_string()));
            }
        }
    }
    let total = p.total.captures(text).and_then(|c| {
        let v = (1..=3)
            .find_map(|i| c.get(i))
            .and_then(|m| num(m.as_str()))?;
        Some(fact_num("shares_offered", v, c[0].trim().to_string()))
    });
    let company = p
        .company
        .captures(text)
        .and_then(|c| Some(fact_num("shares_company", num(&c[1])?, quote_of(text, &c))));
    let selling = p
        .selling
        .captures(text)
        .and_then(|c| Some(fact_num("shares_selling", num(&c[1])?, quote_of(text, &c))));
    // The split must add up to the cover's total; otherwise a sentence was
    // misread and neither part is kept.
    let t = total.as_ref().and_then(|f| f.num);
    let (cn, sn) = (
        company.as_ref().and_then(|f| f.num),
        selling.as_ref().and_then(|f| f.num),
    );
    let split_ok = match (t, cn, sn) {
        (Some(t), Some(c), Some(s)) => (c + s - t).abs() < 0.5,
        (Some(t), Some(c), None) => (c - t).abs() < 0.5,
        (Some(t), None, Some(s)) => (s - t).abs() < 0.5,
        _ => false,
    };
    out.extend(total);
    if split_ok {
        out.extend(company);
        out.extend(selling);
    }
    if let Some(c) = p.option.captures(text) {
        if let Some(v) = num(&c[1]) {
            out.push(fact_num("option_shares", v, quote_of(text, &c)));
        }
    }
    if !is_final {
        if let Some(c) = p
            .range
            .captures_iter(text)
            .find(|c| quote_of(text, c).contains("offering price"))
        {
            if let (Some(lo), Some(hi)) = (num(&c[1]), num(&c[2])) {
                if lo > 0.0 && lo < hi {
                    let q = quote_of(text, &c);
                    out.push(fact_num("price_low", lo, q.clone()));
                    out.push(fact_num("price_high", hi, q));
                }
            }
        }
    } else if let Some(c) = p.final_price.captures_iter(text).find(|c| {
        let q = quote_of(text, c).to_ascii_lowercase();
        !q.contains("estimated") && !q.contains("between")
    }) {
        if let Some(v) = num(&c[1]).filter(|v| *v > 0.0) {
            out.push(fact_num("price_final", v, quote_of(text, &c)));
        }
    }
    if let Some(c) = p.listing.captures(text) {
        let q = quote_of(text, &c);
        out.push(fact_text("exchange", c[1].to_string(), q.clone()));
        out.push(fact_text("symbol", c[2].to_string(), q));
    }
    if let Some(c) = p.trading.captures(text) {
        if let Some(d) = date_at(&c, 1) {
            out.push(fact_text("trading_date", d, quote_of(text, &c)));
        }
    }
    out.retain(|f| verified(text, f));
    out
}

/// The fact of `kind`, if read.
pub fn find<'a>(facts: &'a [CoverFact], kind: &str) -> Option<&'a CoverFact> {
    facts.iter().find(|f| f.kind == kind)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::edgar::head_text;

    pub(crate) const S1A: &str =
        include_str!("../../tests/fixtures/edgar_ipo_s1a_anthropic_head.htm");
    pub(crate) const B4: &str = include_str!("../../tests/fixtures/edgar_ipo_424b4_madeup.htm");

    /// The cover as the check reads it: the first `COVER_PREFIX_BYTES`.
    pub(crate) fn cover_text(html: &str) -> String {
        let mut end = html.len().min(COVER_PREFIX_BYTES);
        while !html.is_char_boundary(end) {
            end -= 1;
        }
        head_text(&html[..end])
    }

    fn get<'a>(f: &'a [CoverFact], kind: &str) -> &'a CoverFact {
        find(f, kind).unwrap_or_else(|| panic!("no {kind}: {f:#?}"))
    }

    #[test]
    fn anthropic_s1a_cover_facts_with_their_sentences() {
        let text = cover_text(S1A);
        let f = extract(&text, "S-1/A");
        let e = get(&f, "exchange");
        assert_eq!(e.text.as_deref(), Some("Nasdaq Global Select Market"));
        assert_eq!(
            e.quote,
            "We have applied to list our Class A common stock on the Nasdaq Global Select Market under the symbol “ANT.”"
        );
        assert_eq!(get(&f, "symbol").text.as_deref(), Some("ANT"));
        let t = get(&f, "shares_offered");
        assert_eq!(t.num, Some(825_000_000.0));
        assert_eq!(t.quote, "Class A Common Stock 825,000,000 Shares");
        let split = "We are selling 801,351,354 shares of Class A common stock and the selling stockholders identified in this prospectus are selling an additional 23,648,646 shares of Class A common stock.";
        let c = get(&f, "shares_company");
        assert_eq!((c.num, c.quote.as_str()), (Some(801_351_354.0), split));
        let s = get(&f, "shares_selling");
        assert_eq!((s.num, s.quote.as_str()), (Some(23_648_646.0), split));
        let range = "It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00.";
        let lo = get(&f, "price_low");
        let hi = get(&f, "price_high");
        assert_eq!((lo.num, lo.quote.as_str()), (Some(70.0), range));
        assert_eq!((hi.num, hi.quote.as_str()), (Some(78.0), range));
        let d = get(&f, "cover_date");
        assert_eq!(d.text.as_deref(), Some("2026-11-10"));
        assert_eq!(d.quote, "Subject to Completion. Dated November 10, 2026.");
        let o = get(&f, "option_shares");
        assert_eq!(o.num, Some(123_750_000.0));
        assert!(
            o.quote.starts_with(
                "To the extent that the underwriters sell more than 825,000,000 shares"
            ),
            "{}",
            o.quote
        );
        // A preliminary prospectus has no final price, pricing date or trading date.
        for k in ["price_final", "prospectus_date", "trading_date"] {
            assert!(find(&f, k).is_none(), "{k}");
        }
        // Every quote is in the document, word for word.
        for x in &f {
            assert!(verify_quote(&text, &x.quote), "{x:?}");
        }
        assert_eq!(f.len(), 9, "{f:#?}");
    }

    #[test]
    fn a_made_up_424b4_cover_gives_the_final_price_and_dates() {
        let text = cover_text(B4);
        let f = extract(&text, "424B4");
        let p = get(&f, "price_final");
        assert_eq!(p.num, Some(24.0));
        assert_eq!(
            p.quote,
            "The initial public offering price is $24.00 per share."
        );
        assert_eq!(
            get(&f, "prospectus_date").text.as_deref(),
            Some("2026-11-04")
        );
        let t = get(&f, "trading_date");
        assert_eq!(t.text.as_deref(), Some("2026-11-05"));
        assert_eq!(
            t.quote,
            "Our Class A common stock is expected to begin trading on the New York Stock Exchange on November 5, 2026."
        );
        assert_eq!(
            get(&f, "exchange").text.as_deref(),
            Some("New York Stock Exchange")
        );
        assert_eq!(get(&f, "symbol").text.as_deref(), Some("NWRB"));
        assert_eq!(get(&f, "shares_offered").num, Some(20_000_000.0));
        assert_eq!(get(&f, "shares_company").num, Some(20_000_000.0));
        assert_eq!(get(&f, "option_shares").num, Some(3_000_000.0));
        // No range on a final prospectus.
        assert!(find(&f, "price_low").is_none());
        // Read as a registration statement, the same words give no final price.
        assert!(find(&extract(&text, "S-1/A"), "price_final").is_none());
    }

    #[test]
    fn on_or_about_is_no_trading_date_and_estimates_are_no_final_price() {
        let t = "Our common stock is expected to begin trading on or about November 5, 2026. \
                 It is estimated that the initial public offering price is $24.00 per share.";
        let f = extract(t, "424B4");
        assert!(find(&f, "trading_date").is_none(), "{f:?}");
        assert!(find(&f, "price_final").is_none(), "{f:?}");
    }

    #[test]
    fn a_split_that_does_not_add_up_is_dropped() {
        let t = "Class A Common Stock 1,000,000 Shares. We are selling 700,000 shares of Class A common stock and the selling stockholders are selling an additional 200,000 shares of Class A common stock.";
        let f = extract(t, "S-1");
        assert_eq!(get(&f, "shares_offered").num, Some(1_000_000.0));
        assert!(find(&f, "shares_company").is_none() && find(&f, "shares_selling").is_none());
    }

    #[test]
    fn a_range_outside_an_offering_price_sentence_is_ignored() {
        let t = "Our revenue grew between $1.00 and $2.00 per user. No range here.";
        assert!(extract(t, "S-1/A").is_empty());
    }

    #[test]
    fn sentences_skip_abbreviations_and_decimals() {
        let t = "Intro. Northwind Robotics, Inc. is offering 5,000,000 shares at $1.50 in the U.S. market. Next.";
        let at = t.find("offering").unwrap();
        assert_eq!(
            sentence(t, at).unwrap(),
            "Northwind Robotics, Inc. is offering 5,000,000 shares at $1.50 in the U.S. market."
        );
    }
}
