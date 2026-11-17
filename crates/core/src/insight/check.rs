//! Code checks an insight passes before the judge sees it: lengths and
//! form, cited ids, numbers, dates, relative time, fiscal labels, outlet
//! attribution, hedged unconfirmed dates and a refusal list of advice,
//! rating and prediction wording.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use chrono::{Datelike, NaiveDate};
use regex::Regex;

use super::{Evidence, EvidenceKind, Insight, Line, Pack};
use crate::ipo::news::visible;
use crate::numbers::{self, Lang};

pub const HOOK_WORDS_EN: usize = 8;
pub const HOOK_CHARS_ZH: usize = 20;
pub const WATCH_WORDS_EN: usize = 40;
pub const WATCH_CHARS_ZH: usize = 100;

/// Advice, rating, target, consensus and prediction wording, refused even
/// when the evidence uses it. Regex fragments matched as whole words,
/// ignoring case (`has_phrase`); a hyphen does not join a word
/// ("Overweight-rated" holds "overweight"), except in the compounds of
/// `EXEMPT_COMPOUNDS`. Words that also have a plain sense ("buy", "sell",
/// "target", "hold", "neutral", "upside") have their own rules
/// (`advice_words`, `advice_hold`).
const REFUSED_EN: &[&str] = &[
    "should",
    "must",
    r"recommend\w*",
    r"will\s+(?:rise|fall|rally|drop|climb|soar|surge|jump|gain|rebound)",
    r"(?:could|may|might|is\s+likely\s+to|are\s+likely\s+to)\s+(?:rise|fall|rally|drop|climb|soar|surge|jump|gain|rebound|recover)",
    r"set\s+to\s+(?:surge|soar|jump|rally|climb|rise)",
    r"poised\s+to",
    r"price\s+targets?",
    r"target\s+prices?",
    r"price\s+objectives?",
    "overweight",
    "underweight",
    r"equal[- ]?weight",
    "outperform",
    "underperform",
    r"market\s+perform",
    r"sector\s+(?:perform|weight)",
    r"peer\s+perform",
    "accumulate",
    r"top\s+picks?",
    r"analysts?['’]?\s+(?:\w+\s+){0,4}?(?:expect\w*|estimat\w*|forecast\w*|views?)",
    r"wall\s+street\s+expect\w*",
    r"(?:street|wall\s+street|market|consensus)\s+(?:estimates?|expectations?|forecasts?|views?)",
    r"(?:above|below|beat|beats|beating|missed|miss|misses|missing|topp\w*|tops?|exceed\w*|ahead\s+of|short\s+of|in\s+line\s+with|versus|vs\.?|against)\s+(?:\w+\s+){0,4}?estimates?",
    "consensus",
    "undervalued",
    "overvalued",
    "bullish",
    "bearish",
    r"entry\s+points?",
];
const EXEMPT_COMPOUNDS: &[&str] = &[
    "sell-off",
    "sell-offs",
    "sell-side",
    "sell-through",
    "buy-side",
    "buy-back",
    "buy-backs",
];
const REFUSED_ZH: &[&str] = &[
    "买入",
    "卖出",
    "建议",
    "应当",
    "应该",
    "目标价",
    "目标股价",
    "价格目标",
    "评级",
    "中性",
    "推荐",
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
    "有望上涨",
    "有望上行",
    "上涨空间",
    "上行空间",
    "下行空间",
    "加仓",
    "减仓",
    "仓位",
    "逢低",
    "抄底",
    "华尔街预期",
    "市场预期",
    "分析师预期",
    "一致预期",
    "预期共识",
    "有望反弹",
    "有望回升",
    "有望走高",
];
const RELATIVE_ZH: &[&str] = &[
    "下周",
    "下个月",
    "下月",
    "明天",
    "明日",
    "本周",
    "本月",
    "近期",
    "近日",
    "下季度",
    "下个季度",
    "本季度",
    "明年",
    "今年晚些时候",
    "今年底",
    "今年年底",
    "今年内",
    "今天",
    "今日",
    "昨天",
    "昨日",
    "即将",
    "不久",
    "稍后",
    "晚些时候",
    "年内",
    "上周",
    "上月",
    "上个月",
    "近几周",
    "近几个月",
];
/// Hedge words a line citing an unconfirmed report date must hold.
const HEDGE_EN: &[&str] = &["expected", "scheduled", "unconfirmed"];
const HEDGE_ZH: &[&str] = &["预计", "预期", "暂定", "未确认"];
/// Chinese hedges that govern an unconfirmed date: 预计 counts only as 市场预计
/// (the market's, not the company's, "英伟达预计于11月17日"); 市场预期 is consensus wording.
const DATE_HEDGE_ZH: &[&str] = &["暂定", "未经公司确认", "未确认", "市场预计"];

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

/// English names that are also ordinary words ("a Fortune 500 company"):
/// they count as naming the outlet only where the line attributes to it.
const COMMON_NAMES: &[&str] = &[
    "Fortune",
    "Guardian",
    "The Guardian",
    "Economist",
    "The Economist",
    "The Information",
    "Verge",
    "The Verge",
];

/// Chinese names that start ordinary words: "市场观察人士" is market
/// observers, not MarketWatch.
const NOT_A_NAME_AFTER: &[(&str, &[&str])] = &[("市场观察", &["人士", "者", "家", "员"])];

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
        // A brand outside the table, or a bare host: its name, and its host
        // with its top-level domain ("stocktitan.net"; never the bare word,
        // since "investing" or "fool" is an ordinary word), in both
        // languages.
        let mut out = vec![outlet.to_string()];
        if let Some(host) = super::pack::outlet_host(outlet) {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&host)) {
                out.push(host);
            }
        }
        return out;
    };
    let mut out: Vec<String> = Vec::new();
    if lang == Lang::Zh {
        out.extend(zh.iter().map(|s| s.to_string()));
    }
    out.push(name.to_string());
    out.extend(en.iter().map(|s| s.to_string()));
    out
}

/// Patterns this module has compiled in this process (`insight::compiled`).
static COMPILED: AtomicUsize = AtomicUsize::new(0);

fn re(p: &str) -> Regex {
    COMPILED.fetch_add(1, Ordering::Relaxed);
    Regex::new(p).expect("valid built-in regex")
}

pub(crate) fn compiled() -> usize {
    COMPILED.load(Ordering::Relaxed)
}

/// English figures in words ("ten percent", "doubled"); see `worded_figures`.
const WORDED_EN: &str = r"(?i)(?:(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety|hundred|a\s+half|half)(?:[- ](?:one|two|three|four|five|six|seven|eight|nine))?\s+(?:percent|per\s+cent|percentage\s+points?|basis\s+points?|%|billion|million|trillion|thousand|dollars|times|fold)|a\s+(?:million|billion|trillion)|(?:doubl|tripl|quadrupl|halv)\w*)";
/// "Hold" as advice or a rating; see `advice_hold`.
const ADVICE_HOLD: &str = r"(?i)(?:(?:from|rated|rating|investors|continue\s+to|keep|recommend\w*)\s+hold|(?:downgrad\w*|upgrad\w*|cut|lower\w*|rais\w*|mov\w*)\s+(?:\w+\s+){0,3}to\s+hold|hold(?:s|ing)?\s+(?:rating|from|onto|on\s+to|(?:the|their|your|its)\s+(?:stock|shares|positions?|stake)|(?:stock|shares|positions?)))";
/// Forward relative time in English; see `relative_en_re`.
const RELATIVE_EN: &str = r"(?i)(?:next\s+(?:week|month|quarter|year|few\s+\w+|several\s+\w+)|(?:over|in|during|within)\s+the\s+(?:next|coming)\s+(?:\w+\s+)?(?:days|weeks|months|quarters|years)|in\s+(?:the\s+)?coming\s+(?:days|weeks|months|quarters)|(?:later\s+)?this\s+(?:week|month|quarter|year)|last\s+(?:week|month)|later\s+today|today|tonight|tomorrow|yesterday|soon|shortly|in\s+the\s+(?:days|weeks|months|quarters|years)\s+ahead|(?:days|weeks|months)\s+ahead|in\s+the\s+near\s+term|near[- ]term|by\s+(?:the\s+)?(?:year[- ]end|end\s+of\s+the\s+year)|later\s+in\s+the\s+year)";
/// "upgraded … to " before a rating word; see `rating_frame`.
const RATING_FRAME: &str =
    r"(?:upgrad|downgrad|cut|lower|rais|mov|reiterat|maintain)\w*\s+(?:[\w.'’-]+\s+){0,3}to\s+$";

/// A word character for the English refusal patterns that is cheap to
/// compile: an ASCII one or any non-ASCII character. Unicode `\w` costs
/// most of these patterns' compile time (the CPU ruling of 2026-11-17).
/// It is a superset of `\w`, so a pattern still matches wherever it did;
/// it can also match where a non-ASCII character that is not a letter or
/// digit (a curly quote, a dash) stands in a word's place. No stored line
/// or evidence text matches differently (`wide_words_match_as_before`).
const WIDE_WORD: &str = r"0-9A-Za-z_\x{80}-\x{10FFFF}";

/// `p` with every `\w` as `WIDE_WORD`: inside a bracket class its ranges,
/// outside it a class of its own with case folding off (folding the
/// non-ASCII range would cost more than `\w`).
fn wide_words(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut in_class = false;
    let mut chars = p.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let next = chars.next().expect("no trailing backslash");
                if next == 'w' {
                    if in_class {
                        out.push_str(WIDE_WORD);
                    } else {
                        out.push_str(&format!("(?-i:[{WIDE_WORD}])"));
                    }
                } else {
                    out.push('\\');
                    out.push(next);
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push(c);
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Compiles every lazily compiled pattern of the code checks, so the
/// message that runs `check` pays the compile up front. One entry per
/// `OnceLock` in this file (`insight::tests` counts them); the number.
pub fn warm() -> usize {
    let day = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date");
    let touch: [fn(NaiveDate); 18] = [
        |_| {
            let _ = worded_figures("");
        },
        |_| {
            let _ = hook_count("");
        },
        |_| {
            let _ = refused_en();
        },
        |_| {
            let _ = advice_hold("");
        },
        |_| {
            let _ = attributes("", "x");
        },
        |_| {
            let _ = digits_only_in_names("", &[]);
        },
        |_| {
            let _ = names_attributed("", "x");
        },
        warm_dates,
        |_| {
            let _ = bare_months("");
        },
        |_| {
            let _ = zh_years("");
        },
        |_| {
            let _ = relative_en_re();
        },
        |_| {
            let _ = relative_zh_re();
        },
        |_| {
            let _ = fiscal_labels("");
        },
        |_| {
            let _ = clause_start("", 0);
        },
        |_| {
            let _ = rating_frame();
        },
        |_| {
            let _ = advice_words("", false);
        },
        |_| {
            let _ = million_zh();
        },
        |_| {
            let _ = personal_affair("");
        },
    ];
    for t in touch {
        t(day);
    }
    touch.len()
}

/// A figure in 百万 or 千万 (compiled once, not per line).
fn million_zh() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"\d\s*(?:百万|千万)"))
}

/// The date marks alone: what the judge's confirmation (`judge::passed`)
/// compiles of this file.
pub(crate) fn warm_dates(day: NaiveDate) {
    let _ = date_marks("", day);
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

/// Whether `text[start..end]` stands alone: no ASCII letter or digit glued
/// on either side. Chinese characters around it are fine ("于Nov 19日").
fn ascii_alone(text: &str, start: usize, end: usize) -> bool {
    let glued = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    !glued(text[..start].chars().next_back()) && !glued(text[end..].chars().next())
}

/// Matches of `r` that stand alone (`ascii_alone`).
fn find_alone<'t>(r: &Regex, text: &'t str) -> Vec<regex::Match<'t>> {
    r.find_iter(text)
        .filter(|m| ascii_alone(text, m.start(), m.end()))
        .collect()
}

/// Figures written in words, which code cannot check: Chinese numerals
/// before a unit ("七十美元"), 百分之…, 三成, 两倍, 翻倍, and English number
/// words before a unit ("ten percent", "a billion dollars", "doubled").
fn worded_figures(text: &str) -> Vec<String> {
    static R: OnceLock<[Regex; 3]> = OnceLock::new();
    let [zh, zh_unit, en] = R.get_or_init(|| {
        [
            re(r"百分之[零〇一二三四五六七八九十百点两]+|[一二三四五六七八九十两]成|[一二三四五六七八九十两半]倍|翻倍|翻番"),
            re(r"[一二三四五六七八九十百千万亿两]+(?:美元|亿|万|%|％|个百分点)"),
            re(&wide_words(WORDED_EN)),
        ]
    });
    let mut out: Vec<String> = zh.find_iter(text).map(|m| m.as_str().to_string()).collect();
    // A run of unit characters alone ("42亿美元" has "亿美元") is a unit after digits.
    out.extend(
        zh_unit
            .find_iter(text)
            .filter(|m| {
                m.as_str().contains([
                    '一', '二', '三', '四', '五', '六', '七', '八', '九', '十', '两',
                ])
            })
            .map(|m| m.as_str().to_string()),
    );
    out.extend(
        find_alone(en, text)
            .into_iter()
            // "double-digit growth" names no figure.
            .filter(|m| {
                !text[m.end()..].starts_with("-digit") && !text[m.end()..].starts_with(" digit")
            })
            .map(|m| m.as_str().to_string()),
    );
    out
}

/// A count in Chinese numerals in a hook ("三大看点"): a hook has no numbers.
fn hook_count(text: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| re(r"[一二三四五六七八九十两]+[大个项条只点家季]"));
    r.find_iter(text)
        // "一个" is the everyday "a"; 第三季度 names a quarter.
        .filter(|m| m.as_str() != "一个")
        // 三季度 (with or without 第) names a quarter: a fiscal label, checked
        // as one, not a count.
        .find(|m| !(m.as_str().ends_with('季') && text[m.end()..].starts_with('度')))
        .map(|m| m.as_str().to_string())
}

fn refused_en() -> &'static [Regex] {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    R.get_or_init(|| {
        REFUSED_EN
            .iter()
            .map(|p| re(&wide_words(&format!("(?i){p}"))))
            .collect()
    })
}

/// A match of `r` in `text` as whole words. A hyphen does not join words,
/// except in `EXEMPT_COMPOUNDS`.
fn has_phrase(text: &str, r: &Regex) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    find_alone(r, text)
        .into_iter()
        .find(|m| {
            !EXEMPT_COMPOUNDS.iter().any(|c| {
                // The compound starts at the match or a hyphen before it.
                let from = lower[..m.start()]
                    .rfind(|ch: char| !(ch.is_ascii_alphabetic() || ch == '-'))
                    .map_or(0, |i| i + 1);
                lower[from..].starts_with(c)
                    && !lower[from + c.len()..].starts_with(|ch: char| ch.is_ascii_alphabetic())
            })
        })
        .map(|m| m.as_str().to_string())
}

/// `word` in `text` as a whole word, ignoring ASCII case.
fn has_word(text: &str, word: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower
        .match_indices(&word.to_ascii_lowercase())
        .any(|(at, m)| ascii_alone(&lower, at, at + m.len()))
}

/// "hold" as advice or a rating ("downgraded to hold", "a hold rating",
/// "investors keep hold", "hold the shares"). The verb in its other senses
/// is fine: "will hold its report", "whether the plan holds", "on hold".
fn advice_hold(text: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| re(&wide_words(ADVICE_HOLD)));
    find_alone(r, text).first().map(|m| m.as_str().to_string())
}

/// Whether `text` names `name`: Chinese names anywhere (unless they start
/// an ordinary word, `NOT_A_NAME_AFTER`), English names as whole words with
/// their capitals ("The Information", not "the information").
fn names(text: &str, name: &str) -> bool {
    if name.chars().any(is_cjk) {
        let not_after: &[&str] = NOT_A_NAME_AFTER
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(&[], |(_, a)| a);
        text.match_indices(name).any(|(at, m)| {
            !not_after
                .iter()
                .any(|a| text[at + m.len()..].starts_with(a))
        })
    } else {
        text.match_indices(name)
            .any(|(at, m)| ascii_alone(text, at, at + m.len()))
    }
}

/// Whether `text` attributes something to the outlet `name`: the name (in
/// any ASCII case) inside an attributing frame. English: "<name> reports",
/// "<name> and Reuters report", "<name>'s report found", "according to
/// <name>", "per <name>", "told <name>". Chinese: "据<name>报道", "<name>称",
/// "<name>报道", "<name>则报道", "据<name>、路透社报道". An ordinary word that happens to be a
/// name ("listed on the Nasdaq", "investing heavily", "no fool would") is
/// not attribution.
fn attributes(text: &str, name: &str) -> bool {
    static R: OnceLock<[Regex; 3]> = OnceLock::new();
    let [after_en, after_zh, before_zh] = R.get_or_init(|| {
        [
            re(r"(?i)^(?:'s|’s)?(?:\s+and\s+[^,.;，。]{1,40}?)?\s+(?:reports?|reported|reporting|says?|said|wrote|writes|notes?|noted|adds?|added|cited|citing|finds?|found|magazine|newspaper)\b"),
            re(r"^\s*(?:[、和与及][^，。,;；]{0,30}?)?[则也还亦另]?(?:报道|称|指出|援引|表示|的报道|消息称)"),
            re(r"据[^，。,;；]{0,30}[、和与及]\s*$"),
        ]
    });
    let cjk = name.chars().any(is_cjk);
    let (hay, needle) = if cjk {
        (text.to_string(), name.to_string())
    } else {
        (text.to_ascii_lowercase(), name.to_ascii_lowercase())
    };
    let not_after: &[&str] = NOT_A_NAME_AFTER
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(&[], |(_, a)| a);
    hay.match_indices(&needle).any(|(at, m)| {
        let end = at + m.len();
        if !cjk && !ascii_alone(&hay, at, end) {
            return false;
        }
        if not_after.iter().any(|a| text[end..].starts_with(a)) {
            return false;
        }
        let before = text[..at].trim_end();
        let lower_before = before.to_ascii_lowercase();
        after_en.is_match(&text[end..])
            || after_zh.is_match(&text[end..])
            || ["according to", "according to the", "per", "told"]
                .iter()
                .any(|b| {
                    lower_before.ends_with(b)
                        && ascii_alone(
                            &lower_before,
                            lower_before.len() - b.len(),
                            lower_before.len(),
                        )
                })
            || before.ends_with('据')
            || before_zh.is_match(&text[..at])
    })
}

/// `text` with every one of `names` (ignoring ASCII case) replaced by
/// "Outlet", so an outlet's name ("24/7 Wall St.", "247wallst.com") is not
/// read as a number or a date.
pub(crate) fn scrub(text: &str, names: &[String]) -> String {
    let mut names: Vec<&String> = names.iter().collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    let mut out = text.to_string();
    for n in names {
        // Built per call for the line's own outlets, never cached: not one
        // of the lazily compiled patterns `warm` covers.
        let r = Regex::new(&format!("(?i){}", regex::escape(n))).expect("an escaped name");
        out = r.replace_all(&out, "Outlet").into_owned();
    }
    out
}

/// Whether every digit in `hook` sits in a name a cited item writes the
/// same way ("GTA 6", "A380") or in a year a cited item names ("2027
/// growth view"); any other digit is a figure.
fn digits_only_in_names(hook: &str, cited: &[String]) -> bool {
    static R: OnceLock<[Regex; 2]> = OnceLock::new();
    let [name, year] = R.get_or_init(|| {
        [
            // A capitalised word and its number ("GTA 6"), or letters and
            // digits glued ("A380", "Q3"); never a lowercase word before a
            // figure ("by 45").
            re(r"[A-Z][A-Za-z.&'’-]*\s?\d+[A-Za-z]*|[A-Za-z]+\d+[A-Za-z]*|\d+[A-Za-z]+"),
            re(r"(?:19|20)\d{2}"),
        ]
    });
    let hook = ascii_digits(hook);
    let named = name.replace_all(&hook, |c: &regex::Captures| {
        let m = c.get(0).expect("match");
        // A percentage or a dollar figure is never a name ("By 45%").
        let figure = hook[m.end()..].starts_with(['%', '％']) || hook[..m.start()].ends_with('$');
        if !figure && cited.iter().any(|t| t.contains(&c[0])) {
            String::new()
        } else {
            c[0].to_string()
        }
    });
    let in_cited = |y: &str| {
        cited.iter().any(|t| {
            t.match_indices(y)
                .any(|(at, m)| ascii_alone(t, at, at + m.len()))
        })
    };
    let years: Vec<(usize, usize)> = find_alone(year, &named)
        .into_iter()
        .filter(|m| in_cited(m.as_str()))
        .map(|m| (m.start(), m.end()))
        .collect();
    let mut rest = named.into_owned();
    for (s, e) in years.into_iter().rev() {
        rest.replace_range(s..e, "");
    }
    !has_digit(&rest)
}

/// Private-life words: a line holding one makes a claim about a named
/// person's private life.
const PERSONAL_EN: &[&str] = &["mistress", "extramarital", "infidelity"];
const PERSONAL_ZH: &[&str] = &["婚外情", "婚外恋", "出轨", "情妇", "绯闻", "不正当关系"];

/// "affair" in its personal sense: "had an affair", "a second affair",
/// "his affair", "the CEO's affair", "CEO affair", "an affair with"; not "a
/// muted affair" or the plural business sense ("current affairs",
/// "corporate affairs", "affairs of the company"). ASCII word boundaries:
/// the English lines it reads are ASCII words.
fn personal_affair(text: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        re(r"(?i-u)(?:\b(?:an|another|second|secret|romantic|alleged|extramarital|had|having|his|her|their|the|ceo)\s+affair\b|\b[a-z]+(?:'|’)s\s+affair\b|\baffair\s+with\b)")
    });
    r.find(text).map(|m| m.as_str().to_string())
}

/// The private-life words in `text`, in either language.
fn personal_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = PERSONAL_EN
        .iter()
        .filter(|w| has_word(text, w))
        .map(|w| w.to_string())
        .collect();
    out.extend(personal_affair(text));
    out.extend(
        PERSONAL_ZH
            .iter()
            .filter(|w| text.contains(**w))
            .map(|w| w.to_string()),
    );
    out
}

/// Rick's people rule (2026-11-17, "Allow it if attributed"), where code
/// can tell: a line with a private-life word (`PERSONAL_EN`, `PERSONAL_ZH`,
/// "affair" in its personal sense) is refused unless the word is not in the
/// hook, a cited news item's text holds a private-life word too, and the
/// sentence attributes that item's outlet. The list is narrow by design: a
/// euphemism ("personal conduct", 私生活风波), other wording for the same
/// claim, or a claim about a person that is not about private life passes
/// it, and so does a claim placed outside the attributing clause ("Benzinga
/// reports the board may act, and the CEO had an affair"). Those are caught
/// only by the judge's `personal_claims_attributed` flag, which fails
/// closed: a missing or non-boolean flag reads as false.
fn private_life(hook: &str, watch: &str, cited: &[&Evidence], lang: Lang) -> Vec<String> {
    let mut bad = Vec::new();
    if let Some(w) = personal_words(hook).first() {
        bad.push(format!(
            "a claim about a person's private life in the hook ({w}): only an attributed sentence may hold one"
        ));
    }
    if let Some(w) = personal_words(watch).first() {
        let reported = cited.iter().any(|e| {
            e.kind == EvidenceKind::News
                && !personal_words(&e.text).is_empty()
                && e.outlet.as_deref().is_some_and(|o| {
                    outlet_names(o, lang)
                        .iter()
                        .any(|n| attributes(&visible(watch), n))
                })
        });
        if !reported {
            bad.push(format!(
                "a claim about a person's private life ({w}) not attributed to the outlet of a cited news item that reports it"
            ));
        }
    }
    bad
}

/// `names`, for a name that is also an ordinary word (case-sensitive): only where the line
/// attributes to it ("Fortune reports", "according to The Economist").
fn names_attributed(text: &str, name: &str) -> bool {
    static AFTER: OnceLock<Regex> = OnceLock::new();
    let after = AFTER.get_or_init(|| {
        re(r"^(?:'s|’s)?\s+(?:reports?|reported|said|says|wrote|writes|magazine|newspaper|cited|citing|found)\b")
    });
    text.match_indices(name).any(|(at, m)| {
        let before = text[..at].trim_end().to_ascii_lowercase();
        ascii_alone(text, at, at + m.len())
            && (after.is_match(&text[at + m.len()..])
                || before.ends_with("according to")
                || before.ends_with("according to the")
                || before.ends_with("told")
                || before.ends_with('据'))
    })
}

/// Abbreviations a period follows without ending the sentence.
const ABBREVIATIONS: &[&str] = &[
    "st", "inc", "corp", "co", "ltd", "mr", "mrs", "ms", "dr", "jr", "sr", "no", "vs", "mt", "ft",
    "gen", "gov", "sen", "rep", "bros", "plc", "jan", "feb", "mar", "apr", "jun", "jul", "aug",
    "sep", "sept", "oct", "nov", "dec", "etc", "approx", "est",
];

/// Byte offsets of the sentence-ending marks in `s`. A period ends a
/// sentence when a space or the end follows it, unless it closes an
/// abbreviation ("St.", "Inc.") or an initial ("U.S.").
fn sentence_ends(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (at, c) in s.char_indices() {
        let end = match c {
            '。' | '！' | '？' | '!' | '?' => true,
            '.' => {
                let next = s[at + 1..].chars().next();
                let token: String = s[..at]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '.')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let short = token.chars().count() == 1 && token.chars().all(char::is_alphabetic);
                let abbr = token.contains('.')
                    || short
                    || ABBREVIATIONS.contains(&token.to_ascii_lowercase().as_str());
                next.is_none_or(char::is_whitespace) && !abbr
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
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn month_number(name: &str) -> u32 {
    const M: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let p = name.to_ascii_lowercase();
    M.iter()
        .position(|m| p.starts_with(m))
        .map_or(0, |i| i as u32 + 1)
}

/// 一..三十一 (and 〇/零 digits of a year) as a number.
fn zh_number(s: &str) -> Option<i64> {
    let digit = |c: char| {
        "〇一二三四五六七八九"
            .chars()
            .position(|d| d == c)
            .or((c == '零').then_some(0))
    };
    if !s.contains('十') {
        // Digit by digit: 二〇二六, 五.
        return s
            .chars()
            .try_fold(0i64, |acc, c| digit(c).map(|d| acc * 10 + d as i64));
    }
    let (tens, ones) = s.split_once('十')?;
    let t = if tens.is_empty() {
        1
    } else {
        digit(tens.chars().next()?)? as i64
    };
    let o = if ones.is_empty() {
        0
    } else {
        digit(ones.chars().next()?)? as i64
    };
    Some(t * 10 + o)
}

/// Words after "the Nth" that make it a count, not a day of the month:
/// "the 10th straight quarter", "the 3rd time".
const COUNT_NOUNS: &[&str] = &[
    "straight",
    "consecutive",
    "quarter",
    "quarters",
    "year",
    "years",
    "time",
    "times",
    "anniversary",
    "largest",
    "biggest",
    "highest",
    "lowest",
    "best",
    "worst",
    "most",
    "month",
    "week",
    "day",
    "place",
    "percentile",
];

/// A date written in a line: its text, and the calendar date, or `None`
/// when it does not exist ("November 31") or cannot be anchored ("the
/// 20th", "Friday", "本月20日").
type Mark = (String, Option<NaiveDate>);

/// Every calendar date written in `text`, in order.
fn date_marks(text: &str, year_of: NaiveDate) -> Vec<Mark> {
    static R: OnceLock<Vec<Regex>> = OnceLock::new();
    let res = R.get_or_init(|| {
        vec![
            // 0: ISO and Y/M/D.
            re(r"((?:19|20)\d{2})[-/](\d{1,2})[-/](\d{1,2})"),
            // 1: Month D[, Y].
            re(&format!(
                r"{MONTH}\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?(?:,?\s+((?:19|20)\d{{2}}))?"
            )),
            // 2: D Month[, Y].
            re(&format!(
                r"(\d{{1,2}})(?:st|nd|rd|th)?\s+{MONTH}(?:,?\s+((?:19|20)\d{{2}}))?"
            )),
            // 3: [Y年]M月D日.
            re(r"(?:((?:19|20)\d{2})\s*年\s*)?(\d{1,2})\s*月\s*(\d{1,2})\s*[日号]"),
            // 4: M/D[/Y].
            re(r"(\d{1,2})/(\d{1,2})(?:/((?:19|20)?\d{2}))?"),
            // 5: Chinese-numeral [Y年]M月D日.
            re(r"(?:([〇零一二三四五六七八九]{4})年)?([一二三四五六七八九十]{1,2})月([一二三四五六七八九十]{1,3})[日号]"),
            // 6-8: days that cannot be anchored: "the 20th", "20日" without a
            // month, weekdays.
            re(r"(?i)the\s+\d{1,2}(?:st|nd|rd|th)"),
            re(r"\d{1,2}\s*[日号]"),
            re(r"(?:Mon|Tues|Wednes|Thurs|Fri|Satur|Sun)day|(?:周|星期|礼拜)[一二三四五六日天]"),
        ]
    });
    let text = ascii_digits(text);
    let num = |m: Option<regex::Match>| m.and_then(|m| m.as_str().parse::<i64>().ok());
    let mut found: Vec<(usize, usize, Option<NaiveDate>)> = Vec::new();
    for (k, r) in res.iter().enumerate() {
        for c in r.captures_iter(&text) {
            let whole = c.get(0).expect("match");
            let (s, e) = (whole.start(), whole.end());
            let latin = matches!(k, 0 | 1 | 2 | 4 | 6);
            if latin && !ascii_alone(&text, s, e) {
                continue;
            }
            // "100日元" is yen.
            if k == 7 && text[e..].starts_with('元') {
                continue;
            }
            // "the 20th" is a day only at a clause's end or before "of"
            // ("on the 20th.", "the 20th of"); "the 10th straight quarter"
            // is a count.
            if k == 6 {
                let next: String = text[e..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .collect::<String>()
                    .to_ascii_lowercase();
                if COUNT_NOUNS.contains(&next.as_str()) {
                    continue;
                }
            }
            let (y, m, d) = match k {
                0 | 3 => (num(c.get(1)), num(c.get(2)), num(c.get(3))),
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
                4 => (
                    num(c.get(3)).map(|y| if y < 100 { 2000 + y } else { y }),
                    num(c.get(1)),
                    num(c.get(2)),
                ),
                5 => (
                    c.get(1).and_then(|y| zh_number(y.as_str())),
                    c.get(2).and_then(|m| zh_number(m.as_str())),
                    c.get(3).and_then(|d| zh_number(d.as_str())),
                ),
                _ => (None, None, None),
            };
            let date = match (m, d) {
                (Some(m), Some(d)) => NaiveDate::from_ymd_opt(
                    y.unwrap_or(year_of.year() as i64) as i32,
                    m as u32,
                    d as u32,
                ),
                _ => None,
            };
            found.push((s, e, date));
        }
    }
    // The longest at each place, then nothing that overlaps it: "11月19日"
    // is one date, not also a bare "19日".
    found.sort_by_key(|&(s, e, _)| (s, std::cmp::Reverse(e)));
    let mut out = Vec::new();
    let mut last_end = 0;
    for (s, e, date) in found {
        if s < last_end {
            continue;
        }
        last_end = e;
        out.push((text[s..e].to_string(), date));
    }
    out
}

/// Calendar dates written in `text` ("Nov 19", "November 19, 2026",
/// "11/19", "11月19日", "2026年11月19日", "十一月十九日", "2026-11-19"), year
/// taken from `year_of` when absent. Every shape is read in both languages,
/// as lines mix scripts. Days that cannot be anchored ("the 20th",
/// "Friday") are left out here; `check` refuses them.
pub fn dates_in(text: &str, _lang: Lang, year_of: NaiveDate) -> Vec<NaiveDate> {
    date_marks(text, year_of)
        .into_iter()
        .filter_map(|(_, d)| d)
        .collect()
}

/// Months named without a day ("in December", "12月"), as month numbers.
fn bare_months(text: &str) -> Vec<(String, u32)> {
    static R: OnceLock<[Regex; 2]> = OnceLock::new();
    let [en, zh] = R.get_or_init(|| {
        [
            re(&format!("(?:{})", MONTH_NAMES.join("|"))),
            re(r"(\d{1,2}|[一二三四五六七八九十]{1,2})\s*月(?:份)?"),
        ]
    });
    let text = ascii_digits(text);
    let dated: Vec<(usize, usize)> = {
        // Spans of full dates, found again by position.
        let mut spans = Vec::new();
        let mut from = 0;
        for (raw, _) in date_marks(&text, NaiveDate::MIN) {
            if let Some(at) = text[from..].find(&raw) {
                spans.push((from + at, from + at + raw.len()));
                from += at + raw.len();
            }
        }
        spans
    };
    let inside = |s: usize, e: usize| dated.iter().any(|&(a, b)| s < b && a < e);
    let mut out = Vec::new();
    for m in find_alone(en, &text) {
        if !inside(m.start(), m.end()) {
            out.push((m.as_str().to_string(), month_number(m.as_str())));
        }
    }
    for c in zh.captures_iter(&text) {
        let w = c.get(0).expect("match");
        let n = &c[1];
        let month = n.parse::<i64>().ok().or_else(|| zh_number(n));
        // "11月19日" is a date; "第4季度"-style counts are not months.
        if inside(w.start(), w.end()) || text[w.end()..].starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        if let Some(m) = month.filter(|m| (1..=12).contains(m)) {
            out.push((w.as_str().to_string(), m as u32));
        }
    }
    out
}

/// Four-digit years written with 年 ("2028年").
fn zh_years(text: &str) -> Vec<i32> {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| re(r"((?:19|20)\d{2})\s*年"));
    r.captures_iter(&ascii_digits(text))
        .filter_map(|c| c[1].parse().ok())
        .collect()
}

fn relative_en_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(&wide_words(RELATIVE_EN)))
}

fn relative_zh_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"未来(?:几|数)(?:天|周|个月|月|个季度|季度|年)|近(?:几|数)(?:天|周|个月)|接下来(?:几|数)(?:天|周|个月|月|个季度|季度)|短期内|年底前|年末前"))
}

fn relative_time(text: &str) -> Option<String> {
    find_alone(relative_en_re(), text)
        .first()
        .map(|m| m.as_str().to_string())
        .or_else(|| {
            RELATIVE_ZH
                .iter()
                .find(|w| {
                    // 去年同期 is "a year earlier", the same as "year over year".
                    text.match_indices(**w)
                        .any(|(at, m)| !(**w == "去年" && text[at + m.len()..].starts_with("同期")))
                })
                .map(|w| w.to_string())
        })
        .or_else(|| relative_zh_re().find(text).map(|m| m.as_str().to_string()))
}

/// A fiscal label and the forms of it a cited text may hold, lowercase,
/// hyphens as spaces.
fn fiscal_labels(text: &str) -> Vec<(String, Vec<String>)> {
    static R: OnceLock<Vec<(Regex, char)>> = OnceLock::new();
    let res = R.get_or_init(|| {
        // 'q': a quarter, 'h': a half, 'y': a fiscal year.
        vec![
            (re(r"(?i)Q([1-4])"), 'q'),
            (re(r"(?i)([1-4])Q"), 'q'),
            (re(r"(?i)H([12])"), 'h'),
            (
                re(r"(?i)(first|second|third|fourth)[- ](?:fiscal[- ])?quarter"),
                'q',
            ),
            (re(r"(?i)(first|second)[- ]half"), 'h'),
            (re(r"(?i)fiscal\s+(?:year\s+)?(20\d{2})"), 'y'),
            (re(r"(?i)FY\s?(\d{4}|\d{2})"), 'y'),
            (re(r"第?([一二三四1-4])季度"), 'q'),
            (re(r"([上下])半年"), 'h'),
            (re(r"(\d{4})\s*财年"), 'y'),
        ]
    });
    const ORD: [&str; 4] = ["first", "second", "third", "fourth"];
    const ZH: [&str; 4] = ["一", "二", "三", "四"];
    let index = |s: &str| -> usize {
        let lower = s.to_ascii_lowercase();
        ORD.iter()
            .position(|o| *o == lower)
            .or_else(|| ZH.iter().position(|z| *z == s))
            .or_else(|| (s == "上").then_some(0))
            .or_else(|| (s == "下").then_some(1))
            .or_else(|| s.parse::<usize>().ok().map(|n| n - 1))
            .unwrap_or(0)
    };
    let quarter = |n: usize| {
        vec![
            format!("q{n}"),
            format!("{n}q"),
            format!("{} quarter", ORD[n - 1]),
            format!("第{}季度", ZH[n - 1]),
        ]
    };
    let half = |n: usize| {
        vec![
            format!("h{n}"),
            format!("{} half", ORD[n - 1]),
            if n == 1 { "上半年" } else { "下半年" }.to_string(),
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
    for (r, kind) in res {
        for c in r.captures_iter(&text) {
            let whole = c.get(0).expect("match");
            let latin = whole.as_str().is_ascii();
            if latin && !ascii_alone(&text, whole.start(), whole.end()) {
                continue;
            }
            // 上一季度 (the prior quarter), 前一季度, 同一季度, 下一季度,
            // 每一季度, 这一季度 and 该一季度 name no fiscal quarter.
            if *kind == 'q'
                && whole.as_str().contains('季')
                && !whole.as_str().starts_with('第')
                && text[..whole.start()].ends_with(['上', '前', '同', '下', '每', '这', '该'])
            {
                continue;
            }
            let forms = match kind {
                'q' => quarter(index(&c[1]) + 1),
                'h' => half(index(&c[1]) + 1),
                _ => year(&c[1]),
            };
            out.push((whole.as_str().to_string(), forms));
        }
    }
    out
}

fn lower_spaced(s: &str) -> String {
    s.to_lowercase().replace('-', " ")
}

/// Every number in `text` traces to a number in `texts` (`numbers::check`;
/// full-width digits read as ASCII).
pub(crate) fn numbers_from(text: &str, lang: Lang, texts: &[&str]) -> Result<(), Vec<String>> {
    let allowed: Vec<f64> = texts
        .iter()
        .flat_map(|t| crate::facts::quote_numbers(&ascii_digits(t)))
        .collect();
    numbers::check(&ascii_digits(text), lang, &allowed)
}

/// The dates written in `text` that are neither in `dates` nor written in
/// `texts`. A date without a year may fall in `year_of`'s year or the one
/// before or after; one that cannot be anchored is always returned.
pub(crate) fn dates_not_in(
    text: &str,
    year_of: Option<NaiveDate>,
    dates: &[NaiveDate],
    texts: &[&str],
) -> Vec<String> {
    let base = year_of.unwrap_or(NaiveDate::MIN);
    let years: Vec<NaiveDate> = [0, -1, 1]
        .iter()
        .filter_map(|k| year_of.and_then(|y| y.with_year(y.year() + k)))
        .collect();
    let mut allowed: Vec<NaiveDate> = dates.to_vec();
    for y in &years {
        for t in texts {
            allowed.extend(dates_in(t, Lang::En, *y));
        }
    }
    let variants: Vec<Vec<Mark>> = years.iter().map(|y| date_marks(text, *y)).collect();
    date_marks(text, base)
        .into_iter()
        .enumerate()
        .filter(|(k, _)| {
            !variants.iter().any(|v| {
                v.get(*k)
                    .and_then(|m| m.1)
                    .is_some_and(|d| allowed.contains(&d))
            })
        })
        .map(|(_, (raw, _))| raw)
        .collect()
}

/// The dates written in `text` with the text that wrote them, each read in
/// `year`'s year and the ones either side.
fn written_dates(text: &str, year: Option<NaiveDate>) -> Vec<(String, NaiveDate)> {
    [0, -1, 1]
        .iter()
        .filter_map(|k| year.and_then(|y| y.with_year(y.year() + k)))
        .flat_map(|y| date_marks(text, y))
        .filter_map(|(raw, d)| d.map(|d| (raw, d)))
        .collect()
}

/// Where the clause holding byte `at` starts: after the last comma,
/// semicolon, colon, sentence end or clause-opening conjunction before it
/// ("and", "but", "while", "whereas"; 并, 而, 但 only before 将, 于 or 在, so
/// 合并 is one word). A decimal point is not a boundary.
fn clause_start(text: &str, at: usize) -> usize {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| {
        re(r"[,，;；:：。!?！？、]|\.(?:\s|$)|\s(?:and|but|while|whereas)\s|[并而但][将于在]")
    });
    r.find_iter(&text[..at]).last().map_or(0, |m| {
        if m.as_str().starts_with(['并', '而', '但']) {
            m.start() + '并'.len_utf8()
        } else {
            m.end()
        }
    })
}

/// The 预计 or 预期 before an unconfirmed date (within eight characters, in
/// its clause) that makes the company, not the market, the one expecting it.
fn company_expects(text: &str, raw: &str) -> Option<String> {
    let at = text.find(raw)?;
    let before = &text[clause_start(text, at)..at];
    let chars: Vec<char> = before.chars().collect();
    let window: String = chars[chars.len().saturating_sub(8)..].iter().collect();
    ["预计", "预期"].iter().find_map(|w| {
        window
            .match_indices(w)
            .any(|(i, _)| !window[..i].ends_with("市场"))
            .then(|| window.clone())
    })
}

/// Whether a hedge word governs the date written as `raw` in `text`: one
/// of the six words before it in its own clause, or of the three after it
/// ("expected to report results on November 20", "a report scheduled for
/// November 20", "November 20, an expected date"); in Chinese, within eight
/// characters before it in its clause or six after it ("预计于11月20日",
/// "财报暂定于11月20日公布"). A hedge about something else earlier in the
/// clause ("Apple's expected revenue … at its November 20 report") does
/// not count.
fn hedge_governs(text: &str, raw: &str, lang: Lang) -> bool {
    let Some(at) = text.find(raw) else {
        return false;
    };
    let before = &text[clause_start(text, at)..at];
    let after_all = &text[at + raw.len()..];
    let after = &after_all[..after_all.find(['。', ';', '；']).unwrap_or(after_all.len())];
    match lang {
        Lang::En => {
            let words = |s: &str| -> Vec<String> {
                s.split(|c: char| !c.is_ascii_alphanumeric())
                    .filter(|w| !w.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect()
            };
            let b = words(before);
            let a = words(after);
            b.iter()
                .rev()
                .take(6)
                .chain(a.iter().take(3))
                .any(|w| HEDGE_EN.contains(&w.as_str()))
        }
        Lang::Zh => {
            let b: String = {
                let chars: Vec<char> = before.chars().collect();
                chars[chars.len().saturating_sub(8)..].iter().collect()
            };
            let a: String = after.chars().take(8).collect();
            DATE_HEDGE_ZH.iter().any(|w| b.contains(w) || a.contains(w))
        }
    }
}

/// The pack's trading date (its move's date), for dates written without a year.
pub(crate) fn year_of(pack: &Pack) -> Option<NaiveDate> {
    pack.evidence
        .iter()
        .find(|e| e.kind == EvidenceKind::Move)
        .or(pack.evidence.first())
        .and_then(|e| e.date)
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
    if let Some(m) = worded_figures(&hook).first() {
        bad.push(format!("hook has a figure in words: {m}"));
    }
    if let Some(m) = hook_count(&hook) {
        bad.push(format!("hook has a figure in words: {m}"));
    }
    if both.contains([';', '；']) {
        bad.push("has a semicolon (two clauses)".to_string());
    }
    if !one_sentence(&watch) {
        bad.push("watch line is not one sentence".to_string());
    }
    let markup = ['*', '`', '#', '<', '>', '[', ']', '\n', '\r'];
    if both.contains(markup) || both.contains("://") || both.to_ascii_lowercase().contains("www.") {
        bad.push("has markup, a link or a line break".to_string());
    }

    // 2. Cites.
    let mut cited: Vec<&Evidence> = Vec::new();
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
    if has_digit(&hook) && !digits_only_in_names(&hook, &cited_text) {
        bad.push("hook has a digit".to_string());
    }
    // A news item with no outlet comes from an aggregator or search page
    // (Google, Yahoo Finance): it has no one to attribute a fact to.
    if cited.iter().any(|e| {
        matches!(e.kind, EvidenceKind::News | EvidenceKind::AnalystAction) && e.outlet.is_none()
    }) {
        bad.push(
            "cites a news item with no outlet (an aggregator), which cannot carry a fact"
                .to_string(),
        );
    }
    // Chinese figures use 万 and 亿.
    if million_zh().is_match(&ascii_digits(&both)) {
        bad.push("a figure in 百万 or 千万: write it in 万 or 亿".to_string());
    }
    // The cited outlets' names, which hold digits ("24/7 Wall St.") or
    // dots, are left out of the number, date and label checks.
    let outlet_forms: Vec<String> = cited
        .iter()
        .filter_map(|e| e.outlet.as_deref())
        .flat_map(|o| outlet_names(o, lang))
        .collect();
    let (both_n, watch_n) = (both.clone(), watch.clone());
    let both = scrub(&both, &outlet_forms);
    let watch = scrub(&watch, &outlet_forms);

    // 3. Numbers, never from an analyst action (its target or rating).
    let number_texts: Vec<&str> = cited
        .iter()
        .zip(&cited_text)
        .filter(|(e, _)| e.kind != EvidenceKind::AnalystAction)
        .map(|(_, t)| t.as_str())
        .collect();
    if let Err(nums) = numbers_from(&both, lang, &number_texts) {
        bad.push(format!(
            "numbers not in the cited evidence: {}",
            nums.join(", ")
        ));
    }
    let cited_lower: Vec<String> = cited_text.iter().map(|t| lower_spaced(t)).collect();
    for w in worded_figures(&watch) {
        if !cited_lower.iter().any(|t| t.contains(&lower_spaced(&w))) {
            bad.push(format!("a figure in words: {w}"));
        }
    }

    // 4. Dates, years and months.
    let year = year_of(pack);
    let pack_dates: Vec<NaiveDate> = pack.evidence.iter().filter_map(|e| e.date).collect();
    let cited_refs: Vec<&str> = cited_text.iter().map(String::as_str).collect();
    for raw in dates_not_in(&both, year, &pack_dates, &cited_refs) {
        bad.push(format!("a date that is not in the evidence: {raw}"));
    }
    let mut known: Vec<NaiveDate> = pack_dates.clone();
    for t in &cited_refs {
        if let Some(y) = year {
            known.extend(dates_in(t, lang, y));
        }
    }
    for y in zh_years(&both) {
        let in_text = cited_refs.iter().any(|t| {
            t.match_indices(&y.to_string())
                .any(|(at, m)| ascii_alone(t, at, at + m.len()))
        });
        if !in_text && !known.iter().any(|d| d.year() == y) {
            bad.push(format!("a year not in the evidence: {y}年"));
        }
    }
    for (raw, month) in bare_months(&both) {
        let named = cited_lower
            .iter()
            .any(|t| t.contains(&MONTH_NAMES[month as usize - 1].to_lowercase()));
        if !named && !known.iter().any(|d| d.month() == month) {
            bad.push(format!("a month that is not in the evidence: {raw}"));
        }
    }

    // 5. Relative time.
    if let Some(w) = relative_time(&both) {
        bad.push(format!("relative time: {w}"));
    }

    // 6. Fiscal labels.
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
    let (both, watch) = (both_n, watch_n);
    for o in &cited_outlets {
        if !outlet_names(o, lang).iter().any(|n| attributes(&watch, n)) {
            bad.push(format!(
                "does not attribute to {o}, the outlet of a cited item (\"{o} reports\", \"据{o}报道\")"
            ));
        }
    }
    for (name, _, _) in OUTLET_NAMES {
        if cited_outlets.contains(name) {
            continue;
        }
        let named = outlet_names(name, lang).iter().any(|n| {
            if COMMON_NAMES.contains(&n.as_str()) {
                names_attributed(&both, n)
            } else {
                names(&both, n)
            }
        });
        if named {
            bad.push(format!("names {name}, which no cited item is from"));
        }
    }
    // The mover-news brands too, where the line attributes to them: many
    // are also ordinary words or companies ("Nasdaq", "Morningstar").
    for (host, name) in super::pack::MOVER_OUTLETS {
        if cited_outlets.contains(name) || OUTLET_NAMES.iter().any(|(n, _, _)| n == name) {
            continue;
        }
        if attributes(&both, name) || attributes(&both, host) {
            bad.push(format!("names {name}, which no cited item is from"));
        }
    }

    // 8. An unconfirmed report date, cited or written, is hedged.
    let unconfirmed: Vec<&Evidence> = pack
        .evidence
        .iter()
        .filter(|e| e.kind == EvidenceKind::ReportDate && e.unconfirmed)
        .collect();
    let firm: Vec<NaiveDate> = pack
        .evidence
        .iter()
        .filter(|e| !e.unconfirmed)
        .filter_map(|e| e.date)
        .collect();
    let written: Vec<NaiveDate> = [0, -1, 1]
        .iter()
        .filter_map(|k| year.and_then(|y| y.with_year(y.year() + k)))
        .flat_map(|y| dates_in(&both, lang, y))
        .collect();
    let leans_on_unconfirmed = cited.iter().any(|e| e.unconfirmed)
        || written
            .iter()
            .any(|d| unconfirmed.iter().any(|e| e.date == Some(*d)) && !firm.contains(d));
    if leans_on_unconfirmed {
        let hedged_in = |clause: &str| match lang {
            Lang::En => HEDGE_EN.iter().any(|w| has_word(clause, w)),
            Lang::Zh => HEDGE_ZH.iter().any(|w| clause.contains(w)),
        };
        // Each unconfirmed date the line writes has a hedge that governs it
        // ("预计营收…，并将于11月20日公布财报" hedges the revenue, not the
        // date); a line that only cites the item hedges it anywhere.
        let mut unhedged = false;
        let mut dated = false;
        for (raw, d) in written_dates(&watch, year) {
            if unconfirmed.iter().any(|e| e.date == Some(d)) && !firm.contains(&d) {
                dated = true;
                if !hedge_governs(&watch, &raw, lang) {
                    unhedged = true;
                }
                if lang == Lang::Zh {
                    if let Some(who) = company_expects(&watch, &raw) {
                        bad.push(format!(
                            "an unconfirmed report date written as the company's own expectation: {who}"
                        ));
                    }
                }
            }
        }
        if unhedged || !dated && !hedged_in(&watch) {
            bad.push("an unconfirmed report date is not hedged in its clause".to_string());
        }
    }

    // A claim about a person's private life (Rick, 2026-11-17: "Allow it if
    // attributed"), where code can tell: see `private_life`.
    bad.extend(private_life(&line.hook, &line.watch, &cited, lang));

    // 9. Refusal list.
    let analyst_cited = cited.iter().any(|e| e.kind == EvidenceKind::AnalystAction);
    bad.extend(
        refused_wording(&both, analyst_cited)
            .into_iter()
            .map(|w| format!("refused wording: {w}")),
    );
    bad
}

/// The advice, rating, target, consensus and prediction wording in `text`
/// (rule 9 of `check`). With `analyst_cited`, a bare "target" is refused
/// too: next to an analyst action it is the price target.
pub fn refused_wording(text: &str, analyst_cited: bool) -> Vec<String> {
    let mut out = Vec::new();
    for r in refused_en() {
        if let Some(m) = has_phrase(text, r) {
            out.push(m);
        }
    }
    out.extend(advice_words(text, analyst_cited));
    if let Some(m) = advice_hold(text) {
        out.push(m);
    }
    for w in REFUSED_ZH {
        if text.contains(w) {
            out.push(w.to_string());
        }
    }
    // 配置 is "allocate" as advice (提高配置), but 资本配置 is capital
    // allocation, a company plan.
    if text
        .match_indices("配置")
        .any(|(at, _)| !(text[..at].ends_with("资本") || text[..at].ends_with("资源")))
    {
        out.push("配置".to_string());
    }
    // 持有 is advice ("继续持有该股"), except as "held" or "holds a stake":
    // 持有的现金, 持有约一半股份, 持有该公司12%股权.
    let held = |rest: &str| {
        let head: String = rest.chars().take(12).collect();
        rest.starts_with(['的', '约'])
            || rest
                .starts_with(|c: char| c.is_ascii_digit() || "一二三四五六七八九十两".contains(c))
            || (head.contains("股份") || head.contains("股权")) && !head.contains("该股")
    };
    if text
        .match_indices("持有")
        .any(|(at, m)| !held(&text[at + m.len()..]))
    {
        out.push("持有".to_string());
    }
    out
}

/// Verbs that make "buy" and "sell" a deal, not advice: "agreed to sell its
/// Texas factory", "a deal to buy a chip designer". Not "will", "would" or
/// "plan": "investors would buy the shares" is advice.
const DEAL_WORDS: &[&str] = &[
    "agreed",
    "agree",
    "agrees",
    "agreement",
    "deal",
    "offer",
    "offered",
    "offers",
    "bid",
    "proposal",
    "proposed",
    "decided",
    "intends",
    "intend",
    "talks",
    "seeks",
    "seeking",
    "sought",
    "announced",
];

/// Words that make "buy" or "sell" advice when they come shortly before it.
const ADVICE_LEAD: &[&str] = &[
    "investors",
    "investor",
    "should",
    "could",
    "may",
    "might",
    "can",
    "time",
    "reason",
    "opportunity",
    "worth",
    "consider",
    "a",
    "an",
    "strong",
    "rated",
    "from",
];

/// The words before a rating: "downgraded Apple to", "moved the stock to".
fn rating_frame() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(&wide_words(RATING_FRAME)))
}

/// "buy", "sell", "target", "neutral", "upside" and "downside" in their
/// advice, rating or prediction senses, allowed in their plain senses.
fn advice_words(text: &str, analyst_cited: bool) -> Vec<String> {
    static R: OnceLock<[Regex; 4]> = OnceLock::new();
    let [trade, target, neutral, side] = R.get_or_init(|| {
        [
            re(r"(?i)(?:buying|buys|buy|selling|sells|sell)"),
            re(r"(?i)(?:(?:stock|share)\s+targets?|targets?\s+(?:to|of)\s+\$\d[\d.,]*(?:\s*(?:billion|million|bn|b|m))?|targets?\s+(?:on|for)\s+(?:the\s+)?(?:stock|shares|company)|targets?)"),
            re(r"(?i)neutral"),
            re(r"(?i)(?:upside|downside)"),
        ]
    });
    let lower = text.to_ascii_lowercase();
    let words_before = |at: usize, n: usize| -> Vec<String> {
        lower[..at]
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .rev()
            .take(n)
            .map(str::to_string)
            .collect()
    };
    let mut out = Vec::new();
    for m in find_alone(trade, text) {
        let exempt = EXEMPT_COMPOUNDS.iter().any(|c| {
            let from = lower[..m.start()]
                .rfind(|ch: char| !(ch.is_ascii_alphabetic() || ch == '-'))
                .map_or(0, |i| i + 1);
            lower[from..].starts_with(c)
        });
        if exempt {
            continue;
        }
        let after = &lower[m.end()..];
        let before = words_before(m.start(), 4);
        let rating = [
            "-rated",
            "-rating",
            " rating",
            " rated",
            " from ",
            " recommendation",
            " call",
            " signal",
        ]
        .iter()
        .any(|a| after.starts_with(a));
        let led = before
            .first()
            .is_some_and(|w| ADVICE_LEAD.contains(&w.as_str()))
            || before.iter().any(|w| {
                matches!(
                    w.as_str(),
                    "investors"
                        | "investor"
                        | "should"
                        | "time"
                        | "reason"
                        | "opportunity"
                        | "worth"
                        | "consider"
                )
            });
        let deal = before.iter().any(|w| DEAL_WORDS.contains(&w.as_str()));
        // A rating: "downgraded Apple to sell", "upgraded the stock to buy".
        let rated_to = rating_frame().is_match(&lower[..m.start()]);
        // A noun: "a buy", "a compelling buy", "an attractive buy" (an "a"
        // within three words, with no "to" between), or a buy at a clause
        // end; and "buying opportunity".
        let mut noun = false;
        for w in before.iter().take(3) {
            if w == "to" {
                break;
            }
            if w == "a" || w == "an" {
                noun = true;
                break;
            }
        }
        let word = m.as_str().to_ascii_lowercase();
        noun |= (word == "buy" || word == "sell")
            && after.starts_with([',', '.', ';', ':', '!', '?'])
            && before.first().is_none_or(|w| w != "to");
        let opportunity = after.starts_with(" opportunit");
        let sentence_start = lower[..m.start()]
            .trim_end()
            .chars()
            .next_back()
            .is_none_or(|c| matches!(c, '.' | ':' | '!' | '?'));
        let object = [
            " the dip",
            " the rally",
            " the news",
            " the stock",
            " the shares",
            " shares",
            " stock",
            " into ",
            " on weakness",
            " on strength",
        ]
        .iter()
        .any(|a| after.starts_with(a));
        if rating
            || rated_to
            || noun
            || opportunity
            || analyst_cited
            || (!deal && (led || sentence_start || object))
        {
            out.push(m.as_str().to_string());
        }
    }
    for m in find_alone(target, text) {
        let bare = m.as_str().to_ascii_lowercase().starts_with("target");
        let bare = bare && !m.as_str().contains(['$', ' ']);
        if !bare || analyst_cited {
            out.push(m.as_str().to_string());
        }
    }
    for m in find_alone(neutral, text) {
        let before = lower[..m.start()].trim_end_matches(['-', ' ']);
        if !(before.ends_with("carbon")
            || before.ends_with("climate")
            || before.ends_with("emissions"))
        {
            out.push(m.as_str().to_string());
        }
    }
    for m in find_alone(side, text) {
        if !lower[..m.start()].ends_with("on the ") {
            out.push(m.as_str().to_string());
        }
    }
    out
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
            quote: None,
            unconfirmed: false,
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

        // 21 characters (trial run 1 raised the limit from 14 to 20).
        let f = zh(line(
            "苹果业绩指引成为下一道重要考验而且值得关注",
            &zh_ok().watch,
            &["e2", "e5"],
        ));
        assert!(refused(&f, "hook"), "{f:?}");
        let watch = format!("据路透社报道，苹果预计营收42亿美元{}。", "并且".repeat(45));
        let f = zh(line("业绩指引成为下一道考验", &watch, &["e2"]));
        assert!(refused(&f, "characters"), "{f:?}");
        // Eight words and twenty characters are fine.
        assert!(en(line(
            "One two three four five six seven eight",
            &en_ok().watch,
            &["e2", "e5"]
        ))
        .is_empty());
        assert!(zh(line(
            "苹果业绩指引成为下一道重要考验而且值得关",
            &zh_ok().watch,
            &["e2", "e5"]
        ))
        .is_empty());
        // Run 1: a Chinese line of more than 80 characters with Latin names
        // now fits.
        let long = "据路透社报道，Rocket Lab预计营收42亿美元，并计划推进Neutron首飞，11月19日的财报将检验这一预期以及该时间表是否有变化，这一点值得公司内外持续密切跟踪。";
        assert!(long.chars().count() > 80, "{}", long.chars().count());
        let f = zh(line("亏损扩大，关注Neutron进度", long, &["e2", "e5"]));
        assert!(f.is_empty(), "{f:?}");
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
        assert!(refused(&f, "in words"), "{f:?}");
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
        assert!(refused(&f, "in words"), "{f:?}");
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
        // A host only with its top-level domain (round 4: "fool" and
        // "investing" are ordinary words).
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

    /// The reviewer's pack (PR #67 review probe), with its company items as
    /// the pack now writes them.
    fn review_pack() -> Pack {
        use EvidenceKind::*;
        Pack {
            ticker: "AAPL".into(),
            name: "Apple".into(),
            evidence: vec![
                ev("e1", Move, "Apple (AAPL) rose 4.20% in premarket trading.", None, Some("2026-11-12")),
                ev("e2", News, "Apple said it expects third quarter revenue of $4.2 billion and a $1.0 billion buyback.", Some("Reuters"), None),
                ev("e3", AnalystAction, "Morgan Stanley upgraded Apple to equal-weight and raised its price target to $250.", Some("CNBC"), None),
                ev("e4", News, "Apple plans to open a factory in Texas next year, the company said.", Some("The Wall Street Journal"), None),
                ev("e5", ReportDate, "Apple is scheduled to report results on November 19, 2026 (earnings calendar).", None, Some("2026-11-19")),
                ev("e6", CompanyGuidance, "the Company expects gross margin of 70 percent in fiscal 2027", None, None),
                ev("e10", MacroEvent, "October CPI on November 13, 2026.", None, Some("2026-11-13")),
            ],
        }
    }

    /// Every input the review found let through (or refused wrongly), as
    /// (language, should refuse, hook, watch, cites).
    #[test]
    fn the_reviews_probe_inputs() {
        const H: &str = "Guidance sets the next test";
        const HZ: &str = "业绩指引成为下一道考验";
        let en_watch = "Reuters reports Apple expects revenue of $4.2 billion, and its November 19 report will show whether that plan holds.";
        let zh_watch = "据路透社报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。";
        let mut cases: Vec<(Lang, bool, String, String, Vec<&str>)> = vec![];
        let mut en_case = |refuse: bool, hook: &str, watch: &str, cites: &[&'static str]| {
            cases.push((
                En,
                refuse,
                hook.to_string(),
                watch.to_string(),
                cites.to_vec(),
            ))
        };
        en_case(false, H, en_watch, &["e2", "e5"]);
        en_case(true, H, "Reuters reports Apple expects revenue to grow ten percent before its November 19 report.", &["e2", "e5"]);
        en_case(
            true,
            H,
            "Reuters reports Apple expects revenue of $4.2 billion before its 11/20 report.",
            &["e2"],
        );
        en_case(
            true,
            H,
            "Reuters reports Apple expects revenue of $4.2 billion before results on the 20th.",
            &["e2"],
        );
        en_case(
            true,
            H,
            "Reuters reports Apple expects revenue of $4.2 billion ahead of Friday's report.",
            &["e2"],
        );
        en_case(
            true,
            H,
            "Reuters reports Apple expects revenue of $4.2 billion before results in December.",
            &["e2"],
        );
        en_case(
            true,
            H,
            "Reuters reports Apple expects revenue of $4.2 billion before results on Dec. 3.",
            &["e2"],
        );
        en_case(
            true,
            H,
            "Reuters reports Apple expects revenue of $4.2 billion by 2028.",
            &["e2"],
        );
        for w in [
            "later this year",
            "this quarter",
            "over the next two weeks",
            "in coming weeks",
            "later today",
            "in the months ahead",
            "shortly",
        ] {
            let watch = format!(
                "Reuters reports Apple expects revenue of $4.2 billion, a figure to track {w}."
            );
            cases.push((En, true, H.to_string(), watch, vec!["e2"]));
        }
        let mut en_case = |refuse: bool, hook: &str, watch: &str, cites: &[&'static str]| {
            cases.push((
                En,
                refuse,
                hook.to_string(),
                watch.to_string(),
                cites.to_vec(),
            ))
        };
        en_case(true, H, "Reuters reports Apple expects fourth-quarter revenue of $4.2 billion, a figure its November 19 report will test.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports Apple expects 4Q revenue of $4.2 billion, a figure its November 19 report will test.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports Apple expects H2 revenue of $4.2 billion, a figure its November 19 report will test.", &["e2", "e5"]);
        en_case(true, "Reuters flags the guidance test", "Apple expects revenue of $4.2 billion, and its November 19 report will show whether that plan holds.", &["e2", "e5"]);
        // A news fact stated without citing its item: the judge's job (the
        // review's "only the judge can catch" list).
        en_case(
            false,
            H,
            "Apple plans to open a factory in Texas, a project to track at its November 19 report.",
            &["e5"],
        );
        en_case(true, H, "CNBC reports Morgan Stanley upgraded Apple to equal-weight, while Reuters reports revenue of $4.2 billion before its November 19 report.", &["e2", "e3", "e5"]);
        en_case(true, H, "CNBC reports Morgan Stanley raised its target to $250, and Reuters reports Apple expects revenue of $4.2 billion before its November 19 report.", &["e2", "e3", "e5"]);
        en_case(true, H, "CNBC reports Morgan Stanley raised its view to $250, and Reuters reports Apple expects revenue of $4.2 billion before its November 19 report.", &["e2", "e3", "e5"]);
        en_case(true, H, "Reuters reports the Buy-rated company expects revenue of $4.2 billion before its November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports the Overweight-rated company expects revenue of $4.2 billion before its November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports revenue of $4.2 billion, above what Wall Street expects, ahead of the November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports revenue of $4.2 billion, ahead of the consensus estimate, before the November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports revenue of $4.2 billion, leaving upside for the shares into the November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports revenue of $4.2 billion, and the shares could climb into the November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports revenue of $4.2 billion, which may make the November 19 report an attractive entry point.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports Apple expects revenue of $4.2 billion; investors may wish to add exposure before November 19.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports Apple expects revenue of $4.2 billion. investors may wish to add exposure before November 19.", &["e2", "e5"]);
        en_case(false, H, "Reuters reports Apple expects revenue of $4.2 billion, a figure to weigh with St. Louis Fed data and the October CPI on November 13.", &["e2", "e10"]);
        en_case(false, H, "Reuters reports Apple Inc. Chief Executive comments came with revenue of $4.2 billion before November 19.", &["e2", "e5"]);
        en_case(false, H, "Reuters reports Apple expects revenue of $4.2 billion, and the company will hold its report on November 19.", &["e2", "e5"]);
        en_case(false, H, "Reuters reports Apple expects revenue of $4.2 billion, a test for a Fortune company at its November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports Apple expects revenue of $4.2 billion, which Fortune reported before the November 19 report.", &["e2", "e5"]);
        en_case(false, H, "Reuters reports a $1.0 billion buyback after the sell-off, a plan to track at the November 19 report.", &["e2", "e5"]);
        en_case(true, H, "Reuters reports Apple expects revenue of $4.2 billion, and analysts downgraded the stock to hold before November 19.", &["e2", "e5"]);
        let mut zh_case = |refuse: bool, watch: &str, cites: &[&'static str]| {
            cases.push((
                Zh,
                refuse,
                HZ.to_string(),
                watch.to_string(),
                cites.to_vec(),
            ))
        };
        zh_case(false, zh_watch, &["e2", "e5"]);
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，十一月二十日的财报将检验这一预期。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "公司预计毛利率达百分之七十五，后续财报将检验这一目标。",
            &["e6"],
        );
        zh_case(
            true,
            "公司预计毛利率提升三成，后续财报将检验这一目标。",
            &["e6"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计第4季度营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计四季度营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计Q4营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        );
        zh_case(
            false,
            "据路透社报道，苹果预计Q3营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        );
        zh_case(
            false,
            "据路透社报道，苹果预计十一月十九日公布财报，届时将检验营收42亿美元的预期。",
            &["e2", "e5"],
        );
        for w in [
            "下季度",
            "明年",
            "今年晚些时候",
            "即将",
            "近日",
            "接下来几周",
            "下月",
        ] {
            cases.push((
                Zh,
                true,
                HZ.to_string(),
                format!("据路透社报道，苹果预计营收42亿美元，{w}值得关注。"),
                vec!["e2"],
            ));
        }
        let mut zh_case = |refuse: bool, watch: &str, cites: &[&'static str]| {
            cases.push((
                Zh,
                refuse,
                HZ.to_string(),
                watch.to_string(),
                cites.to_vec(),
            ))
        };
        zh_case(true, "据美国消费者新闻与商业频道报道，摩根士丹利将目标股价上调至250美元，据路透社报道苹果预计营收42亿美元。", &["e2", "e3"]);
        zh_case(true, "据美国消费者新闻与商业频道报道，摩根士丹利将评级上调至中性，据路透社报道苹果预计营收42亿美元。", &["e2", "e3"]);
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，值得推荐关注11月19日财报。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，股价有望上涨至11月19日。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，可在11月19日前逢低加仓。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，本月20日的财报将检验这一预期。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，11/20的财报将检验这一预期。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "公司预计2028年毛利率达70%，后续财报将检验这一目标。",
            &["e6"],
        );
        zh_case(
            false,
            "公司预计2027财年毛利率达70%，后续财报将检验这一目标。",
            &["e6"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元；投资者不妨在11月19日前提高配置。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，周五的财报将检验这一预期。",
            &["e2"],
        );
        zh_case(
            false,
            "据路透社报道，苹果预计营收42亿美元，公司持有的现金将在11月19日财报中披露。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据路透社报道，苹果预计营收42亿美元，投资者可继续持有至11月19日财报。",
            &["e2", "e5"],
        );
        zh_case(
            false,
            "据路透社报道，苹果预计营收42亿美元，市场观察人士关注11月19日的财报。",
            &["e2", "e5"],
        );
        zh_case(
            true,
            "据市场观察报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
            &["e2", "e5"],
        );

        let p = review_pack();
        let mut wrong = vec![];
        for (lang, refuse, hook, watch, cites) in &cases {
            let l = line(hook, watch, cites);
            let i = match lang {
                En => insight(l, zh_ok()),
                Zh => insight(en_ok(), l),
            };
            let f = check(&p, &i);
            if f.is_empty() == *refuse {
                wrong.push(format!("{watch} -> {f:?}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn an_unconfirmed_report_date_must_be_hedged() {
        let mut p = pack();
        p.evidence.push(Evidence {
            id: "e7".into(),
            kind: EvidenceKind::ReportDate,
            text: "Expected report date, not confirmed by the company: January 29, 2027.".into(),
            outlet: None,
            url: None,
            date: Some(d("2027-01-29")),
            quote: None,
            unconfirmed: true,
        });
        let en = |watch: &str, cites: &[&str]| {
            check(
                &p,
                &insight(line("Guidance sets the next test", watch, cites), zh_ok()),
            )
        };
        let zh = |watch: &str, cites: &[&str]| {
            check(
                &p,
                &insight(en_ok(), line("业绩指引成为下一道考验", watch, cites)),
            )
        };
        let plain =
            "Reuters reports Apple expects revenue of $4.2 billion before its January 29 report.";
        let f = en(plain, &["e2", "e7"]);
        assert!(refused(&f, "not hedged"), "{f:?}");
        // Writing the date without citing the item needs the hedge too.
        let f = en(plain, &["e2"]);
        assert!(refused(&f, "not hedged"), "{f:?}");
        for hedged in [
            "Reuters reports Apple expects revenue of $4.2 billion before its expected January 29 report.",
            "Reuters reports Apple expects revenue of $4.2 billion before its report, scheduled for January 29.",
            "Reuters reports Apple expects revenue of $4.2 billion before its unconfirmed January 29 report.",
        ] {
            assert!(en(hedged, &["e2", "e7"]).is_empty(), "{hedged}");
        }
        let f = zh(
            "据路透社报道，苹果营收42亿美元，1月29日的财报将检验这一数字。",
            &["e2", "e7"],
        );
        assert!(refused(&f, "not hedged"), "{f:?}");
        // After trial run 2, 预计 and 预期 hedge a date only with 市场 as
        // their subject: alone they read as the company's own expectation.
        for w in ["暂定", "未确认", "未经公司确认", "市场预计"] {
            let watch =
                format!("据路透社报道，苹果营收42亿美元，{w}1月29日公布的财报将检验这一数字。");
            assert!(zh(&watch, &["e2", "e7"]).is_empty(), "{watch}");
        }
        for w in ["预计", "预期"] {
            let watch =
                format!("据路透社报道，苹果营收42亿美元，{w}1月29日公布的财报将检验这一数字。");
            assert!(!zh(&watch, &["e2", "e7"]).is_empty(), "{watch}");
        }
        // A confirmed date needs no hedge.
        assert!(en(&en_ok().watch, &["e2", "e5"]).is_empty());
    }

    /// Round 2, M1: refused_wording against every stored mover reason of
    /// 2026-11-04 to 2026-11-12. Only consensus comparisons and analyst
    /// actions are refused; deals ("agreed to sell"), plans ("targets a
    /// launch") and sell-offs pass.
    #[test]
    fn real_reasons_are_refused_only_for_consensus_and_analyst_wording() {
        let fixture = include_str!("testdata/reasons_2026_11.tsv");
        let mut n = 0;
        for line in fixture.lines().filter(|l| !l.starts_with('#')) {
            let mut cols = line.splitn(3, '\t');
            let (want, ticker, quote) = (
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap(),
            );
            let got = refused_wording(quote, false);
            assert_eq!(got.is_empty(), want == "ok", "{ticker}: {quote} -> {got:?}");
            n += 1;
        }
        assert_eq!(n, 50);
    }

    /// Round 2 review inputs (M1, M3, L1, L2), as (language, should refuse,
    /// hook, watch, cites) against the review pack plus an unconfirmed
    /// report date on November 20.
    #[test]
    fn the_round_two_probe_inputs() {
        const H: &str = "Guidance sets the next test";
        const HZ: &str = "业绩指引成为下一道考验";
        let mut p = review_pack();
        p.evidence.push(Evidence {
            id: "e11".into(),
            kind: EvidenceKind::ReportDate,
            text: "Expected report date, not confirmed by the company: November 20, 2026.".into(),
            outlet: None,
            url: None,
            date: Some(d("2026-11-20")),
            quote: None,
            unconfirmed: true,
        });
        let en_cases: &[(bool, &str, &str, &[&str])] = &[
            // M1: deals and plans pass; consensus and advice senses do not.
            (false, H, "Reuters reports Apple agreed to sell its Texas factory before its November 19 report.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple agreed to buy a chip designer before its November 19 report.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple targets a first launch before its November 19 report.", &["e2", "e5"]),
            (false, H, "The company guided an operating margin target of 70 percent for fiscal 2027.", &["e6"]),
            (false, H, "Reuters reports a $1.0 billion buyback, with sell-through data to track at the November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, above analysts' estimates, before its November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, ahead of Street estimates, before its November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, versus market estimates, before its November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports Apple expects revenue of $4.2 billion, and investors may wish to buy before November 19.", &["e2", "e5"]),
            (true, "A buy case into results", "Reuters reports Apple expects revenue of $4.2 billion before its November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports Apple expects revenue of $4.2 billion, a reason to sell the stock before November 19.", &["e2", "e5"]),
            (true, H, "CNBC reports Morgan Stanley raised its target on the shares, and Reuters reports revenue of $4.2 billion.", &["e2", "e3"]),
            (true, H, "CNBC reports Morgan Stanley moved Apple, and Reuters reports revenue of $4.2 billion against a target.", &["e2", "e3"]),
            // L1: plain senses pass.
            (false, H, "Reuters reports Apple expects revenue of $4.2 billion and a carbon neutral supply chain by its November 19 report.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple expects revenue of $4.2 billion for the 10th straight quarter of growth before November 19.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple expects double-digit growth and revenue of $4.2 billion before November 19.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple expects revenue of $4.2 billion, which the November 19 report will test on the downside.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, leaving upside for the shares into the November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion before results on the 20th.", &["e2"]),
            // L2: prediction and relative time.
            (true, H, "Reuters reports revenue of $4.2 billion, and the shares could gain into the November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, and the shares look poised to react at the November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, a figure to watch in the near term.", &["e2"]),
            (true, H, "Reuters reports revenue of $4.2 billion, a figure to watch by year-end.", &["e2"]),
            (true, H, "Reuters reports revenue of $4.2 billion, a figure to watch later in the year.", &["e2"]),
            // M3: the hedge belongs to the date's own clause.
            (true, H, "Reuters reports Apple expected revenue of $4.2 billion, and it reports results on November 20.", &["e2", "e11"]),
            (false, H, "Reuters reports Apple expects revenue of $4.2 billion, and it is expected to report results on November 20.", &["e2", "e11"]),
            (false, H, "Reuters reports revenue of $4.2 billion before a report scheduled for November 20.", &["e2", "e11"]),
        ];
        let zh_cases: &[(bool, &str, &str, &[&str])] = &[
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，高于分析师预期，11月19日的财报将检验这一水平。",
                &["e2", "e5"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，与一致预期相符，11月19日的财报将检验这一水平。",
                &["e2", "e5"],
            ),
            (
                false,
                HZ,
                "据路透社报道，苹果营收42亿美元，高于去年同期，11月19日的财报将检验这一水平。",
                &["e2", "e5"],
            ),
            (
                false,
                HZ,
                "据路透社报道，苹果营收42亿美元，资本配置计划将在11月19日的财报中披露。",
                &["e2", "e5"],
            ),
            (
                false,
                HZ,
                "据路透社报道，苹果营收42亿美元，公司持有约一半合资公司股份，11月19日财报将披露。",
                &["e2", "e5"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，投资者可继续持有该股至11月19日财报。",
                &["e2", "e5"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，股价有望反弹至11月19日。",
                &["e2", "e5"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，短期内值得关注。",
                &["e2"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，年底前值得关注。",
                &["e2"],
            ),
            (
                true,
                "苹果财报三大看点",
                "据路透社报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
                &["e2", "e5"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果预计营收42亿美元，并将于11月20日公布财报。",
                &["e2", "e11"],
            ),
            // Since trial run 2: 预计 alone reads as the company's own date.
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，预计于11月20日公布财报。",
                &["e2", "e11"],
            ),
            (
                false,
                HZ,
                "据路透社报道，苹果营收42亿美元，财报暂定于11月20日公布。",
                &["e2", "e11"],
            ),
        ];
        let mut wrong = vec![];
        for (refuse, hook, watch, cites) in en_cases {
            let f = check(&p, &insight(line(hook, watch, cites), zh_ok()));
            if f.is_empty() == *refuse {
                wrong.push(format!("{watch} -> {f:?}"));
            }
        }
        for (refuse, hook, watch, cites) in zh_cases {
            let f = check(&p, &insight(en_ok(), line(hook, watch, cites)));
            if f.is_empty() == *refuse {
                wrong.push(format!("{hook} {watch} -> {f:?}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Round 3 review inputs (M1, M2, L1, L2, L4), as (language, should
    /// refuse, hook, watch, cites), with an unconfirmed report date on
    /// November 20 (e11).
    #[test]
    fn the_round_three_probe_inputs() {
        const H: &str = "Guidance sets the next test";
        const HZ: &str = "业绩指引成为下一道考验";
        let mut p = review_pack();
        p.evidence.push(Evidence {
            id: "e11".into(),
            kind: EvidenceKind::ReportDate,
            text: "Expected report date, not confirmed by the company: November 20, 2026.".into(),
            outlet: None,
            url: None,
            date: Some(d("2026-11-20")),
            quote: None,
            unconfirmed: true,
        });
        let en_cases: &[(bool, &str, &str, &[&str])] = &[
            // M1: a rating after "upgraded/downgraded … to".
            (true, H, "CNBC reports Morgan Stanley downgraded Apple to sell, while Reuters reports revenue of $4.2 billion before November 19.", &["e2", "e3", "e5"]),
            (true, H, "CNBC reports Morgan Stanley upgraded Apple to buy, while Reuters reports revenue of $4.2 billion before November 19.", &["e2", "e3", "e5"]),
            (true, H, "Reuters reports Apple expects revenue of $4.2 billion, and the stock was downgraded to sell before November 19.", &["e2", "e5"]),
            // M1: any buy or sell beside a cited analyst action.
            (true, H, "CNBC reports Morgan Stanley turned to Apple as Reuters reports it will sell more phones before November 19.", &["e2", "e3", "e5"]),
            // M2: advice the deal words used to hide, and buy as a noun.
            (true, H, "Reuters reports Apple expects revenue of $4.2 billion, and investors would buy the shares before its November 19 report.", &["e2", "e5"]),
            (true, H, "Reuters reports Apple expects revenue of $4.2 billion, which investors who plan to buy the stock can test on November 19.", &["e2", "e5"]),
            (true, H, "Reuters reports the deal and revenue of $4.2 billion create a buying opportunity before November 19.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, which makes the stock a compelling buy before November 19.", &["e2", "e5"]),
            (true, H, "Reuters reports revenue of $4.2 billion, an attractive buy before the November 19 report.", &["e2", "e5"]),
            // Deal senses still pass.
            (false, H, "Reuters reports Apple agreed to buy a chip designer before its November 19 report.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple will buy back $1.0 billion of stock before its November 19 report.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple sells iPhones in Texas, with revenue of $4.2 billion before November 19.", &["e2", "e5"]),
            (false, H, "Reuters reports Apple signed a deal to sell its Texas factory before its November 19 report.", &["e2", "e5"]),
            // L1: a bare day mid-clause is refused; a count is not a day.
            (true, H, "Reuters reports revenue of $4.2 billion before results on the 20th ahead of the CPI.", &["e2"]),
            (false, H, "Reuters reports revenue of $4.2 billion for the 10th straight quarter before November 19.", &["e2", "e5"]),
            (false, H, "Reuters reports revenue of $4.2 billion, above $4 billion for the 3rd time, before November 19.", &["e2", "e5"]),
            // L2: a hedge right after the date's comma counts.
            (false, H, "Reuters reports revenue of $4.2 billion, and Apple reports on November 20, an expected date.", &["e2", "e11"]),
            // L4: the hedge must govern the date, not something earlier.
            (true, H, "Reuters reports Apple's expected revenue of $4.2 billion will be tested at its November 20 report.", &["e2", "e11"]),
            (false, H, "Reuters reports revenue of $4.2 billion, and Apple is expected to report results on November 20.", &["e2", "e11"]),
            (false, H, "Reuters reports revenue of $4.2 billion before a report scheduled for November 20.", &["e2", "e11"]),
        ];
        let zh_cases: &[(bool, &str, &str, &[&str])] = &[
            // L2: everyday 一个 and a quarter's name in a hook are fine.
            (
                false,
                "苹果迎来一个关键节点",
                "据路透社报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
                &["e2", "e5"],
            ),
            (
                false,
                "第三季度成为关键",
                "据路透社报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
                &["e2", "e5"],
            ),
            (
                true,
                "苹果三大看点",
                "据路透社报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。",
                &["e2", "e5"],
            ),
            // L2: 合并 does not split the hedge from its date (since trial
            // run 2 the hedge is 暂定, not the company's own 预计).
            (
                false,
                HZ,
                "据路透社报道，苹果营收42亿美元，财报暂定合并后于11月20日公布。",
                &["e2", "e11"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果营收42亿美元，苹果预计合并后于11月20日公布财报。",
                &["e2", "e11"],
            ),
            // L4: 预计 about the revenue does not hedge the date.
            (
                true,
                HZ,
                "据路透社报道，预计营收42亿美元的苹果将于11月20日公布财报。",
                &["e2", "e11"],
            ),
            (
                true,
                HZ,
                "据路透社报道，苹果预计营收42亿美元，并将于11月20日公布财报。",
                &["e2", "e11"],
            ),
            (
                false,
                HZ,
                "据路透社报道，苹果营收42亿美元，财报暂定于11月20日公布。",
                &["e2", "e11"],
            ),
            (
                false,
                HZ,
                "据路透社报道，苹果营收42亿美元，财报于11月20日公布（未确认）。",
                &["e2", "e11"],
            ),
        ];
        let mut wrong = vec![];
        for (refuse, hook, watch, cites) in en_cases {
            let f = check(&p, &insight(line(hook, watch, cites), zh_ok()));
            if f.is_empty() == *refuse {
                wrong.push(format!("{watch} -> {f:?}"));
            }
        }
        for (refuse, hook, watch, cites) in zh_cases {
            let f = check(&p, &insight(en_ok(), line(hook, watch, cites)));
            if f.is_empty() == *refuse {
                wrong.push(format!("{hook} {watch} -> {f:?}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// A pack as trial run 1 stored it (2026-11), outlets named by the
    /// current `outlet_name` from each item's host, and no macro events.
    fn run_one_pack(
        ticker: &str,
        date: &str,
        items: &[(EvidenceKind, Option<&str>, &str, Option<&str>)],
    ) -> Pack {
        let mut evidence = vec![ev(
            "e1",
            EvidenceKind::Move,
            &format!("{ticker} moved in premarket trading on {date}."),
            None,
            Some(date),
        )];
        for (k, (kind, host, text, day)) in items.iter().enumerate() {
            let outlet =
                host.and_then(|h| crate::insight::pack::outlet_name(&format!("https://{h}/a")));
            let mut e = ev(&format!("e{}", k + 2), *kind, text, None, *day);
            e.outlet = outlet;
            evidence.push(e);
        }
        Pack {
            ticker: ticker.into(),
            name: ticker.into(),
            evidence,
        }
    }

    /// Rick, 2026-11-17 ("Allow it if attributed"): a claim about a named
    /// person, an allegation included, passes when the line names the outlet
    /// that reported it in an attributing frame, and fails without one.
    #[test]
    fn a_claim_about_a_person_passes_only_when_attributed() {
        use EvidenceKind::*;
        let p = run_one_pack("NVDA", "2026-11-12", &[
            (News, Some("benzinga.com"), "Reports said the chief executive had an extramarital affair and the board may weigh his removal before the November 19, 2026 report.", None),
            (ReportDate, None, "NVDA's report date is confirmed for November 19, 2026 (earnings calendar).", Some("2026-11-19")),
        ]);
        let attributed = insight(
            line(
                "Board scrutiny follows the report",
                "Benzinga reports the chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question.",
                &["e2", "e3"],
            ),
            line(
                "董事会审视成焦点",
                "据Benzinga报道，首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。",
                &["e2", "e3"],
            ),
        );
        let f = check(&p, &attributed);
        assert!(f.is_empty(), "{f:?}");
        let mut bare = attributed.clone();
        bare.en.watch = "The chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question.".into();
        bare.zh.watch = "首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。".into();
        let f = check(&p, &bare);
        assert!(refused(&f, "en:"), "{f:?}");
        assert!(refused(&f, "zh:"), "{f:?}");
        assert!(f.iter().all(|r| !r.contains("personal")), "{f:?}");
    }

    /// Review M2, the reviewer's probe: what code catches of Rick's people
    /// rule ("Allow it if attributed"), and what only the judge's
    /// `personal_claims_attributed` flag can.
    #[test]
    fn private_life_claims_pass_code_only_when_attributed_to_their_reporter() {
        use EvidenceKind::*;
        let p = run_one_pack("NVDA", "2026-11-12", &[
            (News, Some("benzinga.com"), "Reports said the chief executive had an extramarital affair and the board may weigh his removal before the November 19, 2026 report.", None),
            (ReportDate, None, "NVDA's report date is confirmed for November 19, 2026 (earnings calendar).", Some("2026-11-19")),
        ]);
        let case = |en: (&str, &str), zh: (&str, &str), cites: &[&str]| {
            check(
                &p,
                &insight(line(en.0, en.1, cites), line(zh.0, zh.1, cites)),
            )
        };
        // Baseline: attributed to the outlet whose item reports it.
        let f = case(
            ("Board scrutiny follows the report", "Benzinga reports the chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question."),
            ("董事会审视成焦点", "据Benzinga报道，首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。"),
            &["e2", "e3"],
        );
        assert!(f.is_empty(), "{f:?}");
        // P1: the outlet frames one clause and the claim sits in the next.
        // Code cannot tell the clauses' claims apart: it passes code, and
        // only the judge's flag catches it.
        let f = case(
            ("Board scrutiny follows the report", "Benzinga reports the board may weigh a removal, and the chief executive had an extramarital affair before the November 19 report."),
            ("董事会审视成焦点", "据Benzinga报道，董事会或考虑撤换，而首席执行官确有婚外情，11月19日财报是关键。"),
            &["e2", "e3"],
        );
        assert!(f.is_empty(), "{f:?}");
        // P2: the claim as PreBell's own in the hook.
        let f = case(
            ("CEO affair puts the board on notice", "Benzinga reports the chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question."),
            ("CEO婚外情令董事会承压", "据Benzinga报道，首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。"),
            &["e2", "e3"],
        );
        assert!(
            refused(
                &f,
                "zh: a claim about a person's private life in the hook (婚外情)"
            ),
            "{f:?}"
        );
        // Since review R3 the English hook is refused on its own too.
        assert!(
            refused(
                &f,
                "en: a claim about a person's private life in the hook (CEO affair)"
            ),
            "{f:?}"
        );
        // P3: cites only the calendar item; nothing reports the claim.
        let f = case(
            ("Board scrutiny before the report", "The chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question."),
            ("董事会审视成焦点", "首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。"),
            &["e3"],
        );
        assert!(
            refused(
                &f,
                "en: a claim about a person's private life (extramarital) not attributed"
            ),
            "{f:?}"
        );
        assert!(
            refused(
                &f,
                "zh: a claim about a person's private life (婚外情) not attributed"
            ),
            "{f:?}"
        );
        // P4: attributed to an outlet no cited item is from.
        let f = case(
            ("Board scrutiny before the report", "The Daily Mail reports the chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question."),
            ("董事会审视成焦点", "据每日邮报报道，首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。"),
            &["e3"],
        );
        assert!(
            refused(&f, "en: a claim about a person's private life"),
            "{f:?}"
        );
        assert!(
            refused(&f, "zh: a claim about a person's private life"),
            "{f:?}"
        );
        // P6: vague sourcing.
        let f = case(
            ("Board scrutiny before the report", "People familiar say the chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question."),
            ("董事会审视成焦点", "知情人士称首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。"),
            &["e3"],
        );
        assert!(
            refused(&f, "en: a claim about a person's private life"),
            "{f:?}"
        );
        assert!(
            refused(&f, "zh: a claim about a person's private life"),
            "{f:?}"
        );
        // P5 and P7 were refused before and still are.
        let f = case(
            ("Board scrutiny before the report", "Reuters reports the chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question."),
            ("董事会审视成焦点", "据路透社报道，首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。"),
            &["e3"],
        );
        assert!(!f.is_empty());
        let f = case(
            ("Benzinga reports board scrutiny", "The chief executive had an extramarital affair, so whether the board acts before the November 19 report is the open question."),
            ("据Benzinga报道董事会审视", "首席执行官有婚外情，董事会是否在11月19日财报前采取行动是关键。"),
            &["e2", "e3"],
        );
        assert!(!f.is_empty());
        // "affair" in its other senses is no claim about a person.
        assert!(
            personal_words("a muted affair before the report, as current affairs go").is_empty()
        );
    }

    /// Review R3: the wider wording in both languages is refused when no
    /// cited item reports it, and the business sense of "affairs" and 事务
    /// is not a private-life word.
    #[test]
    fn wider_private_life_wording_is_caught_and_the_business_sense_is_not() {
        use EvidenceKind::*;
        let p = run_one_pack(
            "NVDA",
            "2026-11-12",
            &[(
                ReportDate,
                None,
                "NVDA's report date is confirmed for November 19, 2026 (earnings calendar).",
                Some("2026-11-19"),
            )],
        );
        let zh_ok = "董事会是否在11月19日财报前采取行动是关键。";
        let en_ok = "Whether the board acts before the November 19 report is the open question.";
        for (en, word) in [
            ("The CEO affair has the board's attention, so whether the board acts before the November 19 report is the open question.", "CEO affair"),
            ("The chief executive admitted his affair, so whether the board acts before the November 19 report is the open question.", "his affair"),
            ("The CEO's affair has the board's attention, so whether the board acts before the November 19 report is the open question.", "CEO's affair"),
            ("The chief executive’s affair has the board's attention, so whether the board acts before the November 19 report is the open question.", "executive’s affair"),
            ("Reports of infidelity have the board's attention, so whether the board acts before the November 19 report is the open question.", "infidelity"),
        ] {
            let f = check(&p, &insight(line("Board scrutiny before the report", en, &["e2"]), line("董事会审视成焦点", zh_ok, &["e2"])));
            assert!(refused(&f, &format!("en: a claim about a person's private life ({word})")), "{en}: {f:?}");
        }
        for (zh, word) in [
            (
                "首席执行官承认婚外恋，董事会是否在11月19日财报前采取行动是关键。",
                "婚外恋",
            ),
            (
                "首席执行官被指有不正当关系，董事会是否在11月19日财报前采取行动是关键。",
                "不正当关系",
            ),
        ] {
            let f = check(
                &p,
                &insight(
                    line("Board scrutiny before the report", en_ok, &["e2"]),
                    line("董事会审视成焦点", zh, &["e2"]),
                ),
            );
            assert!(
                refused(
                    &f,
                    &format!("zh: a claim about a person's private life ({word})")
                ),
                "{zh}: {f:?}"
            );
        }
        for text in [
            "a state of affairs the board must address",
            "its corporate affairs team and public affairs office",
            "government affairs spending and current affairs",
            "the affairs of the company are in order",
            "a muted affair before the report",
            "公司事务部门负责政府事务",
        ] {
            assert!(personal_words(text).is_empty(), "{text}");
        }
    }

    /// Trial run 1 (2026-11-13): lines refused for flaws in the rules, as the
    /// writer wrote them (the macro-event clauses, whose items are gone, cut).
    #[test]
    fn the_trial_run_one_lines() {
        use EvidenceKind::*;
        let check_both = |p: &Pack, en: (&str, &str), zh: (&str, &str), cites: &[&str]| {
            check(
                p,
                &insight(line(en.0, en.1, cites), line(zh.0, zh.1, cites)),
            )
        };
        // Outlets outside the 25: a brand, a host, a host without its domain.
        let skyd = run_one_pack("SKYD", "2026-11-10", &[
            (News, Some("247wallst.com"), "CFO Dennis Cinelli said that will create a $200 million drag to Q4 2026 adjusted earnings before interest, taxes, depreciation and amortization", None),
            (ReportDate, None, "Skydance reported results on November 9, 2026 (earnings calendar).", Some("2026-11-09")),
        ]);
        assert_eq!(skyd.evidence[1].outlet.as_deref(), Some("24/7 Wall St."));
        let f = check_both(&skyd,
            ("Guided EBITDA drag in focus", "247wallst.com reports CFO Dennis Cinelli said a $200 million drag will hit Q4 2026 adjusted EBITDA, so the size of that hit in the quarter's reported results is what to track."),
            ("EBITDA拖累成为焦点", "据247wallst.com报道，首席财务官Dennis Cinelli称将对2026年第四季度调整后EBITDA造成2亿美元拖累，该影响在季度业绩中的体现是跟踪重点。"),
            &["e2"]);
        assert!(f.is_empty(), "{f:?}");
        let tpl = run_one_pack("TPL", "2026-11-05", &[
            (News, Some("stocktitan.net"), "The Company reported net income of $167.4 million for the third quarter of 2026 compared to net income of $153.9 million for the second quarter of 2026.", None),
        ]);
        let f = check_both(&tpl,
            ("Sequential growth in revenue and income", "Stocktitan.net reports third-quarter 2026 net income of $167.4 million versus $153.9 million in the second quarter, so whether that growth holds is the point to track."),
            ("营收与利润环比增长", "据stocktitan.net报道，2026年第三季度净利润为1.674亿美元，第二季度为1.539亿美元，该增长能否延续是跟踪重点。"),
            &["e2"]);
        assert!(f.is_empty(), "{f:?}");
        let abnb = run_one_pack("ABNB", "2026-11-06", &[
            (News, Some("seekingalpha.com"), "beat on both the top- and bottom-line with 19% revenue growth and a profit of $3.16 per share coming in better than Wall Street expected and encouraging an upbeat outlook for the remainder of the year", None),
        ]);
        let f = check_both(&abnb,
            ("Earnings beat lifts Airbnb premarket", "Seeking Alpha reports an upbeat outlook for the remainder of the year, so whether that outlook holds at the next report is the point to track."),
            ("财报超预期带动爱彼迎盘前上扬", "据seekingalpha.com报道，公司对今年剩余时间给出乐观展望，该展望能否兑现是跟踪重点。"),
            &["e2"]);
        assert!(f.is_empty(), "{f:?}");
        let nbis = run_one_pack("NBIS", "2026-11-11", &[
            (News, Some("benzinga.com"), "The rally followed strong results from rival AI infrastructure provider CoreWeave Inc.", None),
        ]);
        let f = check_both(&nbis,
            ("Nebius follows CoreWeave's results higher", "Benzinga.com reports the rally followed strong results from rival CoreWeave, so whether Nebius's own results support that read-across is the point to track."),
            ("Nebius随CoreWeave业绩走高", "据Benzinga报道，涨势源于竞争对手CoreWeave的强劲业绩，Nebius自身业绩能否印证这一联动是跟踪重点。"),
            &["e2"]);
        assert!(f.is_empty(), "{f:?}");
        // A digit in a hook that is part of a cited name.
        let ttwo = run_one_pack("TTWO", "2026-11-10", &[
            (News, Some("tradingview.com"), "Take-Two CEO Strauss Zelnick said GTA 6 preorders have significantly exceeded initial expectations.", None),
            (ReportDate, None, "Take-Two Interactive reported results on November 9, 2026 (earnings calendar).", Some("2026-11-09")),
        ]);
        let f = check_both(&ttwo,
            ("GTA 6 preorder strength in focus", "tradingview.com reports CEO Strauss Zelnick said GTA 6 preorders have significantly exceeded initial expectations, making further preorder commentary after results on November 9 the item to track."),
            ("GTA 6预购表现受关注", "据tradingview.com报道，CEO Strauss Zelnick称GTA 6预购大幅超出最初预期，11月9日财报之后的预购相关表态是后续跟踪重点。"),
            &["e2", "e3"]);
        assert!(f.is_empty(), "{f:?}");
        let f = check_both(&ttwo,
            ("GTA 7 preorder strength in focus", "tradingview.com reports CEO Strauss Zelnick said GTA 6 preorders have significantly exceeded initial expectations."),
            ("GTA 6预购表现受关注", "据tradingview.com报道，CEO Strauss Zelnick称GTA 6预购大幅超出最初预期。"),
            &["e2"]);
        assert!(refused(&f, "en: hook has a digit"), "{f:?}");
        // An aggregator is no outlet: a line may not rest a fact on it.
        let axon = run_one_pack(
            "AXON",
            "2026-11-06",
            &[(
                News,
                Some("google.com"),
                "lowering its 2026 revenue guidance amid drone deployment delays",
                None,
            )],
        );
        assert_eq!(axon.evidence[1].outlet, None);
        let f = check_both(&axon,
            ("Guidance cut weighs on Axon", "Google reports Axon lowered its 2026 revenue guidance amid drone deployment delays, so progress on those deployments is the item to track."),
            ("下调指引拖累Axon", "据google.com报道，Axon因无人机部署延迟下调了2026年营收指引，这些部署的进展值得跟踪。"),
            &["e2"]);
        assert!(refused(&f, "en: cites a news item with no outlet"), "{f:?}");
        assert!(refused(&f, "zh: cites a news item with no outlet"), "{f:?}");
        // Chinese figures in 万 and 亿, never 百万.
        let rklb = run_one_pack(
            "RKLB",
            "2026-11-10",
            &[
                (
                    News,
                    Some("stocktitan.net"),
                    "Net loss widened to $79.677 million from $18.257 million a year earlier.",
                    None,
                ),
                (
                    News,
                    Some("stocktitan.net"),
                    "Rocket Lab targets Neutron's first launch for no earlier than 2027.",
                    None,
                ),
            ],
        );
        let en_rklb = ("Wider loss, Neutron timing in focus", "Stock Titan reports Rocket Lab targets Neutron's first launch for no earlier than 2027, so any change to that timing matters alongside a net loss that widened to $79.677 million from $18.257 million.");
        let f = check_both(&rklb, en_rklb,
            ("亏损扩大，关注Neutron进度", "据stocktitan.net报道，Rocket Lab目标最早于2027年进行Neutron首飞，净亏损从18.257百万美元扩大至79.677百万美元，该时间表若有变化值得跟踪。"),
            &["e2", "e3"]);
        assert!(refused(&f, "zh: a figure in 百万"), "{f:?}");
        let f = check_both(&rklb, en_rklb,
            ("亏损扩大，关注Neutron进度", "据Stock Titan报道，Rocket Lab目标最早于2027年进行Neutron首飞，净亏损从1825.7万美元扩大至7967.7万美元，该时间表若有变化值得跟踪。"),
            &["e2", "e3"]);
        assert!(f.is_empty(), "{f:?}");
        // Relative time: a plain 接下来 and a 今年 that mirrors "the remainder
        // of the year" pass; with a duration, or later this year, they do not.
        assert_eq!(relative_time("据seekingalpha.com报道，NRG同意以40亿美元向LS Power出售六座天然气发电资产，接下来可关注11月9日的财报。"), None);
        assert_eq!(relative_time("公司对今年剩余时间给出乐观展望"), None);
        for w in [
            "接下来几周",
            "接下来数月",
            "今年晚些时候",
            "今年底",
            "今年内",
        ] {
            assert!(
                relative_time(&format!("苹果营收42亿美元，{w}值得关注。")).is_some(),
                "{w}"
            );
        }
    }

    /// Trial run 2: a year a cited item names may stand in a hook (ANET);
    /// a claim about a person is no longer refused by a word list (NVDA,
    /// Rick 2026-11-17); in Chinese an
    /// unconfirmed date may not be the company's own 预计 (NVDA).
    #[test]
    fn the_trial_run_two_lines() {
        let records: Vec<serde_json::Value> = include_str!("testdata/run2_records.jsonl")
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let get = |post: &str, ticker: &str| -> (Pack, Insight) {
            let r = records
                .iter()
                .find(|r| r["post"] == post && r["ticker"] == ticker)
                .unwrap();
            (
                serde_json::from_value(r["pack"].clone()).unwrap(),
                serde_json::from_value(r["insight"].clone()).unwrap(),
            )
        };
        let (p, i) = get("recap_2026-11-12", "ANET");
        assert_eq!(i.en.hook, "Guidance raised, 2027 growth view given");
        let f = check(&p, &i);
        assert!(f.is_empty(), "{f:?}");
        let mut other = i.clone();
        other.en.hook = "Guidance raised, 2028 growth view given".into();
        assert!(refused(&check(&p, &other), "en: hook has a digit"));

        let (p, i) = get("premarket_2026-11-12-update", "NVDA");
        let f = check(&p, &i);
        // Rick, 2026-11-17: no word list refuses a claim about a person.
        assert!(!refused(&f, "personal"), "{f:?}");
        assert!(
            refused(
                &f,
                "zh: an unconfirmed report date written as the company's own expectation"
            ),
            "{f:?}"
        );

        let (p, mut i) = get("recap_2026-11-09", "NVDA");
        assert!(i.zh.watch.contains("英伟达预计于11月17日发布财报"));
        let f = check(&p, &i);
        assert!(
            refused(
                &f,
                "zh: an unconfirmed report date written as the company's own expectation"
            ),
            "{f:?}"
        );
        i.zh.watch = "据FinanceFeeds报道,董事会已展开独立调查,财报日期暂定为11月17日（未经公司确认），在此之前调查是否出现结论是需要跟踪的要点。".into();
        let f = check(&p, &i);
        assert!(f.is_empty(), "{f:?}");
        i.zh.watch = "据FinanceFeeds报道,董事会已展开独立调查,市场预计其于11月17日发布财报（未经公司确认），调查是否出现结论是需要跟踪的要点。".into();
        let f = check(&p, &i);
        assert!(f.is_empty(), "{f:?}");
        // "6座" for "six" passes the number check.
        let (p, i) = get("premarket_2026-11-09-update", "NRG");
        assert!(i.zh.watch.contains("6座"));
        assert!(check(&p, &i).is_empty());
    }

    /// Trial run 3: TPL's "上一季度" (the prior quarter) is no fiscal label;
    /// NVDA's line fits 40 words with the short English hedge.
    #[test]
    fn the_trial_run_three_lines() {
        let records: Vec<serde_json::Value> = include_str!("testdata/run3_records.jsonl")
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let get = |ticker: &str| -> (Pack, Insight) {
            let r = records.iter().find(|r| r["ticker"] == ticker).unwrap();
            (
                serde_json::from_value(r["pack"].clone()).unwrap(),
                serde_json::from_value(r["insight"].clone()).unwrap(),
            )
        };
        let (p, i) = get("TPL");
        assert!(i.zh.watch.contains("上一季度为2.461亿美元"));
        let f = check(&p, &i);
        assert!(f.is_empty(), "{f:?}");
        for w in ["上一季度", "前一季度", "同一季度", "下一季度", "每一季度"] {
            assert!(fiscal_labels(&format!("营收高于{w}")).is_empty(), "{w}");
        }
        assert_eq!(fiscal_labels("第一季度营收").len(), 1);
        assert_eq!(fiscal_labels("一季度营收").len(), 1);

        let (p, mut i) = get("NVDA");
        assert!(refused(&check(&p, &i), "en: watch line has 45 words"));
        i.en.watch = "TipRanks reports the board opened an independent investigation while saying there is no reason to believe national security or company secrets were compromised, so any update matters before the report expected on November 17 (date not confirmed by the company).".into();
        let f = check(&p, &i);
        assert!(f.is_empty(), "{f:?}");
        assert!(i.en.watch.split_whitespace().count() <= WATCH_WORDS_EN);
    }

    /// Round 4 review inputs (M1, M2, L1, L2, L4, L5, L6).
    #[test]
    fn the_round_four_probe_inputs() {
        use EvidenceKind::*;
        let mk = |host: &str| {
            run_one_pack("AAPL", "2026-11-12", &[
                (News, Some(host), "Apple said it expects revenue of $4.2 billion and will report on November 19, 2026.", None),
                (ReportDate, None, "Apple's report date is confirmed for November 19, 2026 (earnings calendar).", Some("2026-11-19")),
            ])
        };
        let both = |p: &Pack, hook: &str, en: &str, zh: &str, cites: &[&str]| {
            check(
                p,
                &insight(
                    line(hook, en, cites),
                    line("业绩指引成为下一道考验", zh, cites),
                ),
            )
        };
        let zh_ok_for = |name: &str| {
            format!("据{name}报道，苹果预计营收42亿美元，11月19日的财报将检验这一预期。")
        };
        // M1: an ordinary word is not attribution.
        for (host, name, en) in [
            ("nasdaq.com", "Nasdaq", "Apple, listed on the Nasdaq, expects revenue of $4.2 billion before its November 19 report."),
            ("investing.com", "Investing.com", "Apple expects revenue of $4.2 billion, investing heavily before its November 19 report."),
            ("fool.com", "The Motley Fool", "Apple expects revenue of $4.2 billion, and no fool would miss its November 19 report."),
        ] {
            let p = mk(host);
            let f = both(&p, "Guidance sets the next test", en, &zh_ok_for(name), &["e2", "e3"]);
            assert!(refused(&f, "en: does not attribute"), "{host}: {f:?}");
            // In an attributing frame it passes.
            let framed = format!("{name} reports Apple expects revenue of $4.2 billion before its November 19 report.");
            let f = both(&p, "Guidance sets the next test", &framed, &zh_ok_for(name), &["e2", "e3"]);
            assert!(f.is_empty(), "{host}: {f:?}");
        }
        // The host with its domain, never the bare word.
        let p = mk("fool.com");
        assert!(both(
            &p,
            "Guidance sets the next test",
            "Fool.com reports Apple expects revenue of $4.2 billion before its November 19 report.",
            &zh_ok_for("fool.com"),
            &["e2", "e3"]
        )
        .is_empty());
        let f = both(
            &p,
            "Guidance sets the next test",
            "Fool reports Apple expects revenue of $4.2 billion before its November 19 report.",
            &zh_ok_for("fool.com"),
            &["e2", "e3"],
        );
        assert!(refused(&f, "en: does not attribute"), "{f:?}");
        // Other frames: according to, 称, a list.
        let p = mk("benzinga.com");
        assert!(both(&p, "Guidance sets the next test", "According to Benzinga, Apple expects revenue of $4.2 billion before its November 19 report.", "Benzinga称，苹果预计营收42亿美元，11月19日的财报将检验这一预期。", &["e2", "e3"]).is_empty());
        // M2: a mover brand that no cited item is from.
        let r = run_one_pack("AAPL", "2026-11-12", &[
            (News, Some("reuters.com"), "Apple said it expects revenue of $4.2 billion and will report on November 19, 2026.", None),
            (ReportDate, None, "Apple's report date is confirmed for November 19, 2026 (earnings calendar).", Some("2026-11-19")),
        ]);
        let f = both(&r, "Guidance sets the next test", "Benzinga and Reuters report Apple expects revenue of $4.2 billion before its November 19 report.", &zh_ok_for("路透社"), &["e2", "e3"]);
        assert!(
            refused(&f, "en: names Benzinga, which no cited item is from"),
            "{f:?}"
        );
        let f = both(&r, "Guidance sets the next test", "Reuters reports Apple, listed on the Nasdaq, expects revenue of $4.2 billion before its November 19 report.", &zh_ok_for("路透社"), &["e2", "e3"]);
        assert!(f.is_empty(), "{f:?}");
        // L1: a hook figure never passes as a name.
        let q = run_one_pack(
            "AMD",
            "2026-11-06",
            &[(
                News,
                Some("fool.com"),
                "AMD's Q3 revenue rose by 45% year over year to $13.38 billion.",
                None,
            )],
        );
        let f = both(&q, "Q3 revenue rose by 45%", "Fool.com reports AMD's Q3 revenue rose by 45% year over year to $13.38 billion, a pace the next report will test.", "据fool.com报道，AMD第三季度营收同比增长45%，达到133.8亿美元，下次财报将检验这一增速。", &["e2"]);
        assert!(refused(&f, "en: hook has a digit"), "{f:?}");
        let f = both(&q, "Q3 revenue growth in focus", "Fool.com reports AMD's Q3 revenue rose by 45% year over year to $13.38 billion, a pace the next report will test.", "据fool.com报道，AMD第三季度营收同比增长45%，达到133.8亿美元，下次财报将检验这一增速。", &["e2"]);
        assert!(f.is_empty(), "{f:?}");
        // L2: "affair" in any sense is left to attribution and the judge.
        let p = mk("reuters.com");
        let f = both(
            &p,
            "A muted affair before the report",
            "Reuters reports Apple expects revenue of $4.2 billion before its November 19 report.",
            &zh_ok_for("路透社"),
            &["e2", "e3"],
        );
        assert!(f.is_empty(), "{f:?}");
        let f = both(&p, "Guidance sets the next test", "Reuters reports a state of affairs where Apple expects revenue of $4.2 billion before November 19.", &zh_ok_for("路透社"), &["e2", "e3"]);
        assert!(f.is_empty(), "{f:?}");
        // A private-life claim the cited item does not report is refused,
        // attributed or not (review M2): Reuters' item is about guidance.
        let f = both(&p, "Guidance sets the next test", "Reuters reports the CEO had an affair, and Apple expects revenue of $4.2 billion before November 19.", &zh_ok_for("路透社"), &["e2", "e3"]);
        assert!(
            refused(&f, "en: a claim about a person's private life (an affair)"),
            "{f:?}"
        );
        // L4: past periods pass in both languages; forward ones do not.
        assert_eq!(relative_time("revenue rose from last quarter"), None);
        assert_eq!(
            relative_time("revenue rose from the prior quarter and a year earlier"),
            None
        );
        assert_eq!(
            relative_time("营收高于上一季度和上季度，也高于去年同期和去年"),
            None
        );
        assert!(relative_time("revenue will rise next quarter").is_some());
        assert!(relative_time("下季度营收").is_some());
        // L5: "target to $250" is refused by the word rule.
        assert!(
            refused_wording("Morgan Stanley raised its target to $250", false)
                .iter()
                .any(|w| w.contains("$250"))
        );
        // L6: a quarter without 第 in a Chinese hook is a fiscal label.
        let t = run_one_pack(
            "AMD",
            "2026-11-06",
            &[(
                News,
                Some("fool.com"),
                "AMD's third quarter revenue rose by 45% year over year to $13.38 billion.",
                None,
            )],
        );
        let f = both(&t, "Third-quarter revenue in focus", "Fool.com reports AMD's third quarter revenue rose by 45% year over year to $13.38 billion, a pace the next report will test.", "据fool.com报道，AMD第三季度营收同比增长45%，达到133.8亿美元，下次财报将检验这一增速。", &["e2"]);
        assert!(f.is_empty(), "{f:?}");
        let mut i = insight(line("Third-quarter revenue in focus", "Fool.com reports AMD's third quarter revenue rose by 45% year over year to $13.38 billion, a pace the next report will test.", &["e2"]), line("三季度营收成焦点", "据fool.com报道，AMD第三季度营收同比增长45%，达到133.8亿美元，下次财报将检验这一增速。", &["e2"]));
        i.ticker = "AMD".into();
        let f = check(&t, &i);
        assert!(f.is_empty(), "{f:?}");
        i.zh.hook = "四季度营收成焦点".into();
        let f = check(&t, &i);
        assert!(refused(&f, "zh: fiscal label 四季度"), "{f:?}");
    }

    /// Every text the trial stored (lines in both languages, evidence,
    /// mover reasons), plus the texts of `INSIGHT_EQUIV_CORPUS` (one per
    /// line) when it is set.
    fn stored_texts() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut take = |v: &serde_json::Value| {
            for i in ["insight", "variant"] {
                for lang in ["en", "zh"] {
                    for k in ["hook", "watch"] {
                        if let Some(t) = v[i][lang][k].as_str() {
                            out.push(t.to_string());
                        }
                    }
                }
            }
            for e in v["pack"]["evidence"].as_array().into_iter().flatten() {
                for k in ["text", "quote"] {
                    if let Some(t) = e[k].as_str() {
                        out.push(t.to_string());
                    }
                }
            }
        };
        for l in include_str!("testdata/run2_records.jsonl")
            .lines()
            .chain(include_str!("testdata/run3_records.jsonl").lines())
            .chain(
                include_str!("../../../../tools/insight-trial/testdata/final_run_lines.jsonl")
                    .lines(),
            )
        {
            take(&serde_json::from_str(l).unwrap());
        }
        let rec: serde_json::Value =
            serde_json::from_str(include_str!("testdata/shadow/record.json")).unwrap();
        for m in rec["movers"].as_array().unwrap() {
            take(m);
        }
        for l in include_str!("testdata/reasons_2026_11.tsv").lines().skip(1) {
            out.push(l.splitn(3, '\t').nth(2).unwrap_or("").to_string());
        }
        if let Ok(path) = std::env::var("INSIGHT_EQUIV_CORPUS") {
            out.extend(
                std::fs::read_to_string(path)
                    .unwrap()
                    .lines()
                    .map(String::from),
            );
        }
        out
    }

    /// The CPU ruling's condition for a cheaper pattern: on every stored
    /// text, and every lowercase prefix of one (`rating_frame` reads the text
    /// before a word), the new pattern finds exactly what the old one did.
    #[test]
    fn wide_words_match_as_before() {
        let mut pairs: Vec<String> = REFUSED_EN.iter().map(|p| format!("(?i){p}")).collect();
        pairs.extend([WORDED_EN, ADVICE_HOLD, RELATIVE_EN, RATING_FRAME].map(String::from));
        let pairs: Vec<(Regex, Regex)> = pairs
            .iter()
            .filter(|p| p.contains(r"\w"))
            .map(|p| (Regex::new(p).unwrap(), Regex::new(&wide_words(p)).unwrap()))
            .collect();
        assert_eq!(pairs.len(), 8);
        let texts = stored_texts();
        assert!(texts.len() > 250, "{}", texts.len());
        let spans = |r: &Regex, t: &str| -> Vec<(usize, usize)> {
            r.find_iter(t).map(|m| (m.start(), m.end())).collect()
        };
        let mut compared = 0;
        for t in &texts {
            let lower = t.to_ascii_lowercase();
            let prefixes: Vec<&str> = std::iter::once(t.as_str())
                .chain(lower.match_indices(' ').map(|(at, _)| &lower[..at + 1]))
                .collect();
            for p in prefixes {
                for (old, new) in &pairs {
                    assert_eq!(spans(old, p), spans(new, p), "{old} on {p:?}");
                    compared += 1;
                }
            }
        }
        assert!(compared > 10_000, "{compared}");
        // The rewrite itself: a class of its own outside brackets, ranges
        // inside them.
        assert_eq!(
            wide_words(r"a\w+b"),
            r"a(?-i:[0-9A-Za-z_\x{80}-\x{10FFFF}])+b"
        );
        assert_eq!(wide_words(r"[\w.]\s"), r"[0-9A-Za-z_\x{80}-\x{10FFFF}.]\s");
        // And it still matches where `\w` does, beyond ASCII.
        let old = Regex::new(&format!("(?i){}", REFUSED_EN[20])).unwrap();
        let new = Regex::new(&wide_words(&format!("(?i){}", REFUSED_EN[20]))).unwrap();
        let t = "analysts at Société Générale expect";
        assert!(old.is_match(t) && new.is_match(t));
    }
}
