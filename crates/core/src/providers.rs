use crate::facts::Quote;
use async_trait::async_trait;

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
    async fn premarket_quotes(
        &self,
        symbols: &[String],
    ) -> Vec<(String, Result<Quote, ProviderError>)>;
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
