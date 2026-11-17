//! `time`: how much CPU the Worker's insight step uses, measured natively on
//! the trial's stored bundles with no model call. Each stage runs the real
//! `insight::step::insight_tick` over an in-memory store with the stored
//! replies (the final run's lines as the writer's, Claude's verdicts as the
//! judge's), so the time is the step's own work: reading the facts and
//! checks, the packs, the code checks, the judge-support confirmation and
//! the record's JSON. A stage's first call runs in a fresh process, as a
//! cold isolate would (every regular expression compiled on first use);
//! then it runs again warm.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use prebell_core::insight::step::{
    self, insight_tick, record_key, CheckRef, InsightStore, PostRef, Stage,
};
use prebell_core::insight::Insight;
use prebell_core::providers::{LlmProvider, ProviderError};
use serde_json::{json, Value};

use crate::Record;

/// Warm runs per stage; their median is reported.
pub(crate) const WARM_RUNS: usize = 20;

/// The step's store in memory.
#[derive(Default)]
pub(crate) struct Mem {
    pub posts: Vec<PostRef>,
    pub objects: RefCell<BTreeMap<String, String>>,
    pub calendar: Vec<(String, String, String)>,
    pub checks: Vec<CheckRef>,
    pub kv: RefCell<BTreeMap<String, String>>,
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

/// A model that answers at once with a stored reply.
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

/// One post's bundle and stored replies.
pub(crate) struct Case {
    pub store: Mem,
    pub facts_key: String,
    pub writer: Value,
    pub judge: Value,
    pub now: DateTime<Utc>,
}

fn fixed_time() -> DateTime<Utc> {
    "2026-11-13T12:00:00Z".parse().expect("time")
}

/// The case for the post `name` (`premarket_2026-11-05`, `recap_2026-11-12`,
/// `premarket_2026-11-09-update`) in `facts`, with its stored lines.
pub(crate) fn case(facts: &Path, name: &str, lines: &[Record]) -> Case {
    let (content_type, rest) = name.split_once('_').expect("post name");
    let date = rest.strip_suffix("-update").unwrap_or(rest);
    let facts_key = format!("{content_type}/{rest}.json");
    let mut store = Mem {
        posts: vec![PostRef {
            content_type: content_type.into(),
            trading_date: date.into(),
            slug: name.into(),
            facts_key: facts_key.clone(),
        }],
        ..Default::default()
    };
    let read = |f: &str| std::fs::read_to_string(facts.join(f)).expect("read facts");
    store
        .objects
        .borrow_mut()
        .insert(facts_key.clone(), read(&format!("{name}.json")));
    let calendar: Vec<(String, String, String)> =
        serde_json::from_str(&read("calendar.json")).expect("calendar.json");
    store.calendar = calendar;
    let mut names: Vec<String> = std::fs::read_dir(facts)
        .expect("facts dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("earnings_") && n.ends_with(".json"))
        .collect();
    names.sort();
    for n in names {
        let text = read(&n);
        let e: Value = serde_json::from_str(&text).expect("earnings bundle");
        let key = format!("earnings/{n}");
        store.checks.push(CheckRef {
            ticker: e["ticker"].as_str().unwrap_or("").into(),
            filing_date: e["filing_date"].as_str().unwrap_or("").into(),
            accepted_at: e["accepted_at"].as_str().unwrap_or("").into(),
            facts_key: key.clone(),
        });
        store.objects.borrow_mut().insert(key, text);
    }
    let mine: Vec<&Record> = lines.iter().filter(|r| r.post == name).collect();
    let insights: Vec<&Insight> = mine.iter().filter_map(|r| r.insight.as_ref()).collect();
    // Claude's stored verdicts. They predate judge v3's people flag, which
    // a missing value reads as "no": set it, so `passed` does its full work
    // instead of stopping at the flag.
    let verdicts: Vec<Value> = mine
        .iter()
        .filter_map(|r| r.judges.get("claude").and_then(|j| j.verdict.clone()))
        .map(|mut v| {
            v.personal_claims_attributed = true;
            serde_json::to_value(v).expect("verdict")
        })
        .collect();
    Case {
        store,
        facts_key,
        writer: json!({ "insights": insights }),
        judge: json!({ "verdicts": verdicts }),
        now: fixed_time(),
    }
}

/// Microseconds one tick takes.
async fn tick_us(c: &Case) -> (u128, String) {
    let cfg = step::config(None, None, None).expect("config");
    let (w, j) = (Canned(c.writer.clone()), Canned(c.judge.clone()));
    let t = Instant::now();
    let r = insight_tick(c.now, &c.store, &w, &j, &cfg).await;
    let us = t.elapsed().as_micros();
    (us, r.map(|t| t.line).unwrap_or_else(|e| e))
}

/// Puts the store back as it was before the write stage.
fn reset_write(c: &Case) {
    let mut o = c.store.objects.borrow_mut();
    o.retain(|k, _| !k.starts_with("insights/"));
    c.store.kv.borrow_mut().clear();
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// One stage of one post in this process: `(first call, warm median, line)`.
/// For the judge stage, the written record is made first by a write stage
/// run in another process (`written`), so this process starts cold.
pub(crate) async fn measure(c: &Case, stage: &str, written: Option<&str>) -> (u128, u128, String) {
    let key = record_key(&c.facts_key);
    let set_written = || {
        reset_write(c);
        if let Some(w) = written {
            c.store
                .objects
                .borrow_mut()
                .insert(key.clone(), w.to_string());
        }
    };
    if stage == "judge" {
        set_written();
    }
    let (first, line) = tick_us(c).await;
    let mut warm = Vec::new();
    for _ in 0..WARM_RUNS {
        match stage {
            "judge" => set_written(),
            _ => reset_write(c),
        }
        warm.push(tick_us(c).await.0);
    }
    (first, median(warm), line)
}

/// The written record a write stage leaves for `c` (after `measure`).
pub(crate) async fn written_record(c: &Case) -> Option<String> {
    reset_write(c);
    tick_us(c).await;
    let rec = c
        .store
        .objects
        .borrow()
        .get(&record_key(&c.facts_key))
        .cloned()?;
    let parsed: step::Record = serde_json::from_str(&rec).ok()?;
    (parsed.stage == Stage::Written).then_some(rec)
}

/// The post names in `facts` (briefs and recaps), sorted.
pub(crate) fn posts(facts: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(facts)
        .expect("facts dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| {
            (n.starts_with("premarket_") || n.starts_with("recap_")) && n.ends_with(".json")
        })
        .map(|n| n.trim_end_matches(".json").to_string())
        .collect();
    v.sort();
    v
}

/// One child process: `time --one STAGE --post NAME ...`, printing one
/// JSON line.
pub(crate) fn spawn(args: &[&str]) -> Value {
    let exe = std::env::current_exe().expect("current exe");
    let out = Command::new(exe).args(args).output().expect("spawn");
    if !out.status.success() {
        panic!(
            "child {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim()).expect("child output")
}

/// `(worst, median)` of `v`.
pub(crate) fn worst_median(v: &[u128]) -> (u128, u128) {
    (*v.iter().max().unwrap_or(&0), median(v.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recap of 2026-11-06 as the step's own test fixture stores it.
    fn fixture_case() -> Case {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/core/src/insight/testdata/shadow");
        let calendar = std::fs::read_to_string(dir.join("calendar.json")).unwrap();
        let tmp = crate::tests::tmp("timing");
        std::fs::create_dir_all(&tmp).unwrap();
        for f in [
            "recap_2026-11-06.json",
            "earnings_AMD_0000002488-26-000188.json",
            "earnings_CHTR_0001091667-26-000072.json",
        ] {
            std::fs::copy(dir.join(f), tmp.join(f)).unwrap();
        }
        std::fs::write(tmp.join("calendar.json"), calendar).unwrap();
        let replies: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("recap_2026-11-06.replies.json")).unwrap(),
        )
        .unwrap();
        let mut c = case(&tmp, "recap_2026-11-06", &[]);
        c.writer = replies["writer"].clone();
        c.judge = replies["judge"].clone();
        std::fs::remove_dir_all(tmp).unwrap();
        c
    }

    /// The timed stages are the real ones: a write stage that leaves a
    /// written record, then a judge stage that passes the three lines.
    #[tokio::test]
    async fn the_timed_ticks_run_both_stages() {
        let c = fixture_case();
        let (_, _, line) = measure(&c, "write", None).await;
        assert!(
            line.ends_with("3 passed the checks; judge queued"),
            "{line}"
        );
        let w = written_record(&c).await.expect("a written record");
        let (_, _, line) = measure(&c, "judge", Some(&w)).await;
        assert!(line.ends_with("judged 3 lines, 3 passed; done"), "{line}");
        assert_eq!(worst_median(&[5, 1, 3]), (5, 3));
    }
}
