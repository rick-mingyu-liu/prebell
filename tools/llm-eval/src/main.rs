//! Claude vs DeepSeek on saved fact bundles.
//!
//! For every fact bundle in `--facts` and every model in `--models`, drafts EN
//! and ZH with the production `write` functions (prompt, parse, number check
//! and the in-build retry, unchanged) through the production providers
//! (`llm::llm_from_env`). Only the HTTP layer is ours: it times each call,
//! reads the token usage from the response, prices it and enforces a hard
//! spend cap. A draft that fails its build is rebuilt up to `--builds` times,
//! as later ticks would in production.
//!
//! Keys come from `--dev-vars` (or the environment) and stay in memory: no
//! request header is ever logged or written.
//!
//! ```text
//! cargo run --release -- --facts DIR --out DIR --dev-vars PATH \
//!     [--models deepseek-chat,claude-sonnet-5-5,claude-opus-5-5] \
//!     [--budget 6.0] [--builds 3] [--only SUBSTR] [--jobs 6] [--effort low] [--ledger PATH]
//! ```
//!
//! Fact bundles are the R2 objects named by `posts.facts_r2_key`, saved as
//! `<key with / replaced by _>` (e.g. `earnings_SPG_0001104659-26-123325.json`):
//! `wrangler r2 object get prebell-facts/<key> --remote --file <path>`. The
//! file name's prefix (earnings_, premarket_, recap_) picks the content type.
//! `analyze.py` turns the results into the report's tables.
//!
//! Writes `results.jsonl` (one line per post x model x language), each final
//! draft as markdown under `out/<model>/`, and appends every call's cost to
//! the ledger (default `out/spend.jsonl`), which later runs read back so the
//! cap spans runs.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{Datelike, Timelike, Utc, Weekday};
use futures::stream::{self, StreamExt};
use prebell_core::earnings::facts::EarningsFacts;
use prebell_core::facts::PremarketFacts;
use prebell_core::llm::llm_from_env;
use prebell_core::providers::{Http, HttpResp, LlmProvider, ProviderError};
use prebell_core::recap::RecapFacts;
use prebell_core::Lang;
use serde::Serialize;
use serde_json::{json, Value};

/// USD per 1M tokens: (uncached input, cached input, output).
fn prices(model: &str, at: chrono::DateTime<Utc>) -> (f64, f64, f64) {
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
fn deepseek_peak(at: chrono::DateTime<Utc>) -> bool {
    let h = at.hour();
    !matches!(at.weekday(), Weekday::Sat | Weekday::Sun)
        && ((1..4).contains(&h) || (6..10).contains(&h))
}

#[derive(Serialize, Clone, Debug, Default)]
struct Call {
    ms: u128,
    status: u16,
    served_model: String,
    stop_reason: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cost_usd: f64,
    /// DeepSeek at peak rates, for the monthly estimate's upper bound.
    cost_peak_usd: f64,
    #[serde(skip)]
    user_prompt: String,
    #[serde(skip)]
    reply: String,
}

/// Spend shared by every job: settled cost plus reservations for calls in flight.
struct Budget {
    cap: f64,
    spent: Cell<f64>,
    reserved: Cell<f64>,
    exceeded: Cell<bool>,
    ledger: PathBuf,
}

impl Budget {
    fn reserve(&self, usd: f64) -> bool {
        if self.spent.get() + self.reserved.get() + usd > self.cap {
            self.exceeded.set(true);
            return false;
        }
        self.reserved.set(self.reserved.get() + usd);
        true
    }
    fn settle(&self, reserved: f64, actual: f64, model: &str) {
        self.reserved.set(self.reserved.get() - reserved);
        self.spent.set(self.spent.get() + actual);
        let line = json!({"at": Utc::now().to_rfc3339(), "model": model, "usd": actual});
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.ledger)
            .expect("open spend ledger");
        writeln!(f, "{line}").expect("write spend ledger");
    }
}

fn read_ledger(path: &Path) -> f64 {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["usd"].as_f64())
        .sum()
}

/// A real HTTP client that records every LLM call of one job.
#[derive(Clone)]
struct RecordingHttp {
    client: reqwest::Client,
    model: String,
    budget: Rc<Budget>,
    calls: Rc<RefCell<Vec<Call>>>,
}

impl RecordingHttp {
    /// Worst case for one call: the prompt at ~3 chars a token plus a long reply.
    fn reserve_for(&self, body: &Value) -> f64 {
        let (inp, _, out) = prices(&self.model, Utc::now());
        let in_tok = body.to_string().len() as f64 / 3.0;
        let out_tok = if self.model.starts_with("claude") {
            8000.0
        } else {
            4096.0
        };
        (in_tok * inp + out_tok * out) / 1e6
    }
}

fn user_prompt(body: &Value) -> String {
    let msgs = body["messages"].as_array().cloned().unwrap_or_default();
    msgs.iter()
        .rev()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Usage and reply text from a Claude or DeepSeek response body.
fn read_usage(model: &str, v: &Value, c: &mut Call) {
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

fn price_call(model: &str, c: &Call, at: chrono::DateTime<Utc>) -> f64 {
    let (inp, cached, out) = prices(model, at);
    let uncached = c.input_tokens.saturating_sub(c.cached_input_tokens) as f64;
    (uncached * inp + c.cached_input_tokens as f64 * cached + c.output_tokens as f64 * out) / 1e6
}

#[async_trait(?Send)]
impl Http for RecordingHttp {
    async fn get(&self, url: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
        Err(ProviderError::Network(format!(
            "eval harness makes no GET ({url})"
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
            return Err(ProviderError::Network("eval budget cap reached".into()));
        }
        let mut req = self.client.post(url).json(body);
        // `.json` already sets Content-Type; a second one makes DeepSeek answer 415.
        for (k, v) in headers
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("content-type"))
        {
            req = req.header(*k, *v);
        }
        let started = Instant::now();
        let sent = req.send().await;
        let mut call = Call {
            user_prompt: user_prompt(body),
            ..Call::default()
        };
        let result = match sent {
            Err(e) => Err(ProviderError::Network(e.without_url().to_string())),
            Ok(r) => {
                let status = r.status().as_u16();
                match r.text().await {
                    Err(e) => Err(ProviderError::Network(e.without_url().to_string())),
                    Ok(text) => {
                        call.status = status;
                        if let Ok(v) = serde_json::from_str::<Value>(&text) {
                            read_usage(&self.model, &v, &mut call);
                            if status >= 400 {
                                // Error type only, as the production provider logs it.
                                call.stop_reason = format!(
                                    "http {status} {}",
                                    v.pointer("/error/type")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                );
                            }
                        }
                        Ok(HttpResp {
                            status,
                            body: text,
                            set_cookie: None,
                        })
                    }
                }
            }
        };
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
        self.budget.settle(reserve, call.cost_usd, &self.model);
        if let Err(e) = &result {
            call.stop_reason = format!("transport: {e}");
        }
        self.calls.borrow_mut().push(call);
        result
    }
}

#[allow(clippy::large_enum_variant)] // three values per run, never moved in bulk
enum Facts {
    Earnings(EarningsFacts),
    Premarket(PremarketFacts),
    Recap(RecapFacts),
}

impl Facts {
    fn load(path: &Path) -> (String, Facts) {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let json = fs::read_to_string(path).expect("read facts");
        fn parse<T: serde::de::DeserializeOwned>(name: &str, json: &str) -> T {
            serde_json::from_str(json).unwrap_or_else(|e| panic!("{name}: {e}"))
        }
        let f = if name.starts_with("earnings_") {
            Facts::Earnings(parse(&name, &json))
        } else if name.starts_with("premarket_") {
            Facts::Premarket(parse(&name, &json))
        } else if name.starts_with("recap_") {
            Facts::Recap(parse(&name, &json))
        } else {
            panic!("unknown facts kind: {name}")
        };
        (name, f)
    }

    fn kind(&self) -> &'static str {
        match self {
            Facts::Earnings(_) => "earnings",
            Facts::Premarket(_) => "premarket",
            Facts::Recap(_) => "recap",
        }
    }

    async fn write(&self, llm: &dyn LlmProvider, lang: Lang) -> Result<String, Vec<String>> {
        let r = match self {
            Facts::Earnings(f) => prebell_core::earnings::draft::write(llm, f, lang).await,
            Facts::Premarket(f) => prebell_core::premarket::draft::write(llm, f, lang).await,
            Facts::Recap(f) => prebell_core::recap::draft::write(llm, f, lang).await,
        };
        r.map(|(_, md)| md)
    }
}

#[derive(Serialize)]
struct Attempt {
    build: usize,
    #[serde(flatten)]
    call: Call,
    /// "ok", or why the verifier (or the provider) rejected this attempt.
    outcome: String,
}

#[derive(Serialize)]
struct JobResult {
    post: String,
    kind: &'static str,
    model: String,
    lang: &'static str,
    published: bool,
    first_pass: bool,
    builds: usize,
    calls: usize,
    total_ms: u128,
    cost_usd: f64,
    cost_peak_usd: f64,
    attempts: Vec<Attempt>,
    final_errors: Vec<String>,
    budget_stop: bool,
}

const RETRY_MARK: &str = "Fix them and reply again:\n- ";

/// Why attempt `i` of a build failed: the retry note the next attempt carried,
/// or, for the build's last attempt, the errors `write` returned.
fn outcomes(calls: &[Call], result: &Result<String, Vec<String>>) -> Vec<String> {
    (0..calls.len())
        .map(|i| match calls.get(i + 1) {
            Some(next) => next
                .user_prompt
                .split(RETRY_MARK)
                .nth(1)
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "failed (no retry note)".into()),
            None => match result {
                Ok(_) => "ok".into(),
                Err(e) => e.join("; "),
            },
        })
        .collect()
}

fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Zh => "zh",
    }
}

fn warn(m: &str) {
    eprintln!("warn: {m}");
}

struct Cfg {
    keys: HashMap<String, String>,
    budget: Rc<Budget>,
    client: reqwest::Client,
    builds: usize,
    effort: Option<String>,
    out: PathBuf,
}

async fn run_job(cfg: &Cfg, post: &str, facts: &Facts, model: &str, lang: Lang) -> JobResult {
    let provider = if model.starts_with("claude") {
        "claude"
    } else {
        "deepseek"
    };
    let key_var = prebell_core::llm::api_key_var(provider).expect("known provider");
    let http = RecordingHttp {
        client: cfg.client.clone(),
        model: model.to_string(),
        budget: cfg.budget.clone(),
        calls: Rc::new(RefCell::new(vec![])),
    };
    let llm = llm_from_env(
        provider,
        http.clone(),
        cfg.keys.get(key_var).cloned().unwrap_or_default(),
        Some(model.to_string()),
        cfg.effort.clone(),
        warn,
    )
    .expect("provider config");
    let mut attempts = vec![];
    let mut final_errors = vec![];
    let mut published = false;
    let mut builds = 0;
    for build in 1..=cfg.builds {
        builds = build;
        let n0 = http.calls.borrow().len();
        let r = facts.write(llm.as_ref(), lang).await;
        let calls: Vec<Call> = http.calls.borrow()[n0..].to_vec();
        for (call, outcome) in calls.iter().zip(outcomes(&calls, &r)) {
            attempts.push(Attempt {
                build,
                call: call.clone(),
                outcome,
            });
        }
        match r {
            Ok(md) => {
                let dir = cfg.out.join(model);
                fs::create_dir_all(&dir).expect("out dir");
                fs::write(dir.join(format!("{post}.{}.md", lang_code(lang))), md)
                    .expect("write md");
                published = true;
                break;
            }
            Err(e) => {
                // Keep the rejected reply for the failure analysis.
                if let Some(last) = calls.last() {
                    let dir = cfg.out.join(model).join("rejected");
                    fs::create_dir_all(&dir).expect("out dir");
                    let p = dir.join(format!("{post}.{}.b{build}.json", lang_code(lang)));
                    fs::write(p, &last.reply).expect("write rejected");
                }
                final_errors = e;
                if cfg.budget.exceeded.get() {
                    break;
                }
            }
        }
    }
    let first_pass = attempts.first().is_some_and(|a| a.outcome == "ok");
    JobResult {
        post: post.to_string(),
        kind: facts.kind(),
        model: model.to_string(),
        lang: lang_code(lang),
        published,
        first_pass,
        builds,
        calls: attempts.len(),
        total_ms: attempts.iter().map(|a| a.call.ms).sum(),
        cost_usd: attempts.iter().map(|a| a.call.cost_usd).sum(),
        cost_peak_usd: attempts.iter().map(|a| a.call.cost_peak_usd).sum(),
        final_errors: if published { vec![] } else { final_errors },
        budget_stop: cfg.budget.exceeded.get(),
        attempts,
    }
}

/// `KEY=value` lines (optionally quoted); only the two LLM keys are kept.
fn load_keys(dev_vars: Option<&str>) -> HashMap<String, String> {
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

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let facts_dir = PathBuf::from(arg(&args, "--facts").expect("--facts DIR"));
    let out = PathBuf::from(arg(&args, "--out").expect("--out DIR"));
    let models: Vec<String> = arg(&args, "--models")
        .unwrap_or_else(|| "deepseek-chat,claude-sonnet-5-5,claude-opus-5-5".into())
        .split(',')
        .map(String::from)
        .collect();
    let cap: f64 = arg(&args, "--budget").map_or(6.0, |v| v.parse().expect("--budget USD"));
    let builds: usize = arg(&args, "--builds").map_or(3, |v| v.parse().expect("--builds N"));
    let jobs: usize = arg(&args, "--jobs").map_or(6, |v| v.parse().expect("--jobs N"));
    let only = arg(&args, "--only");
    fs::create_dir_all(&out).expect("out dir");

    let ledger = arg(&args, "--ledger").map_or_else(|| out.join("spend.jsonl"), PathBuf::from);
    let budget = Rc::new(Budget {
        cap,
        spent: Cell::new(read_ledger(&ledger)),
        reserved: Cell::new(0.0),
        exceeded: Cell::new(false),
        ledger,
    });
    eprintln!(
        "spent before this run: ${:.4} of ${cap:.2}",
        budget.spent.get()
    );

    let cfg = Cfg {
        keys: load_keys(arg(&args, "--dev-vars").as_deref()),
        budget: budget.clone(),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("http client"),
        builds,
        effort: arg(&args, "--effort"),
        out: out.clone(),
    };
    for m in &models {
        let var = if m.starts_with("claude") {
            "ANTHROPIC_API_KEY"
        } else {
            "DEEPSEEK_API_KEY"
        };
        assert!(cfg.keys.contains_key(var), "{var} not set for {m}");
    }

    let mut paths: Vec<PathBuf> = fs::read_dir(&facts_dir)
        .expect("facts dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| {
            only.as_ref()
                .is_none_or(|o| p.to_string_lossy().contains(o.as_str()))
        })
        .collect();
    paths.sort();
    let posts: Vec<(String, Facts)> = paths.iter().map(|p| Facts::load(p)).collect();

    // Interleave models so a budget stop cuts every arm, not only the last one.
    let mut todo = vec![];
    for (post, f) in &posts {
        for lang in [Lang::En, Lang::Zh] {
            for m in &models {
                todo.push((post.as_str(), f, m.as_str(), lang));
            }
        }
    }
    eprintln!("{} jobs", todo.len());
    let results_path = out.join("results.jsonl");
    let cfg = &cfg;
    let mut done = stream::iter(todo)
        .map(|(post, f, m, lang)| run_job(cfg, post, f, m, lang))
        .buffer_unordered(jobs);
    let mut n = 0;
    while let Some(r) = done.next().await {
        n += 1;
        eprintln!(
            "[{n}] {} {} {}: published={} first_pass={} calls={} {:.1}s ${:.4} (total ${:.4})",
            r.post,
            r.model,
            r.lang,
            r.published,
            r.first_pass,
            r.calls,
            r.total_ms as f64 / 1000.0,
            r.cost_usd,
            budget.spent.get()
        );
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&results_path)
            .expect("results");
        writeln!(f, "{}", serde_json::to_string(&r).expect("json")).expect("write results");
    }
    eprintln!(
        "spent: ${:.4} of ${cap:.2}{}",
        budget.spent.get(),
        if budget.exceeded.get() {
            " (budget cap reached: some jobs stopped)"
        } else {
            ""
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_note_becomes_previous_attempt_outcome() {
        let call = |prompt: &str| Call {
            user_prompt: prompt.into(),
            ..Call::default()
        };
        let calls = [
            call("facts"),
            call("facts\n\nYour previous reply had problems. Fix them and reply again:\n- These numbers are not in the facts: 4.12 亿\n"),
        ];
        assert_eq!(
            outcomes(&calls, &Ok("md".into())),
            ["These numbers are not in the facts: 4.12 亿", "ok"]
        );
        assert_eq!(
            outcomes(&calls, &Err(vec!["a".into(), "b".into()])),
            ["These numbers are not in the facts: 4.12 亿", "a; b"]
        );
    }

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
}
