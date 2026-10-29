use crate::facts::Quote;
use async_trait::async_trait;
use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResp {
    pub status: u16,
    pub body: String,
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
    /// `close_quote` for each symbol, in order.
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
