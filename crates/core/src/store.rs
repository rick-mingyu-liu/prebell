//! Persistence boundary for the run state machine. Rows mirror
//! `migrations/0001_init.sql` column for column; status strings use the
//! schema's allowed values. Task 11 implements `Store` on D1/R2.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};

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
    /// R2 key of the rendered cover image, once one exists.
    pub cover_key: Option<String>,
    /// UTC ISO 8601 with `Z`: when `revise_post` last rewrote the post.
    pub revised_at: Option<String>,
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
    /// The two text lines drawn on the cover image.
    pub cover_line1: Option<String>,
    pub cover_line2: Option<String>,
}

#[async_trait(?Send)]
pub trait Store {
    /// Reads return `Err` on a store failure and `Ok(None)` only when the row is absent.
    async fn get_run(&self, key: &str) -> Result<Option<RunRow>, String>;
    /// Insert or replace by `run_key`.
    async fn upsert_run(&self, r: &RunRow) -> Result<(), String>;
    async fn get_post_by_run(&self, run_key: &str) -> Result<Option<PostRow>, String>;
    /// Published posts of `content_type` for the ET `trading_date`, by id.
    async fn posts_on(
        &self,
        content_type: &str,
        trading_date: &str,
    ) -> Result<Vec<PostRow>, String>;
    /// Insert or update by `run_key`; returns the existing id when the run
    /// already has a post, so re-runs never create a second post. An update
    /// keeps the stored `revised_at`: only `revise_post` changes it.
    async fn upsert_post(&self, p: &PostRow) -> Result<i64, String>;
    /// Rewrites the post `p.id` in one transaction: its `facts_r2_key`,
    /// `cover_key` and `revised_at` (every other column, `published_at`
    /// included, is kept), the translations `t` (insert or replace by
    /// `(post_id, lang)`) and its sources (replaced by `s`). On `Err`
    /// nothing was written.
    async fn revise_post(
        &self,
        p: &PostRow,
        t: &[TranslationRow],
        s: &[NewsReason],
    ) -> Result<(), String>;
    async fn get_translation(
        &self,
        post_id: i64,
        lang: &str,
    ) -> Result<Option<TranslationRow>, String>;
    /// Insert or replace by `(post_id, lang)`.
    async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String>;
    /// Replaces the post's sources with `s` (idempotent per post).
    async fn add_sources(&self, post_id: i64, s: &[NewsReason]) -> Result<(), String>;
    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String>;
    /// Whether any alert, sent or not, has `run_key`.
    async fn has_alert(&self, run_key: &str) -> Result<bool, String>;
    /// Stores the facts JSON (R2 in production) under `key`.
    async fn put_facts(&self, key: &str, json: &str) -> Result<(), String>;
    async fn get_facts(&self, key: &str) -> Result<Option<String>, String>;
    /// Stores binary content (an image in R2 in production) under `key`.
    async fn put_bytes(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), String>;
    /// Insert or replace the kv row `key`; `now` is its `updated_at`.
    async fn kv_set(&self, key: &str, value: &str, now: &str) -> Result<(), String>;
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
    /// `key -> (bytes, content type)` written by `put_bytes`.
    pub bytes: RefCell<BTreeMap<String, (Vec<u8>, String)>>,
    pub kv: RefCell<BTreeMap<String, String>>,
    /// Makes `posts_on` fail, as a D1 outage would.
    pub fail_posts_on: Cell<bool>,
    /// Makes `get_facts` fail for this key, as an R2 outage would.
    pub fail_facts_key: RefCell<Option<String>>,
    /// Makes `put_bytes` fail and store nothing, as an R2 outage would.
    pub fail_put_bytes: Cell<bool>,
    /// Makes `revise_post` fail and write nothing, as a failed D1 batch would.
    pub fail_revise_post: Cell<bool>,
}

#[async_trait(?Send)]
impl Store for MemStore {
    async fn get_run(&self, key: &str) -> Result<Option<RunRow>, String> {
        Ok(self.runs.borrow().get(key).cloned())
    }
    async fn upsert_run(&self, r: &RunRow) -> Result<(), String> {
        self.runs.borrow_mut().insert(r.run_key.clone(), r.clone());
        Ok(())
    }
    async fn get_post_by_run(&self, run_key: &str) -> Result<Option<PostRow>, String> {
        Ok(self
            .posts
            .borrow()
            .iter()
            .find(|p| p.run_key == run_key)
            .cloned())
    }
    async fn posts_on(
        &self,
        content_type: &str,
        trading_date: &str,
    ) -> Result<Vec<PostRow>, String> {
        if self.fail_posts_on.get() {
            return Err("d1: posts_on failed".into());
        }
        let mut out: Vec<PostRow> = self
            .posts
            .borrow()
            .iter()
            .filter(|p| {
                p.content_type == content_type
                    && p.trading_date == trading_date
                    && p.published_at.is_some()
            })
            .cloned()
            .collect();
        out.sort_by_key(|p| p.id);
        Ok(out)
    }
    async fn upsert_post(&self, p: &PostRow) -> Result<i64, String> {
        let mut posts = self.posts.borrow_mut();
        if let Some(existing) = posts.iter_mut().find(|e| e.run_key == p.run_key) {
            let id = existing.id;
            let revised_at = existing.revised_at.take();
            *existing = PostRow {
                id,
                revised_at,
                ..p.clone()
            };
            return id.ok_or_else(|| "stored post has no id".to_string());
        }
        let id = posts.len() as i64 + 1;
        posts.push(PostRow {
            id: Some(id),
            ..p.clone()
        });
        Ok(id)
    }
    async fn revise_post(
        &self,
        p: &PostRow,
        t: &[TranslationRow],
        s: &[NewsReason],
    ) -> Result<(), String> {
        if self.fail_revise_post.get() {
            return Err("d1: revise_post failed".into());
        }
        let id = p.id.ok_or("revise_post: post has no id")?;
        let mut posts = self.posts.borrow_mut();
        let existing = posts
            .iter_mut()
            .find(|e| e.id == Some(id))
            .ok_or_else(|| format!("revise_post: no post {id}"))?;
        existing.facts_r2_key = p.facts_r2_key.clone();
        existing.cover_key = p.cover_key.clone();
        existing.revised_at = p.revised_at.clone();
        let mut translations = self.translations.borrow_mut();
        for row in t {
            translations.insert((id, row.lang.clone()), row.clone());
        }
        self.sources.borrow_mut().insert(id, s.to_vec());
        Ok(())
    }
    async fn get_translation(
        &self,
        post_id: i64,
        lang: &str,
    ) -> Result<Option<TranslationRow>, String> {
        Ok(self
            .translations
            .borrow()
            .get(&(post_id, lang.to_string()))
            .cloned())
    }
    async fn upsert_translation(&self, t: &TranslationRow) -> Result<(), String> {
        self.translations
            .borrow_mut()
            .insert((t.post_id, t.lang.clone()), t.clone());
        Ok(())
    }
    async fn add_sources(&self, post_id: i64, s: &[NewsReason]) -> Result<(), String> {
        self.sources.borrow_mut().insert(post_id, s.to_vec());
        Ok(())
    }
    async fn add_alert(&self, run_key: &str, level: &str, msg: &str) -> Result<(), String> {
        self.alerts
            .borrow_mut()
            .push((run_key.into(), level.into(), msg.into()));
        Ok(())
    }
    async fn has_alert(&self, run_key: &str) -> Result<bool, String> {
        Ok(self.alerts.borrow().iter().any(|a| a.0 == run_key))
    }
    async fn put_facts(&self, key: &str, json: &str) -> Result<(), String> {
        self.facts.borrow_mut().insert(key.into(), json.into());
        Ok(())
    }
    async fn get_facts(&self, key: &str) -> Result<Option<String>, String> {
        if self.fail_facts_key.borrow().as_deref() == Some(key) {
            return Err(format!("r2: get {key} failed"));
        }
        Ok(self.facts.borrow().get(key).cloned())
    }
    async fn put_bytes(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), String> {
        if self.fail_put_bytes.get() {
            return Err("r2: put_bytes failed".into());
        }
        self.bytes
            .borrow_mut()
            .insert(key.into(), (bytes.to_vec(), content_type.into()));
        Ok(())
    }
    async fn kv_set(&self, key: &str, value: &str, _now: &str) -> Result<(), String> {
        self.kv.borrow_mut().insert(key.into(), value.into());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> PostRow {
        PostRow {
            id: None,
            run_key: "brief:2026-11-04".into(),
            content_type: "brief".into(),
            template_id: "t".into(),
            trading_date: "2026-11-04".into(),
            slug: "s".into(),
            facts_r2_key: "facts/x.json".into(),
            published_at: Some("2026-11-04T12:00:00Z".into()),
            cover_key: Some("covers/x.png".into()),
            revised_at: None,
        }
    }

    #[tokio::test]
    async fn mem_store_round_trips_cover_fields() {
        let s = MemStore::default();
        let id = s.upsert_post(&post()).await.unwrap();
        let got = s
            .get_post_by_run("brief:2026-11-04")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.cover_key.as_deref(), Some("covers/x.png"));

        let t = TranslationRow {
            post_id: id,
            lang: "en".into(),
            status: "published".into(),
            title: "T".into(),
            summary: "S".into(),
            body_md: "B".into(),
            updated_at: "2026-11-04T12:00:00Z".into(),
            cover_line1: Some("Revenue $10.4B".into()),
            cover_line2: None,
        };
        s.upsert_translation(&t).await.unwrap();
        let got = s.get_translation(id, "en").await.unwrap().unwrap();
        assert_eq!(got.cover_line1.as_deref(), Some("Revenue $10.4B"));
        assert_eq!(got.cover_line2, None);
    }

    #[tokio::test]
    async fn mem_store_put_bytes_and_kv_set() {
        let s = MemStore::default();
        s.put_bytes("covers/x.png", &[1, 2, 3], "image/png")
            .await
            .unwrap();
        assert_eq!(
            s.bytes.borrow()["covers/x.png"],
            (vec![1, 2, 3], "image/png".to_string())
        );
        s.kv_set("index_line", "{}", "2026-11-04T00:00:00Z")
            .await
            .unwrap();
        assert_eq!(s.kv.borrow()["index_line"], "{}");
    }

    fn tr(post_id: i64, lang: &str, title: &str) -> TranslationRow {
        TranslationRow {
            post_id,
            lang: lang.into(),
            status: "published".into(),
            title: title.into(),
            summary: "S".into(),
            body_md: "B".into(),
            updated_at: "2026-11-04T12:35:00Z".into(),
            cover_line1: None,
            cover_line2: None,
        }
    }
    fn reason(url: &str) -> NewsReason {
        NewsReason {
            url: url.into(),
            title: "t".into(),
            quote: "q".into(),
            summary_en: "s".into(),
        }
    }

    #[tokio::test]
    async fn mem_store_revise_post_rewrites_in_place_and_upsert_keeps_revised_at() {
        let s = MemStore::default();
        let id = s.upsert_post(&post()).await.unwrap();
        s.upsert_translation(&tr(id, "en", "old")).await.unwrap();
        s.add_sources(id, &[reason("https://a.test")])
            .await
            .unwrap();
        let revised = PostRow {
            id: Some(id),
            slug: "ignored".into(),
            published_at: Some("ignored".into()),
            facts_r2_key: "facts/y.json".into(),
            cover_key: Some("covers/x-update.svg".into()),
            revised_at: Some("2026-11-04T12:35:00Z".into()),
            ..post()
        };
        s.revise_post(
            &revised,
            &[tr(id, "en", "new"), tr(id, "zh", "新")],
            &[reason("https://b.test")],
        )
        .await
        .unwrap();
        let got = s.posts.borrow()[0].clone();
        assert_eq!(
            got,
            PostRow {
                id: Some(id),
                facts_r2_key: "facts/y.json".into(),
                cover_key: Some("covers/x-update.svg".into()),
                revised_at: Some("2026-11-04T12:35:00Z".into()),
                ..post()
            }
        );
        assert_eq!(
            s.get_translation(id, "en").await.unwrap().unwrap().title,
            "new"
        );
        assert_eq!(
            s.get_translation(id, "zh").await.unwrap().unwrap().title,
            "新"
        );
        assert_eq!(s.sources.borrow()[&id], [reason("https://b.test")]);

        // A later upsert by run key keeps the revision time.
        assert_eq!(s.upsert_post(&post()).await.unwrap(), id);
        assert_eq!(
            s.posts.borrow()[0].revised_at.as_deref(),
            Some("2026-11-04T12:35:00Z")
        );

        // A failed rewrite writes nothing.
        s.fail_revise_post.set(true);
        let before = (s.posts.borrow().clone(), s.translations.borrow().clone());
        assert!(s
            .revise_post(&revised, &[tr(id, "en", "x")], &[])
            .await
            .is_err());
        assert_eq!(
            (s.posts.borrow().clone(), s.translations.borrow().clone()),
            before
        );
        assert_eq!(s.sources.borrow()[&id], [reason("https://b.test")]);
    }
}
