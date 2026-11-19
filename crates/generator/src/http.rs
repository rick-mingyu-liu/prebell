//! `Http` and `Timer` over the Workers `fetch` and timer APIs.

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use prebell_core::providers::{utf8_prefix, Http, HttpResp, ProviderError, Timer};
use worker::{
    wasm_bindgen::JsValue, AbortController, Delay, Fetch, Headers, Method, Request, RequestInit,
};

#[derive(Clone, Copy, Default)]
pub struct WorkerHttp;

fn net(e: worker::Error) -> ProviderError {
    ProviderError::Network(e.to_string())
}

/// Aborts the request if dropped before `disarm`, e.g. when a caller's
/// deadline wins the race against it.
struct AbortOnDrop(Option<AbortController>);

impl AbortOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(c) = self.0.take() {
            c.abort();
        }
    }
}

impl WorkerHttp {
    async fn send(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<String>,
    ) -> Result<HttpResp, ProviderError> {
        self.send_capped(method, url, headers, body, None).await
    }

    /// With `Cap::Fail`, a body over the cap (by `Content-Length` or while
    /// streaming) fails with `TooLarge` without reading the rest; with
    /// `Cap::Keep`, the first bytes up to the cap are kept and the rest is
    /// not read.
    async fn send_capped(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<String>,
        cap: Option<Cap>,
    ) -> Result<HttpResp, ProviderError> {
        let h = Headers::new();
        for (k, v) in headers {
            h.set(k, v).map_err(net)?;
        }
        if body.is_some() && !h.has("content-type").map_err(net)? {
            h.set("content-type", "application/json").map_err(net)?;
        }
        let mut init = RequestInit::new();
        init.with_method(method)
            .with_headers(h)
            .with_body(body.map(|b| JsValue::from_str(&b)));
        let req = Request::new_with_init(url, &init).map_err(net)?;
        let controller = AbortController::default();
        let signal = controller.signal();
        let mut guard = AbortOnDrop(Some(controller));
        let mut resp = Fetch::Request(req)
            .send_with_signal(&signal)
            .await
            .map_err(net)?;
        let status = resp.status_code();
        let set_cookie = resp.headers().get("set-cookie").map_err(net)?;
        let content_type = resp.headers().get("content-type").map_err(net)?;
        let body = match cap {
            None => resp.text().await.map_err(net)?,
            Some(Cap::Fail(max)) => {
                let declared = resp.headers().get("content-length").map_err(net)?;
                if declared.is_some_and(|n| n.parse::<usize>().is_ok_and(|n| n > max)) {
                    return Err(ProviderError::TooLarge(max));
                }
                let mut stream = resp.stream().map_err(net)?;
                let mut buf = Vec::new();
                while let Some(chunk) = stream.next().await {
                    buf.extend_from_slice(&chunk.map_err(net)?);
                    if buf.len() > max {
                        return Err(ProviderError::TooLarge(max));
                    }
                }
                String::from_utf8(buf).map_err(|e| ProviderError::Parse(e.to_string()))?
            }
            Some(Cap::Keep(max)) => {
                let mut stream = resp.stream().map_err(net)?;
                let mut buf = Vec::new();
                while let Some(chunk) = stream.next().await {
                    buf.extend_from_slice(&chunk.map_err(net)?);
                    if buf.len() >= max {
                        // Dropping the guard below aborts the rest of the body.
                        return Ok(HttpResp {
                            status,
                            body: utf8_prefix(buf, max)?,
                            set_cookie,
                            content_type,
                        });
                    }
                }
                utf8_prefix(buf, max)?
            }
        };
        guard.disarm();
        Ok(HttpResp {
            status,
            body,
            set_cookie,
            content_type,
        })
    }
}

#[async_trait(?Send)]
impl Http for WorkerHttp {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
        self.send(Method::Get, url, headers, None).await
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<HttpResp, ProviderError> {
        self.send(Method::Post, url, headers, Some(body.to_string()))
            .await
    }

    async fn patch_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<HttpResp, ProviderError> {
        self.send(Method::Patch, url, headers, Some(body.to_string()))
            .await
    }

    async fn get_capped(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<HttpResp, ProviderError> {
        self.send_capped(Method::Get, url, headers, None, Some(Cap::Fail(max_bytes)))
            .await
    }

    async fn get_prefix(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        max_bytes: usize,
    ) -> Result<HttpResp, ProviderError> {
        self.send_capped(Method::Get, url, headers, None, Some(Cap::Keep(max_bytes)))
            .await
    }
}

/// How `send_capped` treats a body over its size limit.
#[derive(Clone, Copy)]
enum Cap {
    Fail(usize),
    Keep(usize),
}

#[derive(Clone, Copy, Default)]
pub struct WorkerTimer;

#[async_trait(?Send)]
impl Timer for WorkerTimer {
    async fn sleep(&self, d: Duration) {
        Delay::from(d).await
    }
}
