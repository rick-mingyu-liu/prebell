//! Persistence for earnings beyond Plan 1's `Store`: detected filings (the
//! build queue), post charts, the earnings calendar and small generator
//! state. Rows mirror `migrations/0003_earnings.sql` column for column.
//! The generator implements this on D1; `MemEarningsStore` serves tests.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use async_trait::async_trait;

use crate::earnings::calendar::Change;
use crate::store::{MemStore, RunRow};

/// `earnings_filings` row. `state`: open | done.
#[derive(Debug, Clone, PartialEq)]
pub struct FilingRow {
    pub accession: String,
    pub cik: u64,
    pub ticker: String,
    pub form: String,
    pub items: String,
    pub filing_date: String,
    pub report_date: String,
    /// UTC ISO 8601 with `Z`.
    pub accepted_at: String,
    pub primary_doc: String,
    pub run_key: String,
    pub state: String,
    pub detected_at: String,
}

/// `post_charts` row.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartRow {
    pub post_id: i64,
    pub name: String,
    pub r2_key: String,
    pub alt_en: String,
    pub alt_zh: String,
    pub sort: i64,
}

/// `earnings_calendar` row. `time_slot`: bmo | amc | unknown; `status`:
/// unconfirmed | confirmed | reported | quarantined; `source`: yahoo | ir | edgar.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarRow {
    pub ticker: String,
    /// ET `YYYY-MM-DD`.
    pub report_date: String,
    pub time_slot: String,
    pub status: String,
    pub source: String,
    pub proof_url: Option<String>,
    pub proof_quote: Option<String>,
    pub conflict_reason: Option<String>,
    pub first_seen_at: String,
    pub checked_at: String,
    pub next_check_at: Option<String>,
    pub updated_at: String,
}

#[async_trait(?Send)]
pub trait EarningsStore {
    /// Inserts unless the accession exists; true when a row was inserted.
    async fn insert_filing(&self, f: &FilingRow) -> Result<bool, String>;
    /// Which of these accessions are already recorded (D1: one query per 100).
    async fn known_accessions(&self, accessions: &[String]) -> Result<HashSet<String>, String>;
    /// Inserts each filing and its new run, in one call (D1: one batch per 50
    /// pairs). An existing accession or run is left as it is
    /// (`ON CONFLICT DO NOTHING`), so a build that already took the run is
    /// never reset to `pending`.
    async fn insert_new_filings(&self, new: &[(FilingRow, RunRow)]) -> Result<(), String>;
    /// `state = 'open'`, oldest acceptance first.
    async fn open_filings(&self) -> Result<Vec<FilingRow>, String>;
    /// The open filings with their runs (none yet for a filing whose run was
    /// never written), oldest acceptance first, in one query.
    async fn open_runs(&self) -> Result<Vec<(FilingRow, Option<RunRow>)>, String>;
    async fn set_filing_state(&self, accession: &str, state: &str) -> Result<(), String>;
    /// The run that owns the post with this slug, if any.
    async fn slug_owner(&self, slug: &str) -> Result<Option<String>, String>;
    /// Replaces the post's charts.
    async fn set_charts(&self, post_id: i64, charts: &[ChartRow]) -> Result<(), String>;
    async fn calendar_for(&self, ticker: &str) -> Result<Vec<CalendarRow>, String>;
    /// Rows with `from <= report_date <= to`, by date then ticker.
    async fn calendar_between(&self, from: &str, to: &str) -> Result<Vec<CalendarRow>, String>;
    /// Insert or replace by `(ticker, report_date)`.
    async fn upsert_calendar(&self, r: &CalendarRow) -> Result<(), String>;
    async fn delete_calendar(&self, ticker: &str, report_date: &str) -> Result<(), String>;
    /// Applies the changes in order, in as few calls as the store allows (D1:
    /// one batch per 100 statements).
    async fn write_calendar(&self, changes: &[Change]) -> Result<(), String>;
    async fn kv_get(&self, key: &str) -> Result<Option<String>, String>;
    async fn kv_set(&self, key: &str, value: &str, now: &str) -> Result<(), String>;
}

/// In-memory `EarningsStore` for tests; reads post slugs from the shared `MemStore`.
#[derive(Default)]
pub struct MemEarningsStore {
    pub base: Rc<MemStore>,
    pub filings: RefCell<BTreeMap<String, FilingRow>>,
    pub charts: RefCell<BTreeMap<i64, Vec<ChartRow>>>,
    pub calendar: RefCell<BTreeMap<(String, String), CalendarRow>>,
    pub kv: RefCell<BTreeMap<String, String>>,
    /// Calls made (each one D1 query in the generator; `open_runs` counts as
    /// its one `open_filings`).
    pub calls: Cell<usize>,
    /// `kv_get` of this key fails, as a D1 error would.
    pub fail_kv: RefCell<Option<String>>,
    /// `kv_set` of this key fails, as a D1 error would.
    pub fail_kv_set: RefCell<Option<String>>,
}

impl MemEarningsStore {
    pub fn new(base: Rc<MemStore>) -> Self {
        Self {
            base,
            ..Default::default()
        }
    }
}

#[async_trait(?Send)]
impl EarningsStore for MemEarningsStore {
    async fn insert_filing(&self, f: &FilingRow) -> Result<bool, String> {
        self.calls.set(self.calls.get() + 1);
        let mut m = self.filings.borrow_mut();
        if m.contains_key(&f.accession) {
            return Ok(false);
        }
        m.insert(f.accession.clone(), f.clone());
        Ok(true)
    }
    async fn known_accessions(&self, accessions: &[String]) -> Result<HashSet<String>, String> {
        self.calls.set(self.calls.get() + 1);
        let m = self.filings.borrow();
        Ok(accessions
            .iter()
            .filter(|a| m.contains_key(*a))
            .cloned()
            .collect())
    }
    async fn insert_new_filings(&self, new: &[(FilingRow, RunRow)]) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        let mut m = self.filings.borrow_mut();
        let mut runs = self.base.runs.borrow_mut();
        for (f, r) in new {
            m.entry(f.accession.clone()).or_insert_with(|| f.clone());
            runs.entry(r.run_key.clone()).or_insert_with(|| r.clone());
        }
        Ok(())
    }
    async fn open_runs(&self) -> Result<Vec<(FilingRow, Option<RunRow>)>, String> {
        let open = self.open_filings().await?;
        let runs = self.base.runs.borrow();
        Ok(open
            .into_iter()
            .map(|f| {
                let r = runs.get(&f.run_key).cloned();
                (f, r)
            })
            .collect())
    }
    async fn open_filings(&self) -> Result<Vec<FilingRow>, String> {
        self.calls.set(self.calls.get() + 1);
        let mut v: Vec<FilingRow> = self
            .filings
            .borrow()
            .values()
            .filter(|f| f.state == "open")
            .cloned()
            .collect();
        v.sort_by(|a, b| a.accepted_at.cmp(&b.accepted_at));
        Ok(v)
    }
    async fn set_filing_state(&self, accession: &str, state: &str) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        let mut m = self.filings.borrow_mut();
        let f = m.get_mut(accession).ok_or("no such filing")?;
        f.state = state.into();
        Ok(())
    }
    async fn slug_owner(&self, slug: &str) -> Result<Option<String>, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(self
            .base
            .posts
            .borrow()
            .iter()
            .find(|p| p.slug == slug)
            .map(|p| p.run_key.clone()))
    }
    async fn set_charts(&self, post_id: i64, charts: &[ChartRow]) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        self.charts.borrow_mut().insert(post_id, charts.to_vec());
        Ok(())
    }
    async fn calendar_for(&self, ticker: &str) -> Result<Vec<CalendarRow>, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(self
            .calendar
            .borrow()
            .values()
            .filter(|r| r.ticker == ticker)
            .cloned()
            .collect())
    }
    async fn calendar_between(&self, from: &str, to: &str) -> Result<Vec<CalendarRow>, String> {
        self.calls.set(self.calls.get() + 1);
        let mut v: Vec<CalendarRow> = self
            .calendar
            .borrow()
            .values()
            .filter(|r| r.report_date.as_str() >= from && r.report_date.as_str() <= to)
            .cloned()
            .collect();
        v.sort_by(|a, b| (&a.report_date, &a.ticker).cmp(&(&b.report_date, &b.ticker)));
        Ok(v)
    }
    async fn upsert_calendar(&self, r: &CalendarRow) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        self.calendar
            .borrow_mut()
            .insert((r.ticker.clone(), r.report_date.clone()), r.clone());
        Ok(())
    }
    async fn delete_calendar(&self, ticker: &str, report_date: &str) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        self.calendar
            .borrow_mut()
            .remove(&(ticker.to_string(), report_date.to_string()));
        Ok(())
    }
    async fn write_calendar(&self, changes: &[Change]) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        let mut cal = self.calendar.borrow_mut();
        for c in changes {
            match c {
                Change::Upsert(r) => {
                    cal.insert((r.ticker.clone(), r.report_date.clone()), (**r).clone());
                }
                Change::Delete { ticker, date } => {
                    cal.remove(&(ticker.clone(), date.clone()));
                }
            }
        }
        Ok(())
    }
    async fn kv_get(&self, key: &str) -> Result<Option<String>, String> {
        self.calls.set(self.calls.get() + 1);
        if self.fail_kv.borrow().as_deref() == Some(key) {
            return Err(format!("d1: kv {key} failed"));
        }
        Ok(self.kv.borrow().get(key).cloned())
    }
    async fn kv_set(&self, key: &str, value: &str, _now: &str) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        if self.fail_kv_set.borrow().as_deref() == Some(key) {
            return Err(format!("d1: kv {key} failed"));
        }
        self.kv.borrow_mut().insert(key.into(), value.into());
        Ok(())
    }
}
