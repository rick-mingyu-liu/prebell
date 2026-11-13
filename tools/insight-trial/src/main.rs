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
//! `seed` sends each judge deliberately wrong variants of the lines that
//! passed the checks, beside the original lines as controls; `page` writes
//! the labelling page; `score` reads the owner's labels back and writes the
//! judge report.
//!
//! ```text
//! cargo run --release -- run --facts DIR --out DIR --dev-vars PATH [--budget 3.0] [--only SUBSTR] [--fresh] [--plan]
//! cargo run --release -- seed --out DIR --dev-vars PATH [--budget 3.0] [--fresh] [--plan]
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

use chrono::{DateTime, NaiveDate, Utc};
use prebell_core::cover::Session;
use prebell_core::earnings::facts::EarningsFacts;
use prebell_core::facts::{Mover, PremarketFacts, Theme};
use prebell_core::insight::judge::{self, Verdict, JUDGE_VERSION};
use prebell_core::insight::pack::{packs, PackInputs, ReportStatus};
use prebell_core::insight::write::{self, WRITER_VERSION};
use prebell_core::insight::{check, Insight, Pack};
use prebell_core::llm::{api_key_var, llm_from_env};
use prebell_core::providers::{Http, LlmProvider};
use prebell_core::recap::RecapFacts;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use http::{estimate_usd, max_output_tokens, Budget, RecordingHttp, ReqwestHttp};

/// The writer is always Claude.
const CLAUDE_MODEL: &str = "claude-sonnet-5-5";
/// The DeepSeek provider's default model (`llm::llm_from_env` with no model),
/// named here only for the price table and the ledger.
const DEEPSEEK_MODEL: &str = "deepseek-flash";
/// The judges compared, in report order.
pub(crate) const JUDGES: [&str; 2] = ["claude", "deepseek"];
/// A judge result for a call the budget stopped: `run` makes it on a re-run.
pub(crate) const NOT_RUN: &str = "not run: budget cap reached";

fn model_of(provider: &str) -> &'static str {
    match provider {
        "claude" => CLAUDE_MODEL,
        _ => DEEPSEEK_MODEL,
    }
}

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
    fn as_of_utc(&self) -> DateTime<Utc> {
        match self {
            Post::Premarket(f) => f.as_of_utc,
            Post::Recap(f) => f.as_of_utc,
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
    pub calendar: Vec<(String, NaiveDate, ReportStatus)>,
    pub macro_events: Vec<(NaiveDate, String)>,
}

impl Inputs {
    /// The eligible movers' packs for one post, and how many movers it lists.
    pub fn packs(&self, post: &Post) -> (Vec<Pack>, usize) {
        let movers = post.movers();
        let p = packs(&PackInputs {
            session: post.session(),
            trading_date: post.trading_date(),
            as_of_utc: post.as_of_utc(),
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
            let rows: Vec<(String, String, String)> = parse(&name, &json);
            for (t, d, status) in rows {
                match ReportStatus::parse(&status) {
                    Some(s) => i.calendar.push((t, date(&d), s)),
                    None => eprintln!("calendar: skipping {t} {d} ({status})"),
                }
            }
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
    let model = model_of(provider);
    let pinned = (provider == "claude").then(|| CLAUDE_MODEL.to_string());
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
    /// The judge call failed, or (`NOT_RUN`) the budget stopped it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl JudgeResult {
    /// The judge answered for this mover: its call worked and its reply
    /// held a verdict. A failed call or an omitted mover is not an answer,
    /// so it counts neither as a pass nor as a catch.
    pub fn answered(&self) -> bool {
        self.error.is_none() && self.verdict.is_some()
    }
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

/// A line's identity across runs: the writer version and the four texts.
/// The labelling page keys its saved choices by it, and `score` uses it to
/// notice labels or seeds made for an older line.
pub(crate) fn line_key(i: &Insight) -> String {
    // FNV-1a, 64 bits: stable across builds, unlike the std hasher.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [
        WRITER_VERSION,
        &i.en.hook,
        &i.en.watch,
        &i.zh.hook,
        &i.zh.watch,
    ] {
        for b in part.bytes().chain([0]) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
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

/// One judge's verdicts on `insights`, or why its call failed. `Err` only
/// when the budget stopped the call.
async fn judge_one(
    m: &Models,
    llm: &dyn LlmProvider,
    packs: &[Pack],
    insights: &[Insight],
) -> Result<Result<Vec<Verdict>, String>, BudgetStop> {
    let r = judge::judge(llm, packs, insights).await;
    if r.is_err() && m.budget.exceeded.get() {
        return Err(BudgetStop);
    }
    Ok(r.map_err(|e| e.to_string()))
}

/// The answer of one judge on one mover. `sent` is false for a mover the
/// checks kept from the judges.
pub(crate) fn judge_result(
    r: &Result<Vec<Verdict>, String>,
    pack: &Pack,
    insight: Option<&Insight>,
    sent: bool,
) -> JudgeResult {
    match (r, insight) {
        (Err(e), _) if sent => JudgeResult {
            verdict: None,
            passed: false,
            error: Some(e.clone()),
        },
        (Ok(vs), Some(i)) if sent => {
            let verdict = vs.iter().find(|v| v.ticker == pack.ticker).cloned();
            JudgeResult {
                passed: verdict.as_ref().is_some_and(|v| judge::passed(pack, i, v)),
                verdict,
                error: None,
            }
        }
        _ => JudgeResult {
            verdict: None,
            passed: false,
            error: None,
        },
    }
}

/// `run` stopped at the cap during a post. `records` holds what was paid
/// for (the writer's lines and the judges that answered), when the writer
/// call went out.
pub(crate) struct Stopped {
    pub records: Option<Vec<Record>>,
}

/// Writer, checks and judges for one post's packs. `earlier` holds this
/// post's records from a run the budget stopped: their writer results are
/// reused, and only the judges marked `NOT_RUN` are called.
pub(crate) async fn run_post(
    post: &str,
    packs: &[Pack],
    m: &Models,
    earlier: &[Record],
) -> Result<Vec<Record>, Stopped> {
    let mut insights: BTreeMap<String, Insight> = BTreeMap::new();
    let mut errors: BTreeMap<String, String> = BTreeMap::new();
    if earlier.is_empty() {
        match write::write(m.writer.as_ref(), packs).await {
            Err(e) => {
                if m.budget.exceeded.get() {
                    return Err(Stopped { records: None });
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
                                .find(|p| {
                                    e.to_ascii_uppercase()
                                        .starts_with(&format!("{}:", p.ticker.to_ascii_uppercase()))
                                })
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
    } else {
        for r in earlier {
            if let Some(i) = &r.insight {
                insights.insert(r.ticker.clone(), i.clone());
            }
            if let Some(e) = &r.write_error {
                errors.insert(r.ticker.clone(), e.clone());
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

    // Each judge's results per mover: kept from the earlier run when it
    // answered there, else called now.
    let mut results: BTreeMap<String, BTreeMap<String, JudgeResult>> = BTreeMap::new();
    let mut stopped = false;
    for (name, llm) in &m.judges {
        let kept: Option<BTreeMap<String, JudgeResult>> = (!earlier.is_empty())
            .then(|| {
                earlier
                    .iter()
                    .map(|r| r.judges.get(name).cloned().map(|j| (r.ticker.clone(), j)))
                    .collect::<Option<BTreeMap<_, _>>>()
            })
            .flatten()
            // A judge whose call failed (the budget, a network error, bad
            // JSON) is asked again.
            .filter(|js| js.values().all(|j| j.error.is_none()));
        if let Some(js) = kept {
            results.insert(name.clone(), js);
            continue;
        }
        let r = if stopped {
            Err(BudgetStop)
        } else {
            judge_one(m, llm.as_ref(), packs, &to_judge).await
        };
        let r = match r {
            Ok(r) => r,
            Err(BudgetStop) => {
                stopped = true;
                Err(NOT_RUN.to_string())
            }
        };
        let per_mover = packs
            .iter()
            .map(|p| {
                let sent = to_judge.iter().any(|i| i.ticker == p.ticker);
                (
                    p.ticker.clone(),
                    judge_result(&r, p, insights.get(&p.ticker), sent),
                )
            })
            .collect();
        results.insert(name.clone(), per_mover);
    }

    let records: Vec<Record> = packs
        .iter()
        .map(|p| {
            let sent = to_judge.iter().any(|i| i.ticker == p.ticker);
            let judges: BTreeMap<String, JudgeResult> = results
                .iter()
                .map(|(name, per)| (name.clone(), per[&p.ticker].clone()))
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
        .collect();
    if stopped {
        Err(Stopped {
            records: Some(records),
        })
    } else {
        Ok(records)
    }
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

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) {
    fs::write(path, "").unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    append_jsonl(path, rows);
}

pub(crate) fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| parse(&path.display().to_string(), l))
        .collect()
}

/// `path`'s rows, or none when it does not exist (or `fresh`).
fn kept_jsonl<T: serde::de::DeserializeOwned>(path: &Path, fresh: bool) -> Vec<T> {
    if fresh || !path.exists() {
        vec![]
    } else {
        read_jsonl(path)
    }
}

/// Average output tokens of earlier calls by `role` on `model` in the
/// ledger, if any.
fn ledger_out_tokens(ledger: &Path, role: &str, model: &str) -> Option<f64> {
    let rows: Vec<Value> = kept_jsonl(ledger, false);
    let outs: Vec<f64> = rows
        .iter()
        .filter(|v| v["role"] == role && v["model"] == model)
        .filter_map(|v| v["output_tokens"].as_f64())
        .collect();
    (!outs.is_empty()).then(|| outs.iter().sum::<f64>() / outs.len() as f64)
}

/// Assumed output tokens for a call over `movers` movers, when the ledger
/// has no earlier call to go by: Claude spends some on thinking.
fn assumed_out_tokens(role: &str, model: &str, movers: usize) -> f64 {
    let thinking = if model.starts_with("claude") {
        1500.0
    } else {
        0.0
    };
    let per = if role == "writer" { 600.0 } else { 500.0 };
    thinking + per * movers as f64
}

/// The calls a step will make and what they should cost.
#[derive(Default, Debug)]
pub(crate) struct Plan {
    /// (role, model) -> calls.
    pub calls: BTreeMap<(String, String), usize>,
    pub expected_usd: f64,
    pub worst_usd: f64,
}

impl Plan {
    fn add(&mut self, ledger: &Path, role: &str, model: &str, prompt_chars: usize, movers: usize) {
        *self
            .calls
            .entry((role.to_string(), model.to_string()))
            .or_default() += 1;
        let out = ledger_out_tokens(ledger, role, model)
            .unwrap_or_else(|| assumed_out_tokens(role, model, movers));
        self.expected_usd += estimate_usd(model, prompt_chars, out);
        self.worst_usd += estimate_usd(model, prompt_chars, max_output_tokens(model));
    }

    fn total_calls(&self) -> usize {
        self.calls.values().sum()
    }

    fn describe(&self, step: &str) -> String {
        let calls: Vec<String> = self
            .calls
            .iter()
            .map(|((role, model), n)| format!("{n} {role} calls on {model}"))
            .collect();
        format!(
            "{step} plan: {} calls ({}), expected ${:.2}, at most ${:.2} if every reply ran to its token limit",
            self.total_calls(),
            if calls.is_empty() { "none".to_string() } else { calls.join(", ") },
            self.expected_usd,
            self.worst_usd
        )
    }

    /// Refuses to start when the expected cost would pass what is left.
    fn fits(&self, budget: &Budget) -> Result<(), String> {
        let left = budget.cap - budget.spent.get();
        if self.expected_usd > left {
            return Err(format!(
                "not starting: expected ${:.2} but only ${left:.2} of the ${:.2} cap is left",
                self.expected_usd, budget.cap
            ));
        }
        Ok(())
    }
}

/// The posts `run` still has to do: with eligible movers and not already
/// complete in `lines.jsonl`.
fn run_plan(inputs: &Inputs, done: &[Record], ledger: &Path) -> Plan {
    let mut plan = Plan::default();
    for (name, post) in &inputs.posts {
        let (packs, _) = inputs.packs(post);
        let earlier: Vec<Record> = done.iter().filter(|r| r.post == *name).cloned().collect();
        if packs.is_empty() || complete(&earlier) {
            continue;
        }
        let (system, user) = write::prompt(&packs);
        if earlier.is_empty() {
            plan.add(
                ledger,
                "writer",
                CLAUDE_MODEL,
                system.len() + user.len(),
                packs.len(),
            );
        }
        // The judge reads the same packs plus two lines per mover.
        let (jsystem, _) = judge::prompt(&[], &[]);
        let jchars = jsystem.len() + user.len() + 600 * packs.len();
        for j in JUDGES {
            plan.add(ledger, "judge", model_of(j), jchars, packs.len());
        }
    }
    plan
}

/// A post's records are complete when every judge answered without an
/// error: a call the budget stopped, or that failed, is made again.
fn complete(records: &[Record]) -> bool {
    !records.is_empty()
        && records.iter().all(|r| {
            JUDGES
                .iter()
                .all(|j| r.judges.get(*j).is_some_and(|x| x.error.is_none()))
        })
}

/// `run`: every post in `inputs`, kept in `lines.jsonl` and `posts.jsonl`.
/// A post already complete there is skipped (unless `fresh`), and one a
/// budget stop left part-done is finished without paying for its writer
/// again. A post with no eligible mover makes no call. Stops at the budget
/// cap, keeping every result paid for.
pub(crate) async fn run(
    inputs: &Inputs,
    m: &Models,
    out: &Path,
    fresh: bool,
) -> Result<(), String> {
    fs::create_dir_all(out).expect("out dir");
    let lines_path = out.join("lines.jsonl");
    let posts_path = out.join("posts.jsonl");
    let mut lines: Vec<Record> = kept_jsonl(&lines_path, fresh);
    let mut posts: Vec<Value> = kept_jsonl(&posts_path, fresh);
    write_jsonl(&lines_path, &lines);
    write_jsonl(&posts_path, &posts);
    let save = |lines: &[Record], posts: &[Value]| {
        write_jsonl(&lines_path, lines);
        write_jsonl(&posts_path, posts);
    };
    for (name, post) in &inputs.posts {
        let (packs, movers) = inputs.packs(post);
        let earlier: Vec<Record> = lines.iter().filter(|r| r.post == *name).cloned().collect();
        if complete(&earlier) {
            eprintln!("{name}: already in lines.jsonl, skipped (pass --fresh to redo)");
            continue;
        }
        posts.retain(|p| p["post"] != name.as_str());
        posts.push(json!({"post": name, "movers": movers, "eligible": packs.len()}));
        if packs.is_empty() {
            eprintln!("{name}: no eligible mover of {movers}");
            save(&lines, &posts);
            continue;
        }
        let r = run_post(name, &packs, m, &earlier).await;
        let (records, stop) = match r {
            Ok(records) => (records, false),
            Err(Stopped { records: Some(rs) }) => (rs, true),
            Err(Stopped { records: None }) => {
                posts.retain(|p| p["post"] != name.as_str());
                save(&lines, &posts);
                return Err(stop_message(&m.budget));
            }
        };
        lines.retain(|r| r.post != *name);
        lines.extend(records.iter().cloned());
        save(&lines, &posts);
        if stop {
            return Err(format!(
                "{} ({name} is kept part-done: a re-run calls only its missing judges)",
                stop_message(&m.budget)
            ));
        }
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
    }
    Ok(())
}

/// The calls `seed` would make for `plans`.
fn seed_plan(plans: &[seed::PostPlan], ledger: &Path) -> Plan {
    let mut plan = Plan::default();
    for p in plans {
        for call in &p.calls {
            let (packs, insights) = call_items(p, call);
            let (system, user) = judge::prompt(&packs, &insights);
            for j in JUDGES {
                plan.add(
                    ledger,
                    "judge",
                    model_of(j),
                    system.len() + user.len(),
                    call.len(),
                );
            }
        }
    }
    plan
}

/// The posts `seed` will do. All of them when their expected cost fits in
/// `left`; otherwise a fixed-seed random sample of whole posts that fits,
/// with a note saying which and how many rows that sends.
pub(crate) fn fit_seed(
    plans: Vec<seed::PostPlan>,
    ledger: &Path,
    left: f64,
) -> Result<(Vec<seed::PostPlan>, Plan, Option<String>), String> {
    let costs: Vec<f64> = plans
        .iter()
        .map(|p| seed_plan(std::slice::from_ref(p), ledger).expected_usd)
        .collect();
    let total: f64 = costs.iter().sum();
    if total <= left {
        let plan = seed_plan(&plans, ledger);
        return Ok((plans, plan, None));
    }
    let mut order: Vec<usize> = (0..plans.len()).collect();
    page::Rng(page::SEED).shuffle(&mut order);
    let mut keep = vec![false; plans.len()];
    let mut spent = 0.0;
    for k in order {
        if spent + costs[k] <= left {
            spent += costs[k];
            keep[k] = true;
        }
    }
    let all = plans.len();
    let kept: Vec<seed::PostPlan> = plans
        .into_iter()
        .zip(&keep)
        .filter_map(|(p, k)| k.then_some(p))
        .collect();
    if kept.is_empty() {
        return Err(format!(
            "not starting: no post's seeding fits in the ${left:.2} left under the cap (all {all} would cost ${total:.2})"
        ));
    }
    let sent = |control: bool| -> usize {
        kept.iter()
            .map(|p| {
                p.calls
                    .iter()
                    .flatten()
                    .filter(|&&k| (p.rows[k].0.kind == seed::Kind::Control) == control)
                    .count()
            })
            .sum()
    };
    let note = format!(
        "all {all} posts would cost ${total:.2}, more than the ${left:.2} left under the cap: seeding a random sample (fixed seed) of {} posts: {}; that sends each judge {} wrong lines and {} controls",
        kept.len(),
        kept.iter().map(|p| p.post.as_str()).collect::<Vec<_>>().join(", "),
        sent(false),
        sent(true)
    );
    let plan = seed_plan(&kept, ledger);
    Ok((kept, plan, Some(note)))
}

/// One seed call's packs and lines. Each mover appears once per call, under
/// its own ticker, as production sends it.
fn call_items(p: &seed::PostPlan, call: &[usize]) -> (Vec<Pack>, Vec<Insight>) {
    call.iter()
        .map(|&k| (p.rows[k].1.clone(), p.rows[k].0.insight.clone()))
        .unzip()
}

/// `seed`: for every line in `lines.jsonl` that passed the checks, its wrong
/// variants and the line itself as a control, to `seeded.jsonl`. The rows
/// the checks let through go to each judge, at most one row per mover per
/// call. A post already in `seeded.jsonl` is skipped unless `fresh`.
pub(crate) async fn seed(m: &Models, out: &Path, fresh: bool) -> Result<(), String> {
    let records: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
    let path = out.join("seeded.jsonl");
    let mut done: Vec<seed::Seeded> = kept_jsonl(&path, fresh);
    write_jsonl(&path, &done);
    let plans: Vec<seed::PostPlan> = seed::plan(&records)
        .into_iter()
        .filter(|p| !done.iter().any(|d| d.post == p.post))
        .collect();
    let left = m.budget.cap - m.budget.spent.get();
    let (plans, plan, note) = fit_seed(plans, &m.budget.ledger, left)?;
    if let Some(note) = note {
        eprintln!("{note}");
    }
    eprintln!("{}", plan.describe("seed"));
    for p in plans {
        let mut rows: Vec<seed::Seeded> = p.rows.iter().map(|(s, _)| s.clone()).collect();
        for call in &p.calls {
            let (packs, insights) = call_items(&p, call);
            for (name, llm) in &m.judges {
                let r = judge_one(m, llm.as_ref(), &packs, &insights)
                    .await
                    .map_err(|_| {
                        format!(
                            "{} ({} is not written: re-run seed to redo it)",
                            stop_message(&m.budget),
                            p.post
                        )
                    })?;
                for (&k, (pack, insight)) in call.iter().zip(packs.iter().zip(&insights)) {
                    rows[k]
                        .judges
                        .insert(name.clone(), judge_result(&r, pack, Some(insight), true));
                }
            }
        }
        let summary: Vec<String> = JUDGES
            .iter()
            .map(|j| {
                let caught = rows.iter().filter(|r| r.caught(j) == Some(true)).count();
                let counted = rows.iter().filter(|r| r.caught(j).is_some()).count();
                format!("{j} caught {caught} of {counted}")
            })
            .collect();
        eprintln!(
            "{}: {} rows in {} calls per judge, {} refused by the checks, {} (spent ${:.4})",
            p.post,
            rows.len(),
            p.calls.len(),
            rows.iter().filter(|r| !r.check.is_empty()).count(),
            summary.join(", "),
            m.budget.spent.get()
        );
        append_jsonl(&path, &rows);
        done.extend(rows);
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

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn need(args: &[String], name: &str) -> String {
    arg(args, name).unwrap_or_else(|| {
        eprintln!("missing {name}");
        std::process::exit(2)
    })
}

fn budget(args: &[String], out: &Path) -> Rc<Budget> {
    let cap: f64 = arg(args, "--budget").map_or(3.0, |v| v.parse().expect("--budget USD"));
    fs::create_dir_all(out).expect("out dir");
    Rc::new(Budget::new(cap, out.join("spend.jsonl")))
}

/// The models for a step that spends money: both keys must be present.
fn live_models(args: &[String], budget: Rc<Budget>) -> Models {
    let keys = http::load_keys(arg(args, "--dev-vars").as_deref());
    for var in ["ANTHROPIC_API_KEY", "DEEPSEEK_API_KEY"] {
        if !keys.contains_key(var) {
            eprintln!("{var} is not set: pass --dev-vars PATH or set it in the environment");
            std::process::exit(2);
        }
    }
    eprintln!(
        "spent before this run: ${:.4} of ${:.2}",
        budget.spent.get(),
        budget.cap
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .expect("http client");
    models(Rc::new(ReqwestHttp { client }), budget, &keys)
}

fn finish(m: &Models, r: Result<(), String>) {
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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: insight-trial run --facts DIR --out DIR --dev-vars PATH [--budget 3.0] [--only SUBSTR] [--fresh] [--plan]\n       insight-trial seed --out DIR --dev-vars PATH [--budget 3.0] [--fresh] [--plan]\n       insight-trial page --out DIR [--cards 60]\n       insight-trial score --out DIR --labels FILE\n       insight-trial events-json ROWS.json";
    match args.get(1).map(String::as_str) {
        Some("run") => {
            let out = PathBuf::from(need(&args, "--out"));
            let inputs = load_inputs(
                Path::new(&need(&args, "--facts")),
                arg(&args, "--only").as_deref(),
            );
            let fresh = flag(&args, "--fresh");
            let budget = budget(&args, &out);
            let done: Vec<Record> = kept_jsonl(&out.join("lines.jsonl"), fresh);
            let plan = run_plan(&inputs, &done, &budget.ledger);
            eprintln!(
                "{} posts, {} earnings checks, {} calendar rows, {} macro events",
                inputs.posts.len(),
                inputs.checks.len(),
                inputs.calendar.len(),
                inputs.macro_events.len()
            );
            eprintln!("{}", plan.describe("run"));
            if flag(&args, "--plan") {
                return;
            }
            if let Err(e) = plan.fits(&budget) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            let m = live_models(&args, budget);
            let r = run(&inputs, &m, &out, fresh).await;
            finish(&m, r);
        }
        Some("seed") => {
            let out = PathBuf::from(need(&args, "--out"));
            let fresh = flag(&args, "--fresh");
            let budget = budget(&args, &out);
            if flag(&args, "--plan") {
                let records: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
                let done: Vec<seed::Seeded> = kept_jsonl(&out.join("seeded.jsonl"), fresh);
                let plans: Vec<seed::PostPlan> = seed::plan(&records)
                    .into_iter()
                    .filter(|p| !done.iter().any(|d| d.post == p.post))
                    .collect();
                let left = budget.cap - budget.spent.get();
                match fit_seed(plans, &budget.ledger, left) {
                    Ok((_, plan, note)) => {
                        if let Some(note) = note {
                            eprintln!("{note}");
                        }
                        eprintln!("{}", plan.describe("seed"));
                    }
                    Err(e) => eprintln!("{e}"),
                }
                return;
            }
            let m = live_models(&args, budget);
            let r = seed(&m, &out, fresh).await;
            finish(&m, r);
        }
        Some("page") => {
            let out = PathBuf::from(need(&args, "--out"));
            let n = arg(&args, "--cards").map_or(page::CARDS, |v| v.parse().expect("--cards N"));
            let records: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
            let must = page::must_show(&records);
            if must > n {
                eprintln!(
                    "warning: {must} lines a judge said no to or the judges disagreed on: the page shows all of them, more than {n}"
                );
            }
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

    fn one_post(date: &str) -> (String, Post) {
        (
            format!("premarket_{date}"),
            premarket(date, vec![mover("AAPL", vec![reuters()])]),
        )
    }

    fn budget(out: &Path, cap: f64) -> Rc<Budget> {
        Rc::new(Budget::new(cap, out.join("spend.jsonl")))
    }

    #[tokio::test]
    async fn a_post_with_no_eligible_mover_makes_no_call() {
        let out = tmp("no-eligible");
        let writer = Mock::new(vec![]);
        let judge = Mock::new(vec![]);
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        let i = inputs(vec![(
            "premarket_2026-11-12",
            premarket("2026-11-12", vec![mover("AAPL", vec![])]),
        )]);
        run(&i, &m, &out, false).await.unwrap();
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
        let m = mock_models(budget(&out, 0.0), &writer, &[("claude", &judge)]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        let e = run(&i, &m, &out, false).await.unwrap_err();
        assert!(
            e.contains("budget cap reached: spent $0.0000 of $0.00"),
            "{e}"
        );
        assert_eq!(writer.calls.get(), 0);
        assert!(out.join("lines.jsonl").exists());
        fs::remove_dir_all(out).unwrap();

        // A cap that covers the first post (each call billed about $0.09,
        // each reserving $0.16 first) but not the second's writer.
        let out = tmp("budget-second");
        let writer = Mock::new(vec![
            claude_body(&writer_reply(CLEAN), 9000),
            claude_body(&writer_reply(CLEAN), 9000),
        ]);
        let judge = Mock::new(vec![claude_body(&judge_reply(true), 9000)]);
        let m = mock_models(budget(&out, 0.3), &writer, &[("claude", &judge)]);
        let (n1, p1) = one_post("2026-11-12");
        let (n2, p2) = one_post("2026-11-13");
        let i = inputs(vec![(&n1, p1), (&n2, p2)]);
        let e = run(&i, &m, &out, false).await.unwrap_err();
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
        let again = Budget::new(0.3, out.join("spend.jsonl"));
        assert!((again.spent.get() - m.budget.spent.get()).abs() < 1e-12);
        fs::remove_dir_all(out).unwrap();
    }

    /// Review M5: a stop between the writer and a judge keeps the paid
    /// writer lines, and the re-run calls only the missing judge; a
    /// complete post is skipped unless --fresh.
    #[tokio::test]
    async fn a_rerun_finishes_a_stopped_post_and_skips_done_ones() {
        let out = tmp("rerun");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 9000)]);
        let judge = Mock::new(vec![]);
        // $0.2 covers the writer's reservation, not the judge's after it.
        let m = mock_models(budget(&out, 0.2), &writer, &[("claude", &judge)]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        let e = run(&i, &m, &out, false).await.unwrap_err();
        assert!(e.contains("part-done"), "{e}");
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].insight.is_some());
        assert_eq!(lines[0].judges["claude"].error.as_deref(), Some(NOT_RUN));

        judge
            .replies
            .borrow_mut()
            .push_back(claude_body(&judge_reply(true), 200));
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        run(&i, &m, &out, false).await.unwrap();
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 1));
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].judges["claude"].passed);

        // Done: a third run makes no call; --fresh redoes it.
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        run(&i, &m, &out, false).await.unwrap();
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 1));
        writer
            .replies
            .borrow_mut()
            .push_back(claude_body(&writer_reply(CLEAN), 200));
        judge
            .replies
            .borrow_mut()
            .push_back(claude_body(&judge_reply(true), 200));
        run(&i, &m, &out, true).await.unwrap();
        assert_eq!((writer.calls.get(), judge.calls.get()), (2, 2));
        assert_eq!(read_jsonl::<Record>(&out.join("lines.jsonl")).len(), 1);
        fs::remove_dir_all(out).unwrap();
    }

    #[tokio::test]
    async fn lines_record_every_stage() {
        let out = tmp("every-stage");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 200)]);
        let judge = Mock::new(vec![deepseek_body(&judge_reply(true))]);
        let m = mock_models(budget(&out, 3.0), &writer, &[("deepseek", &judge)]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        run(&i, &m, &out, false).await.unwrap();
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
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        run(&i, &m, &out, false).await.unwrap();
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

    #[tokio::test]
    async fn a_writer_error_is_kept_on_its_mover() {
        // Review L5: a lowercase ticker in the reply still lands on its mover.
        let out = tmp("writer-error");
        let writer = Mock::new(vec![claude_body(
            &json!({"insights": [{"ticker": "aapl", "en": {}}, {"ticker": "TSLA"}]}),
            200,
        )]);
        let m = mock_models(budget(&out, 3.0), &writer, &[]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        run(&i, &m, &out, false).await.unwrap();
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert!(lines[0].insight.is_none());
        assert_eq!(
            lines[0].write_error.as_deref(),
            Some("AAPL: missing or incomplete en")
        );
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn a_failed_call_or_an_omitted_mover_is_not_an_answer() {
        let pack = Pack {
            ticker: "AAPL".into(),
            name: "Apple".into(),
            evidence: vec![],
        };
        let failed = judge_result(&Err("http status 500".into()), &pack, None, true);
        assert!(!failed.answered() && !failed.passed);
        let omitted = judge_result(&Ok(vec![]), &pack, None, true);
        assert!(!omitted.answered() && !omitted.passed);
    }

    #[tokio::test]
    async fn seed_sends_one_row_per_mover_per_call_with_controls() {
        let out = tmp("seed");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 200)]);
        let judge = Mock::new(vec![deepseek_body(&judge_reply(true))]);
        let m = mock_models(budget(&out, 3.0), &writer, &[("deepseek", &judge)]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        run(&i, &m, &out, false).await.unwrap();

        // Round 1 holds the control (passed again), round 2 the advice
        // variant (rejected: caught).
        judge
            .replies
            .borrow_mut()
            .push_back(deepseek_body(&judge_reply(true)));
        judge
            .replies
            .borrow_mut()
            .push_back(deepseek_body(&judge_reply(false)));
        seed(&m, &out, false).await.unwrap();
        assert_eq!(judge.calls.get(), 3);
        let rows: Vec<seed::Seeded> = read_jsonl(&out.join("seeded.jsonl"));
        let kinds: Vec<seed::Kind> = rows.iter().map(|r| r.kind).collect();
        // One number in each line and no other value cited: the number
        // swaps could only make a true line, so they are skipped and
        // counted; there is no second outlet or direction word to swap.
        assert_eq!(
            kinds,
            [
                seed::Kind::NumberSwap,
                seed::Kind::ZhOnlyNumber,
                seed::Kind::FormalAdvice,
                seed::Kind::Control
            ]
        );
        assert!(rows[0].skipped && rows[0].judges.is_empty());
        assert!(rows[1].skipped && rows[1].judges.is_empty());
        assert!(rows[2].check.is_empty(), "{:?}", rows[2].check);
        assert_eq!(rows[2].caught("deepseek"), Some(true));
        assert!(rows[3].judges["deepseek"].passed);
        assert_eq!(rows[2].ticker, "AAPL");
        assert_eq!(
            rows[2].judges["deepseek"].verdict.as_ref().unwrap().ticker,
            "AAPL"
        );
        assert!(rows[2].original_passed["deepseek"]);

        // A re-run skips the post; --plan-style estimates count nothing.
        seed(&m, &out, false).await.unwrap();
        assert_eq!(judge.calls.get(), 3);
        fs::remove_dir_all(out).unwrap();
    }

    #[tokio::test]
    async fn seed_stops_before_starting_when_the_plan_would_pass_the_cap() {
        let out = tmp("seed-plan");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 200)]);
        let judge = Mock::new(vec![claude_body(&judge_reply(true), 200)]);
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        run(&i, &m, &out, false).await.unwrap();
        let records: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        let plan = seed_plan(&seed::plan(&records), &out.join("spend.jsonl"));
        // Two rounds (control, advice) for each of the two judges.
        assert_eq!(plan.total_calls(), 4);
        assert!(plan.expected_usd > 0.0 && plan.worst_usd > plan.expected_usd);
        let spent = m.budget.spent.get();
        let tight = mock_models(
            budget(&out, spent + plan.expected_usd / 2.0),
            &writer,
            &[("claude", &judge)],
        );
        let e = seed(&tight, &out, false).await.unwrap_err();
        assert!(e.contains("not starting"), "{e}");
        assert_eq!(judge.calls.get(), 1);
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn the_run_plan_counts_one_writer_and_one_call_per_judge_per_post() {
        let (n1, p1) = one_post("2026-11-12");
        let (n2, p2) = one_post("2026-11-13");
        let i = inputs(vec![
            (&n1, p1),
            (&n2, p2),
            (
                "premarket_2026-11-14",
                premarket("2026-11-14", vec![mover("AAPL", vec![])]),
            ),
        ]);
        let dir = tmp("run-plan");
        let plan = run_plan(&i, &[], &dir.join("spend.jsonl"));
        let calls: Vec<((String, String), usize)> = plan.calls.clone().into_iter().collect();
        assert_eq!(
            calls,
            [
                (("judge".into(), CLAUDE_MODEL.into()), 2),
                (("judge".into(), DEEPSEEK_MODEL.into()), 2),
                (("writer".into(), CLAUDE_MODEL.into()), 2),
            ]
        );
        assert!(plan.describe("run").contains("6 calls"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn line_keys_change_with_the_text() {
        let i: Insight =
            serde_json::from_value(writer_reply(CLEAN)["insights"][0].clone()).unwrap();
        let mut j = i.clone();
        assert_eq!(line_key(&i), line_key(&j));
        j.zh.watch.push('。');
        assert_ne!(line_key(&i), line_key(&j));
        assert_eq!(line_key(&i).len(), 16);
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
        fs::write(
            dir.join("calendar.json"),
            r#"[["AAPL","2026-11-19","unconfirmed"],["AAPL","2026-08-01","quarantined"]]"#,
        )
        .unwrap();
        fs::write(dir.join("events.json"), r#"[["2026-11-13","October CPI"]]"#).unwrap();
        fs::write(dir.join("macro_events.rows.json"), "[]").unwrap();
        let i = load_inputs(&dir, Some("11-12"));
        assert_eq!(i.posts.len(), 1);
        assert_eq!(
            i.calendar,
            [(
                "AAPL".to_string(),
                date("2026-11-19"),
                ReportStatus::Unconfirmed
            )]
        );
        assert_eq!(
            i.macro_events,
            [(date("2026-11-13"), "October CPI".to_string())]
        );
        let (packs, movers) = i.packs(&i.posts[0].1);
        assert_eq!((packs.len(), movers), (1, 1));
        let kinds: Vec<_> = packs[0].evidence.iter().map(|e| e.kind).collect();
        use prebell_core::insight::EvidenceKind::*;
        assert_eq!(kinds, [Move, News, ReportDate, MacroEvent]);
        assert!(packs[0].evidence[2].unconfirmed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn events_are_titled_as_the_site_titles_them() {
        let rows = json!([
            {"et_date": "2026-11-13", "series": "cpi", "ref_period": "2026-10", "variant": null},
            {"et_date": "2026-11-14", "series": "unknown_series", "ref_period": null, "variant": null},
        ]);
        assert_eq!(events_json(&rows), json!([["2026-11-13", "October CPI"]]));
    }

    /// Round 2, M2: when seeding every post would pass what is left under
    /// the cap, a fixed-seed sample of whole posts that fits is taken, and
    /// the note says which and how many rows it sends.
    #[test]
    fn seed_samples_posts_when_the_plan_passes_the_cap() {
        use crate::page::tests::record;
        let records: Vec<Record> = (1..=6)
            .flat_map(|d| {
                let post = format!("recap_2026-11-0{d}");
                vec![
                    record(&post, &format!("A{d}"), (true, true)),
                    record(&post, &format!("B{d}"), (true, false)),
                ]
            })
            .collect();
        let dir = tmp("seed-fit");
        let ledger = dir.join("spend.jsonl");
        let plans = seed::plan(&records);
        let total = seed_plan(&plans, &ledger).expected_usd;
        let (all, _, note) = fit_seed(seed::plan(&records), &ledger, total + 0.01).unwrap();
        assert_eq!((all.len(), note), (6, None));
        let left = total / 2.0;
        let (kept, plan, note) = fit_seed(seed::plan(&records), &ledger, left).unwrap();
        let note = note.unwrap();
        assert!(!kept.is_empty() && kept.len() < 6, "{}", kept.len());
        assert!(plan.expected_usd <= left);
        assert!(note.contains("random sample (fixed seed)"), "{note}");
        assert!(note.contains(&format!("of {} posts", kept.len())), "{note}");
        assert!(
            note.contains("wrong lines and") && note.contains("controls"),
            "{note}"
        );
        for p in &kept {
            assert!(note.contains(&p.post), "{note}");
        }
        // The same sample every time.
        let (again, _, _) = fit_seed(seed::plan(&records), &ledger, left).unwrap();
        assert_eq!(
            kept.iter().map(|p| &p.post).collect::<Vec<_>>(),
            again.iter().map(|p| &p.post).collect::<Vec<_>>()
        );
        let e = fit_seed(seed::plan(&records), &ledger, 0.0).err().unwrap();
        assert!(e.contains("not starting"), "{e}");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Round 2, L5: a judge call that failed for a reason other than the
    /// budget is made again on a re-run; the writer is not.
    #[tokio::test]
    async fn a_failed_judge_call_is_retried_on_a_rerun() {
        let out = tmp("retry");
        let writer = Mock::new(vec![claude_body(&writer_reply(CLEAN), 200)]);
        let judge = Mock::new(vec![]);
        let m = mock_models(budget(&out, 3.0), &writer, &[("claude", &judge)]);
        let (n, p) = one_post("2026-11-12");
        let i = inputs(vec![(&n, p)]);
        // No reply queued: the judge call fails with a network error.
        run(&i, &m, &out, false).await.unwrap();
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert!(lines[0].judges["claude"].error.is_some());
        judge
            .replies
            .borrow_mut()
            .push_back(claude_body(&judge_reply(true), 200));
        run(&i, &m, &out, false).await.unwrap();
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 2));
        let lines: Vec<Record> = read_jsonl(&out.join("lines.jsonl"));
        assert!(lines[0].judges["claude"].passed);
        fs::remove_dir_all(out).unwrap();
    }
}
