//! `InsightStore` on D1 and R2, for the `insight` job (`insight::step`). It
//! writes only R2 keys under `insights/` (the step names them) and the kv
//! rows `insight_calls:*`. Read failures are returned as `Err`, never as no
//! rows.

use async_trait::async_trait;
use prebell_core::insight::step::{CheckRef, InsightStore, PostRef};
use prebell_core::store::Store;
use serde::Deserialize;

use crate::store::{int, s, D1Store};

/// Published briefs and recaps from a trading date on (`idx_posts_trading_date`).
pub(crate) const INSIGHT_POSTS: &str = "SELECT content_type, trading_date, slug, facts_r2_key \
     FROM posts WHERE content_type IN ('premarket', 'recap') AND trading_date >= ?1 \
     AND published_at IS NOT NULL ORDER BY trading_date DESC, id DESC";

/// The movers' calendar rows in a date range, quarantined ones left out.
pub(crate) const CALENDAR_ROWS: &str = "SELECT ticker, report_date, status FROM earnings_calendar \
     WHERE ticker IN (SELECT value FROM json_each(?1)) AND report_date BETWEEN ?2 AND ?3 \
     AND status <> 'quarantined'";

/// The movers' published earnings checks filed in a date range
/// (`idx_earnings_filings_ticker`).
pub(crate) const EARNINGS_CHECKS: &str =
    "SELECT f.ticker, f.filing_date, f.accepted_at, p.facts_r2_key \
     FROM earnings_filings f JOIN posts p ON p.run_key = f.run_key \
     WHERE p.content_type = 'earnings' AND p.published_at IS NOT NULL \
     AND f.ticker IN (SELECT value FROM json_each(?1)) AND f.filing_date BETWEEN ?2 AND ?3";

/// One more call when the count is below the cap, in one statement, so two
/// messages at once never both take the last call. No row back: at the cap.
pub(crate) const KV_CLAIM: &str = "INSERT INTO kv (key, value, updated_at) VALUES (?1, '1', ?3) \
     ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(kv.value AS INTEGER) + 1 AS TEXT), \
     updated_at = excluded.updated_at WHERE CAST(kv.value AS INTEGER) < ?2 RETURNING value";

#[derive(Deserialize)]
struct PostRec {
    content_type: String,
    trading_date: String,
    slug: String,
    facts_r2_key: String,
}

#[derive(Deserialize)]
struct CalendarRec {
    ticker: String,
    report_date: String,
    status: String,
}

#[derive(Deserialize)]
struct CheckRec {
    ticker: String,
    filing_date: String,
    accepted_at: String,
    facts_r2_key: String,
}

#[derive(Deserialize)]
struct ValueRec {
    value: String,
}

fn tickers_json(tickers: &[String]) -> String {
    serde_json::to_string(tickers).unwrap_or_else(|_| "[]".into())
}

#[async_trait(?Send)]
impl InsightStore for D1Store {
    async fn insight_posts(&self, since: &str) -> Result<Vec<PostRef>, String> {
        let rows: Vec<PostRec> = self.all(INSIGHT_POSTS, &[s(since)]).await?;
        Ok(rows
            .into_iter()
            .map(|r| PostRef {
                content_type: r.content_type,
                trading_date: r.trading_date,
                slug: r.slug,
                facts_key: r.facts_r2_key,
            })
            .collect())
    }

    async fn get_text(&self, key: &str) -> Result<Option<String>, String> {
        Store::get_facts(self, key).await
    }

    async fn put_text(&self, key: &str, text: &str) -> Result<(), String> {
        Store::put_bytes(self, key, text.as_bytes(), "application/json").await
    }

    async fn calendar_rows(
        &self,
        tickers: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<(String, String, String)>, String> {
        let rows: Vec<CalendarRec> = self
            .all(CALENDAR_ROWS, &[s(&tickers_json(tickers)), s(from), s(to)])
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.ticker, r.report_date, r.status))
            .collect())
    }

    async fn earnings_checks(
        &self,
        tickers: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<CheckRef>, String> {
        let rows: Vec<CheckRec> = self
            .all(
                EARNINGS_CHECKS,
                &[s(&tickers_json(tickers)), s(from), s(to)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| CheckRef {
                ticker: r.ticker,
                filing_date: r.filing_date,
                accepted_at: r.accepted_at,
                facts_key: r.facts_r2_key,
            })
            .collect())
    }

    async fn kv_get(&self, key: &str) -> Result<Option<String>, String> {
        let row: Option<ValueRec> = self
            .first("SELECT value FROM kv WHERE key = ?1", &[s(key)])
            .await?;
        Ok(row.map(|r| r.value))
    }

    async fn kv_claim(&self, key: &str, cap: u32, now: &str) -> Result<Option<u32>, String> {
        let row: Option<ValueRec> = self
            .first(KV_CLAIM, &[s(key), int(i64::from(cap)), s(now)])
            .await?;
        row.map(|r| r.value.parse().map_err(|e| format!("{KV_CLAIM}: {e}")))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The statements are checked against the migrations in SQLite by
    /// hand (see the PR); these pin what they read and write.
    #[test]
    fn the_queries_read_only_published_rows_of_their_kind() {
        assert!(INSIGHT_POSTS.contains("content_type IN ('premarket', 'recap')"));
        assert!(INSIGHT_POSTS.contains("published_at IS NOT NULL"));
        assert!(EARNINGS_CHECKS.contains("p.content_type = 'earnings'"));
        assert!(EARNINGS_CHECKS.contains("p.published_at IS NOT NULL"));
        assert!(CALENDAR_ROWS.contains("status <> 'quarantined'"));
        for q in [INSIGHT_POSTS, CALENDAR_ROWS, EARNINGS_CHECKS] {
            assert!(q.starts_with("SELECT "), "{q}");
        }
    }

    #[test]
    fn the_claim_counts_only_below_the_cap() {
        assert!(
            KV_CLAIM.starts_with("INSERT INTO kv (key, value, updated_at) VALUES (?1, '1', ?3)")
        );
        assert!(KV_CLAIM.contains("WHERE CAST(kv.value AS INTEGER) < ?2 RETURNING value"));
    }

    #[test]
    fn tickers_go_as_one_json_array() {
        assert_eq!(
            tickers_json(&["AMD".into(), "BRK-B".into()]),
            r#"["AMD","BRK-B"]"#
        );
    }
}
