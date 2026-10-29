//! Bilingual number extraction and the "every number is traceable" check.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    En,
    Zh,
}

/// A number found in text.
///
/// `value` is the signed, fully scaled value (e.g. "922.8亿" -> 9.228e10,
/// "下跌1.17%" -> -1.17). `multiplier` is the unit scale applied (1 for plain
/// numbers and percents); `signed` is true when an explicit sign marker
/// (`+ - − ▲ ▼` or `上涨 下跌 涨 跌`) sits directly before the number.
#[derive(Debug, Clone, PartialEq)]
pub struct Num {
    pub raw: String,
    pub value: f64,
    pub decimals: u8,
    pub is_percent: bool,
    pub multiplier: f64,
    pub signed: bool,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid built-in regex")
}

/// Core numeric token: thousands-grouped first (leftmost-first alternation),
/// then plain digits, each with an optional `.`/`．` fraction.
fn number_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"\d{1,3}(?:[,，]\d{3})+(?:[.．]\d+)?|\d+(?:[.．]\d+)?"))
}

/// Spans whose digits are never checked: dates, clock times, fiscal labels,
/// list ordinals, exchange-suffixed tickers and index names. Patterns that
/// must not run into a following digit end with `(?:\D|$)`.
fn ignore_res() -> &'static [Regex] {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| {
        const MONTH: &str = r"(?:Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)";
        [
            // Dates.
            r"\d{4}[-/.]\d{1,2}[-/.]\d{1,2}".to_string(),
            r"\d{1,2}/\d{1,2}(?:/\d{2,4})?".to_string(),
            format!(r"{MONTH}\.?\s+\d{{1,2}}(?:st|nd|rd|th)?(?:,?\s+\d{{4}})?"),
            format!(r"\d{{1,2}}(?:st|nd|rd|th)?\s+{MONTH}(?:,?\s+\d{{4}})?"),
            r"\d{4}\s?年".to_string(),
            r"\d{1,2}\s?月".to_string(),
            r"\d{1,2}\s?[日号]".to_string(),
            // Clock times.
            r"\d{1,2}[:：]\d{2}(?:[:：]\d{2})?".to_string(),
            r"\d{1,2}\s?(?:AM|PM|am|pm|a\.m\.|p\.m\.)(?:[^A-Za-z]|$)".to_string(),
            // "点" alone means index points ("下跌300点"), so only "8点30分"/"8点半"/"8时".
            r"\d{1,2}\s?点\s?(?:\d{1,2}\s?分|半)".to_string(),
            r"\d{1,2}\s?时".to_string(),
            // Fiscal labels.
            // Bare "Q3"/"H1"/"FY2027" are dropped by the glued-letter rule; this
            // covers the year in "Q3 2026" / "FY 2027".
            r"(?:Q[1-4]|H[12])\s(?:19|20)\d{2}(?:[^\d%％.．,，]|$)".to_string(),
            r"FY\s?\d{2,4}".to_string(),
            r"[1-4]Q\d{2,4}".to_string(),
            r"(?i:fiscal)\s+(?:(?i:year)\s+)?\d{4}".to_string(),
            r"第\s?\d+".to_string(),
            r"\d\s?季度?".to_string(),
            r"\d{2,4}\s?财年".to_string(),
            // List ordinals at line start ("1." / "2)" / "3、").
            r"(?m)^[ \t]*\d{1,3}[.)．、](?:[ \t]|$)".to_string(),
            // Exchange-suffixed tickers (0700.HK, 600519.SS).
            r"\d{3,6}\.(?:HK|SS|SZ|SH|TW|KS|KQ|T|L)(?:[^A-Za-z]|$)".to_string(),
            // Index names.
            r"S&P\s?(?:500|400|600|100)(?:\D|$)".to_string(),
            r"(?i:nasdaq)[\s-]?100(?:\D|$)".to_string(),
            r"Russell\s?(?:1000|2000|3000)(?:\D|$)".to_string(),
            r"Dow(?:\s+Jones)?\s?30(?:\D|$)".to_string(),
            r"(?:(?i:euro)\s?)?(?i:stoxx)\s?(?:50|600)(?:\D|$)".to_string(),
            r"(?:Nikkei\s?225|FTSE\s?100|DAX\s?40|CAC\s?40|CSI\s?300)(?:\D|$)".to_string(),
            r"(?:标普|纳斯达克|纳指|罗素|道琼斯|沪深|中证|日经|富时)\s?\d+".to_string(),
        ]
        .iter()
        .map(|p| re(p))
        .collect()
    })
}

fn prev_char(text: &str, at: usize) -> Option<char> {
    text[..at].chars().next_back()
}

/// Parses the unit after the number core. Returns (bytes consumed, multiplier, is_percent).
fn parse_unit(rest: &str, has_dollar: bool) -> (usize, f64, bool) {
    let trimmed = rest.trim_start_matches([' ', '\u{3000}']);
    let ws = rest.len() - trimmed.len();
    let next_is_letter = |s: &str| s.chars().next().is_some_and(|c| c.is_ascii_alphabetic());

    if let Some(c) = trimmed.chars().next().filter(|c| *c == '%' || *c == '％') {
        return (ws + c.len_utf8(), 1.0, true);
    }
    for word in ["percent", "per cent"] {
        if trimmed.len() >= word.len()
            && trimmed.is_char_boundary(word.len())
            && trimmed[..word.len()].eq_ignore_ascii_case(word)
            && ws > 0
            && !next_is_letter(&trimmed[word.len()..])
        {
            return (ws + word.len(), 1.0, true);
        }
    }
    // zh multipliers, longest first.
    for (unit, mult) in [("万亿", 1e12), ("亿", 1e8), ("万", 1e4)] {
        if trimmed.starts_with(unit) {
            return (ws + unit.len(), mult, false);
        }
    }
    // en words.
    for (word, mult) in [
        ("thousand", 1e3),
        ("million", 1e6),
        ("billion", 1e9),
        ("trillion", 1e12),
    ] {
        if trimmed.len() >= word.len()
            && trimmed.is_char_boundary(word.len())
            && trimmed[..word.len()].eq_ignore_ascii_case(word)
            && !next_is_letter(&trimmed[word.len()..])
        {
            return (ws + word.len(), mult, false);
        }
    }
    // Abbreviations: attached, or after one space for $-amounts.
    if ws == 0 || (has_dollar && ws == 1) {
        for (abbr, mult) in [
            ("bn", 1e9),
            ("mn", 1e6),
            ("tn", 1e12),
            ("B", 1e9),
            ("M", 1e6),
            ("K", 1e3),
            ("k", 1e3),
            ("T", 1e12),
        ] {
            if trimmed.starts_with(abbr) && !next_is_letter(&trimmed[abbr.len()..]) {
                return (ws + abbr.len(), mult, false);
            }
        }
    }
    (0, 1.0, false)
}

/// Extracts every checkable number from `text`, skipping dates, times,
/// fiscal labels, list ordinals, tickers and index names.
///
/// Rules are applied the same for both languages because drafts mix scripts
/// (zh drafts carry `$` amounts and tickers); `_lang` is kept for callers.
pub fn extract(text: &str, _lang: Lang) -> Vec<Num> {
    let ignored: Vec<(usize, usize)> = ignore_res()
        .iter()
        .flat_map(|r| r.find_iter(text).map(|m| (m.start(), m.end())))
        // A span starting mid-number ("1.25日") is not a date/time. Commas are not
        // guarded: "，" is ordinary zh punctuation ("19%，2026年").
        .filter(|&(a, _)| {
            !(text[a..].starts_with(|c: char| c.is_ascii_digit())
                && prev_char(text, a).is_some_and(|c| c.is_ascii_digit() || c == '.' || c == '．'))
        })
        .collect();

    let mut out = Vec::new();
    for m in number_re().find_iter(text) {
        let (s, e) = (m.start(), m.end());
        if ignored.iter().any(|&(a, b)| s < b && a < e) {
            continue;
        }
        // Ticker-embedded digits: a letter glued on either side.
        if prev_char(text, s).is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }

        let mut raw_start = s;
        let has_dollar = prev_char(text, s) == Some('$');
        if has_dollar {
            raw_start -= 1;
        }

        // Sign marker.
        let mut sign: Option<f64> = None;
        match prev_char(text, raw_start) {
            Some(c @ ('+' | '−')) => {
                sign = Some(if c == '+' { 1.0 } else { -1.0 });
                raw_start -= c.len_utf8();
            }
            Some('-') => {
                let before = prev_char(text, raw_start - 1);
                if !before.is_some_and(|c| c.is_alphanumeric()) {
                    sign = Some(-1.0);
                    raw_start -= 1;
                }
            }
            _ => {
                let head = text[..raw_start].trim_end_matches([' ', '\u{3000}']);
                match head.chars().next_back() {
                    Some(c @ ('▲' | '▼')) => {
                        sign = Some(if c == '▲' { 1.0 } else { -1.0 });
                        raw_start = head.len() - c.len_utf8();
                    }
                    Some('涨') => sign = Some(1.0),
                    Some('跌') => sign = Some(-1.0),
                    _ => {}
                }
            }
        }

        let (unit_len, multiplier, is_percent) = parse_unit(&text[e..], has_dollar);
        let end = e + unit_len;
        // A letter glued after the number (e.g. "5G", "3D", "1st") is not a figure.
        if unit_len == 0 && text[e..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }

        let core: String = m
            .as_str()
            .chars()
            .filter(|c| *c != ',' && *c != '，')
            .map(|c| if c == '．' { '.' } else { c })
            .collect();
        let decimals = core
            .split_once('.')
            .map_or(0, |(_, f)| f.len().min(u8::MAX as usize) as u8);
        let Ok(magnitude) = core.parse::<f64>() else {
            continue;
        };

        out.push(Num {
            raw: text[raw_start..end].to_string(),
            value: sign.unwrap_or(1.0) * magnitude * multiplier,
            decimals,
            is_percent,
            multiplier,
            signed: sign.is_some(),
        });
    }
    out
}

/// A bare integer 0-10 with no unit, percent, decimals, sign or currency.
fn is_small_count(n: &Num) -> bool {
    !n.is_percent
        && !n.signed
        && n.multiplier == 1.0
        && n.decimals == 0
        && !n.raw.contains('$')
        && (0.0..=10.0).contains(&n.value)
}

fn matches(n: &Num, allowed: f64) -> bool {
    let scale = 10f64.powi(n.decimals as i32);
    let mut draft = n.value / n.multiplier;
    let mut cand = allowed / n.multiplier;
    if !n.signed {
        draft = draft.abs();
        cand = cand.abs();
    }
    // Round both to the draft's decimals; the draft side absorbs float noise
    // from the division (e.g. 922.8e8 / 1e8).
    ((cand * scale).round() - (draft * scale).round()).abs() < 0.5
}

/// Checks that every number in `text` traces to some value in `allowed`.
/// Returns the `raw` text of each unmatched number, in order.
pub fn check(text: &str, lang: Lang, allowed: &[f64]) -> Result<(), Vec<String>> {
    let bad: Vec<String> = extract(text, lang)
        .into_iter()
        .filter(|n| !is_small_count(n) && !allowed.iter().any(|&a| matches(n, a)))
        .map(|n| n.raw)
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Brief's exact cases.
    #[test]
    fn brief_es_futures() {
        assert_eq!(
            check(
                "ES futures +0.45% at 5,812.25",
                Lang::En,
                &[0.4512, 5812.25]
            ),
            Ok(())
        );
    }
    #[test]
    fn brief_revenue_billion() {
        assert_eq!(
            check("Revenue hit $92.28 billion", Lang::En, &[92_279_000_000.0]),
            Ok(())
        );
    }
    #[test]
    fn brief_zh_yi_and_percent() {
        assert_eq!(
            check(
                "营收922.8亿美元，增长19%",
                Lang::Zh,
                &[92_279_000_000.0, 18.97]
            ),
            Ok(())
        );
    }
    #[test]
    fn brief_zh_signed_fullwidth_percent() {
        assert_eq!(check("NVDA下跌1.17％", Lang::Zh, &[-1.1734]), Ok(()));
    }
    #[test]
    fn brief_mismatch() {
        assert_eq!(
            check("TSLA rose 3.2%", Lang::En, &[2.9]),
            Err(vec!["3.2%".into()])
        );
    }
    #[test]
    fn brief_en_dates_times_fiscal() {
        assert_eq!(
            check("2026-10-29 8:30 AM ET, Q3 FY2027 results", Lang::En, &[]),
            Ok(())
        );
    }
    #[test]
    fn brief_zh_dates_times_fiscal() {
        assert_eq!(
            check("2026年10月29日上午8:30，第三季", Lang::Zh, &[]),
            Ok(())
        );
    }
    #[test]
    fn brief_zh_wanyi() {
        assert_eq!(check("市值1.2万亿美元", Lang::Zh, &[1.2e12]), Ok(()));
    }

    // Additional cases.
    #[test]
    fn index_names_ignored() {
        assert_eq!(check("S&P 500 rose 0.28%", Lang::En, &[0.28]), Ok(()));
        assert_eq!(
            check(
                "Nasdaq-100 and Russell 2000, Dow 30, Nasdaq 100",
                Lang::En,
                &[]
            ),
            Ok(())
        );
        assert_eq!(
            check("标普500上涨0.28%，纳斯达克100与罗素2000", Lang::Zh, &[0.28]),
            Ok(())
        );
    }
    #[test]
    fn small_count_accepted() {
        assert_eq!(check("3 stocks led; top 5 movers", Lang::En, &[]), Ok(()));
        assert_eq!(check("11 stocks", Lang::En, &[]), Err(vec!["11".into()]));
        assert_eq!(check("rose 3%", Lang::En, &[]), Err(vec!["3%".into()]));
    }
    #[test]
    fn signed_mismatch_fails() {
        assert_eq!(check("▲1.2%", Lang::En, &[-1.2]), Err(vec!["▲1.2%".into()]));
        assert_eq!(
            check("下跌1.2%", Lang::Zh, &[1.2]),
            Err(vec!["1.2%".into()])
        );
    }
    #[test]
    fn unsigned_compares_absolute() {
        assert_eq!(check("NVDA fell 1.17%", Lang::En, &[-1.1734]), Ok(()));
    }
    #[test]
    fn thousands_separators() {
        assert_eq!(check("closed at 1,234.5", Lang::En, &[1234.5]), Ok(()));
        assert_eq!(check("收于1，234．5点", Lang::Zh, &[1234.5]), Ok(()));
    }
    #[test]
    fn abbreviations_attached_or_after_dollar() {
        assert_eq!(
            check("$1.2B deal and 3.4bn more", Lang::En, &[1.2e9, 3.4e9]),
            Ok(())
        );
        assert_eq!(check("$1.2 B", Lang::En, &[1.2e9]), Ok(()));
        // Stray word "B" is not a multiplier: 12 must match 12 itself.
        assert_eq!(check("Class 12 B shares", Lang::En, &[12.0]), Ok(()));
    }
    #[test]
    fn ordinals_and_tickers_ignored() {
        assert_eq!(check("1. Apple\n2. 0700.HK", Lang::En, &[]), Ok(()));
        assert_eq!(check("Oct 29, 2026 at 9:25 AM", Lang::En, &[]), Ok(()));
    }
    #[test]
    fn zh_index_points_are_checked() {
        assert_eq!(
            check("道指下跌300点", Lang::Zh, &[]),
            Err(vec!["300".into()])
        );
        assert_eq!(
            check("收于5812.25点", Lang::Zh, &[]),
            Err(vec!["5812.25".into()])
        );
        assert_eq!(
            check("收于1，234．5点", Lang::Zh, &[]),
            Err(vec!["1，234．5".into()])
        );
        assert_eq!(check("上午8点30分", Lang::Zh, &[]), Ok(()));
        assert_eq!(
            check("增长19%，2026年10月29日公布", Lang::Zh, &[19.0]),
            Ok(())
        );
    }
    #[test]
    fn fiscal_year_and_not_percent() {
        assert_eq!(
            check("Q3 2026 results; FY 2027 guide", Lang::En, &[]),
            Ok(())
        );
        assert_eq!(
            check("Dow fell 300 points", Lang::En, &[]),
            Err(vec!["300".into()])
        );
    }
    #[test]
    fn hyphen_range_not_a_sign() {
        assert_eq!(check("up 10-15%", Lang::En, &[10.0, 15.0]), Ok(()));
        assert_eq!(
            check("ES -0.3%", Lang::En, &[0.3]),
            Err(vec!["-0.3%".into()])
        );
    }
    #[test]
    fn multiple_failures_in_order() {
        assert_eq!(
            check(
                "TSLA rose 3.2% to $250.10, volume 12 million",
                Lang::En,
                &[12e6]
            ),
            Err(vec!["3.2%".into(), "$250.10".into()])
        );
    }
    #[test]
    fn extract_fields() {
        let n = extract("营收922.8亿美元", Lang::Zh);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].raw, "922.8亿");
        assert_eq!(n[0].decimals, 1);
        assert!(!n[0].is_percent);
        assert!((n[0].value - 92_280_000_000.0).abs() < 1.0);
        let n = extract("NVDA下跌1.17％", Lang::Zh);
        assert!(n[0].is_percent && n[0].signed && n[0].value < 0.0);
    }
}
