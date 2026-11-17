//! Bilingual number extraction and the "every number is traceable" check.

use std::sync::atomic::{AtomicUsize, Ordering};
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
/// `from_range` marks a range's first figure that takes the unit written
/// once after the second ("2.32 to 2.36 million", "$4.3 to $4.4 billion").
/// `at` is where `raw` starts in the text.
#[derive(Debug, Clone, PartialEq)]
pub struct Num {
    pub raw: String,
    pub value: f64,
    pub decimals: u8,
    pub is_percent: bool,
    pub multiplier: f64,
    pub signed: bool,
    pub from_range: bool,
    pub at: usize,
}

/// Patterns this module has compiled in this process (`insight::compiled`).
static COMPILED: AtomicUsize = AtomicUsize::new(0);

fn re(pattern: &str) -> Regex {
    COMPILED.fetch_add(1, Ordering::Relaxed);
    Regex::new(pattern).expect("valid built-in regex")
}

pub(crate) fn compiled() -> usize {
    COMPILED.load(Ordering::Relaxed)
}

/// Compiles every lazily compiled pattern of this module, so a caller can
/// pay the compile before it pays for a model call. One entry per
/// `OnceLock` in this file (`insight::tests` counts them); the number.
pub fn warm() -> usize {
    let touch: [fn(); 3] = [
        || {
            let _ = number_re();
        },
        || {
            let _ = ignore_res();
        },
        // Both of its patterns (`R` and `CHANGE`) at the top of the call.
        || range_units("", &mut [], &[]),
    ];
    for t in touch {
        t();
    }
    4
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
        const MD: &str = r"(?:0?[1-9]|1[0-2])";
        const DD: &str = r"(?:0?[1-9]|[12]\d|3[01])";
        const END: &str = r"(?:[^\d.．]|[.．](?:\D|$)|$)";
        [
            // Dates.
            // Bounded numeric dates: same separator twice, month <= 12, day <= 31,
            // no digit or decimal on either side (the leading side is guarded in
            // `extract`), so "5812.25-5830.50" and "2650.1/2680.3" stay figures.
            format!(r"(?:19|20)\d{{2}}-{MD}-{DD}{END}"),
            format!(r"(?:19|20)\d{{2}}/{MD}/{DD}{END}"),
            format!(r"(?:19|20)\d{{2}}\.{MD}\.{DD}{END}"),
            format!(r"{MD}/{DD}(?:/(?:19|20)?\d{{2}})?{END}"),
            format!(r"{MONTH}\.?\s+\d{{1,2}}(?:st|nd|rd|th)?(?:,?\s+\d{{4}})?"),
            format!(r"\d{{1,2}}(?:st|nd|rd|th)?\s+{MONTH}\b(?:,?\s+\d{{4}})?"),
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
            // Period compounds ("30-year yield", "52-week high").
            r"\d+-(?:year|yr|week|wk|day|month|quarter)s?(?:[^A-Za-z]|$)".to_string(),
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
            r"(?:标普\s?500|纳斯达克\s?100|纳指\s?100|罗素\s?2000|道琼斯\s?30|沪深\s?300|中证\s?500|中证\s?1000|日经\s?225|富时\s?100)(?:\D|$)".to_string(),
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
    // zh multipliers, longest first ("2.36百万桶" is 2.36 million barrels).
    for (unit, mult) in [
        ("千万亿", 1e15),
        ("万亿", 1e12),
        ("千万", 1e7),
        ("百万", 1e6),
        ("亿", 1e8),
        ("万", 1e4),
    ] {
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
            ("mm", 1e6),
            ("B", 1e9),
            ("M", 1e6),
            ("K", 1e3),
            ("k", 1e3),
            ("T", 1e12),
            ("m", 1e6),
            ("b", 1e9),
        ] {
            if trimmed.starts_with(abbr) && !next_is_letter(&trimmed[abbr.len()..]) {
                return (ws + abbr.len(), mult, false);
            }
        }
    }
    // Unit-1 suffixes (basis points, multiples, points): checked as-is.
    if ws == 0 {
        for suffix in ["bps", "bp", "pts", "x"] {
            if trimmed.starts_with(suffix) && !next_is_letter(&trimmed[suffix.len()..]) {
                return (suffix.len(), 1.0, false);
            }
        }
    }
    (0, 1.0, false)
}

/// Glued suffixes that mark a non-figure ("5G", "3D", "1st", "22nd").
fn is_exempt_suffix(rest: &str) -> bool {
    let word: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    matches!(word.as_str(), "G" | "D" | "st" | "nd" | "rd" | "th")
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
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for m in number_re().find_iter(text) {
        let (s, e) = (m.start(), m.end());
        if ignored.iter().any(|&(a, b)| s < b && a < e) {
            continue;
        }
        // Ticker-embedded digits and Q/H/FY labels: a letter glued before.
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
        // Other glued letters are still checked, except a few non-figure suffixes.
        if unit_len == 0 && is_exempt_suffix(&text[e..]) {
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
            from_range: false,
            at: raw_start,
        });
        spans.push((raw_start, end));
    }
    range_units(text, &mut out, &spans);
    out
}

/// A range writes its unit once, after the second figure: "2.32 to 2.36
/// million", "$4.3 to $4.4 billion", "232至236万". The first figure takes
/// that unit when it has none of its own, is under 1,000 and is within a
/// factor of ten of the second ("from 508,000 to 1.8 million" keeps its
/// own units). A change ("grew 9 to $45 billion", "rose by 5 to ...") is
/// not a range, nor are two columns ("$1.62 ~$7.25 billion").
fn range_units(text: &str, out: &mut [Num], spans: &[(usize, usize)]) {
    static R: OnceLock<Regex> = OnceLock::new();
    static CHANGE: OnceLock<Regex> = OnceLock::new();
    let between = R.get_or_init(|| re(r"^\s*(?:to|through|-|–|—|至|到)\s*$"));
    let change = CHANGE.get_or_init(|| {
        re(r"(?i)\b(?:grew|grow|grows|rose|rise|rises|fell|fall|falls|increased|decreased|up|down|by)\s*$")
    });
    for i in 1..out.len() {
        let (a, b) = (&out[i - 1], &out[i]);
        let (first, second) = (a.value.abs(), b.value.abs() / b.multiplier);
        let range = a.multiplier == 1.0
            && !a.is_percent
            && b.multiplier != 1.0
            && !b.is_percent
            && first < 1000.0
            && first > 0.0
            && (0.1..=10.0).contains(&(first / second))
            && between.is_match(&text[spans[i - 1].1..spans[i].0])
            && !change.is_match(&text[..spans[i - 1].0]);
        if range {
            let m = b.multiplier;
            let a = &mut out[i - 1];
            a.value *= m;
            a.multiplier = m;
            a.from_range = true;
        }
    }
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
    check_allowed(
        text,
        lang,
        &Allowed {
            any: allowed,
            ..Allowed::default()
        },
    )
}

/// What a draft's numbers may trace to, by how each value may be written.
#[derive(Default)]
pub struct Allowed<'a> {
    /// Any number.
    pub any: &'a [f64],
    /// A quantity in its own unit, with the unit words one of which must
    /// follow it ("13 MTPA" is 13 million tonnes: "1300万吨"); never money
    /// or a percentage.
    pub quantities: &'a [(f64, &'a [&'a str])],
    /// A count ("twelve months"): only a bare integer right before a time
    /// unit ("12 months", "12个月").
    pub counts: &'a [f64],
    /// A year: only as four digits ("2024", not "2,024").
    pub years: &'a [f64],
    /// A rate from a table quote ("Revenue $13,467 $14,766 10% 10%"): only
    /// a percentage.
    pub rates: &'a [f64],
    /// A value that holds only in context: `ok` gets the sentence (or line)
    /// the number sits in and where in it the number starts.
    pub in_context: &'a [(f64, Holds<'a>)],
}

/// Whether a value holds in a sentence, for a number at a byte offset in it.
pub type Holds<'a> = &'a dyn Fn(&str, usize) -> bool;

/// Time units a count may precede.
const TIME_UNITS: &[&str] = &[
    "months",
    "month",
    "years",
    "year",
    "quarters",
    "quarter",
    "weeks",
    "week",
    "days",
    "day",
    "个月",
    "个季度",
    "季度",
    "年",
    "周",
    "天",
];

/// Money written after a number ("1300万美元", "港元", "人民币").
const MONEY_AFTER: &[&str] = &[
    "美元",
    "港元",
    "日元",
    "欧元",
    "英镑",
    "人民币",
    "元",
    "dollar",
    "usd",
    "hkd",
    "rmb",
    "eur",
    "gbp",
    "jpy",
    "cny",
];

/// The sentence or line around byte `at`, and where it starts: up to a line
/// break, "。", "!", "?" or a period that ends a sentence (not a decimal
/// point). A "；" joins clauses of one sentence ("...22.72亿美元；前九个月为
/// 46.54亿美元").
pub fn sentence_at(text: &str, at: usize) -> (usize, &str) {
    let ends = |i: usize, c: char| {
        matches!(c, '\n' | '。' | '!' | '?' | '！' | '？')
            || c == '.' && !text[i + 1..].starts_with(|d: char| d.is_ascii_digit())
    };
    let start = text[..at]
        .char_indices()
        .rev()
        .find(|&(i, c)| ends(i, c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[at..]
        .char_indices()
        .find(|&(i, c)| ends(at + i, c))
        .map_or(text.len(), |(i, _)| at + i);
    (start, &text[start..end])
}

/// `check` with values that hold only as a quantity, a count, a year, a
/// rate or in context.
pub fn check_allowed(text: &str, lang: Lang, a: &Allowed) -> Result<(), Vec<String>> {
    let bad: Vec<String> = extract(text, lang)
        .into_iter()
        .filter(|n| {
            let bare = n.multiplier == 1.0 && n.decimals == 0 && !n.signed;
            let after = text[n.at + n.raw.len()..].trim_start().to_lowercase();
            let money = n.raw.contains('$')
                || text[..n.at].ends_with(['€', '£', '¥', '￥'])
                || MONEY_AFTER.iter().any(|u| after.starts_with(u));
            let plain = !n.is_percent && !money;
            let core = n.raw.trim_start_matches(['+', '-', '−']);
            // Main read "412百万" as 412: what it accepted so still passes.
            let main_reading = ["百万", "千万"].iter().any(|u| n.raw.ends_with(u)) && {
                let mut plain = n.clone();
                plain.value /= n.multiplier;
                plain.multiplier = 1.0;
                a.any.iter().any(|&v| matches(&plain, v))
            };
            !(is_small_count(n)
                || main_reading
                || a.any.iter().any(|&v| matches(n, v))
                || plain
                    && a.quantities.iter().any(|(v, units)| {
                        matches(n, *v) && units.iter().any(|u| after.starts_with(u))
                    })
                || plain
                    && bare
                    && TIME_UNITS.iter().any(|u| after.starts_with(u))
                    && a.counts.iter().any(|&v| matches(n, v))
                || bare
                    && core.len() == 4
                    && core.chars().all(|c| c.is_ascii_digit())
                    && a.years.iter().any(|&v| matches(n, v))
                || n.is_percent && a.rates.iter().any(|&v| matches(n, v))
                || a.in_context.iter().any(|(v, ok)| {
                    let (start, s) = sentence_at(text, n.at);
                    matches(n, *v) && ok(s, n.at - start)
                }))
        })
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

    #[test]
    fn a_range_writes_its_unit_once() {
        // Each figure as written and the multiplier it is read with.
        let read = |t: &str| {
            extract(t, Lang::En)
                .iter()
                .map(|n| {
                    (
                        (n.value / n.multiplier * 100.0).round() / 100.0,
                        n.multiplier,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            read("2.32 to 2.36 million barrels"),
            [(2.32, 1e6), (2.36, 1e6)]
        );
        assert_eq!(read("$4.3 to $4.4 billion"), [(4.3, 1e9), (4.4, 1e9)]);
        assert_eq!(read("$4.3 - $4.4 billion"), [(4.3, 1e9), (4.4, 1e9)]);
        assert_eq!(read("232至236万桶"), [(232.0, 1e4), (236.0, 1e4)]);
        assert_eq!(read("每日2.32至2.36百万桶"), [(2.32, 1e6), (2.36, 1e6)]);
        assert_eq!(read("1.5千万"), [(1.5, 1e7)]);
        assert_eq!(read("5千万亿"), [(5.0, 1e15)]);
        // Not a range of one unit: its own magnitude, a rate, a list, a
        // first figure far from the second.
        assert_eq!(
            read("from 508,000 to 1.8 million"),
            [(508_000.0, 1.0), (1.8, 1e6)]
        );
        assert_eq!(read("grew 16% to 4.1 billion"), [(16.0, 1.0), (4.1, 1e9)]);
        assert_eq!(read("rose 5 and 2.36 million"), [(5.0, 1.0), (2.36, 1e6)]);
        assert_eq!(read("from 900 to 2.3 million"), [(900.0, 1.0), (2.3, 1e6)]);
        // A change, and two columns, are no range.
        assert_eq!(
            read("Backlog grew 9 to $45 billion"),
            [(9.0, 1.0), (45.0, 1e9)]
        );
        assert_eq!(read("rose by 2 to 2.36 million"), [(2.0, 1.0), (2.36, 1e6)]);
        assert_eq!(
            read("Adjusted EPS $1.62 ~$7.25 billion"),
            [(1.62, 1.0), (7.25, 1e9)]
        );
    }

    #[test]
    fn a_hundred_million_word_keeps_main_reading_too() {
        // Main read "412百万美元" as 412; it still passes where 412 is
        // allowed, and as 412 million where that is.
        assert_eq!(check("412百万美元", Lang::Zh, &[412.0]), Ok(()));
        assert_eq!(check("412百万美元", Lang::Zh, &[4.12e8]), Ok(()));
        assert!(check("413百万美元", Lang::Zh, &[412.0]).is_err());
        assert!(check("4.12千万美元", Lang::Zh, &[4.12e8]).is_err());
    }

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
    // Fix round 1 regressions.
    #[test]
    fn fix1_glued_suffixes_are_checked() {
        for (text, ok) in [
            ("Revenue was $1.2m", 1.2e6),
            ("$3.4b", 3.4e9),
            ("Fed cut 50bp", 50.0),
            ("yield up 12bps", 12.0),
            ("trades at 25x earnings", 25.0),
            ("gained 15pts", 15.0),
            ("$2.5mm raise", 2.5e6),
            ("rose 12abc", 12.0),
        ] {
            assert!(check(text, Lang::En, &[]).is_err(), "should fail: {text}");
            assert_eq!(check(text, Lang::En, &[ok]), Ok(()), "should pass: {text}");
        }
        // Whitelisted glued suffixes stay ignored.
        assert_eq!(
            check("5G rollout, 3D chips, 21st and 32nd", Lang::En, &[]),
            Ok(())
        );
    }
    #[test]
    fn fix2_decimal_ranges_not_dates() {
        assert!(check("ES traded 5812.25-5830.50", Lang::En, &[]).is_err());
        assert_eq!(
            check("ES traded 5812.25-5830.50", Lang::En, &[5812.25, 5830.5]),
            Ok(())
        );
        assert!(check("Gold 2650.1/2680.3", Lang::En, &[]).is_err());
        assert_eq!(
            check("Gold 2650.1/2680.3", Lang::En, &[2650.1, 2680.3]),
            Ok(())
        );
        assert_eq!(
            check("on 2026-10-29. Also 2026/10/29 and 10/29", Lang::En, &[]),
            Ok(())
        );
    }
    #[test]
    fn fix3_zh_index_exact_pairs_only() {
        assert!(check("纳指18500点", Lang::Zh, &[]).is_err());
        assert_eq!(check("纳指18500点", Lang::Zh, &[18500.0]), Ok(()));
        assert!(check("道琼斯42000点", Lang::Zh, &[]).is_err());
        assert_eq!(check("道琼斯42000点", Lang::Zh, &[42000.0]), Ok(()));
        assert_eq!(check("沪深300、中证1000与日经225", Lang::Zh, &[]), Ok(()));
    }
    #[test]
    fn fix4_period_compounds_ignored_bare_years_checked() {
        // Bare years are checked like any figure; the facts allow the trading
        // year and its neighbours (`facts::numbers_in_facts`).
        assert!(check("full-year 2026 guidance", Lang::En, &[]).is_err());
        assert_eq!(
            check("full-year 2026 guidance", Lang::En, &[2026.0]),
            Ok(())
        );
        assert_eq!(
            check("Russell futures at 2045", Lang::En, &[2026.0]),
            Err(vec!["2045".into()])
        );
        assert_eq!(
            check("Russell futures at 2045", Lang::En, &[2045.0]),
            Ok(())
        );
        assert_eq!(check("the 30-year yield", Lang::En, &[]), Ok(()));
        assert_eq!(check("52-week high", Lang::En, &[]), Ok(()));
        assert_eq!(check("a 90-day window, 6-month low", Lang::En, &[]), Ok(()));
        // A year-shaped figure with a unit, decimal, percent, sign or $ is still checked.
        assert!(check("revenue of $2026 million", Lang::En, &[]).is_err());
        assert!(check("gold at 2026.5", Lang::En, &[]).is_err());
        assert!(check("道指上涨2000点", Lang::Zh, &[]).is_err());
    }
    #[test]
    fn month_needs_word_boundary() {
        assert!(check("10 Juniper shares", Lang::En, &[]).is_ok()); // small count
        assert!(check("25 Juniper shares", Lang::En, &[]).is_err());
        assert_eq!(check("29 Oct 2026", Lang::En, &[]), Ok(()));
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
