//! The Friday earnings week post (design
//! `docs/superpowers/specs/2026-11-24-weekly-earnings-design.md`): a post of
//! content type `week`, written by code in English and Chinese from one facts
//! bundle, `week/<friday>.json` in R2.
//!
//! - **Section 1** lists the earnings checks published since the previous
//!   week post, by market value, each with its two cover lines as stored.
//!   The window is stored, never re-derived from timestamps that move: a
//!   check is listed when its English text was published by the bundle's
//!   `cutoff` (the first successful build), it is not in the previous
//!   published bundle's `listed`, and its report date is on or after that
//!   post's Monday (this week's Monday for the first post). A rebuild lists
//!   the stored `listed` again, re-read by slug.
//! - **Section 2** is next week's calendar (`digest::week_days`) as of the
//!   bundle's `as_of`, with each row's status, and the week's events.
//! - **Section 3** links the live calendar of next week.
//!
//! The page is checked (`guard::verify_page`) before anything is written,
//! and published as the IPO write-ups are (`job`). `job` also holds the
//! trigger, the correction path and the alerts.

pub mod guard;
pub mod job;
pub mod period;
pub mod render;
pub mod store;

use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::digest::{market_cap, Company, Day, Slot};
use crate::earnings::{find, Tier};
use period::Period;
use store::CheckRow;

pub const CONTENT_TYPE: &str = "week";
pub const TEMPLATE_ID: &str = "week-v1";
/// Publish only with at least this many checks in section 1, or this many
/// write-up companies expected next week (the lead's call).
pub const MIN_CHECKS: usize = 2;
pub const MIN_WRITEUPS: usize = 2;

/// `WEEK_POST_ENABLED`: on only when it is exactly `true`.
pub fn enabled(raw: Option<&str>) -> bool {
    raw == Some("true")
}

pub fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `week:2026-12-04`.
pub fn run_key(friday: NaiveDate) -> String {
    format!("week:{}", ymd(friday))
}

/// The Friday of a run key, when it is one (a Friday, written canonically).
pub fn friday_of(run_key: &str) -> Option<NaiveDate> {
    let d: NaiveDate = run_key.strip_prefix("week:")?.parse().ok()?;
    (d.weekday() == chrono::Weekday::Fri && ymd(d) == run_key[5..]).then_some(d)
}

/// `earnings-week-2026-12-04`.
pub fn slug(friday: NaiveDate) -> String {
    format!("earnings-week-{}", ymd(friday))
}

/// The bundle's R2 key: `week/2026-12-04.json`.
pub fn facts_key(friday: NaiveDate) -> String {
    format!("week/{}.json", ymd(friday))
}

/// The cover stem of revision `n` (for the card task): the first build's is
/// the slug, a correction's `earnings-week-<friday>-r<n>`, so `cards_tick`
/// renders it a new card.
pub fn cover_stem(friday: NaiveDate, revision: u32) -> String {
    match revision {
        0 => slug(friday),
        n => format!("{}-r{n}", slug(friday)),
    }
}

/// An earnings check in section 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub slug: String,
    pub ticker: String,
    /// The watchlist's display name, else the ticker.
    pub name: String,
    pub trading_date: String,
    pub period: Option<Period>,
    /// The two cover lines exactly as stored in `post_translations`.
    pub en: [String; 2],
    pub zh: [String; 2],
    /// The Chinese translation's status: published | pending | failed |
    /// missing.
    pub zh_status: String,
    /// The market value it was ordered by (the dated snapshot; 0 when
    /// absent), and its place in that order, from 1.
    pub cap: u64,
    pub rank: usize,
}

/// A calendar row in section 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expected {
    pub ticker: String,
    pub name: String,
    /// full | calendar.
    pub tier: String,
    pub date: String,
    /// bmo | amc | unknown.
    pub slot: String,
    /// unconfirmed | confirmed | reported.
    pub status: String,
    /// yahoo | ir | edgar, with the row's proof and when it was checked.
    pub source: String,
    pub proof_url: Option<String>,
    pub checked_at: String,
}

/// An economy & Fed event of next week (`digest::Event`, with its date).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeekEvent {
    pub date: String,
    pub series: String,
    pub ref_period: Option<String>,
    pub variant: Option<String>,
    /// RFC 3339; `None` for all-day events.
    pub starts_at: Option<String>,
    pub status: String,
    pub link: String,
    pub moved_from: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    /// Checks in section 1.
    pub checks: usize,
    /// Companies expected next week, and those whose date is confirmed (or
    /// already reported).
    pub expected: usize,
    pub confirmed: usize,
    /// Write-up companies expected next week.
    pub writeups: usize,
    /// Companies expected each day, Monday to Friday.
    pub per_day: [usize; 5],
}

/// Everything the page states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub friday: String,
    pub monday: String,
    pub next_monday: String,
    /// Section 1's lower report date: the previous week post's Monday, or
    /// this week's.
    pub since: String,
    /// The first successful build (RFC 3339); kept on every rebuild.
    pub cutoff: String,
    /// When section 2 was read: the cutoff, or a correction's rebuild.
    pub as_of: String,
    /// 0 for the first build, n for the n-th correction.
    pub revision: u32,
    /// The first and last report dates of the listed checks.
    pub window: Option<[String; 2]>,
    pub listed: Vec<String>,
    pub checks: Vec<Check>,
    pub expected: Vec<Expected>,
    pub events: Vec<WeekEvent>,
    pub counts: Counts,
}

fn day(s: &str) -> NaiveDate {
    s.parse().unwrap_or_default()
}

impl Bundle {
    pub fn friday(&self) -> NaiveDate {
        day(&self.friday)
    }
    pub fn monday(&self) -> NaiveDate {
        day(&self.monday)
    }
    pub fn next_monday(&self) -> NaiveDate {
        day(&self.next_monday)
    }
    pub fn as_of(&self) -> DateTime<Utc> {
        self.as_of.parse().unwrap_or_default()
    }
    /// Section 1's dates: from the earlier of this Monday and the first
    /// listed report, to the Friday.
    pub fn range(&self) -> (NaiveDate, NaiveDate) {
        let first = self.window.as_ref().map(|w| day(&w[0]));
        let from = first.map_or(self.monday(), |f| f.min(self.monday()));
        (from, self.friday())
    }
    /// The minimum (`MIN_CHECKS`, `MIN_WRITEUPS`).
    pub fn enough(&self) -> bool {
        self.counts.checks >= MIN_CHECKS || self.counts.writeups >= MIN_WRITEUPS
    }
}

/// The Monday of `friday`'s week.
pub fn monday_of(friday: NaiveDate) -> NaiveDate {
    friday - Duration::days(4)
}

/// Section 1's checks from the rows read, minus `exclude` (the previous
/// bundle's `listed`), by market value: largest first, a company missing
/// from the snapshot or at 0 there after the rest, by ticker, then slug.
pub fn checks(rows: Vec<CheckRow>, exclude: &[String]) -> Vec<Check> {
    let mut out: Vec<Check> = rows
        .into_iter()
        .filter(|r| !exclude.contains(&r.slug))
        .filter_map(|r| {
            let ticker = r.ticker()?.to_string();
            let name = find(&ticker).map_or(ticker.clone(), |w| w.name.to_string());
            let line = |l: &Option<String>| l.clone().unwrap_or_default();
            Some(Check {
                period: period::period(&r.slug, day(&r.trading_date)),
                cap: market_cap(&ticker),
                en: [line(&r.en[0]), line(&r.en[1])],
                zh: [line(&r.zh[0]), line(&r.zh[1])],
                zh_status: r.zh_status.clone().unwrap_or_else(|| "missing".into()),
                slug: r.slug,
                name,
                ticker,
                trading_date: r.trading_date,
                rank: 0,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        let key = |c: &Check| {
            let cap = Some(c.cap).filter(|c| *c > 0);
            (
                cap.is_none(),
                std::cmp::Reverse(cap),
                c.ticker.clone(),
                c.slug.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    for (i, c) in out.iter_mut().enumerate() {
        c.rank = i + 1;
    }
    out
}

fn expected_of(c: &Company, date: NaiveDate, slot: &str, tier: Tier) -> Expected {
    Expected {
        ticker: c.ticker.clone(),
        name: c.name.clone(),
        tier: match tier {
            Tier::Full => "full",
            Tier::Calendar => "calendar",
        }
        .into(),
        date: ymd(date),
        slot: slot.into(),
        status: c.status.clone(),
        source: c.source.clone(),
        proof_url: c.proof_url.clone(),
        checked_at: c.checked_at.clone(),
    }
}

/// Inputs of `assemble` that are not rows.
pub struct Stamp {
    pub friday: NaiveDate,
    pub since: NaiveDate,
    pub cutoff: DateTime<Utc>,
    pub as_of: DateTime<Utc>,
    pub revision: u32,
}

/// The bundle: section 1's checks (already chosen and ordered), and
/// next week's days (`digest::week_days`), largest companies first in each
/// slot as `week_days` orders them.
pub fn assemble(s: &Stamp, checks: Vec<Check>, days: &[Day]) -> Bundle {
    let mut expected = Vec::new();
    let mut events = Vec::new();
    let mut per_day = [0usize; 5];
    for (i, d) in days.iter().enumerate().take(5) {
        for (label, slot) in [("bmo", &d.bmo), ("amc", &d.amc), ("unknown", &d.unknown)] {
            let Slot { writeups, rest, .. } = slot;
            for c in writeups {
                expected.push(expected_of(c, d.date, label, Tier::Full));
            }
            for c in rest {
                expected.push(expected_of(c, d.date, label, Tier::Calendar));
            }
            per_day[i] += writeups.len() + rest.len();
        }
        for e in &d.events {
            events.push(WeekEvent {
                date: ymd(d.date),
                series: e.series.clone(),
                ref_period: e.ref_period.clone(),
                variant: e.variant.clone(),
                starts_at: e.starts_at.map(iso),
                status: e.status.clone(),
                link: e.link.clone(),
                moved_from: e.moved_from.clone(),
            });
        }
    }
    let window = {
        let mut dates: Vec<&String> = checks.iter().map(|c| &c.trading_date).collect();
        dates.sort();
        match (dates.first(), dates.last()) {
            (Some(a), Some(b)) => Some([(*a).clone(), (*b).clone()]),
            _ => None,
        }
    };
    let counts = Counts {
        checks: checks.len(),
        expected: expected.len(),
        confirmed: expected
            .iter()
            .filter(|e| e.status != "unconfirmed")
            .count(),
        writeups: expected.iter().filter(|e| e.tier == "full").count(),
        per_day,
    };
    Bundle {
        friday: ymd(s.friday),
        monday: ymd(monday_of(s.friday)),
        next_monday: ymd(s.friday + Duration::days(3)),
        since: ymd(s.since),
        cutoff: iso(s.cutoff),
        as_of: iso(s.as_of),
        revision: s.revision,
        window,
        listed: checks.iter().map(|c| c.slug.clone()).collect(),
        checks,
        expected,
        events,
        counts,
    }
}

#[cfg(test)]
pub(crate) mod tests;
