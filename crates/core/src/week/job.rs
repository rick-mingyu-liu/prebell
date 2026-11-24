//! The `week` message: build, publish, correct.
//!
//! **Trigger** (`follow_up`, on every earnings poll while
//! `WEEK_POST_ENABLED` is exactly `true`; off is nothing, without a read).
//! One indexed read of the `week:*` runs per poll. The message is queued:
//! - on Friday from 18:00 ET (`digest::due`), once that day's calendar
//!   refresh has finished or from 21:45 ET (`digest::closing`), while the
//!   run `week:<friday>` is not `published`, `skipped` or `failed`, and is
//!   not in flight or waiting to retry; from 21:55 ET (`digest::last_try`)
//!   whatever it waits for, so the last try can alert;
//! - on any weekday, for a correction: a `week:*` run set back to `pending`
//!   (or left `building` by a crash) whose post is live.
//!
//! **Build** (`week_tick`, on the serial build queue; one build per
//! message). Friday's run first, else the oldest correction. A first build
//! stamps `cutoff` with its own time; a rebuild keeps the stored bundle's
//! `cutoff`, `since` and `listed` (re-read by slug), reads the calendar as it
//! now stands (`as_of`), and counts its `revision`. Both languages are
//! rendered and checked (`guard`) before anything is written; then the
//! bundle goes to R2, a first build writes the post row without text
//! (invisible) and both translations land in one transaction
//! (`revise_post`). A rebuild never calls `upsert_post` (which overwrites
//! `published_at`): it publishes through `revise_post` with `revised_at`
//! set. A first build below the minimum is `skipped`, logged, never alerted.
//! A failed build retries after 5, 15, 30 and 60 minutes with a warn alert;
//! the fifth fails the run with an error alert. From 21:55 ET on the Friday,
//! a run neither published nor skipped raises one error alert.
//!
//! **Stale checks.** A first build also reads the last four published week
//! bundles and raises one warn alert per post (`week-stale:<friday>`) when a
//! check it listed was rebuilt or revised after its `cutoff`.

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use std::collections::HashSet;

use super::render::{cover_lines, write};
use super::store::{WeekRun, WeekStore};
use super::{
    assemble, checks, enabled, facts_key, friday_of, iso, monday_of, run_key, slug, Bundle, Stamp,
    CONTENT_TYPE, TEMPLATE_ID,
};
use crate::clock::et;
use crate::digest::{closing, due, last_try, week_days};
use crate::earnings::store::EarningsStore;
use crate::events::store::EventStore;
use crate::jobs::alert_once_today;
use crate::numbers::Lang;
use crate::schedule::EarningsStep;
use crate::store::{PostRow, RunRow, Store, TranslationRow};

/// Pauses after the 1st, 2nd, 3rd and later failed builds.
pub const BACKOFF_MINUTES: [i64; 4] = [5, 15, 30, 60];
/// Builds a run gets before it is `failed`.
pub const MAX_BUILDS: i64 = 5;
/// A `building` run younger than this is in flight.
pub const LEASE_MINUTES: i64 = 15;
/// Earlier week bundles a first build reads (the stale check).
pub const PREVIOUS: usize = 4;

fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn since(s: &str, now: DateTime<Utc>) -> Duration {
    parse_iso(s).map_or(Duration::MAX, |t| now - t)
}

fn leased(r: &RunRow, now: DateTime<Utc>) -> bool {
    r.status == "building" && since(&r.updated_at, now) < Duration::minutes(LEASE_MINUTES)
}

fn backoff(attempts: i64) -> Duration {
    let i = usize::try_from(attempts.max(1) - 1).unwrap_or(0);
    Duration::minutes(BACKOFF_MINUTES[i.min(BACKOFF_MINUTES.len() - 1)])
}

fn waiting(r: &RunRow, now: DateTime<Utc>) -> bool {
    r.status == "pending"
        && r.attempts > 0
        && parse_iso(&r.updated_at).is_some_and(|t| now < t + backoff(r.attempts))
}

fn done(r: &RunRow) -> bool {
    matches!(r.status.as_str(), "published" | "skipped" | "failed")
}

/// Today, when it is a Friday (ET).
pub fn this_friday(now: DateTime<Utc>) -> Option<NaiveDate> {
    let d = et(now).date_naive();
    (d.weekday() == Weekday::Fri).then_some(d)
}

/// A correction to build: a live post's run set back to `pending` (not
/// waiting to retry), or left `building` by a crash (lease expired).
fn correction(w: &WeekRun, now: DateTime<Utc>) -> bool {
    w.live
        && friday_of(&w.run.run_key).is_some()
        && match w.run.status.as_str() {
            "pending" => !waiting(&w.run, now),
            "building" => !leased(&w.run, now),
            _ => false,
        }
}

/// What a poll at `now` queues (module docs); off is nothing, with no read.
pub async fn follow_up(
    raw: Option<&str>,
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    ws: &dyn WeekStore,
) -> Result<Option<EarningsStep>, String> {
    if !enabled(raw) || !crate::earnings::in_window(now) {
        return Ok(None);
    }
    let friday = this_friday(now).filter(|_| due(now));
    let key = friday.map(run_key).unwrap_or_default();
    let runs = ws.week_runs(&key).await?;
    if runs
        .iter()
        .any(|w| w.run.run_key != key && correction(w, now))
    {
        return Ok(Some(EarningsStep::Week));
    }
    if friday.is_none() {
        return Ok(None);
    }
    let run = runs.iter().find(|w| w.run.run_key == key);
    if let Some(w) = run {
        if done(&w.run) {
            return Ok(None);
        }
        if w.live && correction(w, now) {
            return Ok(Some(EarningsStep::Week));
        }
        if !last_try(now) && (leased(&w.run, now) || waiting(&w.run, now)) {
            return Ok(None);
        }
    }
    let ready = last_try(now)
        || closing(now)
        || crate::earnings::calendar::refreshed_today(es, now).await?;
    Ok(ready.then_some(EarningsStep::Week))
}

fn new_run(friday: NaiveDate, now: DateTime<Utc>) -> RunRow {
    RunRow {
        run_key: run_key(friday),
        content_type: CONTENT_TYPE.into(),
        trading_date: super::ymd(friday),
        status: "pending".into(),
        attempts: 0,
        last_error: None,
        created_at: iso(now),
        updated_at: iso(now),
    }
}

/// One `week` message (module docs). `Ok` is the log line.
pub async fn week_tick(
    now: DateTime<Utc>,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    store: &dyn Store,
    ws: &dyn WeekStore,
) -> Result<String, String> {
    let friday = this_friday(now).filter(|_| due(now));
    let key = friday.map(run_key).unwrap_or_default();
    let runs = ws.week_runs(&key).await?;
    let mut out: Vec<String> = Vec::new();
    let mut built = false;

    if let Some(f) = friday {
        let w = runs.iter().find(|w| w.run.run_key == key);
        let what = match w {
            Some(w) if done(&w.run) => None,
            Some(w) if w.live && correction(w, now) => {
                built = true;
                Some(correct(now, f, w, es, events, store, ws).await?)
            }
            Some(w) if leased(&w.run, now) => Some("in flight".to_string()),
            Some(w) if waiting(&w.run, now) => Some("waiting to retry".to_string()),
            Some(w) if w.run.attempts >= MAX_BUILDS => {
                let err = w
                    .run
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "the build never finished".into());
                Some(give_up(now, f, w.run.clone(), err, store).await?)
            }
            _ => {
                let ready =
                    closing(now) || crate::earnings::calendar::refreshed_today(es, now).await?;
                if ready {
                    built = true;
                    let run = w.map(|w| w.run.clone());
                    let live = w.is_some_and(|w| w.live);
                    Some(build(now, f, run, live, es, events, store, ws).await?)
                } else {
                    Some("waiting for today's calendar refresh to finish".to_string())
                }
            }
        };
        if let Some(what) = what {
            out.push(format!("{key}: {what}"));
        }
    }

    if !built {
        if let Some(w) = runs
            .iter()
            .filter(|w| w.run.run_key != key)
            .find(|w| correction(w, now))
        {
            let f = friday_of(&w.run.run_key).expect("checked by correction");
            let what = correct(now, f, w, es, events, store, ws).await?;
            out.push(format!("{}: {what}", w.run.run_key));
        }
    }

    if let (Some(f), true) = (friday, last_try(now)) {
        let open = store
            .get_run(&key)
            .await?
            .is_none_or(|r| !matches!(r.status.as_str(), "published" | "skipped"));
        if open {
            let msg = format!("week post for {} not published", super::ymd(f));
            if alert_once_today(now, "week-missed", "error", &msg, store).await? {
                out.push(format!("{key}: alerted, not published by the last try"));
            }
        }
    }
    if out.is_empty() {
        out.push("nothing to build".into());
    }
    Ok(out.join("; "))
}

/// A live post's run to build: a first build that died after its batch is
/// closed; a correction whose builds are spent (store errors leave it
/// `building`, picked up again after each lease) fails with an alert;
/// otherwise it is rebuilt.
#[allow(clippy::too_many_arguments)]
async fn correct(
    now: DateTime<Utc>,
    friday: NaiveDate,
    w: &WeekRun,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    store: &dyn Store,
    ws: &dyn WeekStore,
) -> Result<String, String> {
    let crashed = w.run.status == "building" && w.run.last_error.as_deref() != Some(REBUILDING);
    if !crashed && w.run.attempts >= MAX_BUILDS {
        let err = format!(
            "the correction did not finish in {MAX_BUILDS} builds; the post stays as it was"
        );
        return give_up(now, friday, w.run.clone(), err, store).await;
    }
    build(
        now,
        friday,
        Some(w.run.clone()),
        true,
        es,
        events,
        store,
        ws,
    )
    .await
}

/// The stored bundle of a live post.
async fn stored(store: &dyn Store, post: &PostRow) -> Result<Bundle, String> {
    let json = store
        .get_facts(&post.facts_r2_key)
        .await?
        .ok_or_else(|| format!("bundle {} missing", post.facts_r2_key))?;
    serde_json::from_str(&json).map_err(|e| format!("bundle {}: {e}", post.facts_r2_key))
}

/// The published week posts before `friday`, with their bundles, newest
/// first (a missing or unreadable bundle is left out).
async fn previous(
    friday: NaiveDate,
    store: &dyn Store,
    ws: &dyn WeekStore,
) -> Result<Vec<Bundle>, String> {
    let mut out = Vec::new();
    for p in ws.week_posts(&super::ymd(friday), PREVIOUS).await? {
        if let Ok(b) = stored(store, &p).await {
            out.push(b);
        }
    }
    Ok(out)
}

/// One warn alert per earlier post whose listed checks were rebuilt or
/// revised after it read them (`as_of`: the first build's cutoff, or a
/// correction's time) (`week-stale:<friday>`, once only).
async fn stale(prev: &[Bundle], store: &dyn Store, ws: &dyn WeekStore) -> Result<(), String> {
    let slugs: Vec<String> = prev
        .iter()
        .flat_map(|b| b.listed.iter().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if slugs.is_empty() {
        return Ok(());
    }
    let written = ws.written(&slugs).await?;
    for b in prev {
        let mut late: Vec<&str> = written
            .iter()
            .filter(|w| b.listed.contains(&w.slug))
            .filter(|w| {
                w.revised_at
                    .as_deref()
                    .or(w.published_at.as_deref())
                    // A rebuild re-read its checks at `as_of`.
                    .is_some_and(|t| t > b.as_of.as_str())
            })
            .map(|w| w.slug.as_str())
            .collect();
        late.sort_unstable();
        if late.is_empty() {
            continue;
        }
        let key = format!("week-stale:{}", b.friday);
        if store.has_alert(&key).await? {
            continue;
        }
        let msg = format!(
            "week post for {}: {} changed after the post was built ({}); its figures there may be out of date. Rebuild that week if needed (runbook, \"Earnings week post\")",
            b.friday,
            if late.len() == 1 { "a listed check" } else { "listed checks" },
            late.join(", ")
        );
        store.add_alert(&key, "warn", &msg).await?;
    }
    Ok(())
}

fn translation(
    post_id: i64,
    b: &Bundle,
    lang: Lang,
    title: String,
    summary: String,
    md: String,
    ts: &str,
) -> TranslationRow {
    let [cover_line1, cover_line2] = cover_lines(b, lang);
    TranslationRow {
        post_id,
        lang: match lang {
            Lang::En => "en",
            Lang::Zh => "zh",
        }
        .into(),
        status: "published".into(),
        title,
        summary,
        body_md: md,
        updated_at: ts.into(),
        cover_line1: Some(cover_line1),
        cover_line2: Some(cover_line2),
    }
}

/// Both languages, each checked by `guard::verify_page`.
pub fn render_both(b: &Bundle) -> Result<[(String, String, String); 2], String> {
    let mut out = Vec::new();
    let mut err = Vec::new();
    for lang in [Lang::En, Lang::Zh] {
        let (d, md) = write(b, lang);
        match super::guard::verify_page(b, lang, &md, &cover_lines(b, lang)) {
            Ok(()) => out.push((d.title, d.summary, md)),
            Err(e) => err.push(format!(
                "{}: {}",
                if lang == Lang::En { "en" } else { "zh" },
                e.join("; ")
            )),
        }
    }
    match <[_; 2]>::try_from(out) {
        Ok(both) if err.is_empty() => Ok(both),
        _ => Err(err.join(" | ")),
    }
}

/// The bundle a build at `now` writes (module docs): `previous` is the
/// earlier posts' bundles (newest first), `old` the stored bundle of a
/// rebuild.
#[allow(clippy::too_many_arguments)]
pub async fn bundle_for(
    now: DateTime<Utc>,
    friday: NaiveDate,
    old: Option<&Bundle>,
    prev: &[Bundle],
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    ws: &dyn WeekStore,
) -> Result<Bundle, String> {
    let (cutoff, revision, since) = match old {
        Some(o) => (
            parse_iso(&o.cutoff).unwrap_or(now),
            o.revision + 1,
            o.since.parse().unwrap_or(monday_of(friday)),
        ),
        None => (
            now,
            0,
            prev.first().map_or(monday_of(friday), Bundle::monday),
        ),
    };
    let (rows, exclude) = match old {
        Some(o) => (ws.checks_by_slug(&o.listed).await?, Vec::new()),
        None => (
            ws.week_checks(&super::ymd(since), &iso(cutoff)).await?,
            prev.first().map(|b| b.listed.clone()).unwrap_or_default(),
        ),
    };
    let listed = checks(rows, &exclude);
    let next_monday = friday + Duration::days(3);
    let days = week_days(now, next_monday, es, events).await?;
    Ok(assemble(
        &Stamp {
            friday,
            since,
            cutoff,
            as_of: now,
            revision,
        },
        listed,
        &days,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn build(
    now: DateTime<Utc>,
    friday: NaiveDate,
    run: Option<RunRow>,
    live: bool,
    es: &dyn EarningsStore,
    events: &dyn EventStore,
    store: &dyn Store,
    ws: &dyn WeekStore,
) -> Result<String, String> {
    let ts = iso(now);
    let mut row = run.unwrap_or_else(|| new_run(friday, now));
    let post = store.get_post_by_run(&row.run_key).await?;
    // A first build that died after the batch: the text is live and no
    // rebuild was asked for (a rebuild marks itself, `REBUILDING`).
    if row.status == "building" && live && row.last_error.as_deref() != Some(REBUILDING) {
        row.status = "published".into();
        row.last_error = None;
        row.updated_at = ts;
        store.upsert_run(&row).await?;
        return Ok("published (run closed)".into());
    }
    row.status = "building".into();
    row.attempts += 1;
    row.last_error = live.then(|| REBUILDING.to_string());
    row.updated_at = ts.clone();
    store.upsert_run(&row).await?;

    let old = match (&post, live) {
        (Some(p), true) => match stored(store, p).await {
            Ok(b) => Some(b),
            Err(e) => return not_ready(now, friday, row, e, store).await,
        },
        _ => None,
    };
    let prev = previous(friday, store, ws).await?;
    let b = bundle_for(now, friday, old.as_ref(), &prev, es, events, ws).await?;
    if old.is_none() {
        stale(&prev, store, ws).await?;
        if !b.enough() {
            let why = format!(
                "below the minimum: {} checks, {} write-up companies expected next week",
                b.counts.checks, b.counts.writeups
            );
            row.status = "skipped".into();
            row.last_error = Some(why.clone());
            row.updated_at = ts;
            store.upsert_run(&row).await?;
            return Ok(format!("skipped: {why}"));
        }
    }
    let [en, zh] = match render_both(&b) {
        Ok(both) => both,
        Err(e) => return not_ready(now, friday, row, e, store).await,
    };
    let json = serde_json::to_string(&b).map_err(|e| format!("bundle json: {e}"))?;
    store.put_facts(&facts_key(friday), &json).await?;
    let (p, revised) = match (post, old.is_some()) {
        (Some(mut p), true) => {
            p.facts_r2_key = facts_key(friday);
            p.revised_at = Some(ts.clone());
            (p, true)
        }
        (_, _) => {
            let mut p = PostRow {
                id: None,
                run_key: row.run_key.clone(),
                content_type: CONTENT_TYPE.into(),
                template_id: TEMPLATE_ID.into(),
                trading_date: super::ymd(friday),
                slug: slug(friday),
                facts_r2_key: facts_key(friday),
                published_at: Some(ts.clone()),
                cover_key: None,
                revised_at: None,
            };
            // Invisible until the batch below: no translation yet.
            p.id = Some(store.upsert_post(&p).await?);
            (p, false)
        }
    };
    let id = p.id.ok_or("post has no id")?;
    let rows = [
        translation(id, &b, Lang::En, en.0, en.1, en.2, &ts),
        translation(id, &b, Lang::Zh, zh.0, zh.1, zh.2, &ts),
    ];
    // Both languages and the events' official links (the site links only a
    // post's own sources and same-site paths) in one transaction.
    store.revise_post(&p, &rows, &event_sources(&b)).await?;
    row.status = "published".into();
    row.last_error = None;
    row.updated_at = ts;
    store.upsert_run(&row).await?;
    Ok(if revised {
        format!("revised {} (revision {})", p.slug, b.revision)
    } else {
        format!(
            "published {} ({} checks, {} expected)",
            p.slug, b.counts.checks, b.counts.expected
        )
    })
}

/// The events' official pages, once each, as the post's sources.
pub fn event_sources(b: &Bundle) -> Vec<crate::facts::NewsReason> {
    let mut out: Vec<crate::facts::NewsReason> = Vec::new();
    for e in &b.events {
        if out.iter().any(|s| s.url == e.link) {
            continue;
        }
        out.push(crate::facts::NewsReason {
            url: e.link.clone(),
            title: super::render::event_title(e, Lang::En).unwrap_or_else(|| e.series.clone()),
            quote: String::new(),
            summary_en: String::new(),
        });
    }
    out
}

/// What a rebuild leaves in `last_error` while it runs, so a crash in it is
/// rebuilt again rather than closed.
pub const REBUILDING: &str = "rebuilding";

async fn give_up(
    now: DateTime<Utc>,
    friday: NaiveDate,
    mut row: RunRow,
    err: String,
    store: &dyn Store,
) -> Result<String, String> {
    row.status = "failed".into();
    row.last_error = Some(err.clone());
    row.updated_at = iso(now);
    store.upsert_run(&row).await?;
    store
        .add_alert(
            &row.run_key,
            "error",
            &format!("week post for {} not published: {err}", super::ymd(friday)),
        )
        .await?;
    Ok(format!("failed: {err}"))
}

async fn not_ready(
    now: DateTime<Utc>,
    friday: NaiveDate,
    mut row: RunRow,
    err: String,
    store: &dyn Store,
) -> Result<String, String> {
    if row.attempts >= MAX_BUILDS {
        return give_up(now, friday, row, err, store).await;
    }
    row.status = "pending".into();
    row.last_error = Some(err.clone());
    row.updated_at = iso(now);
    store.upsert_run(&row).await?;
    let msg = format!(
        "week post for {} not published yet (build {} of {MAX_BUILDS}, retried after {} minutes): {err}",
        super::ymd(friday),
        row.attempts,
        backoff(row.attempts).num_minutes()
    );
    alert_once_today(now, &row.run_key, "warn", &msg, store).await?;
    Ok(format!("retrying: {err}"))
}
