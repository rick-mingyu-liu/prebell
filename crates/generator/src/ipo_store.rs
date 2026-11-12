//! `IpoStore` on D1. Column names follow `migrations/0009_ipo_watch.sql`.
//! The state is one batch of five reads and a write is one batch (one
//! transaction). Read failures are returned as `Err`, never as no rows.

use async_trait::async_trait;
use prebell_core::ipo::store::{
    FactRow, FilingRow, IpoState, IpoStore, IpoWrite, ItemRow, TradingRow, WatchRow,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use worker::wasm_bindgen::JsValue;

use crate::store::{err, int, opt, s, D1Store};

fn num(v: Option<f64>) -> JsValue {
    v.map_or(JsValue::NULL, JsValue::from_f64)
}

#[derive(Deserialize)]
struct WatchRec {
    slug: String,
    cik: i64,
    name: String,
    name_zh: Option<String>,
    status: String,
    exchange: Option<String>,
    symbol: Option<String>,
    facts_accession: Option<String>,
    first_trade_date: Option<String>,
    date_status: Option<String>,
    date_source: Option<String>,
    date_source_name: Option<String>,
    date_source_url: Option<String>,
    date_quote: Option<String>,
    date_as_of: Option<String>,
    checked_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct FilingRec {
    accession: String,
    slug: String,
    form: String,
    filing_date: String,
    index_url: String,
    doc_url: Option<String>,
    read_state: String,
    attempts: i64,
    first_seen_at: String,
}

#[derive(Deserialize)]
struct FactRec {
    accession: String,
    kind: String,
    slug: String,
    value_num: Option<f64>,
    value_text: Option<String>,
    quote: String,
}

#[derive(Deserialize)]
struct ItemRec {
    slug: String,
    kind: String,
    url: String,
    outlet: String,
    published_on: String,
    quote: String,
    statement_en: Option<String>,
    statement_zh: Option<String>,
    speaker: Option<String>,
    value_usd: Option<f64>,
    value_date: Option<String>,
    found_at: String,
}

#[derive(Deserialize)]
struct TradingRec {
    slug: String,
    session_date: String,
    symbol: String,
    open: Option<f64>,
    close: f64,
    volume: Option<f64>,
    ipo_price: Option<f64>,
    change_pct: Option<f64>,
    source_url: String,
    fetched_at: String,
}

const WATCH_COLS: &str = "slug, cik, name, name_zh, status, exchange, symbol, facts_accession, \
     first_trade_date, date_status, date_source, date_source_name, date_source_url, date_quote, \
     date_as_of, checked_at, updated_at";
const FILING_COLS: &str =
    "accession, slug, form, filing_date, index_url, doc_url, read_state, attempts, first_seen_at";
const FACT_COLS: &str = "accession, kind, slug, value_num, value_text, quote";
const ITEM_COLS: &str =
    "slug, kind, url, outlet, published_on, quote, statement_en, statement_zh, \
     speaker, value_usd, value_date, found_at";
const TRADING_COLS: &str = "slug, session_date, symbol, open, close, volume, ipo_price, \
     change_pct, source_url, fetched_at";

/// `INSERT ... ON CONFLICT(pk) DO UPDATE SET` every other column.
fn upsert(table: &str, cols: &str, pk: &[&str]) -> String {
    let names: Vec<&str> = cols.split(',').map(str::trim).collect();
    let marks: Vec<String> = (1..=names.len()).map(|i| format!("?{i}")).collect();
    let set: Vec<String> = names
        .iter()
        .filter(|c| !pk.contains(c))
        .map(|c| format!("{c} = excluded.{c}"))
        .collect();
    format!(
        "INSERT INTO {table} ({cols}) VALUES ({}) ON CONFLICT({}) DO UPDATE SET {}",
        marks.join(", "),
        pk.join(", "),
        set.join(", ")
    )
}

fn rows<T: DeserializeOwned>(r: &worker::D1Result, ctx: &str) -> Result<Vec<T>, String> {
    r.results::<T>().map_err(|e| err(ctx, e))
}

#[async_trait(?Send)]
impl IpoStore for D1Store {
    async fn ipo_state(&self) -> Result<IpoState, String> {
        let ctx = "ipo_state";
        let sql = [
            format!("SELECT {WATCH_COLS} FROM ipo_watch"),
            format!("SELECT {FILING_COLS} FROM ipo_filings"),
            format!("SELECT {FACT_COLS} FROM ipo_facts"),
            format!("SELECT {ITEM_COLS} FROM ipo_items"),
            format!("SELECT {TRADING_COLS} FROM ipo_trading"),
        ];
        let stmts = sql.iter().map(|q| self.db.prepare(q)).collect();
        self.query();
        let r = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
        if let Some(bad) = r.iter().find(|x| !x.success()) {
            return Err(format!("{ctx}: {}", bad.error().unwrap_or_default()));
        }
        if r.len() != 5 {
            return Err(format!("{ctx}: {} results", r.len()));
        }
        Ok(IpoState {
            watch: rows::<WatchRec>(&r[0], ctx)?
                .into_iter()
                .map(|w| WatchRow {
                    slug: w.slug,
                    cik: w.cik,
                    name: w.name,
                    name_zh: w.name_zh,
                    status: w.status,
                    exchange: w.exchange,
                    symbol: w.symbol,
                    facts_accession: w.facts_accession,
                    first_trade_date: w.first_trade_date,
                    date_status: w.date_status,
                    date_source: w.date_source,
                    date_source_name: w.date_source_name,
                    date_source_url: w.date_source_url,
                    date_quote: w.date_quote,
                    date_as_of: w.date_as_of,
                    checked_at: w.checked_at,
                    updated_at: w.updated_at,
                })
                .collect(),
            filings: rows::<FilingRec>(&r[1], ctx)?
                .into_iter()
                .map(|f| FilingRow {
                    accession: f.accession,
                    slug: f.slug,
                    form: f.form,
                    filing_date: f.filing_date,
                    index_url: f.index_url,
                    doc_url: f.doc_url,
                    read_state: f.read_state,
                    attempts: f.attempts,
                    first_seen_at: f.first_seen_at,
                })
                .collect(),
            facts: rows::<FactRec>(&r[2], ctx)?
                .into_iter()
                .map(|f| FactRow {
                    accession: f.accession,
                    kind: f.kind,
                    slug: f.slug,
                    value_num: f.value_num,
                    value_text: f.value_text,
                    quote: f.quote,
                })
                .collect(),
            items: rows::<ItemRec>(&r[3], ctx)?
                .into_iter()
                .map(|i| ItemRow {
                    slug: i.slug,
                    kind: i.kind,
                    url: i.url,
                    outlet: i.outlet,
                    published_on: i.published_on,
                    quote: i.quote,
                    statement_en: i.statement_en,
                    statement_zh: i.statement_zh,
                    speaker: i.speaker,
                    value_usd: i.value_usd,
                    value_date: i.value_date,
                    found_at: i.found_at,
                })
                .collect(),
            trading: rows::<TradingRec>(&r[4], ctx)?
                .into_iter()
                .map(|t| TradingRow {
                    slug: t.slug,
                    session_date: t.session_date,
                    symbol: t.symbol,
                    open: t.open,
                    close: t.close,
                    volume: t.volume.map(|v| v as i64),
                    ipo_price: t.ipo_price,
                    change_pct: t.change_pct,
                    source_url: t.source_url,
                    fetched_at: t.fetched_at,
                })
                .collect(),
        })
    }

    async fn ipo_write(&self, w: &IpoWrite) -> Result<(), String> {
        let ctx = "ipo_write";
        if w.is_empty() {
            return Ok(());
        }
        let mut stmts = Vec::new();
        let mut add = |sql: &str, args: &[JsValue]| -> Result<(), String> {
            stmts.push(self.db.prepare(sql).bind(args).map_err(|e| err(ctx, e))?);
            Ok(())
        };
        let sql = upsert("ipo_filings", FILING_COLS, &["accession"]);
        for f in &w.filings {
            add(
                &sql,
                &[
                    s(&f.accession),
                    s(&f.slug),
                    s(&f.form),
                    s(&f.filing_date),
                    s(&f.index_url),
                    opt(f.doc_url.as_deref()),
                    s(&f.read_state),
                    int(f.attempts),
                    s(&f.first_seen_at),
                ],
            )?;
        }
        let sql = upsert("ipo_facts", FACT_COLS, &["accession", "kind"]);
        for f in &w.facts {
            add(
                &sql,
                &[
                    s(&f.accession),
                    s(&f.kind),
                    s(&f.slug),
                    num(f.value_num),
                    opt(f.value_text.as_deref()),
                    s(&f.quote),
                ],
            )?;
        }
        let sql = upsert("ipo_items", ITEM_COLS, &["slug", "kind", "url"]);
        for i in &w.items {
            add(
                &sql,
                &[
                    s(&i.slug),
                    s(&i.kind),
                    s(&i.url),
                    s(&i.outlet),
                    s(&i.published_on),
                    s(&i.quote),
                    opt(i.statement_en.as_deref()),
                    opt(i.statement_zh.as_deref()),
                    opt(i.speaker.as_deref()),
                    num(i.value_usd),
                    opt(i.value_date.as_deref()),
                    s(&i.found_at),
                ],
            )?;
        }
        for (slug, kind, url) in &w.drop_items {
            add(
                "DELETE FROM ipo_items WHERE slug = ?1 AND kind = ?2 AND url = ?3",
                &[s(slug), s(kind), s(url)],
            )?;
        }
        let sql = upsert("ipo_trading", TRADING_COLS, &["slug", "session_date"]);
        for t in &w.trading {
            add(
                &sql,
                &[
                    s(&t.slug),
                    s(&t.session_date),
                    s(&t.symbol),
                    num(t.open),
                    JsValue::from_f64(t.close),
                    t.volume.map_or(JsValue::NULL, int),
                    num(t.ipo_price),
                    num(t.change_pct),
                    s(&t.source_url),
                    s(&t.fetched_at),
                ],
            )?;
        }
        let sql = upsert("ipo_watch", WATCH_COLS, &["slug"]);
        for r in &w.watch {
            add(
                &sql,
                &[
                    s(&r.slug),
                    int(r.cik),
                    s(&r.name),
                    opt(r.name_zh.as_deref()),
                    s(&r.status),
                    opt(r.exchange.as_deref()),
                    opt(r.symbol.as_deref()),
                    opt(r.facts_accession.as_deref()),
                    opt(r.first_trade_date.as_deref()),
                    opt(r.date_status.as_deref()),
                    opt(r.date_source.as_deref()),
                    opt(r.date_source_name.as_deref()),
                    opt(r.date_source_url.as_deref()),
                    opt(r.date_quote.as_deref()),
                    opt(r.date_as_of.as_deref()),
                    s(&r.checked_at),
                    s(&r.updated_at),
                ],
            )?;
        }
        self.query();
        let results = self.db.batch(stmts).await.map_err(|e| err(ctx, e))?;
        if let Some(r) = results.iter().find(|r| !r.success()) {
            return Err(format!("{ctx}: {}", r.error().unwrap_or_default()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upserts_set_every_column_but_the_key() {
        assert_eq!(
            upsert("ipo_facts", FACT_COLS, &["accession", "kind"]),
            "INSERT INTO ipo_facts (accession, kind, slug, value_num, value_text, quote) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(accession, kind) DO UPDATE SET \
             slug = excluded.slug, value_num = excluded.value_num, \
             value_text = excluded.value_text, quote = excluded.quote"
        );
    }

    #[test]
    fn the_columns_are_the_migrations() {
        const SQL: &str = include_str!("../../../migrations/0009_ipo_watch.sql");
        for (table, cols) in [
            ("ipo_watch", WATCH_COLS),
            ("ipo_filings", FILING_COLS),
            ("ipo_facts", FACT_COLS),
            ("ipo_items", ITEM_COLS),
            ("ipo_trading", TRADING_COLS),
        ] {
            let body = SQL
                .split(&format!("CREATE TABLE {table} ("))
                .nth(1)
                .and_then(|b| b.split(");").next())
                .unwrap_or_else(|| panic!("{table}"));
            let declared: Vec<&str> = body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("--") && !l.starts_with("PRIMARY KEY"))
                .filter_map(|l| l.split_whitespace().next())
                // Column names are lowercase; CHECK lines and their
                // continuations ("OR ...") are not columns.
                .filter(|w| w.starts_with(|c: char| c.is_ascii_lowercase()))
                .collect();
            let mine: Vec<&str> = cols.split(',').map(str::trim).collect();
            assert_eq!(mine, declared, "{table}");
        }
    }
}
