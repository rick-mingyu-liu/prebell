//! One language's IPO write-up from a milestone's facts bundle
//! (`milestone::IpoArticleFacts`). The model writes the title, the summary
//! and two prose sections; code renders everything else: the figures table,
//! the quoted news, the next step, the sources and the disclaimer.
//!
//! What keeps PreBell's own voice off valuation, the listing date, demand
//! and opinion is a set of allow-shapes, with a word list only as a second
//! line:
//! 1. **The model never sees a news item.** Its input is the filing side of
//!    the bundle (`model_input`); reported news reaches the page only through
//!    the code-rendered "What has been reported" list, word for word and
//!    attributed. The model may not write quotation marks or an outlet name.
//! 2. **Every number is a filing figure** (cover facts, the previous range,
//!    the first session, and code's derived offering sizes), within the
//!    number checker's rounding, and every year one of the filing dates'.
//! 3. **Every date is a filing date** (`allowed_dates`), and a month is
//!    written only inside such a date: no "in November", no weekday, no
//!    "next week".
//! 4. **Every sentence states a filing fact**: it holds a figure, an allowed
//!    date, a form name, the symbol or the exchange (`anchored`). A sentence
//!    of comment alone ("Demand looks strong.") has none.
//! 5. **The headline states the milestone with its figure** (`check_title`).
//!
//! Then the word lists (`EN_REFUSED`, `ZH_REFUSED`) refuse valuation,
//! demand, judgment, inference, prediction, advice, expectations and
//! attribution words in the model's text.

use std::collections::HashSet;
use std::sync::OnceLock;

use chrono::{Datelike, NaiveDate};
use regex::Regex;
use serde_json::{json, Value};

use super::milestone::{IpoArticleFacts, Milestone, QuotedItem};
use super::news::OUTLETS;
use super::FORMS;
use crate::numbers::{check_allowed, extract, Allowed, Lang};
use crate::premarket::draft::{
    is_setext_underline, match_sections, one_line, reject_markup, sections_line, Draft,
};
use crate::providers::{LlmProvider, ProviderError};

pub const CONTENT_TYPE: &str = "ipo";
pub const DISCLAIMER_EN: &str = "This IPO write-up is generated automatically from SEC filings and public news reports, for information only. It is not investment advice.";
pub const DISCLAIMER_ZH: &str =
    "本文根据美国证监会文件和公开新闻报道自动生成，仅供参考，不构成投资建议。";

const PROMPT_EN: &str = include_str!("../../prompts/ipo_v1_en.md");
const PROMPT_ZH: &str = include_str!("../../prompts/ipo_v1_zh.md");
const ATTEMPTS: usize = 2;
/// Sentences one prose section may hold.
const MAX_SENTENCES: usize = 4;

/// `posts.template_id`: `ipo-<milestone>`.
pub fn template_id(m: Milestone) -> String {
    format!("ipo-{}", m.as_str())
}

/// The two sections the model writes, in order.
pub fn headings(lang: Lang) -> [&'static str; 2] {
    match lang {
        Lang::En => ["What happened", "What the filing says"],
        Lang::Zh => ["发生了什么", "文件披露了什么"],
    }
}

fn pick(lang: Lang, en: &'static str, zh: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

// ---------------------------------------------------------------- format

fn trim_dec(v: f64, max: usize) -> String {
    let s = format!("{v:.max$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

fn grouped(v: f64) -> String {
    let int = format!("{:.0}", v.abs());
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A per-share price: "$70.00" / "70.00美元".
fn price(v: f64, lang: Lang) -> String {
    match lang {
        Lang::En => format!("${v:.2}"),
        Lang::Zh => format!("{v:.2}美元"),
    }
}

/// A range: "$70.00–$78.00" / "70.00–78.00美元".
fn range(lo: f64, hi: f64, lang: Lang) -> String {
    match lang {
        Lang::En => format!("${lo:.2}–${hi:.2}"),
        Lang::Zh => format!("{lo:.2}–{hi:.2}美元"),
    }
}

/// A dollar amount: "$57.75 billion" / "577.5亿美元".
fn money(v: f64, lang: Lang) -> String {
    match lang {
        Lang::En if v >= 1e9 => format!("${} billion", trim_dec(v / 1e9, 2)),
        Lang::En if v >= 1e6 => format!("${} million", trim_dec(v / 1e6, 1)),
        Lang::En => format!("${}", grouped(v)),
        Lang::Zh if v >= 1e8 => format!("{}亿美元", trim_dec(v / 1e8, 2)),
        Lang::Zh if v >= 1e4 => format!("{}万美元", trim_dec(v / 1e4, 1)),
        Lang::Zh => format!("{}美元", grouped(v)),
    }
}

fn shares(v: f64, lang: Lang) -> String {
    match lang {
        Lang::En => grouped(v),
        Lang::Zh => format!("{}股", grouped(v)),
    }
}

/// "Nov 10, 2026" / "2026年11月10日".
pub fn day(s: &str, lang: Lang) -> String {
    match s.parse::<NaiveDate>() {
        Ok(d) => match lang {
            Lang::En => d.format("%b %-d, %Y").to_string(),
            Lang::Zh => format!("{}年{}月{}日", d.year(), d.month(), d.day()),
        },
        Err(_) => s.to_string(),
    }
}

/// Markdown special characters in quoted or stored text, escaped.
fn escape_md(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(one.len());
    for c in one.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn name_of(f: &IpoArticleFacts, lang: Lang) -> &str {
    match lang {
        Lang::Zh => f.name_zh.as_deref().unwrap_or(&f.name),
        Lang::En => &f.name,
    }
}

/// The two lines on the post's flat tile, by code.
pub fn cover_lines(f: &IpoArticleFacts, lang: Lang) -> [String; 2] {
    let first = format!("{} IPO", name_of(f, lang));
    let lo_hi = f.num("price_low").zip(f.num("price_high"));
    let second = match (f.milestone, lang) {
        (Milestone::Filed, _) => pick(lang, "Filed for IPO", "递交IPO申请").to_string(),
        (Milestone::RangeSet, Lang::En) => lo_hi.map_or("Price range set".into(), |(a, b)| {
            format!("Price range {}", range(a, b, lang))
        }),
        (Milestone::RangeSet, Lang::Zh) => lo_hi.map_or("公布价格区间".into(), |(a, b)| {
            format!("价格区间 {}", range(a, b, lang))
        }),
        (Milestone::RangeChanged, Lang::En) => lo_hi.map_or("New price range".into(), |(a, b)| {
            format!("New price range {}", range(a, b, lang))
        }),
        (Milestone::RangeChanged, Lang::Zh) => lo_hi
            .map_or("调整价格区间".into(), |(a, b)| {
                format!("新价格区间 {}", range(a, b, lang))
            }),
        (Milestone::Priced, _) => {
            f.num("price_final")
                .map_or(
                    pick(lang, "IPO priced", "完成定价").to_string(),
                    |p| match lang {
                        Lang::En => format!("Priced at {}", price(p, lang)),
                        Lang::Zh => format!("定价 {}", price(p, lang)),
                    },
                )
        }
        (Milestone::Listed, _) => f.session.as_ref().map_or(
            pick(lang, "First day of trading", "首个交易日").to_string(),
            |s| match lang {
                Lang::En => format!("First day close {}", price(s.close, lang)),
                Lang::Zh => format!("首日收盘 {}", price(s.close, lang)),
            },
        ),
        (Milestone::Withdrawn, _) => pick(lang, "IPO withdrawn", "撤回IPO申请").to_string(),
    };
    [first, second]
}

// ---------------------------------------------------------------- input

fn means(m: Milestone) -> &'static str {
    match m {
        Milestone::Filed => "The company filed its first public registration statement for the IPO.",
        Milestone::RangeSet => "The company's registration statement states a price range for the shares for the first time.",
        Milestone::RangeChanged => "A later amendment changed the price range or the number of shares from the previous range filing (`previous`).",
        Milestone::Priced => "The company filed its final prospectus with the offering price.",
        Milestone::Listed => "The shares completed their first regular trading session.",
        Milestone::Withdrawn => "The company filed to withdraw its registration statement.",
    }
}

fn form_label(form: &str) -> &str {
    FORMS
        .iter()
        .find(|(f, _, _)| *f == form)
        .map_or(form, |(_, en, _)| en)
}

/// What the model is given: the filing side of the bundle only, never a
/// news item (module docs, 1).
pub fn model_input(f: &IpoArticleFacts) -> Value {
    let derived: Vec<Value> = f
        .derived
        .iter()
        .map(|d| {
            json!({
                "kind": d.kind,
                "value_usd": d.value_usd,
                "computed_as": format!("{} shares offered × ${:.2} per share", grouped(d.shares), d.price),
            })
        })
        .collect();
    let mut dates: Vec<String> = allowed_dates(f)
        .into_iter()
        .map(|d| d.to_string())
        .collect();
    dates.sort();
    json!({
        "company": f.name,
        "company_zh": f.name_zh,
        "symbol": f.symbol,
        "exchange": f.exchange,
        "milestone": f.milestone.as_str(),
        "milestone_means": means(f.milestone),
        "filing": f.filing.as_ref().map(|x| json!({
            "form": x.form,
            "form_label": form_label(&x.form),
            "filing_date": x.filing_date,
        })),
        "cover_facts": f.cover.iter().map(|c| json!({
            "kind": c.kind,
            "value": c.value_num.map(Value::from).or_else(|| c.value_text.clone().map(Value::from)),
            "quote": c.quote,
        })).collect::<Vec<_>>(),
        "previous": f.previous,
        "derived": derived,
        "first_session": f.session.as_ref().map(|s| json!({
            "session_date": s.session_date,
            "open": s.open,
            "close": s.close,
            "volume": s.volume,
            "offering_price": s.ipo_price,
            "change_from_offering_price_pct": s.change_pct,
        })),
        "filings": f.timeline.iter().map(|t| json!({
            "form": t.form,
            "filing_date": t.filing_date,
        })).collect::<Vec<_>>(),
        "dates_you_may_write": dates,
    })
}

// ---------------------------------------------------------------- allowed

/// The dates the model may write: the filings', the cover's own dates and
/// the first session's.
pub fn allowed_dates(f: &IpoArticleFacts) -> HashSet<NaiveDate> {
    let mut out: HashSet<NaiveDate> = HashSet::new();
    let mut add = |s: &str| {
        if let Ok(d) = s.parse() {
            out.insert(d);
        }
    };
    add(&f.as_of);
    if let Some(x) = &f.filing {
        add(&x.filing_date);
    }
    for k in ["cover_date", "prospectus_date", "trading_date"] {
        if let Some(t) = f.text(k) {
            add(t);
        }
    }
    if let Some(p) = &f.previous {
        add(&p.filing_date);
    }
    if let Some(s) = &f.session {
        add(&s.session_date);
    }
    for t in &f.timeline {
        add(&t.filing_date);
    }
    out
}

/// Every figure the model may write: the cover's numbers, the previous
/// range, the first session and code's derived figures. News figures are
/// not among them.
pub fn allowed_numbers(f: &IpoArticleFacts) -> Vec<f64> {
    let mut out: Vec<f64> = f.cover.iter().filter_map(|c| c.value_num).collect();
    out.extend(f.derived.iter().map(|d| d.value_usd));
    if let Some(p) = &f.previous {
        out.extend(
            [p.price_low, p.price_high, p.shares_offered]
                .into_iter()
                .flatten(),
        );
    }
    if let Some(s) = &f.session {
        out.extend(
            [s.open, Some(s.close), s.volume, s.ipo_price, s.change_pct]
                .into_iter()
                .flatten(),
        );
    }
    out
}

const MONTHS: &str = r"(?:Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sept?(?:ember)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)";

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
}

/// EN "Nov. 10, 2026", "November 10", "10 November 2026"; ZH
/// "2026年11月10日", "11月10日".
fn date_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(&format!(
            r"\b(?P<m1>{MONTHS})\.?\s+(?P<d1>\d{{1,2}})(?:st|nd|rd|th)?\b(?:,?\s+(?P<y1>\d{{4}}))?|\b(?P<d2>\d{{1,2}})\s+(?P<m2>{MONTHS})\b(?:,?\s+(?P<y2>\d{{4}}))?|(?:(?P<y3>\d{{4}})\s*年\s*)?(?P<m3>\d{{1,2}})\s*月\s*(?P<d3>\d{{1,2}})\s*[日号]"
        ))
    })
}

fn month_number(m: &str) -> Option<u32> {
    let m = m.to_ascii_lowercase();
    [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|p| m.starts_with(p))
    .map(|i| i as u32 + 1)
}

/// Whether a written date is one of `allowed` (a date without a year is
/// matched on month and day).
fn date_ok(c: &regex::Captures, allowed: &HashSet<NaiveDate>) -> bool {
    let get = |k: &str| c.name(k).map(|m| m.as_str());
    let (m, d, y) = if let Some(m) = get("m1") {
        (month_number(m), get("d1"), get("y1"))
    } else if let Some(m) = get("m2") {
        (month_number(m), get("d2"), get("y2"))
    } else {
        (get("m3").and_then(|m| m.parse().ok()), get("d3"), get("y3"))
    };
    let (Some(m), Some(d)) = (m, d.and_then(|d| d.parse::<u32>().ok())) else {
        return false;
    };
    let y: Option<i32> = y.and_then(|y| y.parse().ok());
    allowed
        .iter()
        .any(|a| a.month() == m && a.day() == d && y.is_none_or(|y| y == a.year()))
}

/// Month names (EN) or "月" (ZH, not "个月") left after the allowed dates
/// are taken out: a month without its day is a timing claim.
fn bare_month_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(r"\b(?:January|February|March|April|June|July|August|September|October|November|December|Jan|Feb|Mar|Apr|Jun|Jul|Aug|Sept?|Oct|Nov|Dec)\b|\bMay\s+\d|\d+\s*/\s*\d+\s*/\s*\d+|\b\d{4}-\d{2}-\d{2}\b|[^个]月|^月|\d+\s*号")
    })
}

// ---------------------------------------------------------------- words

/// English words the model's text may not hold (whole words, any case):
/// valuation, demand, judgment, inference, prediction and timing, advice,
/// expectations, attribution. A second line behind the allow-shapes.
const EN_REFUSED: &str = r"(?i)\b(?:valu\w*|worth|market\s+cap\w*|capitali[sz]ation|demand\w*|oversubscri\w*|appetite|enthusias\w*|eager\w*|frenz\w*|fervou?r|hype\w*|buzz\w*|hot|hottest|anticipat\w*|closely\s+watched|blockbuster|sought[- ]after|popular\w*|interest\s+(?:from|in|among)|strong\w*|weak\w*|robust|solid|impressive|attractive|cheap\w*|expensive|richly|bargain\w*|premium|discount\w*|bullish|bearish|optimis\w*|pessimis\w*|confiden\w*|risky|success\w*|landmark|historic\w*|record\w*|largest|biggest|ever|most|best|worst|huge|massive|giant|mega\w*|suggest\w*|signal\w*|indicat\w*|sign\s+of|reflect\w*|impl(?:y|ies|ied)|means?\s+that|shows?\s+that|underscor\w*|highlight\w*|points?\s+to|will|would|could|might|may|likely|unlikely|expect\w*|forecast\w*|predict\w*|project(?:s|ed|ion|ions)?|estimat\w*|poised|set\s+to|on\s+track|plans?\s+to|planning|aims?\s+to|intends?\s+to|soon|upcoming|coming|later|eventually|future|today|tomorrow|yesterday|tonight|this\s+(?:week|month|year)|next\s+(?:week|month|year)|monday|tuesday|wednesday|thursday|friday|saturday|sunday|should|ought|recommend\w*|advi[cs]\w*|opportunit\w*|buy\w*|consensus|beat\w*|miss\w*|exceed\w*|surpass\w*|outperform\w*|underperform\w*|according\s+to|report\w*|sources?|people\s+familiar|media|press|rumou?r\w*|analyst\w*|wall\s+street)\b";

/// Chinese words refused in the model's text, as plain text.
const ZH_REFUSED: &[&str] = &[
    "估值",
    "市值",
    "价值",
    "值得",
    "身价",
    "需求",
    "认购",
    "追捧",
    "热捧",
    "热门",
    "火爆",
    "热度",
    "抢手",
    "备受",
    "瞩目",
    "关注",
    "期待",
    "热情",
    "狂热",
    "强劲",
    "疲软",
    "稳健",
    "亮眼",
    "出色",
    "强势",
    "便宜",
    "昂贵",
    "溢价",
    "折价",
    "看涨",
    "看跌",
    "乐观",
    "悲观",
    "信心",
    "成功",
    "失败",
    "历史性",
    "创纪录",
    "纪录",
    "最大",
    "之最",
    "有史以来",
    "史上",
    "巨额",
    "重磅",
    "表明",
    "显示出",
    "意味着",
    "反映",
    "凸显",
    "预示",
    "暗示",
    "标志着",
    "将于",
    "将会",
    "将在",
    "将以",
    "将成为",
    "将要",
    "即将",
    "会在",
    "会于",
    "会以",
    "或会",
    "可能",
    "或将",
    "有望",
    "料将",
    "预计",
    "预期",
    "估计",
    "预测",
    "计划于",
    "计划在",
    "打算",
    "拟于",
    "未来",
    "今天",
    "今日",
    "明天",
    "昨天",
    "本周",
    "下周",
    "本月",
    "下月",
    "今年",
    "明年",
    "年内",
    "年底",
    "近期",
    "近日",
    "日内",
    "星期",
    "周一",
    "周二",
    "周三",
    "周四",
    "周五",
    "周六",
    "周日",
    "建议",
    "应该",
    "应当",
    "机会",
    "买入",
    "卖出",
    "不妨",
    "共识",
    "超出",
    "不及",
    "好于",
    "差于",
    "据报道",
    "报道",
    "据悉",
    "消息人士",
    "知情人士",
    "媒体",
    "传闻",
    "分析师",
    "华尔街",
];

/// Cover terms that hold a refused word but say nothing of the kind.
const EN_TERMS: &[&str] = &["underwriting discounts", "underwriting discount"];

fn en_refused_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(EN_REFUSED))
}

fn refused_word(text: &str) -> Option<String> {
    let mut t = text.to_lowercase();
    for term in EN_TERMS {
        t = t.replace(term, " ");
    }
    if let Some(m) = en_refused_re().find(&t) {
        return Some(m.as_str().to_string());
    }
    ZH_REFUSED
        .iter()
        .find(|w| text.contains(*w))
        .map(|w| w.to_string())
}

const QUOTE_MARKS: &[char] = &['"', '“', '”', '「', '」', '『', '』', '«', '»', '„'];

// ---------------------------------------------------------------- check

/// The model's text with the allowed dates, the form names, the symbol, the
/// exchange and the company's names replaced by placeholders, or the first
/// date that is not allowed.
fn masked(text: &str, f: &IpoArticleFacts) -> Result<String, String> {
    let allowed = allowed_dates(f);
    let mut bad = None;
    let mut out = date_re()
        .replace_all(text, |c: &regex::Captures| {
            if date_ok(c, &allowed) {
                " ⟨DATE⟩ ".to_string()
            } else {
                bad.get_or_insert_with(|| c[0].to_string());
                c[0].to_string()
            }
        })
        .into_owned();
    if let Some(b) = bad {
        return Err(format!(
            "the date {b:?} is not a filing date; write only dates from dates_you_may_write"
        ));
    }
    // Longest form first, so "S-1/A" is not read as "S-1" and "/A".
    let mut forms: Vec<&str> = FORMS.iter().map(|(f, _, _)| *f).collect();
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    for form in forms {
        out = replace_word(&out, form, " ⟨FORM⟩ ", true);
    }
    if let Some(x) = &f.exchange {
        out = replace_word(&out, x, " ⟨EXCH⟩ ", false);
    }
    for name in ["Nasdaq", "NASDAQ", "New York Stock Exchange", "NYSE"] {
        out = replace_word(&out, name, " ⟨EXCH⟩ ", true);
    }
    if let Some(s) = &f.symbol {
        out = replace_word(&out, s, " ⟨SYM⟩ ", true);
    }
    for n in [Some(f.name.as_str()), f.name_zh.as_deref()]
        .into_iter()
        .flatten()
    {
        out = replace_word(&out, n, " ⟨NAME⟩ ", true);
    }
    Ok(out)
}

/// `word` replaced where no ASCII letter or digit touches it (`exact`: as
/// written; else ASCII case-insensitive).
fn replace_word(text: &str, word: &str, with: &str, exact: bool) -> String {
    if word.is_empty() {
        return text.to_string();
    }
    let (hay, needle) = if exact {
        (text.to_string(), word.to_string())
    } else {
        (text.to_ascii_lowercase(), word.to_ascii_lowercase())
    };
    let glued = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    let mut out = String::with_capacity(text.len());
    let mut rest = 0;
    for (i, _) in hay.match_indices(&needle) {
        if i < rest
            || glued(text[..i].chars().next_back())
            || glued(text[i + word.len()..].chars().next())
        {
            continue;
        }
        out.push_str(&text[rest..i]);
        out.push_str(with);
        rest = i + word.len();
    }
    out.push_str(&text[rest..]);
    out
}

/// Words a period may follow without ending a sentence.
const ABBREVIATIONS: &[&str] = &[
    "Inc", "Corp", "Co", "Ltd", "No", "vs", "Mr", "Ms", "Dr", "St", "U.S", "U.K", "L.P", "PBC",
];

/// The sentences of `text` (masked): split after "." "!" "?" and a space,
/// unless the word before is an abbreviation or an initial; and after
/// "。" "！" "？" "；" and line breaks.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let next = chars.get(i + 1).copied();
        let end = match c {
            '。' | '！' | '？' | '；' | '\n' => true,
            '.' | '!' | '?' => {
                let spaced = next.is_none_or(char::is_whitespace);
                let word: String = cur
                    .trim_end_matches(['.', '!', '?'])
                    .rsplit(|ch: char| ch.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .to_string();
                let abbrev = c == '.'
                    && (ABBREVIATIONS.contains(&word.as_str())
                        || (word.chars().count() == 1
                            && word.chars().all(|ch| ch.is_ascii_uppercase())));
                spaced && !abbrev
            }
            _ => false,
        };
        if end {
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// A sentence states a filing fact: a placeholder for an allowed date, a
/// form, the symbol or the exchange, or a figure (anything but a bare count
/// of 0-10; the number check decides whether it is allowed).
fn anchored(sentence: &str, lang: Lang) -> bool {
    ["⟨DATE⟩", "⟨FORM⟩", "⟨SYM⟩", "⟨EXCH⟩"]
        .iter()
        .any(|p| sentence.contains(p))
        || extract(sentence, lang).iter().any(|n| {
            n.is_percent
                || n.raw.contains('$')
                || n.multiplier != 1.0
                || n.decimals > 0
                || n.value.abs() > 10.0
        })
}

/// The years the model may write: the allowed dates'.
fn allowed_years(f: &IpoArticleFacts) -> Vec<f64> {
    let mut y: Vec<f64> = allowed_dates(f)
        .iter()
        .map(|d| f64::from(d.year()))
        .collect();
    y.sort_by(f64::total_cmp);
    y.dedup();
    y
}

fn check_numbers(masked_text: &str, f: &IpoArticleFacts, lang: Lang) -> Result<(), Vec<String>> {
    let any = allowed_numbers(f);
    let years = allowed_years(f);
    check_allowed(
        masked_text,
        lang,
        &Allowed {
            any: &any,
            years: &years,
            ..Allowed::default()
        },
    )
}

/// The figures the headline must hold, and the words one of which must
/// name the step.
fn title_needs(f: &IpoArticleFacts, lang: Lang) -> (Vec<f64>, &'static [&'static str], bool) {
    let zh = lang == Lang::Zh;
    let range = || {
        f.num("price_low")
            .into_iter()
            .chain(f.num("price_high"))
            .collect()
    };
    match f.milestone {
        Milestone::Filed => (
            vec![],
            if zh {
                &["提交", "递交", "申请"]
            } else {
                &["file", "filing"]
            },
            true,
        ),
        Milestone::RangeSet | Milestone::RangeChanged => {
            (range(), if zh { &["区间"] } else { &["range"] }, false)
        }
        Milestone::Priced => (
            f.num("price_final").into_iter().collect(),
            if zh {
                &["定价", "发行价"]
            } else {
                &["price", "priced", "pricing"]
            },
            false,
        ),
        Milestone::Listed => (
            f.session.iter().map(|s| s.close).collect(),
            if zh {
                &["上市", "首日", "收盘", "交易"]
            } else {
                &[
                    "trading",
                    "debut",
                    "close",
                    "closed",
                    "first day",
                    "first session",
                ]
            },
            false,
        ),
        Milestone::Withdrawn => (
            vec![],
            if zh {
                &["撤回", "撤销"]
            } else {
                &["withdraw", "withdrew", "withdrawn", "withdrawal"]
            },
            false,
        ),
    }
}

/// The headline names the company and states the step with its figure.
fn check_title(
    title: &str,
    masked_title: &str,
    f: &IpoArticleFacts,
    lang: Lang,
) -> Result<(), String> {
    let width: usize = title
        .chars()
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum();
    if width > 110 {
        return Err(format!(
            "the title is {width} columns wide; keep it to 110 (about 50 Chinese characters)"
        ));
    }
    if !masked_title.contains("⟨NAME⟩") {
        return Err("the title must name the company".into());
    }
    let (figures, words, form) = title_needs(f, lang);
    let lower = title.to_lowercase();
    if !words.iter().any(|w| lower.contains(w)) {
        return Err(format!(
            "the title must state the step ({}); use one of {words:?}",
            f.milestone.as_str()
        ));
    }
    if form && !masked_title.contains("⟨FORM⟩") {
        return Err("the title must name the form filed (such as S-1)".into());
    }
    let nums = extract(masked_title, lang);
    for v in figures {
        if !nums
            .iter()
            .any(|n| crate::numbers::check(&n.raw, lang, &[v]).is_ok() && !n.is_percent)
        {
            return Err(format!(
                "the title must state the step's figure {v} from the facts"
            ));
        }
    }
    Ok(())
}

enum Problem {
    Llm(ProviderError),
    Malformed(String),
    Numbers(Vec<String>),
}

/// The model's text, every check of the module docs. `Ok` is the draft.
fn check(v: &Value, f: &IpoArticleFacts, lang: Lang) -> Result<Draft, Problem> {
    let title = one_line(v, "title").map_err(Problem::Malformed)?;
    let summary = one_line(v, "summary").map_err(Problem::Malformed)?;
    let items = v
        .get("sections")
        .and_then(Value::as_array)
        .ok_or_else(|| Problem::Malformed("missing \"sections\" array".into()))?;
    let got: Vec<(String, String)> = items
        .iter()
        .map(|s| {
            let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            (
                text("heading").trim().to_string(),
                text("markdown").trim().to_string(),
            )
        })
        .collect();
    let want = headings(lang);
    let sections = match_sections(got, &want, &want).map_err(Problem::Malformed)?;
    let (summary_max, section_max) = match lang {
        Lang::En => (280, 900),
        Lang::Zh => (140, 450),
    };
    let limit = |what: &str, s: &str, max: usize| -> Result<(), Problem> {
        let n = s.chars().count();
        if n > max {
            return Err(Problem::Malformed(format!(
                "{what} is {n} characters; keep it to {max}"
            )));
        }
        Ok(())
    };
    limit("summary", &summary, summary_max)?;
    for (h, m) in &sections {
        if m.is_empty() {
            return Err(Problem::Malformed(format!("section {h:?} is empty")));
        }
        reject_markup(&format!("section {h:?}"), m).map_err(Problem::Malformed)?;
        if m.lines()
            .any(|l| l.trim_start().starts_with('#') || is_setext_underline(l))
        {
            return Err(Problem::Malformed(format!(
                "section {h:?} contains a heading line"
            )));
        }
        limit(&format!("section {h:?}"), m, section_max)?;
    }

    let parts: Vec<(&str, &str)> = [("title", title.as_str()), ("summary", summary.as_str())]
        .into_iter()
        .chain(sections.iter().map(|(h, m)| (h.as_str(), m.as_str())))
        .collect();
    let outlets: Vec<&str> = OUTLETS
        .iter()
        .map(|(_, n)| *n)
        .chain(f.items.iter().map(|i: &QuotedItem| i.outlet.as_str()))
        .collect();
    let mut all_masked = String::new();
    for (what, text) in &parts {
        if text.contains(QUOTE_MARKS) {
            return Err(bad_s(format!(
                "{what}: do not quote anything or use quotation marks"
            )));
        }
        if let Some(o) = outlets
            .iter()
            .find(|o| replace_word(text, o, "", true) != *text)
        {
            return Err(bad_s(format!(
                "{what}: do not name a news outlet ({o:?}); the page adds reported news itself"
            )));
        }
        let m = masked(text, f).map_err(|e| bad_s(format!("{what}: {e}")))?;
        if let Some(x) = bare_month_re().find(&m) {
            return Err(bad_s(format!(
                "{what}: write a date only as a full date from dates_you_may_write, never a month alone, a weekday or a numeric date (found {:?})",
                x.as_str().trim()
            )));
        }
        if let Some(w) = refused_word(&m) {
            return Err(bad_s(format!(
                "{what}: do not use {w:?}; state only what the filing says, without valuation, demand, opinion, prediction or timing"
            )));
        }
        let sentences = split_sentences(&m);
        if !["title", "summary"].contains(what) && sentences.len() > MAX_SENTENCES {
            return Err(bad_s(format!(
                "{what} has {} sentences; keep it to {MAX_SENTENCES}",
                sentences.len()
            )));
        }
        if let Some(s) = sentences.iter().find(|s| !anchored(s, lang)) {
            return Err(bad_s(format!(
                "{what}: every sentence must state a fact from the filing with a figure, a filing date, the form, the symbol or the exchange (not so: {s:?})"
            )));
        }
        if *what == "title" {
            check_title(text, &m, f, lang).map_err(Problem::Malformed)?;
        }
        all_masked.push_str(&m);
        all_masked.push('\n');
    }
    check_numbers(&all_masked, f, lang).map_err(|bad| {
        let mut seen = HashSet::new();
        Problem::Numbers(bad.into_iter().filter(|n| seen.insert(n.clone())).collect())
    })?;
    Ok(Draft {
        lang,
        title,
        summary,
        sections,
    })
}

fn bad_s(s: String) -> Problem {
    Problem::Malformed(s)
}

// ---------------------------------------------------------------- render

fn table(f: &IpoArticleFacts, lang: Lang) -> String {
    let mut rows: Vec<(String, String)> = Vec::new();
    let l = |en: &str, zh: &str| pick_owned(lang, en, zh);
    if let (Some(lo), Some(hi)) = (f.num("price_low"), f.num("price_high")) {
        if f.num("price_final").is_none() {
            rows.push((
                l("Price range per share", "每股价格区间"),
                range(lo, hi, lang),
            ));
        }
    }
    if let Some(p) = &f.previous {
        if let (Some(lo), Some(hi)) = (p.price_low, p.price_high) {
            let label = match lang {
                Lang::En => format!(
                    "Previous price range ({}, {})",
                    p.form,
                    day(&p.filing_date, lang)
                ),
                Lang::Zh => format!("此前价格区间（{}，{}）", p.form, day(&p.filing_date, lang)),
            };
            rows.push((label, range(lo, hi, lang)));
        }
        if let Some(s) = p.shares_offered {
            rows.push((
                l("Previous shares offered", "此前发行股数"),
                shares(s, lang),
            ));
        }
    }
    if let Some(p) = f.num("price_final") {
        rows.push((l("Offering price per share", "每股发行价"), price(p, lang)));
    }
    let share_rows = [
        ("shares_offered", "Shares offered", "发行股数"),
        ("shares_company", "Sold by the company", "公司发售"),
        (
            "shares_selling",
            "Sold by selling stockholders",
            "售股股东发售",
        ),
    ];
    for (k, en, zh) in share_rows {
        if let Some(v) = f.num(k) {
            rows.push((l(en, zh), shares(v, lang)));
        }
    }
    if let Some(v) = f.num("option_shares") {
        let cell = match lang {
            Lang::En => format!("{} shares", grouped(v)),
            Lang::Zh => shares(v, lang),
        };
        rows.push((l("Underwriters' option", "承销商超额配售权"), cell));
    }
    for d in &f.derived {
        let label = match lang {
            Lang::En => format!(
                "Offering at {} (derived: shares offered × price)",
                price(d.price, lang)
            ),
            Lang::Zh => format!(
                "按每股{}计算的发行总额（由发行股数×价格推算）",
                price(d.price, lang)
            ),
        };
        rows.push((label, money(d.value_usd, lang)));
    }
    if let (Some(x), Some(s)) = (&f.exchange, &f.symbol) {
        rows.push((
            l("Exchange and symbol", "交易所与代码"),
            format!("{}{}{s}", escape_md(x), pick(lang, ": ", "：")),
        ));
    }
    if let Some(s) = &f.session {
        let when = day(&s.session_date, lang);
        if let Some(o) = s.open {
            rows.push((
                match lang {
                    Lang::En => format!("Open, {when}"),
                    Lang::Zh => format!("开盘价（{when}）"),
                },
                price(o, lang),
            ));
        }
        rows.push((
            match lang {
                Lang::En => format!("Close, {when}"),
                Lang::Zh => format!("收盘价（{when}）"),
            },
            price(s.close, lang),
        ));
        if let Some(v) = s.volume {
            rows.push((l("Volume", "成交量"), shares(v, lang)));
        }
        if let Some(c) = s.change_pct {
            rows.push((
                l("Change from the offering price", "较发行价变动"),
                format!("{c:+.2}%"),
            ));
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    let mut out = pick(
        lang,
        "| From the filing | |\n|---|---:|\n",
        "| 招股文件数据 | |\n|---|---:|\n",
    )
    .to_string();
    for (k, v) in rows {
        out.push_str(&format!("| {k} | {v} |\n"));
    }
    out.push('\n');
    out
}

fn pick_owned(lang: Lang, en: &str, zh: &str) -> String {
    match lang {
        Lang::En => en.to_string(),
        Lang::Zh => zh.to_string(),
    }
}

/// One news item, quoted word for word with its outlet and date (in
/// Chinese the quote stays in English).
fn item_line(i: &QuotedItem, lang: Lang) -> String {
    let q = escape_md(&i.quote);
    let outlet = escape_md(&i.outlet);
    let when = day(&i.published_on, lang);
    match (lang, i.speaker.as_deref()) {
        (Lang::En, Some(s)) => format!(
            "- {}, in {outlet}, {when}: \"{q}\" ([article]({}))",
            escape_md(s),
            i.url
        ),
        (Lang::En, None) => format!("- {outlet}, {when}: \"{q}\" ([article]({}))", i.url),
        (Lang::Zh, Some(s)) => format!(
            "- {}（{outlet}，{when}）：“{q}”（[原文]({})）",
            escape_md(s),
            i.url
        ),
        (Lang::Zh, None) => format!("- {outlet}（{when}）报道：“{q}”（[原文]({})）", i.url),
    }
}

/// The step that follows, stated generally (no date).
fn next_step(m: Milestone, lang: Lang) -> &'static str {
    match (m, lang) {
        (Milestone::Filed, Lang::En) => "A price range is usually set in an amended registration statement. Pricing follows, with the offering price stated in a final prospectus, and then the first day of trading.",
        (Milestone::Filed, Lang::Zh) => "价格区间通常在修订后的注册声明中公布。之后是定价，发行价在最终招股说明书中载明，然后开始首日交易。",
        (Milestone::RangeSet | Milestone::RangeChanged, Lang::En) => "The range can still change in a later amendment. The offering price is set at pricing and stated in a final prospectus (Form 424B4), after which the shares begin trading.",
        (Milestone::RangeSet | Milestone::RangeChanged, Lang::Zh) => "价格区间仍可能在之后的修订文件中调整。发行价在定价时确定，并在最终招股说明书（424B4表格）中载明，之后股票开始交易。",
        (Milestone::Priced, Lang::En) => "After pricing, the shares begin trading on the exchange. The IPO page adds the first session's close once it is in.",
        (Milestone::Priced, Lang::Zh) => "定价之后，股票在交易所开始交易。首个交易日收盘后，IPO页面会补充收盘数据。",
        (Milestone::Listed, Lang::En) => "The IPO page keeps the closes of the sessions that follow.",
        (Milestone::Listed, Lang::Zh) => "IPO页面会继续记录之后交易日的收盘价。",
        (Milestone::Withdrawn, Lang::En) => "A withdrawn registration statement ends this offering. A company can file a new registration statement later.",
        (Milestone::Withdrawn, Lang::Zh) => "撤回注册声明意味着本次发行终止。公司日后可以重新提交注册声明。",
    }
}

fn meta_line(f: &IpoArticleFacts, lang: Lang) -> String {
    let name = escape_md(name_of(f, lang));
    let mut parts = vec![match (&f.symbol, lang) {
        (Some(s), Lang::En) => format!("**{name} ({s})**"),
        (Some(s), Lang::Zh) => format!("**{name}（{s}）**"),
        (None, _) => format!("**{name}**"),
    }];
    if let Some(x) = &f.exchange {
        parts.push(escape_md(x));
    }
    if let Some(s) = f
        .session
        .as_ref()
        .filter(|_| f.milestone == Milestone::Listed)
    {
        parts.push(match lang {
            Lang::En => format!("First session {}", day(&s.session_date, lang)),
            Lang::Zh => format!("首个交易日{}", day(&s.session_date, lang)),
        });
    } else if let Some(x) = &f.filing {
        parts.push(match lang {
            Lang::En => format!("Form {} filed {}", x.form, day(&x.filing_date, lang)),
            Lang::Zh => format!("{} 提交于{}", x.form, day(&x.filing_date, lang)),
        });
    }
    parts.join(" · ")
}

fn sources(f: &IpoArticleFacts, lang: Lang) -> String {
    let mut out = String::new();
    let filing = |form: &str, date: &str, url: &str| match lang {
        Lang::En => format!(
            "- [Form {form} filed {} (SEC EDGAR)]({url})\n",
            day(date, lang)
        ),
        Lang::Zh => format!(
            "- [{form}，提交于{}（SEC EDGAR）]({url})\n",
            day(date, lang)
        ),
    };
    if let Some(x) = &f.filing {
        out.push_str(&filing(&x.form, &x.filing_date, &x.index_url));
    }
    if let Some(p) = &f.previous {
        out.push_str(&filing(&p.form, &p.filing_date, &p.index_url));
    }
    if let Some(s) = &f.session {
        out.push_str(&match lang {
            Lang::En => format!("- [{} on Yahoo Finance]({})\n", s.symbol, s.source_url),
            Lang::Zh => format!("- [{}（Yahoo Finance）]({})\n", s.symbol, s.source_url),
        });
    }
    // One line per article, though several items may quote it.
    let mut seen = HashSet::new();
    for i in f.items.iter().filter(|i| seen.insert(i.url.as_str())) {
        let sep = pick(lang, ", ", "，");
        out.push_str(&format!(
            "- [{}{sep}{}]({})\n",
            escape_md(&i.outlet),
            day(&i.published_on, lang),
            i.url
        ));
    }
    out
}

/// The full article: title, summary, meta line, the figures table, the
/// model's two sections, the quoted news, the next step, the sources and
/// the disclaimer.
pub fn render_markdown(d: &Draft, f: &IpoArticleFacts) -> String {
    let lang = d.lang;
    let mut out = format!(
        "# {}\n\n{}\n\n{}\n\n",
        d.title,
        d.summary,
        meta_line(f, lang)
    );
    out.push_str(&table(f, lang));
    for (heading, body) in &d.sections {
        out.push_str(&format!("## {heading}\n\n{}\n\n", body.trim()));
    }
    if !f.items.is_empty() {
        out.push_str(pick(
            lang,
            "## What has been reported\n\n",
            "## 已有报道\n\n",
        ));
        for i in &f.items {
            out.push_str(&item_line(i, lang));
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(pick(lang, "## What comes next\n\n", "## 下一步\n\n"));
    out.push_str(next_step(f.milestone, lang));
    out.push_str("\n\n");
    out.push_str(pick(lang, "## Sources\n\n", "## 来源\n\n"));
    out.push_str(&sources(f, lang));
    out.push('\n');
    out.push_str(pick(lang, DISCLAIMER_EN, DISCLAIMER_ZH));
    out.push('\n');
    out
}

/// The sources rows: the filings and the Yahoo page, then each quoted item
/// with its exact quote.
pub fn source_rows(f: &IpoArticleFacts) -> Vec<crate::facts::NewsReason> {
    let row = |url: &str, title: String, quote: &str| crate::facts::NewsReason {
        url: url.into(),
        title,
        quote: quote.into(),
        summary_en: String::new(),
    };
    let mut out = Vec::new();
    if let Some(x) = &f.filing {
        out.push(row(
            &x.index_url,
            format!("{} {} filed {}", f.name, x.form, x.filing_date),
            "",
        ));
    }
    if let Some(p) = &f.previous {
        out.push(row(
            &p.index_url,
            format!("{} {} filed {}", f.name, p.form, p.filing_date),
            "",
        ));
    }
    if let Some(s) = &f.session {
        out.push(row(
            &s.source_url,
            format!("{} on Yahoo Finance", s.symbol),
            "",
        ));
    }
    for i in &f.items {
        out.push(row(&i.url, i.outlet.clone(), &i.quote));
    }
    out
}

// ---------------------------------------------------------------- write

fn prompt(lang: Lang) -> &'static str {
    pick(lang, PROMPT_EN, PROMPT_ZH)
}

fn retry_note(p: &Problem) -> String {
    let line = match p {
        Problem::Llm(e) => format!("The previous request failed: {e}"),
        Problem::Malformed(m) => format!("Your reply was invalid: {m}"),
        Problem::Numbers(n) => format!("These numbers are not in the facts: {}", n.join(", ")),
    };
    format!("\n\nYour previous reply had problems. Fix them and reply again:\n- {line}\n")
}

fn errors(p: Problem) -> Vec<String> {
    match p {
        Problem::Llm(e) => vec![format!("llm: {e}")],
        Problem::Malformed(m) => vec![format!("draft: {m}")],
        Problem::Numbers(n) => n
            .into_iter()
            .map(|x| format!("number not in the facts: {x}"))
            .collect(),
    }
}

/// Checks one reply; `Ok` is the draft and its rendered page.
pub fn accept(v: &Value, f: &IpoArticleFacts, lang: Lang) -> Result<(Draft, String), Vec<String>> {
    check(v, f, lang)
        .map(|d| {
            let md = render_markdown(&d, f);
            (d, md)
        })
        .map_err(errors)
}

/// Drafts one language, retrying once with the problems appended. Returns
/// the draft and its page, or the last attempt's errors.
pub async fn write(
    llm: &dyn LlmProvider,
    f: &IpoArticleFacts,
    lang: Lang,
) -> Result<(Draft, String), Vec<String>> {
    let input = serde_json::to_string_pretty(&model_input(f))
        .map_err(|e| vec![format!("draft: facts json: {e}")])?;
    let base = format!("{input}\n\n{}", sections_line(lang, &headings(lang)));
    let mut user = base.clone();
    let mut last = None;
    for _ in 0..ATTEMPTS {
        let r = llm
            .complete_json(prompt(lang), &user)
            .await
            .map_err(Problem::Llm)
            .and_then(|v| check(&v, f, lang));
        match r {
            Ok(d) => {
                let md = render_markdown(&d, f);
                return Ok((d, md));
            }
            Err(p) => {
                user = format!("{base}{}", retry_note(&p));
                last = Some(p);
            }
        }
    }
    Err(last.map(errors).unwrap_or_default())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ipo::milestone::tests::{anthropic, real_state};
    use crate::ipo::milestone::{bundle, latest};
    use async_trait::async_trait;
    use std::cell::RefCell;

    /// The local preview's write-up: the real state with the made-up news
    /// items of `db/seed-ipo-preview.sql` (FIXTURE rows there).
    pub(crate) fn preview_facts() -> IpoArticleFacts {
        use crate::ipo::store::ItemRow;
        let mut st = real_state();
        let item = |kind: &str,
                    url: &str,
                    outlet: &str,
                    on: &str,
                    quote: &str,
                    speaker: Option<&str>,
                    usd: Option<f64>,
                    date: Option<&str>| ItemRow {
            slug: "anthropic".into(),
            kind: kind.into(),
            url: url.into(),
            outlet: outlet.into(),
            published_on: on.into(),
            quote: quote.into(),
            statement_en: None,
            statement_zh: None,
            speaker: speaker.map(Into::into),
            value_usd: usd,
            value_date: date.map(Into::into),
            found_at: "2026-11-11T15:00:00Z".into(),
        };
        st.items = vec![
            item("valuation", "https://sampletimes.example/2026/10/12/anthropic-valuation", "Sample Times", "2026-10-12", "Bankers have pitched a valuation of about $2 trillion for the listing, according to people familiar with the talks.", None, Some(2e12), None),
            item("valuation", "https://news.example/2026/11/10/anthropic-ipo-pricing", "Example News", "2026-11-10", "At the top of the range the offering would value Anthropic at roughly $1.4 trillion, the people said.", None, Some(1.4e12), None),
            item("expected_date", "https://news.example/2026/11/10/anthropic-ipo-pricing", "Example News", "2026-11-10", "Anthropic is expected to begin trading on the Nasdaq on Nov. 16 after pricing its shares on Nov. 13, the people said.", None, None, Some("2026-11-16")),
            item("company_move", "https://sampletimes.example/2026/11/04/anthropic-office-apps", "Sample Times", "2026-11-04", "Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers.", None, None, None),
            item("view", "https://sampletimes.example/2026/11/09/anthropic-ipo-analysts", "Sample Times", "2026-11-09", "The office tools show how far they want to push into software that businesses already pay for", Some("Jane Doe, an analyst at Example Securities"), None, None),
        ];
        let r = latest(&st, "anthropic").unwrap();
        bundle(
            &st,
            &anthropic(),
            &r,
            "2026-11-11T22:00:00Z".parse().unwrap(),
        )
    }

    /// `db/seed-ipo-articles-preview.sql` and its facts object are what the
    /// generator renders from the preview state.
    #[test]
    fn the_preview_seed_is_the_rendered_write_up() {
        let seed = include_str!("../../../../db/seed-ipo-articles-preview.sql");
        let json = include_str!(
            "../../../../db/preview-facts/ipo/anthropic/range_set-0001628280-26-072484.json"
        );
        let f = preview_facts();
        let stored: IpoArticleFacts = serde_json::from_str(json).unwrap();
        assert_eq!(stored, f);
        let sql = |s: &str| format!("'{}'", s.replace('\'', "''"));
        for lang in [Lang::En, Lang::Zh] {
            let (d, md) = accept(&reply(lang), &f, lang).unwrap();
            let [a, b] = cover_lines(&f, lang);
            for part in [d.title, d.summary, md, a, b] {
                assert!(seed.contains(&sql(&part)), "the seed lacks {part:?}");
            }
        }
        for r in source_rows(&f) {
            assert!(seed.contains(&format!(
                "{}, {}, {}",
                sql(&r.url),
                sql(&r.title),
                sql(&r.quote)
            )));
        }
        assert!(seed.contains(&format!("'{}'", f.facts_key())));
    }

    pub(crate) fn facts() -> IpoArticleFacts {
        let st = real_state();
        let r = latest(&st, "anthropic").unwrap();
        bundle(
            &st,
            &anthropic(),
            &r,
            "2026-11-11T22:00:00Z".parse().unwrap(),
        )
    }

    pub(crate) const EN_TITLE: &str = "Anthropic sets IPO price range at $70 to $78 a share";
    pub(crate) const EN_SUMMARY: &str = "Anthropic's amended S-1 filed Nov. 10, 2026 sets a price range of $70.00 to $78.00 a share for 825,000,000 shares of Class A common stock.";
    pub(crate) const EN_HAPPENED: &str = "Anthropic filed an amended registration statement (Form S-1/A) on Nov. 10, 2026. Its cover sets a price range of $70.00 to $78.00 per share for 825,000,000 shares of Class A common stock. At $70.00 a share, the shares offered come to $57.75 billion, and at $78.00 they come to $64.35 billion.";
    pub(crate) const EN_TERMS: &str = "Anthropic is selling 801,351,354 of the shares, and selling stockholders are selling 23,648,646. The underwriters have an option to purchase up to 123,750,000 additional shares. Anthropic has applied to list its Class A common stock on the Nasdaq Global Select Market under the symbol ANT.";
    pub(crate) const ZH_TITLE: &str = "Anthropic公布IPO价格区间：每股70至78美元";
    pub(crate) const ZH_SUMMARY: &str = "Anthropic于2026年11月10日提交S-1/A修订文件，将8.25亿股A类普通股的价格区间定为每股70.00至78.00美元。";
    pub(crate) const ZH_HAPPENED: &str = "Anthropic于2026年11月10日提交了注册声明修订文件（S-1/A）。文件将价格区间定为每股70.00美元至78.00美元，发行8.25亿股A类普通股。按每股70.00美元计算，发行总额为577.5亿美元；按每股78.00美元计算，为643.5亿美元。";
    pub(crate) const ZH_TERMS: &str = "其中公司发售801,351,354股，售股股东发售23,648,646股。承销商有权额外购买至多123,750,000股。公司已申请在Nasdaq Global Select Market上市，股票代码为ANT。";

    pub(crate) fn reply(lang: Lang) -> Value {
        let [h1, h2] = headings(lang);
        let (t, s, a, b) = match lang {
            Lang::En => (EN_TITLE, EN_SUMMARY, EN_HAPPENED, EN_TERMS),
            Lang::Zh => (ZH_TITLE, ZH_SUMMARY, ZH_HAPPENED, ZH_TERMS),
        };
        json!({"title": t, "summary": s, "sections": [
            {"heading": h1, "markdown": a}, {"heading": h2, "markdown": b}]})
    }

    /// `reply` with one section's text replaced.
    fn with_section(lang: Lang, i: usize, text: &str) -> Value {
        let mut v = reply(lang);
        v["sections"][i]["markdown"] = json!(text);
        v
    }

    fn refused(v: &Value, lang: Lang) -> String {
        match accept(v, &facts(), lang) {
            Ok((_, md)) => panic!("accepted:\n{md}"),
            Err(e) => e.join("; "),
        }
    }

    #[test]
    fn the_real_range_set_drafts_pass_and_render_the_filing_table() {
        let f = facts();
        let (d, md) = accept(&reply(Lang::En), &f, Lang::En).unwrap();
        assert_eq!(d.title, EN_TITLE);
        for want in [
            "# Anthropic sets IPO price range at $70 to $78 a share\n",
            "**Anthropic (ANT)** · Nasdaq Global Select Market · Form S-1/A filed Nov 10, 2026",
            "| Price range per share | $70.00–$78.00 |",
            "| Shares offered | 825,000,000 |",
            "| Sold by the company | 801,351,354 |",
            "| Sold by selling stockholders | 23,648,646 |",
            "| Underwriters' option | 123,750,000 shares |",
            "| Offering at $70.00 (derived: shares offered × price) | $57.75 billion |",
            "| Offering at $78.00 (derived: shares offered × price) | $64.35 billion |",
            "| Exchange and symbol | Nasdaq Global Select Market: ANT |",
            "## What happened\n\n",
            "## What the filing says\n\n",
            "## What comes next\n\nThe range can still change",
            "- [Form S-1/A filed Nov 10, 2026 (SEC EDGAR)](https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm)",
            DISCLAIMER_EN,
        ] {
            assert!(md.contains(want), "missing {want:?} in\n{md}");
        }
        // No news stored: no reported section.
        assert!(!md.contains("What has been reported"));
        let (_, zh) = accept(&reply(Lang::Zh), &f, Lang::Zh).unwrap();
        for want in [
            "**Anthropic（ANT）** · Nasdaq Global Select Market · S-1/A 提交于2026年11月10日",
            "| 每股价格区间 | 70.00–78.00美元 |",
            "| 按每股70.00美元计算的发行总额（由发行股数×价格推算） | 577.5亿美元 |",
            "## 发生了什么\n\n",
            "## 下一步\n\n",
            DISCLAIMER_ZH,
        ] {
            assert!(zh.contains(want), "missing {want:?} in\n{zh}");
        }
        assert_eq!(
            cover_lines(&f, Lang::En),
            [
                "Anthropic IPO".to_string(),
                "Price range $70.00–$78.00".into()
            ]
        );
        assert_eq!(cover_lines(&f, Lang::Zh)[1], "价格区间 70.00–78.00美元");
    }

    #[test]
    fn a_figure_not_in_the_filing_is_refused() {
        // A valuation-sized number, with no refused word around it.
        let e = refused(
            &with_section(
                Lang::En,
                0,
                "At $78.00 a share, the offering comes to $1.4 trillion.",
            ),
            Lang::En,
        );
        assert!(e.contains("$1.4 trillion"), "{e}");
        // A share the model computed itself.
        let e = refused(
            &with_section(
                Lang::En,
                1,
                "Anthropic sells 97% of the 825,000,000 shares.",
            ),
            Lang::En,
        );
        assert!(e.contains("97%"), "{e}");
        let e = refused(
            &with_section(Lang::Zh, 0, "按每股78.00美元计算，公司整体为1.4万亿美元。"),
            Lang::Zh,
        );
        assert!(e.contains("1.4万亿"), "{e}");
    }

    #[test]
    fn every_sentence_must_state_a_filing_fact() {
        for (lang, text) in [
            (Lang::En, "Anthropic filed Form S-1/A on Nov. 10, 2026. Investors are lining up for the shares."),
            (Lang::En, "The S-1/A sets a range of $70.00 to $78.00. It is a big moment for the company."),
            (Lang::Zh, "Anthropic于2026年11月10日提交S-1/A。投资者踊跃参与。"),
        ] {
            let e = refused(&with_section(lang, 0, text), lang);
            assert!(e.contains("every sentence must state a fact"), "{text}: {e}");
        }
    }

    #[test]
    fn a_date_or_timing_not_from_the_filings_is_refused() {
        for (lang, text) in [
            (
                Lang::En,
                "Trading begins on Nov. 19, 2026 under the symbol ANT.",
            ),
            (
                Lang::En,
                "The S-1/A sets a range of $70.00 to $78.00 for a listing in November.",
            ),
            (
                Lang::En,
                "The S-1/A of 11/10/2026 sets a range of $70.00 to $78.00.",
            ),
            (
                Lang::Zh,
                "股票于2026年11月19日在Nasdaq Global Select Market开始交易。",
            ),
            (
                Lang::Zh,
                "S-1/A将价格区间定为每股70.00至78.00美元，11月挂牌。",
            ),
            (
                Lang::Zh,
                "S-1/A将价格区间定为每股70.00至78.00美元，19号开始交易。",
            ),
        ] {
            let e = refused(&with_section(lang, 0, text), lang);
            assert!(e.contains("date"), "{text}: {e}");
        }
        // A weekday is a timing word.
        let e = refused(
            &with_section(
                Lang::En,
                0,
                "The S-1/A sets a range of $70.00 to $78.00; trading starts Thursday.",
            ),
            Lang::En,
        );
        assert!(e.contains("thursday"), "{e}");
    }

    #[test]
    fn news_never_enters_the_models_text() {
        for (lang, text) in [
            (
                Lang::En,
                "Reuters says the S-1/A range of $70.00 to $78.00 is final.",
            ),
            (
                Lang::En,
                "The S-1/A lists the symbol \"ANT\" for the Nasdaq Global Select Market.",
            ),
            (
                Lang::Zh,
                "据彭博，S-1/A将价格区间定为每股70.00至78.00美元。"
                    .replace("彭博", "Bloomberg")
                    .as_str(),
            ),
            (Lang::Zh, "S-1/A载明股票代码为“ANT”。"),
        ] {
            let e = refused(&with_section(lang, 0, text), lang);
            assert!(
                e.contains("outlet") || e.contains("quotation"),
                "{text}: {e}"
            );
        }
    }

    #[test]
    fn valuation_demand_opinion_and_prediction_words_are_refused() {
        for (lang, text) in [
            (
                Lang::En,
                "The S-1/A range of $70.00 to $78.00 values the company richly.",
            ),
            (
                Lang::En,
                "The S-1/A range of $70.00 to $78.00 reflects strong demand.",
            ),
            (
                Lang::En,
                "The 825,000,000 shares will trade on the Nasdaq Global Select Market.",
            ),
            (
                Lang::En,
                "The S-1/A range of $70.00 to $78.00 makes this the largest IPO.",
            ),
            (
                Lang::En,
                "Investors should consider the S-1/A range of $70.00 to $78.00.",
            ),
            (
                Lang::En,
                "The S-1/A range of $70.00 to $78.00 is above expectations.",
            ),
            (
                Lang::En,
                "The S-1/A range of $70.00 to $78.00 was reported earlier.",
            ),
            (
                Lang::Zh,
                "S-1/A的价格区间为每股70.00至78.00美元，认购火爆。",
            ),
            (
                Lang::Zh,
                "8.25亿股将于Nasdaq Global Select Market挂牌交易。",
            ),
            (Lang::Zh, "每股70.00至78.00美元的区间意味着估值很高。"),
            (
                Lang::Zh,
                "S-1/A的区间为每股70.00至78.00美元，值得投资者关注。",
            ),
        ] {
            let e = refused(&with_section(lang, 0, text), lang);
            assert!(e.contains("do not use"), "{text}: {e}");
        }
        // The cover's own term is not a discount claim.
        let ok = with_section(Lang::En, 1, &format!("{EN_TERMS} The underwriters may purchase the 123,750,000 shares at the offering price less the underwriting discount.").replace(" may purchase", " have an option to purchase"));
        assert!(accept(&ok, &facts(), Lang::En).is_ok());
    }

    #[test]
    fn the_title_states_the_step_with_its_figures() {
        let mut v = reply(Lang::En);
        v["title"] = json!("Anthropic files an amended S-1/A");
        assert!(refused(&v, Lang::En).contains("title must state the step"));
        v["title"] = json!("Anthropic sets an IPO price range in its S-1/A");
        assert!(refused(&v, Lang::En).contains("figure"));
        v["title"] = json!("IPO price range set at $70 to $78 a share");
        assert!(refused(&v, Lang::En).contains("name the company"));
        let mut z = reply(Lang::Zh);
        z["title"] = json!("Anthropic公布IPO价格区间：每股70美元起");
        assert!(refused(&z, Lang::Zh).contains("figure"));
    }

    #[test]
    fn quoted_items_render_word_for_word_and_never_reach_the_model() {
        let mut f = facts();
        f.items = vec![
            QuotedItem {
                kind: "valuation".into(),
                outlet: "Reuters".into(),
                url: "https://www.reuters.com/markets/2026-11-10/anthropic-ipo".into(),
                published_on: "2026-11-10".into(),
                quote: "At the top of the price range, Anthropic would be valued at about $1.4 trillion, the people said.".into(),
                speaker: None,
                value_usd: Some(1.4e12),
                value_date: None,
            },
            QuotedItem {
                kind: "view".into(),
                outlet: "Sample Times".into(),
                url: "https://sampletimes.example/a".into(),
                published_on: "2026-11-09".into(),
                quote: "The office tools show how far they want to push".into(),
                speaker: Some("Jane Doe, an analyst at Example Securities".into()),
                value_usd: None,
                value_date: None,
            },
        ];
        let input = serde_json::to_string(&model_input(&f)).unwrap();
        assert!(
            !input.contains("1.4 trillion")
                && !input.contains("Reuters")
                && !input.contains("office tools"),
            "{input}"
        );
        let (_, md) = accept(&reply(Lang::En), &f, Lang::En).unwrap();
        assert!(md.contains("## What has been reported\n\n- Reuters, Nov 10, 2026: \"At the top of the price range, Anthropic would be valued at about $1.4 trillion, the people said.\" ([article](https://www.reuters.com/markets/2026-11-10/anthropic-ipo))"), "{md}");
        assert!(md.contains("- Jane Doe, an analyst at Example Securities, in Sample Times, Nov 9, 2026: \"The office tools show how far they want to push\""), "{md}");
        assert!(md.contains(
            "- [Reuters, Nov 10, 2026](https://www.reuters.com/markets/2026-11-10/anthropic-ipo)"
        ));
        let (_, zh) = accept(&reply(Lang::Zh), &f, Lang::Zh).unwrap();
        assert!(zh.contains("## 已有报道\n\n- Reuters（2026年11月10日）报道：“At the top of the price range, Anthropic would be valued at about $1.4 trillion, the people said.”（[原文](https://www.reuters.com/markets/2026-11-10/anthropic-ipo)）"), "{zh}");
        // The model may still not write the outlet's figure itself.
        let e = refused(
            &with_section(
                Lang::En,
                0,
                "At $78.00, the S-1/A range comes to $1.4 trillion.",
            ),
            Lang::En,
        );
        assert!(e.contains("1.4"), "{e}");
        let rows = source_rows(&f);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].quote, f.items[0].quote);
    }

    #[test]
    fn sentences_split_on_ends_not_on_decimals_or_abbreviations() {
        assert_eq!(
            split_sentences(
                "Anthropic, PBC. sells at $70.00 a share. U.S. listing on ⟨EXCH⟩. Then 2"
            ),
            [
                "Anthropic, PBC. sells at $70.00 a share.",
                "U.S. listing on ⟨EXCH⟩.",
                "Then 2"
            ]
        );
        assert_eq!(split_sentences("甲。乙；丙"), ["甲。", "乙；", "丙"]);
    }

    struct FakeLlm {
        replies: RefCell<Vec<Value>>,
        prompts: RefCell<Vec<String>>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, _: &str, user: &str) -> Result<Value, ProviderError> {
            self.prompts.borrow_mut().push(user.into());
            let mut r = self.replies.borrow_mut();
            assert!(!r.is_empty(), "unexpected extra LLM call");
            Ok(r.remove(0))
        }
    }

    #[tokio::test]
    async fn a_refused_draft_is_retried_once_with_the_reason() {
        let bad = with_section(
            Lang::En,
            0,
            "The S-1/A range of $70.00 to $78.00 reflects strong demand.",
        );
        let l = FakeLlm {
            replies: RefCell::new(vec![bad.clone(), reply(Lang::En)]),
            prompts: RefCell::new(vec![]),
        };
        assert!(write(&l, &facts(), Lang::En).await.is_ok());
        let p = l.prompts.borrow().clone();
        assert!(
            p[1].contains("Your reply was invalid") && p[1].contains("reflects"),
            "{}",
            p[1]
        );
        assert!(
            p[0].contains("\"dates_you_may_write\"")
                && p[0].contains("Write exactly these sections")
        );
        let l = FakeLlm {
            replies: RefCell::new(vec![bad.clone(), bad]),
            prompts: RefCell::new(vec![]),
        };
        let e = write(&l, &facts(), Lang::En).await.unwrap_err();
        assert!(e[0].starts_with("draft: "), "{e:?}");
    }
}
