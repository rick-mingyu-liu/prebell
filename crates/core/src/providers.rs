use crate::facts::Quote;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};

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
    /// The response body passed the caller's byte cap.
    #[error("response too large: over {0} bytes")]
    TooLarge(usize),
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
    /// `get` that fails with `TooLarge` when the body is over `max_bytes`.
    /// Real clients should stop reading at the cap; this default checks
    /// only after the whole body is in memory.
    async fn get_capped(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<HttpResp, ProviderError> {
        let resp = self.get(url, headers).await?;
        if resp.body.len() > max_bytes {
            return Err(ProviderError::TooLarge(max_bytes));
        }
        Ok(resp)
    }
}

/// Waits, so a caller can put a deadline on a request (`worker::Delay` on Workers).
#[async_trait(?Send)]
pub trait Timer {
    async fn sleep(&self, d: std::time::Duration);
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
    /// Regular-session close for the ET date `session`: `last` is the close,
    /// `prev_close` the prior session's close, `volume` the session volume.
    /// A quote from any other session is an error, never today's close.
    /// The default reports the method as unsupported (`Empty`), so providers
    /// without close data compile and their recap fields read as missing.
    async fn close_quote(
        &self,
        _symbol: &str,
        _session: NaiveDate,
    ) -> Result<Quote, ProviderError> {
        Err(ProviderError::Empty)
    }
    /// Closes for many symbols, in order. The default calls `close_quote`
    /// for each; a provider may batch (Yahoo uses v7 quotes).
    async fn close_quotes(
        &self,
        symbols: &[String],
        session: NaiveDate,
    ) -> Vec<(String, Result<Quote, ProviderError>)> {
        let mut out = Vec::with_capacity(symbols.len());
        for s in symbols {
            out.push((s.clone(), self.close_quote(s, session).await));
        }
        out
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider with only the Plan 1 methods.
    struct PremarketOnly;
    #[async_trait(?Send)]
    impl MarketData for PremarketOnly {
        async fn quote(&self, _symbol: &str) -> Result<Quote, ProviderError> {
            Err(ProviderError::Http(500))
        }
        async fn premarket_quotes(
            &self,
            _symbols: &[String],
            _now: DateTime<Utc>,
        ) -> Vec<(String, Result<Quote, ProviderError>)> {
            vec![]
        }
    }

    #[tokio::test]
    async fn close_quotes_default_to_unsupported_in_order() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        assert_eq!(
            PremarketOnly.close_quote("^GSPC", day).await,
            Err(ProviderError::Empty)
        );
        let r = PremarketOnly
            .close_quotes(&["A".into(), "B".into()], day)
            .await;
        assert_eq!(
            r,
            vec![
                ("A".to_string(), Err(ProviderError::Empty)),
                ("B".to_string(), Err(ProviderError::Empty))
            ]
        );
    }
}
