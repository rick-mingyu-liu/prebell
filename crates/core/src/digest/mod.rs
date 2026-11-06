//! The weekly digest email (Rick, 2026-11-06): next week's earnings calendar
//! and economy & Fed dates, with links to last week's earnings checks, as
//! two Buttondown emails, English to `lang:en` subscribers and Chinese to
//! `lang:zh`. Each is saved as a draft that the owner reviews and sends;
//! `DIGEST_MODE=scheduled` schedules them for Sunday 18:00 ET instead.
//!
//! No cron of its own: a Friday evening earnings poll (after that day's
//! calendar refresh) queues one `digest` message until kv `digest_built_for`
//! holds next week's Monday. Everything shown comes from D1, the events
//! snapshot and `clock`; the wording is in `copy`.

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
use crate::providers::Http;
use crate::store::Store;

use buttondown::Failure;

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

/// The `X-Idempotency-Key` of one email: a retry after a lost kv write gets
/// Buttondown's first answer back instead of a second draft.
fn idempotency_key(monday: NaiveDate, lang: Lang) -> String {
    format!("prebell-digest-{}-{}", ymd(monday), lang_code(lang))
}

fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Zh => "zh",
    }
}

/// What happened to one language's email.
enum Done {
    /// Saved as a draft (`scheduled`: and scheduled), with its id.
    Created(String, &'static str),
    /// No such tag: nobody to send it to, and never sent unfiltered.
    NoTag,
}

/// One `digest` queue message. Without a key it does nothing (no D1 read).
/// Otherwise, unless next week's digest is done, it gathers the week, reads
/// Buttondown's tag ids, and saves each language not yet done as a draft
/// for its `lang:` tag (scheduled for Sunday 18:00 ET in `Scheduled` mode,
/// once Buttondown shows the audience filter on the draft). A language is
/// marked done in kv as soon as its email is; a Buttondown failure leaves
/// it for the next poll and alerts once per ET day. Logs only status codes.
/// `Err` is a store error.
#[allow(clippy::too_many_arguments)]
pub async fn digest_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    posts: &dyn DigestStore,
    store: &dyn Store,
    http: &dyn Http,
    key: &str,
    mode: Mode,
    log: &dyn Fn(&str),
) -> Result<String, String> {
    if key.is_empty() {
        return Ok("no BUTTONDOWN_API_KEY on the generator; skipped".into());
    }
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
    let tags = match buttondown::tag_ids(http, key, log).await {
        Ok(t) => t,
        Err(f) => {
            let msg = format!(
                "digest: Buttondown tag list failed ({f}); the next poll retries until 22:00 ET"
            );
            alert_once_today(now, "digest", "error", &msg, store).await?;
            return Ok(format!("{week}: tags {f}"));
        }
    };
    // Past Sunday 18:00 ET (a late run), a schedule would send at once.
    let send = match mode {
        Mode::Scheduled => send_at(monday).filter(|t| *t > now),
        Mode::Draft => None,
    };

    let mut parts = Vec::new();
    for lang in [Lang::En, Lang::Zh] {
        let code = lang_code(lang);
        if done.contains(&code) {
            continue;
        }
        let tag = format!("lang:{code}");
        let result = match tags.get(&tag) {
            None => Ok(Done::NoTag),
            Some(tag_id) => {
                send_one(
                    http,
                    key,
                    &data,
                    lang,
                    tag_id,
                    &idempotency_key(monday, lang),
                    send,
                    log,
                )
                .await
            }
        };
        let (part, alert) = match result {
            Ok(Done::Created(id, what)) => (format!("{code} {what} {id}"), None),
            Ok(Done::NoTag) => (format!("{code} skipped: no {tag} tag"), None),
            // Left for the next poll.
            Err(SendError::Failed(f)) => {
                parts.push(format!("{code} failed: {f}"));
                let msg = format!(
                    "digest: Buttondown {code} draft failed ({f}); the next poll retries until 22:00 ET"
                );
                alert_once_today(now, &format!("digest:{code}"), "error", &msg, store).await?;
                continue;
            }
            // Never scheduled; the owner sets the audience before sending.
            Err(SendError::NoAudience(id)) => (
                format!("{code} draft {id} without its audience"),
                Some(format!(
                    "digest: Buttondown dropped the {tag} audience filter on the {code} draft {id}; left as a draft, set its audience before sending"
                )),
            ),
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
    let how = match (mode, send) {
        (Mode::Draft, _) => "draft".to_string(),
        (Mode::Scheduled, Some(t)) => {
            format!("scheduled {}", t.to_rfc3339_opts(SecondsFormat::Secs, true))
        }
        (Mode::Scheduled, None) => "scheduled, send time passed: draft".to_string(),
    };
    Ok(format!("{week} ({how}): {}", parts.join(", ")))
}

enum SendError {
    Failed(Failure),
    /// The draft was saved but Buttondown's answer lacks the tag filter.
    NoAudience(String),
}

/// Saves `lang`'s email as a draft for `tag_id` and, with `send`, schedules
/// it once the draft shows the filter.
#[allow(clippy::too_many_arguments)]
async fn send_one(
    http: &dyn Http,
    key: &str,
    week: &Week,
    lang: Lang,
    tag_id: &str,
    idempotency: &str,
    send: Option<DateTime<Utc>>,
    log: &dyn Fn(&str),
) -> Result<Done, SendError> {
    let email = buttondown::draft(
        &render::subject(week.monday, lang),
        &render::body(week, lang),
        tag_id,
    );
    let created = buttondown::create(http, key, idempotency, &email, log)
        .await
        .map_err(SendError::Failed)?;
    if !created.targets(tag_id) {
        return Err(SendError::NoAudience(created.id));
    }
    match send {
        None => Ok(Done::Created(created.id, "draft")),
        Some(t) => {
            buttondown::schedule(http, key, &created.id, t, log)
                .await
                .map_err(SendError::Failed)?;
            Ok(Done::Created(created.id, "scheduled"))
        }
    }
}

#[cfg(test)]
mod tests;
