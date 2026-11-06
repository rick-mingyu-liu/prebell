//! `EventStore` on D1. Column names follow `migrations/0008_macro_events.sql`.
//! Read failures are returned as `Err`, never as no rows.

use async_trait::async_trait;
use prebell_core::events::store::{EventRow, EventStore};
use serde::Deserialize;

use crate::store::{err, int, opt, s, D1Store};

const COLS: &str = "id, series, ref_period, variant, starts_at, et_date, tier, status, proof_url, \
     release_url, moved_from, missing_count, first_seen_at, checked_at, updated_at";

/// Rows per D1 batch in `write_events` (each batch is one call).
const WRITE_BATCH: usize = 50;

#[derive(Deserialize)]
struct EventRec {
    id: String,
    series: String,
    ref_period: Option<String>,
    variant: Option<String>,
    starts_at: Option<String>,
    et_date: String,
    tier: i64,
    status: String,
    proof_url: String,
    release_url: Option<String>,
    moved_from: Option<String>,
    missing_count: i64,
    first_seen_at: String,
    checked_at: String,
    updated_at: String,
}

impl From<EventRec> for EventRow {
    fn from(r: EventRec) -> Self {
        EventRow {
            id: r.id,
            series: r.series,
            ref_period: r.ref_period,
            variant: r.variant,
            starts_at: r.starts_at,
            et_date: r.et_date,
            tier: r.tier,
            status: r.status,
            proof_url: r.proof_url,
            release_url: r.release_url,
            moved_from: r.moved_from,
            missing_count: r.missing_count,
            first_seen_at: r.first_seen_at,
            checked_at: r.checked_at,
            updated_at: r.updated_at,
        }
    }
}

fn upsert_sql() -> String {
    format!(
        "INSERT INTO macro_events ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
         ON CONFLICT(id) DO UPDATE SET series = excluded.series, ref_period = excluded.ref_period, \
         variant = excluded.variant, starts_at = excluded.starts_at, et_date = excluded.et_date, \
         tier = excluded.tier, status = excluded.status, proof_url = excluded.proof_url, \
         release_url = excluded.release_url, moved_from = excluded.moved_from, \
         missing_count = excluded.missing_count, checked_at = excluded.checked_at, \
         updated_at = excluded.updated_at"
    )
}

#[async_trait(?Send)]
impl EventStore for D1Store {
    async fn events_between(&self, from: &str, to: &str) -> Result<Vec<EventRow>, String> {
        let sql = format!(
            "SELECT {COLS} FROM macro_events WHERE et_date BETWEEN ?1 AND ?2 \
             ORDER BY et_date, starts_at, id"
        );
        let rows: Vec<EventRec> = self.all(&sql, &[s(from), s(to)]).await?;
        Ok(rows.into_iter().map(EventRow::from).collect())
    }

    async fn write_events(&self, rows: &[EventRow]) -> Result<(), String> {
        let ctx = "write_events";
        let sql = upsert_sql();
        for chunk in rows.chunks(WRITE_BATCH) {
            let mut stmts = Vec::with_capacity(chunk.len());
            for r in chunk {
                let args = [
                    s(&r.id),
                    s(&r.series),
                    opt(r.ref_period.as_deref()),
                    opt(r.variant.as_deref()),
                    opt(r.starts_at.as_deref()),
                    s(&r.et_date),
                    int(r.tier),
                    s(&r.status),
                    s(&r.proof_url),
                    opt(r.release_url.as_deref()),
                    opt(r.moved_from.as_deref()),
                    int(r.missing_count),
                    s(&r.first_seen_at),
                    s(&r.checked_at),
                    s(&r.updated_at),
                ];
                stmts.push(self.db.prepare(&sql).bind(&args).map_err(|e| err(ctx, e))?);
            }
            self.query();
            let results = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
            if let Some(r) = results.iter().find(|r| !r.success()) {
                return Err(format!("{ctx}: {}", r.error().unwrap_or_default()));
            }
        }
        Ok(())
    }
}
