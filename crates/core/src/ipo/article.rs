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
//! build per message (two model calls, up to four with the retries).
//!
//! **Never half published.** Both languages are drafted and checked
//! before anything is written. Then the facts go to R2, the post row is
//! written without text (invisible: the site shows a post only by a
//! published translation), and both translations and the sources land in
//! one D1 batch (`revise_post`, one transaction). A store failure before
//! that batch leaves nothing visible and the next pass writes into the same
//! post (`upsert_post` keeps one per run key); one after it finds the text
//! live and only closes the run.
//!
//! **Late or failing.** A milestone found more than `LATE_DAYS` after its
//! filing or session is `skipped` with a warn alert, never published late.
//! A draft that fails its checks twice (each language gets one retry with
//! the reason) leaves the run `pending` with one warn alert a day, and is
//! tried again every `RETRY_MINUTES`, as an earnings build is; after
//! `MIN_ATTEMPTS` builds and `DEADLINE_HOURS` it is `failed` with an error
//! alert.

use chrono::{DateTime, Duration, Utc};

use super::milestone::{bundle, latest, IpoArticleFacts, Reached};
use super::store::{IpoState, IpoStore};
use super::writeup::{self, cover_lines, source_rows, template_id, CONTENT_TYPE};
use super::{iso, Tracked};
use crate::clock::{et, trading_day, DayKind};
use crate::earnings::store::EarningsStore;
use crate::jobs::alert_once_today;
use crate::numbers::Lang;
use crate::premarket::draft::Draft;
use crate::providers::LlmProvider;
use crate::schedule::EarningsStep;
use crate::store::{PostRow, RunRow, Store, TranslationRow};

/// kv: when a poll next queues `ipo:article` (RFC 3339).
pub const KV_NEXT: &str = "ipo_article_next";
/// A pass with nothing to retry looks again this much later.
pub const IDLE_MINUTES: i64 = 60;
/// Pause between builds of a run whose draft was refused.
pub const RETRY_MINUTES: i64 = 5;
/// A `building` run younger than this is in flight.
const LEASE_MINUTES: i64 = 15;
/// A run gives up after this long, and never before `MIN_ATTEMPTS` builds.
const DEADLINE_HOURS: i64 = 6;
const MIN_ATTEMPTS: i64 = 3;
/// A milestone older than this (ET days from its filing or session) when
/// first found is skipped.
pub const LATE_DAYS: i64 = 7;

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

fn waiting(r: &RunRow, now: DateTime<Utc>) -> bool {
    r.status == "pending"
        && r.attempts > 0
        && since(&r.updated_at, now) < Duration::minutes(RETRY_MINUTES)
}

fn past_deadline(r: &RunRow, now: DateTime<Utc>) -> bool {
    r.attempts >= MIN_ATTEMPTS && since(&r.created_at, now) >= Duration::hours(DEADLINE_HOURS)
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
    llm: &dyn LlmProvider,
) -> Result<ArticleOutcome, String> {
    // Marked first, so a pass that fails is not queued by every poll.
    let idle = now + Duration::minutes(IDLE_MINUTES);
    es.kv_set(KV_NEXT, &iso(idle), &iso(now)).await?;
    let st = ipo.ipo_state().await?;
    let mut out = ArticleOutcome::default();
    let mut next = idle;
    let soon = now + Duration::minutes(RETRY_MINUTES);
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
            Some(run) if leased(&run, now) => {
                next = next.min(soon);
                "in flight".to_string()
            }
            Some(run) if waiting(&run, now) => {
                next = next.min(soon);
                "waiting to retry".to_string()
            }
            Some(run) if past_deadline(&run, now) => {
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
                let b = build(now, &st, es, store, llm, t, &r, run).await?;
                if b.retry {
                    next = next.min(soon);
                }
                out.published |= b.published;
                b.what
            }
        };
        out.runs.push((key, what));
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
    let err = format!(
        "found {} days after {}, more than {LATE_DAYS}: not published late",
        late(r, now),
        super::ymd(r.as_of)
    );
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
    retry: bool,
    published: bool,
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

#[allow(clippy::too_many_arguments)]
async fn build(
    now: DateTime<Utc>,
    st: &IpoState,
    es: &dyn EarningsStore,
    store: &dyn Store,
    llm: &dyn LlmProvider,
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
                retry: false,
                published: false,
            });
        }
    }

    row.status = "building".into();
    row.attempts += 1;
    row.updated_at = ts.clone();
    store.upsert_run(&row).await?;

    let f = bundle(st, t, r, now);
    let en = writeup::write(llm, &f, Lang::En).await;
    // No Chinese call when the English draft already failed.
    let zh = match &en {
        Ok(_) => writeup::write(llm, &f, Lang::Zh).await,
        Err(_) => Err(vec!["not drafted after the English draft failed".into()]),
    };
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
            retry: false,
            published: false,
        });
    }
    let facts_key = f.facts_key();
    let json = serde_json::to_string(&f).map_err(|e| format!("facts json: {e}"))?;
    store.put_facts(&facts_key, &json).await?;
    let mut post = PostRow {
        id: None,
        run_key: key.clone(),
        content_type: CONTENT_TYPE.into(),
        template_id: template_id(f.milestone),
        trading_date: f.as_of.clone(),
        slug: post_slug(es, &f).await?,
        facts_r2_key: facts_key,
        published_at: Some(ts.clone()),
        cover_key: None,
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
    Ok(Built {
        what: format!("published {}", post.slug),
        retry: false,
        published: true,
    })
}

async fn not_ready(
    now: DateTime<Utc>,
    store: &dyn Store,
    t: &Tracked,
    r: &Reached,
    mut row: RunRow,
    err: String,
) -> Result<Built, String> {
    if past_deadline(&row, now) {
        let what = give_up(now, store, t, r, row, err).await?;
        return Ok(Built {
            what,
            retry: false,
            published: false,
        });
    }
    row.status = "pending".into();
    row.last_error = Some(err.clone());
    row.updated_at = iso(now);
    store.upsert_run(&row).await?;
    let msg = format!(
        "IPO write-up for {} not published yet (retrying every {RETRY_MINUTES} minutes): {err}",
        label(t, r)
    );
    alert_once_today(now, &row.run_key, "warn", &msg, store).await?;
    Ok(Built {
        what: format!("retrying: {err}"),
        retry: true,
        published: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::store::MemEarningsStore;
    use crate::ipo::milestone::tests::{anthropic, real_state, ACC};
    use crate::ipo::milestone::IpoArticleFacts;
    use crate::ipo::store::{FactRow, FilingRow, MemIpoStore};
    use crate::ipo::writeup::tests::reply;
    use crate::providers::ProviderError;
    use crate::store::MemStore;
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const RUN: &str = "ipo:anthropic:range_set:0001628280-26-072484";
    const NOW: &str = "2026-11-11T22:00:00Z";

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// Replies in order; counts calls.
    struct FakeLlm {
        replies: RefCell<Vec<Result<Value, ProviderError>>>,
        calls: Cell<usize>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for FakeLlm {
        async fn complete_json(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
            self.calls.set(self.calls.get() + 1);
            let mut r = self.replies.borrow_mut();
            assert!(!r.is_empty(), "unexpected extra LLM call");
            r.remove(0)
        }
    }
    fn llm(replies: Vec<Value>) -> FakeLlm {
        FakeLlm {
            replies: RefCell::new(replies.into_iter().map(Ok).collect()),
            calls: Cell::new(0),
        }
    }
    fn bad(lang: Lang) -> Value {
        let mut v = reply(lang);
        v["sections"][0]["markdown"] = json!("It reflects strong demand at $70.00.");
        v
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
        async fn tick(&self, now: &str, l: &FakeLlm) -> ArticleOutcome {
            article_tick(
                utc(now),
                &[anthropic()],
                &self.ipo,
                &self.es,
                &*self.base,
                l,
            )
            .await
            .unwrap()
        }
        fn visible(&self, lang: &str) -> usize {
            self.base
                .translations
                .borrow()
                .iter()
                .filter(|((_, l), t)| l == lang && t.status == "published")
                .count()
        }
        fn run(&self) -> RunRow {
            self.base.runs.borrow().get(RUN).cloned().expect("run")
        }
    }

    #[tokio::test]
    async fn the_real_state_publishes_one_range_set_write_up_in_both_languages() {
        let w = world();
        let l = llm(vec![reply(Lang::En), reply(Lang::Zh)]);
        let out = w.tick(NOW, &l).await;
        assert_eq!(
            out.runs,
            [(
                RUN.to_string(),
                "published anthropic-ipo-price-range-2026-11-10".into()
            )]
        );
        assert!(out.published);
        assert_eq!(l.calls.get(), 2);
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
                None,
                None
            )
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
            .starts_with("# Anthropic sets IPO price range at $70 to $78 a share\n"));
        assert_eq!(en.cover_line2.as_deref(), Some("Price range $70.00–$78.00"));
        drop(tr);
        let json =
            w.base.facts.borrow()["ipo/anthropic/range_set-0001628280-26-072484.json"].clone();
        let f: IpoArticleFacts = serde_json::from_str(&json).unwrap();
        assert_eq!(
            (f.schema.as_str(), f.cover.len(), f.timeline.len()),
            ("ipo-article/v1", 9, 9)
        );
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["milestone"], "range_set");
        assert_eq!(v["previous"], Value::Null);
        assert_eq!(v["items"], json!([]));
        assert_eq!(w.base.sources.borrow()[&1].len(), 1);
        assert_eq!(w.run().status, "published");
        assert_eq!(out.next, "2026-11-11T23:00:00Z");

        // Published once: later passes make no call and no post.
        let l = llm(vec![]);
        let out = w.tick("2026-11-12T15:00:00Z", &l).await;
        assert!(out.runs.is_empty() && !out.published);
        assert_eq!(w.base.posts.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_chinese_draft_publishes_nothing_until_both_pass() {
        let w = world();
        let l = llm(vec![reply(Lang::En), bad(Lang::Zh), bad(Lang::Zh)]);
        let out = w.tick(NOW, &l).await;
        assert!(
            out.runs[0].1.starts_with("retrying: zh: draft: "),
            "{out:?}"
        );
        assert_eq!((w.visible("en"), w.visible("zh")), (0, 0));
        assert!(w.base.posts.borrow().is_empty());
        let r = w.run();
        assert_eq!((r.status.as_str(), r.attempts), ("pending", 1));
        assert_eq!(out.next, "2026-11-11T22:05:00Z");
        assert_eq!(w.base.alerts.borrow().len(), 1);
        assert_eq!(w.base.alerts.borrow()[0].1, "warn");
        // Before the pause is over, nothing is called.
        let none = llm(vec![]);
        let out = w.tick("2026-11-11T22:03:00Z", &none).await;
        assert_eq!(out.runs[0].1, "waiting to retry");
        // Then both pass, and both go live together.
        let l = llm(vec![reply(Lang::En), reply(Lang::Zh)]);
        let out = w.tick("2026-11-11T22:06:00Z", &l).await;
        assert!(out.published, "{out:?}");
        assert_eq!((w.visible("en"), w.visible("zh")), (1, 1));
        assert_eq!(w.run().attempts, 2);
        // One warn alert a day, not one per attempt.
        assert_eq!(w.base.alerts.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_batch_leaves_nothing_visible_and_the_retry_writes_one_post() {
        let w = world();
        w.base.fail_revise_post.set(true);
        let l = llm(vec![reply(Lang::En), reply(Lang::Zh)]);
        let e = article_tick(utc(NOW), &[anthropic()], &w.ipo, &w.es, &*w.base, &l).await;
        assert!(e.is_err());
        assert_eq!((w.visible("en"), w.visible("zh")), (0, 0));
        assert_eq!(w.base.posts.borrow().len(), 1);
        // A redelivered message while the build's lease runs: nothing.
        w.base.fail_revise_post.set(false);
        let none = llm(vec![]);
        let out = w.tick("2026-11-11T22:01:00Z", &none).await;
        assert_eq!(out.runs[0].1, "in flight");
        // After the lease: built again into the same post.
        let l = llm(vec![reply(Lang::En), reply(Lang::Zh)]);
        let out = w.tick("2026-11-11T22:20:00Z", &l).await;
        assert!(out.published);
        assert_eq!(w.base.posts.borrow().len(), 1);
        assert_eq!((w.visible("en"), w.visible("zh")), (1, 1));
    }

    #[tokio::test]
    async fn text_already_live_only_closes_the_run() {
        let w = world();
        let l = llm(vec![reply(Lang::En), reply(Lang::Zh)]);
        w.tick(NOW, &l).await;
        // As if the run write after the batch had failed.
        let mut r = w.run();
        r.status = "building".into();
        r.updated_at = "2026-11-11T22:00:00Z".into();
        w.base.runs.borrow_mut().insert(RUN.into(), r);
        let none = llm(vec![]);
        let out = w.tick("2026-11-11T22:30:00Z", &none).await;
        assert_eq!(out.runs[0].1, "published (run closed)");
        assert_eq!(w.run().status, "published");
        assert_eq!(none.calls.get(), 0);
    }

    #[tokio::test]
    async fn a_milestone_found_late_is_skipped_with_a_warning() {
        let w = world();
        let none = llm(vec![]);
        let out = w.tick("2026-11-18T22:00:00Z", &none).await;
        assert!(
            out.runs[0]
                .1
                .starts_with("skipped: found 8 days after 2026-11-10"),
            "{out:?}"
        );
        assert_eq!(w.run().status, "skipped");
        assert_eq!(w.base.alerts.borrow()[0].1, "warn");
        // Seven days is still on time.
        let w = world();
        let l = llm(vec![reply(Lang::En), reply(Lang::Zh)]);
        assert!(w.tick("2026-11-17T22:00:00Z", &l).await.published);
    }

    #[tokio::test]
    async fn a_run_that_keeps_failing_gives_up_after_the_deadline() {
        let w = world();
        w.base.runs.borrow_mut().insert(
            RUN.into(),
            RunRow {
                run_key: RUN.into(),
                content_type: "ipo".into(),
                trading_date: "2026-11-10".into(),
                status: "pending".into(),
                attempts: 2,
                last_error: Some("zh: draft: x".into()),
                created_at: "2026-11-11T14:00:00Z".into(),
                updated_at: "2026-11-11T21:00:00Z".into(),
            },
        );
        let l = llm(vec![bad(Lang::En), bad(Lang::En)]);
        let out = w.tick(NOW, &l).await;
        assert!(out.runs[0].1.starts_with("failed: en: "), "{out:?}");
        assert_eq!(w.run().status, "failed");
        let alerts = w.base.alerts.borrow();
        assert_eq!((alerts[0].0.as_str(), alerts[0].1.as_str()), (RUN, "error"));
        assert!(w.base.posts.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_newer_milestone_gets_its_own_write_up_and_the_older_one_none() {
        let w = world();
        // The 424B4 arrives before any write-up: range_set is passed over.
        let fin = "0001628280-26-090001";
        {
            let mut st = w.ipo.state.borrow_mut();
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
        let mut en = reply(Lang::En);
        en["title"] = json!("Anthropic prices its IPO at $80.00 a share");
        en["summary"] = json!("Anthropic's final prospectus (Form 424B4) filed Nov. 18, 2026 sets the offering price at $80.00 a share.");
        en["sections"][0]["markdown"] = json!("Anthropic filed its final prospectus (Form 424B4) on Nov. 18, 2026. It prices 825,000,000 shares at $80.00 each, which come to $66 billion.");
        en["sections"][1]["markdown"] =
            json!("The 424B4 lists the Nasdaq Global Select Market and the symbol ANT.");
        let mut zh = reply(Lang::Zh);
        zh["title"] = json!("Anthropic完成IPO定价：每股80.00美元");
        zh["summary"] = json!(
            "Anthropic于2026年11月18日提交最终招股说明书（424B4），发行价定为每股80.00美元。"
        );
        zh["sections"][0]["markdown"] = json!(
            "Anthropic于2026年11月18日提交424B4。8.25亿股的发行价为每股80.00美元，合计660亿美元。"
        );
        zh["sections"][1]["markdown"] =
            json!("424B4载明的交易所为Nasdaq Global Select Market，股票代码为ANT。");
        let l = llm(vec![en, zh]);
        let out = w.tick("2026-11-18T22:00:00Z", &l).await;
        assert_eq!(
            out.runs,
            [(
                "ipo:anthropic:priced:0001628280-26-090001".to_string(),
                "published anthropic-ipo-priced-2026-11-18".into()
            )],
            "{out:?}"
        );
        assert!(w.base.runs.borrow().get(RUN).is_none());
        let p = w.base.posts.borrow()[0].clone();
        assert_eq!(p.template_id, "ipo-priced");
        assert!(!w.base.translations.borrow()[&(1, "en".to_string())]
            .body_md
            .contains("Price range per share"));
        let _ = ACC;
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
        // A Saturday, and outside the polling window.
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
