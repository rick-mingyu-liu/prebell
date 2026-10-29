//! The earnings calendar: upcoming report dates seeded from an aggregator,
//! confirmed only by primary proof (the company's own announcement, or the
//! 8-K itself), with conflicting dates quarantined.

use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};

use crate::clock::et;
use crate::earnings::store::{CalendarRow, EarningsStore, FilingRow};
use crate::edgar::filing_url;

/// Calendar rows of one ticker this close together describe the same report.
pub const SAME_EVENT_DAYS: i64 = 21;

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// An 8-K Item 2.02 is the strongest proof: its ET acceptance date becomes a
/// `reported` row, replacing any other row of the same event.
pub async fn record_filing(
    es: &dyn EarningsStore,
    f: &FilingRow,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let day = DateTime::parse_from_rfc3339(&f.accepted_at)
        .map(|t| et(t.with_timezone(&Utc)).date_naive())
        .map_err(|e| format!("bad accepted_at: {e}"))?;
    let existing = es.calendar_for(&f.ticker).await?;
    let mut first_seen = iso(now);
    let mut slot = "unknown".to_string();
    for r in &existing {
        let Some(d) = date(&r.report_date) else {
            continue;
        };
        if (d - day).num_days().abs() > SAME_EVENT_DAYS {
            continue;
        }
        if d == day {
            first_seen = r.first_seen_at.clone();
            slot = r.time_slot.clone();
        } else if r.status != "reported" {
            es.delete_calendar(&r.ticker, &r.report_date).await?;
        }
    }
    let ts = iso(now);
    es.upsert_calendar(&CalendarRow {
        ticker: f.ticker.clone(),
        report_date: day.format("%Y-%m-%d").to_string(),
        time_slot: slot,
        status: "reported".into(),
        source: "edgar".into(),
        proof_url: Some(filing_url(f.cik, &f.accession)),
        proof_quote: None,
        conflict_reason: None,
        first_seen_at: first_seen,
        checked_at: ts.clone(),
        next_check_at: None,
        updated_at: ts,
    })
    .await?;
    let _ = Duration::zero();
    Ok(())
}
