//! Persistence boundary for the run state machine. Rows mirror
//! `migrations/0001_init.sql` column for column; status strings use the
//! schema's allowed values. Task 11 implements `Store` on D1/R2.

use std::cell::RefCell;
use std::collections::HashMap;

use async_trait::async_trait;

use crate::facts::NewsReason;

/// `runs` row. `status`: pending | building | published | skipped | failed.
#[derive(Debug, Clone, PartialEq)]
pub struct RunRow {
    pub run_key: String,
    pub content_type: String,
    /// `YYYY-MM-DD` (ET trading date).
    pub trading_date: String,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    /// UTC ISO 8601 with `Z`.
    pub created_at: String,
    pub updated_at: String,
}

/// `posts` row. `id` is `None` until the store assigns one.
#[derive(Debug, Clone, PartialEq)]
pub struct PostRow {
    pub id: Option<i64>,
    pub run_key: String,
    pub content_type: String,
    pub template_id: String,
    pub trading_date: String,
    pub slug: String,
    pub facts_r2_key: String,
    pub published_at: Option<String>,
}

/// `post_translations` row. `lang`: en | zh; `status`: published | pending | failed.
#[derive(Debug, Clone, PartialEq)]
pub struct TranslationRow {
    pub post_id: i64,
    pub lang: String,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub body_md: String,
    pub updated_at: String,
}

#[async_trait(?Send)]
pub trait Store {
    async fn get_run(&self, key: &str) -> Option<RunRow>;
    /// Insert or replace by `run_key`.
    async fn upsert_run(&self, r: &RunRow) -> Result<(), String>;
    async fn get_post_by_run(&self, run_key: &str) -> Option<PostRow>;
    /// Insert or update by `run_key`; returns the existing id when the run
    /// already has a post, so re-runs never create a second post.
    async fn upsert_post(&self, p: &PostRow) -> Result<i64, String>;
    async fn get_translation(&self, post_id: i64, lang: &str) -> Option<TranslationRow>;
    /// Insert or replace by `(post_id, lang)`.
    async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String>;
    /// Replaces the post's sources with `s` (idempotent per post).
    async fn add_sources(&self, post_id: i64, s: &[NewsReason]) -> Result<(), String>;
    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String>;
    /// Stores the facts JSON (R2 in production) under `key`.
    async fn put_facts(&self, key: &str, json: &str) -> Result<(), String>;
    async fn get_facts(&self, key: &str) -> Option<String>;
}

/// In-memory `Store` for tests.
#[derive(Default)]
pub struct MemStore {
    pub runs: RefCell<HashMap<String, RunRow>>,
    pub posts: RefCell<Vec<PostRow>>,
    pub translations: RefCell<HashMap<(i64, String), TranslationRow>>,
    pub sources: RefCell<HashMap<i64, Vec<NewsReason>>>,
    /// `(run_key, level, message)` in insertion order.
    pub alerts: RefCell<Vec<(String, String, String)>>,
    pub facts: RefCell<HashMap<String, String>>,
}

#[async_trait(?Send)]
impl Store for MemStore {
    async fn get_run(&self, key: &str) -> Option<RunRow> {
        self.runs.borrow().get(key).cloned()
    }
    async fn upsert_run(&self, r: &RunRow) -> Result<(), String> {
        self.runs.borrow_mut().insert(r.run_key.clone(), r.clone());
        Ok(())
    }
    async fn get_post_by_run(&self, run_key: &str) -> Option<PostRow> {
        self.posts.borrow().iter().find(|p| p.run_key == run_key).cloned()
    }
    async fn upsert_post(&self, p: &PostRow) -> Result<i64, String> {
        let mut posts = self.posts.borrow_mut();
        if let Some(existing) = posts.iter_mut().find(|e| e.run_key == p.run_key) {
            let id = existing.id;
            *existing = PostRow { id, ..p.clone() };
            return id.ok_or_else(|| "stored post has no id".to_string());
        }
        let id = posts.len() as i64 + 1;
        posts.push(PostRow { id: Some(id), ..p.clone() });
        Ok(id)
    }
    async fn get_translation(&self, post_id: i64, lang: &str) -> Option<TranslationRow> {
        self.translations.borrow().get(&(post_id, lang.to_string())).cloned()
    }
    async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String> {
        self.translations.borrow_mut().insert((t.post_id, t.lang.clone()), t.clone());
        Ok(())
    }
    async fn add_sources(&self, post_id: i64, s: &[NewsReason]) -> Result<(), String> {
        self.sources.borrow_mut().insert(post_id, s.to_vec());
        Ok(())
    }
    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String> {
        self.alerts.borrow_mut().push((run_key.into(), level.into(), msg.into()));
        Ok(())
    }
    async fn put_facts(&self, key: &str, json: &str) -> Result<(), String> {
        self.facts.borrow_mut().insert(key.into(), json.into());
        Ok(())
    }
    async fn get_facts(&self, key: &str) -> Option<String> {
        self.facts.borrow().get(key).cloned()
    }
}
