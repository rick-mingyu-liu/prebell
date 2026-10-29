use crate::facts::Quote;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResp {
    pub status: u16,
    pub body: String,
    /// The response's `Set-Cookie` header, if any. Several are joined by ", ".
    pub set_cookie: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    #[error("http status {0}")]
    Http(u16),
    #[error("network error: {0}")]
    Network(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("empty result")]
    Empty,
    /// The quote's premarket fields disagree with its previous close: likely
    /// left over from an earlier session.
    #[error("stale premarket quote")]
    Stale,
    /// Not requested, because an earlier request in the run was throttled.
    #[error("not requested after HTTP 429")]
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub url: String,
    pub title: String,
    pub content: String,
}

#[async_trait(?Send)]
pub trait Http {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResp, ProviderError>;
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<HttpResp, ProviderError>;
}

#[async_trait(?Send)]
pub trait MarketData {
    async fn quote(&self, symbol: &str) -> Result<Quote, ProviderError>;
    /// Premarket quotes for the ET trading date of `now`.
    async fn premarket_quotes(
        &self,
        symbols: &[String],
        now: DateTime<Utc>,
    ) -> Vec<(String, Result<Quote, ProviderError>)>;
    /// Short provider-specific diagnostic lines about the last calls.
    fn diagnostics(&self) -> Vec<String> {
        Vec::new()
    }
}

#[async_trait(?Send)]
pub trait NewsSearch {
    async fn search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>, ProviderError>;
}

#[async_trait(?Send)]
pub trait LlmProvider {
    async fn complete_json(
        &self,
        system: &str,
        user: &str,
    ) -> Result<serde_json::Value, ProviderError>;
}
