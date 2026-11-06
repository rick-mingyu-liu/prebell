//! `EarningsStore` on D1. Column names follow `migrations/0003_earnings.sql`.
//! Read failures are returned as `Err`, never as a missing row.

use async_trait::async_trait;
use prebell_core::earnings::calendar::Change;
use std::collections::HashSet;

use prebell_core::earnings::store::{CalendarRow, ChartRow, EarningsStore, FilingRow};
use prebell_core::store::RunRow;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use worker::wasm_bindgen::JsValue;

use crate::store::{err, int, opt, s, D1Store};

#[derive(Deserialize)]
struct FilingRec {
    accession: String,
    cik: i64,
    ticker: String,
    form: String,
    items: String,
    filing_date: String,
    report_date: String,
    accepted_at: String,
    primary_doc: String,
    run_key: String,
    state: String,
    detected_at: String,
}

impl From<FilingRec> for FilingRow {
    fn from(r: FilingRec) -> Self {
        FilingRow {
            accession: r.accession,
            cik: u64::try_from(r.cik).unwrap_or_default(),
            ticker: r.ticker,
            form: r.form,
            items: r.items,
            filing_date: r.filing_date,
            report_date: r.report_date,
            accepted_at: r.accepted_at,
            primary_doc: r.primary_doc,
            run_key: r.run_key,
            state: r.state,
            detected_at: r.detected_at,
        }
    }
}

#[derive(Deserialize)]
struct CalRec {
    ticker: String,
    report_date: String,
    time_slot: String,
    status: String,
    source: String,
    proof_url: Option<String>,
    proof_quote: Option<String>,
    conflict_reason: Option<String>,
    first_seen_at: String,
    checked_at: String,
    next_check_at: Option<String>,
    updated_at: String,
}

impl From<CalRec> for CalendarRow {
    fn from(r: CalRec) -> Self {
        CalendarRow {
            ticker: r.ticker,
            report_date: r.report_date,
            time_slot: r.time_slot,
            status: r.status,
            source: r.source,
            proof_url: r.proof_url,
            proof_quote: r.proof_quote,
            conflict_reason: r.conflict_reason,
            first_seen_at: r.first_seen_at,
            checked_at: r.checked_at,
            next_check_at: r.next_check_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Deserialize)]
struct Key {
    accession: String,
}
#[derive(Deserialize)]
struct RunKey {
    run_key: String,
}
#[derive(Deserialize)]
struct Value {
    value: String,
}

const FILING_COLS: &str = "accession, cik, ticker, form, items, filing_date, report_date, \
    accepted_at, primary_doc, run_key, state, detected_at";
const CAL_COLS: &str = "ticker, report_date, time_slot, status, source, proof_url, proof_quote, \
    conflict_reason, first_seen_at, checked_at, next_check_at, updated_at";

const DELETE_CALENDAR: &str =
    "DELETE FROM earnings_calendar WHERE ticker = ?1 AND report_date = ?2";
/// Statements per D1 batch in `write_calendar` (each batch is one call).
const WRITE_BATCH: usize = 100;

fn upsert_calendar_sql() -> String {
    format!(
        "INSERT INTO earnings_calendar ({CAL_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
         ON CONFLICT(ticker, report_date) DO UPDATE SET time_slot = excluded.time_slot, \
         status = excluded.status, source = excluded.source, proof_url = excluded.proof_url, \
         proof_quote = excluded.proof_quote, conflict_reason = excluded.conflict_reason, \
         first_seen_at = excluded.first_seen_at, checked_at = excluded.checked_at, \
         next_check_at = excluded.next_check_at, updated_at = excluded.updated_at"
    )
}

fn filing_args(f: &FilingRow) -> Result<Vec<JsValue>, String> {
    Ok(vec![
        s(&f.accession),
        int(i64::try_from(f.cik).map_err(|e| e.to_string())?),
        s(&f.ticker),
        s(&f.form),
        s(&f.items),
        s(&f.filing_date),
        s(&f.report_date),
        s(&f.accepted_at),
        s(&f.primary_doc),
        s(&f.run_key),
        s(&f.state),
        s(&f.detected_at),
    ])
}

/// An open filing and its run's columns (`r_*`, null without a run). Spelled
/// out rather than `#[serde(flatten)]`, which reads D1's numbers as floats.
#[derive(Deserialize)]
struct OpenRec {
    accession: String,
    cik: i64,
    ticker: String,
    form: String,
    items: String,
    filing_date: String,
    report_date: String,
    accepted_at: String,
    primary_doc: String,
    run_key: String,
    state: String,
    detected_at: String,
    r_content_type: Option<String>,
    r_trading_date: Option<String>,
    r_status: Option<String>,
    r_attempts: Option<i64>,
    r_last_error: Option<String>,
    r_created_at: Option<String>,
    r_updated_at: Option<String>,
}

impl OpenRec {
    fn split(self) -> (FilingRow, Option<RunRow>) {
        let run = match (self.r_status, self.r_created_at, self.r_updated_at) {
            (Some(status), Some(created_at), Some(updated_at)) => Some(RunRow {
                run_key: self.run_key.clone(),
                content_type: self.r_content_type.unwrap_or_default(),
                trading_date: self.r_trading_date.unwrap_or_default(),
                status,
                attempts: self.r_attempts.unwrap_or_default(),
                last_error: self.r_last_error,
                created_at,
                updated_at,
            }),
            _ => None,
        };
        let f = FilingRow::from(FilingRec {
            accession: self.accession,
            cik: self.cik,
            ticker: self.ticker,
            form: self.form,
            items: self.items,
            filing_date: self.filing_date,
            report_date: self.report_date,
            accepted_at: self.accepted_at,
            primary_doc: self.primary_doc,
            run_key: self.run_key,
            state: self.state,
            detected_at: self.detected_at,
        });
        (f, run)
    }
}

/// Bound values per D1 statement are limited (100), so `IN (...)` lists are cut to this.
const IN_CHUNK: usize = 90;
/// Filing + run pairs per batch in `insert_new_filings`.
const INSERT_BATCH: usize = 50;

fn calendar_args(r: &CalendarRow) -> Vec<JsValue> {
    vec![
        s(&r.ticker),
        s(&r.report_date),
        s(&r.time_slot),
        s(&r.status),
        s(&r.source),
        opt(r.proof_url.as_deref()),
        opt(r.proof_quote.as_deref()),
        opt(r.conflict_reason.as_deref()),
        s(&r.first_seen_at),
        s(&r.checked_at),
        opt(r.next_check_at.as_deref()),
        s(&r.updated_at),
    ]
}

impl D1Store {
    pub(crate) async fn all<T: DeserializeOwned>(
        &self,
        sql: &str,
        args: &[JsValue],
    ) -> Result<Vec<T>, String> {
        self.query();
        let stmt = self.db.prepare(sql).bind(args).map_err(|e| err(sql, e))?;
        let r = stmt.all().await.map_err(|e| err(sql, e))?;
        r.results::<T>().map_err(|e| err(sql, e))
    }
}

#[async_trait(?Send)]
impl EarningsStore for D1Store {
    async fn insert_filing(&self, f: &FilingRow) -> Result<bool, String> {
        let sql = format!(
            "INSERT INTO earnings_filings ({FILING_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(accession) DO NOTHING RETURNING accession"
        );
        let r: Option<Key> = self.first(&sql, &filing_args(f)?).await?;
        Ok(r.is_some_and(|k| k.accession == f.accession))
    }

    async fn known_accessions(&self, accessions: &[String]) -> Result<HashSet<String>, String> {
        let mut out = HashSet::new();
        for chunk in accessions.chunks(IN_CHUNK) {
            let marks: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "SELECT accession FROM earnings_filings WHERE accession IN ({})",
                marks.join(", ")
            );
            let args: Vec<JsValue> = chunk.iter().map(|a| s(a)).collect();
            let rows: Vec<Key> = self.all(&sql, &args).await?;
            out.extend(rows.into_iter().map(|k| k.accession));
        }
        Ok(out)
    }

    async fn insert_new_filings(&self, new: &[(FilingRow, RunRow)]) -> Result<(), String> {
        let ctx = "insert_new_filings";
        let filing_sql = format!(
            "INSERT INTO earnings_filings ({FILING_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(accession) DO NOTHING"
        );
        // DO NOTHING: a build that already took the run is never reset to pending.
        let run_sql = "INSERT INTO runs (run_key, content_type, trading_date, status, attempts, last_error, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(run_key) DO NOTHING";
        for chunk in new.chunks(INSERT_BATCH) {
            let mut stmts = Vec::with_capacity(2 * chunk.len());
            for (f, r) in chunk {
                stmts.push(
                    self.db
                        .prepare(&filing_sql)
                        .bind(&filing_args(f)?)
                        .map_err(|e| err(ctx, e))?,
                );
                stmts.push(
                    self.db
                        .prepare(run_sql)
                        .bind(&[
                            s(&r.run_key),
                            s(&r.content_type),
                            s(&r.trading_date),
                            s(&r.status),
                            int(r.attempts),
                            opt(r.last_error.as_deref()),
                            s(&r.created_at),
                            s(&r.updated_at),
                        ])
                        .map_err(|e| err(ctx, e))?,
                );
            }
            self.query();
            let results = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
            if let Some(r) = results.iter().find(|r| !r.success()) {
                return Err(format!("{ctx}: {}", r.error().unwrap_or_default()));
            }
        }
        Ok(())
    }

    async fn open_runs(&self) -> Result<Vec<(FilingRow, Option<RunRow>)>, String> {
        let cols: Vec<String> = FILING_COLS
            .split(',')
            .map(|c| format!("f.{}", c.trim()))
            .collect();
        let sql = format!(
            "SELECT {}, r.content_type AS r_content_type, r.trading_date AS r_trading_date, \
             r.status AS r_status, r.attempts AS r_attempts, r.last_error AS r_last_error, \
             r.created_at AS r_created_at, r.updated_at AS r_updated_at \
             FROM earnings_filings f LEFT JOIN runs r ON r.run_key = f.run_key \
             WHERE f.state = 'open' ORDER BY f.accepted_at",
            cols.join(", ")
        );
        let rows: Vec<OpenRec> = self.all(&sql, &[]).await?;
        Ok(rows.into_iter().map(OpenRec::split).collect())
    }

    async fn open_filings(&self) -> Result<Vec<FilingRow>, String> {
        let sql = format!(
            "SELECT {FILING_COLS} FROM earnings_filings WHERE state = 'open' ORDER BY accepted_at"
        );
        let rows: Vec<FilingRec> = self.all(&sql, &[]).await?;
        Ok(rows.into_iter().map(FilingRow::from).collect())
    }

    async fn set_filing_state(&self, accession: &str, state: &str) -> Result<(), String> {
        self.run(
            "UPDATE earnings_filings SET state = ?2 WHERE accession = ?1",
            &[s(accession), s(state)],
        )
        .await
    }

    async fn slug_owner(&self, slug: &str) -> Result<Option<String>, String> {
        let r: Option<RunKey> = self
            .first("SELECT run_key FROM posts WHERE slug = ?1", &[s(slug)])
            .await?;
        Ok(r.map(|r| r.run_key))
    }

    async fn set_charts(&self, post_id: i64, charts: &[ChartRow]) -> Result<(), String> {
        let ctx = "set_charts";
        let mut stmts = vec![self
            .db
            .prepare("DELETE FROM post_charts WHERE post_id = ?1")
            .bind(&[int(post_id)])
            .map_err(|e| err(ctx, e))?];
        for c in charts {
            stmts.push(
                self.db
                    .prepare(
                        "INSERT INTO post_charts (post_id, name, r2_key, alt_en, alt_zh, sort) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )
                    .bind(&[
                        int(post_id),
                        s(&c.name),
                        s(&c.r2_key),
                        s(&c.alt_en),
                        s(&c.alt_zh),
                        int(c.sort),
                    ])
                    .map_err(|e| err(ctx, e))?,
            );
        }
        // One batch = one transaction: the post never shows half its charts.
        self.query();
        let results = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
        match results.iter().find(|r| !r.success()) {
            Some(r) => Err(format!("{ctx}: {}", r.error().unwrap_or_default())),
            None => Ok(()),
        }
    }

    async fn calendar_for(&self, ticker: &str) -> Result<Vec<CalendarRow>, String> {
        let sql = format!(
            "SELECT {CAL_COLS} FROM earnings_calendar WHERE ticker = ?1 ORDER BY report_date"
        );
        let rows: Vec<CalRec> = self.all(&sql, &[s(ticker)]).await?;
        Ok(rows.into_iter().map(CalendarRow::from).collect())
    }

    async fn calendar_between(&self, from: &str, to: &str) -> Result<Vec<CalendarRow>, String> {
        let sql = format!(
            "SELECT {CAL_COLS} FROM earnings_calendar WHERE report_date BETWEEN ?1 AND ?2 \
             ORDER BY report_date, ticker"
        );
        let rows: Vec<CalRec> = self.all(&sql, &[s(from), s(to)]).await?;
        Ok(rows.into_iter().map(CalendarRow::from).collect())
    }

    async fn upsert_calendar(&self, r: &CalendarRow) -> Result<(), String> {
        self.run(&upsert_calendar_sql(), &calendar_args(r)).await
    }

    async fn delete_calendar(&self, ticker: &str, report_date: &str) -> Result<(), String> {
        self.run(DELETE_CALENDAR, &[s(ticker), s(report_date)])
            .await
    }

    async fn write_calendar(&self, changes: &[Change]) -> Result<(), String> {
        let ctx = "write_calendar";
        let upsert = upsert_calendar_sql();
        for chunk in changes.chunks(WRITE_BATCH) {
            let mut stmts = Vec::with_capacity(chunk.len());
            for c in chunk {
                let (sql, args) = match c {
                    Change::Upsert(r) => (upsert.as_str(), calendar_args(r)),
                    Change::Delete { ticker, date } => (DELETE_CALENDAR, vec![s(ticker), s(date)]),
                };
                stmts.push(self.db.prepare(sql).bind(&args).map_err(|e| err(ctx, e))?);
            }
            self.query();
            let results = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
            if let Some(r) = results.iter().find(|r| !r.success()) {
                return Err(format!("{ctx}: {}", r.error().unwrap_or_default()));
            }
        }
        Ok(())
    }

    async fn kv_get(&self, key: &str) -> Result<Option<String>, String> {
        let r: Option<Value> = self
            .first("SELECT value FROM kv WHERE key = ?1", &[s(key)])
            .await?;
        Ok(r.map(|v| v.value))
    }

    async fn kv_set(&self, key: &str, value: &str, now: &str) -> Result<(), String> {
        self.kv_upsert(key, value, now).await
    }
}
