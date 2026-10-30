use std::sync::OnceLock;

use crate::providers::{Http, NewsSearch, ProviderError, SearchHit};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::America::New_York;
use regex::Regex;
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

fn body(query: &str, max: usize) -> Value {
    json!({
        "query": query,
        "max_results": max,
        "topic": "news",
        "include_raw_content": true,
        "search_depth": "basic",
    })
}

/// RFC 2822 (Tavily's documented example: "Tue, 11 Mar 2025 17:00:00 GMT"),
/// RFC 3339, or a bare date read as midnight UTC.
fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    DateTime::parse_from_rfc2822(s)
        .or_else(|_| DateTime::parse_from_rfc3339(s))
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_time(NaiveTime::MIN).and_utc())
        })
}

/// The end (New York midnight) of a `/2026/10/23/` or `/2026-10-23/` date in
/// the URL path, so a day-only date never makes an article look older.
fn url_date(url: &str) -> Option<DateTime<Utc>> {
    static R: OnceLock<Regex> = OnceLock::new();
    let c = R
        .get_or_init(|| {
            Regex::new(r"/(20\d{2})[/-](\d{2})[/-](\d{2})(?:[/-]|$)").expect("valid built-in regex")
        })
        .captures(url)?;
    let day = NaiveDate::from_ymd_opt(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?)?;
    New_York
        .from_local_datetime(&day.succ_opt()?.and_time(NaiveTime::MIN))
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

/// Tavily's `published_date` is its best estimate and "can be later than the
/// original publish date" (an updated page), so an earlier date in the URL
/// path wins.
fn published(r: &Value, url: &str) -> Option<DateTime<Utc>> {
    let api = r
        .get("published_date")
        .and_then(Value::as_str)
        .and_then(parse_date);
    match (api, url_date(url)) {
        (Some(a), Some(u)) => Some(a.min(u)),
        (a, u) => a.or(u),
    }
}

impl<H: Http> Tavily<H> {
    async fn post(&self, body: &Value) -> Result<Vec<SearchHit>, ProviderError> {
        let auth = format!("Bearer {}", self.api_key);
        let resp = self
            .http
            .post_json(
                URL,
                &[
                    ("Authorization", auth.as_str()),
                    ("Content-Type", "application/json"),
                ],
                body,
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
                    published: published(r, &url),
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

#[async_trait(?Send)]
impl<H: Http> NewsSearch for Tavily<H> {
    async fn search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>, ProviderError> {
        let mut b = body(query, max);
        b["days"] = json!(2);
        self.post(&b).await
    }

    /// `start_date` (YYYY-MM-DD) filters on the publish or last-updated
    /// date; results with no detectable date are kept.
    async fn search_since(
        &self,
        query: &str,
        max: usize,
        since: DateTime<Utc>,
    ) -> Result<Vec<SearchHit>, ProviderError> {
        let mut b = body(query, max);
        b["start_date"] = json!(since.date_naive().format("%Y-%m-%d").to_string());
        self.post(&b).await
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
    async fn parses_published_date_and_keeps_unknown_as_none() {
        let hits = tavily(200, FIX).search("q", 5).await.unwrap();
        assert_eq!(
            hits[0].published,
            Some("2026-10-29T21:05:00Z".parse().unwrap())
        );
        assert_eq!(hits[1].published, None);
    }

    #[tokio::test]
    async fn search_since_sends_start_date_instead_of_days() {
        let t = tavily(200, FIX);
        let since = "2026-10-29T01:20:00Z".parse().unwrap();
        t.search_since("q", 3, since).await.unwrap();
        let seen = t.http.seen.borrow();
        let body = &seen[0].2;
        assert_eq!(body["start_date"], "2026-10-29");
        assert!(body.get("days").is_none(), "{body}");
        assert_eq!(body["topic"], "news");
        assert_eq!(body["max_results"], 3);
    }

    #[test]
    fn date_formats_and_url_dates() {
        let p = |s: &str| parse_date(s).map(|d| d.to_rfc3339());
        assert_eq!(
            p("Tue, 11 Mar 2025 17:00:00 GMT").as_deref(),
            Some("2025-03-11T17:00:00+00:00")
        );
        assert_eq!(
            p("2025-03-11T17:00:00Z").as_deref(),
            Some("2025-03-11T17:00:00+00:00")
        );
        assert_eq!(
            p("2025-03-11").as_deref(),
            Some("2025-03-11T00:00:00+00:00")
        );
        assert_eq!(p("last week"), None);
        // The URL day ends at midnight New York time (EDT here).
        let end = |u: &str| url_date(u).map(|d| d.to_rfc3339());
        assert_eq!(
            end("https://247wallst.com/investing/2026/10/23/arm-stock-has").as_deref(),
            Some("2026-10-24T04:00:00+00:00")
        );
        assert_eq!(
            end("https://www.bloomberg.com/news/articles/2026-10-29/stock-market-today").as_deref(),
            Some("2026-10-30T04:00:00+00:00")
        );
        assert_eq!(end("https://www.benzinga.com/26/10/62726972/x"), None);
        assert_eq!(end("https://n.test/2026/13/40/x"), None);
    }

    #[tokio::test]
    async fn an_older_url_date_wins_over_a_later_update_date() {
        let body = r#"{"results":[{"title":"Arm","url":"https://247wallst.com/investing/2026/10/23/arm-stock","content":"c","published_date":"Thu, 29 Oct 2026 21:05:00 GMT"},
            {"title":"B","url":"https://n.test/2026/10/30/b","content":"c","published_date":"Thu, 29 Oct 2026 21:05:00 GMT"},
            {"title":"C","url":"https://n.test/2026/10/28/c","content":"c","published_date":null}]}"#;
        let hits = tavily(200, body).search("q", 3).await.unwrap();
        let got: Vec<String> = hits
            .iter()
            .map(|h| h.published.unwrap().to_rfc3339())
            .collect();
        assert_eq!(
            got,
            [
                "2026-10-24T04:00:00+00:00",
                "2026-10-29T21:05:00+00:00",
                "2026-10-29T04:00:00+00:00"
            ]
        );
    }

    #[tokio::test]
    async fn http_error_maps() {
        let r = tavily(401, "{}").search("q", 3).await;
        assert_eq!(r, Err(ProviderError::Http(401)));
    }
}
