//! One insight step in WebAssembly, for timing under V8 (`run.mjs`). The
//! module exports `alloc`, `load` and `tick`: `load` reads a snapshot that
//! `insight-trial time --dump` wrote (the store before the step, the stored
//! replies), and each `tick` runs `insight::step::insight_tick` once from
//! that snapshot, with replies that answer at once. The caller times `tick`:
//! the first call in a fresh process pays V8's compile of the code it runs
//! and every pattern's compile, as a cold Worker isolate would.
//!
//! Calibration, timed the same way, from paths that run in production
//! today: `card` (a share card from a stored cover, `cover::card`), `draft`
//! (the number check a recap build runs on its draft, `numbers::check`
//! against the facts' numbers) and `noop` (the fixed cost of a first call).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use prebell_core::insight::step::{insight_tick, CheckRef, Config, InsightStore, PostRef};
use prebell_core::providers::{LlmProvider, ProviderError};
use serde::Deserialize;
use serde_json::Value;

/// What `insight-trial time --dump` writes for one step of one post.
#[derive(Deserialize, Clone)]
struct Snapshot {
    posts: Vec<(String, String, String, String)>,
    objects: BTreeMap<String, String>,
    calendar: Vec<(String, String, String)>,
    checks: Vec<(String, String, String, String)>,
    kv: BTreeMap<String, String>,
    writer: Value,
    judge: Value,
    now: String,
}

struct Mem {
    posts: Vec<PostRef>,
    objects: RefCell<BTreeMap<String, String>>,
    calendar: Vec<(String, String, String)>,
    checks: Vec<CheckRef>,
    kv: RefCell<BTreeMap<String, String>>,
}

#[async_trait(?Send)]
impl InsightStore for Mem {
    async fn insight_posts(&self, _since: &str) -> Result<Vec<PostRef>, String> {
        Ok(self.posts.clone())
    }
    async fn get_text(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.objects.borrow().get(key).cloned())
    }
    async fn put_text(&self, key: &str, text: &str) -> Result<(), String> {
        self.objects.borrow_mut().insert(key.into(), text.into());
        Ok(())
    }
    async fn calendar_rows(
        &self,
        tickers: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<(String, String, String)>, String> {
        Ok(self
            .calendar
            .iter()
            .filter(|(t, d, _)| tickers.contains(t) && (from..=to).contains(&d.as_str()))
            .cloned()
            .collect())
    }
    async fn earnings_checks(
        &self,
        tickers: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<CheckRef>, String> {
        Ok(self
            .checks
            .iter()
            .filter(|c| {
                tickers.contains(&c.ticker) && (from..=to).contains(&c.filing_date.as_str())
            })
            .cloned()
            .collect())
    }
    async fn kv_get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.kv.borrow().get(key).cloned())
    }
    async fn kv_claim(&self, key: &str, cap: u32, _now: &str) -> Result<Option<u32>, String> {
        let n: u32 = self
            .kv
            .borrow()
            .get(key)
            .map_or(0, |v| v.parse().unwrap_or(cap));
        if n >= cap {
            return Ok(None);
        }
        self.kv.borrow_mut().insert(key.into(), (n + 1).to_string());
        Ok(Some(n + 1))
    }
}

struct Canned(Value);

#[async_trait(?Send)]
impl LlmProvider for Canned {
    async fn complete_json(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
        Ok(self.0.clone())
    }
    async fn complete_json_strict(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
        Ok(self.0.clone())
    }
}

thread_local! {
    static SNAPSHOT: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
    /// What `load` was given, as text: a cover SVG for `card`.
    static RAW: RefCell<String> = const { RefCell::new(String::new()) };
    /// `(allowed numbers, draft body)` for `draft`.
    static DRAFT: RefCell<Option<(Vec<f64>, String)>> = const { RefCell::new(None) };
}

/// Returns at once: the fixed cost of a first call into the module.
#[no_mangle]
pub extern "C" fn noop() -> u32 {
    0
}

/// A share card from the loaded cover SVG, as the `cards` job renders it:
/// the PNG's size in bytes, or 0 when it failed.
#[no_mangle]
pub extern "C" fn card() -> u32 {
    RAW.with(|r| {
        let cover = r.borrow();
        prebell_core::cover::card::card_svg(&cover, "2026-11-06")
            .and_then(|svg| prebell_core::cover::card::render_png(&svg).ok())
            .map_or(0, |png| png.len() as u32)
    })
}

/// Reads the loaded `{"facts": <recap facts>, "body": <draft>}` outside the
/// timed call: the numbers the facts allow, and the draft.
#[no_mangle]
pub extern "C" fn prepare_draft() -> u32 {
    let raw = RAW.with(|r| r.borrow().clone());
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return 0;
    };
    let Ok(facts) = serde_json::from_value::<prebell_core::recap::RecapFacts>(v["facts"].clone())
    else {
        return 0;
    };
    let body = v["body"].as_str().unwrap_or("").to_string();
    DRAFT.with(|d| *d.borrow_mut() = Some((prebell_core::recap::numbers_in_facts(&facts), body)));
    1
}

/// The number check a recap build runs on its draft: the unbacked numbers
/// found (0 when every number traces to the facts).
#[no_mangle]
pub extern "C" fn draft() -> u32 {
    DRAFT.with(|d| {
        let d = d.borrow();
        let (allowed, body) = d.as_ref().expect("prepare_draft first");
        match prebell_core::numbers::check(body, prebell_core::numbers::Lang::En, allowed) {
            Ok(()) => 0,
            Err(bad) => bad.len() as u32,
        }
    })
}

/// Room for the caller to write `len` bytes into.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Reads the input at `ptr`: kept as text, and parsed as a step snapshot
/// when it is one; 1 when it parsed as a snapshot.
///
/// # Safety
/// `ptr` holds `len` bytes written by the caller into an `alloc` buffer.
#[no_mangle]
pub unsafe extern "C" fn load(ptr: *mut u8, len: usize) -> u32 {
    let bytes = std::slice::from_raw_parts(ptr, len);
    RAW.with(|r| *r.borrow_mut() = String::from_utf8_lossy(bytes).into_owned());
    match serde_json::from_slice::<Snapshot>(bytes) {
        Ok(s) => {
            SNAPSHOT.with(|x| *x.borrow_mut() = Some(s));
            1
        }
        Err(_) => 0,
    }
}

/// Runs a future whose every await is ready (the store and the replies
/// answer at once).
fn ready<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("a step awaited something that was not ready"),
    }
}

thread_local! {
    static PREPARED: RefCell<Option<(Mem, Canned, Canned, chrono::DateTime<chrono::Utc>)>> =
        const { RefCell::new(None) };
}

/// Builds the store and the replies from the loaded snapshot, outside the
/// timed call.
#[no_mangle]
pub extern "C" fn prepare() {
    let s = SNAPSHOT.with(|x| x.borrow().clone()).expect("load first");
    let store = Mem {
        posts: s
            .posts
            .iter()
            .map(|(c, d, slug, k)| PostRef {
                content_type: c.clone(),
                trading_date: d.clone(),
                slug: slug.clone(),
                facts_key: k.clone(),
            })
            .collect(),
        objects: RefCell::new(s.objects.clone()),
        calendar: s.calendar.clone(),
        checks: s
            .checks
            .iter()
            .map(|(t, f, a, k)| CheckRef {
                ticker: t.clone(),
                filing_date: f.clone(),
                accepted_at: a.clone(),
                facts_key: k.clone(),
            })
            .collect(),
        kv: RefCell::new(s.kv.clone()),
    };
    let now = s.now.parse().expect("now");
    PREPARED.with(|p| {
        *p.borrow_mut() = Some((
            store,
            Canned(s.writer.clone()),
            Canned(s.judge.clone()),
            now,
        ))
    });
}

/// One tick over the prepared store (call `prepare` before each). Returns
/// the patterns the insight code has compiled in this instance so far, plus
/// 1_000_000 when the tick returned an error.
#[no_mangle]
pub extern "C" fn tick() -> u32 {
    PREPARED.with(|p| {
        let p = p.borrow();
        let (store, w, j, now) = p.as_ref().expect("prepare first");
        let cfg: Config = prebell_core::insight::step::config(None, None, None).expect("config");
        let r = ready(insight_tick(*now, store, w, j, &cfg, false));
        prebell_core::insight::compiled() as u32 + if r.is_ok() { 0 } else { 1_000_000 }
    })
}
