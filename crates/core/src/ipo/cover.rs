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

use crate::anchor::normalize;

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

/// ASCII `\b`, `\s` and `\d`: a Unicode pattern compiles 5-10 times slower
/// (about 10 ms for the set natively), and the cover read has 10 ms of CPU.
fn re(p: &str) -> Regex {
    Regex::new(&format!("(?-u){p}")).expect("valid built-in regex")
}

struct Patterns {
    cover_date: Regex,
    prospectus_date: Regex,
    footer_date: Regex,
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
            r"(?i:Subject to Completion)[.,]?\s+(?:Preliminary Prospectus\s+)?[Dd]ated\s+{MONTH}\s+(\d{{1,2}}),\s+(\d{{4}})\.?"
        )),
        prospectus_date: re(&format!(
            r"(?:Prospectus dated|PROSPECTUS DATED|The date of this prospectus is)\s+{MONTH}\s+(\d{{1,2}}),\s+(\d{{4}})"
        )),
        footer_date: re(&format!(r"\b{MONTH}\s+(\d{{1,2}}),\s+(\d{{4}})\s+Table of Contents\b")),
        total: re(&format!(
            r"(?:Class [A-Z] )?(?:Common Stock|Ordinary Shares|Common Shares)\s+{SHARES_NUM}\s+Shares\b|\b{SHARES_NUM}\s+(?:Shares|American Depositary Shares)\s+[A-Za-z0-9 ,.&'()-]{{0,60}}?(?:Common Stock|Ordinary Shares|Common Shares)\b|\b{SHARES_NUM}\s+(?:American Depositary Shares|Ordinary Shares)\b|\b{SHARES_NUM}\s+shares\s+(?:Class [A-Z]\s+)?common stock\b"
        )),
        company: re(&format!(
            r"\bWe are (?:offering|selling) {SHARES_NUM} (?:shares|ordinary shares|American Depositary Shares|ADSs)\b|\bis (?:offering|selling) {SHARES_NUM} (?:shares|ordinary shares|American Depositary Shares|ADSs) of its\b"
        )),
        selling: re(&format!(
            r"selling (?:stockholders?|shareholders?)\b(?u:[^.])*?\b(?:are |is )?(?:offering|selling) (?:an additional |a total of |an aggregate of )?{SHARES_NUM} (?:shares|ordinary shares|American Depositary Shares|ADSs)\b"
        )),
        option: re(&format!(
            r"(?:(?:option|right)(?: for a period of \d+ days)? to purchase|underwriters may (?:also )?purchase) (?:up to )?(?:an additional |a maximum of )?{SHARES_NUM} (?:additional )?(?:shares|ordinary shares|American Depositary Shares|ADSs)\b"
        )),
        range: re(r"between \$\s?(\d{1,4}(?:\.\d{2})?) and \$\s?(\d{1,4}(?:\.\d{2})?)"),
        final_price: re(
            r"(?i)\binitial public offering price(?: per (?:share|ADS))?(?: (?:of|for) (?:our|the|its) [A-Za-z ]{1,60}?)? (?:is|of) \$\s?(\d{1,4}(?:\.\d{2})?)",
        ),
        listing: re(&format!(
            r"\b(?:list|listing|listed)\b(?u:[^.])*? on (?:the |The )?({EXCHANGES})(?: LLC)?(?:\s*\(the “[A-Za-z]+”\)|\s*\(“[A-Za-z]+”\))?,?\s+under the (?:ticker |trading )?symbol\s+(?:“|\x22)([A-Z]{{1,5}}(?:\.[A-Z])?)\.?(?:”|\x22)"
        )),
        trading: re(&format!(
            r"(?:expected to|will) (?:begin|commence) trading\b(?u:[^.])*? on {MONTH} (\d{{1,2}}), (\d{{4}})"
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

/// Filer software often writes curly quotes and dashes as `&#147;`-style
/// Windows-1252 numbers, which decode to C1 control characters (U+0093):
/// read them as the characters they stand for.
fn cp1252_quotes(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{91}' => '‘',
            '\u{92}' => '’',
            '\u{93}' => '“',
            '\u{94}' => '”',
            '\u{96}' => '–',
            '\u{97}' => '—',
            c => c,
        })
        .collect()
}

/// Words that never end a sentence before their period.
const ABBREV: [&str; 12] = [
    "Jr", "Sr", "Mr", "Ms", "Mrs", "Dr", "St", "No", "Nos", "vs", "approx", "Messrs",
];
/// Company suffixes: their period ends a sentence only before a capital
/// ("..., Inc. We are offering" ends; "Reddit, Inc. is offering" does not).
const SUFFIXES: [&str; 5] = ["Inc", "Corp", "Co", "Ltd", "Bros"];

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
            if SUFFIXES.contains(&word.as_str()) {
                return text[i + 1..]
                    .trim_start()
                    .starts_with(|c: char| c.is_ascii_uppercase());
            }
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
    // A table row ends with a figure and no period: "... 4,773,090,000 The
    // underwriters may ..." starts its sentence at "The".
    let start = table_end(&text[start..from]).map_or(start, |k| start + k);
    // A capitalized "We" or "Our" starts a sentence even after a heading
    // with no period ("... Birkenstock Holding plc We are offering ...").
    let start = if PRONOUNS.iter().any(|w| text[from..].starts_with(w)) {
        from
    } else {
        start
    };
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

/// Words that start a cover sentence.
const STARTERS: [&str; 12] = [
    "The ",
    "We ",
    "Our ",
    "This ",
    "To ",
    "Each ",
    "Prior ",
    "In ",
    "Upon ",
    "Following ",
    "All ",
    "An ",
];

/// Pronouns that, capitalized, only ever start a sentence.
const PRONOUNS: [&str; 2] = ["We ", "Our "];

/// Where the last "<figure> <Starter>" boundary in `s` lets a sentence start.
fn table_end(s: &str) -> Option<usize> {
    s.char_indices()
        .filter(|&(i, c)| {
            c == ' '
                && s[..i].ends_with(|d: char| d.is_ascii_digit())
                && STARTERS.iter().any(|w| s[i + 1..].starts_with(w))
        })
        .map(|(i, _)| i + 1)
        .last()
}

/// The sentence holding capture group 0, or the match itself when the
/// sentence is too long.
fn quote_of(text: &str, c: &Captures) -> String {
    let m = c.get(0).expect("group 0");
    let s = sentence(text, m.start()).unwrap_or_else(|| m.as_str().trim().to_string());
    // A cover heading glued to the sentence ("Class A Common Stock Reddit,
    // Inc. is offering ..."): the sentence starts after it, when the match
    // is still whole.
    match heading_end(&s) {
        Some(k) if s[k..].contains(m.as_str().trim()) => s[k..].to_string(),
        _ => s,
    }
}

/// Where a leading cover heading ("Class A Common Stock ", "Ordinary
/// Shares ") ends, when a capitalized word follows it.
fn heading_end(s: &str) -> Option<usize> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        re(r"^(?:Preliminary Prospectus\s+)?(?:Class [A-Z] )?(?:Common Stock|Ordinary Shares|Common Shares|American Depositary Shares)\s+")
    });
    let k = r.find(s)?.end();
    s[k..]
        .starts_with(|c: char| c.is_ascii_uppercase())
        .then_some(k)
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
/// `v` written in `quote` as a cover writes it ("825,000,000", "$70.00",
/// "$24"), not inside a longer number. String matching, not the number
/// checker, whose ~30 Unicode patterns would cost a cold read several ms.
fn num_in_quote(v: f64, quote: &str) -> bool {
    let mut forms = vec![format!("{v:.2}")];
    if v.fract() == 0.0 && v.abs() < 1e15 {
        let digits = format!("{}", v as i64);
        let mut grouped = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        forms.push(grouped);
        forms.push(digits);
    }
    forms.iter().any(|f| {
        quote.match_indices(f.as_str()).any(|(i, _)| {
            let before = quote[..i].chars().next_back();
            let after = quote[i + f.len()..].chars().next();
            let digitish = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit() || c == ',');
            // A trailing "." ends a sentence; ".5" would be a decimal.
            let decimal = quote[i + f.len()..]
                .strip_prefix('.')
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
            !digitish(before) && !digitish(after) && !decimal
        })
    })
}

/// `doc` is `normalize(text)`, made once for every fact (as
/// `anchor::verify_quote` checks it).
fn verified(doc: &str, f: &CoverFact) -> bool {
    let q = normalize(&f.quote);
    if q.chars().count() < 20 || !doc.contains(&q) {
        return false;
    }
    match (&f.num, &f.text) {
        (Some(v), _) => num_in_quote(*v, &f.quote),
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
    let text = &cp1252_quotes(text);
    let p = patterns();
    let is_final = super::is_final(form);
    let mut out: Vec<CoverFact> = Vec::new();

    if let Some(c) = p.cover_date.captures(text) {
        if let Some(d) = date_at(&c, 1) {
            out.push(fact_text("cover_date", d, c[0].trim().to_string()));
        }
    }
    if is_final {
        // "Prospectus dated <date>" first; only without one, the bare date
        // that closes some covers ("... Roberts & Ryan June 4, 2025 Table of
        // Contents"), so an earlier footer date never wins.
        let c = p
            .prospectus_date
            .captures(text)
            .or_else(|| p.footer_date.captures(text));
        if let Some(c) = c {
            if let Some(d) = date_at(&c, 1) {
                out.push(fact_text("prospectus_date", d, c[0].trim().to_string()));
            }
        }
    }
    let total = p.total.captures(text).and_then(|c| {
        let v = (1..=4)
            .find_map(|i| c.get(i))
            .and_then(|m| num(m.as_str()))?;
        // "(Representing 95,500,000 Ordinary Shares" takes its ")".
        let m = c.get(0).expect("group 0");
        let mut q = m.as_str().trim().to_string();
        if q.matches('(').count() > q.matches(')').count() && text[m.end()..].starts_with(')') {
            q.push(')');
        }
        Some(fact_num("shares_offered", v, q))
    });
    let company = p.company.captures(text).and_then(|c| {
        let n = (1..=2).find_map(|i| c.get(i))?;
        Some(fact_num(
            "shares_company",
            num(n.as_str())?,
            quote_of(text, &c),
        ))
    });
    // The selling stockholders' number usually sits in the company's
    // sentence: the same quote then, not a longer one.
    let selling = p.selling.captures(text).and_then(|c| {
        let q = match &company {
            Some(cf) if cf.quote.contains(c[0].trim()) => cf.quote.clone(),
            _ => quote_of(text, &c),
        };
        Some(fact_num("shares_selling", num(&c[1])?, q))
    });
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
        // Not an estimate, a range or another security's price ("The
        // Series A Preferred Stock will have a conversion price equal to the
        // initial public offering price of $30.00").
        let q = quote_of(text, c).to_ascii_lowercase();
        !["estimated", "between", "conversion", "preferred"]
            .iter()
            .any(|w| q.contains(w))
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
    // A trading day only from a final prospectus: a preliminary one's plan
    // may change.
    if let Some(c) = p.trading.captures(text).filter(|_| is_final) {
        if let Some(d) = date_at(&c, 1) {
            out.push(fact_text("trading_date", d, quote_of(text, &c)));
        }
    }
    let doc = normalize(text);
    out.retain(|f| verified(&doc, f));
    out
}

/// An amendment filed only to add exhibits ("is being filed solely for the
/// purpose of filing exhibits"): it has no prospectus cover.
pub fn exhibits_only(text: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)\bsolely (?:for the purpose of |to )(?:fil(?:e|ing)|includ(?:e|ing)|add(?:ing)?) (?:certain |an |the )?(?:exhibits?|Exhibit\s+\d)|\bfor the sole purpose of (?:filing|including) (?:certain |an |the )?exhibits?")
    })
    .is_match(text)
}

/// The kinds a final prospectus must yield to become the facts source.
pub const FINAL_KINDS: [&str; 2] = ["price_final", "shares_offered"];

/// What a filing of `form` is missing of the kinds it must yield on its
/// own: a final prospectus its price and shares. A registration
/// statement's needs depend on the facts it would replace (`resolve`).
pub fn missing_for(facts: &[CoverFact], form: &str) -> Vec<&'static str> {
    if !super::is_final(form) {
        return vec![];
    }
    FINAL_KINDS
        .iter()
        .copied()
        .filter(|k| find(facts, k).is_none())
        .collect()
}

/// The fact of `kind`, if read.
pub fn find<'a>(facts: &'a [CoverFact], kind: &str) -> Option<&'a CoverFact> {
    facts.iter().find(|f| f.kind == kind)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::anchor::verify_quote;
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

    pub(crate) const REDDIT: &str = include_str!("../../tests/fixtures/edgar_ipo_424b4_reddit.htm");
    pub(crate) const ARM: &str = include_str!("../../tests/fixtures/edgar_ipo_424b4_arm.htm");
    pub(crate) const COREWEAVE: &str =
        include_str!("../../tests/fixtures/edgar_ipo_424b4_coreweave.htm");

    /// `(kind, number or text)` of each fact read, in order.
    fn values(f: &[CoverFact]) -> Vec<(&'static str, String)> {
        f.iter()
            .map(|x| {
                let v = x
                    .num
                    .map(|n| n.to_string())
                    .or_else(|| x.text.clone())
                    .unwrap_or_default();
                (x.kind, v)
            })
            .collect()
    }
    fn kv(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(k, v)| (*k, v.to_string())).collect()
    }

    const INSTACART: &str = include_str!("../../tests/fixtures/edgar_ipo_424b4_instacart.htm");
    const CIRCLE: &str = include_str!("../../tests/fixtures/edgar_ipo_424b4_circle.htm");
    const BIRKENSTOCK: &str = include_str!("../../tests/fixtures/edgar_ipo_f1a_birkenstock.htm");

    #[test]
    fn instacart_real_424b4_takes_the_cover_sentence_not_the_preferred_one() {
        let f = extract(&cover_text(INSTACART), "424B4");
        assert_eq!(
            values(&f),
            kv(&[
                ("prospectus_date", "2023-09-18"),
                ("shares_offered", "22000000"),
                ("shares_company", "14100000"),
                ("shares_selling", "7900000"),
                ("option_shares", "3300000"),
                ("price_final", "30"),
                ("exchange", "Nasdaq Global Select Market"),
                ("symbol", "CART"),
            ])
        );
        assert_eq!(
            get(&f, "price_final").quote,
            "The initial public offering price for our common stock is $30.00 per share."
        );
        assert!(missing_for(&f, "424B4").is_empty());
    }

    #[test]
    fn circle_real_424b4_in_lowercase_with_its_pricing_date() {
        let f = extract(&cover_text(CIRCLE), "424B4");
        assert_eq!(
            values(&f),
            kv(&[
                ("prospectus_date", "2025-06-04"),
                ("shares_offered", "34000000"),
                ("shares_company", "14800000"),
                ("shares_selling", "19200000"),
                ("option_shares", "5100000"),
                ("price_final", "31"),
                ("exchange", "New York Stock Exchange"),
                ("symbol", "CRCL"),
            ])
        );
        assert_eq!(
            get(&f, "shares_offered").quote,
            "34,000,000 shares Class A common stock"
        );
        assert_eq!(
            get(&f, "prospectus_date").quote,
            "June 4, 2025 Table of Contents"
        );
        assert!(missing_for(&f, "424B4").is_empty());
    }

    #[test]
    fn birkenstock_real_f1a_in_ordinary_shares() {
        let f = extract(&cover_text(BIRKENSTOCK), "F-1/A");
        assert_eq!(
            values(&f),
            kv(&[
                ("cover_date", "2023-10-04"),
                ("shares_offered", "32258064"),
                ("shares_company", "10752688"),
                ("shares_selling", "21505376"),
                ("option_shares", "4838709"),
                ("price_low", "44"),
                ("price_high", "49"),
                ("exchange", "New York Stock Exchange"),
                ("symbol", "BIRK"),
            ])
        );
        assert_eq!(
            get(&f, "shares_company").quote,
            "We are offering 10,752,688 ordinary shares, no par value, of Birkenstock Holding plc (the “Company”) and the selling shareholder identified in this prospectus is offering an additional 21,505,376 ordinary shares of the Company."
        );
        assert_eq!(
            get(&f, "cover_date").quote,
            "Subject To Completion Dated October 4, 2023"
        );
        // The selling shareholder's number takes the same sentence.
        assert_eq!(
            get(&f, "shares_selling").quote,
            get(&f, "shares_company").quote
        );
    }

    #[test]
    fn reddit_real_424b4() {
        let text = cover_text(REDDIT);
        let f = extract(&text, "424B4");
        assert_eq!(
            values(&f),
            kv(&[
                ("prospectus_date", "2024-03-20"),
                ("shares_offered", "22000000"),
                ("shares_company", "15276527"),
                ("shares_selling", "6723473"),
                ("option_shares", "3300000"),
                ("price_final", "34"),
                ("exchange", "New York Stock Exchange"),
                ("symbol", "RDDT"),
            ])
        );
        assert_eq!(
            get(&f, "price_final").quote,
            "The initial public offering price per share of our Class A common stock is $34.00."
        );
        assert_eq!(
            get(&f, "shares_offered").quote,
            "22,000,000 Shares Reddit, Inc. Class A Common Stock"
        );
        // The heading glued before the split sentence is left out.
        let split = "Reddit, Inc. is offering 15,276,527 shares of its Class A common stock and the selling stockholders identified in this prospectus are offering an aggregate of 6,723,473 shares of Class A common stock.";
        assert_eq!(get(&f, "shares_company").quote, split);
        assert_eq!(get(&f, "shares_selling").quote, split);
        assert!(missing_for(&f, "424B4").is_empty());
        for x in &f {
            assert!(verify_quote(&cp1252_quotes(&text), &x.quote), "{x:?}");
        }
    }

    #[test]
    fn arm_real_424b4_in_ads() {
        let text = cover_text(ARM);
        let f = extract(&text, "424B4");
        assert_eq!(
            values(&f),
            kv(&[
                ("prospectus_date", "2023-09-13"),
                ("shares_offered", "95500000"),
                ("option_shares", "7000000"),
                ("price_final", "51"),
                ("exchange", "Nasdaq Global Select Market"),
                ("symbol", "ARM"),
            ])
        );
        assert_eq!(
            get(&f, "price_final").quote,
            "The initial public offering price per ADS is $51.00."
        );
        // The cover's C1 quote characters are read as curly quotes.
        assert_eq!(
            get(&f, "symbol").quote,
            "Our ADSs have been approved for listing on the Nasdaq Global Select Market (“Nasdaq”) under the symbol “ARM”."
        );
        assert!(get(&f, "option_shares")
            .quote
            .starts_with("The underwriters may also exercise"));
        assert!(missing_for(&f, "424B4").is_empty());
    }

    #[test]
    fn coreweave_real_424b4() {
        let text = cover_text(COREWEAVE);
        let f = extract(&text, "424B4");
        assert_eq!(
            values(&f),
            kv(&[
                ("prospectus_date", "2025-03-27"),
                ("shares_offered", "37500000"),
                ("shares_company", "36590000"),
                ("shares_selling", "910000"),
                ("price_final", "40"),
                ("exchange", "Nasdaq Stock Market"),
                ("symbol", "CRWV"),
            ])
        );
        assert_eq!(
            get(&f, "shares_company").quote,
            "We are offering 36,590,000 shares of our Class A common stock and the selling stockholders identified in this prospectus are offering 910,000 shares of our Class A common stock in this offering."
        );
        assert!(missing_for(&f, "424B4").is_empty());
    }

    #[test]
    fn an_explicit_pricing_date_wins_over_an_earlier_footer_date() {
        let t = "Underwriter One Underwriter Two May 1, 2025 Table of Contents The initial public offering price is $10.00 per share. Prospectus dated June 4, 2025";
        let f = extract(t, "424B4");
        assert_eq!(
            get(&f, "prospectus_date").text.as_deref(),
            Some("2025-06-04")
        );
        // Without the explicit line, the footer date is read.
        let f = extract(&t.replace("Prospectus dated June 4, 2025", ""), "424B4");
        assert_eq!(
            get(&f, "prospectus_date").text.as_deref(),
            Some("2025-05-01")
        );
    }

    #[test]
    fn a_final_prospectus_without_its_price_or_shares_is_incomplete() {
        let only_price = "The initial public offering price per share is $34.00.";
        let f = extract(only_price, "424B4");
        assert_eq!(missing_for(&f, "424B4"), ["shares_offered"]);
        assert_eq!(missing_for(&[], "424B4"), ["price_final", "shares_offered"]);
    }

    #[test]
    fn a_selling_stockholder_is_not_the_company() {
        let t = "Class A Common Stock 1,000,000 Shares. The selling stockholder is offering 1,000,000 shares of Class A common stock.";
        let f = extract(t, "S-1");
        assert!(find(&f, "shares_company").is_none(), "{f:?}");
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
    fn numbers_are_found_as_the_cover_writes_them() {
        assert!(num_in_quote(
            825_000_000.0,
            "Class A Common Stock 825,000,000 Shares"
        ));
        assert!(num_in_quote(70.0, "between $70.00 and $78.00."));
        assert!(num_in_quote(24.0, "is $24 per share"));
        assert!(!num_in_quote(
            825_000.0,
            "Class A Common Stock 825,000,000 Shares"
        ));
        assert!(!num_in_quote(70.0, "between $170.00 and $78.00"));
        assert!(!num_in_quote(70.0, "between $70.005 and $78.00"));
        assert!(!num_in_quote(7.0, "between $70.00 and $78.00"));
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
