//! The week post's page guard (`writeup::verify_page` takes IPO facts and
//! refuses every rounded amount, which every cover line is, so it cannot be
//! used). It proves that the page says what the bundle holds, and nothing
//! else; it does not prove the bundle is right (that rests on the rows'
//! stored sources: the checks' own number checks, the calendar's `source`,
//! `proof_url` and `checked_at`, the events' official links). Checked, with
//! its own tables of weekdays, slot labels and status words, so a template
//! edit cannot pass by changing both sides:
//!
//! 1. **Check lines.** A section 1 line names one listed check by its link,
//!    and every part after it is that check's period or one of its stored
//!    cover lines, character for character (as escaped for Markdown). Each
//!    listed check has exactly one line.
//! 2. **Section 2.** Five day headings, Monday to Friday, each with its own
//!    weekday; slot labels in order; every expected row exactly once, under
//!    its own date and slot, with its own status word; "no reports" only on
//!    a day without rows. The legend sentence is the calendar's, once.
//! 3. **Links.** Every link goes where the bundle says (a safe target), and
//!    its label is that row's name and ticker, an event's title, or the
//!    calendar link's text. Names with digits (3M) pass here, as whole
//!    labels. No bare URL or HTML tag.
//! 4. **Names in the title.** Check names, as whole words.
//! 5. **Counts.** Each count stands in its own phrase and must equal that
//!    count ("8 expected next week" against `counts.expected`), and a zero
//!    form ("No company is expected", 日期均未确认) only when it is zero.
//! 6. **Dates and times.** Each date is one of the bundle's, in its place:
//!    section 1's range in its heading and the summary, the as-of date in
//!    section 2's heading. Each clock time is the as-of time (or a
//!    correction's first publication) followed by "ET" / after 美东时间, or
//!    an event's time with its AM/PM (上午/下午) and "ET" / （美东）.
//!
//! Then no digit may be left. This is stricter than `numbers::check_allowed`,
//! which passes any bare integer from 0 to 10 and never looks at dates,
//! clock times or fiscal labels.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use chrono::{Datelike, Duration, NaiveDate};
use regex::Regex;

use super::render::{
    calendar_url, check_url, event_title, label_of, shown, ticker_url, ENGLISH, SEP,
};
use super::{period, Bundle, Check};
use crate::clock::et;
use crate::digest::render::escape;
use crate::ipo::writeup::{date_ok, date_re, replace_word};
use crate::numbers::Lang;

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid built-in regex")
}

fn link_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"\[((?:\\.|[^\]\\\n])*)\]\(([^)\s]+)\)"))
}

fn time_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"(?-u:\b)\d{1,2}:\d{2}(?-u:\b)"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Count {
    Checks,
    Expected,
    Confirmed,
}

/// Each count's phrases, in both languages (`render`'s templates).
fn count_res(lang: Lang) -> &'static [(Regex, Count)] {
    static EN: OnceLock<Vec<(Regex, Count)>> = OnceLock::new();
    static ZH: OnceLock<Vec<(Regex, Count)>> = OnceLock::new();
    let list = |pats: &[(&str, Count)]| pats.iter().map(|(p, c)| (re(p), *c)).collect();
    match lang {
        Lang::En => EN.get_or_init(|| {
            list(&[
                (r"(\d+) expected next week", Count::Expected),
                (r"(\d+) compan(?:y|ies) expected from", Count::Expected),
                (r"checked (\d+) earnings reports?", Count::Checks),
                (
                    r"(\d+) compan(?:y|ies) (?:is|are) expected to report",
                    Count::Expected,
                ),
                (r"(\d+) dates? (?:is|are) confirmed", Count::Confirmed),
                (r"(\d+) checks? this week", Count::Checks),
            ])
        }),
        Lang::Zh => ZH.get_or_init(|| {
            list(&[
                (r"下周预计\s?(\d+)\s?家", Count::Expected),
                (r"起预计(\d+)家", Count::Expected),
                (r"核对了(\d+)份财报", Count::Checks),
                (r"预计(\d+)家公司公布财报", Count::Expected),
                (r"其中(\d+)家日期已确认", Count::Confirmed),
                (r"本周\s?(\d+)\s?篇速评", Count::Checks),
            ])
        }),
    }
}

/// Every date the page may name.
fn dates(b: &Bundle) -> HashSet<NaiveDate> {
    let mut out: HashSet<NaiveDate> = HashSet::new();
    let (from, to) = b.range();
    // The cutoff's own date is allowed only in a correction's note, which
    // `section_two` checks in place.
    out.extend([from, to, b.monday(), b.friday(), et(b.as_of()).date_naive()]);
    for i in 0..5 {
        out.insert(b.next_monday() + Duration::days(i));
    }
    for e in &b.events {
        out.extend(e.date.parse::<NaiveDate>().ok());
        out.extend(
            e.moved_from
                .as_deref()
                .and_then(|d| d.parse::<NaiveDate>().ok()),
        );
    }
    out
}

/// What a clock time on the page is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Clock {
    /// The as-of time: 24-hour, "ET".
    Stamp,
    /// An event's time: 12-hour with AM/PM (`events::et_time`).
    Event { pm: bool },
}

/// Every clock time the page may print, and what it is.
fn times(b: &Bundle) -> HashMap<String, Clock> {
    let mut out: HashMap<String, Clock> = HashMap::new();
    // The cutoff's time is allowed only in a correction's note
    // (`section_two`, which checks it in place).
    out.insert(super::render::clock(b), Clock::Stamp);
    for e in &b.events {
        if let Some(at) = e
            .starts_at
            .as_deref()
            .and_then(|t| t.parse::<chrono::DateTime<chrono::Utc>>().ok())
        {
            let local = et(at);
            out.entry(local.format("%-I:%M").to_string())
                .or_insert(Clock::Event {
                    pm: local.format("%p").to_string() == "PM",
                });
        }
    }
    out
}

/// Whether the text around a clock time names its time zone as the bundle's
/// time implies.
fn zone_ok(line: &str, start: usize, end: usize, clock: Clock, lang: Lang) -> bool {
    let (before, after) = (&line[..start], &line[end..]);
    match (lang, clock) {
        (Lang::En, Clock::Stamp) => after.starts_with(" ET") && !after.starts_with(" ETC"),
        (Lang::En, Clock::Event { pm }) => after.starts_with(if pm { " PM ET" } else { " AM ET" }),
        (Lang::Zh, Clock::Stamp) => before.contains("美东时间"),
        (Lang::Zh, Clock::Event { pm }) => {
            before.ends_with(if pm { "下午" } else { "上午" }) && after.starts_with("（美东）")
        }
    }
}

/// `url -> labels` the page may link, and the check each check URL names.
fn links<'a>(
    b: &'a Bundle,
    lang: Lang,
) -> (HashMap<String, HashSet<String>>, HashMap<String, &'a Check>) {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    let mut checks = HashMap::new();
    for c in &b.checks {
        let url = check_url(c, lang);
        out.entry(url.clone())
            .or_default()
            .insert(label_of(&c.name, &c.ticker));
        checks.insert(url, c);
    }
    for e in &b.expected {
        out.entry(ticker_url(&e.ticker, lang))
            .or_default()
            .insert(label_of(&e.name, &e.ticker));
    }
    for e in &b.events {
        if let Some(t) = event_title(e, lang) {
            out.entry(e.link.clone()).or_default().insert(escape(&t));
        }
    }
    let text = match lang {
        Lang::En => "The calendar, updated live →",
        Lang::Zh => "实时日历 →",
    };
    out.entry(calendar_url(b, lang))
        .or_default()
        .insert(text.into());
    (out, checks)
}

/// Whether the rest of a check's line, after its link, is that check's
/// period and stored cover lines only, each at most once.
fn check_parts(rest: &str, c: &Check, lang: Lang) -> bool {
    let rest = match rest.strip_prefix(ENGLISH) {
        Some(r) if lang == Lang::Zh && c.zh_status == "failed" => r,
        Some(_) => return false,
        None => rest,
    };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix(SEP) else {
        return false;
    };
    let mut allowed: Vec<String> = c.period.iter().map(|p| period::label(p, lang)).collect();
    let lines = match lang {
        Lang::En => &c.en,
        Lang::Zh => &c.zh,
    };
    allowed.extend(lines.iter().filter(|l| shown(l, c)).map(|l| escape(l)));
    for part in rest.split(SEP) {
        match allowed.iter().position(|a| a == part) {
            Some(i) => {
                allowed.remove(i);
            }
            None => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// The guard's own tables: a template edit must change both sides to pass.

const EN_WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const ZH_WEEKDAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

/// Slot labels, in page order.
fn slot_labels(lang: Lang) -> [(&'static str, &'static str); 3] {
    match lang {
        Lang::En => [
            ("Before open", "bmo"),
            ("After close", "amc"),
            ("Time not given", "unknown"),
        ],
        Lang::Zh => [("盘前", "bmo"), ("盘后", "amc"), ("时间暂缺", "unknown")],
    }
}

/// A write-up company's status words, and a calendar-only group's.
fn status_of_word(word: &str, lang: Lang, writeup: bool) -> Option<&'static str> {
    let table: &[(&str, &str)] = match (lang, writeup) {
        (Lang::En, true) => &[
            ("confirmed", "confirmed"),
            ("date not confirmed", "unconfirmed"),
            ("reported", "reported"),
        ],
        (Lang::En, false) => &[
            ("Confirmed", "confirmed"),
            ("Not confirmed", "unconfirmed"),
            ("Reported", "reported"),
        ],
        (Lang::Zh, true) => &[
            ("日期已确认", "confirmed"),
            ("日期未确认", "unconfirmed"),
            ("已发布", "reported"),
        ],
        (Lang::Zh, false) => &[
            ("已确认", "confirmed"),
            ("未确认", "unconfirmed"),
            ("已发布", "reported"),
        ],
    };
    table.iter().find(|(w, _)| *w == word).map(|(_, s)| *s)
}

/// A day heading as the guard writes it: "Tue, Dec 8" / "12月8日（周二）".
fn day_heading(d: NaiveDate, lang: Lang) -> String {
    let i = d.weekday().num_days_from_monday() as usize;
    match lang {
        Lang::En => format!("### {}, {}", EN_WEEKDAYS[i], d.format("%b %-d")),
        Lang::Zh => format!("### {}月{}日（{}）", d.month(), d.day(), ZH_WEEKDAYS[i]),
    }
}

/// The dates on a line, in order, resolved against the bundle's dates (a
/// date the bundle does not hold is `None`).
fn dates_on(line: &str, allowed: &HashSet<NaiveDate>) -> Vec<Option<NaiveDate>> {
    date_re()
        .captures_iter(line)
        .map(|c| {
            allowed
                .iter()
                .copied()
                .find(|d| date_ok(&c, &HashSet::from([*d])))
        })
        .collect()
}

/// A link target PreBell may write: its own pages, or an https page with
/// no space, quote, bracket or parenthesis.
fn safe_target(url: &str) -> bool {
    static SITE: OnceLock<Regex> = OnceLock::new();
    static WEB: OnceLock<Regex> = OnceLock::new();
    let site = SITE.get_or_init(|| {
        re(r"^/(?:en|zh)/(?:p/[a-z0-9]+(?:-[a-z0-9]+)*|t/[A-Z0-9]+(?:[.-][A-Z0-9]+)*|earnings\?week=\d{4}-\d{2}-\d{2})$")
    });
    let web = WEB.get_or_init(|| re(r"^https://[A-Za-z0-9.-]+(?:/[A-Za-z0-9._~%/?=&+#-]*)?$"));
    site.is_match(url) || web.is_match(url)
}

/// Section 2, line by line (module docs, step 2).
fn section_two(b: &Bundle, lang: Lang, md: &str, bad: &mut Vec<String>) {
    let heading = match lang {
        Lang::En => "## Expected next week",
        Lang::Zh => "## 下周预计公布",
    };
    let lines: Vec<&str> = md.lines().collect();
    let Some(h) = lines.iter().position(|l| l.starts_with(heading)) else {
        bad.push("section 2 heading missing".into());
        return;
    };
    let colon = match lang {
        Lang::En => ": ",
        Lang::Zh => "：",
    };
    let none = match lang {
        Lang::En => "No reports expected.",
        Lang::Zh => "暂无。",
    };
    let days: Vec<NaiveDate> = (0..5)
        .map(|i| b.next_monday() + Duration::days(i))
        .collect();
    let day_keys: Vec<String> = days.iter().map(|d| super::ymd(*d)).collect();
    // Each row by (its ticker page, date, slot), and the rows per day and
    // slot: built once (a peak week has about 200 rows and 300 lines).
    let mut by_link: HashMap<(String, &str, &str), &super::Expected> = HashMap::new();
    let mut per: HashMap<(&str, &str), usize> = HashMap::new();
    for e in &b.expected {
        let key = (
            super::render::ticker_url(&e.ticker, lang),
            e.date.as_str(),
            e.slot.as_str(),
        );
        by_link.insert(key, e);
        *per.entry((e.date.as_str(), e.slot.as_str())).or_default() += 1;
    }
    let mut seen: HashMap<(&str, &str), usize> = HashMap::new();
    let mut day: Option<usize> = None;
    let mut slot: Option<usize> = None;
    let mut note = false;
    let rows_of = |i: usize, sl: Option<&str>| -> usize {
        slot_labels(lang)
            .iter()
            .filter(|(_, code)| sl.is_none_or(|s| s == *code))
            .map(|(_, code)| {
                per.get(&(day_keys[i].as_str(), *code))
                    .copied()
                    .unwrap_or(0)
            })
            .sum()
    };
    for line in &lines[h + 1..] {
        let l = line.trim_end();
        if l.is_empty() {
            continue;
        }
        if l.starts_with("**Economy & Fed:**")
            || l.starts_with("**经济与美联储**")
            || l == super::render::LEGEND_EN
            || l == super::render::LEGEND_ZH
            || l.starts_with("## ")
        {
            break;
        }
        if l.starts_with("### ") {
            let i = day.map_or(0, |i| i + 1);
            match days.get(i) {
                Some(d) if l == day_heading(*d, lang) => {}
                _ => bad.push(format!("section 2: day heading not the bundle's: {l}")),
            }
            day = Some(i);
            slot = None;
            continue;
        }
        let Some(i) = day.filter(|i| *i < 5) else {
            // Only a correction's note may stand between the heading and Monday.
            let marker = match lang {
                Lang::En => "This list was read again on ",
                Lang::Zh => "本列表于美东时间",
            };
            if b.revision > 0 && !note && l.starts_with(marker) {
                note = true;
                // Its two stamps, in order: the re-read (as_of), then the
                // first publication (cutoff).
                let first: Option<chrono::DateTime<chrono::Utc>> = b.cutoff.parse().ok();
                let mut both = dates(b);
                both.extend(first.map(|t| et(t).date_naive()));
                let want_dates = vec![
                    Some(et(b.as_of()).date_naive()),
                    first.map(|t| et(t).date_naive()),
                ];
                let want_times = vec![
                    super::render::clock(b),
                    first.map(super::render::hhmm).unwrap_or_default(),
                ];
                let got_times: Vec<String> = time_re()
                    .find_iter(l)
                    .map(|m| m.as_str().to_string())
                    .collect();
                if dates_on(l, &both) != want_dates || got_times != want_times {
                    bad.push(format!(
                        "correction note: stamps not the bundle's, in order: {l}"
                    ));
                }
                let zone = match lang {
                    Lang::En => l.matches(" ET").count() == 2,
                    Lang::Zh => l.matches("美东时间").count() == 2,
                };
                if !zone {
                    bad.push(format!("correction note: time zones not the bundle's: {l}"));
                }
            } else {
                bad.push(format!("section 2 line not as the bundle: {l}"));
            }
            continue;
        };
        let d = days[i];
        if l == none {
            if rows_of(i, None) != 0 || slot.is_some() {
                bad.push(format!("section 2: \"{none}\" on {d}, which has reports"));
            }
            continue;
        }
        if let Some(label) = l.strip_prefix("**").and_then(|x| x.strip_suffix("**")) {
            match slot_labels(lang).iter().position(|(t, _)| *t == label) {
                Some(k)
                    if slot.is_none_or(|s| k > s)
                        && rows_of(i, Some(slot_labels(lang)[k].1)) > 0 =>
                {
                    slot = Some(k)
                }
                _ => bad.push(format!("section 2: slot label not the bundle's: {l}")),
            }
            continue;
        }
        let (Some(k), Some(item)) = (slot, l.strip_prefix("- ")) else {
            bad.push(format!("section 2 line not as the bundle: {l}"));
            continue;
        };
        let slot_code = slot_labels(lang)[k].1;
        // A write-up company's line: one link, then its status word.
        let (links, word, writeup): (&str, &str, bool) = if item.starts_with('[') {
            match item.rsplit_once(colon) {
                Some((link, w)) if link.ends_with(')') => (link, w, true),
                _ => {
                    bad.push(format!("section 2 line not as the bundle: {l}"));
                    continue;
                }
            }
        } else {
            match item.split_once(colon) {
                Some((w, rest)) => (rest, w, false),
                None => {
                    bad.push(format!("section 2 line not as the bundle: {l}"));
                    continue;
                }
            }
        };
        let Some(status) = status_of_word(word, lang, writeup) else {
            bad.push(format!("section 2: status word not the bundle's: {l}"));
            continue;
        };
        let join = match lang {
            Lang::En => ", ",
            Lang::Zh => "、",
        };
        let found: Vec<_> = link_re().captures_iter(links).collect();
        let rest = link_re().replace_all(links, "");
        if found.is_empty() || !rest.split(join).all(str::is_empty) || (writeup && found.len() != 1)
        {
            bad.push(format!("section 2 line not as the bundle: {l}"));
            continue;
        }
        for c in found {
            let url = &c[2];
            let row = by_link
                .get(&(url.to_string(), day_keys[i].as_str(), slot_code))
                .copied();
            match row {
                Some(e) if e.status == status && (e.tier == "full") == writeup => {
                    *seen
                        .entry((e.ticker.as_str(), e.date.as_str()))
                        .or_default() += 1;
                }
                Some(_) => bad.push(format!("section 2: status or tier not the bundle's: {l}")),
                None => bad.push(format!(
                    "section 2: {url} is not expected on {d} in that slot"
                )),
            }
        }
    }
    // Every correction says it re-read the list; a first build never does.
    if b.revision > 0 && b.as_of != b.cutoff && !note {
        bad.push("correction note missing".into());
    }
    if day != Some(4) {
        bad.push("section 2: not five day headings".into());
    }
    for e in &b.expected {
        let n = seen
            .get(&(e.ticker.as_str(), e.date.as_str()))
            .copied()
            .unwrap_or(0);
        if n != 1 {
            bad.push(format!(
                "section 2: {} on {} listed {n} times",
                e.ticker, e.date
            ));
        }
    }
}

/// The dates of the title, the summary and both section headings are the
/// bundle's dates for those places, in order.
fn date_places(b: &Bundle, lang: Lang, md: &str, bad: &mut Vec<String>) {
    let allowed = dates(b);
    let (from, to) = b.range();
    let nm = b.next_monday();
    let nf = nm + Duration::days(4);
    let as_of = et(b.as_of()).date_naive();
    let long = as_of != b.friday();
    let mut lines = md.lines().filter(|l| !l.trim().is_empty());
    let title = lines.next().unwrap_or_default();
    let summary = lines.next().unwrap_or_default();
    let want_title: Vec<NaiveDate> = if b.counts.checks == 0 {
        vec![nm]
    } else {
        vec![]
    };
    let mut want_summary = vec![from, to];
    match lang {
        Lang::En => {
            want_summary.extend([nm, nf]);
            if long {
                want_summary.push(as_of);
            }
        }
        Lang::Zh => {
            if long {
                want_summary.push(as_of);
            }
            want_summary.extend([nm, nf]);
        }
    }
    let s1 = md
        .lines()
        .find(|l| l.starts_with("## "))
        .unwrap_or_default();
    let s2 = md
        .lines()
        .find(|l| l.starts_with("## Expected next week") || l.starts_with("## 下周预计公布"))
        .unwrap_or_default();
    for (what, line, want) in [
        ("title", title, want_title),
        ("summary", summary, want_summary),
        ("section 1 heading", s1, vec![from, to]),
        ("section 2 heading", s2, vec![as_of]),
    ] {
        let got = dates_on(line, &allowed);
        if got != want.iter().map(|d| Some(*d)).collect::<Vec<_>>() {
            bad.push(format!("{what}: dates not the bundle's: {line}"));
        }
    }
    // The summary and the next-week heading carry as_of's time, once.
    let clock = super::render::clock(b);
    for (what, line) in [("summary", summary), ("section 2 heading", s2)] {
        let got: Vec<&str> = time_re().find_iter(line).map(|m| m.as_str()).collect();
        if got != [clock.as_str()] {
            bad.push(format!("{what}: time not the as-of time: {line}"));
        }
    }
}

/// A zero form for a count that is not zero.
fn zero_forms(b: &Bundle, lang: Lang, text: &str, bad: &mut Vec<String>) {
    let forms: &[(&str, Count)] = match lang {
        Lang::En => &[
            ("No company is expected", Count::Expected),
            ("no company expected from", Count::Expected),
            ("None expected next week", Count::Expected),
            ("no date is confirmed", Count::Confirmed),
            ("published no earnings checks", Count::Checks),
            ("No checks this week", Count::Checks),
            ("No earnings checks were published", Count::Checks),
        ],
        Lang::Zh => &[
            ("暂无公司预计", Count::Expected),
            ("起暂无公司预计", Count::Expected),
            ("下周暂无预计", Count::Expected),
            ("公布财报，日期均未确认", Count::Confirmed),
            ("公布财报，日期未确认", Count::Confirmed),
            ("未发布财报速评", Count::Checks),
            ("本周暂无速评", Count::Checks),
            ("暂无财报速评", Count::Checks),
        ],
    };
    for (form, which) in forms {
        let n = match which {
            Count::Checks => b.counts.checks,
            Count::Expected => b.counts.expected,
            Count::Confirmed => b.counts.confirmed,
        };
        if n != 0 && text.contains(form) {
            bad.push(format!("zero form with a count of {n}: {form}"));
        }
    }
    // 均 ("all") only for more than one company.
    if lang == Lang::Zh && b.counts.expected == 1 && text.contains("日期均未确认") {
        bad.push("均 with one company".into());
    }
}

/// Checks the page `md` and the two cover lines against the bundle (module
/// docs). A failure lists what was not accounted for.
pub fn verify_page(
    b: &Bundle,
    lang: Lang,
    md: &str,
    cover: &[String; 2],
) -> Result<(), Vec<String>> {
    let mut bad: Vec<String> = Vec::new();
    let (allowed_links, check_urls) = links(b, lang);
    section_two(b, lang, md, &mut bad);
    date_places(b, lang, md, &mut bad);
    let legend = match lang {
        Lang::En => super::render::LEGEND_EN,
        Lang::Zh => super::render::LEGEND_ZH,
    };
    let n = md.lines().filter(|l| *l == legend).count();
    if n != 1 {
        bad.push(format!("the calendar's source sentence appears {n} times"));
    }
    let all = format!("{md}\n{}", cover.join("\n"));
    zero_forms(b, lang, &all, &mut bad);
    for c in link_re().captures_iter(md) {
        if !safe_target(&c[2]) {
            bad.push(format!("link target not safe: {}", &c[2]));
        }
    }

    // 1. Check lines.
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut lines: Vec<String> = Vec::new();
    let note_marker = match lang {
        Lang::En => "This list was read again on ",
        Lang::Zh => "本列表于美东时间",
    };
    for line in md.lines().chain(cover.iter().map(String::as_str)) {
        // A correction's note is checked in place (`section_two`).
        if b.revision > 0 && line.starts_with(note_marker) {
            lines.push(String::new());
            continue;
        }
        let Some(m) = line.strip_prefix("- ").and_then(|l| {
            link_re()
                .captures(l)
                .filter(|c| c.get(0).is_some_and(|g| g.start() == 0))
        }) else {
            lines.push(line.to_string());
            continue;
        };
        let Some(c) = check_urls.get(&m[2]) else {
            lines.push(line.to_string());
            continue;
        };
        let rest = &line[2 + m[0].len()..];
        if m[1] != label_of(&c.name, &c.ticker) || !check_parts(rest, c, lang) {
            bad.push(format!("check line not as stored: {line}"));
        }
        *seen.entry(c.slug.as_str()).or_default() += 1;
        lines.push(String::new());
    }
    for c in &b.checks {
        let n = seen.get(c.slug.as_str()).copied().unwrap_or(0);
        if n != 1 {
            bad.push(format!("check {} listed {n} times", c.slug));
        }
    }

    // 2. Links.
    let mut text = lines.join("\n");
    for m in link_re().captures_iter(&text.clone()) {
        match allowed_links.get(&m[2]) {
            None => bad.push(format!("link not in the bundle: {}", &m[2])),
            Some(labels) if !labels.contains(&m[1]) => {
                bad.push(format!("link label not in the bundle: {}", &m[0]))
            }
            Some(_) => {}
        }
    }
    text = link_re().replace_all(&text, " ").into_owned();
    // No bare URL or HTML outside the links (names and lines are escaped).
    for line in text.lines() {
        let raw_tag = line
            .char_indices()
            .any(|(i, c)| matches!(c, '<' | '>') && !line[..i].ends_with('\\'));
        if line.contains("://") || line.contains("www.") || raw_tag {
            bad.push(format!("bare link or tag: {}", line.trim()));
        }
    }

    // 3. Names in the title (the first line).
    if let Some((head, tail)) = text.split_once('\n') {
        let mut head = head.to_string();
        let mut names: Vec<&str> = b.checks.iter().map(|c| c.name.as_str()).collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        for n in names {
            head = replace_word(&head, n, " ");
        }
        text = format!("{head}\n{tail}");
    }

    // 4. Counts, each in its own phrase.
    for (r, which) in count_res(lang) {
        let want = match which {
            Count::Checks => b.counts.checks,
            Count::Expected => b.counts.expected,
            Count::Confirmed => b.counts.confirmed,
        };
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for c in r.captures_iter(&text) {
            let g = c.get(1).expect("one group");
            if g.as_str().parse::<usize>().ok() != Some(want) {
                bad.push(format!("count not in the bundle: {}", &c[0]));
            }
            spans.push((g.start(), g.end()));
        }
        for (s, e) in spans.into_iter().rev() {
            text.replace_range(s..e, " ");
        }
    }

    // 5. Dates and times.
    let allowed = dates(b);
    for c in date_re().captures_iter(&text) {
        if !date_ok(&c, &allowed) {
            bad.push(format!("date not in the bundle: {}", &c[0]));
        }
    }
    text = date_re().replace_all(&text, " ").into_owned();
    let clocks = times(b);
    for line in text.lines() {
        for m in time_re().find_iter(line) {
            match clocks.get(m.as_str()) {
                None => bad.push(format!("time not in the bundle: {}", m.as_str())),
                Some(c) if !zone_ok(line, m.start(), m.end(), *c, lang) => {
                    bad.push(format!("time zone not the bundle's: {}", line.trim()))
                }
                Some(_) => {}
            }
        }
    }
    text = time_re().replace_all(&text, " ").into_owned();

    // Nothing else may hold a digit.
    for line in text
        .lines()
        .filter(|l| l.bytes().any(|c| c.is_ascii_digit()))
    {
        bad.push(format!("number not in the bundle: {}", line.trim()));
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}
