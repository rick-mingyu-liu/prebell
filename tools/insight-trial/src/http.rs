//! The recording HTTP layer, spend cap, price table, ledger and key loading,
//! copied from `tools/llm-eval/src/main.rs` on branch `eval/claude-vs-deepseek`.
//! Three changes: `RecordingHttp` forwards to an inner `Http` (the real
//! `ReqwestHttp`, or a mock in tests) instead of holding the reqwest client,
//! each ledger line also records the call's role and tokens, and `Call`
//! drops `user_prompt` (llm-eval read retry notes from it; the trial has none).
//!
//! Keys stay in memory: no request header is ever logged or written.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{Datelike, Timelike, Utc, Weekday};
use prebell_core::providers::{Http, HttpResp, ProviderError};
use serde::Serialize;
use serde_json::{json, Value};

/// USD per 1M tokens: (uncached input, cached input, output).
pub(crate) fn prices(model: &str, at: chrono::DateTime<Utc>) -> (f64, f64, f64) {
    match model {
        // claude-api skill, cached 2026-10-06.
        "claude-opus-5-5" => (4.00, 0.20, 20.00),
        "claude-sonnet-5-5" => (2.00, 0.20, 10.00),
        // api-docs.deepseek.com/quick_start/pricing, read 2026-11-06: legacy
        // names are served and billed as deepseek-flash. Off-peak is half.
        m if m.starts_with("deepseek") => {
            if deepseek_peak(at) {
                (0.30, 0.006, 1.20)
            } else {
                (0.15, 0.003, 0.60)
            }
        }
        other => panic!("no prices for {other}"),
    }
}

/// DeepSeek peak: 01:00-04:00 and 06:00-10:00 UTC, Monday to Friday.
pub(crate) fn deepseek_peak(at: chrono::DateTime<Utc>) -> bool {
    let h = at.hour();
    !matches!(at.weekday(), Weekday::Sat | Weekday::Sun)
        && ((1..4).contains(&h) || (6..10).contains(&h))
}

#[derive(Serialize, Clone, Debug, Default)]
pub(crate) struct Call {
    pub ms: u128,
    pub status: u16,
    pub served_model: String,
    pub stop_reason: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
    /// DeepSeek at peak rates, for the monthly estimate's upper bound.
    pub cost_peak_usd: f64,
    #[serde(skip)]
    pub reply: String,
}

/// Spend shared by every job: settled cost plus reservations for calls in flight.
pub(crate) struct Budget {
    pub cap: f64,
    pub spent: Cell<f64>,
    pub reserved: Cell<f64>,
    pub exceeded: Cell<bool>,
    pub ledger: PathBuf,
}

impl Budget {
    /// A budget of `cap` USD that has already spent what `ledger` records.
    pub fn new(cap: f64, ledger: PathBuf) -> Self {
        Budget {
            cap,
            // `+ 0.0`: an empty ledger sums to -0.0, which prints as "$-0.0000".
            spent: Cell::new(read_ledger(&ledger) + 0.0),
            reserved: Cell::new(0.0),
            exceeded: Cell::new(false),
            ledger,
        }
    }
    pub fn reserve(&self, usd: f64) -> bool {
        if self.spent.get() + self.reserved.get() + usd > self.cap {
            self.exceeded.set(true);
            return false;
        }
        self.reserved.set(self.reserved.get() + usd);
        true
    }
    pub fn settle(&self, reserved: f64, call: &Call, model: &str, role: &str) {
        let actual = call.cost_usd;
        self.reserved.set(self.reserved.get() - reserved);
        self.spent.set(self.spent.get() + actual);
        let line = json!({"at": Utc::now().to_rfc3339(), "model": model, "role": role,
            "input_tokens": call.input_tokens, "cached_input_tokens": call.cached_input_tokens,
            "output_tokens": call.output_tokens, "usd": actual});
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.ledger)
            .expect("open spend ledger");
        writeln!(f, "{line}").expect("write spend ledger");
    }
}

pub(crate) fn read_ledger(path: &Path) -> f64 {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["usd"].as_f64())
        .sum()
}

/// The real HTTP client. `get` is never used: the trial makes no GET.
pub(crate) struct ReqwestHttp {
    pub client: reqwest::Client,
}

#[async_trait(?Send)]
impl Http for ReqwestHttp {
    async fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
        Err(ProviderError::Network(format!(
            "trial harness makes no GET ({url})"
        )))
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResp, ProviderError> {
        let mut req = self.client.post(url).json(body);
        // `.json` already sets Content-Type; a second one makes DeepSeek answer 415.
        for (k, v) in headers
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("content-type"))
        {
            req = req.header(*k, *v);
        }
        let r = req
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.without_url().to_string()))?;
        let status = r.status().as_u16();
        let text = r
            .text()
            .await
            .map_err(|e| ProviderError::Network(e.without_url().to_string()))?;
        Ok(HttpResp {
            status,
            body: text,
            set_cookie: None,
            content_type: None,
        })
    }
}

/// An HTTP client that enforces the spend cap and records every LLM call.
#[derive(Clone)]
pub(crate) struct RecordingHttp {
    pub inner: Rc<dyn Http>,
    pub model: String,
    /// "writer" or "judge", for the ledger.
    pub role: String,
    pub budget: Rc<Budget>,
    pub calls: Rc<RefCell<Vec<Call>>>,
}

impl RecordingHttp {
    pub fn new(inner: Rc<dyn Http>, model: &str, role: &str, budget: Rc<Budget>) -> Self {
        RecordingHttp {
            inner,
            model: model.to_string(),
            role: role.to_string(),
            budget,
            calls: Rc::new(RefCell::new(vec![])),
        }
    }

    /// Worst case for one call: the prompt at ~3 chars a token plus the
    /// longest reply the provider allows.
    fn reserve_for(&self, body: &Value) -> f64 {
        estimate_usd(
            &self.model,
            body.to_string().len(),
            max_output_tokens(&self.model),
        )
    }
}

/// The most output a call can bill: Claude's `MAX_TOKENS` (16,000, thinking
/// included) and DeepSeek's `max_tokens` (4,096).
pub(crate) fn max_output_tokens(model: &str) -> f64 {
    if model.starts_with("claude") {
        16_000.0
    } else {
        4_096.0
    }
}

/// USD for a call of `prompt_chars` request characters (at ~3 a token) and
/// `out_tokens` output tokens, at today's rates.
pub(crate) fn estimate_usd(model: &str, prompt_chars: usize, out_tokens: f64) -> f64 {
    let (inp, _, out) = prices(model, Utc::now());
    (prompt_chars as f64 / 3.0 * inp + out_tokens * out) / 1e6
}

/// Usage and reply text from a Claude or DeepSeek response body.
pub(crate) fn read_usage(model: &str, v: &Value, c: &mut Call) {
    c.served_model = v["model"].as_str().unwrap_or("").to_string();
    let u = &v["usage"];
    let n = |p: &str| u.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    if model.starts_with("claude") {
        c.stop_reason = v["stop_reason"].as_str().unwrap_or("").to_string();
        c.cached_input_tokens = n("/cache_read_input_tokens");
        c.input_tokens =
            n("/input_tokens") + n("/cache_creation_input_tokens") + c.cached_input_tokens;
        c.output_tokens = n("/output_tokens");
        c.reply = v["content"]
            .as_array()
            .and_then(|b| b.iter().find(|b| b["type"] == "text"))
            .and_then(|b| b["text"].as_str())
            .unwrap_or("")
            .to_string();
    } else {
        let ch = &v["choices"][0];
        c.stop_reason = ch["finish_reason"].as_str().unwrap_or("").to_string();
        c.input_tokens = n("/prompt_tokens");
        c.cached_input_tokens = n("/prompt_cache_hit_tokens");
        c.output_tokens = n("/completion_tokens");
        c.reasoning_tokens = n("/completion_tokens_details/reasoning_tokens");
        c.reply = ch
            .pointer("/message/content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
    }
}

pub(crate) fn price_call(model: &str, c: &Call, at: chrono::DateTime<Utc>) -> f64 {
    let (inp, cached, out) = prices(model, at);
    let uncached = c.input_tokens.saturating_sub(c.cached_input_tokens) as f64;
    (uncached * inp + c.cached_input_tokens as f64 * cached + c.output_tokens as f64 * out) / 1e6
}

#[async_trait(?Send)]
impl Http for RecordingHttp {
    async fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
        Err(ProviderError::Network(format!(
            "trial harness makes no GET ({url})"
        )))
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> Result<HttpResp, ProviderError> {
        let reserve = self.reserve_for(body);
        if !self.budget.reserve(reserve) {
            return Err(ProviderError::Network("trial budget cap reached".into()));
        }
        let started = Instant::now();
        let sent = self.inner.post_json(url, headers, body).await;
        let mut call = Call::default();
        if let Ok(r) = &sent {
            call.status = r.status;
            if let Ok(v) = serde_json::from_str::<Value>(&r.body) {
                read_usage(&self.model, &v, &mut call);
                if r.status >= 400 {
                    // Error type only, as the production provider logs it.
                    call.stop_reason = format!(
                        "http {} {}",
                        r.status,
                        v.pointer("/error/type")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                    );
                }
            }
        }
        call.ms = started.elapsed().as_millis();
        let now = Utc::now();
        call.cost_usd = price_call(&self.model, &call, now);
        call.cost_peak_usd = if self.model.starts_with("deepseek") {
            let peak = chrono::DateTime::parse_from_rfc3339("2026-11-06T02:00:00Z")
                .expect("valid")
                .with_timezone(&Utc);
            price_call(&self.model, &call, peak)
        } else {
            call.cost_usd
        };
        self.budget.settle(reserve, &call, &self.model, &self.role);
        if let Err(e) = &sent {
            call.stop_reason = format!("transport: {e}");
        }
        self.calls.borrow_mut().push(call);
        sent
    }
}

/// `KEY=value` lines (optionally quoted); only the two LLM keys are kept.
pub(crate) fn load_keys(dev_vars: Option<&str>) -> HashMap<String, String> {
    let wanted = ["ANTHROPIC_API_KEY", "DEEPSEEK_API_KEY"];
    let mut keys: HashMap<String, String> = wanted
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect();
    if let Some(p) = dev_vars {
        for line in fs::read_to_string(p).expect("read dev vars").lines() {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                if wanted.contains(&k) {
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    keys.insert(k.to_string(), v.to_string());
                }
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_peak_hours() {
        let at = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&Utc)
        };
        assert!(deepseek_peak(at("2026-11-06T02:30:00Z")));
        assert!(deepseek_peak(at("2026-11-06T09:59:00Z")));
        assert!(!deepseek_peak(at("2026-11-06T13:30:00Z")));
        assert!(!deepseek_peak(at("2026-11-07T02:30:00Z")));
    }

    #[test]
    fn usage_and_price() {
        let v = json!({"model": "claude-sonnet-5-5", "stop_reason": "end_turn",
            "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "{}"}],
            "usage": {"input_tokens": 1000, "cache_read_input_tokens": 0, "output_tokens": 500}});
        let mut c = Call::default();
        read_usage("claude-sonnet-5-5", &v, &mut c);
        assert_eq!(
            (c.input_tokens, c.output_tokens, c.reply.as_str()),
            (1000, 500, "{}")
        );
        assert!((price_call("claude-sonnet-5-5", &c, Utc::now()) - 0.007).abs() < 1e-9);
        let v = json!({"model": "deepseek-chat", "choices": [{"finish_reason": "stop",
            "message": {"content": "{}"}}], "usage": {"prompt_tokens": 1000,
            "prompt_cache_hit_tokens": 400, "completion_tokens": 100}});
        let mut c = Call::default();
        read_usage("deepseek-chat", &v, &mut c);
        let off = chrono::DateTime::parse_from_rfc3339("2026-11-06T13:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // 600 * 0.15 + 400 * 0.003 + 100 * 0.60, per 1M.
        assert!((price_call("deepseek-chat", &c, off) - 0.0001512).abs() < 1e-12);
    }

    #[test]
    fn dev_vars_keep_only_the_two_keys() {
        let dir = std::env::temp_dir().join(format!("insight-trial-keys-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("vars");
        fs::write(
            &p,
            "ANTHROPIC_API_KEY=\"a-test\"\nOTHER=x\nDEEPSEEK_API_KEY='d-test'\n",
        )
        .unwrap();
        let keys = load_keys(Some(p.to_str().unwrap()));
        assert_eq!(
            keys.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("a-test")
        );
        assert_eq!(
            keys.get("DEEPSEEK_API_KEY").map(String::as_str),
            Some("d-test")
        );
        assert!(!keys.contains_key("OTHER"));
        fs::remove_dir_all(dir).unwrap();
    }
}
