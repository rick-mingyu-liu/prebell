//! One language's IPO write-up, written entirely by code from a milestone's
//! facts bundle (`milestone::IpoArticleFacts`). There is no model call: the
//! title, the summary and both prose sections are sentence templates per
//! milestone and language, filled only from the filing facts, the previous
//! range, the first session and code's derived offering size (the lead's
//! ruling after review, 2026-11-11, pending Rick).
//!
//! Every statement is therefore one the stored data proves: a figure with
//! its own kind's words (the selling stockholders' shares only as theirs,
//! the option only as the underwriters'), a price placed against the last
//! range only as within, above or below it, and the offering size only with
//! both inputs and what it is. Reported news reaches the page only through
//! the "What has been reported" list, word for word and attributed.
//!
//! `verify_page` is a cheap guard on the result: every number and date in
//! the page outside the quoted news is one the bundle holds.

use std::collections::HashSet;
use std::sync::OnceLock;

use chrono::{Datelike, NaiveDate};
use regex::Regex;

use super::milestone::{IpoArticleFacts, Milestone, Previous, QuotedItem};
use super::FORMS;
use crate::numbers::{check_allowed, Allowed, Lang};
use crate::premarket::draft::{without_link_targets, Draft};

pub const CONTENT_TYPE: &str = "ipo";
pub const DISCLAIMER_EN: &str = "This IPO write-up is generated automatically from SEC filings and public news reports, for information only. It is not investment advice.";
pub const DISCLAIMER_ZH: &str =
    "本文根据美国证监会文件和公开新闻报道自动生成，仅供参考，不构成投资建议。";

/// What the derived offering size is, under the table wherever it appears.
pub const OFFERING_SIZE_EN: &str = "Offering size is computed by PreBell from the cover: shares offered (including shares sold by selling stockholders, excluding the underwriters' option) × the price per share, gross, before underwriting discounts. It is not the company's proceeds and not a valuation.";
pub const OFFERING_SIZE_ZH: &str = "发行规模由PreBell根据招股文件封面计算：发行股数（含售股股东发售的股份，不含承销商超额配售权）×每股价格，为扣除承销折扣前的总额，并非公司所得款项，也不是估值。";

/// `posts.template_id`: `ipo-<milestone>`.
pub fn template_id(m: Milestone) -> String {
    format!("ipo-{}", m.as_str())
}

/// The two prose sections, in order.
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

/// A per-unit price: "$70.00" / "70.00美元".
fn price(v: f64, lang: Lang) -> String {
    match lang {
        Lang::En => format!("${v:.2}"),
        Lang::Zh => format!("{v:.2}美元"),
    }
}

/// A price without its currency word: "70.00" (the Chinese range's low end).
fn bare(v: f64) -> String {
    format!("{v:.2}")
}

/// A headline price: "70", or "70.50" when it has cents.
fn price_short(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// A range: "$70.00–$78.00" / "70.00–78.00美元".
fn range(lo: f64, hi: f64, lang: Lang) -> String {
    match lang {
        Lang::En => format!("${lo:.2}–${hi:.2}"),
        Lang::Zh => format!("{lo:.2}–{hi:.2}美元"),
    }
}

/// Whether `v` in `unit`s has at most `places` decimals.
fn exact(v: f64, unit: f64, places: i32) -> bool {
    let x = v / unit * 10f64.powi(places);
    (x - x.round()).abs() < 1e-6
}

/// The fewest decimals (at most 4) that write `v` in `unit`s exactly.
fn places(v: f64, unit: f64) -> Option<usize> {
    (0..=4).find(|&k| exact(v, unit, k as i32))
}

/// A dollar amount, never rounded, in the largest unit that stays exact
/// within four decimals: "$57.75 billion", "$4.8705 billion",
/// "$1,419,354,816" / "577.5亿美元", "48.705亿美元", "1,419,354,816美元".
fn money(v: f64, lang: Lang) -> String {
    let unit = |u: f64| (v >= u).then(|| places(v, u)).flatten();
    match lang {
        // A smaller unit only below the larger one: never "141935.4816万".
        Lang::En => match (unit(1e9), unit(1e6).filter(|_| v < 1e9)) {
            (Some(k), _) => format!("${} billion", trim_dec(v / 1e9, k)),
            (None, Some(k)) => format!("${} million", trim_dec(v / 1e6, k)),
            _ => format!("${}", grouped(v)),
        },
        Lang::Zh => match (unit(1e8), unit(1e4).filter(|_| v < 1e8)) {
            (Some(k), _) => format!("{}亿美元", trim_dec(v / 1e8, k)),
            (None, Some(k)) => format!("{}万美元", trim_dec(v / 1e4, k)),
            _ => format!("{}美元", grouped(v)),
        },
    }
}

/// A Chinese count: 亿 or 万 only when that takes at most two decimals
/// (825,000,000 is 8.25亿, 22,000,000 is 2200万); otherwise every digit with
/// separators (801,351,354).
fn zh_count(v: f64) -> String {
    if v >= 1e8 && exact(v, 1e8, 2) {
        format!("{}亿", trim_dec(v / 1e8, 2))
    } else if (1e4..1e8).contains(&v) && exact(v, 1e4, 2) {
        format!("{}万", trim_dec(v / 1e4, 2))
    } else {
        grouped(v)
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

/// A URL as a Markdown link target: spaces, parentheses and angle brackets
/// percent-encoded, so the link cannot end early.
fn link_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

fn name_of(f: &IpoArticleFacts, lang: Lang) -> &str {
    match lang {
        Lang::Zh => f.name_zh.as_deref().unwrap_or(&f.name),
        Lang::En => &f.name,
    }
}

/// What is offered, in each language's words.
struct Unit {
    /// "shares" / "ADSs".
    plural: &'static str,
    /// "share" / "ADS".
    one: &'static str,
    /// "a share" / "an ADS".
    a_one: &'static str,
    /// "股" / "份美国存托股".
    zh: &'static str,
    /// "每股" / "每份美国存托股".
    zh_each: &'static str,
}

fn unit(f: &IpoArticleFacts) -> Unit {
    if f.security == "ads" {
        Unit {
            plural: "ADSs",
            one: "ADS",
            a_one: "an ADS",
            zh: "份美国存托股",
            zh_each: "每份美国存托股",
        }
    } else {
        Unit {
            plural: "shares",
            one: "share",
            a_one: "a share",
            zh: "股",
            zh_each: "每股",
        }
    }
}

/// "825,000,000 shares" / "8.25亿股".
fn count(v: f64, f: &IpoArticleFacts, lang: Lang) -> String {
    let u = unit(f);
    match lang {
        Lang::En => format!("{} {}", grouped(v), u.plural),
        Lang::Zh => format!("{}{}", zh_count(v), u.zh),
    }
}

/// The table's count cells: every digit in both languages.
fn shares_cell(v: f64, f: &IpoArticleFacts, lang: Lang) -> String {
    match lang {
        Lang::En => grouped(v),
        Lang::Zh => format!("{}{}", grouped(v), unit(f).zh),
    }
}

/// A form with what it is: "an amended registration statement (Form
/// S-1/A)" / "注册声明修订稿（S-1/A表格）".
fn form_label(form: &str, lang: Lang) -> String {
    let amended = form.ends_with("/A");
    match lang {
        Lang::En if form.starts_with("424") => format!("its final prospectus (Form {form})"),
        Lang::En if form == "RW" => {
            "a request to withdraw its registration statement (Form RW)".into()
        }
        Lang::En if amended => format!("an amended registration statement (Form {form})"),
        Lang::En => format!("a registration statement (Form {form})"),
        Lang::Zh if form.starts_with("424") => format!("最终招股说明书（{form}表格）"),
        Lang::Zh if form == "RW" => "撤回注册声明的申请（RW表格）".into(),
        Lang::Zh if amended => format!("注册声明修订稿（{form}表格）"),
        Lang::Zh => format!("注册声明（{form}表格）"),
    }
}

/// The cover's word for the other sellers: a foreign issuer (an F-1 in its
/// registration history, as Arm and Birkenstock) says shareholders.
fn sellers(f: &IpoArticleFacts) -> &'static str {
    let foreign = f
        .timeline
        .iter()
        .map(|t| t.form.as_str())
        .chain(f.filing.as_ref().map(|x| x.form.as_str()))
        .any(|form| form.starts_with("F-"));
    if foreign {
        "selling shareholders"
    } else {
        "selling stockholders"
    }
}

/// The exchange as written in each language: the standard Chinese name for
/// the strings the cover patterns produce, the stored English otherwise.
fn exchange_name(x: &str, lang: Lang) -> String {
    let zh = match x {
        "Nasdaq Global Select Market" => Some("纳斯达克全球精选市场"),
        "Nasdaq Global Market" => Some("纳斯达克全球市场"),
        "Nasdaq Capital Market" => Some("纳斯达克资本市场"),
        "Nasdaq Stock Market" => Some("纳斯达克股票市场"),
        "New York Stock Exchange" | "NYSE" => Some("纽约证券交易所"),
        "NYSE American" => Some("纽约证券交易所美国市场"),
        _ => None,
    };
    match (lang, zh) {
        (Lang::Zh, Some(z)) => z.to_string(),
        _ => x.to_string(),
    }
}

/// A Latin-script name before Latin text in Chinese gets a space
/// ("Anthropic IPO文件").
fn zh_join(name: &str, rest: &str) -> String {
    let latin_end = name
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let latin_start = rest
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    if latin_end && latin_start {
        format!("{name} {rest}")
    } else {
        format!("{name}{rest}")
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
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

// ---------------------------------------------------------------- templates

/// Sentences in one language, joined as a paragraph.
fn para(lang: Lang, s: Vec<String>) -> String {
    s.into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(pick(lang, " ", ""))
}

/// Who sells, as the cover splits it.
fn split_sentence(f: &IpoArticleFacts, lang: Lang) -> String {
    let n = name_of(f, lang);
    let total = f.num("shares_offered");
    match (f.num("shares_company"), f.num("shares_selling"), lang) {
        (Some(c), Some(s), Lang::En) if s > 0.0 => format!(
            "{n} is selling {} and the {} are selling {}.",
            count(c, f, lang),
            sellers(f),
            count(s, f, lang)
        ),
        (Some(c), Some(s), Lang::Zh) if s > 0.0 => format!(
            "公司发售{}，售股股东发售{}。",
            count(c, f, lang),
            count(s, f, lang)
        ),
        (Some(c), _, Lang::En) if Some(c) == total => {
            format!("{n} is selling all {}.", count(c, f, lang))
        }
        (Some(c), _, Lang::Zh) if Some(c) == total => {
            format!("全部{}均由公司发售。", count(c, f, lang))
        }
        (Some(c), _, Lang::En) => format!("{n} is selling {}.", count(c, f, lang)),
        (Some(c), _, Lang::Zh) => format!("公司发售{}。", count(c, f, lang)),
        (None, Some(s), Lang::En) => {
            format!("{} are selling {}.", capital(sellers(f)), count(s, f, lang))
        }
        (None, Some(s), Lang::Zh) => format!("售股股东发售{}。", count(s, f, lang)),
        _ => String::new(),
    }
}

fn option_sentence(f: &IpoArticleFacts, lang: Lang) -> String {
    match (f.num("option_shares"), lang) {
        (Some(o), Lang::En) => format!(
            "The underwriters have an option to purchase up to {} more {}.",
            grouped(o),
            unit(f).plural
        ),
        (Some(o), Lang::Zh) => format!("承销商有权额外购买至多{}。", count(o, f, lang)),
        (None, _) => String::new(),
    }
}

/// The derived offering size with both inputs and what it is.
fn size_sentences(f: &IpoArticleFacts, lang: Lang) -> Vec<String> {
    if f.derived.is_empty() {
        return vec![];
    }
    let u = unit(f);
    let parts: Vec<String> = f
        .derived
        .iter()
        .map(|d| match lang {
            Lang::En => format!(
                "at {} {}, the {} offered come to {}",
                price(d.price, lang),
                u.a_one,
                count(d.shares, f, lang),
                money(d.value_usd, lang)
            ),
            Lang::Zh => format!(
                "按{}{}计算，{}的发行规模为{}",
                u.zh_each,
                price(d.price, lang),
                count(d.shares, f, lang),
                money(d.value_usd, lang)
            ),
        })
        .collect();
    match lang {
        Lang::En => vec![
            format!("{}.", capital(&parts.join("; "))),
            format!(
                "These are PreBell's gross figures before underwriting discounts, including any {} sold by {} and excluding the underwriters' option; they are not the company's proceeds.",
                u.plural,
                sellers(f)
            ),
        ],
        Lang::Zh => vec![
            format!("{}。", parts.join("；")),
            "以上为PreBell按发行数量乘以价格算出的总额，未扣除承销折扣，含售股股东发售的部分（如有），不含承销商超额配售权，并非公司所得款项。".into(),
        ],
    }
}

fn exchange_sentence(f: &IpoArticleFacts, lang: Lang) -> String {
    match (&f.exchange, &f.symbol, lang) {
        (Some(x), Some(s), Lang::En) => {
            format!("The filing names the {x} as the exchange and {s} as the symbol.")
        }
        (Some(x), Some(s), Lang::Zh) => {
            format!(
                "文件载明的交易所为{}，股票代码为{s}。",
                exchange_name(x, lang)
            )
        }
        (Some(x), None, Lang::En) => format!("The filing names the {x} as the exchange."),
        (Some(x), None, Lang::Zh) => format!("文件载明的交易所为{}。", exchange_name(x, lang)),
        (None, Some(s), Lang::En) => format!("The filing gives {s} as the symbol."),
        (None, Some(s), Lang::Zh) => format!("文件载明的股票代码为{s}。"),
        (None, None, _) => String::new(),
    }
}

/// Who sells, the option, the size and the exchange: the offering terms.
fn terms(f: &IpoArticleFacts, lang: Lang) -> String {
    let mut s = vec![split_sentence(f, lang), option_sentence(f, lang)];
    s.extend(size_sentences(f, lang));
    s.push(exchange_sentence(f, lang));
    para(lang, s)
}

/// "rose from $70.00 to $76.00" / "is unchanged at $70.00", for one end.
fn moved(old: f64, new: f64, lang: Lang) -> String {
    match lang {
        Lang::En if new > old => format!("rose from {} to {}", price(old, lang), price(new, lang)),
        Lang::En if new < old => format!("fell from {} to {}", price(old, lang), price(new, lang)),
        Lang::En => format!("is unchanged at {}", price(new, lang)),
        Lang::Zh if new > old => format!("由{}上调至{}", price(old, lang), price(new, lang)),
        Lang::Zh if new < old => format!("由{}下调至{}", price(old, lang), price(new, lang)),
        Lang::Zh => format!("维持{}不变", price(new, lang)),
    }
}

/// Where a price sits against a range: only what two numbers prove.
fn placement(p: f64, lo: f64, hi: f64) -> (&'static str, &'static str) {
    if p < lo {
        ("below", "低于")
    } else if p > hi {
        ("above", "高于")
    } else {
        ("within", "处于")
    }
}

/// "for 825,000,000 shares" / "，发行8.25亿股".
fn for_total(f: &IpoArticleFacts, lang: Lang) -> String {
    f.num("shares_offered")
        .map_or(String::new(), |t| match lang {
            Lang::En => format!(" for {}", count(t, f, lang)),
            Lang::Zh => format!("，发行{}", count(t, f, lang)),
        })
}

/// "Anthropic filed ... with the SEC on Nov 10, 2026."
fn filed_sentence(n: &str, label: &str, on: &str, lang: Lang) -> String {
    match lang {
        Lang::En => format!("{n} filed {label} with the SEC on {on}."),
        Lang::Zh => format!("{n}于{on}向美国证监会递交{label}。"),
    }
}

/// The range in a sentence: "a price range of $70.00 to $78.00 per share" /
/// "每股70.00至78.00美元".
fn range_words(lo: f64, hi: f64, u: &Unit, lang: Lang) -> String {
    match lang {
        Lang::En => format!(
            "a price range of {} to {} per {}",
            price(lo, lang),
            price(hi, lang),
            u.one
        ),
        Lang::Zh => format!("{}{}至{}", u.zh_each, bare(lo), price(hi, lang)),
    }
}

struct Text {
    title: String,
    summary: String,
    first: String,
    second: String,
}

fn filed(f: &IpoArticleFacts, lang: Lang) -> Text {
    let n = name_of(f, lang);
    let (form, date) = filing_parts(f);
    let on = day(&date, lang);
    let label = form_label(&form, lang);
    let drs: Vec<&str> = f
        .timeline
        .iter()
        .filter(|t| t.form.starts_with("DRS") && t.filing_date < date)
        .map(|t| t.filing_date.as_str())
        .collect();
    // "DRS/A" only when an amended draft is on file.
    let amended = f
        .timeline
        .iter()
        .any(|t| t.form == "DRS/A" && t.filing_date < date);
    let forms = match (amended, lang) {
        (true, Lang::En) => "Form DRS and DRS/A",
        (false, Lang::En) => "Form DRS",
        (true, Lang::Zh) => "DRS及DRS/A表格",
        (false, Lang::Zh) => "DRS表格",
    };
    let follows = match (drs.iter().min(), drs.len(), lang) {
        (Some(d), 1, Lang::En) => format!(
            "It follows a confidential draft registration statement ({forms}) submitted on {}.",
            day(d, lang)
        ),
        (Some(d), n, Lang::En) => format!(
            "It follows {n} confidential draft registration statements ({forms}), the first submitted on {}.",
            day(d, lang)
        ),
        (Some(d), 1, Lang::Zh) => format!(
            "此前公司已于{}保密提交1份注册草案（{forms}）。",
            day(d, lang)
        ),
        (Some(d), n, Lang::Zh) => format!(
            "此前公司已保密提交{n}份注册草案（{forms}），首份提交于{}。",
            day(d, lang)
        ),
        (None, _, _) => String::new(),
    };
    let cover = match (f.num("shares_offered"), f.num("price_low"), lang) {
        (Some(t), None, Lang::En) => {
            format!("Its cover lists {} and no price range.", count(t, f, lang))
        }
        (Some(t), None, Lang::Zh) => {
            format!("封面列明发行{}，尚未给出价格区间。", count(t, f, lang))
        }
        _ => String::new(),
    };
    let rest = terms(f, lang);
    Text {
        title: match lang {
            Lang::En => format!("{n} files {form} for IPO"),
            Lang::Zh => format!("{n}递交{form}申请IPO"),
        },
        summary: match lang {
            Lang::En => format!(
                "{n} filed {label} with the SEC on {on}, its first public filing for an initial public offering."
            ),
            Lang::Zh => format!(
                "{n}于{on}向美国证监会递交{label}，这是其首次公开发行的首份公开注册文件。"
            ),
        },
        first: para(lang, vec![filed_sentence(n, &label, &on, lang), follows, cover]),
        second: if rest.is_empty() {
            pick(
                lang,
                "The IPO page adds the offering terms from this cover once they are read.",
                "封面上的发行条款读取后，会显示在IPO页面上。",
            )
            .to_string()
        } else {
            rest
        },
    }
}

fn ranged(f: &IpoArticleFacts, lang: Lang) -> Text {
    let n = name_of(f, lang);
    let u = unit(f);
    let (form, date) = filing_parts(f);
    let on = day(&date, lang);
    let label = form_label(&form, lang);
    let (lo, hi) = f
        .num("price_low")
        .zip(f.num("price_high"))
        .unwrap_or_default();
    let changed = f.milestone == Milestone::RangeChanged;
    let prev: Option<&Previous> = f.previous.as_ref().filter(|_| changed);
    let old = prev.and_then(|p| p.price_low.zip(p.price_high));
    let range_moved = old.is_some_and(|o| o != (lo, hi));
    let verb = match old {
        Some((a, b)) if lo >= a && hi >= b && (lo, hi) != (a, b) => ("raises", "上调"),
        Some((a, b)) if lo <= a && hi <= b && (lo, hi) != (a, b) => ("lowers", "下调"),
        _ => ("changes", "调整"),
    };
    let total = f.num("shares_offered");
    // "Sets" only when the facts prove no earlier cover had a range
    // (`first_range`); otherwise the filing only "gives" one.
    let title = match (changed, range_moved || total.is_none(), lang) {
        (false, _, Lang::En) if f.first_range => format!(
            "{n} sets IPO price range at ${} to ${} {}",
            price_short(lo),
            price_short(hi),
            u.a_one
        ),
        (false, _, Lang::En) => format!(
            "{n}'s IPO filing gives a price range of ${} to ${} {}",
            price_short(lo),
            price_short(hi),
            u.a_one
        ),
        (false, _, Lang::Zh) if f.first_range => format!(
            "{n}公布IPO价格区间：{}{}至{}美元",
            u.zh_each,
            price_short(lo),
            price_short(hi)
        ),
        (false, _, Lang::Zh) => format!(
            "{}文件载明价格区间：{}{}至{}美元",
            zh_join(n, "IPO"),
            u.zh_each,
            price_short(lo),
            price_short(hi)
        ),
        (true, true, Lang::En) => format!(
            "{n} {} IPO price range to ${} to ${} {}",
            verb.0,
            price_short(lo),
            price_short(hi),
            u.a_one
        ),
        (true, true, Lang::Zh) => format!(
            "{n}{}IPO价格区间至{}{}至{}美元",
            verb.1,
            u.zh_each,
            price_short(lo),
            price_short(hi)
        ),
        (true, false, Lang::En) => format!(
            "{n} changes IPO {} count to {} at ${} to ${} {}",
            u.one,
            grouped(total.unwrap_or_default()),
            price_short(lo),
            price_short(hi),
            u.a_one
        ),
        (true, false, Lang::Zh) => format!(
            "{n}将IPO发行数量调整为{}，{}{}至{}美元",
            count(total.unwrap_or_default(), f, lang),
            u.zh_each,
            price_short(lo),
            price_short(hi)
        ),
    };
    let words = range_words(lo, hi, &u, lang);
    let for_shares = for_total(f, lang);
    let shares_change = prev.and_then(|p| p.shares_offered).zip(total);
    let summary = match (prev, old, lang) {
        // The range is unchanged: the share count is the news.
        (Some(p), Some(_), Lang::En) if !range_moved => match shares_change {
            Some((ps, t)) => format!(
                "{n}'s {form} filed {on} keeps {words} and changes the number offered from {} to {}, against its {} filed {}.",
                count(ps, f, lang),
                count(t, f, lang),
                p.form,
                day(&p.filing_date, lang)
            ),
            None => format!("{n}'s {form} filed {on} keeps {words}{for_shares}."),
        },
        (Some(p), Some(_), Lang::Zh) if !range_moved => match shares_change {
            Some((ps, t)) => format!(
                "{n}于{on}递交{form}，价格区间维持{words}不变，发行数量由{}调整为{}（相较{}提交的{}）。",
                count(ps, f, lang),
                count(t, f, lang),
                day(&p.filing_date, lang),
                p.form
            ),
            None => format!("{n}于{on}递交{form}，价格区间维持{words}不变{for_shares}。"),
        },
        (Some(p), Some((a, b)), Lang::En) => format!(
            "{n}'s {form} filed {on} sets {words}{for_shares}, against {} to {} in its {} filed {}.",
            price(a, lang),
            price(b, lang),
            p.form,
            day(&p.filing_date, lang)
        ),
        (Some(p), Some((a, b)), Lang::Zh) => format!(
            "{n}于{on}递交{form}，价格区间为{words}{for_shares}；{}提交的{}所载区间为{}至{}。",
            day(&p.filing_date, lang),
            p.form,
            bare(a),
            price(b, lang)
        ),
        (_, _, Lang::En) => format!("{n} filed {label} on {on} with {words}{for_shares}."),
        (_, _, Lang::Zh) => {
            format!("{n}于{on}递交{label}，价格区间为{words}{for_shares}。")
        }
    };
    let mut s1 = vec![filed_sentence(n, &label, &on, lang)];
    if !changed && f.first_range {
        s1.push(
            pick(
                lang,
                "It is the first registration statement for this offering to state a price range.",
                "这是本次发行中首份载明价格区间的注册声明。",
            )
            .to_string(),
        );
    }
    // "Sets" / "定为" only when the range is provably the first.
    s1.push(match (lang, !changed && f.first_range) {
        (Lang::En, true) => format!("The cover sets {words}{for_shares}."),
        (Lang::En, false) => format!("The cover states {words}{for_shares}."),
        (Lang::Zh, true) => format!("封面将价格区间定为{words}{for_shares}。"),
        (Lang::Zh, false) => format!("封面载明的价格区间为{words}{for_shares}。"),
    });
    if let (Some(p), Some((a, b))) = (prev, old) {
        s1.push(match lang {
            Lang::En => format!(
                "Against the {} filed {}, the low end {} and the high end {}.",
                p.form,
                day(&p.filing_date, lang),
                moved(a, lo, lang),
                moved(b, hi, lang)
            ),
            Lang::Zh => format!(
                "与{}提交的{}相比，区间下限{}，上限{}。",
                day(&p.filing_date, lang),
                p.form,
                moved(a, lo, lang),
                moved(b, hi, lang)
            ),
        });
    }
    if let (Some(ps), Some(t)) = (prev.and_then(|p| p.shares_offered), total) {
        s1.push(match (ps == t, lang) {
            (true, Lang::En) => {
                format!("The number offered is unchanged at {}.", count(t, f, lang))
            }
            (true, Lang::Zh) => format!("发行数量维持{}不变。", count(t, f, lang)),
            (false, Lang::En) => format!(
                "The number offered changed from {} to {}.",
                count(ps, f, lang),
                count(t, f, lang)
            ),
            (false, Lang::Zh) => format!(
                "发行数量由{}调整为{}。",
                count(ps, f, lang),
                count(t, f, lang)
            ),
        });
    }
    Text {
        title,
        summary,
        first: para(lang, s1),
        second: terms(f, lang),
    }
}

fn priced(f: &IpoArticleFacts, lang: Lang) -> Text {
    let n = name_of(f, lang);
    let u = unit(f);
    let (form, date) = filing_parts(f);
    let on = day(&date, lang);
    let label = form_label(&form, lang);
    let p = f.num("price_final").unwrap_or_default();
    let for_shares = for_total(f, lang);
    let mut s1 = vec![filed_sentence(n, &label, &on, lang)];
    if let Some(d) = f.text("prospectus_date") {
        s1.push(match lang {
            Lang::En => format!("The prospectus is dated {}.", day(d, lang)),
            Lang::Zh => format!("招股说明书日期为{}。", day(d, lang)),
        });
    }
    s1.push(match (f.num("shares_offered"), lang) {
        (Some(t), Lang::En) => format!(
            "It prices {} at {} per {}.",
            count(t, f, lang),
            price(p, lang),
            u.one
        ),
        (Some(t), Lang::Zh) => format!(
            "发行价为{}{}，发行{}。",
            u.zh_each,
            price(p, lang),
            count(t, f, lang)
        ),
        (None, Lang::En) => format!(
            "It sets the offering price at {} per {}.",
            price(p, lang),
            u.one
        ),
        (None, Lang::Zh) => format!("发行价为{}{}。", u.zh_each, price(p, lang)),
    });
    if let Some(pr) = &f.previous {
        if let (Some(lo), Some(hi)) = (pr.price_low, pr.price_high) {
            let (en, zh) = placement(p, lo, hi);
            s1.push(match lang {
                Lang::En => format!(
                    "That price is {en} the range of {} to {} in the {} filed {}.",
                    price(lo, lang),
                    price(hi, lang),
                    pr.form,
                    day(&pr.filing_date, lang)
                ),
                Lang::Zh => format!(
                    "该价格{zh}{}提交的{}所载区间{}至{}{}。",
                    day(&pr.filing_date, lang),
                    pr.form,
                    bare(lo),
                    price(hi, lang),
                    if en == "within" { "之内" } else { "" }
                ),
            });
        }
    }
    if let Some(d) = f.text("trading_date") {
        s1.push(match lang {
            Lang::En => format!(
                "The prospectus gives {} as the first day of trading.",
                day(d, lang)
            ),
            Lang::Zh => format!("招股说明书载明的首个交易日为{}。", day(d, lang)),
        });
    }
    Text {
        title: match lang {
            Lang::En => format!("{n} prices IPO at ${} {}", price_short(p), u.a_one),
            Lang::Zh => format!("{n}完成IPO定价：{}{}美元", u.zh_each, price_short(p)),
        },
        summary: match lang {
            Lang::En => format!(
                "{n}'s final prospectus (Form {form}) filed {on} sets the offering price at {} per {}{for_shares}.",
                price(p, lang),
                u.one
            ),
            Lang::Zh => format!(
                "{n}于{on}递交最终招股说明书（{form}表格），发行价定为{}{}{for_shares}。",
                u.zh_each,
                price(p, lang)
            ),
        },
        first: para(lang, s1),
        second: terms(f, lang),
    }
}

fn listed(f: &IpoArticleFacts, lang: Lang) -> Text {
    let n = name_of(f, lang);
    let u = unit(f);
    let (form, date) = filing_parts(f);
    let on = day(&date, lang);
    let s = f.session.as_ref();
    let close = s.map_or(0.0, |s| s.close);
    let when = s.map_or(String::new(), |s| day(&s.session_date, lang));
    let change = s.and_then(|s| s.ipo_price.zip(s.change_pct));
    let vs = change.map_or(String::new(), |(ipo, c)| match lang {
        Lang::En => format!(
            ", {} {}% from the {} offering price",
            if c < 0.0 { "down" } else { "up" },
            trim_dec(c.abs(), 2),
            price(ipo, lang)
        ),
        Lang::Zh => format!(
            "，较{}的发行价{}{}%",
            price(ipo, lang),
            if c < 0.0 { "下跌" } else { "上涨" },
            trim_dec(c.abs(), 2)
        ),
    });
    let who = match (&f.symbol, lang) {
        (Some(s), Lang::En) => format!("{n}'s {} ({s})", u.plural),
        (None, Lang::En) => format!("{n}'s {}", u.plural),
        (Some(s), Lang::Zh) => format!("{n}（{s}）"),
        (None, Lang::Zh) => n.to_string(),
    };
    let mut s1 = vec![match (&f.exchange, lang) {
        (Some(x), Lang::En) => format!("{who} began trading on the {x} on {when}."),
        (Some(x), Lang::Zh) => format!("{who}于{when}在{}开始交易。", exchange_name(x, lang)),
        (None, Lang::En) => format!("{who} began trading on {when}."),
        (None, Lang::Zh) => format!("{who}于{when}开始交易。"),
    }];
    if let Some(s) = s {
        let mut bits: Vec<String> = Vec::new();
        if let Some(o) = s.open {
            bits.push(match lang {
                Lang::En => format!("opened at {}", price(o, lang)),
                Lang::Zh => format!("开盘价为{}", price(o, lang)),
            });
        }
        bits.push(match lang {
            Lang::En => format!("closed at {}", price(s.close, lang)),
            Lang::Zh => format!("收盘价为{}", price(s.close, lang)),
        });
        if let Some(v) = s.volume {
            bits.push(match lang {
                Lang::En => format!("{} traded", count(v, f, lang)),
                Lang::Zh => format!("成交{}", count(v, f, lang)),
            });
        }
        s1.push(match lang {
            Lang::En => format!("In that session the {} {}.", u.plural, en_list(&bits)),
            Lang::Zh => format!("当日{}。", bits.join("，")),
        });
        if let Some((ipo, c)) = change {
            s1.push(match lang {
                Lang::En => format!(
                    "The close of {} is {}% {} the offering price of {}.",
                    price(s.close, lang),
                    trim_dec(c.abs(), 2),
                    if c < 0.0 { "below" } else { "above" },
                    price(ipo, lang)
                ),
                Lang::Zh => format!(
                    "收盘价{}较{}的发行价{}{}%。",
                    price(s.close, lang),
                    price(ipo, lang),
                    if c < 0.0 { "低" } else { "高" },
                    trim_dec(c.abs(), 2)
                ),
            });
        }
    }
    let s2 = match (f.num("price_final"), f.num("price_low").zip(f.num("price_high"))) {
        (Some(p), _) => {
            let lead = match (f.num("shares_offered"), lang) {
                (Some(t), Lang::En) => format!(
                    "{n}'s final prospectus (Form {form}) filed {on} priced {} at {} per {}.",
                    count(t, f, lang),
                    price(p, lang),
                    u.one
                ),
                (Some(t), Lang::Zh) => format!(
                    "{n}于{on}递交的最终招股说明书（{form}表格）将{}的发行价定为{}{}。",
                    count(t, f, lang),
                    u.zh_each,
                    price(p, lang)
                ),
                (None, Lang::En) => format!(
                    "{n}'s final prospectus (Form {form}) filed {on} set the offering price at {} per {}.",
                    price(p, lang),
                    u.one
                ),
                (None, Lang::Zh) => format!(
                    "{n}于{on}递交的最终招股说明书（{form}表格）将发行价定为{}{}。",
                    u.zh_each,
                    price(p, lang)
                ),
            };
            para(lang, vec![lead, terms(f, lang)])
        }
        (None, Some((lo, hi))) => para(
            lang,
            vec![
                match lang {
                    Lang::En => format!(
                        "The offering terms PreBell last read are from the {form} filed {on}, which gave {}. The offering price is added once the final prospectus is read.",
                        range_words(lo, hi, &u, lang)
                    ),
                    Lang::Zh => format!(
                        "PreBell最近读取的发行条款来自{on}提交的{form}，价格区间为{}。读取最终招股说明书后会补充发行价。",
                        range_words(lo, hi, &u, lang)
                    ),
                },
                terms(f, lang),
            ],
        ),
        (None, None) => pick(
            lang,
            "The offering terms appear once PreBell reads them from the filings.",
            "PreBell从招股文件读取发行条款后会补充在此。",
        )
        .to_string(),
    };
    Text {
        title: match lang {
            Lang::En => format!("{n} closes first trading day at ${}", price_short(close)),
            Lang::Zh => format!("{n}上市首日收于{}", price(close, lang)),
        },
        summary: match lang {
            Lang::En => format!(
                "{n} {} closed their first trading session on {when} at {}{vs}.",
                u.plural,
                price(close, lang)
            ),
            Lang::Zh => format!(
                "{n}于{when}完成上市首个交易日，收于{}{vs}。",
                price(close, lang)
            ),
        },
        first: para(lang, s1),
        second: s2,
    }
}

/// "a, b and c".
fn en_list(bits: &[String]) -> String {
    match bits {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn withdrawn(f: &IpoArticleFacts, lang: Lang) -> Text {
    let n = name_of(f, lang);
    let u = unit(f);
    let (_, date) = filing_parts(f);
    let on = day(&date, lang);
    let label = form_label("RW", lang);
    let prev = f.previous.as_ref();
    let first = para(
        lang,
        vec![
            filed_sentence(n, &label, &on, lang),
            prev.map_or(String::new(), |p| match lang {
                Lang::En => format!(
                    "The offering terms PreBell last read are from the {} filed {}.",
                    p.form,
                    day(&p.filing_date, lang)
                ),
                Lang::Zh => format!(
                    "PreBell最近读取的发行条款来自{}提交的{}。",
                    day(&p.filing_date, lang),
                    p.form
                ),
            }),
        ],
    );
    let second = match prev {
        Some(p) => match (p.price_low.zip(p.price_high), p.shares_offered, lang) {
            (Some((lo, hi)), Some(t), Lang::En) => format!(
                "That filing set {} for {}.",
                range_words(lo, hi, &u, lang),
                count(t, f, lang)
            ),
            (Some((lo, hi)), Some(t), Lang::Zh) => format!(
                "该文件的价格区间为{}，发行{}。",
                range_words(lo, hi, &u, lang),
                count(t, f, lang)
            ),
            (Some((lo, hi)), None, Lang::En) => {
                format!("That filing set {}.", range_words(lo, hi, &u, lang))
            }
            (Some((lo, hi)), None, Lang::Zh) => {
                format!("该文件的价格区间为{}。", range_words(lo, hi, &u, lang))
            }
            (None, Some(t), Lang::En) => format!("That filing listed {}.", count(t, f, lang)),
            (None, Some(t), Lang::Zh) => format!("该文件列明发行{}。", count(t, f, lang)),
            (None, None, _) => pick(
                lang,
                "No offering terms were stored from it.",
                "未存储该文件的发行条款。",
            )
            .to_string(),
        },
        None => pick(
            lang,
            "No offering terms were stored for this offering.",
            "未存储本次发行的条款。",
        )
        .to_string(),
    };
    Text {
        title: match lang {
            Lang::En => format!("{n} withdraws IPO registration"),
            Lang::Zh => format!("{n}撤回IPO注册"),
        },
        summary: match lang {
            Lang::En => format!(
                "{n} filed Form RW with the SEC on {on} to withdraw its registration statement."
            ),
            Lang::Zh => format!("{n}于{on}向美国证监会递交RW表格，撤回注册声明。"),
        },
        first,
        second,
    }
}

fn filing_parts(f: &IpoArticleFacts) -> (String, String) {
    f.filing
        .as_ref()
        .map(|x| (x.form.clone(), x.filing_date.clone()))
        .unwrap_or_default()
}

/// The text of one language, by code: title, summary and both sections.
pub fn write(f: &IpoArticleFacts, lang: Lang) -> Draft {
    let t = match f.milestone {
        Milestone::Filed => filed(f, lang),
        Milestone::RangeSet | Milestone::RangeChanged => ranged(f, lang),
        Milestone::Priced => priced(f, lang),
        Milestone::Listed => listed(f, lang),
        Milestone::Withdrawn => withdrawn(f, lang),
    };
    let [h1, h2] = headings(lang);
    Draft {
        lang,
        title: t.title,
        summary: t.summary,
        sections: vec![(h1.into(), t.first), (h2.into(), t.second)],
    }
}

// ---------------------------------------------------------------- page

fn table(f: &IpoArticleFacts, lang: Lang) -> String {
    let mut rows: Vec<(String, String)> = Vec::new();
    let u = unit(f);
    let each = |en: &str, zh: &str| match lang {
        Lang::En => format!("{en} per {}", u.one),
        Lang::Zh => format!("{}{zh}", u.zh_each),
    };
    let l = |en: &str, zh: &str| match lang {
        Lang::En => en.to_string(),
        Lang::Zh => zh.to_string(),
    };
    if let (Some(lo), Some(hi)) = (f.num("price_low"), f.num("price_high")) {
        if f.num("price_final").is_none() {
            rows.push((each("Price range", "价格区间"), range(lo, hi, lang)));
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
                l("Previously offered", "此前发行数量"),
                shares_cell(s, f, lang),
            ));
        }
    }
    if let Some(p) = f.num("price_final") {
        rows.push((each("Offering price", "发行价"), price(p, lang)));
    }
    let offered = match lang {
        Lang::En => format!("{} offered", capital(u.plural)),
        Lang::Zh => "发行数量".into(),
    };
    let sold_by = match lang {
        Lang::En => format!("Sold by {}", sellers(f)),
        Lang::Zh => "售股股东发售".into(),
    };
    for (k, label) in [
        ("shares_offered", offered),
        ("shares_company", l("Sold by the company", "公司发售")),
        ("shares_selling", sold_by),
    ] {
        if let Some(v) = f.num(k) {
            rows.push((label, shares_cell(v, f, lang)));
        }
    }
    if let Some(v) = f.num("option_shares") {
        let cell = match lang {
            Lang::En => format!("{} {}", grouped(v), u.plural),
            Lang::Zh => shares_cell(v, f, lang),
        };
        rows.push((l("Underwriters' option", "承销商超额配售权"), cell));
    }
    for d in &f.derived {
        let label = match lang {
            Lang::En => format!(
                "Offering size at {p} ({} {} × {p})",
                grouped(d.shares),
                u.plural,
                p = price(d.price, lang)
            ),
            Lang::Zh => format!(
                "按{}{p}计算的发行规模（{}{}×{p}）",
                u.zh_each,
                grouped(d.shares),
                u.zh,
                p = price(d.price, lang)
            ),
        };
        rows.push((label, money(d.value_usd, lang)));
    }
    if let (Some(x), Some(s)) = (&f.exchange, &f.symbol) {
        rows.push((
            l("Exchange and symbol", "交易所与代码"),
            format!(
                "{}{}{s}",
                escape_md(&exchange_name(x, lang)),
                pick(lang, ": ", "：")
            ),
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
            rows.push((l("Volume", "成交量"), shares_cell(v, f, lang)));
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
    if !f.derived.is_empty() {
        out.push_str(&offering_size_note(f, lang));
        out.push_str("\n\n");
    }
    out
}

/// The definition under the table, in the cover's unit and its word for
/// the other sellers.
pub fn offering_size_note(f: &IpoArticleFacts, lang: Lang) -> String {
    let note = match (lang, f.security == "ads") {
        (Lang::En, true) => OFFERING_SIZE_EN
            .replace(
                "shares offered (including shares sold",
                "ADSs offered (including ADSs sold",
            )
            .replace("the price per share", "the price per ADS"),
        (Lang::Zh, true) => OFFERING_SIZE_ZH
            .replace(
                "发行股数（含售股股东发售的股份",
                "发行的美国存托股数量（含售股股东发售的部分",
            )
            .replace("×每股价格", "×每份价格"),
        (Lang::En, false) => OFFERING_SIZE_EN.to_string(),
        (Lang::Zh, false) => OFFERING_SIZE_ZH.to_string(),
    };
    note.replace("selling stockholders", sellers(f))
}

/// One news item, quoted word for word with its outlet and date (in
/// Chinese the quote stays in English).
fn item_line(i: &QuotedItem, lang: Lang) -> String {
    let q = escape_md(&i.quote);
    let outlet = escape_md(&i.outlet);
    let when = day(&i.published_on, lang);
    let url = link_url(&i.url);
    match (lang, i.speaker.as_deref()) {
        (Lang::En, Some(s)) => format!(
            "- {}, in {outlet}, {when}: \"{q}\" ([article]({url}))",
            escape_md(s)
        ),
        (Lang::En, None) => format!("- {outlet}, {when}: \"{q}\" ([article]({url}))"),
        (Lang::Zh, Some(s)) => format!(
            "- {}（{outlet}，{when}）：“{q}”（[原文]({url})）",
            escape_md(s)
        ),
        (Lang::Zh, None) => format!("- {outlet}（{when}）报道：“{q}”（[原文]({url})）"),
    }
}

/// The step that follows, stated generally (no date).
fn next_step(m: Milestone, lang: Lang) -> &'static str {
    match (m, lang) {
        (Milestone::Filed, Lang::En) => "A price range is usually set in an amended registration statement. Pricing follows, with the offering price stated in a final prospectus, and then the first day of trading.",
        (Milestone::Filed, Lang::Zh) => "价格区间通常在修订后的注册声明中公布。之后是定价，发行价在最终招股说明书中载明，然后开始首日交易。",
        (Milestone::RangeSet | Milestone::RangeChanged, Lang::En) => "The range can still change in a later amendment. The offering price is set at pricing and stated in a final prospectus (Form 424B4 or 424B1), after which the shares begin trading.",
        (Milestone::RangeSet | Milestone::RangeChanged, Lang::Zh) => "价格区间仍可能在之后的修订文件中调整。发行价在定价时确定，并在最终招股说明书（424B4或424B1表格）中载明，之后股票开始交易。",
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
        parts.push(escape_md(&exchange_name(x, lang)));
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
            "- [Form {form} filed {} (SEC EDGAR)]({})\n",
            day(date, lang),
            link_url(url)
        ),
        Lang::Zh => format!(
            "- [{form}，提交于{}（SEC EDGAR）]({})\n",
            day(date, lang),
            link_url(url)
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
            Lang::En => format!(
                "- [{} on Yahoo Finance]({})\n",
                s.symbol,
                link_url(&s.source_url)
            ),
            Lang::Zh => format!(
                "- [{}（Yahoo Finance）]({})\n",
                s.symbol,
                link_url(&s.source_url)
            ),
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
            link_url(&i.url)
        ));
    }
    out
}

const REPORTED: [&str; 2] = ["## What has been reported\n\n", "## 已有报道\n\n"];

/// The full article: title, summary, meta line, the figures table, the two
/// sections, the quoted news, the next step, the sources and the
/// disclaimer (kept in the body; the site does not show it, Rick
/// 2026-11-05).
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
        out.push_str(pick(lang, REPORTED[0], REPORTED[1]));
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

// ---------------------------------------------------------------- guard

/// The dates the page may hold: the filings', the cover's own, the first
/// session's and the quoted items' publication days.
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
    for i in &f.items {
        add(&i.published_on);
    }
    out
}

/// The figures the page may hold: the cover's, the previous range's, the
/// first session's and code's derived offering sizes.
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

/// EN "Nov 10, 2026", "November 10"; ZH "2026年11月10日".
fn date_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        re(&format!(
            r"(?-u:\b)(?P<m1>{MONTHS})\.?\s+(?P<d1>\d{{1,2}})(?-u:\b)(?:,?\s+(?P<y1>\d{{4}}))?|(?:(?P<y3>\d{{4}})\s*年\s*)?(?P<m3>\d{{1,2}})\s*月\s*(?P<d3>\d{{1,2}})\s*[日号]"
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

fn date_ok(c: &regex::Captures, allowed: &HashSet<NaiveDate>) -> bool {
    let get = |k: &str| c.name(k).map(|m| m.as_str());
    let (m, d, y) = if let Some(m) = get("m1") {
        (month_number(m), get("d1"), get("y1"))
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

/// `word` replaced where no ASCII letter or digit touches it.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    if word.is_empty() {
        return text.to_string();
    }
    let glued = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    let mut out = String::with_capacity(text.len());
    let mut rest = 0;
    for (i, _) in text.match_indices(word) {
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

/// Every number and date in `md` outside the quoted news is one the bundle
/// holds (names, symbols and form names, which hold digits, aside). A
/// failure is a bug in a template, and nothing is published.
pub fn verify_page(md: &str, f: &IpoArticleFacts, lang: Lang) -> Result<(), Vec<String>> {
    let reported = pick(lang, REPORTED[0], REPORTED[1]);
    let mut text = md.to_string();
    if let Some(a) = text.find(reported) {
        let body = a + reported.len();
        let end = text[body..]
            .find("\n## ")
            .map_or(text.len(), |e| body + e + 1);
        text.replace_range(a..end, "");
    }
    let mut text = without_link_targets(&text);
    let allowed = allowed_dates(f);
    let mut bad: Vec<String> = date_re()
        .captures_iter(&text)
        .filter(|c| !date_ok(c, &allowed))
        .map(|c| format!("date not in the facts: {}", &c[0]))
        .collect();
    text = date_re().replace_all(&text, " ").into_owned();
    let mut words: Vec<&str> = FORMS.iter().map(|(f, _, _)| *f).collect();
    words.extend(f.symbol.as_deref());
    words.push(f.name.as_str());
    words.extend(f.name_zh.as_deref());
    words.extend(f.exchange.as_deref());
    words.sort_by_key(|w| std::cmp::Reverse(w.len()));
    for w in words {
        text = replace_word(&text, w, " ");
    }
    let years: Vec<f64> = allowed.iter().map(|d| f64::from(d.year())).collect();
    if let Err(n) = check_allowed(
        &text,
        lang,
        &Allowed {
            any: &allowed_numbers(f),
            years: &years,
            ..Allowed::default()
        },
    ) {
        bad.extend(
            n.into_iter()
                .map(|x| format!("number not in the facts: {x}")),
        );
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

/// The write-up of one language and its page, checked by `verify_page`.
pub fn render(f: &IpoArticleFacts, lang: Lang) -> Result<(Draft, String), Vec<String>> {
    let d = write(f, lang);
    let md = render_markdown(&d, f);
    verify_page(&md, f, lang)?;
    Ok((d, md))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ipo::cover::tests::cover_text;
    use crate::ipo::cover::{
        extract,
        tests::{ARM, COREWEAVE, REDDIT, S1A},
    };
    use crate::ipo::milestone::tests::{anthropic, real_state};
    use crate::ipo::milestone::{bundle, latest};
    use crate::ipo::parse_list;
    use crate::ipo::store::{FactRow, FilingRow, IpoState, ItemRow, TradingRow, WatchRow};
    use crate::ipo::Tracked;

    const INSTACART: &str = include_str!("../../tests/fixtures/edgar_ipo_424b4_instacart.htm");
    const CIRCLE: &str = include_str!("../../tests/fixtures/edgar_ipo_424b4_circle.htm");
    const BIRKENSTOCK: &str = include_str!("../../tests/fixtures/edgar_ipo_f1a_birkenstock.htm");

    fn at() -> chrono::DateTime<chrono::Utc> {
        "2026-11-11T22:00:00Z".parse().unwrap()
    }

    pub(crate) fn facts() -> IpoArticleFacts {
        let st = real_state();
        let r = latest(&st, "anthropic").unwrap();
        bundle(&st, &anthropic(), &r, at())
    }

    /// The local preview's write-up: the real state with the made-up news
    /// items of `db/seed-ipo-preview.sql` (FIXTURE rows there).
    pub(crate) fn preview_facts() -> IpoArticleFacts {
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
        bundle(&st, &anthropic(), &r, at())
    }

    fn page(f: &IpoArticleFacts, lang: Lang) -> (Draft, String) {
        render(f, lang).unwrap_or_else(|e| {
            panic!(
                "{lang:?} refused: {e:?}\n{}",
                render_markdown(&write(f, lang), f)
            )
        })
    }

    fn section(d: &Draft, i: usize) -> &str {
        &d.sections[i].1
    }

    pub(crate) const EN_HAPPENED: &str = "Anthropic filed an amended registration statement (Form S-1/A) with the SEC on Nov 10, 2026. The cover states a price range of $70.00 to $78.00 per share for 825,000,000 shares.";
    pub(crate) const EN_TERMS: &str = "Anthropic is selling 801,351,354 shares and the selling stockholders are selling 23,648,646 shares. The underwriters have an option to purchase up to 123,750,000 more shares. At $70.00 a share, the 825,000,000 shares offered come to $57.75 billion; at $78.00 a share, the 825,000,000 shares offered come to $64.35 billion. These are PreBell's gross figures before underwriting discounts, including any shares sold by selling stockholders and excluding the underwriters' option; they are not the company's proceeds. The filing names the Nasdaq Global Select Market as the exchange and ANT as the symbol.";
    pub(crate) const ZH_HAPPENED: &str = "Anthropic于2026年11月10日向美国证监会递交注册声明修订稿（S-1/A表格）。封面载明的价格区间为每股70.00至78.00美元，发行8.25亿股。";
    pub(crate) const ZH_TERMS: &str = "公司发售801,351,354股，售股股东发售23,648,646股。承销商有权额外购买至多123,750,000股。按每股70.00美元计算，8.25亿股的发行规模为577.5亿美元；按每股78.00美元计算，8.25亿股的发行规模为643.5亿美元。以上为PreBell按发行数量乘以价格算出的总额，未扣除承销折扣，含售股股东发售的部分（如有），不含承销商超额配售权，并非公司所得款项。文件载明的交易所为纳斯达克全球精选市场，股票代码为ANT。";

    #[test]
    fn the_real_range_set_is_written_by_code_in_both_languages() {
        let f = facts();
        let (d, md) = page(&f, Lang::En);
        // Its earlier covers were never read: the filing "gives" a range;
        // nothing says it is the first or that it is new.
        assert_eq!(
            d.title,
            "Anthropic's IPO filing gives a price range of $70 to $78 a share"
        );
        for not in ["first", " sets ", "new"] {
            let before = md.split("## What comes next").next().unwrap();
            assert!(!before.contains(not), "{not:?} in\n{md}");
        }
        assert_eq!(d.summary, "Anthropic filed an amended registration statement (Form S-1/A) on Nov 10, 2026 with a price range of $70.00 to $78.00 per share for 825,000,000 shares.");
        assert_eq!(section(&d, 0), EN_HAPPENED);
        assert_eq!(section(&d, 1), EN_TERMS);
        for want in [
            "**Anthropic (ANT)** · Nasdaq Global Select Market · Form S-1/A filed Nov 10, 2026",
            "| Price range per share | $70.00–$78.00 |",
            "| Shares offered | 825,000,000 |",
            "| Sold by the company | 801,351,354 |",
            "| Sold by selling stockholders | 23,648,646 |",
            "| Underwriters' option | 123,750,000 shares |",
            "| Offering size at $70.00 (825,000,000 shares × $70.00) | $57.75 billion |",
            "| Offering size at $78.00 (825,000,000 shares × $78.00) | $64.35 billion |",
            OFFERING_SIZE_EN,
            "(Form 424B4 or 424B1)",
            DISCLAIMER_EN,
        ] {
            assert!(md.contains(want), "missing {want:?} in\n{md}");
        }
        // The earlier covers were never read: no claim to be the first range.
        assert!(!md.contains("first registration statement"));
        assert!(!md.contains("What has been reported"));
        let (z, zmd) = page(&f, Lang::Zh);
        assert_eq!(z.title, "Anthropic IPO文件载明价格区间：每股70至78美元");
        for not in ["首次", "首份", "公布", "定为", "新"] {
            let before = zmd.split("## 下一步").next().unwrap();
            assert!(!before.contains(not), "{not:?} in\n{zmd}");
        }
        assert_eq!(z.summary, "Anthropic于2026年11月10日递交注册声明修订稿（S-1/A表格），价格区间为每股70.00至78.00美元，发行8.25亿股。");
        assert_eq!(section(&z, 0), ZH_HAPPENED);
        assert_eq!(section(&z, 1), ZH_TERMS);
        for want in [
            "| 每股价格区间 | 70.00–78.00美元 |",
            "| 按每股70.00美元计算的发行规模（825,000,000股×70.00美元） | 577.5亿美元 |",
            OFFERING_SIZE_ZH,
            DISCLAIMER_ZH,
        ] {
            assert!(zmd.contains(want), "missing {want:?} in\n{zmd}");
        }
        assert_eq!(
            cover_lines(&f, Lang::En),
            [
                "Anthropic IPO".to_string(),
                "Price range $70.00–$78.00".into()
            ]
        );
        // With every earlier cover read and rangeless, it is the first.
        let mut first = f.clone();
        first.first_range = true;
        let d = write(&first, Lang::En);
        assert_eq!(
            d.title,
            "Anthropic sets IPO price range at $70 to $78 a share"
        );
        assert!(section(&d, 0).contains(
            "It is the first registration statement for this offering to state a price range. The cover sets a price range of $70.00 to $78.00 per share for 825,000,000 shares."
        ));
        let z = write(&first, Lang::Zh);
        assert_eq!(z.title, "Anthropic公布IPO价格区间：每股70至78美元");
        assert!(section(&z, 0).contains("封面将价格区间定为每股70.00至78.00美元，发行8.25亿股。"));
    }

    #[test]
    fn money_stays_exact_in_the_natural_unit() {
        for (v, en, zh) in [
            (57.75e9, "$57.75 billion", "577.5亿美元"),
            (4_870_500_000.0, "$4.8705 billion", "48.705亿美元"),
            (53_625_000_000.0, "$53.625 billion", "536.25亿美元"),
            (748e6, "$748 million", "7.48亿美元"),
            (1_419_354_816.0, "$1,419,354,816", "1,419,354,816美元"),
        ] {
            assert_eq!(
                (money(v, Lang::En), money(v, Lang::Zh)),
                (en.to_string(), zh.to_string()),
                "{v}"
            );
        }
    }

    #[test]
    fn chinese_names_the_exchange_and_spaces_a_latin_name_before_ipo() {
        assert_eq!(
            exchange_name("New York Stock Exchange", Lang::Zh),
            "纽约证券交易所"
        );
        assert_eq!(
            exchange_name("Nasdaq Capital Market", Lang::Zh),
            "纳斯达克资本市场"
        );
        assert_eq!(
            exchange_name("NYSE American", Lang::Zh),
            "纽约证券交易所美国市场"
        );
        assert_eq!(
            exchange_name("Cboe BZX Exchange", Lang::Zh),
            "Cboe BZX Exchange"
        );
        assert_eq!(
            exchange_name("New York Stock Exchange", Lang::En),
            "New York Stock Exchange"
        );
        assert_eq!(zh_join("Anthropic", "IPO"), "Anthropic IPO");
        assert_eq!(zh_join("北风机器人", "IPO"), "北风机器人IPO");
        let (_, zh) = page(&facts(), Lang::Zh);
        assert!(
            zh.contains("**Anthropic（ANT）** · 纳斯达克全球精选市场 ·"),
            "{zh}"
        );
        assert!(
            !zh.split("## 下一步").next().unwrap().contains("其中"),
            "{zh}"
        );
    }

    #[test]
    fn chinese_counts_use_yi_or_wan_only_with_two_decimals() {
        for (v, want) in [
            (825e6, "8.25亿"),
            (801_351_354.0, "801,351,354"),
            (123_750_000.0, "123,750,000"),
            (22e6, "2200万"),
            (36_590_000.0, "3659万"),
            (910_000.0, "91万"),
            (23_648_646.0, "23,648,646"),
            (32_258_064.0, "32,258,064"),
            (9_550.0, "9,550"),
        ] {
            assert_eq!(zh_count(v), want, "{v}");
        }
    }

    /// A state holding one real cover, read, as the facts source.
    fn one_cover(
        slug: &str,
        cik: u64,
        name: &str,
        html: &str,
        form: &str,
        date: &str,
    ) -> (IpoState, Tracked) {
        let t = parse_list(&format!("{slug}\t{cik}\t{name}\t-\t-\n")).unwrap()[0].clone();
        let acc = format!("{cik:010}-25-000001");
        let facts: Vec<FactRow> = extract(&cover_text(html), form)
            .into_iter()
            .map(|c| FactRow {
                accession: acc.clone(),
                kind: c.kind.into(),
                slug: slug.into(),
                value_num: c.num,
                value_text: c.text,
                quote: c.quote,
            })
            .collect();
        let text = |k: &str| {
            facts
                .iter()
                .find(|f| f.kind == k)
                .and_then(|f| f.value_text.clone())
        };
        let st = IpoState {
            watch: vec![WatchRow {
                slug: slug.into(),
                cik: cik as i64,
                name: name.into(),
                status: "x".into(),
                exchange: text("exchange"),
                symbol: text("symbol"),
                ..Default::default()
            }],
            filings: vec![FilingRow {
                accession: acc.clone(),
                slug: slug.into(),
                form: form.into(),
                filing_date: date.into(),
                index_url: format!(
                    "https://www.sec.gov/Archives/edgar/data/{cik}/x/{acc}-index.htm"
                ),
                read_state: "read".into(),
                ..Default::default()
            }],
            facts,
            ..Default::default()
        };
        (st, t)
    }

    fn both(st: &IpoState, t: &Tracked) -> (IpoArticleFacts, String, String) {
        let r = latest(st, &t.slug).expect("a milestone");
        let f = bundle(st, t, &r, at());
        let en = page(&f, Lang::En).1;
        let zh = page(&f, Lang::Zh).1;
        (f, en, zh)
    }

    type Case = (
        &'static str,
        &'static str,
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
    );

    #[test]
    fn each_real_final_prospectus_is_written_from_its_own_cover() {
        // (cover, name, date, English sentences, Chinese sentences)
        let cases: [Case; 5] = [
            (REDDIT, "Reddit", "2024-03-21", &[
                "# Reddit prices IPO at $34 a share",
                "It prices 22,000,000 shares at $34.00 per share.",
                "Reddit is selling 15,276,527 shares and the selling stockholders are selling 6,723,473 shares.",
                "The underwriters have an option to purchase up to 3,300,000 more shares.",
                "At $34.00 a share, the 22,000,000 shares offered come to $748 million.",
                "The filing names the New York Stock Exchange as the exchange and RDDT as the symbol.",
            ], &["# Reddit完成IPO定价：每股34美元", "公司发售15,276,527股，售股股东发售6,723,473股。", "承销商有权额外购买至多330万股。", "发行价为每股34.00美元，发行2200万股。"]),
            (ARM, "Arm", "2023-09-14", &[
                "# Arm prices IPO at $51 an ADS",
                "It prices 95,500,000 ADSs at $51.00 per ADS.",
                "The underwriters have an option to purchase up to 7,000,000 more ADSs.",
                "At $51.00 an ADS, the 95,500,000 ADSs offered come to $4.8705 billion.",
                "| ADSs offered | 95,500,000 |",
                "Offering size is computed by PreBell from the cover: ADSs offered (including ADSs sold",
            ], &["发行价为每份美国存托股51.00美元，发行9550万份美国存托股。", "承销商有权额外购买至多700万份美国存托股。"]),
            (COREWEAVE, "CoreWeave", "2025-03-28", &[
                "CoreWeave is selling 36,590,000 shares and the selling stockholders are selling 910,000 shares.",
                "The filing names the Nasdaq Stock Market as the exchange and CRWV as the symbol.",
            ], &["公司发售3659万股，售股股东发售91万股。"]),
            (INSTACART, "Instacart", "2023-09-19", &[
                "Instacart is selling 14,100,000 shares and the selling stockholders are selling 7,900,000 shares.",
                "The prospectus is dated Sep 18, 2023.",
            ], &["公司发售1410万股，售股股东发售790万股。", "招股说明书日期为2023年9月18日。"]),
            (CIRCLE, "Circle", "2025-06-05", &[
                "Circle is selling 14,800,000 shares and the selling stockholders are selling 19,200,000 shares.",
                "It prices 34,000,000 shares at $31.00 per share.",
            ], &["发行价为每股31.00美元，发行3400万股。"]),
        ];
        for (i, (html, name, date, en_want, zh_want)) in cases.iter().enumerate() {
            let slug = name.to_lowercase();
            let (st, t) = one_cover(&slug, 9_000_000 + i as u64, name, html, "424B4", date);
            let (f, en, zh) = both(&st, &t);
            assert_eq!(f.milestone, Milestone::Priced, "{name}");
            for w in en_want.iter() {
                assert!(en.contains(w), "{name}: missing {w:?} in\n{en}");
            }
            for w in zh_want.iter() {
                assert!(zh.contains(w), "{name}: missing {w:?} in\n{zh}");
            }
        }
        // CoreWeave's cover has no option: no option sentence.
        let (st, t) = one_cover(
            "coreweave",
            9_100_000,
            "CoreWeave",
            COREWEAVE,
            "424B4",
            "2025-03-28",
        );
        let (_, en, zh) = both(&st, &t);
        assert!(
            !en.contains("option to purchase") && !zh.contains("额外购买"),
            "{en}"
        );
        // Arm's ADSs: never "shares" in the text.
        let (st, t) = one_cover("arm", 9_100_001, "Arm", ARM, "424B4", "2023-09-14");
        let (f, en, _) = both(&st, &t);
        assert_eq!(f.security, "ads");
        let body = en.split("## What comes next").next().unwrap();
        assert!(
            !body.contains(" shares ") && !body.contains("per share"),
            "{body}"
        );
    }

    #[test]
    fn birkenstocks_real_f1a_reads_as_a_foreign_filer_range() {
        let (mut st, t) = one_cover(
            "birkenstock",
            1_977_102,
            "Birkenstock",
            BIRKENSTOCK,
            "F-1/A",
            "2023-10-04",
        );
        // Its F-1 was filed first and not read.
        st.filings.push(FilingRow {
            accession: "0001977102-23-000001".into(),
            slug: "birkenstock".into(),
            form: "F-1".into(),
            filing_date: "2023-09-12".into(),
            index_url:
                "https://www.sec.gov/Archives/edgar/data/1977102/x/0001977102-23-000001-index.htm"
                    .into(),
            read_state: "none".into(),
            ..Default::default()
        });
        let (f, en, zh) = both(&st, &t);
        assert_eq!(f.milestone, Milestone::RangeSet);
        for w in [
            "# Birkenstock's IPO filing gives a price range of $44 to $49 a share",
            "Birkenstock filed an amended registration statement (Form F-1/A) with the SEC on Oct 4, 2023.",
            "The cover states a price range of $44.00 to $49.00 per share for 32,258,064 shares.",
            "Birkenstock is selling 10,752,688 shares and the selling shareholders are selling 21,505,376 shares.",
            "| Sold by selling shareholders | 21,505,376 |",
            "including any shares sold by selling shareholders",
        ] {
            assert!(en.contains(w), "missing {w:?} in\n{en}");
        }
        assert!(!en.contains("first registration statement"));
        for w in [
            "每股44.00至49.00美元，发行32,258,064股",
            "公司发售10,752,688股，售股股东发售21,505,376股。",
        ] {
            assert!(zh.contains(w), "missing {w:?} in\n{zh}");
        }
    }

    #[test]
    fn anthropics_real_cover_extracted_gives_the_same_page() {
        let (st, t) = one_cover(
            "anthropic",
            2130357,
            "Anthropic",
            S1A,
            "S-1/A",
            "2026-11-10",
        );
        let (_, en, _) = both(&st, &t);
        assert!(en.contains(EN_TERMS), "{en}");
    }

    fn with_amendment(st: &mut IpoState, acc: &str, date: &str, lo: f64, hi: f64, shares: f64) {
        st.filings.push(FilingRow {
            accession: acc.into(),
            slug: "anthropic".into(),
            form: "S-1/A".into(),
            filing_date: date.into(),
            index_url: format!("https://www.sec.gov/Archives/edgar/data/2130357/x/{acc}-index.htm"),
            read_state: "read".into(),
            ..Default::default()
        });
        for (k, n, tx) in [
            ("price_low", Some(lo), None),
            ("price_high", Some(hi), None),
            ("shares_offered", Some(shares), None),
            ("exchange", None, Some("Nasdaq Global Select Market")),
            ("symbol", None, Some("ANT")),
        ] {
            st.facts.push(FactRow {
                accession: acc.into(),
                kind: k.into(),
                slug: "anthropic".into(),
                value_num: n,
                value_text: tx.map(Into::into),
                quote: "q".into(),
            });
        }
    }

    #[test]
    fn a_changed_range_states_old_and_new_and_which_end_moved() {
        let mut st = real_state();
        with_amendment(
            &mut st,
            "0001628280-26-080003",
            "2026-11-14",
            76.0,
            80.0,
            825e6,
        );
        let r = latest(&st, "anthropic").unwrap();
        let f = bundle(&st, &anthropic(), &r, at());
        let (d, md) = page(&f, Lang::En);
        assert_eq!(
            d.title,
            "Anthropic raises IPO price range to $76 to $80 a share"
        );
        assert!(section(&d, 0).contains("Against the S-1/A filed Nov 10, 2026, the low end rose from $70.00 to $76.00 and the high end rose from $78.00 to $80.00. The number offered is unchanged at 825,000,000 shares."), "{md}");
        assert!(
            d.summary
                .contains("against $70.00 to $78.00 in its S-1/A filed Nov 10, 2026"),
            "{}",
            d.summary
        );
        let (z, _) = page(&f, Lang::Zh);
        assert!(section(&z, 0).contains("与2026年11月10日提交的S-1/A相比，区间下限由70.00美元上调至76.00美元，上限由78.00美元上调至80.00美元。"), "{}", section(&z, 0));
        // A wider range, and a smaller deal.
        let mut st = real_state();
        with_amendment(
            &mut st,
            "0001628280-26-080004",
            "2026-11-14",
            68.0,
            80.0,
            800e6,
        );
        let r = latest(&st, "anthropic").unwrap();
        let (d, _) = page(&bundle(&st, &anthropic(), &r, at()), Lang::En);
        assert!(
            d.title
                .starts_with("Anthropic changes IPO price range to $68 to $80"),
            "{}",
            d.title
        );
        assert!(section(&d, 0).contains("the low end fell from $70.00 to $68.00 and the high end rose from $78.00 to $80.00. The number offered changed from 825,000,000 shares to 800,000,000 shares."), "{}", section(&d, 0));
        // A partial amendment (no exchange or symbol) never becomes the
        // facts source, so it is no milestone.
        let mut st = real_state();
        with_amendment(
            &mut st,
            "0001628280-26-080005",
            "2026-11-14",
            76.0,
            80.0,
            825e6,
        );
        st.facts
            .retain(|x| !(x.accession == "0001628280-26-080005" && x.kind == "symbol"));
        assert_eq!(
            latest(&st, "anthropic").unwrap().milestone,
            Milestone::RangeSet
        );
    }

    fn priced_at(p: f64) -> IpoArticleFacts {
        let mut st = real_state();
        let fin = "0001628280-26-090001";
        st.filings.push(FilingRow {
            accession: fin.into(),
            slug: "anthropic".into(),
            form: "424B4".into(),
            filing_date: "2026-11-18".into(),
            index_url:
                "https://www.sec.gov/Archives/edgar/data/2130357/x/0001628280-26-090001-index.htm"
                    .into(),
            read_state: "read".into(),
            ..Default::default()
        });
        for (k, v) in [("price_final", p), ("shares_offered", 825e6)] {
            st.facts.push(FactRow {
                accession: fin.into(),
                kind: k.into(),
                slug: "anthropic".into(),
                value_num: Some(v),
                value_text: None,
                quote: "q".into(),
            });
        }
        let r = latest(&st, "anthropic").unwrap();
        bundle(&st, &anthropic(), &r, at())
    }

    #[test]
    fn a_price_is_placed_only_within_above_or_below_the_last_range() {
        for (p, en, zh) in [
            (74.0, "within", "处于"),
            (70.0, "within", "处于"),
            (78.0, "within", "处于"),
            (80.0, "above", "高于"),
            (65.0, "below", "低于"),
        ] {
            let f = priced_at(p);
            let (d, _) = page(&f, Lang::En);
            assert!(section(&d, 0).contains(&format!("That price is {en} the range of $70.00 to $78.00 in the S-1/A filed Nov 10, 2026.")), "{}", section(&d, 0));
            let (z, _) = page(&f, Lang::Zh);
            assert!(
                section(&z, 0).contains(&format!(
                    "该价格{zh}2026年11月10日提交的S-1/A所载区间70.00至78.00美元"
                )),
                "{}",
                section(&z, 0)
            );
        }
        let (d, md) = page(&priced_at(80.0), Lang::En);
        assert_eq!(d.title, "Anthropic prices IPO at $80 a share");
        assert!(
            md.contains("| Offering price per share | $80.00 |")
                && md.contains(
                    "At $80.00 a share, the 825,000,000 shares offered come to $66 billion."
                ),
            "{md}"
        );
    }

    #[test]
    fn a_listing_states_the_session_with_both_inputs_of_the_change() {
        let mut st = real_state();
        let fin = "0001628280-26-090001";
        st.filings.push(FilingRow {
            accession: fin.into(),
            slug: "anthropic".into(),
            form: "424B4".into(),
            filing_date: "2026-11-18".into(),
            index_url: "https://www.sec.gov/x-index.htm".into(),
            read_state: "read".into(),
            ..Default::default()
        });
        for (k, v) in [("price_final", 80.0), ("shares_offered", 825e6)] {
            st.facts.push(FactRow {
                accession: fin.into(),
                kind: k.into(),
                slug: "anthropic".into(),
                value_num: Some(v),
                value_text: None,
                quote: "q".into(),
            });
        }
        st.trading.push(TradingRow {
            slug: "anthropic".into(),
            session_date: "2026-11-19".into(),
            symbol: "ANT".into(),
            open: Some(95.0),
            close: 101.5,
            volume: Some(120_000_000),
            ipo_price: Some(80.0),
            change_pct: Some(26.88),
            source_url: "https://finance.yahoo.com/quote/ANT".into(),
            fetched_at: "x".into(),
        });
        st.watch[0].date_source = Some("trading".into());
        st.watch[0].date_status = Some("confirmed".into());
        st.watch[0].first_trade_date = Some("2026-11-19".into());
        let r = latest(&st, "anthropic").unwrap();
        assert!(r.confirmed);
        let f = bundle(&st, &anthropic(), &r, at());
        let (d, md) = page(&f, Lang::En);
        assert_eq!(d.title, "Anthropic closes first trading day at $101.50");
        assert_eq!(d.summary, "Anthropic shares closed their first trading session on Nov 19, 2026 at $101.50, up 26.88% from the $80.00 offering price.");
        assert_eq!(section(&d, 0), "Anthropic's shares (ANT) began trading on the Nasdaq Global Select Market on Nov 19, 2026. In that session the shares opened at $95.00, closed at $101.50 and 120,000,000 shares traded. The close of $101.50 is 26.88% above the offering price of $80.00.");
        assert!(
            md.contains("The close of $101.50 is 26.88% above the offering price of $80.00."),
            "{md}"
        );
        assert!(section(&d, 1).starts_with("Anthropic's final prospectus (Form 424B4) filed Nov 18, 2026 priced 825,000,000 shares at $80.00 per share."), "{}", section(&d, 1));
        let (z, _) = page(&f, Lang::Zh);
        assert_eq!(z.summary, "Anthropic于2026年11月19日完成上市首个交易日，收于101.50美元，较80.00美元的发行价上涨26.88%。");
        assert!(
            section(&z, 0).contains("当日开盘价为95.00美元，收盘价为101.50美元，成交1.2亿股。"),
            "{}",
            section(&z, 0)
        );
    }

    #[test]
    fn a_filing_and_a_withdrawal_state_the_filing_and_its_date() {
        let mut st = real_state();
        st.filings.retain(|f| f.form != "S-1/A");
        st.facts.clear();
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!(r.milestone, Milestone::Filed);
        let f = bundle(&st, &anthropic(), &r, at());
        let (d, _) = page(&f, Lang::En);
        assert_eq!(d.title, "Anthropic files S-1 for IPO");
        assert_eq!(section(&d, 0), "Anthropic filed a registration statement (Form S-1) with the SEC on Oct 26, 2026. It follows 6 confidential draft registration statements (Form DRS and DRS/A), the first submitted on Jun 1, 2026.");
        page(&f, Lang::Zh);

        let mut st = real_state();
        st.filings.push(FilingRow {
            accession: "0001628280-26-095555".into(),
            slug: "anthropic".into(),
            form: "RW".into(),
            filing_date: "2026-11-20".into(),
            index_url: "https://www.sec.gov/x-index.htm".into(),
            read_state: "none".into(),
            ..Default::default()
        });
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!(r.milestone, Milestone::Withdrawn);
        let f = bundle(&st, &anthropic(), &r, at());
        let (d, _) = page(&f, Lang::En);
        assert_eq!(d.title, "Anthropic withdraws IPO registration");
        assert_eq!(d.summary, "Anthropic filed Form RW with the SEC on Nov 20, 2026 to withdraw its registration statement.");
        assert_eq!(
            section(&d, 1),
            "That filing set a price range of $70.00 to $78.00 per share for 825,000,000 shares."
        );
        assert_eq!(
            section(&d, 0),
            "Anthropic filed a request to withdraw its registration statement (Form RW) with the SEC on Nov 20, 2026. The offering terms PreBell last read are from the S-1/A filed Nov 10, 2026."
        );
        let (z, _) = page(&f, Lang::Zh);
        assert_eq!(z.title, "Anthropic撤回IPO注册");
    }

    #[test]
    fn one_draft_is_named_in_the_singular_and_drs_a_only_when_amended() {
        let mut st = real_state();
        st.filings.retain(|f| f.form == "S-1" || f.form == "DRS");
        st.facts.clear();
        let r = latest(&st, "anthropic").unwrap();
        let f = bundle(&st, &anthropic(), &r, at());
        let (d, _) = page(&f, Lang::En);
        assert!(section(&d, 0).ends_with("It follows a confidential draft registration statement (Form DRS) submitted on Jun 1, 2026."), "{}", section(&d, 0));
        let (z, _) = page(&f, Lang::Zh);
        assert!(
            section(&z, 0).ends_with("此前公司已于2026年6月1日保密提交1份注册草案（DRS表格）。"),
            "{}",
            section(&z, 0)
        );
    }

    #[test]
    fn a_share_count_change_alone_names_the_count_not_the_range() {
        let mut st = real_state();
        with_amendment(
            &mut st,
            "0001628280-26-080006",
            "2026-11-14",
            70.0,
            78.0,
            900e6,
        );
        let r = latest(&st, "anthropic").unwrap();
        assert_eq!(r.milestone, Milestone::RangeChanged);
        let f = bundle(&st, &anthropic(), &r, at());
        let (d, _) = page(&f, Lang::En);
        assert_eq!(
            d.title,
            "Anthropic changes IPO share count to 900,000,000 at $70 to $78 a share"
        );
        assert_eq!(d.summary, "Anthropic's S-1/A filed Nov 14, 2026 keeps a price range of $70.00 to $78.00 per share and changes the number offered from 825,000,000 shares to 900,000,000 shares, against its S-1/A filed Nov 10, 2026.");
        let (z, _) = page(&f, Lang::Zh);
        assert_eq!(z.summary, "Anthropic于2026年11月14日递交S-1/A，价格区间维持每股70.00至78.00美元不变，发行数量由8.25亿股调整为9亿股（相较2026年11月10日提交的S-1/A）。");
    }

    #[test]
    fn a_foreign_issuers_sellers_follow_its_registration_form() {
        let (mut st, t) = one_cover("arm", 1_973_239, "Arm", ARM, "424B4", "2023-09-14");
        st.filings.push(FilingRow {
            accession: "0001973239-23-000002".into(),
            slug: "arm".into(),
            form: "F-1".into(),
            filing_date: "2023-08-21".into(),
            index_url:
                "https://www.sec.gov/Archives/edgar/data/1973239/x/0001973239-23-000002-index.htm"
                    .into(),
            read_state: "none".into(),
            ..Default::default()
        });
        let (_, en, _) = both(&st, &t);
        assert!(
            en.contains("including any ADSs sold by selling shareholders"),
            "{en}"
        );
        assert!(!en.contains("stockholders"), "{en}");
    }

    #[test]
    fn a_listing_without_a_final_prospectus_says_what_was_read() {
        let mut st = real_state();
        st.trading.push(TradingRow {
            slug: "anthropic".into(),
            session_date: "2026-11-19".into(),
            symbol: "ANT".into(),
            open: None,
            close: 90.0,
            volume: None,
            ipo_price: None,
            change_pct: None,
            source_url: "https://finance.yahoo.com/quote/ANT".into(),
            fetched_at: "x".into(),
        });
        st.watch[0].date_source = Some("trading".into());
        st.watch[0].date_status = Some("confirmed".into());
        st.watch[0].first_trade_date = Some("2026-11-19".into());
        let r = latest(&st, "anthropic").unwrap();
        let f = bundle(&st, &anthropic(), &r, at());
        let (d, _) = page(&f, Lang::En);
        assert!(section(&d, 1).starts_with("The offering terms PreBell last read are from the S-1/A filed Nov 10, 2026, which gave a price range of $70.00 to $78.00 per share. The offering price is added once the final prospectus is read."), "{}", section(&d, 1));
        assert_eq!(section(&d, 0), "Anthropic's shares (ANT) began trading on the Nasdaq Global Select Market on Nov 19, 2026. In that session the shares closed at $90.00.");
        let (z, _) = page(&f, Lang::Zh);
        assert!(section(&z, 1).starts_with("PreBell最近读取的发行条款来自2026年11月10日提交的S-1/A，价格区间为每股70.00至78.00美元。读取最终招股说明书后会补充发行价。"), "{}", section(&z, 1));
        assert!(
            section(&z, 0)
                .starts_with("Anthropic（ANT）于2026年11月19日在纳斯达克全球精选市场开始交易。"),
            "{}",
            section(&z, 0)
        );
    }

    #[test]
    fn every_number_and_date_outside_the_quotes_comes_from_the_bundle() {
        let f = preview_facts();
        for lang in [Lang::En, Lang::Zh] {
            let md = page(&f, lang).1;
            assert!(verify_page(&md, &f, lang).is_ok());
            // The quoted news may hold its own figures and dates.
            assert!(md.contains("$1.4 trillion") || lang == Lang::Zh);
            let planted = md.replacen("825,000,000", "826,000,000", 1);
            assert!(verify_page(&planted, &f, lang).unwrap_err()[0].contains("826,000,000"));
            let day = pick(lang, "Nov 10, 2026", "2026年11月10日");
            let other = pick(lang, "Nov 16, 2026", "2026年11月16日");
            let planted = md.replacen(day, other, 1);
            assert!(
                verify_page(&planted, &f, lang).unwrap_err()[0].contains("date"),
                "{lang:?}"
            );
        }
    }

    #[test]
    fn quoted_items_render_word_for_word_with_safe_links() {
        let mut f = preview_facts();
        f.items[0].url = "https://www.reuters.com/a (b)/c".into();
        let (_, md) = page(&f, Lang::En);
        assert!(
            md.contains("([article](https://www.reuters.com/a%20%28b%29/c))"),
            "{md}"
        );
        assert!(md.contains("- Jane Doe, an analyst at Example Securities, in Sample Times, Nov 9, 2026: \"The office tools show how far they want to push into software that businesses already pay for\""));
        let (_, zh) = page(&f, Lang::Zh);
        assert!(zh.contains("- Example News（2026年11月10日）报道：“At the top of the range the offering would value Anthropic at roughly $1.4 trillion, the people said.”"), "{zh}");
        let rows = source_rows(&preview_facts());
        assert_eq!(rows.len(), 6);
    }

    #[test]
    fn a_reported_date_is_not_quoted_once_a_filing_or_trading_date_exists() {
        let mut st = real_state();
        st.items = preview_facts()
            .items
            .iter()
            .map(|i| ItemRow {
                slug: "anthropic".into(),
                kind: i.kind.clone(),
                url: i.url.clone(),
                outlet: i.outlet.clone(),
                published_on: i.published_on.clone(),
                quote: i.quote.clone(),
                speaker: i.speaker.clone(),
                value_usd: i.value_usd,
                value_date: i.value_date.clone(),
                found_at: "2026-11-11T15:00:00Z".into(),
                ..Default::default()
            })
            .collect();
        let r = latest(&st, "anthropic").unwrap();
        assert!(bundle(&st, &anthropic(), &r, at())
            .items
            .iter()
            .any(|i| i.kind == "expected_date"));
        st.watch[0].date_source = Some("filing".into());
        assert!(!bundle(&st, &anthropic(), &r, at())
            .items
            .iter()
            .any(|i| i.kind == "expected_date"));
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
            let (d, md) = page(&f, lang);
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
}
