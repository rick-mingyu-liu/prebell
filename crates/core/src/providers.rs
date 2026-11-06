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
    /// When the article was published, if the provider knows.
    pub published: Option<DateTime<Utc>>,
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
    /// A JSON `PATCH`. Only the digest uses it (Buttondown), so clients
    /// without one keep this default, which sends nothing.
    async fn patch_json(
        &self,
        _url: &str,
        _headers: &[(&str, &str)],
        _body: &serde_json::Value,
    ) -> Result<HttpResp, ProviderError> {
        Err(ProviderError::Network("PATCH not supported".into()))
    }
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
    /// `get` that keeps the first `max_bytes` of the body (cut at a character
    /// boundary) and drops the rest. Real clients should stop reading at the
    /// cap; this default trims only after the whole body is in memory.
    async fn get_prefix(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<HttpResp, ProviderError> {
        let mut resp = self.get(url, headers).await?;
        let mut end = resp.body.len().min(max_bytes);
        while !resp.body.is_char_boundary(end) {
            end -= 1;
        }
        resp.body.truncate(end);
        Ok(resp)
    }
}

/// The first `max` bytes of `buf` as text, with a character cut by the cap
/// dropped. Bytes that are not UTF-8 before the cap are an error.
pub fn utf8_prefix(mut buf: Vec<u8>, max: usize) -> Result<String, ProviderError> {
    buf.truncate(max);
    // At most three bytes of a split character hang past the last boundary.
    for _ in 0..4 {
        match String::from_utf8(buf) {
            Ok(s) => return Ok(s),
            Err(e) if e.utf8_error().error_len().is_none() => {
                buf = e.into_bytes();
                buf.pop();
            }
            Err(e) => return Err(ProviderError::Parse(e.to_string())),
        }
    }
    Err(ProviderError::Parse("invalid UTF-8 at the cap".into()))
}

/// `Http` that counts the requests it sends (failed ones too), shared by
/// its clones: the Workers Free plan allows 50 fetch subrequests per
/// invocation, and the generator logs this count per job.
#[derive(Clone)]
pub struct CountingHttp<H> {
    inner: H,
    n: std::rc::Rc<std::cell::Cell<usize>>,
}

impl<H> CountingHttp<H> {
    pub fn new(inner: H) -> Self {
        Self {
            inner,
            n: Default::default(),
        }
    }
    pub fn count(&self) -> usize {
        self.n.get()
    }
    pub fn reset(&self) {
        self.n.set(0);
    }
    fn bump(&self) {
        self.n.set(self.n.get() + 1);
    }
}

#[async_trait(?Send)]
impl<H: Http> Http for CountingHttp<H> {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
        self.bump();
        self.inner.get(url, headers).await
    }
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<HttpResp, ProviderError> {
        self.bump();
        self.inner.post_json(url, headers, body).await
    }
    async fn patch_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<HttpResp, ProviderError> {
        self.bump();
        self.inner.patch_json(url, headers, body).await
    }
    async fn get_capped(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<HttpResp, ProviderError> {
        self.bump();
        self.inner.get_capped(url, headers, max_bytes).await
    }
    async fn get_prefix(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<HttpResp, ProviderError> {
        self.bump();
        self.inner.get_prefix(url, headers, max_bytes).await
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
    /// `search` for news published at or after `since`. A provider that can
    /// filter by date should; callers still check `SearchHit::published`.
    /// The default ignores `since`.
    async fn search_since(
        &self,
        query: &str,
        max: usize,
        _since: DateTime<Utc>,
    ) -> Result<Vec<SearchHit>, ProviderError> {
        self.search(query, max).await
    }
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

    #[derive(Clone)]
    struct Echo;
    #[async_trait(?Send)]
    impl Http for Echo {
        async fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            Ok(HttpResp {
                status: 200,
                body: url.into(),
                set_cookie: None,
            })
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            Err(ProviderError::Http(500))
        }
    }

    #[test]
    fn utf8_prefix_drops_a_character_split_by_the_cap() {
        let s = "ab€".as_bytes().to_vec(); // '€' is 3 bytes
        assert_eq!(utf8_prefix(s.clone(), 10).unwrap(), "ab€");
        assert_eq!(utf8_prefix(s.clone(), 5).unwrap(), "ab€");
        assert_eq!(utf8_prefix(s.clone(), 4).unwrap(), "ab");
        assert_eq!(utf8_prefix(s, 2).unwrap(), "ab");
        assert!(utf8_prefix(vec![b'a', 0xff, b'b'], 3).is_err());
        // A 4-byte character split after any of its first three bytes.
        let s = "a😀".as_bytes().to_vec();
        for cap in 2..=4 {
            assert_eq!(utf8_prefix(s.clone(), cap).unwrap(), "a");
        }
        assert_eq!(utf8_prefix(s, 5).unwrap(), "a😀");
    }

    #[tokio::test]
    async fn counting_http_counts_every_request_across_clones() {
        let h = CountingHttp::new(Echo);
        let h2 = h.clone();
        assert_eq!(h.count(), 0);
        assert_eq!(
            h.get("https://a.test", &[]).await.unwrap().body,
            "https://a.test"
        );
        // A failed request is still a subrequest.
        assert!(h2
            .post_json("https://b.test", &[], &serde_json::json!({}))
            .await
            .is_err());
        // The capped default calls `get` on the inner client once.
        assert!(h2.get_capped("https://c.test", &[], 2).await.is_err());
        // The prefix keeps the first bytes instead of failing.
        assert_eq!(
            h2.get_prefix("https://d.test", &[], 9).await.unwrap().body,
            "https://d"
        );
        assert_eq!((h.count(), h2.count()), (4, 4));
        h.reset();
        assert_eq!(h2.count(), 0);
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
