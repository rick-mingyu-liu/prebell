//! `WeekStore` on D1 (`prebell_core::week::store`). The SQL is checked
//! against the real schema, with its query plans, in
//! `apps/web/tests/weekquery.test.ts`.

use async_trait::async_trait;
use prebell_core::store::{PostRow, RunRow};
use prebell_core::week::store::{CheckRow, WeekRun, WeekStore, Written};
use serde::Deserialize;
use worker::wasm_bindgen::JsValue;

use crate::store::{int, s, D1Store};

/// The `week:*` runs that are `pending` or `building`, and the run `?1`
/// whatever its status, each with whether its post's English text is live:
/// one range read on the `runs` primary key, every poll while the switch is on.
pub(crate) const WEEK_RUNS: &str = "SELECT r.run_key, r.content_type, r.trading_date, r.status, r.attempts, \
     r.last_error, r.created_at, r.updated_at, \
     EXISTS (SELECT 1 FROM posts p CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = 'en' \
     WHERE p.run_key = r.run_key AND t.status = 'published') AS live \
     FROM runs r WHERE r.run_key >= 'week:' AND r.run_key < 'week;' \
     AND (r.status IN ('pending', 'building') OR r.run_key = ?1) ORDER BY r.run_key";

/// Up to `?2` published week posts before the Friday `?1`, newest first.
pub(crate) const WEEK_POSTS: &str =
    "SELECT p.id, p.run_key, p.content_type, p.template_id, p.trading_date, p.slug, \
     p.facts_r2_key, p.published_at, p.cover_key, p.revised_at \
     FROM posts p CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = 'en' \
     WHERE p.content_type = 'week' AND p.trading_date < ?1 AND p.published_at IS NOT NULL \
     AND t.status = 'published' ORDER BY p.trading_date DESC LIMIT ?2";

/// Earnings checks with their English text published, from the report date
/// `?1`, published by `?2`, with both translations' cover lines.
pub(crate) const WEEK_CHECKS: &str =
    "SELECT p.slug, p.run_key, p.trading_date, p.published_at, p.revised_at, \
     en.cover_line1 AS en1, en.cover_line2 AS en2, \
     zh.status AS zh_status, zh.cover_line1 AS zh1, zh.cover_line2 AS zh2 \
     FROM posts p CROSS JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' \
     LEFT JOIN post_translations zh ON zh.post_id = p.id AND zh.lang = 'zh' \
     WHERE p.content_type = 'earnings' AND p.trading_date >= ?1 AND p.published_at IS NOT NULL \
     AND p.published_at <= ?2 AND en.status = 'published' ORDER BY p.trading_date, p.slug";

/// `WEEK_CHECKS`' columns for the slugs in `{slugs}` (a rebuild's stored list).
pub(crate) const CHECKS_BY_SLUG: &str =
    "SELECT p.slug, p.run_key, p.trading_date, p.published_at, p.revised_at, \
     en.cover_line1 AS en1, en.cover_line2 AS en2, \
     zh.status AS zh_status, zh.cover_line1 AS zh1, zh.cover_line2 AS zh2 \
     FROM posts p CROSS JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' \
     LEFT JOIN post_translations zh ON zh.post_id = p.id AND zh.lang = 'zh' \
     WHERE p.slug IN ({slugs}) AND p.content_type = 'earnings' AND p.published_at IS NOT NULL \
     AND en.status = 'published' ORDER BY p.trading_date, p.slug";

/// When the earnings checks in `{slugs}` were last written (the stale check).
pub(crate) const WRITTEN: &str = "SELECT slug, published_at, revised_at FROM posts \
     WHERE slug IN ({slugs}) AND content_type = 'earnings'";

/// Slugs per statement: D1 binds at most 100 values.
const CHUNK: usize = 90;

#[derive(Deserialize)]
struct WeekRunRec {
    run_key: String,
    content_type: String,
    trading_date: String,
    status: String,
    attempts: i64,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
    live: i64,
}

#[derive(Deserialize)]
struct WeekPostRec {
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

#[derive(Deserialize)]
struct CheckRec {
    slug: String,
    run_key: String,
    trading_date: String,
    published_at: String,
    revised_at: Option<String>,
    en1: Option<String>,
    en2: Option<String>,
    zh_status: Option<String>,
    zh1: Option<String>,
    zh2: Option<String>,
}

impl From<CheckRec> for CheckRow {
    fn from(r: CheckRec) -> Self {
        CheckRow {
            slug: r.slug,
            run_key: r.run_key,
            trading_date: r.trading_date,
            published_at: r.published_at,
            revised_at: r.revised_at,
            en: [r.en1, r.en2],
            zh: [r.zh1, r.zh2],
            zh_status: r.zh_status,
        }
    }
}

#[derive(Deserialize)]
struct WrittenRec {
    slug: String,
    published_at: Option<String>,
    revised_at: Option<String>,
}

/// `sql` with `{slugs}` replaced by `?1`..`?n`.
fn marked(sql: &str, n: usize) -> String {
    let marks: Vec<String> = (1..=n).map(|i| format!("?{i}")).collect();
    sql.replace("{slugs}", &marks.join(", "))
}

/// `marked`, and the slugs as its arguments.
fn in_list(sql: &str, slugs: &[String]) -> (String, Vec<JsValue>) {
    (
        marked(sql, slugs.len()),
        slugs.iter().map(|x| s(x)).collect(),
    )
}

#[async_trait(?Send)]
impl WeekStore for D1Store {
    async fn week_runs(&self, this: &str) -> Result<Vec<WeekRun>, String> {
        let rows: Vec<WeekRunRec> = self.all(WEEK_RUNS, &[s(this)]).await?;
        Ok(rows
            .into_iter()
            .map(|r| WeekRun {
                live: r.live != 0,
                run: RunRow {
                    run_key: r.run_key,
                    content_type: r.content_type,
                    trading_date: r.trading_date,
                    status: r.status,
                    attempts: r.attempts,
                    last_error: r.last_error,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                },
            })
            .collect())
    }

    async fn week_posts(&self, friday: &str, limit: usize) -> Result<Vec<PostRow>, String> {
        let rows: Vec<WeekPostRec> = self
            .all(WEEK_POSTS, &[s(friday), int(limit as i64)])
            .await?;
        Ok(rows
            .into_iter()
            .map(|p| PostRow {
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
            })
            .collect())
    }

    async fn week_checks(&self, since: &str, cutoff: &str) -> Result<Vec<CheckRow>, String> {
        let rows: Vec<CheckRec> = self.all(WEEK_CHECKS, &[s(since), s(cutoff)]).await?;
        Ok(rows.into_iter().map(CheckRow::from).collect())
    }

    async fn checks_by_slug(&self, slugs: &[String]) -> Result<Vec<CheckRow>, String> {
        let mut out: Vec<CheckRow> = Vec::new();
        for chunk in slugs.chunks(CHUNK) {
            let (sql, args) = in_list(CHECKS_BY_SLUG, chunk);
            let rows: Vec<CheckRec> = self.all(&sql, &args).await?;
            out.extend(rows.into_iter().map(CheckRow::from));
        }
        out.sort_by(|a, b| (&a.trading_date, &a.slug).cmp(&(&b.trading_date, &b.slug)));
        Ok(out)
    }

    async fn written(&self, slugs: &[String]) -> Result<Vec<Written>, String> {
        let mut out = Vec::new();
        for chunk in slugs.chunks(CHUNK) {
            let (sql, args) = in_list(WRITTEN, chunk);
            let rows: Vec<WrittenRec> = self.all(&sql, &args).await?;
            out.extend(rows.into_iter().map(|r| Written {
                slug: r.slug,
                published_at: r.published_at,
                revised_at: r.revised_at,
            }));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_mark_per_slug() {
        assert!(marked(WRITTEN, 2).contains("slug IN (?1, ?2) AND"));
        assert!(marked(CHECKS_BY_SLUG, 90).contains("?90) AND p.content_type"));
        assert!(CHUNK < 100);
    }
}
