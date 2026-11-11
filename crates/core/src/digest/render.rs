//! The digest as Markdown (Buttondown's Markdown mode), one English email.
//! Plain on purpose: paragraphs, a few headings, lists and links, no layout.
//! Every word is in `copy`; event titles come from the events glossary,
//! company names from the watchlist, and the picks' titles and summaries
//! from the published English pages.

use chrono::{Datelike, Duration, NaiveDate};

use super::copy;
use super::{Day, Event, Slot, Week};
use crate::events::{et_time, note, title};
use crate::numbers::Lang;

pub const SITE: &str = "https://prebell.site";

/// Text safe inside a Markdown link label or list item.
/// Every `{` becomes `&#123;`, so no `{{` or `{%` reaches Buttondown's
/// template tags (the entity renders as `{`).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '{' {
            out.push_str("&#123;");
            continue;
        }
        if matches!(c, '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `escape`, for text that starts a paragraph: a leading `-`, `+` or `#`,
/// or `1.` / `1)`, would make it a list or a heading.
fn escape_block(s: &str) -> String {
    let mut out = escape(s);
    let digits = out.bytes().take_while(u8::is_ascii_digit).count();
    if out.starts_with(['-', '+', '#']) {
        out.insert(0, '\\');
    } else if digits > 0 && out[digits..].starts_with(['.', ')']) {
        out.insert(digits, '\\');
    }
    out
}

fn link(label: &str, url: &str) -> String {
    format!("[{}]({url})", escape(label))
}

/// "Nov 9".
fn month_day(d: NaiveDate) -> String {
    d.format("%b %-d").to_string()
}

/// Monday to Friday: "Nov 9–13", "Nov 30–Dec 4".
pub fn range(monday: NaiveDate) -> String {
    let friday = monday + Duration::days(4);
    let end = if friday.month() == monday.month() {
        friday.day().to_string()
    } else {
        month_day(friday)
    };
    format!("{}–{end}", month_day(monday))
}

/// "Mon, Nov 9", as the site writes a day.
fn day_label(d: NaiveDate) -> String {
    let wd = copy::WEEKDAYS[d.weekday().num_days_from_monday() as usize];
    format!("{wd}, {}", month_day(d))
}

pub fn subject(monday: NaiveDate) -> String {
    copy::SUBJECT.replace("{range}", &range(monday))
}

fn event_line(e: &Event) -> Option<String> {
    let t = title(
        &e.series,
        e.ref_period.as_deref(),
        e.variant.as_deref(),
        Lang::En,
    )?;
    let mut line = match e.starts_at {
        Some(at) => format!("{}: {}", et_time(at, Lang::En), link(&t, &e.link)),
        None => link(&t, &e.link),
    };
    if let Some(n) = note(&e.series, Lang::En) {
        line.push_str(" · ");
        line.push_str(n);
    }
    match (e.status.as_str(), &e.moved_from) {
        ("postponed", _) => line.push_str(copy::POSTPONED),
        ("moved", Some(from)) => {
            let from = from
                .parse::<NaiveDate>()
                .map(month_day)
                .unwrap_or_else(|_| escape(from));
            line.push_str(&copy::MOVED_FROM.replace("{date}", &from));
        }
        _ => {}
    }
    Some(line)
}

fn count(n: usize) -> String {
    if n == 1 {
        copy::ONE_COMPANY.to_string()
    } else {
        copy::COMPANIES.replace("{count}", &n.to_string())
    }
}

fn slot_line(label: &str, s: &Slot) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    let mut items: Vec<String> = s
        .writeups
        .iter()
        .map(|c| {
            let mut t = escape(&format!("{} ({})", c.name, c.ticker));
            if !c.confirmed {
                t.push_str(copy::UNCONFIRMED);
            }
            t
        })
        .collect();
    if items.is_empty() {
        items.push(count(s.others));
    } else if s.others > 0 {
        items.push(copy::MORE.replace("{count}", &s.others.to_string()));
    }
    Some(format!("{label}: {}", items.join(", ")))
}

fn day_block(d: &Day) -> String {
    let mut lines: Vec<String> = d.events.iter().filter_map(event_line).collect();
    let slots = [
        (copy::BMO, &d.bmo),
        (copy::AMC, &d.amc),
        (copy::TIME_UNKNOWN, &d.unknown),
    ];
    let earnings: Vec<String> = slots
        .iter()
        .filter_map(|(label, s)| slot_line(label, s))
        .collect();
    if earnings.is_empty() {
        // A closed day says so already.
        if !d.closed() {
            lines.push(copy::NO_EARNINGS.to_string());
        }
    } else {
        lines.extend(earnings);
    }
    let items: Vec<String> = lines.iter().map(|l| format!("- {l}")).collect();
    format!("### {}\n\n{}", day_label(d.date), items.join("\n"))
}

/// The email body (without Buttondown's editor-mode line): the opening
/// line, last week's picks (left out when there are none), the week ahead,
/// the question, the sign-off and the footer.
pub fn body(w: &Week) -> String {
    let range = range(w.monday);
    let opening = match w.picks.len() {
        0 => copy::OPENING_NONE.replace("{range}", &range),
        n => copy::OPENING
            .replace(
                "{picks}",
                copy::PICK_COUNTS[n.min(copy::PICK_COUNTS.len()) - 1],
            )
            .replace("{range}", &range),
    };
    let mut out = vec![opening];

    // Missing stays missing: no checks, no section.
    if !w.picks.is_empty() {
        out.push(format!("## {}", copy::PICKS_HEADING));
        for p in &w.picks {
            out.push(format!(
                "**{}**",
                link(&p.title, &format!("{SITE}/en/p/{}", p.slug))
            ));
            if !p.summary.trim().is_empty() {
                out.push(escape_block(p.summary.trim()));
            }
        }
        out.push(link(
            copy::ALL_CHECKS,
            &format!("{SITE}/en/archive?type=earnings"),
        ));
    }

    out.push(format!("## {}", copy::CALENDAR_HEADING));
    out.push(copy::TIMES_NOTE.to_string());
    out.extend(w.days.iter().map(day_block));
    out.push(link(
        copy::FULL_CALENDAR,
        &format!("{SITE}/en/earnings?week={}", w.monday.format("%Y-%m-%d")),
    ));

    out.push(copy::QUESTION.to_string());
    out.push(copy::SIGN_OFF.to_string());
    out.push("---".to_string());
    out.push(copy::FOOTER.to_string());
    out.join("\n\n") + "\n"
}
