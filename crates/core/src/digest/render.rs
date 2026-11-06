//! The digest as Markdown (Buttondown's default), one email per language.
//! Layout only: every word is in `copy`, event titles come from the events
//! glossary and company names from the watchlist.

use chrono::{Datelike, Duration, NaiveDate};

use super::copy::{self, Pair};
use super::{Day, Event, Slot, Week, MAX_CHECKS};
use crate::events::{et_time, note, title};
use crate::numbers::Lang;

pub const SITE: &str = "https://prebell.site";

fn pick(pair: Pair, lang: Lang) -> &'static str {
    match lang {
        Lang::En => pair.0,
        Lang::Zh => pair.1,
    }
}

fn code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Zh => "zh",
    }
}

/// Text safe inside a Markdown link label or list item.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn link(label: &str, url: &str) -> String {
    format!("[{}]({url})", escape(label))
}

/// "Nov 9" / "11月9日".
fn month_day(d: NaiveDate, lang: Lang) -> String {
    match lang {
        Lang::En => d.format("%b %-d").to_string(),
        Lang::Zh => format!("{}月{}日", d.month(), d.day()),
    }
}

/// Monday to Friday: "Nov 9–13", "Nov 30–Dec 4" / "11月9日–13日", "11月30日–12月4日".
pub fn range(monday: NaiveDate, lang: Lang) -> String {
    let friday = monday + Duration::days(4);
    let end = match (lang, friday.month() == monday.month()) {
        (Lang::En, true) => friday.day().to_string(),
        (Lang::Zh, true) => format!("{}日", friday.day()),
        (_, false) => month_day(friday, lang),
    };
    format!("{}–{end}", month_day(monday, lang))
}

/// "Mon, Nov 9" / "11月9日 周一", as the site writes a day.
fn day_label(d: NaiveDate, lang: Lang) -> String {
    let wd = pick(
        copy::WEEKDAYS[d.weekday().num_days_from_monday() as usize],
        lang,
    );
    match lang {
        Lang::En => format!("{wd}, {}", month_day(d, lang)),
        Lang::Zh => format!("{} {wd}", month_day(d, lang)),
    }
}

pub fn subject(monday: NaiveDate, lang: Lang) -> String {
    pick(copy::SUBJECT, lang).replace("{range}", &range(monday, lang))
}

fn event_line(e: &Event, lang: Lang) -> Option<String> {
    let t = title(
        &e.series,
        e.ref_period.as_deref(),
        e.variant.as_deref(),
        lang,
    )?;
    let colon = match lang {
        Lang::En => ": ",
        Lang::Zh => "：",
    };
    let mut line = match e.starts_at {
        Some(at) => format!("{}{colon}{}", et_time(at, lang), link(&t, &e.link)),
        None => link(&t, &e.link),
    };
    if let Some(n) = note(&e.series, lang) {
        line.push_str(" · ");
        line.push_str(n);
    }
    match (e.status.as_str(), &e.moved_from) {
        ("postponed", _) => line.push_str(pick(copy::POSTPONED, lang)),
        ("moved", Some(from)) => {
            let from = from
                .parse::<NaiveDate>()
                .map(|d| month_day(d, lang))
                .unwrap_or_else(|_| from.clone());
            line.push_str(&pick(copy::MOVED_FROM, lang).replace("{date}", &from));
        }
        _ => {}
    }
    Some(line)
}

fn count(n: usize, lang: Lang) -> String {
    if n == 1 {
        pick(copy::ONE_COMPANY, lang).to_string()
    } else {
        pick(copy::COMPANIES, lang).replace("{count}", &n.to_string())
    }
}

fn slot_line(label: Pair, s: &Slot, lang: Lang) -> Option<String> {
    if s.is_empty() {
        return None;
    }
    let mut items: Vec<String> = s
        .writeups
        .iter()
        .map(|c| {
            let mut t = escape(&format!("{} ({})", c.name, c.ticker));
            if !c.confirmed {
                t.push_str(pick(copy::UNCONFIRMED, lang));
            }
            t
        })
        .collect();
    if items.is_empty() {
        items.push(count(s.others, lang));
    } else if s.others > 0 {
        items.push(pick(copy::MORE, lang).replace("{count}", &s.others.to_string()));
    }
    let (colon, sep) = match lang {
        Lang::En => (": ", ", "),
        Lang::Zh => ("：", "、"),
    };
    Some(format!("{}{colon}{}", pick(label, lang), items.join(sep)))
}

fn day_block(d: &Day, lang: Lang) -> String {
    let mut lines: Vec<String> = d
        .events
        .iter()
        .filter_map(|e| event_line(e, lang))
        .collect();
    let slots = [
        (copy::BMO, &d.bmo),
        (copy::AMC, &d.amc),
        (copy::TIME_UNKNOWN, &d.unknown),
    ];
    let earnings: Vec<String> = slots
        .iter()
        .filter_map(|(label, s)| slot_line(*label, s, lang))
        .collect();
    if earnings.is_empty() {
        // A closed day says so already.
        if !d.closed() {
            lines.push(pick(copy::NO_EARNINGS, lang).to_string());
        }
    } else {
        lines.extend(earnings);
    }
    let items: Vec<String> = lines.iter().map(|l| format!("- {l}")).collect();
    format!("### {}\n\n{}", day_label(d.date, lang), items.join("\n"))
}

/// The email body (without Buttondown's editor-mode line).
pub fn body(w: &Week, lang: Lang) -> String {
    let l = code(lang);
    let week = w.monday.format("%Y-%m-%d");
    let mut out = vec![
        pick(copy::INTRO, lang).replace("{range}", &range(w.monday, lang)),
        format!("## {}", pick(copy::CALENDAR_HEADING, lang)),
    ];
    out.extend(w.days.iter().map(|d| day_block(d, lang)));
    out.push(link(
        pick(copy::FULL_CALENDAR, lang),
        &format!("{SITE}/{l}/earnings?week={week}"),
    ));

    let items: Vec<String> = w
        .checks
        .iter()
        .filter_map(|p| {
            // A Chinese reader gets the English page while the Chinese one
            // is pending; English is always published first.
            let (t, pl) = match (lang, &p.title_en, &p.title_zh) {
                (Lang::Zh, _, Some(zh)) => (zh, "zh"),
                (_, Some(en), _) => (en, "en"),
                _ => return None,
            };
            Some(format!(
                "- {}",
                link(t, &format!("{SITE}/{pl}/p/{}", p.slug))
            ))
        })
        .take(MAX_CHECKS)
        .collect();
    // Missing stays missing: no checks, no section.
    if !items.is_empty() {
        out.push(format!("## {}", pick(copy::CHECKS_HEADING, lang)));
        out.push(items.join("\n"));
        out.push(link(
            pick(copy::ALL_CHECKS, lang),
            &format!("{SITE}/{l}/archive?type=earnings"),
        ));
    }
    out.join("\n\n") + "\n"
}
