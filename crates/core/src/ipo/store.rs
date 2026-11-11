//! Persistence for IPO watch: the five tables of
//! `migrations/0009_ipo_watch.sql`, column for column. Every job reads the
//! whole (small) state once and writes its changes as one batch, so its D1
//! use is two queries whatever the number of IPOs. The generator implements
//! this on D1; `MemIpoStore` serves tests.

use std::cell::{Cell, RefCell};

use async_trait::async_trait;

/// `ipo_watch` row (see the contract for each column).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WatchRow {
    pub slug: String,
    pub cik: i64,
    pub name: String,
    pub name_zh: Option<String>,
    pub status: String,
    pub exchange: Option<String>,
    pub symbol: Option<String>,
    pub facts_accession: Option<String>,
    pub first_trade_date: Option<String>,
    pub date_status: Option<String>,
    pub date_source: Option<String>,
    pub date_source_name: Option<String>,
    pub date_source_url: Option<String>,
    pub date_quote: Option<String>,
    pub date_as_of: Option<String>,
    pub checked_at: String,
    pub updated_at: String,
}

/// `ipo_filings` row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FilingRow {
    pub accession: String,
    pub slug: String,
    pub form: String,
    pub filing_date: String,
    pub index_url: String,
    pub doc_url: Option<String>,
    /// none | pending | read | failed
    pub read_state: String,
    pub attempts: i64,
    pub first_seen_at: String,
}

/// `ipo_facts` row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FactRow {
    pub accession: String,
    pub kind: String,
    pub slug: String,
    pub value_num: Option<f64>,
    pub value_text: Option<String>,
    pub quote: String,
}

/// `ipo_items` row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemRow {
    pub slug: String,
    pub kind: String,
    pub url: String,
    pub outlet: String,
    pub published_on: String,
    pub quote: String,
    pub statement_en: Option<String>,
    pub statement_zh: Option<String>,
    pub speaker: Option<String>,
    pub value_usd: Option<f64>,
    pub value_date: Option<String>,
    pub found_at: String,
}

impl ItemRow {
    pub fn key(&self) -> (String, String, String) {
        (self.slug.clone(), self.kind.clone(), self.url.clone())
    }
}

/// `ipo_trading` row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TradingRow {
    pub slug: String,
    pub session_date: String,
    pub symbol: String,
    pub open: Option<f64>,
    pub close: f64,
    pub volume: Option<i64>,
    pub ipo_price: Option<f64>,
    pub change_pct: Option<f64>,
    pub source_url: String,
    pub fetched_at: String,
}

/// Everything stored, as one read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IpoState {
    pub watch: Vec<WatchRow>,
    pub filings: Vec<FilingRow>,
    pub facts: Vec<FactRow>,
    pub items: Vec<ItemRow>,
    pub trading: Vec<TradingRow>,
}

impl IpoState {
    pub fn row(&self, slug: &str) -> Option<&WatchRow> {
        self.watch.iter().find(|r| r.slug == slug)
    }
    pub fn filings_of(&self, slug: &str) -> Vec<&FilingRow> {
        self.filings.iter().filter(|f| f.slug == slug).collect()
    }
    pub fn facts_of(&self, accession: &str) -> Vec<&FactRow> {
        self.facts
            .iter()
            .filter(|f| f.accession == accession)
            .collect()
    }
    pub fn items_of(&self, slug: &str) -> Vec<&ItemRow> {
        self.items.iter().filter(|i| i.slug == slug).collect()
    }
    pub fn trading_of(&self, slug: &str) -> Vec<&TradingRow> {
        self.trading.iter().filter(|t| t.slug == slug).collect()
    }

    /// The state after `w` is written (as D1 would leave it).
    pub fn apply(&mut self, w: &IpoWrite) {
        for r in &w.watch {
            upsert(&mut self.watch, r, |x| x.slug == r.slug);
        }
        for r in &w.filings {
            upsert(&mut self.filings, r, |x| x.accession == r.accession);
        }
        for r in &w.facts {
            upsert(&mut self.facts, r, |x| {
                x.accession == r.accession && x.kind == r.kind
            });
        }
        for r in &w.items {
            upsert(&mut self.items, r, |x| x.key() == r.key());
        }
        self.items.retain(|i| !w.drop_items.contains(&i.key()));
        for r in &w.trading {
            upsert(&mut self.trading, r, |x| {
                x.slug == r.slug && x.session_date == r.session_date
            });
        }
    }
}

/// One job's changes, written as one batch: upserts by primary key, and
/// the item keys to delete (the per-kind caps).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IpoWrite {
    pub watch: Vec<WatchRow>,
    pub filings: Vec<FilingRow>,
    pub facts: Vec<FactRow>,
    pub items: Vec<ItemRow>,
    pub drop_items: Vec<(String, String, String)>,
    pub trading: Vec<TradingRow>,
}

impl IpoWrite {
    pub fn is_empty(&self) -> bool {
        self.watch.is_empty()
            && self.filings.is_empty()
            && self.facts.is_empty()
            && self.items.is_empty()
            && self.drop_items.is_empty()
            && self.trading.is_empty()
    }
}

#[async_trait(?Send)]
pub trait IpoStore {
    /// Every row of the five tables (D1: one batch of five reads).
    async fn ipo_state(&self) -> Result<IpoState, String>;
    /// Upserts and deletes, as one batch (D1: one transaction).
    async fn ipo_write(&self, w: &IpoWrite) -> Result<(), String>;
}

/// In-memory `IpoStore` for tests.
#[derive(Default)]
pub struct MemIpoStore {
    pub state: RefCell<IpoState>,
    /// Calls made (each one D1 query or batch in the generator).
    pub calls: Cell<usize>,
    /// When set, every call fails with this message (a missing table).
    pub fail: Option<String>,
}

fn upsert<T: Clone>(v: &mut Vec<T>, row: &T, same: impl Fn(&T) -> bool) {
    match v.iter_mut().find(|r| same(r)) {
        Some(r) => *r = row.clone(),
        None => v.push(row.clone()),
    }
}

#[async_trait(?Send)]
impl IpoStore for MemIpoStore {
    async fn ipo_state(&self) -> Result<IpoState, String> {
        self.calls.set(self.calls.get() + 1);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        Ok(self.state.borrow().clone())
    }
    async fn ipo_write(&self, w: &IpoWrite) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        self.state.borrow_mut().apply(w);
        Ok(())
    }
}
