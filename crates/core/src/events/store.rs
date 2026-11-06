//! Persistence for the macro events calendar: `macro_events` rows
//! (`migrations/0008_macro_events.sql`, column for column). The generator
//! implements this on D1; `MemEventStore` serves tests.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use async_trait::async_trait;

/// `macro_events` row. `status`: scheduled | confirmed | moved | postponed |
/// released. Times are UTC ISO 8601 with `Z`; dates ET `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub id: String,
    pub series: String,
    pub ref_period: Option<String>,
    pub variant: Option<String>,
    pub starts_at: Option<String>,
    pub et_date: String,
    pub tier: i64,
    pub status: String,
    pub proof_url: String,
    pub release_url: Option<String>,
    pub moved_from: Option<String>,
    pub missing_count: i64,
    pub first_seen_at: String,
    pub checked_at: String,
    pub updated_at: String,
}

#[async_trait(?Send)]
pub trait EventStore {
    /// Rows with `from <= et_date <= to`, by date then start time (D1: one query).
    async fn events_between(&self, from: &str, to: &str) -> Result<Vec<EventRow>, String>;
    /// Inserts or replaces each row by id (D1: one batch per 50 rows).
    async fn write_events(&self, rows: &[EventRow]) -> Result<(), String>;
}

/// In-memory `EventStore` for tests.
#[derive(Default)]
pub struct MemEventStore {
    pub rows: RefCell<BTreeMap<String, EventRow>>,
    /// Calls made (each one D1 query or batch in the generator).
    pub calls: Cell<usize>,
    /// When set, every call fails with this message (a missing table).
    pub fail: Option<String>,
}

#[async_trait(?Send)]
impl EventStore for MemEventStore {
    async fn events_between(&self, from: &str, to: &str) -> Result<Vec<EventRow>, String> {
        self.calls.set(self.calls.get() + 1);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        let mut out: Vec<EventRow> = self
            .rows
            .borrow()
            .values()
            .filter(|r| r.et_date.as_str() >= from && r.et_date.as_str() <= to)
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            (&a.et_date, &a.starts_at, &a.id).cmp(&(&b.et_date, &b.starts_at, &b.id))
        });
        Ok(out)
    }
    async fn write_events(&self, rows: &[EventRow]) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        let mut m = self.rows.borrow_mut();
        for r in rows {
            m.insert(r.id.clone(), r.clone());
        }
        Ok(())
    }
}
