//! `Store` on D1 (rows) and R2 (facts JSON). Column names follow
//! `migrations/0001_init.sql`; NULL text columns read back as "". Read
//! failures are returned as `Err`, never as a missing row.

use async_trait::async_trait;
use prebell_core::facts::NewsReason;
use prebell_core::store::{PostRow, RunRow, Store, TranslationRow};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use worker::wasm_bindgen::JsValue;
use worker::{Bucket, D1Database};

pub struct D1Store {
    pub db: D1Database,
    pub facts: Bucket,
}

pub(crate) fn s(v: &str) -> JsValue {
    JsValue::from_str(v)
}
pub(crate) fn opt(v: Option<&str>) -> JsValue {
    v.map_or(JsValue::NULL, JsValue::from_str)
}
fn int(v: i64) -> JsValue {
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
    pub(crate) async fn first<T: DeserializeOwned>(
        &self,
        sql: &str,
        args: &[JsValue],
    ) -> Result<Option<T>, String> {
        let stmt = self.db.prepare(sql).bind(args).map_err(|e| err(sql, e))?;
        stmt.first::<T>(None).await.map_err(|e| err(sql, e))
    }

    pub(crate) async fn run(&self, sql: &str, args: &[JsValue]) -> Result<(), String> {
        let stmt = self.db.prepare(sql).bind(args).map_err(|e| err(sql, e))?;
        let r = stmt.run().await.map_err(|e| err(sql, e))?;
        match r.error() {
            Some(e) if !r.success() => Err(format!("{sql}: {e}")),
            _ => Ok(()),
        }
    }

    pub async fn unsent_alerts(&self) -> Result<Vec<AlertRec>, String> {
        let sql = "SELECT id, run_key, level, message FROM alerts WHERE sent = 0 ORDER BY id";
        let r = self.db.prepare(sql).all().await.map_err(|e| err(sql, e))?;
        r.results::<AlertRec>().map_err(|e| err(sql, e))
    }

    pub async fn mark_alert_sent(&self, id: i64) -> Result<(), String> {
        self.run("UPDATE alerts SET sent = 1 WHERE id = ?1", &[int(id)])
            .await
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
            "SELECT id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at \
             FROM posts WHERE run_key = ?1",
            &[s(run_key)],
        )
        .await?;
        Ok(r.map(|p| PostRow {
            id: Some(p.id),
            run_key: p.run_key,
            content_type: p.content_type,
            template_id: p.template_id,
            trading_date: p.trading_date,
            slug: p.slug,
            facts_r2_key: p.facts_r2_key.unwrap_or_default(),
            published_at: p.published_at,
        }))
    }

    async fn upsert_post(&self, p: &PostRow) -> Result<i64, String> {
        let r: Option<IdRec> = self
            .first(
                "INSERT INTO posts (run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(run_key) DO UPDATE SET content_type = excluded.content_type, \
                 template_id = excluded.template_id, trading_date = excluded.trading_date, slug = excluded.slug, \
                 facts_r2_key = excluded.facts_r2_key, published_at = excluded.published_at \
                 RETURNING id",
                &[
                    s(&p.run_key),
                    s(&p.content_type),
                    s(&p.template_id),
                    s(&p.trading_date),
                    s(&p.slug),
                    s(&p.facts_r2_key),
                    opt(p.published_at.as_deref()),
                ],
            )
            .await?;
        r.map(|r| r.id)
            .ok_or_else(|| "upsert_post returned no id".to_string())
    }

    async fn get_translation(
        &self,
        post_id: i64,
        lang: &str,
    ) -> Result<Option<TranslationRow>, String> {
        let r: Option<TrRec> = self
            .first(
                "SELECT post_id, lang, status, title, summary, body_md, updated_at \
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
        }))
    }

    async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String> {
        self.run(
            "INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(post_id, lang) DO UPDATE SET status = excluded.status, title = excluded.title, \
             summary = excluded.summary, body_md = excluded.body_md, updated_at = excluded.updated_at",
            &[int(t.post_id), s(&t.lang), s(&t.status), s(&t.title), s(&t.summary), s(&t.body_md), s(&t.updated_at)],
        )
        .await
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
                    .prepare(
                        "INSERT INTO sources (post_id, url, title, quote) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .bind(&[int(post_id), s(&r.url), s(&r.title), s(&r.quote)])
                    .map_err(|e| err(ctx, e))?,
            );
        }
        // One batch = one transaction: delete and re-insert land together.
        let results = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
        match results.iter().find(|r| !r.success()) {
            Some(r) => Err(format!("{ctx}: {}", r.error().unwrap_or_default())),
            None => Ok(()),
        }
    }

    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String> {
        self.run(
            "INSERT INTO alerts (run_key, level, message, created_at, sent) VALUES (?1, ?2, ?3, ?4, 0)",
            &[s(run_key), s(level), s(msg), s(&now_iso())],
        )
        .await
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
}
