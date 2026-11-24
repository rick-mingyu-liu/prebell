//! The week post's text, one template per language, filled from the bundle
//! only (no model, and Chinese is not translated from the English). Every
//! date comes from the bundle's `friday`, `next_monday`, `window` and
//! `as_of`, never from the build day.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};

use super::period::label;
use super::{Bundle, Check, Expected, WeekEvent};
use crate::clock::et;
use crate::digest::render::escape;
use crate::events::{et_time, title};
use crate::numbers::Lang;
use crate::premarket::draft::Draft;

/// The calendar's own source sentence (`earnings.legend` in
/// `apps/web/messages/*.json`, without its tier sentence; a test keeps them
/// equal).
pub const LEGEND_EN: &str = "Confirmed: the company announced the date. Unconfirmed: the date comes from an aggregator and may change. Reported: the results are filed with the SEC.";
pub const LEGEND_ZH: &str =
    "已确认：公司已公布日期。未确认：日期来自第三方汇总，可能变动。已发布：业绩已向美国证监会提交。";

/// The closing paragraph, which every generator writes last (the site sets
/// it apart: `splitTail` in `apps/web/src/lib/trim.ts`).
pub const DISCLAIMER_EN: &str = "This weekly post is generated automatically from PreBell's earnings checks and its earnings calendar, for information only. It is not investment advice.";
pub const DISCLAIMER_ZH: &str =
    "本周报根据盘前铃财报速评与财报日历自动生成，仅供参考，不构成投资建议。";

const WEEKDAYS_EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAYS_ZH: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

fn pick<'a>(lang: Lang, en: &'a str, zh: &'a str) -> &'a str {
    match lang {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

fn code(lang: Lang) -> &'static str {
    pick(lang, "en", "zh")
}

fn wd(d: NaiveDate, lang: Lang) -> &'static str {
    let i = d.weekday().num_days_from_monday() as usize;
    match lang {
        Lang::En => WEEKDAYS_EN[i],
        Lang::Zh => WEEKDAYS_ZH[i],
    }
}

/// "Nov 30" / "11月30日".
pub fn date(d: NaiveDate, lang: Lang) -> String {
    match lang {
        Lang::En => d.format("%b %-d").to_string(),
        Lang::Zh => format!("{}月{}日", d.month(), d.day()),
    }
}

/// "Nov 30–Dec 4", "Dec 7–Dec 11" / "11月30日至12月4日": both ends always
/// carry their month, so every number on the page is part of a date.
pub fn range(from: NaiveDate, to: NaiveDate, lang: Lang) -> String {
    match lang {
        Lang::En => format!("{}–{}", date(from, lang), date(to, lang)),
        Lang::Zh => format!("{}至{}", date(from, lang), date(to, lang)),
    }
}

/// "Mon, Dec 7" / "12月7日（周一）".
fn day_label(d: NaiveDate, lang: Lang) -> String {
    match lang {
        Lang::En => format!("{}, {}", wd(d, lang), date(d, lang)),
        Lang::Zh => format!("{}（{}）", date(d, lang), wd(d, lang)),
    }
}

/// The as-of time in ET, 24-hour: "18:05".
pub fn clock(b: &Bundle) -> String {
    hhmm(b.as_of())
}

fn as_of_day(b: &Bundle) -> NaiveDate {
    et(b.as_of()).date_naive()
}

/// An ET time, 24-hour: "18:05".
pub fn hhmm(t: DateTime<Utc>) -> String {
    let t = et(t);
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// "Fri Dec 4, 18:05 ET" / "12月4日周五18:05", for any time.
fn stamp(t: DateTime<Utc>, lang: Lang) -> String {
    let d = et(t).date_naive();
    match lang {
        Lang::En => format!("{} {}, {} ET", wd(d, lang), date(d, lang), hhmm(t)),
        Lang::Zh => format!("{}{}{}", date(d, lang), wd(d, lang), hhmm(t)),
    }
}

/// "Fri Dec 4, 18:05 ET" / "12月4日周五18:05".
fn as_of_long(b: &Bundle, lang: Lang) -> String {
    stamp(b.as_of(), lang)
}

/// The cutoff, parsed.
fn cutoff(b: &Bundle) -> DateTime<Utc> {
    b.cutoff.parse().unwrap_or_default()
}

/// A correction says that next week's list was read again, and when the
/// post first went out; `None` for the first build.
fn reread_note(b: &Bundle, lang: Lang) -> Option<String> {
    if b.revision == 0 || b.as_of == b.cutoff {
        return None;
    }
    let (now, first) = (stamp(b.as_of(), lang), stamp(cutoff(b), lang));
    Some(match lang {
        Lang::En => format!(
            "This list was read again on {now}, after the post was first published on {first}. The checks above are the ones listed then."
        ),
        Lang::Zh => format!(
            "本列表于美东时间{now}重新读取；本文首发于美东时间{first}。上方的速评仍为首发时所列。"
        ),
    })
}

/// The summary's as-of: "Friday 18:05 ET" on the Friday, else the long form.
fn as_of_short(b: &Bundle, lang: Lang) -> String {
    if as_of_day(b) != b.friday() {
        return as_of_long(b, lang);
    }
    match lang {
        Lang::En => format!("Friday {} ET", clock(b)),
        Lang::Zh => format!("周五{}", clock(b)),
    }
}

fn next_range(b: &Bundle, lang: Lang) -> String {
    let m = b.next_monday();
    range(m, m + Duration::days(4), lang)
}

/// "Adobe, Broadcom, Costco", "Salesforce and Marvell", "Marvell" / with
/// "、" and "和": the largest checks, at most three.
fn names(b: &Bundle, lang: Lang) -> String {
    let n: Vec<&str> = b.checks.iter().take(3).map(|c| c.name.as_str()).collect();
    match (lang, n.len()) {
        (_, 1) => n[0].to_string(),
        (Lang::En, 2) => format!("{} and {}", n[0], n[1]),
        (Lang::Zh, 2) => format!("{}和{}", n[0], n[1]),
        (Lang::En, _) => n.join(", "),
        (Lang::Zh, _) => n.join("、"),
    }
}

pub fn title_of(b: &Bundle, lang: Lang) -> String {
    let c = &b.counts;
    if c.checks == 0 {
        let from = date(b.next_monday(), lang);
        return match lang {
            Lang::En if c.expected == 1 => {
                format!("Earnings next week: 1 company expected from {from}")
            }
            Lang::En => format!(
                "Earnings next week: {} companies expected from {from}",
                c.expected
            ),
            Lang::Zh => format!("下周财报：{from}起预计{}家", c.expected),
        };
    }
    let names = names(b, lang);
    match (lang, c.expected) {
        (Lang::En, 0) => format!("Earnings this week: {names}"),
        (Lang::En, n) => format!("Earnings this week: {names}; {n} expected next week"),
        (Lang::Zh, 0) => format!("本周财报：{names}"),
        (Lang::Zh, n) => format!("本周财报：{names}；下周预计{n}家"),
    }
}

pub fn summary_of(b: &Bundle, lang: Lang) -> String {
    let c = &b.counts;
    let (from, to) = b.range();
    let this = range(from, to, lang);
    let next = next_range(b, lang);
    let at = as_of_short(b, lang);
    match lang {
        Lang::En => {
            let first = match c.checks {
                0 => format!("PreBell published no earnings checks for {this}."),
                1 => format!("PreBell checked 1 earnings report published {this}."),
                n => format!("PreBell checked {n} earnings reports published {this}."),
            };
            let second = match c.expected {
                0 => format!("No company is expected to report {next}, as of {at}."),
                n => {
                    let who = if n == 1 {
                        "1 company is".to_string()
                    } else {
                        format!("{n} companies are")
                    };
                    let conf = match c.confirmed {
                        0 => "no date is confirmed".to_string(),
                        1 => "1 date is confirmed".to_string(),
                        k => format!("{k} dates are confirmed"),
                    };
                    format!("{who} expected to report {next}, as of {at}; {conf}.")
                }
            };
            format!("{first} {second}")
        }
        Lang::Zh => {
            let first = match c.checks {
                0 => format!("盘前铃{this}未发布财报速评。"),
                n => format!("盘前铃本周核对了{n}份财报（{this}发布）。"),
            };
            let second = match c.expected {
                0 => format!("截至美东时间{at}，下周（{next}）暂无公司预计公布财报。"),
                n => {
                    let conf = match c.confirmed {
                        0 => "日期均未确认".to_string(),
                        k => format!("其中{k}家日期已确认"),
                    };
                    format!("截至美东时间{at}，下周（{next}）预计{n}家公司公布财报，{conf}。")
                }
            };
            format!("{first}{second}")
        }
    }
}

/// The two cover lines: "3 checks this week", "8 expected next week" /
/// "本周 3 篇速评", "下周预计 8 家".
pub fn cover_lines(b: &Bundle, lang: Lang) -> [String; 2] {
    let c = &b.counts;
    match lang {
        Lang::En => [
            match c.checks {
                0 => "No checks this week".to_string(),
                1 => "1 check this week".to_string(),
                n => format!("{n} checks this week"),
            },
            match c.expected {
                0 => "None expected next week".to_string(),
                n => format!("{n} expected next week"),
            },
        ],
        Lang::Zh => [
            match c.checks {
                0 => "本周暂无速评".to_string(),
                n => format!("本周 {n} 篇速评"),
            },
            match c.expected {
                0 => "下周暂无预计".to_string(),
                n => format!("下周预计 {n} 家"),
            },
        ],
    }
}

/// A company's link label: "Adobe (ADBE)".
pub fn label_of(name: &str, ticker: &str) -> String {
    escape(&format!("{name} ({ticker})"))
}

/// Where a check's line links: its page in the post's language, or the
/// English page when the Chinese translation failed (the Chinese URL of a
/// failed translation redirects home).
pub fn check_url(c: &Check, lang: Lang) -> String {
    let l = if lang == Lang::Zh && c.zh_status == "failed" {
        "en"
    } else {
        code(lang)
    };
    format!("/{l}/p/{}", c.slug)
}

pub fn ticker_url(ticker: &str, lang: Lang) -> String {
    format!("/{}/t/{ticker}", code(lang))
}

pub fn calendar_url(b: &Bundle, lang: Lang) -> String {
    format!("/{}/earnings?week={}", code(lang), b.next_monday)
}

/// The marker after a Chinese line that links to the English check.
pub const ENGLISH: &str = "（英文）";
/// Between a check line's parts.
pub const SEP: &str = " · ";

/// A cover line shows when it holds a digit ("American International Group
/// results" does not).
pub fn shown(line: &str) -> bool {
    line.bytes().any(|b| b.is_ascii_digit())
}

fn lines_of(c: &Check, lang: Lang) -> &[String; 2] {
    match lang {
        Lang::En => &c.en,
        Lang::Zh => &c.zh,
    }
}

/// One check: "- [Salesforce (CRM)](/en/p/…) · Q3 fiscal 2027 · Revenue
/// $10.4B · GAAP EPS $1.23".
fn check_line(c: &Check, lang: Lang) -> String {
    let mut s = format!(
        "- [{}]({})",
        label_of(&c.name, &c.ticker),
        check_url(c, lang)
    );
    if lang == Lang::Zh && c.zh_status == "failed" {
        s.push_str(ENGLISH);
    }
    if let Some(p) = &c.period {
        s.push_str(SEP);
        s.push_str(&label(p, lang));
    }
    for l in lines_of(c, lang).iter().filter(|l| shown(l)) {
        s.push_str(SEP);
        s.push_str(&escape(l));
    }
    s
}

fn link_of(e: &Expected, lang: Lang) -> String {
    format!(
        "[{}]({})",
        label_of(&e.name, &e.ticker),
        ticker_url(&e.ticker, lang)
    )
}

fn status_word(status: &str, lang: Lang) -> &'static str {
    match (status, lang) {
        ("confirmed", Lang::En) => "confirmed",
        ("reported", Lang::En) => "reported",
        (_, Lang::En) => "date not confirmed",
        ("confirmed", Lang::Zh) => "日期已确认",
        ("reported", Lang::Zh) => "已发布",
        (_, Lang::Zh) => "日期未确认",
    }
}

fn group_word(status: &str, lang: Lang) -> &'static str {
    match (status, lang) {
        ("confirmed", Lang::En) => "Confirmed",
        ("reported", Lang::En) => "Reported",
        (_, Lang::En) => "Not confirmed",
        ("confirmed", Lang::Zh) => "已确认",
        ("reported", Lang::Zh) => "已发布",
        (_, Lang::Zh) => "未确认",
    }
}

const SLOTS: [(&str, &str, &str); 3] = [
    ("bmo", "Before open", "盘前"),
    ("amc", "After close", "盘后"),
    ("unknown", "Time n/a", "时间暂缺"),
];

/// One slot: write-up companies first, one line each with their status in
/// words; then one line per status for the calendar-only companies, so a
/// name is never more than a line from its status.
fn slot_block(rows: &[&Expected], label: &str, lang: Lang) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let mut out = vec![format!("**{label}**"), String::new()];
    let colon = pick(lang, ": ", "：");
    for e in rows.iter().filter(|e| e.tier == "full") {
        out.push(format!(
            "- {}{colon}{}",
            link_of(e, lang),
            status_word(&e.status, lang)
        ));
    }
    for status in ["confirmed", "unconfirmed", "reported"] {
        let group: Vec<String> = rows
            .iter()
            .filter(|e| e.tier != "full" && e.status == status)
            .map(|e| link_of(e, lang))
            .collect();
        if !group.is_empty() {
            let join = pick(lang, ", ", "、");
            out.push(format!(
                "- {}{colon}{}",
                group_word(status, lang),
                group.join(join)
            ));
        }
    }
    Some(out.join("\n"))
}

fn day_block(b: &Bundle, d: NaiveDate, lang: Lang) -> String {
    let rows: Vec<&Expected> = b
        .expected
        .iter()
        .filter(|e| e.date == super::ymd(d))
        .collect();
    let blocks: Vec<String> = SLOTS
        .iter()
        .filter_map(|(slot, en, zh)| {
            let in_slot: Vec<&Expected> =
                rows.iter().copied().filter(|e| e.slot == *slot).collect();
            slot_block(&in_slot, pick(lang, en, zh), lang)
        })
        .collect();
    let body = if blocks.is_empty() {
        pick(lang, "No reports expected.", "暂无。").to_string()
    } else {
        blocks.join("\n\n")
    };
    format!("### {}\n\n{body}", day_label(d, lang))
}

/// An event's title as the brief and the digest write it.
pub fn event_title(e: &WeekEvent, lang: Lang) -> Option<String> {
    title(
        &e.series,
        e.ref_period.as_deref(),
        e.variant.as_deref(),
        lang,
    )
}

fn event_item(e: &WeekEvent, lang: Lang) -> Option<String> {
    let t = event_title(e, lang)?;
    let d: NaiveDate = e.date.parse().ok()?;
    // "Wed, Dec 9, 8:30 AM ET: [November CPI](…)" / "12月9日（周三）上午8:30（美东）：[…](…)".
    let mut when = day_label(d, lang);
    if let Some(at) = e.starts_at.as_deref().and_then(|t| t.parse().ok()) {
        when.push_str(pick(lang, ", ", ""));
        when.push_str(&et_time(at, lang));
    }
    let mut s = format!(
        "{when}{}[{}]({})",
        pick(lang, ": ", "："),
        escape(&t),
        e.link
    );
    match (e.status.as_str(), e.moved_from.as_deref()) {
        ("postponed", _) => s.push_str(pick(lang, " (postponed)", "（推迟）")),
        ("moved", Some(from)) => {
            if let Ok(f) = from.parse::<NaiveDate>() {
                match lang {
                    Lang::En => s.push_str(&format!(" (moved from {})", date(f, lang))),
                    Lang::Zh => s.push_str(&format!("（由{}改期）", date(f, lang))),
                }
            }
        }
        _ => {}
    }
    Some(s)
}

/// The page, as Markdown: title, summary, the three sections.
pub fn body(b: &Bundle, lang: Lang, title: &str, summary: &str) -> String {
    let (from, to) = b.range();
    let mut out = vec![format!("# {title}"), summary.to_string()];
    out.push(match lang {
        Lang::En => format!("## Earnings checks published {}", range(from, to, lang)),
        Lang::Zh => format!("## {}发布的财报速评", range(from, to, lang)),
    });
    if b.checks.is_empty() {
        out.push(
            pick(
                lang,
                "No earnings checks were published.",
                "本周暂无财报速评。",
            )
            .into(),
        );
    } else {
        out.push(
            b.checks
                .iter()
                .map(|c| check_line(c, lang))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    out.push(match lang {
        Lang::En => format!("## Expected next week, as of {}", as_of_long(b, lang)),
        Lang::Zh => format!("## 下周预计公布（截至美东时间{}）", as_of_long(b, lang)),
    });
    if let Some(note) = reread_note(b, lang) {
        out.push(note);
    }
    let m = b.next_monday();
    for i in 0..5 {
        out.push(day_block(b, m + Duration::days(i), lang));
    }
    let events: Vec<String> = b
        .events
        .iter()
        .filter_map(|e| event_item(e, lang))
        .collect();
    if !events.is_empty() {
        out.push(match lang {
            Lang::En => format!("**Economy & Fed:** {}", events.join("; ")),
            // The colon outside the bold: CommonMark does not close "**" right
            // after full-width punctuation when a digit follows.
            Lang::Zh => format!("**经济与美联储**：{}", events.join("；")),
        });
    }
    out.push(pick(lang, LEGEND_EN, LEGEND_ZH).to_string());
    out.push(format!(
        "[{}]({})",
        pick(lang, "The calendar, updated live →", "实时日历 →"),
        calendar_url(b, lang)
    ));
    out.push(pick(lang, DISCLAIMER_EN, DISCLAIMER_ZH).to_string());
    out.join("\n\n") + "\n"
}

/// The draft and its page for one language (not yet checked).
pub fn write(b: &Bundle, lang: Lang) -> (Draft, String) {
    let title = title_of(b, lang);
    let summary = summary_of(b, lang);
    let md = body(b, lang, &title, &summary);
    (
        Draft {
            lang,
            title,
            summary,
            sections: vec![],
        },
        md,
    )
}
