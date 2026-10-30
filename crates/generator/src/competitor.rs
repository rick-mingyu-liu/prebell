//! `CompetitorStore` on D1. Tables from `migrations/0004_competitor.sql`;
//! our posts come from the Plan 1 tables, read-only.

use async_trait::async_trait;
use prebell_core::competitor::report::{OurPost, Report, StoredItem};
use prebell_core::competitor::{CompetitorStore, NewItem};
use prebell_core::store::Store;
use serde::Deserialize;

use crate::store::{err, opt, s, D1Store};

#[derive(Deserialize)]
struct CountRec {
    n: i64,
}

#[derive(Deserialize)]
struct ItemRec {
    link: String,
    title: String,
    published_at: String,
    category: Option<String>,
    tickers: String,
}

#[derive(Deserialize)]
struct OurRec {
    slug: String,
    title: Option<String>,
    summary: Option<String>,
    body_md: Option<String>,
    source_titles: Option<String>,
}

#[async_trait(?Send)]
impl CompetitorStore for D1Store {
    async fn has_report(&self, report_date: &str) -> Result<bool, String> {
        let r: Option<CountRec> = self
            .first(
                "SELECT COUNT(*) AS n FROM competitor_reports WHERE report_date = ?1",
                &[s(report_date)],
            )
            .await?;
        Ok(r.is_some_and(|r| r.n > 0))
    }

    async fn upsert_items(&self, items: &[NewItem], seen_at: &str) -> Result<(), String> {
        if items.is_empty() {
            return Ok(());
        }
        let ctx = "upsert_items";
        let sql = "INSERT INTO competitor_items (link, title, published_at, category, tickers, first_seen_at, last_seen_at) \
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
                   ON CONFLICT(link) DO UPDATE SET title = excluded.title, published_at = excluded.published_at, \
                   category = excluded.category, tickers = excluded.tickers, last_seen_at = excluded.last_seen_at";
        let stmts = items
            .iter()
            .map(|i| {
                self.db
                    .prepare(sql)
                    .bind(&[
                        s(&i.link),
                        s(&i.title),
                        opt(i.published_at.as_deref()),
                        s(&i.category),
                        s(&i.tickers.join(" ")),
                        s(seen_at),
                    ])
                    .map_err(|e| err(ctx, e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let results = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
        match results.iter().find(|r| !r.success()) {
            Some(r) => Err(format!("{ctx}: {}", r.error().unwrap_or_default())),
            None => Ok(()),
        }
    }

    async fn items_between(&self, start: &str, end: &str) -> Result<Vec<StoredItem>, String> {
        let sql = "SELECT link, title, published_at, category, tickers FROM competitor_items \
                   WHERE published_at >= ?1 AND published_at < ?2 ORDER BY published_at DESC";
        let rows = self
            .db
            .prepare(sql)
            .bind(&[s(start), s(end)])
            .map_err(|e| err(sql, e))?
            .all()
            .await
            .map_err(|e| err(sql, e))?
            .results::<ItemRec>()
            .map_err(|e| err(sql, e))?;
        Ok(rows
            .into_iter()
            .map(|r| StoredItem {
                link: r.link,
                title: r.title,
                published_at: r.published_at,
                category: r.category.unwrap_or_default(),
                tickers: r.tickers.split_whitespace().map(String::from).collect(),
            })
            .collect())
    }

    async fn our_posts_between(&self, start: &str, end: &str) -> Result<Vec<OurPost>, String> {
        let sql = "SELECT p.slug, t.title, t.summary, t.body_md, \
                   (SELECT group_concat(COALESCE(src.title, ''), char(10)) FROM sources src WHERE src.post_id = p.id) AS source_titles \
                   FROM posts p JOIN post_translations t ON t.post_id = p.id AND t.lang = 'en' \
                   WHERE t.status = 'published' AND p.published_at >= ?1 AND p.published_at < ?2 \
                   ORDER BY p.published_at";
        let rows = self
            .db
            .prepare(sql)
            .bind(&[s(start), s(end)])
            .map_err(|e| err(sql, e))?
            .all()
            .await
            .map_err(|e| err(sql, e))?
            .results::<OurRec>()
            .map_err(|e| err(sql, e))?;
        Ok(rows
            .into_iter()
            .map(|r| OurPost {
                slug: r.slug,
                text: [r.title, r.summary, r.body_md, r.source_titles]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect())
    }

    async fn put_report(&self, r: &Report, digest: &str, created_at: &str) -> Result<(), String> {
        let json = serde_json::to_string(r).map_err(|e| format!("report json: {e}"))?;
        self.run(
            "INSERT INTO competitor_reports (report_date, window_start, window_end, report_json, digest, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(report_date) DO UPDATE SET window_start = excluded.window_start, \
             window_end = excluded.window_end, report_json = excluded.report_json, \
             digest = excluded.digest, created_at = excluded.created_at",
            &[
                s(&r.report_date),
                s(&r.window_start),
                s(&r.window_end),
                s(&json),
                s(digest),
                s(created_at),
            ],
        )
        .await
    }

    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String> {
        Store::add_alert(self, run_key, level, msg).await
    }
}
