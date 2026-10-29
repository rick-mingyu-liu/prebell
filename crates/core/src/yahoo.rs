use crate::facts::Quote;
use crate::providers::{Http, MarketData, ProviderError};
use async_trait::async_trait;
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Deserialize;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

pub struct Yahoo<H: Http> {
    pub http: H,
}

impl<H: Http> Yahoo<H> {
    pub fn new(http: H) -> Self {
        Self { http }
    }

    /// The 1-minute chart body for today, with pre/post-market bars. Index
    /// symbols carry `^`, which is percent-encoded in the path.
    async fn chart(&self, symbol: &str) -> Result<String, ProviderError> {
        let path = symbol.replace('^', "%5E");
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{path}?interval=1m&range=1d&includePrePost=true"
        );
        let resp = self
            .http
            .get(&url, &[("User-Agent", UA), ("Accept", "application/json")])
            .await?;
        if resp.status >= 400 {
            return Err(ProviderError::Http(resp.status));
        }
        Ok(resp.body)
    }
}

#[derive(Deserialize)]
struct Root {
    chart: Chart,
}
#[derive(Deserialize)]
struct Chart {
    result: Option<Vec<ResultItem>>,
    error: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct ResultItem {
    meta: Option<Meta>,
    #[serde(default)]
    timestamp: Vec<i64>,
    indicators: Option<Indicators>,
}
#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "chartPreviousClose")]
    chart_previous_close: Option<f64>,
    #[serde(rename = "exchangeTimezoneName")]
    tz: Option<String>,
    #[serde(rename = "regularMarketPrice")]
    regular_price: Option<f64>,
    /// Unix seconds of the last regular-session trade.
    #[serde(rename = "regularMarketTime")]
    regular_time: Option<i64>,
    #[serde(rename = "regularMarketVolume")]
    regular_volume: Option<f64>,
}
#[derive(Deserialize)]
struct Indicators {
    #[serde(default)]
    quote: Vec<Bars>,
}
#[derive(Deserialize, Default)]
struct Bars {
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<f64>>,
}

fn first_result(body: &str) -> Result<ResultItem, ProviderError> {
    let root: Root = serde_json::from_str(body).map_err(|e| ProviderError::Parse(e.to_string()))?;
    if root.chart.error.as_ref().is_some_and(|e| !e.is_null()) {
        return Err(ProviderError::Empty);
    }
    root.chart
        .result
        .and_then(|r| r.into_iter().next())
        .ok_or(ProviderError::Empty)
}

fn exchange_tz(meta: &Meta) -> Tz {
    meta.tz
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(chrono_tz::America::New_York)
}

/// The regular-session close from the chart `meta`, which Yahoo keeps at the
/// official close after 16:00 while post-market bars keep arriving.
fn parse_close(symbol: &str, body: &str, session: NaiveDate) -> Result<Quote, ProviderError> {
    let meta = first_result(body)?.meta.ok_or(ProviderError::Empty)?;
    let last = meta.regular_price.ok_or(ProviderError::Empty)?;
    let prev_close = meta.chart_previous_close.ok_or(ProviderError::Empty)?;
    let at = meta.regular_time.ok_or(ProviderError::Empty)?;
    let day = Utc
        .timestamp_opt(at, 0)
        .single()
        .ok_or(ProviderError::Empty)?
        .with_timezone(&exchange_tz(&meta))
        .date_naive();
    if day != session {
        return Err(ProviderError::Parse(format!(
            "stale quote: last trade on {day}, not {session}"
        )));
    }
    if prev_close == 0.0 {
        return Err(ProviderError::Empty);
    }
    Ok(Quote {
        symbol: symbol.to_string(),
        last,
        prev_close,
        change_pct: (last - prev_close) / prev_close * 100.0,
        volume: meta.regular_volume.map(|v| v as u64),
    })
}

fn parse_quote(symbol: &str, body: &str) -> Result<Quote, ProviderError> {
    let item = first_result(body)?;
    let meta = item.meta.ok_or(ProviderError::Empty)?;
    let prev_close = meta.chart_previous_close.ok_or(ProviderError::Empty)?;
    let tz = exchange_tz(&meta);
    let bars = item
        .indicators
        .and_then(|i| i.quote.into_iter().next())
        .unwrap_or_default();

    // (local datetime, close, volume) for non-null closes.
    let pts: Vec<(chrono::DateTime<Tz>, f64, Option<f64>)> = item
        .timestamp
        .iter()
        .enumerate()
        .filter_map(|(i, &t)| {
            let c = (*bars.close.get(i)?)?;
            let v = bars.volume.get(i).copied().flatten();
            let dt = Utc.timestamp_opt(t, 0).single()?.with_timezone(&tz);
            Some((dt, c, v))
        })
        .collect();
    let (final_dt, final_close, _) = pts.last().ok_or(ProviderError::Empty)?;
    let open = NaiveTime::from_hms_opt(9, 30, 0).unwrap();
    let pre: Vec<_> = pts
        .iter()
        .filter(|(dt, _, _)| dt.date_naive() == final_dt.date_naive() && dt.time() < open)
        .collect();
    let (last, volume) = match pre.last() {
        Some((_, c, _)) => {
            let vols: Vec<f64> = pre.iter().filter_map(|p| p.2).collect();
            let vol = if vols.is_empty() {
                None
            } else {
                Some(vols.iter().sum::<f64>() as u64)
            };
            (*c, vol)
        }
        None => (*final_close, None),
    };
    if prev_close == 0.0 {
        return Err(ProviderError::Empty);
    }
    Ok(Quote {
        symbol: symbol.to_string(),
        last,
        prev_close,
        change_pct: (last - prev_close) / prev_close * 100.0,
        volume,
    })
}

#[async_trait(?Send)]
impl<H: Http> MarketData for Yahoo<H> {
    async fn quote(&self, symbol: &str) -> Result<Quote, ProviderError> {
        parse_quote(symbol, &self.chart(symbol).await?)
    }

    async fn close_quote(&self, symbol: &str, session: NaiveDate) -> Result<Quote, ProviderError> {
        parse_close(symbol, &self.chart(symbol).await?, session)
    }

    async fn close_quotes(
        &self,
        symbols: &[String],
        session: NaiveDate,
    ) -> Vec<(String, Result<Quote, ProviderError>)> {
        let mut out = Vec::with_capacity(symbols.len());
        for chunk in symbols.chunks(20) {
            let results = futures::future::join_all(
                chunk
                    .iter()
                    .map(|s| async move { self.close_quote(s, session).await }),
            )
            .await;
            out.extend(chunk.iter().cloned().zip(results));
        }
        out
    }

    async fn premarket_quotes(
        &self,
        symbols: &[String],
    ) -> Vec<(String, Result<Quote, ProviderError>)> {
        let mut out = Vec::with_capacity(symbols.len());
        for chunk in symbols.chunks(20) {
            let results =
                futures::future::join_all(chunk.iter().map(|s| async move { self.quote(s).await }))
                    .await;
            out.extend(chunk.iter().cloned().zip(results));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::HttpResp;
    use std::collections::HashMap;

    const ES: &str = include_str!("../tests/fixtures/yahoo_chart_ES=F.json");
    const AAPL: &str = include_str!("../tests/fixtures/yahoo_chart_AAPL_pre.json");
    const HTML: &str = include_str!("../tests/fixtures/yahoo_429.html");
    const GSPC: &str = include_str!("../tests/fixtures/yahoo_chart_GSPC_close.json");
    const NVDA: &str = include_str!("../tests/fixtures/yahoo_chart_NVDA_close.json");

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    struct FakeHttp(HashMap<String, (u16, String)>);
    #[async_trait(?Send)]
    impl Http for FakeHttp {
        async fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
        ) -> Result<HttpResp, ProviderError> {
            assert!(headers
                .iter()
                .any(|(k, v)| *k == "User-Agent" && v.contains("Mozilla")));
            let (status, body) = self
                .0
                .get(url)
                .cloned()
                .ok_or(ProviderError::Network(url.into()))?;
            Ok(HttpResp { status, body })
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &serde_json::Value,
        ) -> Result<HttpResp, ProviderError> {
            unreachable!()
        }
    }
    fn url(s: &str) -> String {
        format!("https://query1.finance.yahoo.com/v8/finance/chart/{s}?interval=1m&range=1d&includePrePost=true")
    }
    fn yahoo(items: &[(&str, u16, &str)]) -> Yahoo<FakeHttp> {
        Yahoo::new(FakeHttp(
            items
                .iter()
                .map(|(s, c, b)| (url(s), (*c, b.to_string())))
                .collect(),
        ))
    }

    #[tokio::test]
    async fn es_fixture_parses_with_prev_close() {
        let q = yahoo(&[("ES=F", 200, ES)]).quote("ES=F").await.unwrap();
        assert_eq!(q.prev_close, 5890.0);
        assert_eq!(q.last, 5912.75);
        assert!((q.change_pct - (5912.75 - 5890.0) / 5890.0 * 100.0).abs() < 1e-9);
        assert_eq!(q.volume, Some(1200 + 800 + 500 + 600));
    }

    #[tokio::test]
    async fn aapl_premarket_excludes_regular_session_and_null_volume() {
        let q = yahoo(&[("AAPL", 200, AAPL)]).quote("AAPL").await.unwrap();
        assert_eq!(q.last, 228.5);
        assert_eq!(q.volume, Some(3000 + 12000 + 4000));
    }

    #[tokio::test]
    async fn null_close_entries_are_skipped() {
        // ES fixture has a null close at 07:30; last bar must not be it, and result is still fine.
        let body = ES.replace("5912.75", "null").replace("5910.5", "5911.0");
        let q = yahoo(&[("ES=F", 200, &body)]).quote("ES=F").await.unwrap();
        assert_eq!(q.last, 5911.0);
    }

    #[tokio::test]
    async fn no_premarket_bars_falls_back_to_latest_bar() {
        let body = AAPL
            .replace("226.1", "null")
            .replace("227.4", "null")
            .replace("228.0", "null")
            .replace("228.5", "null");
        let q = yahoo(&[("AAPL", 200, &body)]).quote("AAPL").await.unwrap();
        assert_eq!(q.last, 230.2);
        assert_eq!(q.volume, None);
    }

    #[tokio::test]
    async fn status_429_is_http_error() {
        let r = yahoo(&[("ES=F", 429, HTML)]).quote("ES=F").await;
        assert_eq!(r, Err(ProviderError::Http(429)));
    }

    #[tokio::test]
    async fn html_body_with_200_is_parse_error() {
        let r = yahoo(&[("ES=F", 200, HTML)]).quote("ES=F").await;
        assert!(matches!(r, Err(ProviderError::Parse(_))));
    }

    #[tokio::test]
    async fn empty_result_is_empty() {
        let r = yahoo(&[("X", 200, r#"{"chart":{"result":[],"error":null}}"#)])
            .quote("X")
            .await;
        assert_eq!(r, Err(ProviderError::Empty));
        let r = yahoo(&[(
            "X",
            200,
            r#"{"chart":{"result":null,"error":{"code":"Not Found"}}}"#,
        )])
        .quote("X")
        .await;
        assert_eq!(r, Err(ProviderError::Empty));
    }

    #[tokio::test]
    async fn premarket_quotes_keeps_order_and_errors() {
        let y = yahoo(&[("ES=F", 200, ES), ("AAPL", 429, HTML)]);
        let r = y.premarket_quotes(&["ES=F".into(), "AAPL".into()]).await;
        assert_eq!(r[0].0, "ES=F");
        assert!(r[0].1.is_ok());
        assert_eq!(r[1].1, Err(ProviderError::Http(429)));
    }

    #[tokio::test]
    async fn index_close_comes_from_meta_and_encodes_caret() {
        // FakeHttp only knows the encoded URL, so an unencoded `^` fails.
        let y = yahoo(&[("%5EGSPC", 200, GSPC)]);
        let q = y.close_quote("^GSPC", day("2026-10-29")).await.unwrap();
        assert_eq!(q.symbol, "^GSPC");
        assert_eq!(q.last, 5850.12);
        assert_eq!(q.prev_close, 5820.4);
        assert!((q.change_pct - (5850.12 - 5820.4) / 5820.4 * 100.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn stock_close_ignores_after_hours_bars() {
        let q = yahoo(&[("NVDA", 200, NVDA)])
            .close_quote("NVDA", day("2026-10-29"))
            .await
            .unwrap();
        assert_eq!(q.last, 182.35);
        assert_eq!(q.prev_close, 176.9);
        assert_eq!(q.volume, Some(210_000_000));
    }

    #[tokio::test]
    async fn close_from_another_session_is_stale() {
        let y = yahoo(&[("NVDA", 200, NVDA)]);
        let r = y.close_quote("NVDA", day("2026-10-30")).await;
        assert!(
            matches!(&r, Err(ProviderError::Parse(m)) if m.contains("stale")),
            "{r:?}"
        );
    }

    #[tokio::test]
    async fn close_without_market_time_or_price_is_empty() {
        let y = yahoo(&[("NVDA", 200, &NVDA.replace("\"regularMarketTime\"", "\"x\""))]);
        assert_eq!(
            y.close_quote("NVDA", day("2026-10-29")).await,
            Err(ProviderError::Empty)
        );
        let y = yahoo(&[(
            "NVDA",
            200,
            &NVDA.replace("\"regularMarketPrice\"", "\"x\""),
        )]);
        assert_eq!(
            y.close_quote("NVDA", day("2026-10-29")).await,
            Err(ProviderError::Empty)
        );
    }

    #[tokio::test]
    async fn close_errors_and_order() {
        let y = yahoo(&[("%5EGSPC", 429, HTML), ("NVDA", 200, NVDA)]);
        let r = y
            .close_quotes(&["^GSPC".into(), "NVDA".into()], day("2026-10-29"))
            .await;
        assert_eq!(r[0], ("^GSPC".to_string(), Err(ProviderError::Http(429))));
        assert_eq!(r[1].0, "NVDA");
        assert_eq!(r[1].1.as_ref().unwrap().last, 182.35);
        let r = yahoo(&[("NVDA", 200, HTML)])
            .close_quote("NVDA", day("2026-10-29"))
            .await;
        assert!(matches!(r, Err(ProviderError::Parse(_))));
    }
}
