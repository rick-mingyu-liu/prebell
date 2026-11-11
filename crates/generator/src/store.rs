//! `Store` on D1 (rows) and R2 (facts JSON). Column names follow
//! `migrations/0001_init.sql`; NULL text columns read back as "". Read
//! failures are returned as `Err`, never as a missing row.

use std::cell::Cell;

use async_trait::async_trait;
use prebell_core::cover::card::{CardStore, CoverPost};
use prebell_core::digest::{DigestStore, PostLink};
use prebell_core::facts::NewsReason;
use prebell_core::store::{PostRow, ReportRow, RunRow, Store, TranslationRow};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use worker::wasm_bindgen::JsValue;
use worker::{Bucket, D1Database, HttpMetadata};

pub struct D1Store {
    pub db: D1Database,
    pub facts: Bucket,
    /// D1 queries run since the last `reset_queries` (Free plan: 50 per
    /// invocation; a batch is one).
    pub queries: Cell<usize>,
}

pub(crate) fn s(v: &str) -> JsValue {
    JsValue::from_str(v)
}
pub(crate) fn opt(v: Option<&str>) -> JsValue {
    v.map_or(JsValue::NULL, JsValue::from_str)
}
pub(crate) fn int(v: i64) -> JsValue {
    // D1 takes JS numbers; BigInt is rejected.
    JsValue::from_f64(v as f64)
}
pub(crate) fn err(ctx: &str, e: worker::Error) -> String {
    format!("{ctx}: {e}")
}

#[derive(Deserialize)]
struct RunRec {
    run_key: String,
    content_type: String,
    trading_date: String,
    status: String,
    attempts: i64,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct PostRec {
    id: i64,
    run_key: String,
    content_type: String,
    template_id: String,
    trading_date: String,
    slug: String,
    facts_r2_key: Option<String>,
    published_at: Option<String>,
    cover_key: Option<String>,
    revised_at: Option<String>,
}

impl From<PostRec> for PostRow {
    fn from(p: PostRec) -> Self {
        PostRow {
            id: Some(p.id),
            run_key: p.run_key,
            content_type: p.content_type,
            template_id: p.template_id,
            trading_date: p.trading_date,
            slug: p.slug,
            facts_r2_key: p.facts_r2_key.unwrap_or_default(),
            published_at: p.published_at,
            cover_key: p.cover_key,
            revised_at: p.revised_at,
        }
    }
}

#[derive(Deserialize)]
struct TrRec {
    post_id: i64,
    lang: String,
    status: String,
    title: Option<String>,
    summary: Option<String>,
    body_md: Option<String>,
    updated_at: Option<String>,
    cover_line1: Option<String>,
    cover_line2: Option<String>,
}

/// A `post_translations` upsert of the row `$rows` gives (`?1`-`?9`).
macro_rules! upsert_translation {
    ($rows:literal) => {
        concat!(
            "INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at, cover_line1, cover_line2) ",
            $rows,
            " ON CONFLICT(post_id, lang) DO UPDATE SET status = excluded.status, title = excluded.title, \
             summary = excluded.summary, body_md = excluded.body_md, updated_at = excluded.updated_at, \
             cover_line1 = excluded.cover_line1, cover_line2 = excluded.cover_line2"
        )
    };
}

const UPSERT_TRANSLATION: &str = upsert_translation!("VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)");
/// Writes only while post `?1` exists with `revised_at IS ?10`. The WHERE
/// also keeps SQLite from reading `ON CONFLICT` as a join constraint.
const UPSERT_TRANSLATION_AT_REVISION: &str = upsert_translation!(
    "SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9 \
     WHERE EXISTS (SELECT 1 FROM posts WHERE id = ?1 AND revised_at IS ?10)"
);

fn translation_args(t: &TranslationRow) -> [JsValue; 9] {
    [
        int(t.post_id),
        s(&t.lang),
        s(&t.status),
        s(&t.title),
        s(&t.summary),
        s(&t.body_md),
        s(&t.updated_at),
        opt(t.cover_line1.as_deref()),
        opt(t.cover_line2.as_deref()),
    ]
}

/// `UPSERT_TRANSLATION_AT_REVISION`'s arguments.
fn at_revision_args(t: &TranslationRow, revised_at: Option<&str>) -> Vec<JsValue> {
    let mut args = translation_args(t).to_vec();
    args.push(opt(revised_at));
    args
}

const INSERT_SOURCE: &str =
    "INSERT INTO sources (post_id, url, title, quote) VALUES (?1, ?2, ?3, ?4)";
/// `INSERT_SOURCE`, only while post `?1` exists.
const INSERT_SOURCE_OF_POST: &str = "INSERT INTO sources (post_id, url, title, quote) \
     SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM posts WHERE id = ?1)";

/// `upsert_post`. `revised_at` is written on insert and kept on conflict:
/// only `revise_post` changes it.
const UPSERT_POST: &str =
    "INSERT INTO posts (run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, cover_key, revised_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
     ON CONFLICT(run_key) DO UPDATE SET content_type = excluded.content_type, \
     template_id = excluded.template_id, trading_date = excluded.trading_date, slug = excluded.slug, \
     facts_r2_key = excluded.facts_r2_key, published_at = excluded.published_at, \
     cover_key = excluded.cover_key \
     RETURNING id";

/// `revise_post`'s first statement; every other column, `published_at` included, is kept.
const REVISE_POST: &str =
    "UPDATE posts SET facts_r2_key = ?1, cover_key = ?2, revised_at = ?3 WHERE id = ?4";

/// Rows a statement changed; 0 when D1 does not say.
fn changes(r: &worker::D1Result) -> usize {
    r.meta().ok().flatten().and_then(|m| m.changes).unwrap_or(0)
}

/// Runs `stmts` as one D1 batch, which is one transaction.
async fn batch(
    db: &D1Database,
    ctx: &str,
    stmts: Vec<worker::D1PreparedStatement>,
) -> Result<Vec<worker::D1Result>, String> {
    let results = db.batch(stmts).await.map_err(|e| err(ctx, e))?;
    match results.iter().find(|r| !r.success()) {
        Some(r) => Err(format!("{ctx}: {}", r.error().unwrap_or_default())),
        None => Ok(results),
    }
}

#[derive(Deserialize)]
struct IdRec {
    id: i64,
}

/// An `alerts` row not yet delivered.
#[derive(Deserialize, Debug, Clone)]
pub struct AlertRec {
    pub id: i64,
    pub run_key: Option<String>,
    pub level: String,
    pub message: String,
}

pub fn now_iso() -> String {
    let ms = worker::Date::now().as_millis() as i64;
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl D1Store {
    pub fn new(db: D1Database, facts: Bucket) -> Self {
        Self {
            db,
            facts,
            queries: Cell::new(0),
        }
    }

    /// Counts one D1 query (a statement or a batch) against the Free plan's 50.
    pub(crate) fn query(&self) {
        self.queries.set(self.queries.get() + 1);
    }

    /// Returns the queries counted so far and starts again from 0.
    pub fn reset_queries(&self) -> usize {
        self.queries.replace(0)
    }

    pub(crate) async fn first<T: DeserializeOwned>(
        &self,
        sql: &str,
        args: &[JsValue],
    ) -> Result<Option<T>, String> {
        self.query();
        let stmt = self.db.prepare(sql).bind(args).map_err(|e| err(sql, e))?;
        stmt.first::<T>(None).await.map_err(|e| err(sql, e))
    }

    /// Insert or replace a `kv` row; shared by the `Store` and `EarningsStore` impls.
    pub(crate) async fn kv_upsert(&self, key: &str, value: &str, now: &str) -> Result<(), String> {
        self.run(
            "INSERT INTO kv (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            &[s(key), s(value), s(now)],
        )
        .await
    }

    pub(crate) async fn run(&self, sql: &str, args: &[JsValue]) -> Result<(), String> {
        self.query();
        let stmt = self.db.prepare(sql).bind(args).map_err(|e| err(sql, e))?;
        let r = stmt.run().await.map_err(|e| err(sql, e))?;
        match r.error() {
            Some(e) if !r.success() => Err(format!("{sql}: {e}")),
            _ => Ok(()),
        }
    }
}

#[async_trait(?Send)]
impl crate::alerts::AlertQueue for D1Store {
    async fn unsent_alerts(&self, limit: usize) -> Result<Vec<AlertRec>, String> {
        let sql =
            "SELECT id, run_key, level, message FROM alerts WHERE sent = 0 ORDER BY id LIMIT ?1";
        self.query();
        let r = self
            .db
            .prepare(sql)
            .bind(&[int(limit as i64)])
            .map_err(|e| err(sql, e))?
            .all()
            .await
            .map_err(|e| err(sql, e))?;
        r.results::<AlertRec>().map_err(|e| err(sql, e))
    }

    async fn mark_alerts_sent(&self, ids: &[i64]) -> Result<(), String> {
        let marks: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "UPDATE alerts SET sent = 1 WHERE id IN ({})",
            marks.join(", ")
        );
        let args: Vec<JsValue> = ids.iter().map(|id| int(*id)).collect();
        self.run(&sql, &args).await
    }
}

#[async_trait(?Send)]
impl Store for D1Store {
    async fn get_run(&self, key: &str) -> Result<Option<RunRow>, String> {
        let r: Option<RunRec> = self.first(
            "SELECT run_key, content_type, trading_date, status, attempts, last_error, created_at, updated_at \
             FROM runs WHERE run_key = ?1",
            &[s(key)],
        )
        .await?;
        Ok(r.map(|r| RunRow {
            run_key: r.run_key,
            content_type: r.content_type,
            trading_date: r.trading_date,
            status: r.status,
            attempts: r.attempts,
            last_error: r.last_error,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    async fn upsert_run(&self, r: &RunRow) -> Result<(), String> {
        self.run(
            "INSERT INTO runs (run_key, content_type, trading_date, status, attempts, last_error, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(run_key) DO UPDATE SET content_type = excluded.content_type, \
             trading_date = excluded.trading_date, status = excluded.status, attempts = excluded.attempts, \
             last_error = excluded.last_error, created_at = excluded.created_at, updated_at = excluded.updated_at",
            &[
                s(&r.run_key),
                s(&r.content_type),
                s(&r.trading_date),
                s(&r.status),
                int(r.attempts),
                opt(r.last_error.as_deref()),
                s(&r.created_at),
                s(&r.updated_at),
            ],
        )
        .await
    }

    async fn get_post_by_run(&self, run_key: &str) -> Result<Option<PostRow>, String> {
        let r: Option<PostRec> = self.first(
            "SELECT id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, cover_key, revised_at \
             FROM posts WHERE run_key = ?1",
            &[s(run_key)],
        )
        .await?;
        Ok(r.map(PostRow::from))
    }

    async fn posts_on(
        &self,
        content_type: &str,
        trading_date: &str,
    ) -> Result<Vec<PostRow>, String> {
        let sql = "SELECT id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, cover_key, revised_at \
                   FROM posts WHERE content_type = ?1 AND trading_date = ?2 AND published_at IS NOT NULL ORDER BY id";
        self.query();
        let r = self
            .db
            .prepare(sql)
            .bind(&[s(content_type), s(trading_date)])
            .map_err(|e| err(sql, e))?
            .all()
            .await
            .map_err(|e| err(sql, e))?;
        let rows = r.results::<PostRec>().map_err(|e| err(sql, e))?;
        Ok(rows.into_iter().map(PostRow::from).collect())
    }

    async fn upsert_post(&self, p: &PostRow) -> Result<i64, String> {
        let r: Option<IdRec> = self
            .first(
                UPSERT_POST,
                &[
                    s(&p.run_key),
                    s(&p.content_type),
                    s(&p.template_id),
                    s(&p.trading_date),
                    s(&p.slug),
                    s(&p.facts_r2_key),
                    opt(p.published_at.as_deref()),
                    opt(p.cover_key.as_deref()),
                    opt(p.revised_at.as_deref()),
                ],
            )
            .await?;
        r.map(|r| r.id)
            .ok_or_else(|| "upsert_post returned no id".to_string())
    }

    async fn revise_post(
        &self,
        p: &PostRow,
        t: &[TranslationRow],
        src: &[NewsReason],
    ) -> Result<(), String> {
        let ctx = "revise_post";
        let id = p.id.ok_or("revise_post: post has no id")?;
        let mut stmts = vec![self
            .db
            .prepare(REVISE_POST)
            .bind(&[
                s(&p.facts_r2_key),
                opt(p.cover_key.as_deref()),
                opt(p.revised_at.as_deref()),
                int(id),
            ])
            .map_err(|e| err(ctx, e))?];
        // Every later statement is conditional on the post, so a post gone
        // missing gets nothing written for it.
        for tr in t {
            stmts.push(
                self.db
                    .prepare(UPSERT_TRANSLATION_AT_REVISION)
                    .bind(&at_revision_args(tr, p.revised_at.as_deref()))
                    .map_err(|e| err(ctx, e))?,
            );
        }
        stmts.push(
            self.db
                .prepare("DELETE FROM sources WHERE post_id = ?1")
                .bind(&[int(id)])
                .map_err(|e| err(ctx, e))?,
        );
        for r in src {
            stmts.push(
                self.db
                    .prepare(INSERT_SOURCE_OF_POST)
                    .bind(&[int(id), s(&r.url), s(&r.title), s(&r.quote)])
                    .map_err(|e| err(ctx, e))?,
            );
        }
        // One batch = one transaction: the post, its text and its sources land together.
        self.query();
        let results = batch(&self.db, ctx, stmts).await?;
        match results.first().map(changes) {
            Some(1) => Ok(()),
            _ => Err(format!("{ctx}: no post {id}")),
        }
    }

    async fn get_translation(
        &self,
        post_id: i64,
        lang: &str,
    ) -> Result<Option<TranslationRow>, String> {
        let r: Option<TrRec> = self
            .first(
                "SELECT post_id, lang, status, title, summary, body_md, updated_at, cover_line1, cover_line2 \
             FROM post_translations WHERE post_id = ?1 AND lang = ?2",
                &[int(post_id), s(lang)],
            )
            .await?;
        Ok(r.map(|t| TranslationRow {
            post_id: t.post_id,
            lang: t.lang,
            status: t.status,
            title: t.title.unwrap_or_default(),
            summary: t.summary.unwrap_or_default(),
            body_md: t.body_md.unwrap_or_default(),
            updated_at: t.updated_at.unwrap_or_default(),
            cover_line1: t.cover_line1,
            cover_line2: t.cover_line2,
        }))
    }

    async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String> {
        self.run(UPSERT_TRANSLATION, &translation_args(t)).await
    }

    async fn upsert_translation_at_revision(
        &self,
        t: &TranslationRow,
        revised_at: Option<&str>,
    ) -> Result<bool, String> {
        let sql = UPSERT_TRANSLATION_AT_REVISION;
        let stmt = self
            .db
            .prepare(sql)
            .bind(&at_revision_args(t, revised_at))
            .map_err(|e| err(sql, e))?;
        self.query();
        let r = stmt.run().await.map_err(|e| err(sql, e))?;
        match r.error() {
            Some(e) if !r.success() => Err(format!("{sql}: {e}")),
            _ => Ok(changes(&r) > 0),
        }
    }

    async fn add_sources(&self, post_id: i64, src: &[NewsReason]) -> Result<(), String> {
        let ctx = "add_sources";
        let mut stmts = vec![self
            .db
            .prepare("DELETE FROM sources WHERE post_id = ?1")
            .bind(&[int(post_id)])
            .map_err(|e| err(ctx, e))?];
        for r in src {
            stmts.push(
                self.db
                    .prepare(INSERT_SOURCE)
                    .bind(&[int(post_id), s(&r.url), s(&r.title), s(&r.quote)])
                    .map_err(|e| err(ctx, e))?,
            );
        }
        // One batch = one transaction: delete and re-insert land together.
        self.query();
        batch(&self.db, ctx, stmts).await.map(|_| ())
    }

    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String> {
        self.run(
            "INSERT INTO alerts (run_key, level, message, created_at, sent) VALUES (?1, ?2, ?3, ?4, 0)",
            &[s(run_key), s(level), s(msg), s(&now_iso())],
        )
        .await
    }

    async fn has_alert(&self, run_key: &str) -> Result<bool, String> {
        #[derive(serde::Deserialize)]
        struct One {
            n: i64,
        }
        let r: Option<One> = self
            .first(
                "SELECT COUNT(*) AS n FROM alerts WHERE run_key = ?1",
                &[s(run_key)],
            )
            .await?;
        Ok(r.is_some_and(|r| r.n > 0))
    }

    async fn put_facts(&self, key: &str, json: &str) -> Result<(), String> {
        self.facts
            .put(key, json.to_string())
            .execute()
            .await
            .map(|_| ())
            .map_err(|e| err("R2 put", e))
    }

    async fn get_facts(&self, key: &str) -> Result<Option<String>, String> {
        let ctx = format!("R2 get {key}");
        let Some(obj) = self
            .facts
            .get(key)
            .execute()
            .await
            .map_err(|e| err(&ctx, e))?
        else {
            return Ok(None);
        };
        let body = obj
            .body()
            .ok_or_else(|| format!("{ctx}: object has no body"))?;
        body.text().await.map(Some).map_err(|e| err(&ctx, e))
    }

    async fn put_bytes(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), String> {
        self.facts
            .put(key, bytes.to_vec())
            .http_metadata(HttpMetadata {
                content_type: Some(content_type.into()),
                ..Default::default()
            })
            .execute()
            .await
            .map(|_| ())
            .map_err(|e| err("R2 put", e))
    }

    async fn kv_set(&self, key: &str, value: &str, now: &str) -> Result<(), String> {
        self.kv_upsert(key, value, now).await
    }

    async fn reported_between(&self, from: &str, to: &str) -> Result<Vec<ReportRow>, String> {
        let rows: Vec<ReportRec> = self.all(REPORTED_BETWEEN, &[s(from), s(to)]).await?;
        Ok(rows
            .into_iter()
            .map(|r| ReportRow {
                ticker: r.ticker,
                report_date: r.report_date,
                time_slot: r.time_slot,
                accepted_at: r
                    .accepted_at
                    .map(|a| a.split(' ').map(str::to_string).collect())
                    .unwrap_or_default(),
                proof_url: r.proof_url,
                post_slug: r.post_slug,
            })
            .collect())
    }
}

/// Reported calendar rows from `?1` to `?2` (ET dates) with the earnings
/// check for each and the measured filing times, in one query. The check is
/// an earnings post with a published EN translation, matched by ticker and ET
/// date as the web's calendar matches them; CROSS JOIN pins the join order so
/// SQLite starts from `idx_posts_trading_date`. `accepted_at` lists (space
/// separated) the ticker's filings accepted from the UTC day before the
/// report date to the UTC day after it, which covers the ET date; the reader
/// keeps those on the ET date.
pub(crate) const REPORTED_BETWEEN: &str = "SELECT c.ticker, c.report_date, c.time_slot, c.proof_url, x.slug AS post_slug, \
     (SELECT GROUP_CONCAT(f2.accepted_at, ' ') FROM earnings_filings f2 WHERE f2.ticker = c.ticker \
       AND f2.accepted_at >= date(c.report_date, '-1 day') AND f2.accepted_at < date(c.report_date, '+2 day')) AS accepted_at \
     FROM earnings_calendar c \
     LEFT JOIN (SELECT f.ticker, p.trading_date, MIN(p.slug) AS slug FROM posts p \
       CROSS JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' AND en.status = 'published' \
       CROSS JOIN earnings_filings f ON f.run_key = p.run_key \
       WHERE p.trading_date BETWEEN ?1 AND ?2 AND p.published_at IS NOT NULL \
       GROUP BY f.ticker, p.trading_date) x ON x.ticker = c.ticker AND x.trading_date = c.report_date \
     WHERE c.report_date BETWEEN ?1 AND ?2 AND c.status = 'reported' \
     ORDER BY c.report_date, c.ticker";

#[derive(Deserialize)]
struct ReportRec {
    ticker: String,
    report_date: String,
    time_slot: String,
    proof_url: Option<String>,
    post_slug: Option<String>,
    accepted_at: Option<String>,
}

/// Newest published posts with an SVG cover and a trading date on or after
/// `?2`, for the `cards` job (`?1` is the limit). The date bound reads
/// `idx_posts_trading_date`, so the rows read stay flat as history grows. A
/// revised post counts from its update, which gave it a new cover key.
pub(crate) const COVER_POSTS: &str = "SELECT p.cover_key, p.trading_date, f.ticker FROM posts p \
     LEFT JOIN earnings_filings f ON f.run_key = p.run_key \
     WHERE p.trading_date >= ?2 AND p.published_at IS NOT NULL AND p.cover_key LIKE 'covers/%.svg' \
     ORDER BY COALESCE(p.revised_at, p.published_at) DESC LIMIT ?1";

#[derive(Deserialize)]
struct CoverPostRec {
    cover_key: String,
    trading_date: String,
    ticker: Option<String>,
}

#[async_trait(?Send)]
impl CardStore for D1Store {
    async fn cover_posts(&self, limit: usize, since: &str) -> Result<Vec<CoverPost>, String> {
        self.query();
        let stmt = self
            .db
            .prepare(COVER_POSTS)
            .bind(&[int(limit as i64), s(since)])
            .map_err(|e| err(COVER_POSTS, e))?;
        let rows = stmt.all().await.map_err(|e| err(COVER_POSTS, e))?;
        let rows: Vec<CoverPostRec> = rows.results().map_err(|e| err(COVER_POSTS, e))?;
        Ok(rows
            .into_iter()
            .map(|r| CoverPost {
                cover_key: r.cover_key,
                trading_date: r.trading_date,
                ticker: r.ticker,
            })
            .collect())
    }

    async fn has_object(&self, key: &str) -> Result<bool, String> {
        self.facts
            .head(key)
            .await
            .map(|o| o.is_some())
            .map_err(|e| err(&format!("R2 head {key}"), e))
    }

    async fn get_text(&self, key: &str) -> Result<Option<String>, String> {
        Store::get_facts(self, key).await
    }

    async fn put_png(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        Store::put_bytes(self, key, bytes, "image/png").await
    }

    async fn put_text(&self, key: &str, text: &str) -> Result<(), String> {
        Store::put_bytes(self, key, text.as_bytes(), "text/plain").await
    }
}

/// The digest's posts: those whose English page is published, with that
/// page's title and one-line summary, by type and trading-date range.
pub(crate) const DIGEST_POSTS: &str = "SELECT p.slug, p.trading_date, f.ticker, \
     en.title AS title, en.summary AS summary \
     FROM posts p \
     JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' \
     LEFT JOIN earnings_filings f ON f.run_key = p.run_key \
     WHERE p.content_type = ?1 AND p.trading_date BETWEEN ?2 AND ?3 AND p.published_at IS NOT NULL \
     AND en.status = 'published' AND en.title IS NOT NULL \
     ORDER BY p.trading_date, p.published_at, p.id";

#[derive(Deserialize)]
struct DigestPostRec {
    slug: String,
    trading_date: String,
    ticker: Option<String>,
    title: String,
    summary: Option<String>,
}

#[async_trait(?Send)]
impl DigestStore for D1Store {
    async fn published_posts(
        &self,
        content_type: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<PostLink>, String> {
        let rows: Vec<DigestPostRec> = self
            .all(DIGEST_POSTS, &[s(content_type), s(from), s(to)])
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| PostLink {
                slug: r.slug,
                trading_date: r.trading_date,
                ticker: r.ticker,
                title: r.title,
                summary: r.summary.unwrap_or_default(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The column list of an `ON CONFLICT ... DO UPDATE SET` clause.
    fn updated_columns(sql: &str) -> Vec<&str> {
        let set = &sql[sql.find("DO UPDATE SET").expect("an upsert") + "DO UPDATE SET".len()..];
        let set = set.split(" RETURNING").next().unwrap();
        set.split(',')
            .map(|a| a.split('=').next().unwrap().trim())
            .collect()
    }

    #[test]
    fn upsert_post_writes_revised_at_on_insert_and_keeps_it_on_conflict() {
        let (insert, _) = UPSERT_POST.split_once("ON CONFLICT").unwrap();
        assert!(insert.contains("cover_key, revised_at)"), "{insert}");
        assert!(insert.contains("?8, ?9)"), "{insert}");
        let cols = updated_columns(UPSERT_POST);
        assert!(!cols.contains(&"revised_at"), "{cols:?}");
        assert!(cols.contains(&"published_at") && cols.contains(&"cover_key"));
        assert!(UPSERT_POST.ends_with("RETURNING id"));
    }

    #[test]
    fn revise_post_sets_only_the_facts_key_cover_key_and_revised_at() {
        assert_eq!(
            REVISE_POST,
            "UPDATE posts SET facts_r2_key = ?1, cover_key = ?2, revised_at = ?3 WHERE id = ?4"
        );
        // Its other statements write only while the post exists.
        assert!(INSERT_SOURCE_OF_POST.ends_with("WHERE EXISTS (SELECT 1 FROM posts WHERE id = ?1)"));
        assert!(UPSERT_TRANSLATION_AT_REVISION.contains(
            "WHERE EXISTS (SELECT 1 FROM posts WHERE id = ?1 AND revised_at IS ?10) ON CONFLICT"
        ));
    }

    #[test]
    fn both_translation_upserts_replace_the_same_columns() {
        let cols = [
            "status",
            "title",
            "summary",
            "body_md",
            "updated_at",
            "cover_line1",
            "cover_line2",
        ];
        assert_eq!(updated_columns(UPSERT_TRANSLATION), cols);
        assert_eq!(updated_columns(UPSERT_TRANSLATION_AT_REVISION), cols);
        assert!(UPSERT_TRANSLATION
            .contains(") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT"));
        assert!(UPSERT_TRANSLATION_AT_REVISION
            .contains(") SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9 WHERE"));
    }
}
