//! The week post's page guard (`writeup::verify_page` takes IPO facts and
//! refuses every rounded amount, which every cover line is, so it cannot be
//! used). It proves the page is the bundle, copied exactly: every digit on
//! the page, the cover lines included, must be accounted for, in order:
//!
//! 1. **Check lines.** A section 1 line names one listed check by its link,
//!    and every part after it is that check's period or one of its stored
//!    cover lines, character for character (as escaped for Markdown). Each
//!    listed check has exactly one line.
//! 2. **Links.** Every link goes where the bundle says, and its label is that
//!    row's name and ticker, an event's title, or the calendar link's text.
//!    Names with digits (3M) pass here, as whole labels.
//! 3. **Names in the title.** Check names, as whole words.
//! 4. **Counts.** Each count stands in its own phrase and must equal that
//!    count: "8 expected next week" is checked against `counts.expected`,
//!    not against any number the bundle holds, so an off-by-one cannot hide
//!    behind another field.
//! 5. **Dates and times.** Each date is one of the bundle's, each clock time
//!    the as-of time or an event's.
//!
//! Then no digit may be left. This is stricter than `numbers::check_allowed`,
//! which passes any bare integer from 0 to 10 and never looks at dates,
//! clock times or fiscal labels.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use chrono::{Duration, NaiveDate};
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
    out.extend([from, to, b.monday(), b.friday(), et(b.as_of()).date_naive()]);
    out.extend(b.cutoff.parse().ok().map(|t| et(t).date_naive()));
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

/// Every clock time the page may print: the as-of time, and each event's
/// time as `events::et_time` writes it ("8:30", "14:00" never: 12-hour).
fn times(b: &Bundle) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    out.insert(super::render::clock(b));
    // A correction's note names when the post first went out.
    if let Ok(t) = b.cutoff.parse() {
        out.insert(super::render::hhmm(t));
    }
    for e in &b.events {
        if let Some(at) = e.starts_at.as_deref().and_then(|t| t.parse().ok()) {
            out.insert(et(at).format("%-I:%M").to_string());
        }
    }
    out
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
    allowed.extend(lines.iter().filter(|l| shown(l)).map(|l| escape(l)));
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

    // 1. Check lines.
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut lines: Vec<String> = Vec::new();
    for line in md.lines().chain(cover.iter().map(String::as_str)) {
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
    for m in time_re().find_iter(&text) {
        if !clocks.contains(m.as_str()) {
            bad.push(format!("time not in the bundle: {}", m.as_str()));
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
