//! The weekly digest email: one English email to every subscriber (Rick,
//! 2026-11-10: "just english"), with three of last week's earnings checks,
//! next week's earnings calendar and economy & Fed dates, and one question.
//! It is saved as a draft that the owner reviews and sends;
//! `DIGEST_MODE=scheduled` asks for Sunday 18:00 ET instead.
//!
//! No cron of its own: a Friday evening earnings poll (after that day's
//! calendar refresh) queues one `digest` message until kv `digest_built_for`
//! holds next week's Monday, and only while `DIGEST_ENABLED` is `true`
//! (`gate`). The message saves the draft with `buttondown::Buttondown` when
//! the Worker has `BUTTONDOWN_API_KEY`, and is a dry run without it
//! (`run_kind`).
//! Everything shown comes from D1, the events snapshot and `clock`; the
//! wording is in `copy`.

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
use crate::store::Store;

/// kv key: next week's Monday (`YYYY-MM-DD`) once its email is saved.
pub const KV_BUILT: &str = "digest_built_for";
/// kv key, `Scheduled` mode: `<Monday> <email id>` once that week's draft is
/// created and before it is scheduled, so a retry schedules the same email
/// and never creates a second one, whatever the provider does with the
/// idempotency key.
pub const KV_EMAIL: &str = "digest_email";
/// Fridays from this ET hour: after the recap and the day's earnings
/// checks, with polls (and so retries) every 5 minutes until 22:00 ET.
pub const FROM_HOUR: u32 = 18;
/// `DIGEST_MODE=scheduled` sends at this ET hour on the Sunday before the week.
pub const SEND_HOUR: u32 = 18;
/// Last week's earnings checks picked at most, largest companies first.
pub const PICKS: usize = 3;
/// Fridays from this ET time, the poll queues the digest even when the
/// day's calendar refresh has not finished (a failed read's retry still
/// pending), and the digest is built from the calendar as it stands, with an
/// alert: otherwise the week would be skipped (polls stop at 22:00 ET).
pub const CLOSING: (u32, u32) = (21, 45);
/// From this ET time on Friday a run is the week's last try (the last poll
/// is 21:55 ET): one that saves no draft alerts that the week is missed.
pub const LAST_TRY: (u32, u32) = (21, 55);

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

/// `DIGEST_ENABLED`: on only when it is exactly `true` (no surrounding
/// spaces or newline); unset or anything else is off.
pub fn enabled(raw: Option<&str>) -> bool {
    raw == Some("true")
}

/// What a `digest` message does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// `DIGEST_ENABLED` is not `true`: nothing.
    Off,
    /// On, but no `BUTTONDOWN_API_KEY`: gather and render, send nothing.
    DryRun,
    /// On with a key: save the draft in Buttondown.
    Send,
}

/// From `DIGEST_ENABLED` (`enabled_raw`) and `BUTTONDOWN_API_KEY` (`key`;
/// unset, empty or blank is no key). Off wins whatever the key.
pub fn run_kind(enabled_raw: Option<&str>, key: Option<&str>) -> RunKind {
    if !enabled(enabled_raw) {
        RunKind::Off
    } else if key.is_some_and(|k| !k.trim().is_empty()) {
        RunKind::Send
    } else {
        RunKind::DryRun
    }
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

/// Friday at or after `at` (ET hour, minute).
fn friday_from(now: DateTime<Utc>, at: (u32, u32)) -> bool {
    let local = et(now);
    local.weekday() == Weekday::Fri && (local.hour(), local.minute()) >= at
}

/// Whether the Friday window is closing (`CLOSING`): the poll queues the
/// digest whatever the calendar refresh's state.
pub fn closing(now: DateTime<Utc>) -> bool {
    friday_from(now, CLOSING)
}

/// Whether a run at `now` is the week's last try (`LAST_TRY`).
pub fn last_try(now: DateTime<Utc>) -> bool {
    friday_from(now, LAST_TRY)
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

/// Whether the email for the week ahead of `now` is saved.
pub async fn built(es: &dyn EarningsStore, now: DateTime<Utc>) -> Result<bool, String> {
    Ok(es.kv_get(KV_BUILT).await?.as_deref() == Some(ymd(week_ahead(now)).as_str()))
}

/// A post whose English page is published, with that page's title and
/// one-line summary as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct PostLink {
    pub slug: String,
    pub trading_date: String,
    /// The filing's ticker (earnings checks).
    pub ticker: Option<String>,
    pub title: String,
    pub summary: String,
}

/// The posts the digest links to. The Worker's is `D1Store`.
#[async_trait(?Send)]
pub trait DigestStore {
    /// Posts of `content_type` with `from <= trading_date <= to` whose
    /// English page is published, oldest first (D1: one query).
    async fn published_posts(
        &self,
        content_type: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<PostLink>, String>;
}

/// A company on the calendar, with its row's status and where the date
/// came from (the week post shows both; the email only `confirmed`).
#[derive(Debug, Clone, PartialEq)]
pub struct Company {
    pub ticker: String,
    /// The watchlist's display name (the ticker for a row no longer on it).
    pub name: String,
    /// The company announced the date (or it is reported); otherwise an
    /// aggregator's date.
    pub confirmed: bool,
    /// The calendar row's status: unconfirmed | confirmed | reported.
    pub status: String,
    /// The row's `source` (yahoo | ir | edgar), `proof_url` and `checked_at`.
    pub source: String,
    pub proof_url: Option<String>,
    pub checked_at: String,
}

/// One report slot of a day: its write-up companies, largest first, and how
/// many other companies report then (`rest` names them, largest first).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slot {
    pub writeups: Vec<Company>,
    pub others: usize,
    /// The calendar-only companies `others` counts.
    pub rest: Vec<Company>,
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
    /// Up to `PICKS` of last week's earnings checks, largest companies first.
    pub picks: Vec<PostLink>,
}

/// Market caps from the snapshot the write-up tier is cut from; 0 when absent.
pub(crate) fn market_cap(ticker: &str) -> u64 {
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

/// Up to `PICKS` of `posts` (oldest first, as `DigestStore` returns them):
/// a post with an empty title is dropped, a ticker is picked once (its
/// newest check), and the rest are ordered largest company first; a company
/// with no market cap in the snapshot (absent, or 0) comes after every one
/// that has one, a post with no ticker after those, and ties go by ticker,
/// then slug.
pub fn pick(posts: Vec<PostLink>) -> Vec<PostLink> {
    let mut seen = std::collections::HashSet::new();
    let mut picks: Vec<PostLink> = posts
        .into_iter()
        .rev()
        .filter(|p| !p.title.trim().is_empty())
        .filter(|p| p.ticker.as_ref().is_none_or(|t| seen.insert(t.clone())))
        .collect();
    picks.sort_by(|a, b| {
        let key = |p: &PostLink| {
            let cap = p.ticker.as_deref().map(market_cap).filter(|c| *c > 0);
            let t = p.ticker.clone();
            (
                cap.is_none(),
                std::cmp::Reverse(cap),
                t.is_none(),
                t,
                p.slug.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    picks.truncate(PICKS);
    picks
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
/// closures from `clock`, and the `PICKS` largest companies' earnings checks
/// of the week before.
pub async fn gather(
    now: DateTime<Utc>,
    monday: NaiveDate,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    posts: &dyn DigestStore,
) -> Result<Week, String> {
    let days = week_days(now, monday, es, events).await?;
    let last_monday = monday - Duration::days(7);
    let picks = pick(
        posts
            .published_posts(
                crate::earnings::draft::CONTENT_TYPE,
                &ymd(last_monday),
                &ymd(last_monday + Duration::days(6)),
            )
            .await?,
    );
    Ok(Week {
        monday,
        days,
        picks,
    })
}

/// `gather`'s days of `monday`'s week, without the picks (the week post
/// reads its checks itself): two reads, the events and the calendar.
pub async fn week_days(
    now: DateTime<Utc>,
    monday: NaiveDate,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
) -> Result<Vec<Day>, String> {
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
        let w = find(&c.ticker);
        let company = Company {
            ticker: c.ticker.clone(),
            name: w.as_ref().map_or(c.ticker.clone(), |w| w.name.to_string()),
            confirmed: c.status != "unconfirmed",
            status: c.status.clone(),
            source: c.source.clone(),
            proof_url: c.proof_url.clone(),
            checked_at: c.checked_at.clone(),
        };
        if w.is_some_and(|w| w.tier == Tier::Full) {
            slot.writeups.push(company);
        } else {
            slot.others += 1;
            slot.rest.push(company);
        }
    }
    Ok(days)
}

/// The email, as rendered.
#[derive(Debug, Clone, PartialEq)]
pub struct Email {
    pub subject: String,
    /// Markdown.
    pub body: String,
    /// The same for every attempt at this week, so a provider that honours
    /// it answers a retry with the first email, not a second.
    pub idempotency_key: String,
    /// `Some` in `Scheduled` mode: send then. `None`: a draft only.
    pub send_at: Option<DateTime<Utc>>,
}

/// What a `Sender` did with the email.
#[derive(Debug, Clone, PartialEq)]
pub enum Saved {
    /// Saved as a draft: the provider's email id.
    Draft(String),
    /// Saved, but not as a draft: it must not be scheduled, and the owner
    /// must look. The log note and the alert text.
    NeedsOwner { note: String, alert: String },
}

/// Where the email goes; the rest does not depend on the provider. `Err` is
/// a failure to retry on the next poll, as text free of keys and response
/// bodies.
#[async_trait(?Send)]
pub trait Sender {
    /// Saves `email` as a draft for every subscriber.
    async fn create(&self, email: &Email) -> Result<Saved, String>;
    /// Schedules the saved draft `id` to send at `at`. `Ok` too when the
    /// provider shows it already scheduled for `at` (a repeat after an
    /// answer that was lost).
    async fn schedule(&self, id: &str, at: DateTime<Utc>) -> Result<(), String>;
}

/// The id saved under `KV_EMAIL` for `week`, if any.
async fn saved_email(es: &dyn EarningsStore, week: &str) -> Result<Option<String>, String> {
    Ok(es
        .kv_get(KV_EMAIL)
        .await?
        .and_then(|v| v.strip_prefix(&format!("{week} ")).map(String::from))
        .filter(|id| !id.is_empty()))
}

/// The week's email.
pub fn email(week: &Week, send_at: Option<DateTime<Utc>>) -> Email {
    Email {
        subject: render::subject(week.monday),
        body: render::body(week),
        idempotency_key: format!("prebell-digest-{}", ymd(week.monday)),
        send_at,
    }
}

/// One `digest` queue message, Fridays from 18:00 ET only (`due`; a stray
/// message on another day does nothing). Unless next week's email is saved,
/// it waits for the day's calendar refresh to finish, or from `CLOSING`
/// goes ahead without it (`digest-stale` once the draft is saved: a warning
/// in `Draft` mode; in `Scheduled` mode an error, and the email is held as a
/// draft, not scheduled), gathers the week and renders the email. Without a
/// `sender` that is all (a dry run: nothing is sent or marked). With one, the
/// email is saved (and, in `Scheduled` mode, its id kept in kv `KV_EMAIL`
/// and the email scheduled for `send_at`, Sunday 18:00 ET; a later run that
/// week schedules that id and never creates another) and marked
/// done in kv at once; a failure leaves it for the next poll and alerts once
/// per ET day (`digest`). A run from `LAST_TRY` that saves no draft alerts
/// `digest-missed`; an answer that is not a draft alerts `digest-not-draft`
/// whatever was alerted before. A store error is alerted (best effort) and
/// left for the next poll; it never surfaces as a config error.
pub async fn digest_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    posts: &dyn DigestStore,
    store: &dyn Store,
    sender: Option<&dyn Sender>,
    mode: Mode,
) -> Result<String, String> {
    if !due(now) {
        return Ok("not due (Fridays from 18:00 ET only); skipped".into());
    }
    let week = ymd(week_ahead(now));
    match run(now, es, events, posts, store, sender, mode).await {
        Ok(line) => Ok(line),
        Err(e) => {
            let (key, msg) = if last_try(now) {
                (
                    "digest-missed",
                    format!("digest: no draft for the week of {week}: the last try before 22:00 ET hit a store error ({e}); nothing retries until next Friday"),
                )
            } else {
                (
                    "digest",
                    format!("digest: store error ({e}); the next poll retries until 22:00 ET"),
                )
            };
            // The store may be what failed: the alert is best effort.
            let note = match alert_once_today(now, key, "error", &msg, store).await {
                Ok(_) => "",
                Err(_) => "; alert failed too",
            };
            Ok(format!(
                "{week}: store error, left for the next poll: {e}{note}"
            ))
        }
    }
}

/// The once-a-week alert that no draft will be saved for `week`.
async fn missed(
    now: DateTime<Utc>,
    week: &str,
    why: &str,
    store: &dyn Store,
) -> Result<bool, String> {
    let msg = format!(
        "digest: no draft for the week of {week}: {why}; nothing retries until next Friday"
    );
    alert_once_today(now, "digest-missed", "error", &msg, store).await
}

async fn run(
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
    if es.kv_get(KV_BUILT).await?.as_deref() == Some(week.as_str()) {
        return Ok(format!("{week}: already built"));
    }
    // Next week's dates are as complete as they get once today's refresh
    // finished. Near 22:00 ET a refresh still waiting for its retry would
    // skip the week, so the digest goes ahead and says so.
    let stale = !crate::earnings::calendar::refreshed_today(es, now).await?;
    if stale && !closing(now) {
        return Ok(format!(
            "{week}: waiting for today's calendar refresh to finish"
        ));
    }
    let data = gather(now, monday, es, events, posts).await?;
    // Past Sunday 18:00 ET (a late run), a schedule would send at once. A
    // week built from an unfinished calendar refresh is never scheduled:
    // nobody reads a scheduled email first, so it is held as a draft and
    // alerted (below) for the owner to check and send.
    let send = match mode {
        Mode::Scheduled if !stale => send_at(monday).filter(|t| *t > now),
        _ => None,
    };
    let mut how = match (mode, send) {
        (Mode::Draft, _) => "draft".to_string(),
        (Mode::Scheduled, Some(t)) => {
            format!("scheduled {}", t.to_rfc3339_opts(SecondsFormat::Secs, true))
        }
        (Mode::Scheduled, None) if stale => "scheduled, held as a draft".to_string(),
        (Mode::Scheduled, None) => "scheduled, send time passed: draft".to_string(),
    };
    if stale {
        how.push_str(", calendar refresh unfinished");
    }
    let e = email(&data, send);

    let Some(sender) = sender else {
        if last_try(now) {
            missed(
                now,
                &week,
                "the digest is on but ran as a dry run, with no Buttondown key",
                store,
            )
            .await?;
        }
        return Ok(format!(
            "{week} ({how}): dry run, no sender: {:?} ({} picks, {} bytes)",
            e.subject,
            data.picks.len(),
            e.body.len()
        ));
    };

    let at = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    // Scheduled mode: once a draft is created its id is in kv before the
    // schedule call, and a later run for the week schedules that id rather
    // than create another (a lost answer or a failed kv write must never
    // make two emails). Draft mode creates and stops.
    let stored = match mode {
        Mode::Scheduled => saved_email(es, &week).await?,
        Mode::Draft => None,
    };
    let mut known = stored.clone();
    let sent: Result<(String, Option<String>), String> = async {
        let id = match stored {
            Some(id) => id,
            None => match sender.create(&e).await? {
                Saved::Draft(id) => id,
                Saved::NeedsOwner { note, alert } => return Ok((note, Some(alert))),
            },
        };
        let Some(t) = e.send_at else {
            return Ok((format!("draft {id}"), None));
        };
        if known.is_none() {
            es.kv_set(KV_EMAIL, &format!("{week} {id}"), &at)
                .await
                .map_err(|err| format!("email {id} was created but its id was not saved ({err}), so it was not scheduled"))?;
            known = Some(id.clone());
        }
        sender.schedule(&id, t).await?;
        Ok((format!("scheduled {id}"), None))
    }
    .await;
    let (note, alert) = match sent {
        Ok(v) => v,
        Err(err) => {
            if last_try(now) {
                // Scheduling is a second call: a draft may be saved but not
                // scheduled, and nothing retries it.
                let why = match (send, &known) {
                    (Some(_), Some(id)) => format!("the last try before 22:00 ET failed ({err}); email {id} is saved in Buttondown but could not be confirmed as scheduled (it may be scheduled already): check it, and schedule or send it by hand if it is not"),
                    (Some(_), None) => format!("the last try before 22:00 ET failed ({err}); a draft may be saved in Buttondown but not scheduled: schedule or send it by hand"),
                    (None, _) => format!("the last try before 22:00 ET failed ({err})"),
                };
                missed(now, &week, &why, store).await?;
            } else {
                let msg = format!(
                    "digest: the email failed ({err}); the next poll retries until 22:00 ET"
                );
                alert_once_today(now, "digest", "error", &msg, store).await?;
            }
            return Ok(format!("{week} ({how}): failed: {err}"));
        }
    };
    // Its own key: the one warning that an email may be going out must not
    // be swallowed by an earlier failure's alert that evening.
    if let Some(msg) = alert {
        alert_once_today(now, "digest-not-draft", "error", &msg, store).await?;
    }
    // Before the week is marked, so a held draft is never left unalerted.
    if stale {
        let (level, msg) = match mode {
            // Held: it goes out only if the owner sends it.
            Mode::Scheduled => (
                "error",
                format!(
                    "digest: the email for the week of {week} was saved as a draft and NOT scheduled: today's calendar refresh had not finished by 21:45 ET (a failed read was waiting for its retry), so some dates may be out of date; check them, then send or schedule it in Buttondown by Sunday 18:00 ET"
                ),
            ),
            Mode::Draft => (
                "warn",
                format!(
                    "digest: the draft for the week of {week} was built from the calendar as it stood: today's calendar refresh had not finished by 21:45 ET (a failed read was waiting for its retry), so some dates may be out of date; check them before sending"
                ),
            ),
        };
        alert_once_today(now, "digest-stale", level, &msg, store).await?;
    }
    es.kv_set(KV_BUILT, &week, &at).await?;
    Ok(format!("{week} ({how}): {note}"))
}

#[cfg(test)]
mod tests;
