//! The premarket update. From 08:35 ET, once the 8:30 ET economic releases
//! are out, a second build rebuilds the day's brief from fresh facts and
//! rewrites the same post (same id, slug and URL) in EN and ZH together.
//! It tracks its own run, `premarket-update:YYYY-MM-DD`; the post keeps the
//! base run's key. It writes only when the gate and both drafts pass, so a
//! failed or unneeded update leaves the 8:00 brief as it was, and it never
//! alerts.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;

use super::{
    gate_error, is_stale, iso, new_run, retry_later, superseded, translation_zh, Content, Hooks,
    Premarket, TickOutcome,
};
use crate::clock::{decide, premarket_update_window, SlotDecision};
use crate::cover::{store_graphic, INDEX_LINE_KEY};
use crate::events::watch::WatchSource;
use crate::numbers::Lang;
use crate::providers::{LlmProvider, MarketData, NewsSearch};
use crate::store::{PostRow, RunRow, Store, TranslationRow};

/// `runs.content_type` of the update's run.
const CONTENT_TYPE: &str = "premarket-update";

/// The update's run key, `premarket-update:YYYY-MM-DD`.
pub fn run_key(trading_date: NaiveDate) -> String {
    format!("{CONTENT_TYPE}:{}", trading_date.format("%Y-%m-%d"))
}

/// The update's facts key, `premarket/YYYY-MM-DD-update.json`, beside the
/// base's `premarket/YYYY-MM-DD.json`. Readers follow `posts.facts_r2_key`.
pub fn facts_key(trading_date: NaiveDate) -> String {
    format!("premarket/{}-update.json", trading_date.format("%Y-%m-%d"))
}

/// One update cron firing. A store error is reported as `Retrying`.
pub async fn tick(
    now: DateTime<Utc>,
    store: &dyn Store,
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
) -> TickOutcome {
    let hooks = Hooks {
        log: &|_| {},
        extra: &Vec::new,
        budget: &|| 0,
    };
    tick_with_hooks(now, store, md, ns, llm, &hooks).await
}

/// `tick` with the caller's `Hooks`.
pub async fn tick_with_hooks(
    now: DateTime<Utc>,
    store: &dyn Store,
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    hooks: &Hooks<'_>,
) -> TickOutcome {
    tick_with_watch(now, store, md, ns, llm, None, hooks).await
}

/// `tick_with_hooks` with "What to watch" from `watch`, which also runs the
/// released check for the 8:30 releases the update is for.
pub async fn tick_with_watch(
    now: DateTime<Utc>,
    store: &dyn Store,
    md: &dyn MarketData,
    ns: &dyn NewsSearch,
    llm: &dyn LlmProvider,
    watch: Option<&dyn WatchSource>,
    hooks: &Hooks<'_>,
) -> TickOutcome {
    let c = Premarket {
        md,
        ns,
        llm,
        watch,
        reports: Some(store),
    };
    step(now, store, &c, hooks)
        .await
        .unwrap_or_else(TickOutcome::Retrying)
}

async fn step(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &Premarket<'_>,
    hooks: &Hooks<'_>,
) -> Result<TickOutcome, String> {
    let (date, past) = match decide(now, premarket_update_window) {
        SlotDecision::InWindow { trading_date } => (trading_date, false),
        SlotDecision::PastDeadline { trading_date } => (trading_date, true),
        SlotDecision::NotToday | SlotDecision::TooEarly => return Ok(TickOutcome::Nothing),
    };
    // Any read error aborts the tick before a write: a failed read is never "no row".
    let run = store.get_run(&run_key(date)).await?;
    if let Some(r) = &run {
        let finished = r.status == "published" || r.status == "skipped";
        // Let an in-flight build finish; once its lease expires it is rebuilt,
        // or skipped past the deadline.
        if finished || (r.status == "building" && !is_stale(r, now)) {
            return Ok(TickOutcome::Nothing);
        }
    }
    let base_key = c.run_key(date);
    let base = store.get_run(&base_key).await?;
    if !base.is_some_and(|r| r.status == "published") {
        // The base may still publish late; past the deadline there is nothing to update.
        return if past {
            skip(now, store, date, run, "base not published".into()).await
        } else {
            Ok(TickOutcome::Nothing)
        };
    }
    // Data errors no later firing fixes: skip rather than retry all day.
    let Some(post) = store.get_post_by_run(&base_key).await? else {
        return skip(
            now,
            store,
            date,
            run,
            "published base run has no post".into(),
        )
        .await;
    };
    // Only the update writes `revised_at`, so the rewrite landed and the run
    // write after it was lost (a crash or a store error): no second build.
    if post.revised_at.is_some() {
        let mut row = run.unwrap_or_else(|| new_run(now, run_key(date), CONTENT_TYPE, date));
        row.status = "published".into();
        row.last_error = None;
        row.updated_at = iso(now);
        store.upsert_run(&row).await?;
        return Ok(TickOutcome::Nothing);
    }
    match published_after_releases(&post, date) {
        Some(true) => {
            return skip(now, store, date, run, "base published after 08:30".into()).await;
        }
        Some(false) => {}
        None => {
            return skip(
                now,
                store,
                date,
                run,
                "base post has no publish time".into(),
            )
            .await
        }
    }
    if past {
        let why = run
            .as_ref()
            .and_then(|r| r.last_error.clone())
            .unwrap_or_else(|| "no attempt succeeded".into());
        return skip(now, store, date, run, why).await;
    }
    build(now, store, c, hooks, date, run, post).await
}

/// Whether the base post went out at or after 08:30 ET on `date`, so its
/// build already saw the 8:30 releases; `None` without a readable publish time.
fn published_after_releases(post: &PostRow, date: NaiveDate) -> Option<bool> {
    let at = post
        .published_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())?;
    let releases = NaiveTime::from_hms_opt(8, 30, 0).expect("valid built-in time");
    let releases = New_York
        .from_local_datetime(&date.and_time(releases))
        .earliest()
        .expect("08:30 is never in a DST gap");
    Some(at >= releases)
}

/// Marks the update `skipped` with `why`, without an alert: the base brief stands.
async fn skip(
    now: DateTime<Utc>,
    store: &dyn Store,
    date: NaiveDate,
    run: Option<RunRow>,
    why: String,
) -> Result<TickOutcome, String> {
    let mut row = run.unwrap_or_else(|| new_run(now, run_key(date), CONTENT_TYPE, date));
    row.status = "skipped".into();
    row.last_error = Some(why);
    row.updated_at = iso(now);
    store.upsert_run(&row).await?;
    Ok(TickOutcome::Skipped)
}

/// Builds the brief again and, only when the gate and both drafts pass,
/// rewrites the base `post` with it.
async fn build(
    now: DateTime<Utc>,
    store: &dyn Store,
    c: &Premarket<'_>,
    hooks: &Hooks<'_>,
    date: NaiveDate,
    run: Option<RunRow>,
    post: PostRow,
) -> Result<TickOutcome, String> {
    let ts = iso(now);
    let mut row = run.unwrap_or_else(|| new_run(now, run_key(date), CONTENT_TYPE, date));
    row.status = "building".into();
    row.attempts += 1;
    row.updated_at = ts.clone();
    store.upsert_run(&row).await?;

    let facts = c.build_facts(date, now, hooks.budget, hooks.log).await;
    let mut lines = c.diagnostics(&facts);
    lines.extend((hooks.extra)());
    (hooks.log)(&format!("diagnostics: {}", lines.join("; ")));
    if let Some(err) = gate_error(&c.missing_required(&facts), &lines) {
        return retry_later(store, row, err).await;
    }
    let (en, en_md) = match c.write(&facts, Lang::En).await {
        Ok(x) => x,
        Err(e) => return retry_later(store, row, format!("en: {}", e.join("; "))).await,
    };
    let (zh, zh_md) = match c.write(&facts, Lang::Zh).await {
        Ok(x) => x,
        Err(e) => return retry_later(store, row, format!("zh: {}", e.join("; "))).await,
    };
    if superseded(store, &row.run_key).await? {
        return Ok(TickOutcome::Nothing);
    }

    // A new key per attempt: covers are cached for a day, so a key the site
    // may already serve (the base's, or an earlier attempt's) never changes.
    let cover_slug = format!("{}-update-{}", post.slug, row.attempts);
    let (cover_key, outcome, cover_err) =
        store_graphic(store, &cover_slug, c.cover_graphic(&facts)).await;
    (hooks.log)(&format!("cover={outcome}"));
    if let Some(e) = cover_err {
        (hooks.log)(&format!("cover write failed: {e}"));
        // The post is untouched and keeps its 8:00 graphic; the next firing tries again.
        return retry_later(store, row, format!("cover: {e}")).await;
    }
    // Its own facts key, written first and switched to in the rewrite below,
    // so the post and its facts change together; the 8:00 facts stay as they are.
    let facts_key = facts_key(date);
    let json = serde_json::to_string(&facts).map_err(|e| format!("facts json: {e}"))?;
    if let Err(e) = store.put_facts(&facts_key, &json).await {
        return retry_later(store, row, e).await;
    }

    let post_id = post.id.ok_or("post has no id")?;
    let cover = c.cover_lines(&facts);
    let [en_line1, en_line2] = cover.en;
    let en_row = TranslationRow {
        post_id,
        lang: "en".into(),
        status: "published".into(),
        title: en.title,
        summary: en.summary,
        body_md: en_md,
        updated_at: ts.clone(),
        cover_line1: Some(en_line1),
        cover_line2: Some(en_line2),
    };
    let zh_row = translation_zh(
        post_id,
        "published",
        zh.title,
        zh.summary,
        zh_md,
        Some(cover.zh),
        &ts,
    );
    let revised = PostRow {
        facts_r2_key: facts_key,
        cover_key,
        revised_at: Some(ts.clone()),
        ..post
    };
    // On failure nothing changed (the new facts and cover are unreferenced),
    // so the next firing retries.
    let rewrite = store
        .revise_post(&revised, &[en_row, zh_row], &c.sources(&facts))
        .await;
    if let Err(e) = rewrite {
        return retry_later(store, row, e).await;
    }
    // The front page's index line; a failed write keeps the previous one.
    let index_line = match serde_json::to_string(&c.index_snapshot(&facts)) {
        Ok(json) => store.kv_set(INDEX_LINE_KEY, &json, &ts).await,
        Err(e) => Err(format!("index snapshot json: {e}")),
    };
    if let Err(e) = index_line {
        (hooks.log)(&format!("index line not written: {e}"));
    }

    row.status = "published".into();
    row.last_error = None;
    row.updated_at = ts;
    store.upsert_run(&row).await?;
    Ok(TickOutcome::Published)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{llm, md, utc, FakeLlm, FakeNs, KEY, URL};
    use super::super::tick as base_tick;
    use super::*;
    use crate::cover::{IndexSnapshot, INDEX_LINE_KEY};
    use crate::facts::{NewsReason, PremarketFacts};
    use crate::providers::ProviderError;
    use crate::store::{MemStore, PostRow, RunRow, TranslationRow};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::cell::Cell;
    use std::collections::{BTreeMap, HashMap};

    const UPDATE_KEY: &str = "premarket-update:2026-10-29";
    const FACTS_KEY: &str = "premarket/2026-10-29.json";
    const UPDATE_FACTS_KEY: &str = "premarket/2026-10-29-update.json";
    /// 08:00 ET on Thu 2026-10-29 (EDT), when the base brief publishes.
    const BASE_AT: &str = "2026-10-29T12:00:00Z";
    /// 08:35 ET, the update's first firing.
    const UPDATE_AT: &str = "2026-10-29T12:35:00Z";

    /// Everything a reader can see of the day's brief.
    #[derive(Debug, PartialEq)]
    struct Snapshot {
        posts: Vec<PostRow>,
        translations: HashMap<(i64, String), TranslationRow>,
        sources: HashMap<i64, Vec<NewsReason>>,
        facts: HashMap<String, String>,
        bytes: BTreeMap<String, (Vec<u8>, String)>,
        kv: BTreeMap<String, String>,
    }
    fn snapshot(s: &MemStore) -> Snapshot {
        Snapshot {
            posts: s.posts.borrow().clone(),
            translations: s.translations.borrow().clone(),
            sources: s.sources.borrow().clone(),
            facts: s.facts.borrow().clone(),
            bytes: s.bytes.borrow().clone(),
            kv: s.kv.borrow().clone(),
        }
    }

    fn old_source() -> NewsReason {
        NewsReason {
            url: "https://news.test/old".into(),
            title: "Old".into(),
            quote: "old".into(),
            summary_en: "old".into(),
        }
    }

    /// A store holding the 08:00 ET base brief (EN and ZH published, or ZH
    /// pending when `zh_ok` is false), with text and sources marked "8:00"
    /// so a rewrite shows.
    async fn base_brief(zh_ok: bool) -> MemStore {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, zh_ok));
        let out = base_tick(utc(BASE_AT), &s, &m, &FakeNs, &l).await;
        let want = if zh_ok {
            TickOutcome::Published
        } else {
            TickOutcome::PublishedEnOnly
        };
        assert_eq!(out, want);
        for t in s.translations.borrow_mut().values_mut() {
            if t.status == "published" {
                t.title = "8:00 title".into();
            }
        }
        s.sources.borrow_mut().insert(1, vec![old_source()]);
        s
    }

    fn run(s: &MemStore) -> Option<RunRow> {
        s.runs.borrow().get(UPDATE_KEY).cloned()
    }
    /// The facts the post points at.
    fn stored_facts(s: &MemStore) -> PremarketFacts {
        let key = s.posts.borrow()[0].facts_r2_key.clone();
        serde_json::from_str(&s.facts.borrow()[&key]).unwrap()
    }

    #[test]
    fn run_key_and_facts_key_format() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        assert_eq!(run_key(date), UPDATE_KEY);
        assert_eq!(facts_key(date), UPDATE_FACTS_KEY);
    }

    #[tokio::test]
    async fn update_does_nothing_before_8_35_or_on_a_closed_day() {
        let s = base_brief(true).await;
        let before = snapshot(&s);
        let (m, l) = (md(true), llm(true, true));
        for t in ["2026-10-29T12:34:00Z", "2026-11-26T13:40:00Z"] {
            assert_eq!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing,
                "{t}"
            );
        }
        assert_eq!((m.calls.get(), l.calls.get()), (0, 0));
        assert_eq!(run(&s), None);
        assert_eq!(snapshot(&s), before);
    }

    #[tokio::test]
    async fn update_waits_for_the_base_then_skips_quietly_at_the_deadline() {
        let s = MemStore::default();
        // The base is still retrying: no quotes yet.
        let out = base_tick(utc(BASE_AT), &s, &md(false), &FakeNs, &llm(true, true)).await;
        assert!(matches!(out, TickOutcome::Retrying(_)), "{out:?}");
        let (m, l) = (md(true), llm(true, true));
        for t in [UPDATE_AT, "2026-10-29T13:20:00Z"] {
            assert_eq!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing
            );
        }
        assert_eq!((m.calls.get(), l.calls.get()), (0, 0));
        assert_eq!(run(&s), None);
        // 09:25 ET and still no base: skipped, no alert.
        assert_eq!(
            tick(utc("2026-10-29T13:25:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Skipped
        );
        let r = run(&s).unwrap();
        assert_eq!(r.status, "skipped");
        assert_eq!(r.content_type, "premarket-update");
        assert_eq!(r.attempts, 0);
        assert_eq!(r.last_error.as_deref(), Some("base not published"));
        assert!(s.alerts.borrow().is_empty());
        assert!(s.posts.borrow().is_empty());
        assert_eq!(
            tick(utc("2026-10-29T13:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert!(s.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn update_is_skipped_when_the_base_published_at_or_after_8_30() {
        for base_at in ["2026-10-29T12:30:00Z", "2026-10-29T12:50:00Z"] {
            let s = MemStore::default();
            let out = base_tick(utc(base_at), &s, &md(true), &FakeNs, &llm(true, true)).await;
            assert_eq!(out, TickOutcome::Published);
            let before = snapshot(&s);
            let (m, l) = (md(true), llm(true, true));
            assert_eq!(
                tick(utc("2026-10-29T12:55:00Z"), &s, &m, &FakeNs, &l).await,
                TickOutcome::Skipped,
                "{base_at}"
            );
            assert_eq!((m.calls.get(), l.calls.get()), (0, 0));
            let r = run(&s).unwrap();
            assert_eq!(r.status, "skipped");
            assert_eq!(r.last_error.as_deref(), Some("base published after 08:30"));
            assert_eq!(snapshot(&s), before);
            assert!(s.alerts.borrow().is_empty());
            assert_eq!(
                tick(utc("2026-10-29T13:00:00Z"), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing
            );
        }
    }

    #[tokio::test]
    async fn the_8_30_threshold_is_08_30_eastern_under_edt_and_est() {
        // (base publish time, update firing, outcome): 08:29 builds, 08:30 skips.
        for (base_at, at, want) in [
            (
                "2026-10-29T12:29:00Z",
                "2026-10-29T12:35:00Z",
                TickOutcome::Published,
            ),
            (
                "2026-10-29T12:30:00Z",
                "2026-10-29T12:35:00Z",
                TickOutcome::Skipped,
            ),
            (
                "2026-11-02T13:29:00Z",
                "2026-11-02T13:35:00Z",
                TickOutcome::Published,
            ),
            (
                "2026-11-02T13:30:00Z",
                "2026-11-02T13:35:00Z",
                TickOutcome::Skipped,
            ),
        ] {
            let s = MemStore::default();
            let (m, l) = (md(true), llm(true, true));
            assert_eq!(
                base_tick(utc(base_at), &s, &m, &FakeNs, &l).await,
                TickOutcome::Published,
                "{base_at}"
            );
            assert_eq!(tick(utc(at), &s, &m, &FakeNs, &l).await, want, "{base_at}");
        }
    }

    #[tokio::test]
    async fn update_skips_a_base_with_no_post_or_no_publish_time() {
        // A published base run with no post: skipped at once, not retried all day.
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        let mut base = building(BASE_AT);
        base.run_key = KEY.into();
        base.content_type = "premarket".into();
        base.status = "published".into();
        s.runs.borrow_mut().insert(KEY.into(), base);
        assert_eq!(
            tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await,
            TickOutcome::Skipped
        );
        assert_eq!(
            run(&s).unwrap().last_error.as_deref(),
            Some("published base run has no post")
        );
        // A base post whose publish time is unreadable.
        let s = base_brief(true).await;
        s.posts.borrow_mut()[0].published_at = Some("not a time".into());
        let before = snapshot(&s);
        assert_eq!(
            tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await,
            TickOutcome::Skipped
        );
        assert_eq!(
            run(&s).unwrap().last_error.as_deref(),
            Some("base post has no publish time")
        );
        assert_eq!(snapshot(&s), before);
        assert_eq!((m.calls.get(), l.calls.get()), (0, 0));
        assert!(s.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_landed_rewrite_whose_run_write_was_lost_is_marked_published_without_a_build() {
        // The rewrite committed, then the invocation died before the run write.
        for at in ["2026-10-29T12:55:00Z", "2026-10-29T13:30:00Z"] {
            let s = base_brief(true).await;
            let (m, l) = (md(true), llm(true, true));
            assert_eq!(
                tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await,
                TickOutcome::Published
            );
            s.runs
                .borrow_mut()
                .insert(UPDATE_KEY.into(), building(UPDATE_AT));
            let after = snapshot(&s);
            let calls = (m.calls.get(), l.calls.get());
            // After the lease, in the window or past the deadline.
            assert_eq!(
                tick(utc(at), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing,
                "{at}"
            );
            assert_eq!((m.calls.get(), l.calls.get()), calls, "{at}: no build");
            let r = run(&s).unwrap();
            assert_eq!(
                (r.status.as_str(), r.last_error),
                ("published", None),
                "{at}"
            );
            // The served cover and the rest of the post are not rewritten.
            assert_eq!(snapshot(&s), after, "{at}");
        }
    }

    #[tokio::test]
    async fn a_cover_write_error_retries_and_keeps_the_8_00_cover() {
        let s = base_brief(true).await;
        let before = snapshot(&s);
        s.fail_put_bytes.set(true);
        let (m, l) = (md(true), llm(true, true));
        assert_eq!(
            tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying("cover: r2: put_bytes failed".into())
        );
        assert_eq!(snapshot(&s), before);
        let r = run(&s).unwrap();
        assert_eq!(
            (r.status.as_str(), r.last_error.as_deref()),
            ("pending", Some("cover: r2: put_bytes failed"))
        );
        // Next firing the write works and the update lands.
        s.fail_put_bytes.set(false);
        assert_eq!(
            tick(utc("2026-10-29T12:40:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        assert_eq!(
            s.posts.borrow()[0].cover_key.as_deref(),
            Some("covers/premarket-2026-10-29-update-2.svg")
        );
        // Still failing at 09:25: skipped, and the 8:00 brief with its cover stands.
        let s = base_brief(true).await;
        let before = snapshot(&s);
        s.fail_put_bytes.set(true);
        tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await;
        assert_eq!(
            tick(utc("2026-10-29T13:25:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Skipped
        );
        assert_eq!(
            run(&s).unwrap().last_error.as_deref(),
            Some("cover: r2: put_bytes failed")
        );
        assert_eq!(snapshot(&s), before);
        assert!(s.alerts.borrow().is_empty());
    }

    #[tokio::test]
    async fn the_update_reads_the_watch_list_at_835() {
        // The watch source runs the released check, so the 8:35 build is the
        // one that sees the 8:30 release (events::watch tests that part).
        let s = base_brief(true).await;
        let (m, l) = (md(true), llm(true, true));
        let w = super::super::tests::FakeWatch::default();
        let hooks = Hooks {
            log: &|_| {},
            extra: &Vec::new,
            budget: &|| 0,
        };
        let out = tick_with_watch(utc(UPDATE_AT), &s, &m, &FakeNs, &l, Some(&w), &hooks).await;
        assert_eq!(out, TickOutcome::Published);
        assert_eq!(w.calls.borrow().len(), 1);
        assert_eq!(w.calls.borrow()[0].1, utc(UPDATE_AT));
    }

    #[tokio::test]
    async fn update_rewrites_the_same_post() {
        let s = base_brief(true).await;
        let base_run = s.runs.borrow()[KEY].clone();
        let base_post = s.posts.borrow()[0].clone();
        let base_cover = base_post.cover_key.clone().unwrap();
        let (m, l) = (md(true), llm(true, true));
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let hooks = Hooks {
            log: &log,
            extra: &Vec::new,
            budget: &|| 0,
        };
        let out = tick_with_hooks(utc(UPDATE_AT), &s, &m, &FakeNs, &l, &hooks).await;
        assert_eq!(out, TickOutcome::Published);

        // One post, same id, run key, slug and publish time; new revision,
        // facts key and cover.
        let key = "covers/premarket-2026-10-29-update-1.svg";
        assert_eq!(
            *s.posts.borrow(),
            [PostRow {
                facts_r2_key: UPDATE_FACTS_KEY.into(),
                cover_key: Some(key.into()),
                revised_at: Some(UPDATE_AT.into()),
                ..base_post
            }]
        );
        assert_eq!(s.posts.borrow()[0].published_at.as_deref(), Some(BASE_AT));
        assert!(s.bytes.borrow()[key].0.starts_with(b"<svg"));
        // The old cover key is not reused (covers are cached for a day).
        assert_ne!(base_cover, key);
        // Both translations rewritten and published.
        for lang in ["en", "zh"] {
            let t = s.translations.borrow()[&(1, lang.to_string())].clone();
            assert_eq!(t.status, "published", "{lang}");
            assert_eq!(t.title, "Futures firm; Apple jumps", "{lang}");
            assert_eq!(t.updated_at, UPDATE_AT, "{lang}");
            assert!(t.cover_line1.is_some(), "{lang}");
        }
        assert!(s.translations.borrow()[&(1, "zh".to_string())]
            .body_md
            .contains("## 期货"));
        // Sources replaced, fresh facts under their own key (the 8:00 facts
        // kept), index line rewritten.
        let src = s.sources.borrow()[&1].clone();
        assert_eq!(
            src.iter().map(|r| r.url.as_str()).collect::<Vec<_>>(),
            [URL]
        );
        assert_eq!(stored_facts(&s).as_of_utc, utc(UPDATE_AT));
        let base_facts: PremarketFacts =
            serde_json::from_str(&s.facts.borrow()[FACTS_KEY]).unwrap();
        assert_eq!(base_facts.as_of_utc, utc(BASE_AT));
        assert_eq!(s.facts.borrow().len(), 2);
        let snap: IndexSnapshot = serde_json::from_str(&s.kv.borrow()[INDEX_LINE_KEY]).unwrap();
        assert_eq!(snap.as_of_utc, UPDATE_AT);
        // Runs: the update is published; the base is untouched.
        let r = run(&s).unwrap();
        assert_eq!(
            (r.status.as_str(), r.attempts, r.last_error.as_deref()),
            ("published", 1, None)
        );
        assert_eq!(r.content_type, "premarket-update");
        assert_eq!(s.runs.borrow()[KEY], base_run);
        assert!(s.alerts.borrow().is_empty());
        {
            let logs = logs.borrow();
            assert!(logs[0].starts_with("diagnostics: "), "{logs:?}");
            assert_eq!(logs[1..], ["cover=ok"], "{logs:?}");
        }

        // Later firings of either job change nothing.
        let after = snapshot(&s);
        for t in ["2026-10-29T12:40:00Z", "2026-10-29T13:25:00Z"] {
            assert_eq!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing
            );
            assert_eq!(
                base_tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Nothing
            );
        }
        assert_eq!(snapshot(&s), after);
    }

    #[tokio::test]
    async fn gate_or_draft_failure_leaves_the_post_untouched_and_retries() {
        // (market data ok, EN ok, ZH ok, error prefix)
        for (md_ok, en_ok, zh_ok, prefix) in [
            (false, true, true, "futures, movers | quotes ok=0 "),
            (true, false, true, "en: "),
            (true, true, false, "zh: "),
        ] {
            let s = base_brief(true).await;
            let before = snapshot(&s);
            let (m, l) = (md(md_ok), llm(en_ok, zh_ok));
            match tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await {
                TickOutcome::Retrying(e) => assert!(e.starts_with(prefix), "{e}"),
                o => panic!("{prefix}: {o:?}"),
            }
            assert_eq!(snapshot(&s), before, "{prefix}");
            let r = run(&s).unwrap();
            assert_eq!((r.status.as_str(), r.attempts), ("pending", 1), "{prefix}");
            assert!(r.last_error.unwrap().starts_with(prefix));
            assert!(s.alerts.borrow().is_empty());

            // The next firing retries, and this time everything passes.
            let (m, l) = (md(true), llm(true, true));
            assert_eq!(
                tick(utc("2026-10-29T12:40:00Z"), &s, &m, &FakeNs, &l).await,
                TickOutcome::Published,
                "{prefix}"
            );
            assert_eq!(run(&s).unwrap().attempts, 2);
            assert_eq!(
                s.posts.borrow()[0].revised_at.as_deref(),
                Some("2026-10-29T12:40:00Z")
            );
            assert_eq!(
                s.posts.borrow()[0].cover_key.as_deref(),
                Some("covers/premarket-2026-10-29-update-2.svg")
            );
        }
    }

    #[tokio::test]
    async fn failing_update_is_skipped_quietly_past_9_25() {
        let s = base_brief(true).await;
        let before = snapshot(&s);
        let (m, l) = (md(true), llm(true, false));
        for t in [UPDATE_AT, "2026-10-29T13:20:00Z"] {
            assert!(matches!(
                tick(utc(t), &s, &m, &FakeNs, &l).await,
                TickOutcome::Retrying(_)
            ));
        }
        assert_eq!(
            tick(utc("2026-10-29T13:25:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Skipped
        );
        let r = run(&s).unwrap();
        assert_eq!((r.status.as_str(), r.attempts), ("skipped", 2));
        // The last failure stays on the run.
        assert!(r.last_error.unwrap().starts_with("zh: "));
        assert!(s.alerts.borrow().is_empty());
        assert_eq!(snapshot(&s), before);
        assert_eq!(
            tick(utc("2026-10-29T13:30:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert!(s.alerts.borrow().is_empty());
    }

    /// Lists today's CPI, released with its figures.
    struct ReleasedCpi;
    #[async_trait(?Send)]
    impl crate::events::watch::WatchSource for ReleasedCpi {
        async fn watch_list(
            &self,
            _: NaiveDate,
            _: DateTime<Utc>,
            _: &dyn Fn() -> usize,
        ) -> crate::facts::Field<Vec<crate::events::watch::WatchItem>> {
            use crate::events::figures::{FigureKey, ReleaseFigure};
            use crate::events::watch::{WatchItem, WatchKind};
            let fig = |key, value| ReleaseFigure {
                key,
                value,
                quote: "The release's sentence.".into(),
            };
            crate::facts::Field::Value(vec![WatchItem {
                kind: WatchKind::Event,
                id: "bls:cpi:2026-09".into(),
                time_et: Some("8:30 AM ET".into()),
                time_zh: Some("上午8:30（美东）".into()),
                title: "September CPI".into(),
                title_zh: "9月消费者物价指数（CPI）".into(),
                note: None,
                note_zh: None,
                status: "released".into(),
                link: Some("https://www.bls.gov/news.release/archives/cpi_10292026.htm".into()),
                source_only: false,
                moved_from: None,
                figures: vec![
                    fig(FigureKey::CpiMonth, 0.4),
                    fig(FigureKey::CoreMonth, 0.3),
                ],
            }])
        }
    }

    /// Drafts that always set the CPI against forecasts in their summary.
    struct HotterThanExpected(FakeLlm);
    #[async_trait(?Send)]
    impl crate::providers::LlmProvider for HotterThanExpected {
        async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
            let mut v = self.0.complete_json(system, user).await?;
            if v.get("summary").is_some() {
                v["summary"] = Value::from("CPI came in hotter than expected.");
            }
            Ok(v)
        }
    }

    #[tokio::test]
    async fn a_forecast_guard_refusal_keeps_the_8_00_brief_and_retries() {
        let s = base_brief(true).await;
        let before = snapshot(&s);
        let (m, l) = (md(true), HotterThanExpected(llm(true, true)));
        let hooks = Hooks {
            log: &|_| {},
            extra: &Vec::new,
            budget: &|| 0,
        };
        let w = ReleasedCpi;
        for t in [UPDATE_AT, "2026-10-29T13:20:00Z"] {
            match tick_with_watch(utc(t), &s, &m, &FakeNs, &l, Some(&w), &hooks).await {
                TickOutcome::Retrying(e) => {
                    assert!(e.contains("compares a released figure"), "{e}")
                }
                o => panic!("{o:?}"),
            }
            // Nothing a reader sees changed.
            assert_eq!(snapshot(&s), before);
        }
        assert_eq!(
            tick_with_watch(
                utc("2026-10-29T13:25:00Z"),
                &s,
                &m,
                &FakeNs,
                &l,
                Some(&w),
                &hooks
            )
            .await,
            TickOutcome::Skipped
        );
        assert_eq!(snapshot(&s), before);
        assert_eq!(s.posts.borrow()[0].revised_at, None);
        assert!(s.alerts.borrow().is_empty());

        // A draft that states the figure only goes through.
        let s = base_brief(true).await;
        let l = llm(true, true);
        assert_eq!(
            tick_with_watch(utc(UPDATE_AT), &s, &m, &FakeNs, &l, Some(&w), &hooks).await,
            TickOutcome::Published
        );
        let en = s.translations.borrow()[&(1, "en".to_string())]
            .body_md
            .clone();
        assert!(en.contains("✓ Released: +0.4% on the month"), "{en}");
    }

    fn building(updated_at: &str) -> RunRow {
        RunRow {
            run_key: UPDATE_KEY.into(),
            content_type: "premarket-update".into(),
            trading_date: "2026-10-29".into(),
            status: "building".into(),
            attempts: 1,
            last_error: None,
            created_at: UPDATE_AT.into(),
            updated_at: updated_at.into(),
        }
    }

    #[tokio::test]
    async fn a_live_base_post_never_marks_the_update_published_without_a_build() {
        let s = base_brief(true).await;
        let before = snapshot(&s);
        let (m, l) = (md(false), llm(true, true));
        // A fresh `building` update is left alone.
        s.runs
            .borrow_mut()
            .insert(UPDATE_KEY.into(), building(UPDATE_AT));
        assert_eq!(
            tick(utc("2026-10-29T12:45:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert_eq!(run(&s).unwrap(), building(UPDATE_AT));
        assert_eq!(m.calls.get(), 0);
        // Once its lease expires a crashed update rebuilds: the base's live
        // EN does not count as the update having published.
        let out = tick(utc("2026-10-29T12:50:00Z"), &s, &m, &FakeNs, &l).await;
        assert!(matches!(out, TickOutcome::Retrying(_)), "{out:?}");
        assert!(m.calls.get() > 0, "it built");
        let r = run(&s).unwrap();
        assert_eq!((r.status.as_str(), r.attempts), ("pending", 2));
        // Past the deadline a fresh build is still left to finish.
        s.runs
            .borrow_mut()
            .insert(UPDATE_KEY.into(), building("2026-10-29T13:15:00Z"));
        assert_eq!(
            tick(utc("2026-10-29T13:25:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Nothing
        );
        assert_eq!(run(&s).unwrap().status, "building");
        assert_eq!(snapshot(&s), before);
    }

    #[tokio::test]
    async fn failed_rewrite_leaves_the_post_on_the_8_00_facts() {
        let s = base_brief(true).await;
        let before = snapshot(&s);
        s.fail_revise_post.set(true);
        let (m, l) = (md(true), llm(true, true));
        assert_eq!(
            tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await,
            TickOutcome::Retrying("d1: revise_post failed".into())
        );
        let after = snapshot(&s);
        assert_eq!(after.posts, before.posts);
        assert_eq!(after.posts[0].facts_r2_key, FACTS_KEY);
        assert_eq!(after.translations, before.translations);
        assert_eq!(after.sources, before.sources);
        assert_eq!(after.kv, before.kv);
        // The 8:00 facts are untouched; the update's own are an unreferenced orphan.
        assert_eq!(after.facts[FACTS_KEY], before.facts[FACTS_KEY]);
        assert_eq!(
            after
                .facts
                .keys()
                .filter(|k| *k != FACTS_KEY)
                .collect::<Vec<_>>(),
            [UPDATE_FACTS_KEY]
        );
        // Back to pending with the error, so the next firing retries.
        let r = run(&s).unwrap();
        assert_eq!(
            (r.status.as_str(), r.last_error.as_deref()),
            ("pending", Some("d1: revise_post failed"))
        );
        s.fail_revise_post.set(false);
        assert_eq!(
            tick(utc("2026-10-29T12:40:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        assert_eq!(stored_facts(&s).as_of_utc, utc("2026-10-29T12:40:00Z"));
        // Each attempt draws its own cover key.
        assert_eq!(
            s.posts.borrow()[0].cover_key.as_deref(),
            Some("covers/premarket-2026-10-29-update-2.svg")
        );
    }

    #[tokio::test]
    async fn base_zh_retry_reads_the_facts_the_post_points_at() {
        let s = base_brief(true).await;
        let (m, l) = (md(true), llm(true, true));
        assert_eq!(
            tick(utc(UPDATE_AT), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        // With the 8:00 facts gone and ZH pending again, a base ZH retry
        // can only draft from the update's facts key.
        s.facts.borrow_mut().remove(FACTS_KEY);
        s.translations
            .borrow_mut()
            .get_mut(&(1, "zh".to_string()))
            .unwrap()
            .status = "pending".into();
        assert_eq!(
            base_tick(utc("2026-10-29T12:45:00Z"), &s, &m, &FakeNs, &l).await,
            TickOutcome::Published
        );
        let zh = s.translations.borrow()[&(1, "zh".to_string())].clone();
        assert_eq!(
            (zh.status.as_str(), zh.updated_at.as_str()),
            ("published", "2026-10-29T12:45:00Z")
        );
    }

    /// An LLM that, on its first ZH draft, lets the update land (a full
    /// update tick) before it answers, as a concurrent update would.
    struct UpdateLandsMidDraft<'a> {
        inner: FakeLlm,
        store: &'a MemStore,
        landed: Cell<bool>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for UpdateLandsMidDraft<'_> {
        async fn complete_json(&self, system: &str, user: &str) -> Result<Value, ProviderError> {
            if system.starts_with("你负责撰写美股盘前晨报") && !self.landed.replace(true)
            {
                let (m, l) = (md(true), llm(true, true));
                let out = tick(utc("2026-10-29T12:40:00Z"), self.store, &m, &FakeNs, &l).await;
                assert_eq!(out, TickOutcome::Published);
            }
            self.inner.complete_json(system, user).await
        }
    }

    #[tokio::test]
    async fn base_zh_retry_from_the_old_facts_does_not_overwrite_the_update() {
        // The base published EN only at 08:00; its ZH retry at 08:45 drafts
        // from the 8:00 facts while the update (stamped 08:40) lands.
        let s = base_brief(false).await;
        assert_eq!(
            s.translations.borrow()[&(1, "zh".to_string())].status,
            "pending"
        );
        let racing = UpdateLandsMidDraft {
            inner: llm(true, true),
            store: &s,
            landed: Cell::new(false),
        };
        let out = base_tick(utc("2026-10-29T12:45:00Z"), &s, &md(true), &FakeNs, &racing).await;
        assert_eq!(out, TickOutcome::Nothing);
        assert!(racing.landed.get());
        let zh = s.translations.borrow()[&(1, "zh".to_string())].clone();
        assert_eq!(zh.status, "published");
        assert_eq!(zh.updated_at, "2026-10-29T12:40:00Z", "the update's ZH");
        assert_eq!(run(&s).unwrap().status, "published");
        // Without an update, the same retry still publishes the base ZH.
        let s = base_brief(false).await;
        let out = base_tick(
            utc("2026-10-29T12:45:00Z"),
            &s,
            &md(true),
            &FakeNs,
            &llm(true, true),
        )
        .await;
        assert_eq!(out, TickOutcome::Published);
        assert_eq!(
            s.translations.borrow()[&(1, "zh".to_string())].updated_at,
            "2026-10-29T12:45:00Z"
        );
    }

    #[tokio::test]
    async fn base_and_update_never_make_two_posts_for_one_day() {
        let s = MemStore::default();
        let (m, l) = (md(true), llm(true, true));
        // Every 5-minute firing from 08:00 to 09:30 ET, both jobs each time.
        for i in 0..=18 {
            let t = utc(BASE_AT) + chrono::Duration::minutes(5 * i);
            base_tick(t, &s, &m, &FakeNs, &l).await;
            tick(t, &s, &m, &FakeNs, &l).await;
        }
        let posts = s.posts.borrow();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].slug, "premarket-2026-10-29");
        assert_eq!(posts[0].run_key, KEY);
        assert_eq!(posts[0].published_at.as_deref(), Some(BASE_AT));
        assert_eq!(posts[0].revised_at.as_deref(), Some(UPDATE_AT));
        assert_eq!(run(&s).unwrap().attempts, 1);
    }
}
