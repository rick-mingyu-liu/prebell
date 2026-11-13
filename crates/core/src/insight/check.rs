//! Code checks an insight passes before the judge sees it: lengths and
//! form, cited ids, numbers, dates, relative time, fiscal labels, outlet
//! attribution and a refusal list of advice and prediction wording.

use std::sync::OnceLock;

use chrono::{Datelike, NaiveDate};
use regex::Regex;

use super::{EvidenceKind, Insight, Line, Pack};
use crate::ipo::news::visible;
use crate::numbers::{self, Lang};

pub const HOOK_WORDS_EN: usize = 8;
pub const HOOK_CHARS_ZH: usize = 14;
pub const WATCH_WORDS_EN: usize = 40;
pub const WATCH_CHARS_ZH: usize = 80;

/// Advice and prediction wording, refused even when the evidence uses it.
/// English entries match as whole words (a hyphen joins a word, so
/// "sell-off" and "buyback" are not "sell" and "buy"), ignoring case.
const REFUSED_EN: &[&str] = &[
    "buy",
    "sell",
    "hold",
    "should",
    "must",
    "recommend",
    "will rise",
    "will fall",
    "will rally",
    "will drop",
    "set to surge",
    "price target",
    "target price",
    "overweight",
    "underweight",
    "outperform",
    "underperform",
    "analysts expect",
    "undervalued",
    "overvalued",
    "bullish",
    "bearish",
];
const REFUSED_ZH: &[&str] = &[
    "买入",
    "卖出",
    "持有",
    "建议",
    "应当",
    "应该",
    "目标价",
    "看涨",
    "看跌",
    "看好",
    "看空",
    "增持",
    "减持",
    "跑赢",
    "跑输",
    "低估",
    "高估",
    "分析师预计",
    "将上涨",
    "将下跌",
];
const RELATIVE_ZH: &[&str] = &["下周", "下个月", "明天", "本周", "近期"];

/// `(English name as in ipo::news::OUTLETS, other English forms, Chinese forms)`.
/// The zh line may name an outlet in either script.
const OUTLET_NAMES: &[(&str, &[&str], &[&str])] = &[
    ("Reuters", &[], &["路透社", "路透"]),
    ("Bloomberg", &[], &["彭博社", "彭博"]),
    (
        "The Wall Street Journal",
        &["Wall Street Journal", "WSJ"],
        &["华尔街日报"],
    ),
    ("Financial Times", &["FT"], &["金融时报"]),
    ("CNBC", &[], &["美国消费者新闻与商业频道"]),
    (
        "The New York Times",
        &["New York Times", "NYT"],
        &["纽约时报"],
    ),
    ("MarketWatch", &[], &["市场观察"]),
    ("Barron's", &["Barron’s", "Barrons"], &["巴伦周刊"]),
    ("Axios", &[], &[]),
    ("Fortune", &[], &["《财富》", "财富杂志"]),
    ("Business Insider", &[], &["商业内幕"]),
    ("AP", &["Associated Press"], &["美联社"]),
    ("The Information", &[], &[]),
    ("TechCrunch", &[], &[]),
    ("The Verge", &[], &[]),
    ("Semafor", &[], &[]),
    (
        "Investor's Business Daily",
        &["Investor’s Business Daily", "IBD"],
        &["投资者商业日报"],
    ),
    ("The Economist", &["Economist"], &["经济学人"]),
    ("The Washington Post", &["Washington Post"], &["华盛顿邮报"]),
    ("The Guardian", &["Guardian"], &["卫报"]),
    ("BBC", &[], &["英国广播公司"]),
    ("South China Morning Post", &["SCMP"], &["南华早报"]),
    ("Nikkei Asia", &["Nikkei"], &["日经亚洲"]),
    ("Caixin Global", &["Caixin"], &["财新"]),
];

type OutletRow = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

fn row(outlet: &str) -> Option<&'static OutletRow> {
    OUTLET_NAMES.iter().find(|(name, _, _)| *name == outlet)
}

/// Accepted names for an outlet in a language ("Reuters" → zh ["路透社",
/// "路透", "Reuters"]). An outlet outside the table is accepted by its
/// English name or host in both languages.
pub fn outlet_names(outlet: &str, lang: Lang) -> Vec<String> {
    let Some((name, en, zh)) = row(outlet) else {
        return vec![outlet.to_string()];
    };
    let mut out: Vec<String> = Vec::new();
    if lang == Lang::Zh {
        out.extend(zh.iter().map(|s| s.to_string()));
    }
    out.push(name.to_string());
    out.extend(en.iter().map(|s| s.to_string()));
    out
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

/// Full-width digits as ASCII, so the number and date checks read them.
fn ascii_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
            _ => c,
        })
        .collect()
}

fn has_digit(s: &str) -> bool {
    s.chars()
        .any(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

/// A figure written in Chinese numerals ("七十美元", "三亿").
fn chinese_numeral_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"[一二三四五六七八九十百千万亿两]+(?:美元|亿|万|%|％|个百分点)"))
}

/// The first figure in `text` written in Chinese numerals. A run of unit
/// characters alone ("42亿美元" has "亿美元") is a unit after digits.
fn chinese_numeral(text: &str) -> Option<String> {
    chinese_numeral_re()
        .find_iter(text)
        .find(|m| {
            m.as_str().contains([
                '一', '二', '三', '四', '五', '六', '七', '八', '九', '十', '两',
            ])
        })
        .map(|m| m.as_str().to_string())
}

/// `phrase` in `text` as whole words: no letter, digit or hyphen joined on
/// either side. `fold` ignores ASCII case.
fn has_word(text: &str, phrase: &str, fold: bool) -> bool {
    let joined = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-');
    let (hay, needle) = if fold {
        (text.to_ascii_lowercase(), phrase.to_ascii_lowercase())
    } else {
        (text.to_string(), phrase.to_string())
    };
    hay.match_indices(&needle).any(|(at, m)| {
        !joined(hay[..at].chars().next_back()) && !joined(hay[at + m.len()..].chars().next())
    })
}

/// Whether `text` names `name`: Chinese names anywhere, English names as
/// whole words with their capitals ("The Information", not "the information").
fn names(text: &str, name: &str) -> bool {
    if name.chars().any(is_cjk) {
        text.contains(name)
    } else {
        has_word(text, name, false)
    }
}

/// Byte offsets of the sentence-ending marks in `s`. A period ends a
/// sentence only at the end, or before a space and a capital or Chinese
/// character, and not after a single letter ("U.S. Steel").
fn sentence_ends(s: &str) -> Vec<usize> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    for (k, &(at, c)) in chars.iter().enumerate() {
        let end = match c {
            '。' | '！' | '？' | '!' | '?' => true,
            '.' => {
                let next = chars.get(k + 1).map(|x| x.1);
                let after = chars[k + 1..]
                    .iter()
                    .map(|x| x.1)
                    .find(|c| !c.is_whitespace());
                let initial = k >= 1
                    && chars[k - 1].1.is_alphabetic()
                    && (k == 1 || !chars[k - 2].1.is_alphanumeric());
                !initial
                    && match next {
                        None => true,
                        Some(n) if n.is_whitespace() => {
                            after.is_none_or(|a| a.is_uppercase() || is_cjk(a))
                        }
                        _ => false,
                    }
            }
            _ => false,
        };
        if end {
            out.push(at);
        }
    }
    out
}

fn one_sentence(watch: &str) -> bool {
    let body = watch.trim_end_matches(['"', '\'', '”', '’', ')', '）', '」', '』', ' ']);
    match sentence_ends(body).as_slice() {
        [] => true,
        [at] => body[*at..].chars().count() == 1,
        _ => false,
    }
}

const MONTH: &str = r"(Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)";

fn month_number(name: &str) -> u32 {
    const M: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let p = name.to_ascii_lowercase();
    M.iter()
        .position(|m| p.starts_with(m))
        .map_or(0, |i| i as u32 + 1)
}

/// Every calendar date written in `text`, in order: `None` for one that does
/// not exist ("November 31").
fn date_marks(text: &str, year_of: NaiveDate) -> Vec<Option<NaiveDate>> {
    static R: OnceLock<[Regex; 4]> = OnceLock::new();
    let res = R.get_or_init(|| {
        [
            re(r"\b((?:19|20)\d{2})-(\d{1,2})-(\d{1,2})\b"),
            re(&format!(
                r"\b{MONTH}\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?\b(?:,?\s+((?:19|20)\d{{2}})\b)?"
            )),
            re(&format!(
                r"\b(\d{{1,2}})(?:st|nd|rd|th)?\s+{MONTH}\b(?:,?\s+((?:19|20)\d{{2}})\b)?"
            )),
            re(r"(?:((?:19|20)\d{2})\s*年\s*)?(\d{1,2})\s*月\s*(\d{1,2})\s*[日号]"),
        ]
    });
    let text = ascii_digits(text);
    let num = |m: Option<regex::Match>| m.and_then(|m| m.as_str().parse::<i64>().ok());
    let mut found: Vec<(usize, usize, Option<NaiveDate>)> = Vec::new();
    for (k, r) in res.iter().enumerate() {
        for c in r.captures_iter(&text) {
            let whole = c.get(0).expect("match");
            let (y, m, d) = match k {
                0 => (num(c.get(1)), num(c.get(2)), num(c.get(3))),
                1 => (
                    num(c.get(3)),
                    Some(month_number(&c[1]) as i64),
                    num(c.get(2)),
                ),
                2 => (
                    num(c.get(3)),
                    Some(month_number(&c[2]) as i64),
                    num(c.get(1)),
                ),
                _ => (num(c.get(1)), num(c.get(2)), num(c.get(3))),
            };
            let y = y.unwrap_or(year_of.year() as i64);
            let date = match (m, d) {
                (Some(m), Some(d)) => NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32),
                _ => None,
            };
            found.push((whole.start(), whole.end(), date));
        }
    }
    found.sort_by_key(|&(s, e, _)| (s, std::cmp::Reverse(e)));
    let mut out = Vec::new();
    let mut last_end = 0;
    for (s, e, date) in found {
        if s < last_end {
            continue;
        }
        last_end = e;
        out.push(date);
    }
    out
}

/// Calendar dates written in `text` ("Nov 19", "November 19, 2026",
/// "11月19日", "2026年11月19日", "2026-11-19"), year taken from `year_of`
/// when absent. Every shape is read in both languages, as lines mix scripts.
pub fn dates_in(text: &str, _lang: Lang, year_of: NaiveDate) -> Vec<NaiveDate> {
    date_marks(text, year_of).into_iter().flatten().collect()
}

fn relative_en_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"(?i)\b(?:next\s+(?:week|month|quarter|year)|tomorrow|this\s+(?:week|month)|soon|in\s+the\s+coming\s+(?:days|weeks|months))\b")
    })
}

fn relative_zh_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"未来几(?:天|周|个月)"))
}

/// A fiscal label and the forms of it a cited text may hold, lowercase.
fn fiscal_labels(text: &str) -> Vec<(String, Vec<String>)> {
    static R: OnceLock<[Regex; 5]> = OnceLock::new();
    let res = R.get_or_init(|| {
        [
            re(r"(?i)\bQ([1-4])\b"),
            re(r"(?i)\bfiscal\s+(?:year\s+)?(20\d{2})\b"),
            re(r"(?i)\bFY\s?(\d{4}|\d{2})\b"),
            re(r"第([一二三四])季度"),
            re(r"(\d{4})\s*财年"),
        ]
    });
    const ORD: [&str; 4] = ["first", "second", "third", "fourth"];
    const ZH: [&str; 4] = ["一", "二", "三", "四"];
    let quarter = |n: usize| {
        vec![
            format!("q{n}"),
            format!("{} quarter", ORD[n - 1]),
            format!("{n}q"),
            format!("第{}季度", ZH[n - 1]),
        ]
    };
    let year = |y: &str| {
        let y = if y.len() == 2 {
            format!("20{y}")
        } else {
            y.to_string()
        };
        vec![
            format!("fiscal {y}"),
            format!("fiscal year {y}"),
            format!("fy{y}"),
            format!("fy {y}"),
            format!("fy{}", &y[2..]),
            format!("{y}财年"),
        ]
    };
    let text = ascii_digits(text);
    let mut out = Vec::new();
    for (k, r) in res.iter().enumerate() {
        for c in r.captures_iter(&text) {
            let forms = match k {
                0 => quarter(c[1].parse().expect("digit")),
                3 => quarter(ZH.iter().position(|z| *z == &c[1]).expect("listed") + 1),
                _ => year(&c[1]),
            };
            out.push((c[0].to_string(), forms));
        }
    }
    out
}

/// Empty when the insight may go to the judge; else one reason per failure,
/// each starting "en:" or "zh:".
pub fn check(pack: &Pack, i: &Insight) -> Vec<String> {
    let mut out = Vec::new();
    for (lang, line, tag) in [(Lang::En, &i.en, "en"), (Lang::Zh, &i.zh, "zh")] {
        out.extend(
            check_line(pack, line, lang)
                .into_iter()
                .map(|e| format!("{tag}: {e}")),
        );
    }
    out
}

fn check_line(pack: &Pack, line: &Line, lang: Lang) -> Vec<String> {
    let mut bad = Vec::new();
    let hook = visible(line.hook.trim());
    let watch = visible(line.watch.trim());
    let both = format!("{hook} {watch}");

    // 1. Form.
    if hook.is_empty() || watch.is_empty() {
        bad.push("empty hook or watch line".to_string());
    }
    match lang {
        Lang::En => {
            let (h, w) = (
                hook.split_whitespace().count(),
                watch.split_whitespace().count(),
            );
            if h > HOOK_WORDS_EN {
                bad.push(format!("hook has {h} words (at most {HOOK_WORDS_EN})"));
            }
            if w > WATCH_WORDS_EN {
                bad.push(format!(
                    "watch line has {w} words (at most {WATCH_WORDS_EN})"
                ));
            }
            if both.chars().any(is_cjk) {
                bad.push("has Chinese characters".to_string());
            }
        }
        Lang::Zh => {
            let count = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
            let (h, w) = (count(&hook), count(&watch));
            if h > HOOK_CHARS_ZH {
                bad.push(format!("hook has {h} characters (at most {HOOK_CHARS_ZH})"));
            }
            if w > WATCH_CHARS_ZH {
                bad.push(format!(
                    "watch line has {w} characters (at most {WATCH_CHARS_ZH})"
                ));
            }
            if !hook.chars().any(is_cjk) || !watch.chars().any(is_cjk) {
                bad.push("has no Chinese characters".to_string());
            }
        }
    }
    if has_digit(&hook) {
        bad.push("hook has a digit".to_string());
    }
    if let Some(m) = chinese_numeral(&hook) {
        bad.push(format!("hook has a figure in Chinese numerals: {m}"));
    }
    if !one_sentence(&watch) {
        bad.push("watch line is not one sentence".to_string());
    }
    let markup = ['*', '`', '#', '<', '>', '[', ']', '\n', '\r'];
    if both.contains(markup) || both.contains("://") || both.to_ascii_lowercase().contains("www.") {
        bad.push("has markup, a link or a line break".to_string());
    }

    // 2. Cites.
    let mut cited = Vec::new();
    for id in &line.cites {
        match pack.evidence.iter().find(|e| e.id == *id) {
            Some(e) => cited.push(e),
            None => bad.push(format!("cites {id}, which is not in this mover's evidence")),
        }
    }
    if line.cites.is_empty() {
        bad.push("cites nothing".to_string());
    } else if !cited.is_empty() && cited.iter().all(|e| e.kind == EvidenceKind::AnalystAction) {
        bad.push("cites only an analyst action".to_string());
    }
    let cited_text: Vec<String> = cited.iter().map(|e| ascii_digits(&e.text)).collect();

    // 3. Numbers.
    let allowed: Vec<f64> = cited_text
        .iter()
        .flat_map(|t| crate::facts::quote_numbers(t))
        .collect();
    if let Err(nums) = numbers::check(&ascii_digits(&both), lang, &allowed) {
        bad.push(format!(
            "numbers not in the cited evidence: {}",
            nums.join(", ")
        ));
    }
    if let Some(m) = chinese_numeral(&both) {
        bad.push(format!("a figure in Chinese numerals: {m}"));
    }

    // 4. Dates: a year-less date may fall in the year before or after.
    let year_of = pack
        .evidence
        .iter()
        .find(|e| e.kind == EvidenceKind::Move)
        .or(pack.evidence.first())
        .and_then(|e| e.date);
    let marks = |y: Option<NaiveDate>| y.map_or_else(Vec::new, |y| date_marks(&both, y));
    let shifted = |years: i32| year_of.and_then(|y| y.with_year(y.year() + years));
    let variants = [marks(year_of), marks(shifted(-1)), marks(shifted(1))];
    let mut allowed_dates: Vec<NaiveDate> = pack.evidence.iter().filter_map(|e| e.date).collect();
    for y in [year_of, shifted(-1), shifted(1)].into_iter().flatten() {
        for t in &cited_text {
            allowed_dates.extend(dates_in(t, lang, y));
        }
    }
    let unanchored = year_of.is_none() && !date_marks(&both, NaiveDate::MIN).is_empty();
    let written = variants[0].len();
    for k in 0..written {
        let ok = variants.iter().any(|v| {
            v.get(k)
                .copied()
                .flatten()
                .is_some_and(|d| allowed_dates.contains(&d))
        });
        if !ok {
            bad.push("a date that is not in the evidence".to_string());
        }
    }
    if unanchored {
        bad.push("a date that is not in the evidence".to_string());
    }

    // 5. Relative time.
    let rel = relative_en_re()
        .find(&both)
        .map(|m| m.as_str().to_string())
        .or_else(|| {
            RELATIVE_ZH
                .iter()
                .find(|w| both.contains(**w))
                .map(|w| w.to_string())
        })
        .or_else(|| relative_zh_re().find(&both).map(|m| m.as_str().to_string()));
    if let Some(w) = rel {
        bad.push(format!("relative time: {w}"));
    }

    // 6. Fiscal labels.
    let cited_lower: Vec<String> = cited_text.iter().map(|t| t.to_lowercase()).collect();
    for (label, forms) in fiscal_labels(&both) {
        if !forms
            .iter()
            .any(|f| cited_lower.iter().any(|t| t.contains(f.as_str())))
        {
            bad.push(format!("fiscal label {label} is not in the cited evidence"));
        }
    }

    // 7. Attribution.
    let cited_outlets: Vec<&str> = cited
        .iter()
        .filter(|e| matches!(e.kind, EvidenceKind::News | EvidenceKind::AnalystAction))
        .filter_map(|e| e.outlet.as_deref())
        .collect();
    for o in &cited_outlets {
        if !outlet_names(o, lang).iter().any(|n| names(&watch, n)) {
            bad.push(format!("does not name {o}, the outlet of a cited item"));
        }
    }
    for (name, _, _) in OUTLET_NAMES {
        if cited_outlets.contains(name) {
            continue;
        }
        if outlet_names(name, lang).iter().any(|n| names(&both, n)) {
            bad.push(format!("names {name}, which no cited item is from"));
        }
    }

    // 8. Refusal list.
    for w in REFUSED_EN {
        if has_word(&both, w, true) {
            bad.push(format!("refused wording: {w}"));
        }
    }
    for w in REFUSED_ZH {
        if both.contains(w) {
            bad.push(format!("refused wording: {w}"));
        }
    }
    bad
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{Evidence, EvidenceKind, Insight, Line, Pack};
    use crate::Lang::{En, Zh};
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn ev(
        id: &str,
        kind: EvidenceKind,
        text: &str,
        outlet: Option<&str>,
        date: Option<&str>,
    ) -> Evidence {
        Evidence {
            id: id.into(),
            kind,
            text: text.into(),
            outlet: outlet.map(str::to_string),
            url: None,
            date: date.map(d),
        }
    }

    fn pack() -> Pack {
        use EvidenceKind::*;
        Pack {
            ticker: "AAPL".into(),
            name: "Apple".into(),
            evidence: vec![
                ev("e1", Move, "Apple (AAPL) rose 4.20% in premarket trading.", None, Some("2026-11-12")),
                ev(
                    "e2",
                    News,
                    "Apple said it expects third quarter revenue of $4.2 billion and a $1.0 billion buyback.",
                    Some("Reuters"),
                    None,
                ),
                ev("e3", AnalystAction, "Morgan Stanley upgraded Apple to overweight.", Some("CNBC"), None),
                ev(
                    "e4",
                    News,
                    "Apple plans to open a factory in Texas next year, the company said.",
                    Some("The Wall Street Journal"),
                    None,
                ),
                ev(
                    "e5",
                    ReportDate,
                    "Apple is scheduled to report results on November 19, 2026 (earnings calendar).",
                    None,
                    Some("2026-11-19"),
                ),
                ev(
                    "e6",
                    CompanyGuidance,
                    "the Company expects gross margin of 70 percent in fiscal 2027",
                    None,
                    None,
                ),
            ],
        }
    }

    fn line(hook: &str, watch: &str, cites: &[&str]) -> Line {
        Line {
            hook: hook.into(),
            watch: watch.into(),
            cites: cites.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn en_ok() -> Line {
        line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion, and its November 19 report will show whether that plan holds.",
            &["e2", "e5"],
        )
    }

    fn zh_ok() -> Line {
        line(
            "业绩指引成为下一道考验",
            "据路透社报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        )
    }

    fn insight(en: Line, zh: Line) -> Insight {
        Insight {
            ticker: "AAPL".into(),
            en,
            zh,
        }
    }

    /// Failures for an insight with `en` (and the clean zh line).
    fn en(l: Line) -> Vec<String> {
        check(&pack(), &insight(l, zh_ok()))
    }

    /// Failures for an insight with `zh` (and the clean en line).
    fn zh(l: Line) -> Vec<String> {
        check(&pack(), &insight(en_ok(), l))
    }

    fn refused(failures: &[String], what: &str) -> bool {
        failures.iter().any(|f| f.contains(what))
    }

    #[test]
    fn a_clean_insight_passes() {
        assert_eq!(
            check(&pack(), &insight(en_ok(), zh_ok())),
            Vec::<String>::new()
        );
    }

    #[test]
    fn lengths_are_enforced_in_both_languages() {
        let f = en(long_hook());
        assert!(refused(&f, "hook"), "{f:?}");
        let watch = format!(
            "Reuters reports Apple expects revenue of $4.2 billion{}.",
            " and more".repeat(20)
        );
        let f = en(line("Guidance sets the next test", &watch, &["e2"]));
        assert!(refused(&f, "words"), "{f:?}");

        let f = zh(line(
            "苹果业绩指引成为下一道重要考验",
            &zh_ok().watch,
            &["e2", "e5"],
        ));
        assert!(refused(&f, "hook"), "{f:?}");
        let watch = format!("据路透社报道，苹果预计营收42亿美元{}。", "并且".repeat(40));
        let f = zh(line("业绩指引成为下一道考验", &watch, &["e2"]));
        assert!(refused(&f, "characters"), "{f:?}");
        // Eight words and fourteen characters are fine.
        assert!(en(line(
            "One two three four five six seven eight",
            &en_ok().watch,
            &["e2", "e5"]
        ))
        .is_empty());
        assert!(zh(line(
            "苹果业绩指引成为下一道重要考",
            &zh_ok().watch,
            &["e2", "e5"]
        ))
        .is_empty());
    }

    fn long_hook() -> Line {
        line(
            "One two three four five six seven eight nine",
            &en_ok().watch,
            &["e2", "e5"],
        )
    }

    #[test]
    fn a_hook_with_a_digit_is_refused() {
        let f = en(line(
            "Q3 guidance sets the test",
            &en_ok().watch,
            &["e2", "e5"],
        ));
        assert!(refused(&f, "digit"), "{f:?}");
        let f = zh(line("第３季度指引成考验", &zh_ok().watch, &["e2", "e5"]));
        assert!(refused(&f, "digit"), "{f:?}");
        let f = zh(line("七十美元指引成考验", &zh_ok().watch, &["e2", "e5"]));
        assert!(refused(&f, "Chinese numeral"), "{f:?}");
    }

    #[test]
    fn two_sentences_are_refused() {
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion. The November 19 report will show more.",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "one sentence"), "{f:?}");
        let f = zh(line(
            "业绩指引成为下一道考验",
            "据路透社报道，苹果预计营收42亿美元。11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "one sentence"), "{f:?}");
        // Decimals and initials are not sentence ends.
        assert!(en(line(
            "Guidance sets the next test",
            "Reuters reports the U.S. company expects revenue of $4.2 billion before its November 19 report.",
            &["e2", "e5"],
        ))
        .is_empty());
    }

    #[test]
    fn markup_links_and_scripts_are_refused() {
        let f = en(line(
            "**Guidance** sets the test",
            &en_ok().watch,
            &["e2", "e5"],
        ));
        assert!(refused(&f, "markup"), "{f:?}");
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion, see https://reuters.com/x for more.",
            &["e2"],
        ));
        assert!(refused(&f, "markup"), "{f:?}");
        let f = en(line(
            "Guidance 指引 sets the test",
            &en_ok().watch,
            &["e2", "e5"],
        ));
        assert!(refused(&f, "Chinese"), "{f:?}");
        let f = zh(line(
            "Guidance sets the test",
            &zh_ok().watch,
            &["e2", "e5"],
        ));
        assert!(refused(&f, "no Chinese"), "{f:?}");
    }

    #[test]
    fn a_number_not_in_the_cited_items_is_refused() {
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $5.1 billion, and its November 19 report will show whether that holds.",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "$5.1 billion"), "{f:?}");
        // A number from an item the line does not cite is refused too.
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion, with a gross margin target of 70 percent.",
            &["e2"],
        ));
        assert!(refused(&f, "70 percent"), "{f:?}");
    }

    #[test]
    fn chinese_numerals_for_a_figure_are_refused() {
        let f = zh(line(
            "业绩指引成为下一道考验",
            "公司给出的指引为七十美元，后续财报将检验这一水平。",
            &["e6"],
        ));
        assert!(refused(&f, "Chinese numeral"), "{f:?}");
    }

    #[test]
    fn full_width_digits_are_checked() {
        let ok = zh(line(
            "业绩指引成为下一道考验",
            "公司给出的毛利率指引为７０％，后续财报将检验这一水平。",
            &["e6"],
        ));
        assert!(ok.is_empty(), "{ok:?}");
        let bad = zh(line(
            "业绩指引成为下一道考验",
            "公司给出的毛利率指引为７１％，后续财报将检验这一水平。",
            &["e6"],
        ));
        assert!(refused(&bad, "71"), "{bad:?}");
        let ok = zh(line(
            "业绩指引成为下一道考验",
            "公司给出的指引为７０美元，后续财报将检验这一水平。",
            &["e6"],
        ));
        assert!(ok.is_empty(), "{ok:?}");
        let bad = zh(line(
            "业绩指引成为下一道考验",
            "公司给出的指引为７１美元，后续财报将检验这一水平。",
            &["e6"],
        ));
        assert!(refused(&bad, "71"), "{bad:?}");
    }

    #[test]
    fn chinese_and_english_dates_are_recognised() {
        let y = d("2026-11-12");
        assert_eq!(dates_in("将于11月19日", Zh, y), [d("2026-11-19")]);
        assert_eq!(dates_in("2026年11月19日公布", Zh, y), [d("2026-11-19")]);
        assert_eq!(dates_in("２０２７年１月５日", Zh, y), [d("2027-01-05")]);
        assert_eq!(
            dates_in("reports on Nov 19 after the close", En, y),
            [d("2026-11-19")]
        );
        assert_eq!(
            dates_in("on November 19, 2026, it", En, y),
            [d("2026-11-19")]
        );
        assert_eq!(dates_in("on Nov. 19th", En, y), [d("2026-11-19")]);
        assert_eq!(dates_in("due 2026-11-19", En, y), [d("2026-11-19")]);
        assert_eq!(dates_in("on 19 November 2026", En, y), [d("2026-11-19")]);
        assert_eq!(
            dates_in("Nov 19 and Dec 3, 2027", En, y),
            [d("2026-11-19"), d("2027-12-03")]
        );
        // A month and a year are not a day; neither are figures.
        assert!(dates_in("in March 2027 it expects $4.2 billion", En, y).is_empty());
        assert!(dates_in("a 52-week high of 19.5", En, y).is_empty());
    }

    #[test]
    fn a_date_not_in_the_pack_is_refused() {
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion, and its November 20 report will show whether that holds.",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "date"), "{f:?}");
        let f = zh(line(
            "业绩指引成为下一道考验",
            "据路透社报道，苹果预计营收42亿美元，11月20日的财报将检验这一预期。",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "date"), "{f:?}");
        // The trading date (the move's date) is in the pack.
        assert!(en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion after its November 12 move.",
            &["e2"],
        ))
        .is_empty());
        // A date that does not exist is refused.
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion ahead of November 31.",
            &["e2"],
        ));
        assert!(refused(&f, "date"), "{f:?}");
    }

    #[test]
    fn relative_time_is_refused_in_both_languages() {
        for w in [
            "next week",
            "tomorrow",
            "this month",
            "soon",
            "in the coming weeks",
            "Next quarter",
        ] {
            let watch = format!(
                "Reuters reports Apple expects revenue of $4.2 billion, a figure to track {w}."
            );
            let f = en(line("Guidance sets the next test", &watch, &["e2"]));
            assert!(refused(&f, "relative time"), "{w}: {f:?}");
        }
        for w in ["下周", "下个月", "明天", "本周", "近期", "未来几个月"] {
            let watch = format!("据路透社报道，苹果预计营收42亿美元，{w}值得关注。");
            let f = zh(line("业绩指引成为下一道考验", &watch, &["e2"]));
            assert!(refused(&f, "relative time"), "{w}: {f:?}");
        }
    }

    #[test]
    fn a_fiscal_label_must_be_in_the_cited_text() {
        // e2 says "third quarter".
        let ok = zh(line(
            "业绩指引成为下一道考验",
            "据路透社报道，苹果预计第三季度营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        ));
        assert!(ok.is_empty(), "{ok:?}");
        let f = zh(line(
            "业绩指引成为下一道考验",
            "据路透社报道，苹果预计第四季度营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "fiscal label"), "{f:?}");
        let ok = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects Q3 revenue of $4.2 billion, and its November 19 report will show whether that holds.",
            &["e2", "e5"],
        ));
        assert!(ok.is_empty(), "{ok:?}");
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects Q4 revenue of $4.2 billion, and its November 19 report will show whether that holds.",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "fiscal label"), "{f:?}");
        // e6 says "fiscal 2027"; 2027财年 matches it, fiscal 2028 does not.
        assert!(zh(line(
            "业绩指引成为下一道考验",
            "公司预计2027财年毛利率为70%，财报将检验这一目标。",
            &["e6"]
        ))
        .is_empty());
        let f = en(line(
            "Guidance sets the next test",
            "The company expects a gross margin of 70 percent in fiscal 2028.",
            &["e6"],
        ));
        assert!(refused(&f, "fiscal label"), "{f:?}");
    }

    #[test]
    fn news_must_name_its_outlet() {
        let watch = |who: &str| {
            format!("{who} Apple plans to open a factory in Texas, a project to track at its November 19 report.")
        };
        assert!(en(line(
            "A Texas plant comes into view",
            &watch("The Wall Street Journal reports"),
            &["e4", "e5"]
        ))
        .is_empty());
        assert!(en(line(
            "A Texas plant comes into view",
            &watch("WSJ reports"),
            &["e4", "e5"]
        ))
        .is_empty());
        let f = en(line(
            "A Texas plant comes into view",
            &watch("It is reported"),
            &["e4", "e5"],
        ));
        assert!(refused(&f, "The Wall Street Journal"), "{f:?}");
        let f = en(line(
            "A Texas plant comes into view",
            &watch("Bloomberg reports"),
            &["e4", "e5"],
        ));
        assert!(refused(&f, "The Wall Street Journal"), "{f:?}");
        assert!(refused(&f, "Bloomberg"), "{f:?}");
        // Naming a second outlet nothing cited comes from is refused too.
        let f = en(line(
            "A Texas plant comes into view",
            &watch("WSJ and Bloomberg report"),
            &["e4", "e5"],
        ));
        assert!(refused(&f, "Bloomberg"), "{f:?}");

        assert!(en(en_ok()).is_empty());
        assert!(zh(zh_ok()).is_empty());
        // An invisible character inside the name still names the outlet.
        let hidden = en_ok().watch.replace("Reuters", "Reu\u{200b}ters");
        assert!(en(line("Guidance sets the next test", &hidden, &["e2", "e5"])).is_empty());
        let hidden = zh_ok().watch.replace("路透社", "路\u{200b}透社");
        assert!(zh(line("业绩指引成为下一道考验", &hidden, &["e2", "e5"])).is_empty());
        assert!(zh(line(
            "业绩指引成为下一道考验",
            "Reuters报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"]
        ))
        .is_empty());
        let f = zh(line(
            "业绩指引成为下一道考验",
            "据彭博社报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "Reuters"), "{f:?}");
        let f = zh(line(
            "业绩指引成为下一道考验",
            "苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "Reuters"), "{f:?}");
    }

    #[test]
    fn outlet_names_cover_both_languages() {
        assert_eq!(outlet_names("Reuters", Zh), ["路透社", "路透", "Reuters"]);
        assert!(outlet_names("The Wall Street Journal", En).contains(&"WSJ".to_string()));
        assert!(!outlet_names("The Wall Street Journal", En).contains(&"华尔街日报".to_string()));
        assert!(outlet_names("The Wall Street Journal", Zh).contains(&"华尔街日报".to_string()));
        assert_eq!(
            outlet_names("marketscreener.com", Zh),
            ["marketscreener.com"]
        );
        // Every allowlisted outlet has a row.
        for (_, name) in crate::ipo::news::OUTLETS {
            assert!(row(name).is_some(), "{name}");
        }
    }

    #[test]
    fn citing_only_an_analyst_action_is_refused() {
        let f = en(line(
            "Morgan Stanley turns positive",
            "CNBC reports Morgan Stanley upgraded Apple, ahead of its November 19 report.",
            &["e3"],
        ));
        assert!(refused(&f, "analyst action"), "{f:?}");
        let f = en(line("Guidance sets the next test", &en_ok().watch, &[]));
        assert!(refused(&f, "cites nothing"), "{f:?}");
    }

    #[test]
    fn refusal_words_are_refused_even_when_the_evidence_uses_them() {
        let f = en(line(
            "Guidance sets the next test",
            "CNBC reports Morgan Stanley rates Apple overweight, and Reuters reports revenue of $4.2 billion.",
            &["e2", "e3"],
        ));
        assert!(refused(&f, "overweight"), "{f:?}");
        let f = en(line(
            "A buy signal before results",
            &en_ok().watch,
            &["e2", "e5"],
        ));
        assert!(refused(&f, "buy"), "{f:?}");
        let f = en(line(
            "Guidance sets the next test",
            "Reuters reports Apple expects revenue of $4.2 billion, so shares will rise into the November 19 report.",
            &["e2", "e5"],
        ));
        assert!(refused(&f, "will rise"), "{f:?}");
        let f = zh(line("机构看好业绩指引", &zh_ok().watch, &["e2", "e5"]));
        assert!(refused(&f, "看好"), "{f:?}");
        // Whole words only: a buyback and a sell-off are not advice.
        assert!(en(line(
            "Buyback plan sets the test",
            "Reuters reports a $1.0 billion buyback after the sell-off, a plan to track at the November 19 report.",
            &["e2", "e5"],
        ))
        .is_empty());
    }

    #[test]
    fn unknown_cite_ids_are_refused() {
        let f = en(line(
            "Guidance sets the next test",
            &en_ok().watch,
            &["e2", "e99"],
        ));
        assert!(refused(&f, "e99"), "{f:?}");
    }

    #[test]
    fn failures_say_which_language() {
        let f = zh(line(
            "业绩指引成为下一道考验",
            "苹果预计营收51亿美元，下周值得关注。",
            &["e2", "e9"],
        ));
        assert!(f.len() >= 3, "{f:?}");
        assert!(f.iter().all(|x| x.starts_with("zh: ")), "{f:?}");
        let f = en(line(
            "Guidance sets the next test",
            "Apple expects revenue of $5.1 billion soon.",
            &["e2"],
        ));
        assert!(f.len() >= 3, "{f:?}");
        assert!(f.iter().all(|x| x.starts_with("en: ")), "{f:?}");
    }
}
