//! One insight step in WebAssembly, for timing under V8 (`run.mjs`). The
//! module exports `alloc`, `load` and `tick`: `load` reads a snapshot that
//! `insight-trial time --dump` wrote (the store before the step, the stored
//! replies), and each `tick` runs `insight::step::insight_tick` once from
//! that snapshot, with replies that answer at once. The caller times `tick`:
//! the first call in a fresh process pays V8's compile of the code it runs
//! and every pattern's compile, as a cold Worker isolate would.

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
}

/// Room for the caller to write `len` bytes into.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Reads the snapshot JSON at `ptr`; 1 when it parsed.
///
/// # Safety
/// `ptr` holds `len` bytes written by the caller into an `alloc` buffer.
#[no_mangle]
pub unsafe extern "C" fn load(ptr: *mut u8, len: usize) -> u32 {
    let bytes = std::slice::from_raw_parts(ptr, len);
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
        let r = ready(insight_tick(*now, store, w, j, &cfg));
        prebell_core::insight::compiled() as u32 + if r.is_ok() { 0 } else { 1_000_000 }
    })
}
