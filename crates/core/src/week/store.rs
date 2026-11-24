//! What the week post reads beyond `Store`, `EarningsStore` and
//! `EventStore`: its runs, the earlier week posts and the earnings checks.
//! The Worker's is `D1Store` (SQL in `crates/generator/src/store.rs`, checked
//! against the real schema in `apps/web/tests/weekquery.test.ts`).

use async_trait::async_trait;

use crate::store::{MemStore, PostRow, RunRow};

/// A `week:*` run, and whether its post's English text is published.
#[derive(Debug, Clone, PartialEq)]
pub struct WeekRun {
    pub run: RunRow,
    pub live: bool,
}

/// An earnings check as the week post lists it: the post, its two
/// translations' cover lines as stored, and the Chinese status.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRow {
    pub slug: String,
    /// `earnings:<TICKER>:<accession>`.
    pub run_key: String,
    pub trading_date: String,
    pub published_at: String,
    pub revised_at: Option<String>,
    pub en: [Option<String>; 2],
    pub zh: [Option<String>; 2],
    /// published | pending | failed, or `None` without a Chinese row.
    pub zh_status: Option<String>,
}

impl CheckRow {
    /// The ticker in the run key.
    pub fn ticker(&self) -> Option<&str> {
        let mut parts = self.run_key.splitn(3, ':');
        (parts.next() == Some("earnings"))
            .then(|| parts.next())
            .flatten()
    }
}

/// When a post was last written (`published_at`, `revised_at`).
#[derive(Debug, Clone, PartialEq)]
pub struct Written {
    pub slug: String,
    pub published_at: Option<String>,
    pub revised_at: Option<String>,
}

#[async_trait(?Send)]
pub trait WeekStore {
    /// One read: the `week:*` runs that are `pending` or `building`, and the
    /// run `this` whatever its status, each with whether its post is live.
    async fn week_runs(&self, this: &str) -> Result<Vec<WeekRun>, String>;
    /// Up to `limit` published week posts (English text published) with a
    /// `trading_date` before `friday`, newest first.
    async fn week_posts(&self, friday: &str, limit: usize) -> Result<Vec<PostRow>, String>;
    /// Earnings checks whose English text is published, with
    /// `trading_date >= since` and `published_at <= cutoff`, by date then slug.
    async fn week_checks(&self, since: &str, cutoff: &str) -> Result<Vec<CheckRow>, String>;
    /// The same, for these slugs whatever their dates.
    async fn checks_by_slug(&self, slugs: &[String]) -> Result<Vec<CheckRow>, String>;
    /// When each of these earnings checks was last written.
    async fn written(&self, slugs: &[String]) -> Result<Vec<Written>, String>;
}

impl MemStore {
    fn check_row(&self, p: &PostRow) -> Option<CheckRow> {
        let id = p.id?;
        let t = self.translations.borrow();
        let en = t.get(&(id, "en".to_string()))?;
        if en.status != "published" || p.content_type != "earnings" {
            return None;
        }
        let zh = t.get(&(id, "zh".to_string()));
        Some(CheckRow {
            slug: p.slug.clone(),
            run_key: p.run_key.clone(),
            trading_date: p.trading_date.clone(),
            published_at: p.published_at.clone()?,
            revised_at: p.revised_at.clone(),
            en: [en.cover_line1.clone(), en.cover_line2.clone()],
            zh: [
                zh.and_then(|z| z.cover_line1.clone()),
                zh.and_then(|z| z.cover_line2.clone()),
            ],
            zh_status: zh.map(|z| z.status.clone()),
        })
    }

    fn live(&self, run_key: &str) -> bool {
        let posts = self.posts.borrow();
        let Some(id) = posts
            .iter()
            .find(|p| p.run_key == run_key)
            .and_then(|p| p.id)
        else {
            return false;
        };
        self.translations
            .borrow()
            .get(&(id, "en".to_string()))
            .is_some_and(|t| t.status == "published")
    }
}

#[async_trait(?Send)]
impl WeekStore for MemStore {
    async fn week_runs(&self, this: &str) -> Result<Vec<WeekRun>, String> {
        let mut runs: Vec<RunRow> = self
            .runs
            .borrow()
            .values()
            .filter(|r| r.run_key.starts_with("week:"))
            .filter(|r| matches!(r.status.as_str(), "pending" | "building") || r.run_key == this)
            .cloned()
            .collect();
        runs.sort_by(|a, b| a.run_key.cmp(&b.run_key));
        Ok(runs
            .into_iter()
            .map(|run| WeekRun {
                live: self.live(&run.run_key),
                run,
            })
            .collect())
    }

    async fn week_posts(&self, friday: &str, limit: usize) -> Result<Vec<PostRow>, String> {
        let mut out: Vec<PostRow> = self
            .posts
            .borrow()
            .iter()
            .filter(|p| p.content_type == super::CONTENT_TYPE)
            .filter(|p| p.trading_date.as_str() < friday && p.published_at.is_some())
            .filter(|p| self.live(&p.run_key))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.trading_date.cmp(&a.trading_date));
        out.truncate(limit);
        Ok(out)
    }

    async fn week_checks(&self, since: &str, cutoff: &str) -> Result<Vec<CheckRow>, String> {
        let mut out: Vec<CheckRow> = self
            .posts
            .borrow()
            .iter()
            .filter(|p| p.trading_date.as_str() >= since)
            .filter(|p| p.published_at.as_deref().is_some_and(|t| t <= cutoff))
            .filter_map(|p| self.check_row(p))
            .collect();
        out.sort_by(|a, b| (&a.trading_date, &a.slug).cmp(&(&b.trading_date, &b.slug)));
        Ok(out)
    }

    async fn checks_by_slug(&self, slugs: &[String]) -> Result<Vec<CheckRow>, String> {
        let mut out: Vec<CheckRow> = self
            .posts
            .borrow()
            .iter()
            .filter(|p| slugs.contains(&p.slug))
            .filter_map(|p| self.check_row(p))
            .collect();
        out.sort_by(|a, b| (&a.trading_date, &a.slug).cmp(&(&b.trading_date, &b.slug)));
        Ok(out)
    }

    async fn written(&self, slugs: &[String]) -> Result<Vec<Written>, String> {
        Ok(self
            .posts
            .borrow()
            .iter()
            .filter(|p| p.content_type == "earnings" && slugs.contains(&p.slug))
            .map(|p| Written {
                slug: p.slug.clone(),
                published_at: p.published_at.clone(),
                revised_at: p.revised_at.clone(),
            })
            .collect())
    }
}
