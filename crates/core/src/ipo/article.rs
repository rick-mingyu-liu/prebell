//! IPO write-ups at filing milestones (design
//! `docs/superpowers/specs/2026-11-11-ipo-articles-design.md`, contract
//! `2026-11-11-ipo-articles-contract.md`): the `ipo:article` message.
//!
//! An earnings poll queues it (`follow_up`) while `IPO_ARTICLES_ENABLED`
//! and `IPO_WATCH_ENABLED` are both exactly `true`, once the kv time
//! `ipo_article_next` is reached: an hour after an idle pass, five minutes
//! after a pass that left a retry. It rides the serial earnings-build
//! queue, so two builds never run at once.
//!
//! Each message reads the IPO state once and, for each tracked IPO, its
//! latest milestone (`milestone::latest`). The run key
//! `ipo:<ipo_slug>:<milestone>:<key>` makes it once only: a run that is
//! `published`, `skipped` or `failed` is never built again. At most one
//! build per message. The text is written by code (`writeup`): no model
//! call, so a build is a few D1 queries and one R2 write.
//!
//! **Never half published.** Both languages are rendered and checked
//! (`writeup::verify_page`) before anything is written. Then the facts go
//! to R2, the post row is written without text (invisible: the site shows
//! a post only by a published translation), and both translations and the
//! sources land in one D1 batch (`revise_post`, one transaction). A store
//! failure before that batch leaves nothing visible and the next pass
//! writes into the same post (`upsert_post` keeps one per run key); one
//! after it finds the text live and only closes the run.
//!
//! **Late, unconfirmed or failing.** A milestone found more than
//! `LATE_DAYS` after its filing or session is `skipped` with a warn alert,
//! never published late. `listed` waits until the stored session is the
//! filings' first trading day (`Reached::confirmed`); one never confirmed
//! is skipped by the same rule. A build that fails (a store or R2 error, or
//! a page the guard refuses) is retried after 5, 15, 30 and 60 minutes; the
//! fifth failed build marks the run `failed` with an error alert, so a
//! milestone costs at most `MAX_BUILDS` builds over about two hours.
//!
//! **Cover and share card.** A build draws the milestone's cover graphic
//! from the facts (`cover::ipo`) and writes it to R2 as
//! `covers/<post slug>.svg` before the post row, so the post goes live with
//! its `cover_key`; a withdrawal, a milestone missing a figure, or a failed
//! R2 write leaves it NULL (the flat tile) and never fails the build. The
//! message then queues a `cards` message, which renders the share card in
//! an invocation of its own (`cover::card`).
//!
//! **Cover backfill.** A message that built nothing gives at most one
//! already-published write-up without a cover its cover, recomputed from
//! its stored facts bundle: a write-up of the last `card::WINDOW_DAYS` days
//! (older ones would get no card), not a withdrawal, with its text live.
//! Its attempts are counted in R2 (`covers/<post slug>.attempts`, never
//! served) before each try, `COVER_TRIES` at most; facts that give no
//! graphic use them all up at once. Setting `cover_key` (`upsert_post`,
//! which keeps `published_at` and `revised_at`) makes it once only.

use chrono::{DateTime, Duration, Utc};

use super::milestone::{bundle, latest, reached, IpoArticleFacts, Milestone, Reached};
use super::store::{IpoState, IpoStore};
use super::writeup::{self, cover_lines, source_rows, template_id, CONTENT_TYPE};
use super::{iso, Tracked};
use crate::clock::{et, trading_day, DayKind};
use crate::cover::card::WINDOW_DAYS;
use crate::cover::ipo::ipo_graphic;
use crate::cover::store_graphic;
use crate::earnings::store::EarningsStore;
use crate::jobs::alert_once_today;
use crate::numbers::Lang;
use crate::premarket::draft::Draft;
use crate::schedule::EarningsStep;
use crate::store::{PostRow, RunRow, Store, TranslationRow};

/// kv: when a poll next queues `ipo:article` (RFC 3339).
pub const KV_NEXT: &str = "ipo_article_next";
/// A pass with nothing to retry looks again this much later.
pub const IDLE_MINUTES: i64 = 60;
/// Pauses after the 1st, 2nd, 3rd and later failed builds.
pub const BACKOFF_MINUTES: [i64; 4] = [5, 15, 30, 60];
/// Builds a run gets before it is `failed`.
pub const MAX_BUILDS: i64 = 5;
/// A `building` run younger than this is in flight.
const LEASE_MINUTES: i64 = 15;
/// A milestone older than this (ET days from its filing or session) when
/// first found is skipped.
pub const LATE_DAYS: i64 = 7;
/// Tries the cover backfill gives one write-up.
pub const COVER_TRIES: u32 = 3;

/// `IPO_ARTICLES_ENABLED`: on only when it is exactly `true`.
pub fn enabled(raw: Option<&str>) -> bool {
    raw == Some("true")
}

/// The `ipo:article` message an earnings poll at `now` queues: with both
/// switches on, on a trading day in the polling window, once `KV_NEXT` is
/// reached (or unset or unreadable). Off is nothing, without touching the
/// store.
pub async fn follow_up(
    articles: Option<&str>,
    watch: Option<&str>,
    list: &[Tracked],
    es: &dyn EarningsStore,
    now: DateTime<Utc>,
) -> Result<Option<EarningsStep>, String> {
    let trading = trading_day(et(now).date_naive()) != DayKind::Closed;
    if !enabled(articles)
        || !super::enabled(watch)
        || list.is_empty()
        || !crate::earnings::in_window(now)
        || !trading
    {
        return Ok(None);
    }
    let due = match es.kv_get(KV_NEXT).await? {
        Some(t) => DateTime::parse_from_rfc3339(&t).map_or(true, |t| now >= t.with_timezone(&Utc)),
        None => true,
    };
    Ok(due.then_some(EarningsStep::IpoArticle))
}

/// What one message did, per run key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArticleOutcome {
    /// `(run_key, what happened)`.
    pub runs: Vec<(String, String)>,
    /// `KV_NEXT` as written.
    pub next: String,
    /// Whether a write-up went live.
    pub published: bool,
    /// Whether a write-up got its cover (built with one, or backfilled), so
    /// its share card should be rendered now.
    pub card: bool,
}

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

/// The pause after `attempts` failed builds.
fn backoff(attempts: i64) -> Duration {
    let i = usize::try_from(attempts.max(1) - 1).unwrap_or(0);
    Duration::minutes(BACKOFF_MINUTES[i.min(BACKOFF_MINUTES.len() - 1)])
}

/// When a failed run may be built again.
fn retry_at(r: &RunRow) -> Option<DateTime<Utc>> {
    parse_iso(&r.updated_at).map(|t| t + backoff(r.attempts))
}

fn waiting(r: &RunRow, now: DateTime<Utc>) -> bool {
    r.status == "pending" && r.attempts > 0 && retry_at(r).is_some_and(|t| now < t)
}

fn spent(r: &RunRow) -> bool {
    r.attempts >= MAX_BUILDS
}

fn late(r: &Reached, now: DateTime<Utc>) -> i64 {
    (et(now).date_naive() - r.as_of).num_days()
}

fn label(t: &Tracked, r: &Reached) -> String {
    format!("{} {} ({})", t.name, r.milestone.as_str(), r.key)
}

/// One `ipo:article` message (module docs).
pub async fn article_tick(
    now: DateTime<Utc>,
    list: &[Tracked],
    ipo: &dyn IpoStore,
    es: &dyn EarningsStore,
    store: &dyn Store,
) -> Result<ArticleOutcome, String> {
    // Marked first, so a pass that fails is not queued by every poll.
    let idle = now + Duration::minutes(IDLE_MINUTES);
    es.kv_set(KV_NEXT, &iso(idle), &iso(now)).await?;
    let st = ipo.ipo_state().await?;
    let mut out = ArticleOutcome::default();
    let mut next = idle;
    let soon = now + backoff(1);
    let mut built = false;
    for t in list {
        let Some(r) = latest(&st, &t.slug) else {
            continue;
        };
        let key = r.run_key(&t.slug);
        let run = store.get_run(&key).await?;
        let what = match run {
            Some(run) if matches!(run.status.as_str(), "published" | "skipped" | "failed") => {
                continue
            }
            None if late(&r, now) > LATE_DAYS => skip_late(now, store, t, &r).await?,
            // The stored session is not yet the filings' first trading day.
            None if !r.confirmed => "waiting for the first trading day to be confirmed".into(),
            Some(run) if leased(&run, now) => {
                if let Some(t) = parse_iso(&run.updated_at) {
                    next = next.min(t + Duration::minutes(LEASE_MINUTES));
                }
                "in flight".to_string()
            }
            Some(run) if waiting(&run, now) => {
                next = next.min(retry_at(&run).unwrap_or(soon));
                "waiting to retry".to_string()
            }
            Some(run) if spent(&run) => {
                let err = run
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "the build never finished".into());
                give_up(now, store, t, &r, run, err).await?
            }
            _ if built => {
                next = next.min(soon);
                "waiting behind another build".to_string()
            }
            run => {
                built = true;
                let b = build(now, &st, es, store, t, &r, run).await?;
                if let Some(at) = b.retry {
                    next = next.min(at);
                }
                out.published |= b.published;
                out.card |= b.card;
                b.what
            }
        };
        out.runs.push((key, what));
    }
    // One build or one backfill per message.
    if !built {
        match backfill_cover(now, &st, list, store).await {
            Ok(Some((key, what, card))) => {
                out.card |= card;
                out.runs.push((key, what));
            }
            Ok(None) => {}
            // Never fails the pass: the next one tries again.
            Err(e) => out
                .runs
                .push(("cover backfill".into(), format!("failed: {e}"))),
        }
    }
    if next != idle {
        es.kv_set(KV_NEXT, &iso(next), &iso(now)).await?;
    }
    out.next = iso(next);
    Ok(out)
}

fn new_run(key: &str, r: &Reached, now: DateTime<Utc>) -> RunRow {
    RunRow {
        run_key: key.into(),
        content_type: CONTENT_TYPE.into(),
        trading_date: super::ymd(r.as_of),
        status: "pending".into(),
        attempts: 0,
        last_error: None,
        created_at: iso(now),
        updated_at: iso(now),
    }
}

async fn skip_late(
    now: DateTime<Utc>,
    store: &dyn Store,
    t: &Tracked,
    r: &Reached,
) -> Result<String, String> {
    let key = r.run_key(&t.slug);
    let err = if r.confirmed {
        format!(
            "found {} days after {}, more than {LATE_DAYS}: not published late",
            late(r, now),
            super::ymd(r.as_of)
        )
    } else {
        format!(
            "the session of {} was never confirmed as the first trading day within {LATE_DAYS} days",
            super::ymd(r.as_of)
        )
    };
    let mut row = new_run(&key, r, now);
    row.status = "skipped".into();
    row.last_error = Some(err.clone());
    store.upsert_run(&row).await?;
    store
        .add_alert(
            &key,
            "warn",
            &format!("IPO write-up for {} skipped: {err}", label(t, r)),
        )
        .await?;
    Ok(format!("skipped: {err}"))
}

async fn give_up(
    now: DateTime<Utc>,
    store: &dyn Store,
    t: &Tracked,
    r: &Reached,
    mut row: RunRow,
    err: String,
) -> Result<String, String> {
    row.status = "failed".into();
    row.last_error = Some(err.clone());
    row.updated_at = iso(now);
    store.upsert_run(&row).await?;
    store
        .add_alert(
            &row.run_key,
            "error",
            &format!("IPO write-up for {} not published: {err}", label(t, r)),
        )
        .await?;
    Ok(format!("failed: {err}"))
}

struct Built {
    what: String,
    /// When to come back for a failed build.
    retry: Option<DateTime<Utc>>,
    published: bool,
    /// Published with a cover.
    card: bool,
}

/// The post's slug (contract), with the key's tail when another run owns it.
async fn post_slug(es: &dyn EarningsStore, f: &IpoArticleFacts) -> Result<String, String> {
    let base = format!(
        "{}-ipo-{}-{}",
        f.ipo_slug,
        f.milestone.slug_words(),
        f.as_of
    );
    Ok(match es.slug_owner(&base).await? {
        Some(owner) if owner != f.run_key => {
            let compact: String = f
                .key
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            format!("{base}-{}", &compact[compact.len().saturating_sub(6)..])
        }
        _ => base,
    })
}

fn translation(
    post_id: i64,
    f: &IpoArticleFacts,
    d: &Draft,
    md: String,
    ts: &str,
) -> TranslationRow {
    let [cover_line1, cover_line2] = cover_lines(f, d.lang);
    TranslationRow {
        post_id,
        lang: match d.lang {
            Lang::En => "en",
            Lang::Zh => "zh",
        }
        .into(),
        status: "published".into(),
        title: d.title.clone(),
        summary: d.summary.clone(),
        body_md: md,
        updated_at: ts.into(),
        cover_line1: Some(cover_line1),
        cover_line2: Some(cover_line2),
    }
}

async fn build(
    now: DateTime<Utc>,
    st: &IpoState,
    es: &dyn EarningsStore,
    store: &dyn Store,
    t: &Tracked,
    r: &Reached,
    run: Option<RunRow>,
) -> Result<Built, String> {
    let key = r.run_key(&t.slug);
    let ts = iso(now);
    let mut row = run.unwrap_or_else(|| new_run(&key, r, now));

    // The text is live already (a failure after the batch): close the run.
    if let Some(post) = store.get_post_by_run(&key).await? {
        let id = post.id.ok_or("post has no id")?;
        if store
            .get_translation(id, "en")
            .await?
            .is_some_and(|x| x.status == "published")
        {
            row.status = "published".into();
            row.last_error = None;
            row.updated_at = ts;
            store.upsert_run(&row).await?;
            return Ok(Built {
                what: "published (run closed)".into(),
                retry: None,
                published: false,
                card: false,
            });
        }
    }

    row.status = "building".into();
    row.attempts += 1;
    row.updated_at = ts.clone();
    store.upsert_run(&row).await?;

    let f = bundle(st, t, r, now);
    let en = writeup::render(&f, Lang::En);
    let zh = writeup::render(&f, Lang::Zh);
    let ((en, en_md), (zh, zh_md)) = match (en, zh) {
        (Ok(a), Ok(b)) => (a, b),
        (en, zh) => {
            let mut err = Vec::new();
            if let Err(e) = en {
                err.push(format!("en: {}", e.join("; ")));
            }
            if let Err(e) = zh {
                err.push(format!("zh: {}", e.join("; ")));
            }
            return not_ready(now, store, t, r, row, err.join(" | ")).await;
        }
    };

    // Another message finished it meanwhile.
    if store
        .get_run(&key)
        .await?
        .is_some_and(|x| matches!(x.status.as_str(), "published" | "failed" | "skipped"))
    {
        return Ok(Built {
            what: "finished elsewhere".into(),
            retry: None,
            published: false,
            card: false,
        });
    }
    let facts_key = f.facts_key();
    let json = serde_json::to_string(&f).map_err(|e| format!("facts json: {e}"))?;
    store.put_facts(&facts_key, &json).await?;
    let slug = post_slug(es, &f).await?;
    // Never fails the build: without a cover the post keeps the flat tile.
    let (cover_key, cover, cover_err) = store_graphic(store, &slug, ipo_graphic(&f)).await;
    let mut post = PostRow {
        id: None,
        run_key: key.clone(),
        content_type: CONTENT_TYPE.into(),
        template_id: template_id(f.milestone),
        trading_date: f.as_of.clone(),
        slug,
        facts_r2_key: facts_key,
        published_at: Some(ts.clone()),
        cover_key,
        revised_at: None,
    };
    // Invisible until the batch below: no translation yet.
    let id = store.upsert_post(&post).await?;
    post.id = Some(id);
    let rows = [
        translation(id, &f, &en, en_md, &ts),
        translation(id, &f, &zh, zh_md, &ts),
    ];
    // Both languages and the sources in one transaction.
    store.revise_post(&post, &rows, &source_rows(&f)).await?;
    row.status = "published".into();
    row.last_error = None;
    row.updated_at = ts;
    store.upsert_run(&row).await?;
    let cover = match cover_err {
        Some(e) => format!("{cover}: {e}"),
        None => cover.to_string(),
    };
    Ok(Built {
        what: format!("published {} (cover {cover})", post.slug),
        retry: None,
        published: true,
        card: post.cover_key.is_some(),
    })
}

/// The cover backfill (module docs): `(run_key, what happened, whether a
/// cover was added)` for the one write-up it tried, or `None`.
async fn backfill_cover(
    now: DateTime<Utc>,
    st: &IpoState,
    list: &[Tracked],
    store: &dyn Store,
) -> Result<Option<(String, String, bool)>, String> {
    for t in list {
        for r in reached(st, &t.slug) {
            if r.milestone == Milestone::Withdrawn || late(&r, now) > WINDOW_DAYS {
                continue;
            }
            let key = r.run_key(&t.slug);
            let Some(mut post) = store.get_post_by_run(&key).await? else {
                continue;
            };
            let Some(id) = post.id else { continue };
            if post.cover_key.is_some() || post.published_at.is_none() {
                continue;
            }
            // Read first, so a given-up write-up costs no translation read.
            let counter = format!("covers/{}.attempts", post.slug);
            let tried: u32 = match store.get_facts(&counter).await? {
                Some(n) => n.trim().parse().unwrap_or(COVER_TRIES),
                None => 0,
            };
            if tried >= COVER_TRIES {
                continue;
            }
            // Only a live write-up: a half-built one gets its cover from its build.
            if !store
                .get_translation(id, "en")
                .await?
                .is_some_and(|x| x.status == "published")
            {
                continue;
            }
            // Counted first, so a pass that dies here still counts.
            store.put_facts(&counter, &(tried + 1).to_string()).await?;
            let svg = match store.get_facts(&post.facts_r2_key).await? {
                Some(json) => match serde_json::from_str::<IpoArticleFacts>(&json) {
                    Ok(f) => ipo_graphic(&f),
                    Err(e) => {
                        let what = format!("no cover: bad facts bundle: {e}");
                        return Ok(Some((key, what, false)));
                    }
                },
                None => {
                    let what = format!("no cover: facts bundle {} missing", post.facts_r2_key);
                    return Ok(Some((key, what, false)));
                }
            };
            let Some(svg) = svg else {
                // The facts give no graphic, now or later: not tried again.
                store.put_facts(&counter, &COVER_TRIES.to_string()).await?;
                return Ok(Some((
                    key,
                    "no cover: the facts give no graphic".into(),
                    false,
                )));
            };
            let (cover_key, outcome, err) = store_graphic(store, &post.slug, Some(svg)).await;
            let Some(cover_key) = cover_key else {
                let what = format!(
                    "no cover: {outcome} (try {} of {COVER_TRIES}): {}",
                    tried + 1,
                    err.unwrap_or_default()
                );
                return Ok(Some((key, what, false)));
            };
            post.cover_key = Some(cover_key.clone());
            store.upsert_post(&post).await?;
            return Ok(Some((key, format!("cover added: {cover_key}"), true)));
        }
    }
    Ok(None)
}

async fn not_ready(
    now: DateTime<Utc>,
    store: &dyn Store,
    t: &Tracked,
    r: &Reached,
    mut row: RunRow,
    err: String,
) -> Result<Built, String> {
    if spent(&row) {
        let what = give_up(now, store, t, r, row, err).await?;
        return Ok(Built {
            what,
            retry: None,
            published: false,
            card: false,
        });
    }
    row.status = "pending".into();
    row.last_error = Some(err.clone());
    row.updated_at = iso(now);
    store.upsert_run(&row).await?;
    let at = retry_at(&row);
    let msg = format!(
        "IPO write-up for {} not published yet (build {} of {MAX_BUILDS}, retried after {} minutes): {err}",
        label(t, r),
        row.attempts,
        backoff(row.attempts).num_minutes()
    );
    alert_once_today(now, &row.run_key, "warn", &msg, store).await?;
    Ok(Built {
        what: format!("retrying: {err}"),
        retry: at,
        published: false,
        card: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::store::MemEarningsStore;
    use crate::ipo::milestone::tests::{anthropic, real_state};
    use crate::ipo::milestone::IpoArticleFacts;
    use crate::ipo::store::{FactRow, FilingRow, MemIpoStore, TradingRow};
    use crate::store::MemStore;
    use serde_json::{json, Value};
    use std::rc::Rc;

    const RUN: &str = "ipo:anthropic:range_set:0001628280-26-072484";
    const NOW: &str = "2026-11-11T22:00:00Z";

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    struct World {
        ipo: MemIpoStore,
        base: Rc<MemStore>,
        es: MemEarningsStore,
    }
    fn world() -> World {
        let base = Rc::new(MemStore::default());
        let ipo = MemIpoStore::default();
        *ipo.state.borrow_mut() = real_state();
        World {
            ipo,
            es: MemEarningsStore::new(base.clone()),
            base,
        }
    }
    impl World {
        async fn try_tick(&self, now: &str) -> Result<ArticleOutcome, String> {
            article_tick(utc(now), &[anthropic()], &self.ipo, &self.es, &*self.base).await
        }
        async fn tick(&self, now: &str) -> ArticleOutcome {
            self.try_tick(now).await.unwrap()
        }
        fn visible(&self, lang: &str) -> usize {
            self.base
                .translations
                .borrow()
                .iter()
                .filter(|((_, l), t)| l == lang && t.status == "published")
                .count()
        }
        fn run(&self, key: &str) -> RunRow {
            self.base.runs.borrow().get(key).cloned().expect("run")
        }
        fn add_final(&self) {
            let fin = "0001628280-26-090001";
            let mut st = self.ipo.state.borrow_mut();
            st.filings.push(FilingRow {
                accession: fin.into(),
                slug: "anthropic".into(),
                form: "424B4".into(),
                filing_date: "2026-11-18".into(),
                index_url: "https://www.sec.gov/Archives/edgar/data/2130357/000162828026090001/0001628280-26-090001-index.htm".into(),
                read_state: "read".into(),
                ..Default::default()
            });
            for (k, v) in [("price_final", 80.0), ("shares_offered", 825e6)] {
                st.facts.push(FactRow {
                    accession: fin.into(),
                    kind: k.into(),
                    slug: "anthropic".into(),
                    value_num: Some(v),
                    value_text: None,
                    quote: "q".into(),
                });
            }
        }
        fn add_session(&self, day: &str, confirmed: bool) {
            let mut st = self.ipo.state.borrow_mut();
            st.trading.push(TradingRow {
                slug: "anthropic".into(),
                session_date: day.into(),
                symbol: "ANT".into(),
                open: Some(95.0),
                close: 101.5,
                volume: Some(120_000_000),
                ipo_price: Some(80.0),
                change_pct: Some(26.88),
                source_url: "https://finance.yahoo.com/quote/ANT".into(),
                fetched_at: format!("{day}T21:30:00Z"),
            });
            let w = &mut st.watch[0];
            if confirmed {
                w.first_trade_date = Some(day.into());
                w.date_status = Some("confirmed".into());
                w.date_source = Some("trading".into());
            } else {
                w.first_trade_date = Some("2026-11-19".into());
                w.date_status = Some("expected".into());
                w.date_source = Some("filing".into());
            }
        }
    }

    #[tokio::test]
    async fn the_real_state_publishes_one_range_set_write_up_in_both_languages() {
        let w = world();
        let out = w.tick(NOW).await;
        assert_eq!(
            out.runs,
            [(
                RUN.to_string(),
                "published anthropic-ipo-price-range-2026-11-10 (cover ok)".into()
            )]
        );
        assert!(out.published && out.card);
        let posts = w.base.posts.borrow().clone();
        assert_eq!(posts.len(), 1);
        let p = &posts[0];
        assert_eq!(
            (
                p.run_key.as_str(),
                p.content_type.as_str(),
                p.template_id.as_str(),
                p.trading_date.as_str(),
                p.facts_r2_key.as_str(),
                p.cover_key.as_deref(),
                p.revised_at.as_deref()
            ),
            (
                RUN,
                "ipo",
                "ipo-range_set",
                "2026-11-10",
                "ipo/anthropic/range_set-0001628280-26-072484.json",
                Some("covers/anthropic-ipo-price-range-2026-11-10.svg"),
                None
            )
        );
        // The cover is the facts' own graphic, in R2 before the post went live.
        let (svg, ctype) =
            w.base.bytes.borrow()["covers/anthropic-ipo-price-range-2026-11-10.svg"].clone();
        assert_eq!(ctype, "image/svg+xml");
        let svg = String::from_utf8(svg).unwrap();
        assert!(
            svg.contains(">$70.00<") && svg.contains(">$78.00<"),
            "{svg}"
        );
        assert!(p.published_at.is_some());
        let tr = w.base.translations.borrow().clone();
        let en = &tr[&(1, "en".to_string())];
        let zh = &tr[&(1, "zh".to_string())];
        assert_eq!(
            (en.status.as_str(), zh.status.as_str()),
            ("published", "published")
        );
        assert!(en
            .body_md
            .starts_with("# Anthropic's IPO filing gives a price range of $70 to $78 a share\n"));
        assert!(zh
            .body_md
            .starts_with("# Anthropic IPO文件载明价格区间：每股70至78美元\n"));
        assert_eq!(en.cover_line2.as_deref(), Some("Price range $70.00–$78.00"));
        let json =
            w.base.facts.borrow()["ipo/anthropic/range_set-0001628280-26-072484.json"].clone();
        let f: IpoArticleFacts = serde_json::from_str(&json).unwrap();
        assert_eq!(
            (f.schema.as_str(), f.cover.len(), f.timeline.len()),
            ("ipo-article/v1", 9, 9)
        );
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["milestone"], "range_set");
        assert_eq!(v["items"], json!([]));
        assert_eq!(w.base.sources.borrow()[&1].len(), 1);
        assert_eq!(w.run(RUN).status, "published");
        assert_eq!(out.next, "2026-11-11T23:00:00Z");

        // Published once: later passes write nothing.
        let out = w.tick("2026-11-12T15:00:00Z").await;
        assert!(out.runs.is_empty() && !out.published && !out.card);
        assert_eq!(w.base.posts.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_batch_leaves_nothing_visible_and_the_retry_writes_one_post() {
        let w = world();
        w.base.fail_revise_post.set(true);
        assert!(w.try_tick(NOW).await.is_err());
        assert_eq!((w.visible("en"), w.visible("zh")), (0, 0));
        assert_eq!(w.base.posts.borrow().len(), 1);
        // A redelivered message while the build's lease runs: nothing.
        w.base.fail_revise_post.set(false);
        let out = w.tick("2026-11-11T22:01:00Z").await;
        assert_eq!(out.runs[0].1, "in flight");
        assert_eq!(out.next, "2026-11-11T22:15:00Z");
        // After the lease: built again into the same post.
        let out = w.tick("2026-11-11T22:20:00Z").await;
        assert!(out.published);
        assert_eq!(w.base.posts.borrow().len(), 1);
        assert_eq!((w.visible("en"), w.visible("zh")), (1, 1));
        assert_eq!(w.run(RUN).attempts, 2);
    }

    #[tokio::test]
    async fn a_build_that_keeps_failing_stops_after_five() {
        let w = world();
        w.base.fail_revise_post.set(true);
        // Each failed build holds its lease for 15 minutes.
        for (i, at) in [
            "2026-11-11T22:00:00Z",
            "2026-11-11T22:16:00Z",
            "2026-11-11T22:32:00Z",
            "2026-11-11T22:48:00Z",
            "2026-11-11T23:04:00Z",
        ]
        .iter()
        .enumerate()
        {
            assert!(w.try_tick(at).await.is_err(), "build {}", i + 1);
            assert_eq!(w.run(RUN).attempts, i as i64 + 1);
        }
        let out = w.tick("2026-11-11T23:20:00Z").await;
        assert!(out.runs[0].1.starts_with("failed: "), "{out:?}");
        assert_eq!(w.run(RUN).status, "failed");
        let alerts = w.base.alerts.borrow().clone();
        assert_eq!((alerts[0].0.as_str(), alerts[0].1.as_str()), (RUN, "error"));
        // Failed is final.
        assert!(w.tick("2026-11-12T15:00:00Z").await.runs.is_empty());
        assert_eq!((w.visible("en"), w.visible("zh")), (0, 0));
    }

    #[test]
    fn failed_builds_back_off() {
        let mins: Vec<i64> = (1..=6).map(|n| backoff(n).num_minutes()).collect();
        assert_eq!(mins, [5, 15, 30, 60, 60, 60]);
        let mut r = RunRow {
            run_key: RUN.into(),
            content_type: "ipo".into(),
            trading_date: "2026-11-10".into(),
            status: "pending".into(),
            attempts: 2,
            last_error: Some("x".into()),
            created_at: "2026-11-11T22:00:00Z".into(),
            updated_at: "2026-11-11T22:10:00Z".into(),
        };
        assert!(waiting(&r, utc("2026-11-11T22:24:00Z")));
        assert!(!waiting(&r, utc("2026-11-11T22:25:00Z")));
        r.attempts = MAX_BUILDS;
        assert!(spent(&r));
    }

    #[tokio::test]
    async fn text_already_live_only_closes_the_run() {
        let w = world();
        w.tick(NOW).await;
        let mut r = w.run(RUN);
        r.status = "building".into();
        w.base.runs.borrow_mut().insert(RUN.into(), r);
        let out = w.tick("2026-11-11T22:30:00Z").await;
        assert_eq!(out.runs[0].1, "published (run closed)");
        assert_eq!(w.run(RUN).status, "published");
    }

    #[tokio::test]
    async fn a_milestone_found_late_is_skipped_with_a_warning() {
        let w = world();
        let out = w.tick("2026-11-18T22:00:00Z").await;
        assert!(
            out.runs[0]
                .1
                .starts_with("skipped: found 8 days after 2026-11-10"),
            "{out:?}"
        );
        assert_eq!(w.run(RUN).status, "skipped");
        assert_eq!(w.base.alerts.borrow()[0].1, "warn");
        let w = world();
        assert!(w.tick("2026-11-17T22:00:00Z").await.published);
    }

    #[tokio::test]
    async fn a_newer_milestone_gets_its_own_write_up_and_the_older_one_none() {
        let w = world();
        w.add_final();
        let out = w.tick("2026-11-18T22:00:00Z").await;
        assert_eq!(
            out.runs,
            [(
                "ipo:anthropic:priced:0001628280-26-090001".to_string(),
                "published anthropic-ipo-priced-2026-11-18 (cover ok)".into()
            )],
            "{out:?}"
        );
        assert!(w.base.runs.borrow().get(RUN).is_none());
        assert_eq!(w.base.posts.borrow()[0].template_id, "ipo-priced");
        let en = w.base.translations.borrow()[&(1, "en".to_string())]
            .body_md
            .clone();
        assert!(
            en.starts_with("# Anthropic prices IPO at $80 a share\n"),
            "{en}"
        );
        assert!(en.contains("That price is above the range of $70.00 to $78.00 in the S-1/A filed Nov 10, 2026."), "{en}");
    }

    #[tokio::test]
    async fn listed_waits_until_the_session_is_the_confirmed_first_day() {
        let w = world();
        w.add_final();
        w.tick("2026-11-18T22:00:00Z").await;
        // Day one missed: the stored session is not the filings' day.
        w.add_session("2026-11-20", false);
        let out = w.tick("2026-11-20T22:00:00Z").await;
        assert_eq!(
            out.runs,
            [(
                "ipo:anthropic:listed:2026-11-20".to_string(),
                "waiting for the first trading day to be confirmed".into()
            )]
        );
        assert!(w
            .base
            .runs
            .borrow()
            .get("ipo:anthropic:listed:2026-11-20")
            .is_none());
        // Never confirmed: skipped after 7 days, with a warning.
        let out = w.tick("2026-11-30T15:00:00Z").await;
        assert!(out.runs[0].1.contains("never confirmed"), "{out:?}");
        assert_eq!(w.run("ipo:anthropic:listed:2026-11-20").status, "skipped");

        // Confirmed: written.
        let w = world();
        w.add_final();
        w.tick("2026-11-18T22:00:00Z").await;
        w.add_session("2026-11-19", true);
        let out = w.tick("2026-11-19T22:00:00Z").await;
        assert!(out.published, "{out:?}");
        let en = w.base.translations.borrow()[&(2, "en".to_string())]
            .body_md
            .clone();
        assert!(
            en.starts_with("# Anthropic closes first trading day at $101.50\n"),
            "{en}"
        );
    }

    const SLUG: &str = "anthropic-ipo-price-range-2026-11-10";
    const COVER: &str = "covers/anthropic-ipo-price-range-2026-11-10.svg";
    const TRIES: &str = "covers/anthropic-ipo-price-range-2026-11-10.attempts";

    impl World {
        /// The live write-up as the first deploy published it: no cover.
        async fn published_without_cover(&self) {
            self.tick(NOW).await;
            let mut posts = self.base.posts.borrow_mut();
            posts[0].cover_key = None;
            posts[0].revised_at = Some("2026-11-11T22:30:00Z".into());
            self.base.bytes.borrow_mut().clear();
        }
        fn post(&self) -> PostRow {
            self.base.posts.borrow()[0].clone()
        }
        fn tries(&self) -> Option<String> {
            self.base.facts.borrow().get(TRIES).cloned()
        }
    }

    #[tokio::test]
    async fn a_published_write_up_without_a_cover_gets_one_on_a_later_pass() {
        let w = world();
        w.published_without_cover().await;
        let before = w.post();
        let out = w.tick("2026-11-12T15:00:00Z").await;
        assert_eq!(
            out.runs,
            [(RUN.to_string(), format!("cover added: {COVER}"))]
        );
        assert!(out.card && !out.published);
        let after = w.post();
        assert_eq!(after.cover_key.as_deref(), Some(COVER));
        // Only the cover changed: not the slug, the dates or the text.
        assert_eq!(
            PostRow {
                cover_key: None,
                ..after.clone()
            },
            before
        );
        let (svg, _) = w.base.bytes.borrow()[COVER].clone();
        let svg = String::from_utf8(svg).unwrap();
        let f: IpoArticleFacts =
            serde_json::from_str(&w.base.facts.borrow()[&after.facts_r2_key]).unwrap();
        assert_eq!(Some(svg), ipo_graphic(&f));
        assert_eq!(w.tries().as_deref(), Some("1"));
        // Once only: the next pass finds the cover and does nothing.
        let out = w.tick("2026-11-12T16:00:00Z").await;
        assert!(out.runs.is_empty() && !out.card, "{out:?}");
    }

    #[tokio::test]
    async fn the_backfill_gives_up_after_three_failed_tries() {
        let w = world();
        w.published_without_cover().await;
        w.base.fail_put_bytes.set(true);
        for (n, at) in ["15:00", "16:00", "17:00"].iter().enumerate() {
            let out = w.tick(&format!("2026-11-12T{at}:00Z")).await;
            assert_eq!(out.runs.len(), 1, "{out:?}");
            assert!(
                out.runs[0]
                    .1
                    .starts_with(&format!("no cover: err (try {} of 3)", n + 1)),
                "{out:?}"
            );
            assert!(!out.card);
        }
        w.base.fail_put_bytes.set(false);
        let out = w.tick("2026-11-12T18:00:00Z").await;
        assert!(out.runs.is_empty(), "{out:?}");
        assert_eq!(w.post().cover_key, None);
        assert_eq!(w.tries().as_deref(), Some("3"));
    }

    #[tokio::test]
    async fn facts_without_a_graphic_or_missing_are_not_tried_again() {
        // The stored bundle lost its range: no graphic, now or later.
        let w = world();
        w.published_without_cover().await;
        let key = w.post().facts_r2_key;
        let mut f: IpoArticleFacts = serde_json::from_str(&w.base.facts.borrow()[&key]).unwrap();
        f.cover.retain(|c| c.kind != "price_high");
        w.base
            .facts
            .borrow_mut()
            .insert(key.clone(), serde_json::to_string(&f).unwrap());
        let out = w.tick("2026-11-12T15:00:00Z").await;
        assert_eq!(out.runs[0].1, "no cover: the facts give no graphic");
        assert!(w.tick("2026-11-12T16:00:00Z").await.runs.is_empty());
        assert!(w.base.bytes.borrow().is_empty());

        // The bundle is gone: three tries, then none.
        let w = world();
        w.published_without_cover().await;
        w.base.facts.borrow_mut().remove(&key);
        for at in ["15:00", "16:00", "17:00"] {
            let out = w.tick(&format!("2026-11-12T{at}:00Z")).await;
            assert!(out.runs[0].1.contains("missing"), "{out:?}");
        }
        assert!(w.tick("2026-11-12T18:00:00Z").await.runs.is_empty());
    }

    #[tokio::test]
    async fn the_backfill_skips_old_and_unpublished_write_ups() {
        // More than WINDOW_DAYS after the filing: no card would follow.
        let w = world();
        w.published_without_cover().await;
        assert!(w.tick("2026-11-25T15:00:00Z").await.runs.is_empty());
        assert_eq!(w.post().cover_key, None);
        assert_eq!(w.tries(), None);
        // Text not live: left to its build.
        let w = world();
        w.published_without_cover().await;
        w.base
            .translations
            .borrow_mut()
            .get_mut(&(1, "en".to_string()))
            .unwrap()
            .status = "pending".into();
        assert!(w.tick("2026-11-12T15:00:00Z").await.runs.is_empty());
        assert_eq!(w.post().cover_key, None);
    }

    #[tokio::test]
    async fn a_build_and_a_backfill_never_share_a_message() {
        let w = world();
        w.published_without_cover().await;
        w.add_final();
        let out = w.tick("2026-11-18T22:00:00Z").await;
        assert_eq!(out.runs.len(), 1, "{out:?}");
        assert!(out.runs[0].0.contains(":priced:") && out.published && out.card);
        assert_eq!(w.post().cover_key, None);
        // The next pass, with nothing to build, covers the older write-up.
        let out = w.tick("2026-11-18T23:00:00Z").await;
        assert_eq!(
            out.runs,
            [(RUN.to_string(), format!("cover added: {COVER}"))]
        );
        assert_eq!(w.post().slug, SLUG);
    }

    #[tokio::test]
    async fn a_failed_cover_write_still_publishes_without_a_cover() {
        let w = world();
        w.base.fail_put_bytes.set(true);
        let out = w.tick(NOW).await;
        assert_eq!(
            out.runs[0].1,
            format!("published {SLUG} (cover err: r2: put_bytes failed)")
        );
        assert!(out.published && !out.card);
        assert_eq!(w.post().cover_key, None);
        assert_eq!((w.visible("en"), w.visible("zh")), (1, 1));
        // The backfill covers it on a later pass.
        w.base.fail_put_bytes.set(false);
        let out = w.tick("2026-11-12T15:00:00Z").await;
        assert!(out.card, "{out:?}");
        assert_eq!(w.post().cover_key.as_deref(), Some(COVER));
    }

    #[tokio::test]
    async fn polls_queue_the_message_only_with_both_switches_and_once_due() {
        let w = world();
        let list = [anthropic()];
        let at = utc("2026-11-11T15:00:00Z");
        let on = Some("true");
        assert_eq!(
            follow_up(on, on, &list, &w.es, at).await.unwrap(),
            Some(EarningsStep::IpoArticle)
        );
        for (a, b) in [(None, on), (on, None), (Some("TRUE"), on), (on, Some("1"))] {
            assert_eq!(follow_up(a, b, &list, &w.es, at).await.unwrap(), None);
        }
        assert_eq!(follow_up(on, on, &[], &w.es, at).await.unwrap(), None);
        assert_eq!(
            follow_up(on, on, &list, &w.es, utc("2026-11-14T15:00:00Z"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            follow_up(on, on, &list, &w.es, utc("2026-11-11T04:00:00Z"))
                .await
                .unwrap(),
            None
        );
        w.es.kv
            .borrow_mut()
            .insert(KV_NEXT.into(), "2026-11-11T15:30:00Z".into());
        assert_eq!(follow_up(on, on, &list, &w.es, at).await.unwrap(), None);
        assert_eq!(
            follow_up(on, on, &list, &w.es, utc("2026-11-11T15:30:00Z"))
                .await
                .unwrap(),
            Some(EarningsStep::IpoArticle)
        );
        w.es.kv
            .borrow_mut()
            .insert(KV_NEXT.into(), "garbage".into());
        assert!(follow_up(on, on, &list, &w.es, at).await.unwrap().is_some());
    }
}
