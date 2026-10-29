use crate::providers::{Http, NewsSearch, ProviderError, SearchHit};
use async_trait::async_trait;
use serde_json::{json, Value};

const URL: &str = "https://api.tavily.com/search";

pub struct Tavily<H> {
    pub http: H,
    pub api_key: String,
}

impl<H> Tavily<H> {
    pub fn new(http: H, api_key: String) -> Self {
        Self { http, api_key }
    }
}

#[async_trait(?Send)]
impl<H: Http> NewsSearch for Tavily<H> {
    async fn search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>, ProviderError> {
        let body = json!({
            "query": query,
            "max_results": max,
            "topic": "news",
            "days": 2,
            "include_raw_content": true,
            "search_depth": "basic",
        });
        let auth = format!("Bearer {}", self.api_key);
        let resp = self
            .http
            .post_json(
                URL,
                &[
                    ("Authorization", auth.as_str()),
                    ("Content-Type", "application/json"),
                ],
                &body,
            )
            .await?;
        if resp.status >= 400 {
            return Err(ProviderError::Http(resp.status));
        }
        let v: Value =
            serde_json::from_str(&resp.body).map_err(|e| ProviderError::Parse(e.to_string()))?;
        let results = v
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::Parse("missing results".into()))?;
        let s = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let hits: Vec<SearchHit> = results
            .iter()
            .filter_map(|r| {
                let url = s(r, "url");
                if url.is_empty() {
                    return None;
                }
                let raw = s(r, "raw_content");
                let content = if raw.trim().is_empty() {
                    s(r, "content")
                } else {
                    raw
                };
                Some(SearchHit {
                    url,
                    title: s(r, "title"),
                    content,
                })
            })
            .collect();
        if hits.is_empty() {
            return Err(ProviderError::Empty);
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::HttpResp;
    use std::cell::RefCell;

    const FIX: &str = include_str!("../tests/fixtures/tavily_search.json");

    type Seen = Vec<(String, Vec<(String, String)>, Value)>;
    struct FakeHttp {
        status: u16,
        body: String,
        seen: RefCell<Seen>,
    }
    #[async_trait(?Send)]
    impl Http for FakeHttp {
        async fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            unreachable!()
        }
        async fn post_json(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            body: &Value,
        ) -> Result<HttpResp, ProviderError> {
            self.seen.borrow_mut().push((
                url.into(),
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body.clone(),
            ));
            Ok(HttpResp {
                status: self.status,
                body: self.body.clone(),
                set_cookie: None,
            })
        }
    }
    fn tavily(status: u16, body: &str) -> Tavily<FakeHttp> {
        Tavily::new(
            FakeHttp {
                status,
                body: body.into(),
                seen: RefCell::new(vec![]),
            },
            "tk".into(),
        )
    }

    #[tokio::test]
    async fn parses_hits_with_raw_content_fallback_and_drops_empty_url() {
        let t = tavily(200, FIX);
        let hits = t.search("premarket news", 5).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].content, "Full article text A with more detail.");
        assert_eq!(hits[1].content, "Summary B");
        assert!(hits.iter().all(|h| !h.content.is_empty()));
    }

    #[tokio::test]
    async fn sends_expected_request() {
        let t = tavily(200, FIX);
        t.search("q", 7).await.unwrap();
        let seen = t.http.seen.borrow();
        let (url, headers, body) = &seen[0];
        assert_eq!(url, "https://api.tavily.com/search");
        assert!(headers.contains(&("Authorization".into(), "Bearer tk".into())));
        assert_eq!(body["query"], "q");
        assert_eq!(body["max_results"], 7);
        assert_eq!(body["topic"], "news");
        assert_eq!(body["days"], 2);
        assert_eq!(body["include_raw_content"], true);
    }

    #[tokio::test]
    async fn http_error_maps() {
        let r = tavily(401, "{}").search("q", 3).await;
        assert_eq!(r, Err(ProviderError::Http(401)));
    }
}
