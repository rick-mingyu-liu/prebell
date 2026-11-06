//! The weekly digest email (Rick, 2026-11-06): next week's earnings calendar
//! and economy & Fed dates, with links to last week's earnings checks, one
//! email per language. Each is saved as a draft that the owner reviews and
//! sends; `DIGEST_MODE=scheduled` asks for Sunday 18:00 ET instead.
//!
//! No cron of its own: a Friday evening earnings poll (after that day's
//! calendar refresh) queues one `digest` message until kv `digest_built_for`
//! holds next week's Monday, and only while `DIGEST_ENABLED` is `true`
//! (`gate`).
//! Everything shown comes from D1, the events snapshot and `clock`; the
//! wording is in `copy`.
//!
//! Off for now: Buttondown's free tier has no tag targeting, and how to
//! split the languages is undecided (Rick, 2026-11-06: "maybe let's design
//! that in the future"; `docs/decisions.md`). Sending is behind `Sender`;
//! `buttondown::Buttondown` is one, tested but not wired into the Worker,
//! which runs `digest_tick` without a sender (a dry run).

pub mod buttondown;
pub mod copy;
pub mod render;

use std::collections::HashMap;
use std::sync::OnceLock;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, Timelike, Utc, Weekday};

use crate::clock::et;
use crate::earnings::store::EarningsStore;
use crate::earnings::{find, Tier};
use crate::events::check::{last_loaded, new_row};
use crate::events::parse::et_instant;
use crate::events::store::{EventRow, EventStore};
use crate::events::{closures, planned, TIER_NOTABLE};
use crate::jobs::alert_once_today;
use crate::numbers::Lang;
use crate::store::Store;

/// kv key: next week's Monday (`YYYY-MM-DD`) once both emails are done;
/// `<monday>:en` (or `:zh`) while only that one is.
pub const KV_BUILT: &str = "digest_built_for";
/// Fridays from this ET hour: after the recap and the day's earnings
/// checks, with polls (and so retries) every 5 minutes until 22:00 ET.
pub const FROM_HOUR: u32 = 18;
/// `DIGEST_MODE=scheduled` sends at this ET hour on the Sunday before the week.
pub const SEND_HOUR: u32 = 18;
/// Last week's earnings checks listed at most, largest companies first.
pub const MAX_CHECKS: usize = 10;

const CAPS: &str = include_str!("../../data/marketcap-snapshot.tsv");

/// `DIGEST_MODE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The default: drafts the owner reviews and sends.
    Draft,
    /// Scheduled for Sunday 18:00 ET (`send_at`).
    Scheduled,
}

/// `DIGEST_MODE`: unset or empty is `Draft`; anything but `draft` or
/// `scheduled` is a config error.
pub fn parse_mode(raw: Option<&str>) -> Result<Mode, String> {
    match raw.map(str::trim).filter(|v| !v.is_empty()) {
        None | Some("draft") => Ok(Mode::Draft),
        Some("scheduled") => Ok(Mode::Scheduled),
        Some(other) => Err(format!(
            "unknown DIGEST_MODE: {other} (use draft or scheduled)"
        )),
    }
}

/// `DIGEST_ENABLED`: on only when it is exactly `true`; unset or anything
/// else is off.
pub fn enabled(raw: Option<&str>) -> bool {
    raw.map(str::trim) == Some("true")
}

/// Drops the `digest` follow-up from a poll's list unless `DIGEST_ENABLED`
/// (`raw`) is on, so the Friday trigger never queues it while off.
pub fn gate(follow_ups: &mut Vec<crate::schedule::EarningsStep>, raw: Option<&str>) {
    if !enabled(raw) {
        follow_ups.retain(|s| *s != crate::schedule::EarningsStep::Digest);
    }
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Whether a poll at `now` should queue the digest: Fridays from
/// `FROM_HOUR` ET (polls run to 22:00 ET).
pub fn due(now: DateTime<Utc>) -> bool {
    let local = et(now);
    local.weekday() == Weekday::Fri && local.hour() >= FROM_HOUR
}

/// The Monday after `now`'s ET date: the week the digest is about.
pub fn week_ahead(now: DateTime<Utc>) -> NaiveDate {
    let today = et(now).date_naive();
    today + Duration::days(7 - i64::from(today.weekday().num_days_from_monday()))
}

/// Sunday `SEND_HOUR`:00 in New York before `monday`, in UTC, with that
/// Sunday's own offset (DST ends and starts on a Sunday).
pub fn send_at(monday: NaiveDate) -> Option<DateTime<Utc>> {
    et_instant(monday - Duration::days(1), SEND_HOUR, 0)
}

/// Whether both emails for the week ahead of `now` are done.
pub async fn built(es: &dyn EarningsStore, now: DateTime<Utc>) -> Result<bool, String> {
    Ok(es.kv_get(KV_BUILT).await?.as_deref() == Some(ymd(week_ahead(now)).as_str()))
}

/// A published post, as the sitemap lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PostLink {
    pub slug: String,
    pub trading_date: String,
    /// The filing's ticker (earnings checks).
    pub ticker: Option<String>,
    /// Each language's title, only when that translation is published.
    pub title_en: Option<String>,
    pub title_zh: Option<String>,
}

/// The posts the digest links to. The Worker's is `D1Store`.
#[async_trait(?Send)]
pub trait DigestStore {
    /// Published posts of `content_type` with `from <= trading_date <= to`
    /// and at least one published translation, oldest first (D1: one query).
    async fn published_posts(
        &self,
        content_type: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<PostLink>, String>;
}

/// A write-up company on the calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct Company {
    pub ticker: String,
    pub name: String,
    /// The company announced the date (or it is reported); otherwise an
    /// aggregator's date.
    pub confirmed: bool,
}

/// One report slot of a day: its write-up companies, largest first, and how
/// many other companies report then.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slot {
    pub writeups: Vec<Company>,
    pub others: usize,
}

impl Slot {
    pub fn is_empty(&self) -> bool {
        self.writeups.is_empty() && self.others == 0
    }
}

/// An economy & Fed event or a market closure, titled by `events::title`.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub series: String,
    pub ref_period: Option<String>,
    pub variant: Option<String>,
    /// `None` for all-day events (elections, closures).
    pub starts_at: Option<DateTime<Utc>>,
    /// scheduled | confirmed | moved | postponed | released.
    pub status: String,
    /// The release once out, else the official schedule.
    pub link: String,
    /// The previous ET date of a moved event.
    pub moved_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    /// All-day events first, then by time.
    pub events: Vec<Event>,
    pub bmo: Slot,
    pub amc: Slot,
    pub unknown: Slot,
}

impl Day {
    pub fn closed(&self) -> bool {
        self.events.iter().any(|e| e.series == "closed")
    }
}

/// Everything one digest shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Week {
    /// The week ahead's Monday.
    pub monday: NaiveDate,
    /// Monday to Friday.
    pub days: Vec<Day>,
    /// Last week's earnings checks, largest companies first.
    pub checks: Vec<PostLink>,
}

/// Market caps from the snapshot the write-up tier is cut from; 0 when absent.
fn market_cap(ticker: &str) -> u64 {
    static M: OnceLock<HashMap<&'static str, u64>> = OnceLock::new();
    M.get_or_init(|| {
        CAPS.lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (t, cap) = l.split_once('\t')?;
                Some((t.trim(), cap.trim().parse().ok()?))
            })
            .collect()
    })
    .get(ticker)
    .copied()
    .unwrap_or(0)
}

fn event(r: &EventRow) -> Event {
    Event {
        series: r.series.clone(),
        ref_period: r.ref_period.clone(),
        variant: r.variant.clone(),
        starts_at: r.starts_at.as_deref().and_then(|t| t.parse().ok()),
        status: r.status.clone(),
        link: r.release_url.clone().unwrap_or_else(|| r.proof_url.clone()),
        moved_from: r.moved_from.clone().filter(|_| r.status == "moved"),
    }
}

/// The week of `monday`: its calendar rows (the statuses the calendar page
/// shows), its tier 1-2 events from `macro_events` (from the snapshot, as
/// the brief does, while the table is missing or not yet loaded), its
/// closures from `clock`, and the earnings checks of the week before.
pub async fn gather(
    now: DateTime<Utc>,
    monday: NaiveDate,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    posts: &dyn DigestStore,
) -> Result<Week, String> {
    let friday = monday + Duration::days(4);
    let (from, to) = (ymd(monday), ymd(friday));

    let snapshot = || -> Vec<EventRow> {
        planned(monday, friday)
            .iter()
            .map(|p| new_row(p, now))
            .collect()
    };
    let rows = match events.events_between(&from, &to).await {
        Ok(r) if !r.is_empty() => r,
        Ok(r) => match last_loaded(es).await {
            Ok(None) => snapshot(),
            _ => r,
        },
        Err(_) => snapshot(),
    };
    // Closures always from `clock`, whatever the table holds.
    let mut evs: Vec<(NaiveDate, Event)> = rows
        .iter()
        .filter(|r| r.tier <= i64::from(TIER_NOTABLE))
        .filter(|r| !matches!(r.series.as_str(), "closed" | "early_close"))
        .filter_map(|r| Some((r.et_date.parse().ok()?, event(r))))
        .chain(
            closures(monday, friday)
                .iter()
                .map(|p| (p.et_date, event(&new_row(p, now)))),
        )
        .collect();
    evs.sort_by(|(da, a), (db, b)| {
        (da, a.starts_at.is_some(), a.starts_at).cmp(&(db, b.starts_at.is_some(), b.starts_at))
    });

    let mut days: Vec<Day> = (0..5)
        .map(|i| Day {
            date: monday + Duration::days(i),
            events: vec![],
            bmo: Slot::default(),
            amc: Slot::default(),
            unknown: Slot::default(),
        })
        .collect();
    for (date, e) in evs {
        if let Some(d) = days.iter_mut().find(|d| d.date == date) {
            d.events.push(e);
        }
    }

    let mut cal = es.calendar_between(&from, &to).await?;
    cal.retain(|c| matches!(c.status.as_str(), "unconfirmed" | "confirmed" | "reported"));
    cal.sort_by_key(|c| std::cmp::Reverse(market_cap(&c.ticker)));
    for c in cal {
        let Some(day) = days.iter_mut().find(|d| ymd(d.date) == c.report_date) else {
            continue;
        };
        let slot = match c.time_slot.as_str() {
            "bmo" => &mut day.bmo,
            "amc" => &mut day.amc,
            _ => &mut day.unknown,
        };
        match find(&c.ticker).filter(|w| w.tier == Tier::Full) {
            Some(w) => slot.writeups.push(Company {
                ticker: w.ticker.to_string(),
                name: w.name.to_string(),
                confirmed: c.status != "unconfirmed",
            }),
            None => slot.others += 1,
        }
    }

    let last_monday = monday - Duration::days(7);
    let mut checks = posts
        .published_posts(
            crate::earnings::draft::CONTENT_TYPE,
            &ymd(last_monday),
            &ymd(last_monday + Duration::days(6)),
        )
        .await?;
    // Stable: equal caps keep their publishing order.
    checks.sort_by_key(|p| std::cmp::Reverse(p.ticker.as_deref().map_or(0, market_cap)));

    Ok(Week {
        monday,
        days,
        checks,
    })
}

fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Zh => "zh",
    }
}

/// One language's email, as rendered.
#[derive(Debug, Clone, PartialEq)]
pub struct Email {
    pub lang: Lang,
    pub subject: String,
    /// Markdown.
    pub body: String,
    /// The same for every attempt at this week and language, so a provider
    /// that honours it answers a retry with the first email, not a second.
    pub idempotency_key: String,
    /// `Some` in `Scheduled` mode: send then. `None`: a draft only.
    pub send_at: Option<DateTime<Utc>>,
}

/// What a `Sender` did with one email.
#[derive(Debug, Clone, PartialEq)]
pub enum Sent {
    /// Saved (and scheduled, when asked): a short note for the log line,
    /// e.g. `draft em_01`.
    Done(String),
    /// Nobody to send it to (no subscriber in that language yet); never
    /// sent to everyone instead.
    Skipped(String),
    /// Saved, but it must not go out until the owner looks: the log note
    /// and the alert text.
    NeedsOwner { note: String, alert: String },
}

/// Where the emails go. The choice of provider and of how the languages
/// are split lives entirely behind this (`docs/decisions.md`, "Digest
/// language split").
#[async_trait(?Send)]
pub trait Sender {
    /// Saves `email` for its language's readers. `Err` is a failure to
    /// retry on the next poll, as text free of keys and response bodies.
    async fn send(&self, email: &Email) -> Result<Sent, String>;
}

/// The email for `lang`.
pub fn email(week: &Week, lang: Lang, send_at: Option<DateTime<Utc>>) -> Email {
    Email {
        lang,
        subject: render::subject(week.monday, lang),
        body: render::body(week, lang),
        idempotency_key: format!("prebell-digest-{}-{}", ymd(week.monday), lang_code(lang)),
        send_at,
    }
}

/// One `digest` queue message. Unless next week's digest is done, it
/// gathers the week and renders both languages. Without a `sender` that is
/// all (a dry run: nothing is sent or marked). With one, each language not
/// yet done is sent (for `send_at`, Sunday 18:00 ET, in `Scheduled` mode)
/// and marked done in kv at once; a failure leaves it for the next poll and
/// alerts once per ET day per language. `Err` is a store error.
#[allow(clippy::too_many_arguments)]
pub async fn digest_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    posts: &dyn DigestStore,
    store: &dyn Store,
    sender: Option<&dyn Sender>,
    mode: Mode,
) -> Result<String, String> {
    let monday = week_ahead(now);
    let week = ymd(monday);
    let progress = es.kv_get(KV_BUILT).await?.unwrap_or_default();
    if progress == week {
        return Ok(format!("{week}: already built"));
    }
    let mut done: Vec<&str> = progress
        .strip_prefix(&format!("{week}:"))
        .map(|langs| langs.split(',').filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();

    let data = gather(now, monday, es, events, posts).await?;
    // Past Sunday 18:00 ET (a late run), a schedule would send at once.
    let send = match mode {
        Mode::Scheduled => send_at(monday).filter(|t| *t > now),
        Mode::Draft => None,
    };
    let how = match (mode, send) {
        (Mode::Draft, _) => "draft".to_string(),
        (Mode::Scheduled, Some(t)) => {
            format!("scheduled {}", t.to_rfc3339_opts(SecondsFormat::Secs, true))
        }
        (Mode::Scheduled, None) => "scheduled, send time passed: draft".to_string(),
    };

    let Some(sender) = sender else {
        let parts: Vec<String> = [Lang::En, Lang::Zh]
            .iter()
            .map(|l| {
                let e = email(&data, *l, send);
                format!("{} {:?} ({} bytes)", lang_code(*l), e.subject, e.body.len())
            })
            .collect();
        return Ok(format!(
            "{week} ({how}): dry run, no sender: {}",
            parts.join(", ")
        ));
    };

    let mut parts = Vec::new();
    for lang in [Lang::En, Lang::Zh] {
        let code = lang_code(lang);
        if done.contains(&code) {
            continue;
        }
        let (part, alert) = match sender.send(&email(&data, lang, send)).await {
            Ok(Sent::Done(note)) => (format!("{code} {note}"), None),
            Ok(Sent::Skipped(why)) => (format!("{code} skipped: {why}"), None),
            Ok(Sent::NeedsOwner { note, alert }) => (format!("{code} {note}"), Some(alert)),
            // Left for the next poll.
            Err(e) => {
                parts.push(format!("{code} failed: {e}"));
                let msg = format!(
                    "digest: the {code} email failed ({e}); the next poll retries until 22:00 ET"
                );
                alert_once_today(now, &format!("digest:{code}"), "error", &msg, store).await?;
                continue;
            }
        };
        parts.push(part);
        if let Some(msg) = alert {
            alert_once_today(now, &format!("digest:{code}"), "error", &msg, store).await?;
        }
        done.push(code);
        let value = if ["en", "zh"].iter().all(|c| done.contains(c)) {
            week.clone()
        } else {
            format!("{week}:{}", done.join(","))
        };
        let at = now.to_rfc3339_opts(SecondsFormat::Secs, true);
        es.kv_set(KV_BUILT, &value, &at).await?;
    }
    Ok(format!("{week} ({how}): {}", parts.join(", ")))
}

#[cfg(test)]
mod tests;
