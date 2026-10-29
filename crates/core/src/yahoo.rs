use crate::facts::Quote;
use crate::providers::{Http, HttpResp, MarketData, ProviderError};
use async_trait::async_trait;
use chrono::{NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// Sets the session cookie; it answers 404, which is normal.
const FC_URL: &str = "https://fc.yahoo.com";
/// Returns the crumb as a plain-text body for a request carrying the cookie.
const CRUMB_URL: &str = "https://query1.finance.yahoo.com/v1/test/getcrumb";
const QUOTE_URL: &str = "https://query1.finance.yahoo.com/v7/finance/quote";
/// Symbols per v7 quote request. Requests are sent one after another.
const QUOTE_BATCH: usize = 50;

#[derive(Clone)]
struct Session {
    cookie: String,
    crumb: String,
}

pub struct Yahoo<H: Http> {
    pub http: H,
    /// Cookie and crumb for the v7 quote endpoint, fetched once and kept for
    /// the life of this struct (refreshed once on a 401/403).
    session: RefCell<Option<Session>>,
}

impl<H: Http> Yahoo<H> {
    pub fn new(http: H) -> Self {
        Self {
            http,
            session: RefCell::new(None),
        }
    }

    async fn session(&self, refresh: bool) -> Result<Session, ProviderError> {
        let cached = self.session.borrow().clone();
        if let (false, Some(s)) = (refresh, cached) {
            return Ok(s);
        }
        let fc = self.http.get(FC_URL, &[("User-Agent", UA)]).await?;
        let cookie = fc
            .set_cookie
            .as_deref()
            .map(cookie_header)
            .unwrap_or_default();
        if cookie.is_empty() {
            return Err(if fc.status >= 400 && fc.status != 404 {
                ProviderError::Http(fc.status)
            } else {
                ProviderError::Parse("fc.yahoo.com set no cookie".into())
            });
        }
        let r = self
            .http
            .get(CRUMB_URL, &[("User-Agent", UA), ("Cookie", &cookie)])
            .await?;
        if r.status >= 400 {
            return Err(ProviderError::Http(r.status));
        }
        let crumb = r.body.trim();
        if crumb.is_empty()
            || crumb.len() > 64
            || crumb.contains(|c: char| c.is_whitespace() || "<>{}\"".contains(c))
        {
            return Err(ProviderError::Parse("getcrumb returned no crumb".into()));
        }
        let s = Session {
            cookie,
            crumb: crumb.to_string(),
        };
        *self.session.borrow_mut() = Some(s.clone());
        Ok(s)
    }

    async fn quote_batch(
        &self,
        symbols: &[String],
        s: &Session,
    ) -> Result<HttpResp, ProviderError> {
        let list: Vec<String> = symbols.iter().map(|x| encode(x)).collect();
        let url = format!(
            "{QUOTE_URL}?symbols={}&crumb={}",
            list.join(","),
            encode(&s.crumb)
        );
        self.http
            .get(
                &url,
                &[
                    ("User-Agent", UA),
                    ("Accept", "application/json"),
                    ("Cookie", &s.cookie),
                ],
            )
            .await
    }
}

/// The `name=value` pairs of a `Set-Cookie` value, as a `Cookie` header.
/// Several headers may arrive joined by ", ", and `Expires` dates contain a
/// comma too, so a segment counts only when it starts with `name=`.
fn cookie_header(set_cookie: &str) -> String {
    set_cookie
        .split(", ")
        .filter_map(|seg| {
            let pair = seg.split(';').next()?.trim();
            let (name, _) = pair.split_once('=')?;
            (!name.is_empty() && !name.contains(char::is_whitespace)).then_some(pair)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Deserialize)]
struct QuoteRoot {
    #[serde(rename = "quoteResponse")]
    quote_response: QuoteResponse,
}
#[derive(Deserialize)]
struct QuoteResponse {
    result: Option<Vec<serde_json::Value>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct V7Quote {
    symbol: String,
    pre_market_price: Option<f64>,
    pre_market_change: Option<f64>,
    pre_market_change_percent: Option<f64>,
    regular_market_price: Option<f64>,
}

type Batch = HashMap<String, Result<Quote, ProviderError>>;

/// One v7 quote body, keyed by each item's `symbol` field (never by position).
fn parse_batch(body: &str) -> Result<Batch, ProviderError> {
    let root: QuoteRoot =
        serde_json::from_str(body).map_err(|e| ProviderError::Parse(e.to_string()))?;
    let mut out = HashMap::new();
    for item in root.quote_response.result.unwrap_or_default() {
        let Some(sym) = item
            .get("symbol")
            .and_then(|v| v.as_str())
            .map(String::from)
        else {
            continue;
        };
        let q = serde_json::from_value::<V7Quote>(item)
            .map_err(|e| ProviderError::Parse(e.to_string()))
            .and_then(v7_quote);
        out.insert(sym, q);
    }
    Ok(out)
}

/// Premarket fields of a v7 quote. Before the open (the brief runs around
/// 8:30 AM ET) `regularMarketPrice` is still the prior session's close, so it
/// is `prev_close`. It is cross-checked against `preMarketPrice -
/// preMarketChange`; when they disagree that difference is used instead.
/// v7 has no premarket volume, so `volume` is `None`.
fn v7_quote(v: V7Quote) -> Result<Quote, ProviderError> {
    // No premarket trades yet.
    let last = v.pre_market_price.ok_or(ProviderError::Empty)?;
    let derived = v.pre_market_change.map(|c| last - c);
    let prev_close = match (v.regular_market_price, derived) {
        (Some(r), Some(d)) if (r - d).abs() > 0.01_f64.max(r.abs() * 1e-4) => d,
        (Some(r), _) => r,
        (None, Some(d)) => d,
        (None, None) => return Err(ProviderError::Empty),
    };
    if !last.is_finite() || !prev_close.is_finite() || prev_close <= 0.0 {
        return Err(ProviderError::Empty);
    }
    Ok(Quote {
        symbol: v.symbol,
        last,
        prev_close,
        // Percent units already: 1.15 means +1.15%.
        change_pct: v
            .pre_market_change_percent
            .unwrap_or((last - prev_close) / prev_close * 100.0),
        volume: None,
    })
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

fn parse_quote(symbol: &str, body: &str) -> Result<Quote, ProviderError> {
    let root: Root = serde_json::from_str(body).map_err(|e| ProviderError::Parse(e.to_string()))?;
    if root.chart.error.as_ref().is_some_and(|e| !e.is_null()) {
        return Err(ProviderError::Empty);
    }
    let item = root
        .chart
        .result
        .and_then(|r| r.into_iter().next())
        .ok_or(ProviderError::Empty)?;
    let meta = item.meta.ok_or(ProviderError::Empty)?;
    let prev_close = meta.chart_previous_close.ok_or(ProviderError::Empty)?;
    let tz: Tz = meta
        .tz
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(chrono_tz::America::New_York);
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
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?interval=1m&range=1d&includePrePost=true"
        );
        let resp = self
            .http
            .get(&url, &[("User-Agent", UA), ("Accept", "application/json")])
            .await?;
        if resp.status >= 400 {
            return Err(ProviderError::Http(resp.status));
        }
        parse_quote(symbol, &resp.body)
    }

    /// Batched v7 quotes, `QUOTE_BATCH` symbols per request. A failed cookie
    /// or crumb fetch fails every symbol; after a 429 the remaining chunks are
    /// not requested and fail with 429 too, so a throttled IP is not hammered.
    async fn premarket_quotes(
        &self,
        symbols: &[String],
    ) -> Vec<(String, Result<Quote, ProviderError>)> {
        let fail_all = |e: ProviderError| {
            symbols
                .iter()
                .map(|s| (s.clone(), Err(e.clone())))
                .collect()
        };
        let mut session = match self.session(false).await {
            Ok(s) => s,
            Err(e) => return fail_all(e),
        };
        let mut refreshed = false;
        let mut throttled = false;
        let mut out = Vec::with_capacity(symbols.len());
        for chunk in symbols.chunks(QUOTE_BATCH) {
            let r = if throttled {
                Err(ProviderError::Http(429))
            } else {
                let mut r = self.quote_batch(chunk, &session).await;
                if !refreshed && r.as_ref().is_ok_and(|x| matches!(x.status, 401 | 403)) {
                    refreshed = true;
                    r = match self.session(true).await {
                        Ok(s) => {
                            session = s;
                            self.quote_batch(chunk, &session).await
                        }
                        Err(e) => Err(e),
                    };
                }
                r.and_then(|x| match x.status {
                    s if s >= 400 => Err(ProviderError::Http(s)),
                    _ => parse_batch(&x.body),
                })
            };
            throttled |= r.as_ref().err() == Some(&ProviderError::Http(429));
            match r {
                Ok(batch) => out.extend(chunk.iter().map(|s| {
                    let q = batch.get(s).cloned().unwrap_or(Err(ProviderError::Empty));
                    (s.clone(), q)
                })),
                Err(e) => out.extend(chunk.iter().map(|s| (s.clone(), Err(e.clone())))),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::HttpResp;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const ES: &str = include_str!("../tests/fixtures/yahoo_chart_ES=F.json");
    const AAPL: &str = include_str!("../tests/fixtures/yahoo_chart_AAPL_pre.json");
    const HTML: &str = include_str!("../tests/fixtures/yahoo_429.html");
    const BATCH: &str = include_str!("../tests/fixtures/yahoo_quote_batch.json");
    const CRUMB: &str = include_str!("../tests/fixtures/yahoo_getcrumb.txt");
    const SET_COOKIE: &str = "A3=d=AQABBxyz&S=AQAAAabc; Expires=Fri, 29 Oct 2027 12:00:00 GMT; Max-Age=31557600; Domain=.yahoo.com; Path=/; SameSite=None; Secure; HttpOnly";
    const COOKIE: &str = "A3=d=AQABBxyz&S=AQAAAabc";

    #[derive(Clone, Debug)]
    struct Req {
        url: String,
        headers: Vec<(String, String)>,
    }
    impl Req {
        fn header(&self, k: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.as_str())
        }
    }

    type Route = Box<dyn Fn(&Req, &[Req]) -> HttpResp>;
    /// Answers every GET through `route`, which also sees the earlier requests.
    struct FakeHttp {
        route: Route,
        seen: RefCell<Vec<Req>>,
    }
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
            let req = Req {
                url: url.into(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            };
            let resp = (self.route)(&req, &self.seen.borrow());
            self.seen.borrow_mut().push(req);
            Ok(resp)
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
    fn resp(status: u16, body: &str) -> HttpResp {
        HttpResp {
            status,
            body: body.into(),
            set_cookie: None,
        }
    }
    fn fake(route: impl Fn(&Req, &[Req]) -> HttpResp + 'static) -> Yahoo<FakeHttp> {
        Yahoo::new(FakeHttp {
            route: Box::new(route),
            seen: RefCell::new(vec![]),
        })
    }
    fn url(s: &str) -> String {
        format!("https://query1.finance.yahoo.com/v8/finance/chart/{s}?interval=1m&range=1d&includePrePost=true")
    }
    fn yahoo(items: &[(&str, u16, &str)]) -> Yahoo<FakeHttp> {
        let m: HashMap<String, (u16, String)> = items
            .iter()
            .map(|(s, c, b)| (url(s), (*c, b.to_string())))
            .collect();
        fake(move |r, _| {
            let (c, b) = m.get(&r.url).cloned().unwrap_or((404, String::new()));
            resp(c, &b)
        })
    }
    fn seen(y: &Yahoo<FakeHttp>, prefix: &str) -> Vec<Req> {
        y.http
            .seen
            .borrow()
            .iter()
            .filter(|r| r.url.starts_with(prefix))
            .cloned()
            .collect()
    }
    fn syms(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }
    /// The `symbols` and `crumb` query values of a v7 quote URL, decoded.
    fn query(url: &str) -> (Vec<String>, String) {
        let q = url.split_once('?').unwrap().1;
        let mut symbols = vec![];
        let mut crumb = String::new();
        for kv in q.split('&') {
            let (k, v) = kv.split_once('=').unwrap();
            let v = v.replace("%2F", "/").replace("%3D", "=");
            match k {
                "symbols" => symbols = v.split(',').map(String::from).collect(),
                "crumb" => crumb = v,
                _ => {}
            }
        }
        (symbols, crumb)
    }
    /// A v7 body with one premarket quote per symbol.
    fn batch_for(symbols: &[String]) -> String {
        let items: Vec<serde_json::Value> = symbols
            .iter()
            .map(|s| {
                serde_json::json!({"symbol": s, "marketState": "PRE", "regularMarketPrice": 100.0,
                    "preMarketPrice": 101.0, "preMarketChange": 1.0, "preMarketChangePercent": 1.0})
            })
            .collect();
        serde_json::json!({"quoteResponse": {"result": items, "error": null}}).to_string()
    }
    /// fc.yahoo.com sets the cookie (with a 404), getcrumb needs it, and the
    /// quote endpoint needs both; `quote` answers authorised quote requests.
    fn yahoo_v7(quote: impl Fn(&Req, &[Req]) -> HttpResp + 'static) -> Yahoo<FakeHttp> {
        fake(move |r, prior| {
            if r.url == FC_URL {
                HttpResp {
                    set_cookie: Some(SET_COOKIE.into()),
                    ..resp(404, "<html>404</html>")
                }
            } else if r.url == CRUMB_URL {
                if r.header("Cookie") == Some(COOKIE) {
                    resp(200, CRUMB)
                } else {
                    resp(401, "")
                }
            } else if r.url.starts_with(QUOTE_URL) {
                let (_, crumb) = query(&r.url);
                if r.header("Cookie") != Some(COOKIE) || crumb != CRUMB {
                    resp(
                        401,
                        r#"{"finance":{"result":null,"error":{"code":"Unauthorized","description":"Invalid Crumb"}}}"#,
                    )
                } else {
                    quote(r, prior)
                }
            } else {
                resp(404, "")
            }
        })
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
    async fn crumb_flow_sends_cookie_and_crumb_and_is_cached() {
        let y = yahoo_v7(|_, _| resp(200, BATCH));
        let r = y.premarket_quotes(&syms(&["AAPL"])).await;
        assert!(r[0].1.is_ok(), "{r:?}");
        let crumb = seen(&y, CRUMB_URL);
        assert_eq!(crumb.len(), 1);
        assert_eq!(crumb[0].header("Cookie"), Some(COOKIE));
        let quotes = seen(&y, QUOTE_URL);
        assert_eq!(quotes.len(), 1);
        assert!(
            quotes[0].url.starts_with(
                "https://query1.finance.yahoo.com/v7/finance/quote?symbols=AAPL&crumb="
            ),
            "{}",
            quotes[0].url
        );
        // The crumb's `/` is percent-encoded in the URL.
        assert!(
            quotes[0].url.ends_with("crumb=zP1x0k%2FLq7e"),
            "{}",
            quotes[0].url
        );
        assert_eq!(quotes[0].header("Cookie"), Some(COOKIE));

        // A second call reuses the cached cookie and crumb.
        y.premarket_quotes(&syms(&["MSFT"])).await;
        assert_eq!(seen(&y, FC_URL).len(), 1);
        assert_eq!(seen(&y, CRUMB_URL).len(), 1);
        assert_eq!(seen(&y, QUOTE_URL).len(), 2);
    }

    #[tokio::test]
    async fn quotes_are_chunked_by_50_sequentially() {
        let y = yahoo_v7(|r, _| resp(200, &batch_for(&query(&r.url).0)));
        let all: Vec<String> = (0..120).map(|i| format!("S{i:03}")).collect();
        let r = y.premarket_quotes(&all).await;
        let quotes = seen(&y, QUOTE_URL);
        let sizes: Vec<usize> = quotes.iter().map(|q| query(&q.url).0.len()).collect();
        assert_eq!(sizes, vec![50, 50, 20]);
        assert_eq!(query(&quotes[2].url).0[0], "S100");
        assert_eq!(r.len(), 120);
        assert!(r.iter().all(|(_, q)| q.is_ok()));
        assert_eq!(r.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>(), all);
    }

    #[tokio::test]
    async fn results_map_by_symbol_field_not_position() {
        let y = yahoo_v7(|_, _| resp(200, BATCH));
        let want = syms(&["AAPL", "BRK-B", "MSFT", "NVDA", "TSLA", "ZZZZ"]);
        let r = y.premarket_quotes(&want).await;
        assert_eq!(r.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>(), want);
        let get = |s: &str| r.iter().find(|(k, _)| k == s).unwrap().1.clone();

        let aapl = get("AAPL").unwrap();
        assert_eq!(aapl.symbol, "AAPL");
        assert_eq!(aapl.last, 232.15);
        assert_eq!(aapl.prev_close, 229.5);
        assert_eq!(aapl.change_pct, 1.154684);
        assert_eq!(aapl.volume, None);
        let msft = get("MSFT").unwrap();
        assert_eq!((msft.last, msft.prev_close), (405.2, 411.3));
        assert_eq!(msft.change_pct, -1.483102);
        assert_eq!(get("BRK-B").unwrap().last, 481.0);
        // Absent from the response.
        assert_eq!(get("ZZZZ"), Err(ProviderError::Empty));
    }

    #[tokio::test]
    async fn missing_premarket_price_is_empty() {
        let y = yahoo_v7(|_, _| resp(200, BATCH));
        let r = y.premarket_quotes(&syms(&["NVDA"])).await;
        assert_eq!(r[0].1, Err(ProviderError::Empty));
    }

    #[tokio::test]
    async fn prev_close_falls_back_when_regular_price_disagrees() {
        // TSLA: preMarketPrice 260.4 - preMarketChange 10.4 = 250.0, but regularMarketPrice is 255.0.
        let y = yahoo_v7(|_, _| resp(200, BATCH));
        let q = y.premarket_quotes(&syms(&["TSLA"])).await[0]
            .1
            .clone()
            .unwrap();
        assert_eq!(q.last, 260.4);
        assert!((q.prev_close - 250.0).abs() < 1e-9, "{}", q.prev_close);
        assert_eq!(q.change_pct, 4.16);
    }

    #[tokio::test]
    async fn unauthorized_refreshes_crumb_once_then_succeeds() {
        // The first crumb is stale: the quote endpoint rejects it.
        let y = fake(|r, prior| {
            let crumbs = prior.iter().filter(|p| p.url == CRUMB_URL).count();
            if r.url == FC_URL {
                HttpResp {
                    set_cookie: Some(SET_COOKIE.into()),
                    ..resp(404, "")
                }
            } else if r.url == CRUMB_URL {
                resp(200, if crumbs == 0 { "stale" } else { "fresh" })
            } else if query(&r.url).1 == "fresh" {
                resp(200, BATCH)
            } else {
                resp(401, "")
            }
        });
        let r = y.premarket_quotes(&syms(&["AAPL"])).await;
        assert!(r[0].1.is_ok(), "{r:?}");
        assert_eq!(seen(&y, CRUMB_URL).len(), 2);
        assert_eq!(seen(&y, QUOTE_URL).len(), 2);
    }

    #[tokio::test]
    async fn persistent_unauthorized_refreshes_only_once() {
        let y = yahoo_v7(|_, _| resp(403, ""));
        let all: Vec<String> = (0..120).map(|i| format!("S{i:03}")).collect();
        let r = y.premarket_quotes(&all).await;
        assert!(r.iter().all(|(_, q)| *q == Err(ProviderError::Http(403))));
        assert_eq!(seen(&y, CRUMB_URL).len(), 2);
    }

    #[tokio::test]
    async fn status_429_fails_every_symbol_and_stops_requesting() {
        let y = yahoo_v7(|_, _| resp(429, HTML));
        let all: Vec<String> = (0..120).map(|i| format!("S{i:03}")).collect();
        let r = y.premarket_quotes(&all).await;
        assert_eq!(r.len(), 120);
        assert!(r.iter().all(|(_, q)| *q == Err(ProviderError::Http(429))));
        // Throttled: the remaining chunks are not requested.
        assert_eq!(seen(&y, QUOTE_URL).len(), 1);
    }

    #[tokio::test]
    async fn throttled_cookie_or_crumb_fails_every_symbol() {
        let y = fake(|_, _| resp(429, HTML));
        let r = y.premarket_quotes(&syms(&["AAPL", "MSFT"])).await;
        assert!(
            r.iter().all(|(_, q)| *q == Err(ProviderError::Http(429))),
            "{r:?}"
        );
        assert_eq!(y.http.seen.borrow().len(), 1);

        let y = fake(|r, _| {
            if r.url == FC_URL {
                HttpResp {
                    set_cookie: Some(SET_COOKIE.into()),
                    ..resp(404, "")
                }
            } else {
                resp(429, "Too Many Requests")
            }
        });
        let r = y.premarket_quotes(&syms(&["AAPL"])).await;
        assert_eq!(r[0].1, Err(ProviderError::Http(429)));
    }

    #[tokio::test]
    async fn html_quote_body_is_parse_error() {
        let y = yahoo_v7(|_, _| resp(200, HTML));
        let r = y.premarket_quotes(&syms(&["AAPL"])).await;
        assert!(matches!(r[0].1, Err(ProviderError::Parse(_))), "{r:?}");
    }

    #[test]
    fn cookie_header_keeps_only_name_value_pairs() {
        assert_eq!(cookie_header(SET_COOKIE), COOKIE);
        // Workers joins several Set-Cookie headers with ", "; Expires has a comma too.
        let two = format!("{SET_COOKIE}, A1=d=xyz; Expires=Fri, 29 Oct 2027 12:00:00 GMT; Path=/");
        assert_eq!(cookie_header(&two), format!("{COOKIE}; A1=d=xyz"));
        assert_eq!(cookie_header(""), "");
    }
}
