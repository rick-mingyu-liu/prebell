//! Mover insights, offline trial (Plan 1 of the mover-insights spec).
//!
//! `run` reads saved fact bundles, builds each brief's and recap's evidence
//! packs, makes one writer call per post (Claude), runs the code checks, then
//! one judge call per judge (Claude and DeepSeek) over the lines that passed
//! them. Everything goes through the production code in `prebell_core::insight`
//! and the production providers (`llm::llm_from_env`); only the HTTP layer is
//! ours (`http.rs`): it prices each call and enforces a hard spend cap that
//! spans runs (the ledger is `OUT/spend.jsonl`).
//!
//! `seed` makes deliberately wrong variants of the lines that passed the
//! checks and asks each judge about them; `page` writes the labelling page;
//! `score` reads the owner's labels back and writes the judge report.
//!
//! ```text
//! cargo run --release -- run --facts DIR --out DIR --dev-vars PATH [--budget 3.0] [--only SUBSTR]
//! cargo run --release -- seed --out DIR --dev-vars PATH [--budget 3.0]
//! cargo run --release -- page --out DIR [--cards 60]
//! cargo run --release -- score --out DIR --labels FILE
//! cargo run --release -- events-json ROWS.json > events.json
//! ```
//!
//! Keys come from `--dev-vars` (or the environment) and stay in memory: no
//! request header or key is ever printed or written.

mod http;
mod page;
mod score;
mod seed;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::NaiveDate;
use prebell_core::cover::Session;
use prebell_core::earnings::facts::EarningsFacts;
use prebell_core::facts::{Mover, PremarketFacts, Theme};
use prebell_core::insight::judge::{self, Verdict, JUDGE_VERSION};
use prebell_core::insight::pack::{packs, PackInputs};
use prebell_core::insight::write::{self, WRITER_VERSION};
use prebell_core::insight::{check, Insight, Pack};
use prebell_core::llm::{api_key_var, llm_from_env};
use prebell_core::providers::{Http, LlmProvider};
use prebell_core::recap::RecapFacts;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use http::{Budget, RecordingHttp, ReqwestHttp};

/// The writer is always Claude.
const CLAUDE_MODEL: &str = "claude-sonnet-5-5";
/// The DeepSeek provider's default model (`llm::llm_from_env` with no model),
/// named here only for the price table and the ledger.
const DEEPSEEK_MODEL: &str = "deepseek-flash";
/// The judges compared, in report order.
pub(crate) const JUDGES: [&str; 2] = ["claude", "deepseek"];

#[allow(clippy::large_enum_variant)] // a few dozen values per run, never moved in bulk
pub(crate) enum Post {
    Premarket(PremarketFacts),
    Recap(RecapFacts),
}

impl Post {
    fn session(&self) -> Session {
        match self {
            Post::Premarket(_) => Session::Premarket,
            Post::Recap(_) => Session::Recap,
        }
    }
    fn trading_date(&self) -> NaiveDate {
        match self {
            Post::Premarket(f) => f.trading_date,
            Post::Recap(f) => f.trading_date,
        }
    }
    /// Gainers then losers, as listed.
    fn movers(&self) -> Vec<Mover> {
        let (g, l) = match self {
            Post::Premarket(f) => (&f.gainers, &f.losers),
            Post::Recap(f) => (&f.gainers, &f.losers),
        };
        g.iter().chain(l).cloned().collect()
    }
    fn themes(&self) -> &[Theme] {
        match self {
            Post::Premarket(f) => &f.themes,
            Post::Recap(f) => &f.themes,
        }
    }
}

/// Everything `--facts` holds.
#[derive(Default)]
pub(crate) struct Inputs {
    /// (file stem, facts), by file name.
    pub posts: Vec<(String, Post)>,
    pub checks: Vec<EarningsFacts>,
    pub calendar: Vec<(String, NaiveDate)>,
    pub macro_events: Vec<(NaiveDate, String)>,
}

impl Inputs {
    /// The eligible movers' packs for one post, and how many movers it lists.
    pub fn packs(&self, post: &Post) -> (Vec<Pack>, usize) {
        let movers = post.movers();
        let p = packs(&PackInputs {
            session: post.session(),
            trading_date: post.trading_date(),
            movers: &movers,
            themes: post.themes(),
            checks: &self.checks,
            calendar: &self.calendar,
            macro_events: &self.macro_events,
        });
        (p, movers.len())
    }
}

fn date(s: &str) -> NaiveDate {
    s.parse().unwrap_or_else(|e| panic!("bad date {s:?}: {e}"))
}

fn parse<T: serde::de::DeserializeOwned>(name: &str, json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Reads `dir` as `fetch-facts.sh` saves it. `only` keeps the posts whose
/// file name contains it; earnings checks, the calendar and events are
/// always read.
pub(crate) fn load_inputs(dir: &Path, only: Option<&str>) -> Inputs {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut i = Inputs::default();
    for p in paths {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let json = fs::read_to_string(&p).expect("read facts");
        if name == "calendar" {
            let rows: Vec<(String, String)> = parse(&name, &json);
            i.calendar = rows.into_iter().map(|(t, d)| (t, date(&d))).collect();
        } else if name == "events" {
            let rows: Vec<(String, String)> = parse(&name, &json);
            i.macro_events = rows.into_iter().map(|(d, n)| (date(&d), n)).collect();
        } else if name.starts_with("earnings_") {
            i.checks.push(parse(&name, &json));
        } else if name.starts_with("premarket_") || name.starts_with("recap_") {
            if only.is_some_and(|o| !name.contains(o)) {
                continue;
            }
            let post = if name.starts_with("premarket_") {
                Post::Premarket(parse(&name, &json))
            } else {
                Post::Recap(parse(&name, &json))
            };
            i.posts.push((name, post));
        } else {
            eprintln!("skipping {}: not a facts bundle", p.display());
        }
    }
    i
}

/// The writer and the judges, each through its own recording HTTP layer.
pub(crate) struct Models {
    pub writer: Box<dyn LlmProvider>,
    pub judges: Vec<(String, Box<dyn LlmProvider>)>,
    pub budget: Rc<Budget>,
}

fn warn(m: &str) {
    eprintln!("warn: {m}");
}

/// `provider` ("claude" or "deepseek") for `role`, through `inner`.
pub(crate) fn provider(
    provider: &str,
    role: &str,
    inner: Rc<dyn Http>,
    budget: &Rc<Budget>,
    keys: &HashMap<String, String>,
) -> Box<dyn LlmProvider> {
    let (model, pinned) = match provider {
        "claude" => (CLAUDE_MODEL, Some(CLAUDE_MODEL.to_string())),
        _ => (DEEPSEEK_MODEL, None),
    };
    let http = RecordingHttp::new(inner, model, role, budget.clone());
    let key = api_key_var(provider)
        .and_then(|v| keys.get(v))
        .cloned()
        .unwrap_or_default();
    llm_from_env(provider, http, key, pinned, Some("low".into()), warn).expect("provider config")
}

pub(crate) fn models(
    inner: Rc<dyn Http>,
    budget: Rc<Budget>,
    keys: &HashMap<String, String>,
) -> Models {
    Models {
        writer: provider("claude", "writer", inner.clone(), &budget, keys),
        judges: JUDGES
            .iter()
            .map(|j| {
                (
                    j.to_string(),
                    provider(j, "judge", inner.clone(), &budget, keys),
                )
            })
            .collect(),
        budget,
    }
}

/// One judge's answer on one mover.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct JudgeResult {
    pub verdict: Option<Verdict>,
    pub passed: bool,
    /// The judge call failed (not the budget: a budget stop ends the run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One line of `lines.jsonl`: one mover that had a pack.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Record {
    pub post: String,
    pub ticker: String,
    pub pack: Pack,
    pub insight: Option<Insight>,
    pub write_error: Option<String>,
    pub check: Vec<String>,
    pub judges: BTreeMap<String, JudgeResult>,
    pub published_if: BTreeMap<String, bool>,
    pub writer_version: String,
    pub judge_version: String,
}

/// The run stopped because the next call would pass the cap.
#[derive(Debug)]
pub(crate) struct BudgetStop;

pub(crate) fn stop_message(b: &Budget) -> String {
    format!(
        "budget cap reached: spent ${:.4} of ${:.2}; results so far are kept",
        b.spent.get(),
        b.cap
    )
}

/// Every judge's verdicts on `insights`, or why its call failed.
pub(crate) async fn judge_all(
    m: &Models,
    packs: &[Pack],
    insights: &[Insight],
) -> Result<BTreeMap<String, Result<Vec<Verdict>, String>>, BudgetStop> {
    let mut out = BTreeMap::new();
    for (name, llm) in &m.judges {
        let r = judge::judge(llm.as_ref(), packs, insights).await;
        if r.is_err() && m.budget.exceeded.get() {
            return Err(BudgetStop);
        }
        out.insert(name.clone(), r.map_err(|e| e.to_string()));
    }
    Ok(out)
}

/// The answer of one judge on one mover from `judge_all`'s result.
pub(crate) fn judge_result(
    r: &Result<Vec<Verdict>, String>,
    pack: &Pack,
    sent: bool,
) -> JudgeResult {
    match r {
        Err(e) if sent => JudgeResult {
            verdict: None,
            passed: false,
            error: Some(e.clone()),
        },
        Err(_) => JudgeResult {
            verdict: None,
            passed: false,
            error: None,
        },
        Ok(vs) => {
            let verdict = vs.iter().find(|v| v.ticker == pack.ticker).cloned();
            JudgeResult {
                passed: verdict.as_ref().is_some_and(|v| judge::passed(pack, v)),
                verdict,
                error: None,
            }
        }
    }
}

/// Writer, checks and judges for one post's packs.
pub(crate) async fn run_post(
    post: &str,
    packs: &[Pack],
    m: &Models,
) -> Result<Vec<Record>, BudgetStop> {
    let mut insights: BTreeMap<String, Insight> = BTreeMap::new();
    let mut errors: BTreeMap<String, String> = BTreeMap::new();
    match write::write(m.writer.as_ref(), packs).await {
        Err(e) => {
            if m.budget.exceeded.get() {
                return Err(BudgetStop);
            }
            for p in packs {
                errors.insert(p.ticker.clone(), format!("writer call failed: {e}"));
            }
        }
        Ok(items) => {
            for item in items {
                match item {
                    Ok(i) => {
                        insights.insert(i.ticker.clone(), i);
                    }
                    Err(e) => {
                        let owner = packs
                            .iter()
                            .find(|p| e.starts_with(&format!("{}:", p.ticker)))
                            .filter(|p| !insights.contains_key(&p.ticker));
                        match owner {
                            Some(p) => {
                                errors.insert(p.ticker.clone(), e);
                            }
                            None => eprintln!("{post}: writer reply item dropped: {e}"),
                        }
                    }
                }
            }
        }
    }

    let checks: BTreeMap<String, Vec<String>> = packs
        .iter()
        .filter_map(|p| {
            insights
                .get(&p.ticker)
                .map(|i| (p.ticker.clone(), check::check(p, i)))
        })
        .collect();
    let to_judge: Vec<Insight> = packs
        .iter()
        .filter(|p| checks.get(&p.ticker).is_some_and(Vec::is_empty))
        .map(|p| insights[&p.ticker].clone())
        .collect();
    let verdicts = judge_all(m, packs, &to_judge).await?;

    Ok(packs
        .iter()
        .map(|p| {
            let sent = to_judge.iter().any(|i| i.ticker == p.ticker);
            let judges: BTreeMap<String, JudgeResult> = verdicts
                .iter()
                .map(|(name, r)| (name.clone(), judge_result(r, p, sent)))
                .collect();
            let published_if = judges
                .iter()
                .map(|(name, j)| (name.clone(), sent && j.passed))
                .collect();
            Record {
                post: post.to_string(),
                ticker: p.ticker.clone(),
                pack: p.clone(),
                insight: insights.get(&p.ticker).cloned(),
                write_error: errors.get(&p.ticker).cloned(),
                check: checks.get(&p.ticker).cloned().unwrap_or_default(),
                judges,
                published_if,
                writer_version: WRITER_VERSION.to_string(),
                judge_version: JUDGE_VERSION.to_string(),
            }
        })
        .collect())
}

pub(crate) fn append_jsonl<T: Serialize>(path: &Path, rows: &[T]) {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    for r in rows {
        writeln!(f, "{}", serde_json::to_string(r).expect("json")).expect("write jsonl");
    }
}

pub(crate) fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| parse(&path.display().to_string(), l))
        .collect()
}

/// `run`: every post in `inputs`, writing `lines.jsonl` (fresh each run) and
/// `posts.jsonl` (movers and eligible movers per post). A post with no
/// eligible mover makes no call. Stops at the budget cap, keeping every post
/// already finished.
pub(crate) async fn run(inputs: &Inputs, m: &Models, out: &Path) -> Result<(), String> {
    fs::create_dir_all(out).expect("out dir");
    let lines = out.join("lines.jsonl");
    let posts = out.join("posts.jsonl");
    fs::write(&lines, "").expect("lines.jsonl");
    fs::write(&posts, "").expect("posts.jsonl");
    for (name, post) in &inputs.posts {
        let (packs, movers) = inputs.packs(post);
        let summary = json!({"post": name, "movers": movers, "eligible": packs.len()});
        if packs.is_empty() {
            eprintln!("{name}: no eligible mover of {movers}");
            append_jsonl(&posts, &[summary]);
            continue;
        }
        let records = run_post(name, &packs, m)
            .await
            .map_err(|_| stop_message(&m.budget))?;
        let written = records.iter().filter(|r| r.insight.is_some()).count();
        let checked = records
            .iter()
            .filter(|r| r.insight.is_some() && r.check.is_empty())
            .count();
        let passes: Vec<String> = JUDGES
            .iter()
            .map(|j| {
                let n = records
                    .iter()
                    .filter(|r| r.published_if.get(*j) == Some(&true))
                    .count();
                format!("{j} {n}")
            })
            .collect();
        eprintln!(
            "{name}: {} eligible of {movers}, {written} written, {checked} passed the checks, judge passes: {} (spent ${:.4})",
            packs.len(),
            passes.join(", "),
            m.budget.spent.get()
        );
        append_jsonl(&lines, &records);
        append_jsonl(&posts, &[summary]);
    }
    Ok(())
}

/// `seed`: wrong variants of every line in `lines.jsonl` that passed the
/// checks, written to `seeded.jsonl` (fresh each run). The variants the
/// checks let through go to each judge, one call per judge per post; each
/// variant is sent under its own name ("AAPL-v2") so one call can hold
/// several variants of one mover.
pub(crate) async fn seed(m: &Models, out: &Path) -> Result<(), String> {
    let records: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
    let path = out.join("seeded.jsonl");
    fs::write(&path, "").expect("seeded.jsonl");
    let all = seed::seed_records(&records);
    let mut posts: Vec<&str> = all.iter().map(|(s, _)| s.post.as_str()).collect();
    posts.dedup();
    for post in posts {
        let mut rows: Vec<seed::Seeded> = Vec::new();
        let mut sent: Vec<(usize, Pack, Insight)> = Vec::new();
        for (s, pack) in all.iter().filter(|(s, _)| s.post == post) {
            if s.check.is_empty() {
                let name = format!("{}-v{}", s.ticker, sent.len() + 1);
                let mut p = pack.clone();
                p.ticker = name.clone();
                let mut i = s.insight.clone();
                i.ticker = name;
                sent.push((rows.len(), p, i));
            }
            rows.push(s.clone());
        }
        let packs: Vec<Pack> = sent.iter().map(|(_, p, _)| p.clone()).collect();
        let insights: Vec<Insight> = sent.iter().map(|(_, _, i)| i.clone()).collect();
        let verdicts = judge_all(m, &packs, &insights)
            .await
            .map_err(|_| stop_message(&m.budget))?;
        for (row, p, _) in &sent {
            rows[*row].judges = verdicts
                .iter()
                .map(|(name, r)| (name.clone(), judge_result(r, p, true)))
                .collect();
        }
        let caught: Vec<String> = JUDGES
            .iter()
            .map(|j| {
                let n = rows.iter().filter(|r| r.caught(j) == Some(true)).count();
                format!("{j} caught {n}")
            })
            .collect();
        eprintln!(
            "{post}: {} seeded, {} refused by the checks, {} (spent ${:.4})",
            rows.len(),
            rows.len() - sent.len(),
            caught.join(", "),
            m.budget.spent.get()
        );
        append_jsonl(&path, &rows);
    }
    Ok(())
}

/// `events-json`: D1 `macro_events` rows (`fetch-facts.sh`) as
/// `[["YYYY-MM-DD", title], ...]`, titled as the site titles them. Rows the
/// site does not show (a series without a title) are left out.
fn events_json(rows: &Value) -> Value {
    let rows = rows.as_array().cloned().unwrap_or_default();
    let s = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
    Value::Array(
        rows.iter()
            .filter_map(|r| {
                let title = prebell_core::events::title(
                    &s(r, "series")?,
                    s(r, "ref_period").as_deref(),
                    s(r, "variant").as_deref(),
                    prebell_core::Lang::En,
                )?;
                Some(json!([s(r, "et_date")?, title]))
            })
            .collect(),
    )
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn need(args: &[String], name: &str) -> String {
    arg(args, name).unwrap_or_else(|| {
        eprintln!("missing {name}");
        std::process::exit(2)
    })
}

/// The models for a run that spends money: both keys must be present.
fn live_models(args: &[String], out: &Path) -> Models {
    let keys = http::load_keys(arg(args, "--dev-vars").as_deref());
    for var in ["ANTHROPIC_API_KEY", "DEEPSEEK_API_KEY"] {
        if !keys.contains_key(var) {
            eprintln!("{var} is not set: pass --dev-vars PATH or set it in the environment");
            std::process::exit(2);
        }
    }
    let cap: f64 = arg(args, "--budget").map_or(3.0, |v| v.parse().expect("--budget USD"));
    fs::create_dir_all(out).expect("out dir");
    let budget = Rc::new(Budget::new(cap, out.join("spend.jsonl")));
    eprintln!(
        "spent before this run: ${:.4} of ${cap:.2}",
        budget.spent.get()
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .expect("http client");
    models(Rc::new(ReqwestHttp { client }), budget, &keys)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: insight-trial run --facts DIR --out DIR --dev-vars PATH [--budget 3.0] [--only SUBSTR]\n       insight-trial seed --out DIR --dev-vars PATH [--budget 3.0]\n       insight-trial page --out DIR [--cards 60]\n       insight-trial score --out DIR --labels FILE\n       insight-trial events-json ROWS.json";
    match args.get(1).map(String::as_str) {
        Some("run") => {
            let out = PathBuf::from(need(&args, "--out"));
            let inputs = load_inputs(
                Path::new(&need(&args, "--facts")),
                arg(&args, "--only").as_deref(),
            );
            let m = live_models(&args, &out);
            eprintln!(
                "{} posts, {} earnings checks",
                inputs.posts.len(),
                inputs.checks.len()
            );
            let r = run(&inputs, &m, &out).await;
            eprintln!(
                "spent: ${:.4} of ${:.2}",
                m.budget.spent.get(),
                m.budget.cap
            );
            if let Err(e) = r {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some("seed") => {
            let out = PathBuf::from(need(&args, "--out"));
            let m = live_models(&args, &out);
            let r = seed(&m, &out).await;
            eprintln!(
                "spent: ${:.4} of ${:.2}",
                m.budget.spent.get(),
                m.budget.cap
            );
            if let Err(e) = r {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some("page") => {
            let out = PathBuf::from(need(&args, "--out"));
            let n = arg(&args, "--cards").map_or(page::CARDS, |v| v.parse().expect("--cards N"));
            let records: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
            let html = page::page(&records, n);
            fs::write(out.join("label.html"), &html).expect("label.html");
            eprintln!(
                "wrote {} with {} lines",
                out.join("label.html").display(),
                html.matches("<article class=\"card\"").count()
            );
        }
        Some("score") => {
            let out = PathBuf::from(need(&args, "--out"));
            let labels: Vec<score::Label> = parse(
                "labels",
                &fs::read_to_string(need(&args, "--labels")).expect("read labels"),
            );
            fn optional<T: serde::de::DeserializeOwned>(out: &Path, name: &str) -> Vec<T> {
                let p = out.join(name);
                if p.exists() {
                    read_jsonl(&p)
                } else {
                    eprintln!("{} is missing: counted as empty", p.display());
                    vec![]
                }
            }
            let records: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
            let seeded: Vec<seed::Seeded> = optional(&out, "seeded.jsonl");
            let posts: Vec<Value> = optional(&out, "posts.jsonl");
            let spend: Vec<Value> = optional(&out, "spend.jsonl");
            let md = score::report(&records, &posts, &seeded, &spend, &labels);
            fs::write(out.join("report.md"), &md).expect("report.md");
            println!("{md}");
        }
        Some("events-json") => {
            let rows = args.get(2).unwrap_or_else(|| {
                eprintln!("{usage}");
                std::process::exit(2)
            });
            let v: Value = parse(rows, &fs::read_to_string(rows).expect("read rows"));
            println!("{}", events_json(&v));
        }
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_trait::async_trait;
    use prebell_core::facts::{Field, NewsReason, Quote};
    use prebell_core::providers::{HttpResp, ProviderError};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    /// A provider endpoint that answers from a queue and counts requests.
    #[derive(Default)]
    pub(crate) struct Mock {
        pub replies: RefCell<VecDeque<String>>,
        pub calls: Cell<usize>,
    }

    impl Mock {
        pub fn new(replies: Vec<String>) -> Rc<Self> {
            Rc::new(Mock {
                replies: RefCell::new(replies.into()),
                calls: Cell::new(0),
            })
        }
    }

    #[async_trait(?Send)]
    impl Http for Mock {
        async fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResp, ProviderError> {
            unreachable!("no GET")
        }
        async fn post_json(
            &self,
            _: &str,
            _: &[(&str, &str)],
            _: &Value,
        ) -> Result<HttpResp, ProviderError> {
            self.calls.set(self.calls.get() + 1);
            let body = self
                .replies
                .borrow_mut()
                .pop_front()
                .ok_or(ProviderError::Network("no reply queued".into()))?;
            Ok(HttpResp {
                status: 200,
                body,
                set_cookie: None,
            })
        }
    }

    /// A Claude response whose text is `reply`, billed `out_tokens`.
    pub(crate) fn claude_body(reply: &Value, out_tokens: u64) -> String {
        json!({"model": CLAUDE_MODEL, "stop_reason": "end_turn",
            "content": [{"type": "text", "text": reply.to_string()}],
            "usage": {"input_tokens": 1000, "output_tokens": out_tokens}})
        .to_string()
    }

    pub(crate) fn deepseek_body(reply: &Value) -> String {
        json!({"model": DEEPSEEK_MODEL, "choices": [{"finish_reason": "stop",
            "message": {"content": reply.to_string()}}],
            "usage": {"prompt_tokens": 1000, "completion_tokens": 200}})
        .to_string()
    }

    /// Models over mocks: the writer and the named judges.
    pub(crate) fn mock_models(
        budget: Rc<Budget>,
        writer: &Rc<Mock>,
        judges: &[(&str, &Rc<Mock>)],
    ) -> Models {
        let keys = HashMap::new();
        Models {
            writer: provider("claude", "writer", writer.clone(), &budget, &keys),
            judges: judges
                .iter()
                .map(|(j, mock)| {
                    let inner: Rc<dyn Http> = (*mock).clone();
                    (j.to_string(), provider(j, "judge", inner, &budget, &keys))
                })
                .collect(),
            budget,
        }
    }

    pub(crate) fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("insight-trial-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    pub(crate) const QUOTE: &str =
        "Apple said it expects revenue of $4.2 billion in the December quarter.";

    pub(crate) fn mover(sym: &str, reasons: Vec<NewsReason>) -> Mover {
        Mover {
            quote: Quote {
                symbol: sym.into(),
                last: 104.2,
                prev_close: 100.0,
                change_pct: 4.2,
                volume: None,
            },
            name: "Apple".into(),
            reasons,
            search_failed: false,
            fallback: None,
        }
    }

    pub(crate) fn reuters() -> NewsReason {
        NewsReason {
            url: "https://www.reuters.com/technology/apple-2026-11-12/".into(),
            title: "Apple shares rise on outlook".into(),
            quote: QUOTE.into(),
            summary_en: "Apple expects $4.2 billion in revenue.".into(),
        }
    }

    pub(crate) fn premarket(date: &str, gainers: Vec<Mover>) -> Post {
        fn missing<T>() -> Field<T> {
            Field::Missing {
                reason: "test".into(),
            }
        }
        Post::Premarket(PremarketFacts {
            trading_date: date.parse().unwrap(),
            as_of_utc: format!("{date}T12:00:00Z").parse().unwrap(),
            futures: vec![],
            gainers,
            losers: vec![],
            macro_items: missing(),
            watch_list: missing(),
            themes: vec![],
            theme_candidates: vec![],
            diagnostics: vec![],
        })
    }

    pub(crate) fn writer_reply(en_watch: &str) -> Value {
        json!({"insights": [{"ticker": "AAPL",
            "en": {"hook": "Guidance sets the next test", "watch": en_watch, "cites": ["e2"]},
            "zh": {"hook": "业绩指引成为下一道考验", "watch": "据路透社报道，苹果预计当季营收42亿美元，后续财报将检验这一数字。", "cites": ["e2"]}}]})
    }

    pub(crate) const CLEAN: &str = "Reuters reports Apple expects revenue of $4.2 billion in the December quarter, a figure its coming report will test.";

    pub(crate) fn judge_reply(ok: bool) -> Value {
        json!({"verdicts": [{"ticker": "AAPL",
            "en_claims": [{"claim": "revenue of $4.2 billion", "evidence_id": "e2", "support": "expects revenue of $4.2 billion"}],
            "zh_claims": [{"claim": "营收42亿美元", "evidence_id": "e2", "support": "expects revenue of $4.2 billion"}],
            "en_supported": ok, "zh_supported": true, "no_advice": true, "formal": true, "same_meaning": true, "notes": ""}]})
    }

    fn inputs(posts: Vec<(&str, Post)>) -> Inputs {
        Inputs {
            posts: posts.into_iter().map(|(n, p)| (n.to_string(), p)).collect(),
            ..Inputs::default()
        }
    }

    #[tokio::test]
    async fn a_post_with_no_eligible_mover_makes_no_call() {
        let out = tmp("no-eligible");
        let writer = Mock::new(vec![]);
        let judge = Mock::new(vec![]);
        let budget = Rc::new(Budget::new(3.0, out.join("spend.jsonl")));
        let m = mock_models(budget, &writer, &[("claude", &judge)]);
        let i = inputs(vec![(
            "premarket_2026-11-12",
            premarket("2026-11-12", vec![mover("AAPL", vec![])]),
        )]);
        run(&i, &m, &out).await.unwrap();
        assert_eq!((writer.calls.get(), judge.calls.get()), (0, 0));
        assert_eq!(fs::read_to_string(out.join("lines.jsonl")).unwrap(), "");
        let posts: Vec<Value> = read_jsonl(&out.join("posts.jsonl"));
        assert_eq!(
            posts,
            [json!({"post": "premarket_2026-11-12", "movers": 1, "eligible": 0})]
        );
        fs::remove_dir_all(out).unwrap();
    }

    #[tokio::test]
    async fn the_budget_stops_the_run_and_keeps_results() {
        // A cap of 0 stops before the first call.
        let out = tmp("budget-zero");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 200)]);
        let judge = Mock::new(vec![]);
        let m = mock_models(
            Rc::new(Budget::new(0.0, out.join("spend.jsonl"))),
            &writer,
            &[("claude", &judge)],
        );
        let i = inputs(vec![(
            "premarket_2026-11-12",
            premarket("2026-11-12", vec![mover("AAPL", vec![reuters()])]),
        )]);
        let e = run(&i, &m, &out).await.unwrap_err();
        assert!(
            e.contains("budget cap reached: spent $0.0000 of $0.00"),
            "{e}"
        );
        assert_eq!(writer.calls.get(), 0);
        assert!(out.join("lines.jsonl").exists());
        fs::remove_dir_all(out).unwrap();

        // A cap that covers the first post (each call billed about $0.09)
        // but not the second: the first post's line is kept.
        let out = tmp("budget-second");
        let writer = Mock::new(vec![
            claude_body(&writer_reply(CLEAN), 9000),
            claude_body(&writer_reply(CLEAN), 9000),
        ]);
        let judge = Mock::new(vec![claude_body(&judge_reply(true), 9000)]);
        let m = mock_models(
            Rc::new(Budget::new(0.2, out.join("spend.jsonl"))),
            &writer,
            &[("claude", &judge)],
        );
        let i = inputs(vec![
            (
                "premarket_2026-11-12",
                premarket("2026-11-12", vec![mover("AAPL", vec![reuters()])]),
            ),
            (
                "premarket_2026-11-13",
                premarket("2026-11-13", vec![mover("AAPL", vec![reuters()])]),
            ),
        ]);
        let e = run(&i, &m, &out).await.unwrap_err();
        assert!(e.contains("budget cap reached"), "{e}");
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 1));
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].post, "premarket_2026-11-12");
        // The ledger holds both calls, with role and tokens, and a later run reads it back.
        let spend: Vec<Value> = read_jsonl(&out.join("spend.jsonl"));
        assert_eq!(spend.len(), 2);
        assert_eq!(spend[0]["role"], "writer");
        assert_eq!(spend[1]["role"], "judge");
        assert_eq!(spend[0]["output_tokens"], 9000);
        let again = Budget::new(0.2, out.join("spend.jsonl"));
        assert!((again.spent.get() - m.budget.spent.get()).abs() < 1e-12);
        fs::remove_dir_all(out).unwrap();
    }

    #[tokio::test]
    async fn lines_record_every_stage() {
        let out = tmp("every-stage");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 200)]);
        let judge = Mock::new(vec![deepseek_body(&judge_reply(true))]);
        let m = mock_models(
            Rc::new(Budget::new(3.0, out.join("spend.jsonl"))),
            &writer,
            &[("deepseek", &judge)],
        );
        let i = inputs(vec![(
            "premarket_2026-11-12",
            premarket("2026-11-12", vec![mover("AAPL", vec![reuters()])]),
        )]);
        run(&i, &m, &out).await.unwrap();
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert_eq!(lines.len(), 1);
        let r = &lines[0];
        assert_eq!(
            (r.post.as_str(), r.ticker.as_str()),
            ("premarket_2026-11-12", "AAPL")
        );
        assert_eq!(r.pack.evidence[1].text, QUOTE);
        assert_eq!(r.insight.as_ref().unwrap().en.watch, CLEAN);
        assert_eq!(r.write_error, None);
        assert_eq!(r.check, Vec::<String>::new());
        let j = &r.judges["deepseek"];
        assert!(j.verdict.is_some());
        assert!(j.passed);
        assert!(r.published_if["deepseek"]);
        assert_eq!(r.writer_version, WRITER_VERSION);
        // The raw line has the planned shape, and no key or header anywhere.
        let raw = fs::read_to_string(out.join("lines.jsonl")).unwrap();
        let v: Value = serde_json::from_str(raw.trim()).unwrap();
        for k in [
            "post",
            "ticker",
            "pack",
            "insight",
            "write_error",
            "check",
            "judges",
            "published_if",
        ] {
            assert!(v.get(k).is_some(), "{k}");
        }
        let spend = fs::read_to_string(out.join("spend.jsonl")).unwrap();
        for text in [&raw, &spend] {
            assert!(
                !text.contains("x-api-key") && !text.contains("Bearer"),
                "{text}"
            );
        }
        fs::remove_dir_all(out).unwrap();
    }

    #[tokio::test]
    async fn a_check_failure_never_reaches_the_judge() {
        let out = tmp("check-failure");
        let bad = "Reuters reports Apple expects revenue of $4.2 billion, so investors should buy before the report.";
        let writer = Mock::new(vec![claude_body(&writer_reply(bad), 200)]);
        let judge = Mock::new(vec![]);
        let m = mock_models(
            Rc::new(Budget::new(3.0, out.join("spend.jsonl"))),
            &writer,
            &[("claude", &judge)],
        );
        let i = inputs(vec![(
            "premarket_2026-11-12",
            premarket("2026-11-12", vec![mover("AAPL", vec![reuters()])]),
        )]);
        run(&i, &m, &out).await.unwrap();
        assert_eq!(judge.calls.get(), 0);
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        let r = &lines[0];
        assert!(r.insight.is_some());
        assert!(
            r.check.iter().any(|c| c.contains("should")),
            "{:?}",
            r.check
        );
        assert_eq!(
            r.judges["claude"],
            JudgeResult {
                verdict: None,
                passed: false,
                error: None
            }
        );
        assert!(!r.published_if["claude"]);
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn a_writer_error_is_kept_on_its_mover() {
        let out = tmp("writer-error");
        let writer = Mock::new(vec![claude_body(
            &json!({"insights": [{"ticker": "AAPL", "en": {}}, {"ticker": "TSLA"}]}),
            200,
        )]);
        let m = mock_models(
            Rc::new(Budget::new(3.0, out.join("spend.jsonl"))),
            &writer,
            &[],
        );
        let i = inputs(vec![(
            "premarket_2026-11-12",
            premarket("2026-11-12", vec![mover("AAPL", vec![reuters()])]),
        )]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(run(&i, &m, &out)).unwrap();
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert!(lines[0].insight.is_none());
        assert!(
            lines[0].write_error.as_deref().unwrap().contains("en"),
            "{:?}",
            lines[0].write_error
        );
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn inputs_load_as_fetch_facts_saves_them() {
        let dir = tmp("load");
        let Post::Premarket(p) = premarket("2026-11-12", vec![mover("AAPL", vec![reuters()])])
        else {
            unreachable!()
        };
        fs::write(
            dir.join("premarket_2026-11-12.json"),
            serde_json::to_string(&p).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("premarket_2026-11-13.json"),
            serde_json::to_string(&p).unwrap(),
        )
        .unwrap();
        fs::write(dir.join("calendar.json"), r#"[["AAPL","2026-11-19"]]"#).unwrap();
        fs::write(dir.join("events.json"), r#"[["2026-11-13","October CPI"]]"#).unwrap();
        fs::write(dir.join("macro_events.rows.json"), "[]").unwrap();
        let i = load_inputs(&dir, Some("11-12"));
        assert_eq!(i.posts.len(), 1);
        assert_eq!(i.calendar, [("AAPL".to_string(), date("2026-11-19"))]);
        assert_eq!(
            i.macro_events,
            [(date("2026-11-13"), "October CPI".to_string())]
        );
        let (packs, movers) = i.packs(&i.posts[0].1);
        assert_eq!((packs.len(), movers), (1, 1));
        let kinds: Vec<_> = packs[0].evidence.iter().map(|e| e.kind).collect();
        use prebell_core::insight::EvidenceKind::*;
        assert_eq!(kinds, [Move, News, ReportDate, MacroEvent]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn seed_judges_only_the_variants_the_checks_let_through() {
        let out = tmp("seed");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 200)]);
        let judge = Mock::new(vec![deepseek_body(&judge_reply(true))]);
        let m = mock_models(
            Rc::new(Budget::new(3.0, out.join("spend.jsonl"))),
            &writer,
            &[("deepseek", &judge)],
        );
        let i = inputs(vec![(
            "premarket_2026-11-12",
            premarket("2026-11-12", vec![mover("AAPL", vec![reuters()])]),
        )]);
        run(&i, &m, &out).await.unwrap();

        // The variant judge sees "AAPL-v1" and passes nothing.
        let mut reply = judge_reply(false);
        reply["verdicts"][0]["ticker"] = json!("AAPL-v1");
        judge.replies.borrow_mut().push_back(deepseek_body(&reply));
        seed(&m, &out).await.unwrap();
        assert_eq!(judge.calls.get(), 2);
        let rows: Vec<seed::Seeded> = read_jsonl(&out.join("seeded.jsonl"));
        let kinds: Vec<seed::Kind> = rows.iter().map(|r| r.kind).collect();
        // One number in each line and one outlet in the pack: the swaps
        // change a digit (the checks refuse that), no outlet or direction to swap.
        assert_eq!(
            kinds,
            [
                seed::Kind::NumberSwap,
                seed::Kind::ZhOnlyNumber,
                seed::Kind::FormalAdvice
            ]
        );
        assert!(!rows[0].check.is_empty() && rows[0].judges.is_empty());
        assert!(!rows[1].check.is_empty() && rows[1].judges.is_empty());
        assert!(rows[2].check.is_empty());
        assert_eq!(rows[2].caught("deepseek"), Some(true));
        assert_eq!(rows[2].ticker, "AAPL");
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn events_are_titled_as_the_site_titles_them() {
        let rows = json!([
            {"et_date": "2026-11-13", "series": "cpi", "ref_period": "2026-10", "variant": null},
            {"et_date": "2026-11-14", "series": "unknown_series", "ref_period": null, "variant": null},
        ]);
        assert_eq!(events_json(&rows), json!([["2026-11-13", "October CPI"]]));
    }
}
