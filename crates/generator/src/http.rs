//! `Http` over the Workers `fetch` API.

use async_trait::async_trait;
use marketsite_core::providers::{Http, HttpResp, ProviderError};
use worker::{wasm_bindgen::JsValue, Fetch, Headers, Method, Request, RequestInit};

#[derive(Clone, Copy, Default)]
pub struct WorkerHttp;

fn net(e: worker::Error) -> ProviderError {
    ProviderError::Network(e.to_string())
}

impl WorkerHttp {
    async fn send(
        &self,
        method: Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<String>,
    ) -> Result<HttpResp, ProviderError> {
        let h = Headers::new();
        for (k, v) in headers {
            h.set(k, v).map_err(net)?;
        }
        if body.is_some() && !h.has("content-type").map_err(net)? {
            h.set("content-type", "application/json").map_err(net)?;
        }
        let mut init = RequestInit::new();
        init.with_method(method).with_headers(h).with_body(body.map(|b| JsValue::from_str(&b)));
        let req = Request::new_with_init(url, &init).map_err(net)?;
        let mut resp = Fetch::Request(req).send().await.map_err(net)?;
        let status = resp.status_code();
        let body = resp.text().await.map_err(net)?;
        Ok(HttpResp { status, body })
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
        self.send(Method::Post, url, headers, Some(body.to_string())).await
    }
}
