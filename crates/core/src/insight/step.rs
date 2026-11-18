//! The `insight` queue job: the shadow stage of mover insights (Plan 2,
//! `docs/superpowers/plans/2026-11-17-mover-insights-shadow.md`). After a
//! premarket brief, its 8:35 update or a recap is published, the writer and
//! the code checks run over the post's evidence packs, then the judge, and
//! every line and verdict is stored in R2 under `insights/`. Readers see
//! nothing: no table the site reads is written.
//!
//! A paid reply is stored before any heavy work, so a message killed for CPU
//! never makes a paid call be paid again, and each message compiles only
//! the patterns it needs, before it pays (the lead's CPU ruling of
//! 2026-11-17). A bundle's record goes through these stages, each step in
//! its own queue message unless noted:
//!
//! 1. write (no record yet): build the packs (the pack patterns), claim a
//!    call, call the writer, store its reply as a `replied` record. No code
//!    check runs here.
//! 2. check (`replied`): read the stored reply, run the code checks (the
//!    check and number patterns), store `written` (or `done` when no line
//!    passed them).
//! 3. judge (`written`): compile the confirmation patterns (numbers and
//!    dates), claim a call, call the judge, store its reply as `judged`,
//!    then, in the same message, confirm the verdicts and store `done`.
//! 4. verdicts (`judged`, only when step 3 was killed after storing the
//!    reply): confirm the stored verdicts and store `done`.
//!
//! A tick does the next step of the oldest unfinished record first, else
//! the write step of the newest bundle without a record.
//!
//! Bounded like the share cards: at most `MAX_ATTEMPTS` write tries per
//! bundle, judge tries per record, and tries of a code-only step on a stored
//! reply, each counted before the work it guards (so a message killed for CPU or wall time still counts), and at
//! most the monthly cap of model calls, each counted before it is made. A
//! failure never alerts and never touches a post.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::judge::{self, Verdict, JUDGE_VERSION};
use super::pack::{self, PackInputs, ReportStatus};
use super::write::{self, WRITER_VERSION};
use super::{check, Insight, Pack};
use crate::clock::{et, trading_day, DayKind};
use crate::cover::Session;
use crate::earnings::facts::EarningsFacts;
use crate::facts::{Mover, PremarketFacts, Theme};
use crate::providers::LlmProvider;
use crate::recap::RecapFacts;

/// `INSIGHTS_MODE`. `On` (showing lines to readers) is Plan 3's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Shadow,
}

/// `INSIGHTS_MODE`: unset or `off` is `Off`, `shadow` is `Shadow`, anything
/// else is a config error (the step then does nothing).
pub fn mode(raw: Option<&str>) -> Result<Mode, String> {
    match raw {
        None | Some("off") => Ok(Mode::Off),
        Some("shadow") => Ok(Mode::Shadow),
        Some(other) => Err(format!(
            "unknown INSIGHTS_MODE: {other} (use off or shadow)"
        )),
    }
}

/// Model calls a calendar month (ET) when `INSIGHT_MONTHLY_CALLS` is unset:
/// $2.40 to $6 a month at the trial's measured $0.008 to $0.02 a call.
pub const MONTHLY_CALLS_DEFAULT: u32 = 300;
/// Write tries per facts bundle, and judge tries per record.
pub const MAX_ATTEMPTS: u32 = 3;
pub const RECORD_VERSION: &str = "insight-record/v1";
/// Writer and judge effort (Claude).
pub const EFFORT: &str = "low";
/// Attempt counters a write stage reads at most per tick, newest bundle
/// first, so a run of given-up bundles cannot push a tick past its
/// subrequest budget (see `insight_tick`).
const ATTEMPT_READS: usize = 3;

/// The step's own settings, independent of `LLM_PROVIDER` and `LLM_MODEL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `claude` or `deepseek`.
    pub writer: &'static str,
    pub judge: &'static str,
    pub writer_model: &'static str,
    pub judge_model: &'static str,
    /// Model calls allowed a calendar month (ET).
    pub cap: u32,
}

fn provider(var: &str, raw: Option<&str>) -> Result<(&'static str, &'static str), String> {
    match raw.unwrap_or("claude") {
        "claude" => Ok(("claude", crate::claude::DEFAULT_MODEL)),
        "deepseek" => Ok(("deepseek", crate::llm::DEEPSEEK_DEFAULT_MODEL)),
        other => Err(format!("unknown {var}: {other} (use claude or deepseek)")),
    }
}

/// `INSIGHT_WRITER_PROVIDER`, `INSIGHT_JUDGE_PROVIDER` (both default
/// `claude`) and `INSIGHT_MONTHLY_CALLS` (default `MONTHLY_CALLS_DEFAULT`).
pub fn config(
    writer: Option<&str>,
    judge: Option<&str>,
    monthly_calls: Option<&str>,
) -> Result<Config, String> {
    let (writer, writer_model) = provider("INSIGHT_WRITER_PROVIDER", writer)?;
    let (judge, judge_model) = provider("INSIGHT_JUDGE_PROVIDER", judge)?;
    let cap = match monthly_calls {
        None => MONTHLY_CALLS_DEFAULT,
        Some(raw) => raw
            .trim()
            .parse()
            .map_err(|_| format!("INSIGHT_MONTHLY_CALLS is not a whole number of calls: {raw}"))?,
    };
    Ok(Config {
        writer,
        judge,
        writer_model,
        judge_model,
        cap,
    })
}

/// Whether a cron firing at `now` is in a sweep window: 08:10-09:30 or
/// 16:10-17:30 ET on a trading day.
pub fn sweep_due(now: DateTime<Utc>) -> bool {
    let t = et(now);
    if trading_day(t.date_naive()) == DayKind::Closed {
        return false;
    }
    let hm = (t.hour(), t.minute());
    ((8, 10)..=(9, 30)).contains(&hm) || ((16, 10)..=(17, 30)).contains(&hm)
}

/// Whether a cron firing at `now` queues an `insight` message: in a sweep
/// window, unless the mode is off. A bad mode queues one, so the message
/// reports the config error.
pub fn sweep_wanted(raw_mode: Option<&str>, now: DateTime<Utc>) -> bool {
    !matches!(mode(raw_mode), Ok(Mode::Off)) && sweep_due(now)
}

/// A published premarket brief or recap, as the step reads it from `posts`.
#[derive(Debug, Clone, PartialEq)]
pub struct PostRef {
    /// `premarket` or `recap`.
    pub content_type: String,
    /// ET `YYYY-MM-DD`.
    pub trading_date: String,
    pub slug: String,
    /// The bundle the post shows now: after the 8:35 update, the update's.
    pub facts_key: String,
}

/// A published earnings check of one of a post's movers, from `posts` and
/// `earnings_filings`.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRef {
    pub ticker: String,
    /// ET `YYYY-MM-DD`.
    pub filing_date: String,
    /// RFC 3339.
    pub accepted_at: String,
    pub facts_key: String,
}

/// What the step reads and writes (D1 and R2 in production). It writes only
/// R2 keys under `insights/` and the kv rows `insight_calls:*`.
#[async_trait(?Send)]
pub trait InsightStore {
    /// Published `premarket` and `recap` posts with a trading date of
    /// `since` (YYYY-MM-DD) or later, in one query.
    async fn insight_posts(&self, since: &str) -> Result<Vec<PostRef>, String>;
    /// An R2 object's text; `None` when it does not exist.
    async fn get_text(&self, key: &str) -> Result<Option<String>, String>;
    async fn put_text(&self, key: &str, text: &str) -> Result<(), String>;
    /// `(ticker, report date, status)` calendar rows for `tickers` with a
    /// report date from `from` to `to`, quarantined rows left out, in one
    /// query.
    async fn calendar_rows(
        &self,
        tickers: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<(String, String, String)>, String>;
    /// Published earnings checks of `tickers` filed from `from` to `to`, in
    /// one query.
    async fn earnings_checks(
        &self,
        tickers: &[String],
        from: &str,
        to: &str,
    ) -> Result<Vec<CheckRef>, String>;
    async fn kv_get(&self, key: &str) -> Result<Option<String>, String>;
    /// Adds one to the count in kv `key` (a missing row is 0) when it is
    /// below `cap`, in one statement; the new count, or `None` at the cap.
    async fn kv_claim(&self, key: &str, cap: u32, now: &str) -> Result<Option<u32>, String>;
}

/// The R2 key of a facts bundle's record: `insights/<facts key without
/// ".json">.json`, e.g. `insights/premarket/2026-11-17-update.json`.
pub fn record_key(facts_key: &str) -> String {
    format!("insights/{}.json", stem(facts_key))
}

fn stem(facts_key: &str) -> &str {
    facts_key.strip_suffix(".json").unwrap_or(facts_key)
}

/// The R2 key counting a bundle's write tries (never served).
fn attempts_key(facts_key: &str) -> String {
    format!("insights/{}.attempts", stem(facts_key))
}

/// The kv row counting this ET month's model calls.
fn calls_key(now: DateTime<Utc>) -> String {
    format!("insight_calls:{}", et(now).format("%Y-%m"))
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// The writer's reply is stored; the code checks have not run.
    Replied,
    /// The code checks ran; the judge has not answered.
    Written,
    /// The judge's reply is stored; its verdicts are not confirmed yet.
    Judged,
    /// Final: never overwritten.
    Done,
}

/// One eligible mover in a record.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoverRecord {
    pub ticker: String,
    pub pack: Pack,
    /// `None` when the writer left the mover out or its item was unusable.
    pub insight: Option<Insight>,
    pub write_error: Option<String>,
    /// The code checks' refusals; empty when the line went to the judge.
    pub check: Vec<String>,
    pub verdict: Option<Verdict>,
    pub judge_error: Option<String>,
    /// The judge passed it and code confirmed the judge's support.
    pub passed: bool,
}

/// The audit record of one facts bundle (one per bundle: the 8:00 brief and
/// its 8:35 update each have one).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Record {
    pub version: String,
    pub stage: Stage,
    pub judge_attempts: u32,
    pub post_slug: String,
    pub content_type: String,
    pub trading_date: String,
    pub facts_key: String,
    /// RFC 3339: when the record was last written.
    pub at: String,
    pub writer_version: String,
    pub judge_version: String,
    pub writer_model: String,
    pub judge_model: String,
    pub movers: Vec<MoverRecord>,
    /// The writer's reply as it came, stored before anything else ran on it.
    pub writer_reply: Option<Value>,
    /// The judge's reply as it came, stored before its verdicts are confirmed.
    pub judge_reply: Option<Value>,
    /// Tries of the code-only step on a stored reply (the checks, or the
    /// confirmation), counted before it runs; 0 again at each new stage.
    pub work_attempts: u32,
    /// Model calls made for this record.
    pub calls: u32,
    /// Why the record is final without every line judged ("no eligible
    /// mover", "no line passed the code checks", ...).
    pub skipped_reason: Option<String>,
}

impl Record {
    /// The movers whose line passed the code checks: what the judge sees.
    fn to_judge(&self) -> (Vec<Pack>, Vec<Insight>) {
        self.movers
            .iter()
            .filter(|m| m.check.is_empty())
            .filter_map(|m| m.insight.clone().map(|i| (m.pack.clone(), i)))
            .unzip()
    }
}

/// What one `insight` message did.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub line: String,
    /// Queue one more `insight` message now: a record waits for its judge.
    pub follow_up: bool,
}

fn tick(line: String) -> Tick {
    Tick {
        line,
        follow_up: false,
    }
}

fn iso(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A bundle the step may write a record for.
#[derive(Debug, Clone)]
struct Candidate {
    facts_key: String,
    post: PostRef,
}

/// Every bundle of `posts`, newest first: by trading date, the recap before
/// the brief, the 8:35 update before the 8:00 brief. The posts table points
/// only at the update's bundle once it runs, so the 8:00 one is named by
/// its fixed key (`premarket/<date>.json`).
fn candidates(mut posts: Vec<PostRef>) -> Vec<Candidate> {
    posts.sort_by(|a, b| {
        (&b.trading_date, b.content_type == "recap")
            .cmp(&(&a.trading_date, a.content_type == "recap"))
    });
    let mut out: Vec<Candidate> = Vec::new();
    for p in posts {
        let mut keys = vec![p.facts_key.clone()];
        if p.content_type == "premarket" {
            let base = format!("premarket/{}.json", p.trading_date);
            if base != p.facts_key {
                keys.push(base);
            }
        }
        for k in keys {
            if !out.iter().any(|c| c.facts_key == k) {
                out.push(Candidate {
                    facts_key: k,
                    post: p.clone(),
                });
            }
        }
    }
    out
}

/// A post's facts, as the pack builder needs them.
struct Post {
    session: Session,
    trading_date: NaiveDate,
    as_of_utc: DateTime<Utc>,
    movers: Vec<Mover>,
    themes: Vec<Theme>,
}

fn post_facts(content_type: &str, json: &str) -> Result<Post, String> {
    let gl = |g: Vec<Mover>, l: Vec<Mover>| g.into_iter().chain(l).collect();
    match content_type {
        "premarket" => {
            let f: PremarketFacts = serde_json::from_str(json).map_err(|e| e.to_string())?;
            Ok(Post {
                session: Session::Premarket,
                trading_date: f.trading_date,
                as_of_utc: f.as_of_utc,
                movers: gl(f.gainers, f.losers),
                themes: f.themes,
            })
        }
        "recap" => {
            let f: RecapFacts = serde_json::from_str(json).map_err(|e| e.to_string())?;
            Ok(Post {
                session: Session::Recap,
                trading_date: f.trading_date,
                as_of_utc: f.as_of_utc,
                movers: gl(f.gainers, f.losers),
                themes: f.themes,
            })
        }
        other => Err(format!("not a brief or recap: {other}")),
    }
}

/// For each ticker, the newest check accepted before `cutoff`: the one
/// `pack::packs` would use, so at most one bundle is read per mover.
fn chosen_checks(refs: Vec<CheckRef>, cutoff: DateTime<Utc>) -> Vec<CheckRef> {
    let mut best: Vec<(DateTime<Utc>, CheckRef)> = Vec::new();
    for r in refs {
        let Ok(at) = DateTime::parse_from_rfc3339(&r.accepted_at) else {
            continue;
        };
        let at = at.with_timezone(&Utc);
        if at >= cutoff {
            continue;
        }
        match best
            .iter_mut()
            .find(|(_, b)| b.ticker.eq_ignore_ascii_case(&r.ticker))
        {
            Some(slot) if slot.0 < at => *slot = (at, r),
            Some(_) => {}
            None => best.push((at, r)),
        }
    }
    best.into_iter().map(|(_, r)| r).collect()
}

/// The month's count before a write stage counts an attempt: at the cap,
/// nothing is tried.
async fn under_cap(now: DateTime<Utc>, store: &dyn InsightStore, cap: u32) -> Result<bool, String> {
    let used: u32 = match store.kv_get(&calls_key(now)).await? {
        Some(v) => v.trim().parse().unwrap_or(cap),
        None => 0,
    };
    Ok(used < cap)
}

/// Counts one model call this month; `false` at the cap.
async fn claim_call(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    cap: u32,
) -> Result<bool, String> {
    if cap == 0 {
        return Ok(false);
    }
    Ok(store
        .kv_claim(&calls_key(now), cap, &iso(now))
        .await?
        .is_some())
}

/// The line for a tick stopped by the cap, which says so in full once a
/// month (the first tick to find it).
async fn capped(now: DateTime<Utc>, store: &dyn InsightStore, cap: u32) -> Result<Tick, String> {
    let month = et(now).format("%Y-%m");
    let first = store
        .kv_claim(&format!("{}:capped", calls_key(now)), 1, &iso(now))
        .await?
        .is_some();
    Ok(tick(if first {
        format!(
            "monthly cap of {cap} model calls reached for {month}; insights stop until next month"
        )
    } else {
        "at the monthly cap; skipped".into()
    }))
}

/// Writes `rec` unless a record is already there; whether it wrote. Two
/// messages racing on one bundle may both pay the writer; the first record
/// stands.
async fn put_new(store: &dyn InsightStore, rec: &Record) -> Result<bool, String> {
    let key = record_key(&rec.facts_key);
    if store.get_text(&key).await?.is_some() {
        return Ok(false);
    }
    let json = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    store.put_text(&key, &json).await?;
    Ok(true)
}

/// The stage and counters of a stored record, read without the rest of it
/// (unknown fields are skipped): what a tick needs to choose its work, and
/// what a write compares before it replaces the record.
#[derive(Deserialize, Debug, Clone)]
struct Head {
    version: String,
    stage: Stage,
    #[serde(default)]
    at: String,
    #[serde(default)]
    judge_attempts: u32,
    #[serde(default)]
    work_attempts: u32,
}

impl Head {
    fn read(text: &str) -> Option<Head> {
        serde_json::from_str::<Head>(text)
            .ok()
            .filter(|h| h.version == RECORD_VERSION)
    }
    fn seen(&self) -> Seen {
        Seen {
            stage: self.stage,
            judge_attempts: self.judge_attempts,
            work_attempts: self.work_attempts,
        }
    }
}

/// Each bundle and its stored record, by head and text: `None`, no record;
/// `Some(None)`, a record left alone (another version, or unreadable).
type Found = Vec<(Candidate, Option<Option<(Head, String)>>)>;

/// What this tick last saw of a stored record. A write replaces the record
/// only when the stored one still matches: never a later stage, a stored
/// reply or another run's count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seen {
    stage: Stage,
    judge_attempts: u32,
    work_attempts: u32,
}

impl Seen {
    fn of(r: &Record) -> Seen {
        Seen {
            stage: r.stage,
            judge_attempts: r.judge_attempts,
            work_attempts: r.work_attempts,
        }
    }
}

/// Replaces the stored record with `rec` when it still is what this tick
/// saw (`seen`), then remembers `rec` as seen; whether it wrote. On a
/// mismatch nothing is written: another run moved the record on.
async fn put_if_seen(
    store: &dyn InsightStore,
    rec: &Record,
    seen: &mut Seen,
) -> Result<bool, String> {
    let key = record_key(&rec.facts_key);
    let stored = store.get_text(&key).await?;
    if stored.as_deref().and_then(Head::read).map(|h| h.seen()) != Some(*seen) {
        return Ok(false);
    }
    let json = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    store.put_text(&key, &json).await?;
    *seen = Seen::of(rec);
    Ok(true)
}

/// Stores a paid judge reply (`rec` at `judged`) when the stored record is
/// still `written`, whatever another run's try count says (the higher count
/// is kept); whether it wrote. A reply is never written over a later stage
/// or another stored reply.
async fn put_reply(
    store: &dyn InsightStore,
    rec: &mut Record,
    seen: &mut Seen,
) -> Result<bool, String> {
    let key = record_key(&rec.facts_key);
    let stored = store.get_text(&key).await?;
    let Some(h) = stored.as_deref().and_then(Head::read) else {
        return Ok(false);
    };
    if h.stage != Stage::Written {
        return Ok(false);
    }
    rec.judge_attempts = rec.judge_attempts.max(h.judge_attempts);
    let json = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    store.put_text(&key, &json).await?;
    *seen = Seen::of(rec);
    Ok(true)
}

/// How long a record or attempt counter another message just wrote is left
/// to it by sweeps: a chain's own messages carry on, a sweep does not cut in.
pub const LEASE_MINUTES: i64 = 5;

/// Whether `at` (RFC 3339) is within the lease before `now`.
fn leased(at: &str, now: DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(at)
        .is_ok_and(|t| now.signed_duration_since(t) < chrono::Duration::minutes(LEASE_MINUTES))
}

/// An attempt counter: `"<n> <RFC 3339 time>"` (a bare `"<n>"` reads as
/// written long ago).
fn read_attempts(text: &str) -> (u32, Option<String>) {
    let mut parts = text.split_whitespace();
    let n = parts
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(MAX_ATTEMPTS);
    (n, parts.next().map(String::from))
}

/// The line of a step that closed its record: one more message when another
/// bundle still has no record, so a publish-queued message spent on an older
/// record does not leave the new one for the next sweep.
fn closed(line: String, others_waiting: bool) -> Tick {
    Tick {
        line,
        follow_up: others_waiting,
    }
}

/// The patterns the write step compiles, before it pays: the pack builder's.
pub fn warm_write() -> usize {
    pack::warm()
}

/// The patterns the check step compiles: the code checks' and the numbers'.
pub fn warm_check() -> usize {
    check::warm() + crate::numbers::warm()
}

/// The patterns the judge step compiles before it pays, and the verdict
/// step uses: the numbers' and the date marks (`judge::passed`).
pub fn warm_judge() -> usize {
    check::warm_dates(NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"));
    crate::numbers::warm() + 1
}

/// One `insight` message. `Ok` is a log line and whether to queue another
/// message; `Err` a store error (logged, never alerted; the next sweep
/// tries again within the attempt limits).
///
/// Subrequests in the worst case (each D1 query, R2 get or put, model call
/// and queue send is one, the follow-up send in `jobs::run_job` included):
/// the posts query and a record read per bundle of today and the previous
/// trading day (at most 6), then one step and the send:
/// - write: at most `ATTEMPT_READS` attempt counters, the month's count,
///   the attempt write, the facts, the calendar and earnings-check queries,
///   one earnings bundle per mover (at most 8), the cap claim, the writer
///   call, a re-read and the write, the send: 28;
/// - check: a re-read and a write to count the try, a re-read and the
///   write, up to `ATTEMPT_READS` attempt counters to tell whether another
///   bundle waits, the send: 15;
/// - judge: the month's count, a re-read and a write to count the try, the
///   cap claim, the judge call, a re-read and the write of the reply, a
///   re-read and the final write, the attempt counters, the send: 20;
/// - verdicts: as check, 15.
///
/// At most 5 are D1 queries and 1 is a fetch. Alert delivery runs after the
/// step in the same invocation (its 2 D1 queries and up to 20 webhook
/// posts): after a write message the total can reach exactly 50, and only
/// when 20 or more alerts are pending.
pub async fn insight_tick(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    writer: &dyn LlmProvider,
    judge: &dyn LlmProvider,
    cfg: &Config,
    sweep: bool,
) -> Result<Tick, String> {
    // Today and the previous trading day: a record a Friday left open is
    // finished on Monday, and the same across a market holiday.
    let since = crate::why::previous_trading_day(et(now).date_naive())
        .format("%Y-%m-%d")
        .to_string();
    let cands = candidates(store.insight_posts(&since).await?);
    if cands.is_empty() {
        return Ok(tick("insight: no brief or recap to read".into()));
    }
    // Each bundle's record, by its head only: `None`, no record; `Some(None)`,
    // a record of another version or unreadable, left alone like a done one.
    let mut found: Found = Vec::new();
    for c in cands {
        let rec = store
            .get_text(&record_key(&c.facts_key))
            .await?
            .map(|t| Head::read(&t).map(|h| (h, t)));
        found.push((c, rec));
    }
    // The next step of the oldest unfinished record first; a sweep leaves a
    // record another message touched within the lease to that message. A
    // record whose head reads but whose body does not (a shape changed
    // without a new version) is left alone like one of another version,
    // and named in the line, so it never stops the bundles behind it.
    let mut unreadable: Vec<String> = Vec::new();
    for (c, r) in found.iter().rev() {
        let Some(Some((h, text))) = r else {
            continue;
        };
        if h.stage == Stage::Done || sweep && leased(&h.at, now) {
            continue;
        }
        let rec: Record = match serde_json::from_str(text) {
            Ok(rec) => rec,
            Err(e) => {
                unreadable.push(format!("{}: {e}", c.facts_key));
                continue;
            }
        };
        let others = waiting(store, &found, &c.facts_key).await?;
        let t = match rec.stage {
            Stage::Replied => check_step(now, store, rec, others).await,
            Stage::Written => judge_step(now, store, judge, cfg, rec, others).await,
            Stage::Judged => verdict_step(now, store, rec, others).await,
            Stage::Done => unreachable!("an open record is not done"),
        }?;
        return Ok(noted(t, &unreadable));
    }
    // Else the write step of the newest bundle without a record.
    let mut read = 0;
    for (c, rec) in &found {
        if rec.is_some() {
            continue;
        }
        if read == ATTEMPT_READS {
            break;
        }
        read += 1;
        let (tried, at) = match store.get_text(&attempts_key(&c.facts_key)).await? {
            Some(t) => read_attempts(&t),
            None => (0, None),
        };
        if sweep && at.as_deref().is_some_and(|a| leased(a, now)) {
            continue;
        }
        if tried < MAX_ATTEMPTS {
            let t = write_step(now, store, writer, cfg, c, tried).await?;
            return Ok(noted(t, &unreadable));
        }
    }
    Ok(noted(tick("insight: nothing to do".into()), &unreadable))
}

/// `t` with the unreadable records this tick left alone named in its line.
fn noted(mut t: Tick, unreadable: &[String]) -> Tick {
    if !unreadable.is_empty() {
        t.line = format!(
            "{}; unreadable record left alone: {}",
            t.line,
            unreadable.join("; ")
        );
    }
    t
}

/// Whether a bundle other than `except` still waits for its write step: no
/// record, and write tries left (a bundle whose tries are used up waits for
/// nothing). Reads at most `ATTEMPT_READS` attempt counters.
async fn waiting(store: &dyn InsightStore, found: &Found, except: &str) -> Result<bool, String> {
    let mut read = 0;
    for (c, r) in found {
        if r.is_some() || c.facts_key == except {
            continue;
        }
        if read == ATTEMPT_READS {
            break;
        }
        read += 1;
        let tried = match store.get_text(&attempts_key(&c.facts_key)).await? {
            Some(t) => read_attempts(&t).0,
            None => 0,
        };
        if tried < MAX_ATTEMPTS {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Closes a record whose code-only step was tried `MAX_ATTEMPTS` times, or
/// counts one more try; `Ok(None)` when it may go on.
async fn count_work(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    rec: &mut Record,
    seen: &mut Seen,
    what: &str,
    others: bool,
) -> Result<Option<Tick>, String> {
    let key = rec.facts_key.clone();
    if rec.work_attempts >= MAX_ATTEMPTS {
        rec.stage = Stage::Done;
        rec.skipped_reason = Some(format!("{what} did not finish in {MAX_ATTEMPTS} tries"));
        rec.at = iso(now);
        if !put_if_seen(store, rec, seen).await? {
            return Ok(Some(changed(&key)));
        }
        return Ok(Some(closed(
            format!("insight {key}: {what} given up after {MAX_ATTEMPTS} tries"),
            others,
        )));
    }
    rec.work_attempts += 1;
    rec.at = iso(now);
    if !put_if_seen(store, rec, seen).await? {
        return Ok(Some(changed(&key)));
    }
    Ok(None)
}

/// The line of a tick whose record another run moved on: it wrote nothing.
fn changed(key: &str) -> Tick {
    tick(format!(
        "insight {key}: record changed by another run; left to it"
    ))
}

async fn write_step(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    writer: &dyn LlmProvider,
    cfg: &Config,
    c: &Candidate,
    tried: u32,
) -> Result<Tick, String> {
    let key = &c.facts_key;
    if !under_cap(now, store, cfg.cap).await? {
        return capped(now, store, cfg.cap).await;
    }
    // Counted first, so a message killed mid-way still counts.
    store
        .put_text(&attempts_key(key), &format!("{} {}", tried + 1, iso(now)))
        .await?;
    let attempt = format!("attempt {} of {MAX_ATTEMPTS}", tried + 1);
    warm_write();
    let Some(json) = store.get_text(key).await? else {
        return Ok(tick(format!("insight {key}: facts missing; {attempt}")));
    };
    let post = match post_facts(&c.post.content_type, &json) {
        Ok(p) => p,
        Err(e) => {
            return Ok(tick(format!(
                "insight {key}: facts unreadable: {e}; {attempt}"
            )))
        }
    };
    let td = post.trading_date;
    // A brief or recap lists at most 8 movers; the bound keeps the earnings
    // reads at one per mover, 8 at most, whatever the facts hold.
    let tickers: Vec<String> = post
        .movers
        .iter()
        .take(8)
        .map(|m| m.quote.symbol.clone())
        .collect();
    let (from, to) = pack::report_window(td);
    let calendar: Vec<(String, NaiveDate, ReportStatus)> = store
        .calendar_rows(&tickers, &from.to_string(), &to.to_string())
        .await?
        .into_iter()
        .filter_map(|(t, d, s)| Some((t, d.parse().ok()?, ReportStatus::parse(&s)?)))
        .collect();
    let (from, to, cutoff) = pack::check_window(post.session, td, post.as_of_utc);
    let refs = store
        .earnings_checks(&tickers, &from.to_string(), &to.to_string())
        .await?;
    let mut checks: Vec<EarningsFacts> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for r in chosen_checks(refs, cutoff) {
        match store.get_text(&r.facts_key).await? {
            Some(t) => match serde_json::from_str(&t) {
                Ok(f) => checks.push(f),
                Err(e) => notes.push(format!("{} unreadable: {e}", r.facts_key)),
            },
            None => notes.push(format!("{} missing", r.facts_key)),
        }
    }
    let packs = pack::packs(&PackInputs {
        session: post.session,
        trading_date: td,
        as_of_utc: post.as_of_utc,
        movers: &post.movers,
        themes: &post.themes,
        checks: &checks,
        calendar: &calendar,
    });
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(" ({})", notes.join("; "))
    };
    let mut rec = Record {
        version: RECORD_VERSION.into(),
        stage: Stage::Done,
        judge_attempts: 0,
        post_slug: c.post.slug.clone(),
        content_type: c.post.content_type.clone(),
        trading_date: c.post.trading_date.clone(),
        facts_key: key.clone(),
        at: iso(now),
        writer_version: WRITER_VERSION.into(),
        judge_version: JUDGE_VERSION.into(),
        writer_model: cfg.writer_model.into(),
        judge_model: cfg.judge_model.into(),
        movers: packs
            .into_iter()
            .map(|p| MoverRecord {
                ticker: p.ticker.clone(),
                pack: p,
                insight: None,
                write_error: None,
                check: vec![],
                verdict: None,
                judge_error: None,
                passed: false,
            })
            .collect(),
        writer_reply: None,
        judge_reply: None,
        work_attempts: 0,
        calls: 0,
        skipped_reason: None,
    };
    if rec.movers.is_empty() {
        rec.skipped_reason = Some("no eligible mover".into());
        let line = if put_new(store, &rec).await? {
            format!("insight {key}: no eligible mover; done{notes}")
        } else {
            format!("insight {key}: record already written by another run")
        };
        return Ok(tick(line));
    }
    if !claim_call(now, store, cfg.cap).await? {
        return capped(now, store, cfg.cap).await;
    }
    let packs: Vec<Pack> = rec.movers.iter().map(|m| m.pack.clone()).collect();
    let (system, user) = write::prompt(&packs);
    let reply = match writer.complete_json(&system, &user).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(tick(format!(
                "insight {key}: writer call failed: {e}; {attempt}{notes}"
            )))
        }
    };
    // Stored as it came, before any check runs on it: a message killed from
    // here on never makes the writer be paid again.
    rec.stage = Stage::Replied;
    rec.writer_reply = Some(reply);
    rec.calls = 1;
    if !put_new(store, &rec).await? {
        return Ok(tick(format!(
            "insight {key}: record already written by another run"
        )));
    }
    Ok(Tick {
        line: format!(
            "insight {key}: {} eligible, writer replied; checks queued{notes}",
            rec.movers.len()
        ),
        follow_up: true,
    })
}

/// The code checks over a stored writer reply.
async fn check_step(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    mut rec: Record,
    others: bool,
) -> Result<Tick, String> {
    let key = rec.facts_key.clone();
    let mut seen = Seen::of(&rec);
    if let Some(t) = count_work(now, store, &mut rec, &mut seen, "the code checks", others).await? {
        return Ok(t);
    }
    warm_check();
    let packs: Vec<Pack> = rec.movers.iter().map(|m| m.pack.clone()).collect();
    let reply = rec.writer_reply.clone().unwrap_or(Value::Null);
    let mut insights: Vec<Insight> = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();
    for item in write::parse(&reply, &packs) {
        match item {
            Ok(i) => insights.push(i),
            Err(e) => {
                let owner = packs.iter().find(|p| {
                    e.to_ascii_uppercase()
                        .starts_with(&format!("{}:", p.ticker.to_ascii_uppercase()))
                });
                if let Some(p) = owner {
                    if !errors.iter().any(|(t, _)| *t == p.ticker) {
                        errors.push((p.ticker.clone(), e));
                    }
                }
            }
        }
    }
    for m in rec.movers.iter_mut() {
        m.insight = insights.iter().find(|i| i.ticker == m.ticker).cloned();
        m.check = m
            .insight
            .as_ref()
            .map(|i| check::check(&m.pack, i))
            .unwrap_or_default();
        m.write_error = errors
            .iter()
            .find(|(t, _)| *t == m.ticker && m.insight.is_none())
            .map(|(_, e)| e.clone());
    }
    let to_judge = rec.to_judge().1.len();
    if to_judge > 0 {
        rec.stage = Stage::Written;
    } else {
        rec.stage = Stage::Done;
        rec.skipped_reason = Some("no line passed the code checks".into());
    }
    rec.work_attempts = 0;
    rec.at = iso(now);
    if !put_if_seen(store, &rec, &mut seen).await? {
        return Ok(changed(&key));
    }
    let written = rec.movers.iter().filter(|m| m.insight.is_some()).count();
    Ok(Tick {
        line: format!(
            "insight {key}: {} eligible, {written} written, {to_judge} passed the checks; {}",
            rec.movers.len(),
            if to_judge > 0 { "judge queued" } else { "done" }
        ),
        follow_up: to_judge > 0 || others,
    })
}

async fn judge_step(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    judge_llm: &dyn LlmProvider,
    cfg: &Config,
    mut rec: Record,
    others: bool,
) -> Result<Tick, String> {
    let key = rec.facts_key.clone();
    let mut seen = Seen::of(&rec);
    if rec.judge_attempts >= MAX_ATTEMPTS {
        // The last try was killed before it could record its outcome.
        rec.stage = Stage::Done;
        rec.skipped_reason = Some(format!("the judge failed {MAX_ATTEMPTS} times"));
        rec.at = iso(now);
        if !put_if_seen(store, &rec, &mut seen).await? {
            return Ok(changed(&key));
        }
        return Ok(closed(
            format!("insight {key}: judge given up after {MAX_ATTEMPTS} tries"),
            others,
        ));
    }
    // At the cap, nothing is tried.
    if !under_cap(now, store, cfg.cap).await? {
        return capped(now, store, cfg.cap).await;
    }
    // Counted first, in the record itself, so a message killed mid-way still
    // counts; a record another run finished is left alone.
    rec.judge_attempts += 1;
    // The judge that answers, as this deploy runs it (a deploy between the
    // write and the judge changes them).
    rec.judge_version = JUDGE_VERSION.into();
    rec.judge_model = cfg.judge_model.into();
    rec.at = iso(now);
    if !put_if_seen(store, &rec, &mut seen).await? {
        return Ok(changed(&key));
    }
    // Compiled before the call is claimed and made, so a kill from a cold
    // compile costs this try, not a call.
    warm_judge();
    if !claim_call(now, store, cfg.cap).await? {
        return capped(now, store, cfg.cap).await;
    }
    rec.calls += 1;
    let attempt = format!("judge attempt {} of {MAX_ATTEMPTS}", rec.judge_attempts);
    let (packs, insights) = rec.to_judge();
    let (system, user) = judge::prompt(&packs, &insights);
    match judge_llm.complete_json_strict(&system, &user).await {
        Ok(reply) => {
            // Stored as it came, before its verdicts are confirmed, with the
            // first confirmation try counted: a kill from here on never makes
            // the judge be paid again.
            rec.stage = Stage::Judged;
            rec.judge_reply = Some(reply);
            rec.work_attempts = 1;
            rec.at = iso(now);
            if !put_reply(store, &mut rec, &mut seen).await? {
                return Ok(changed(&key));
            }
            confirm(now, store, rec, seen, others).await
        }
        Err(e) => {
            let sent = |m: &MoverRecord| m.check.is_empty() && m.insight.is_some();
            for m in rec.movers.iter_mut().filter(|m| sent(m)) {
                m.judge_error = Some(e.to_string());
            }
            let given_up = rec.judge_attempts >= MAX_ATTEMPTS;
            let line = if given_up {
                rec.stage = Stage::Done;
                rec.skipped_reason = Some(format!("the judge failed {MAX_ATTEMPTS} times"));
                format!("insight {key}: judge call failed: {e}; {attempt}; given up")
            } else {
                format!("insight {key}: judge call failed: {e}; {attempt}")
            };
            rec.at = iso(now);
            if !put_if_seen(store, &rec, &mut seen).await? {
                return Ok(changed(&key));
            }
            Ok(closed(line, given_up && others))
        }
    }
}

/// The confirmation of a stored judge reply, when the judge step was killed
/// after storing it.
async fn verdict_step(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    mut rec: Record,
    others: bool,
) -> Result<Tick, String> {
    let mut seen = Seen::of(&rec);
    if let Some(t) = count_work(
        now,
        store,
        &mut rec,
        &mut seen,
        "the verdict confirmation",
        others,
    )
    .await?
    {
        return Ok(t);
    }
    warm_judge();
    confirm(now, store, rec, seen, others).await
}

/// Confirms the stored judge reply's verdicts and closes the record.
async fn confirm(
    now: DateTime<Utc>,
    store: &dyn InsightStore,
    mut rec: Record,
    mut seen: Seen,
    others: bool,
) -> Result<Tick, String> {
    let key = rec.facts_key.clone();
    let (_, insights) = rec.to_judge();
    let reply = rec.judge_reply.clone().unwrap_or(Value::Null);
    let verdicts = judge::parse(&reply, &insights);
    let sent = |m: &MoverRecord| m.check.is_empty() && m.insight.is_some();
    for m in rec.movers.iter_mut().filter(|m| sent(m)) {
        let v = verdicts.iter().find(|v| v.ticker == m.ticker).cloned();
        m.passed = match (&v, &m.insight) {
            (Some(v), Some(i)) => judge::passed(&m.pack, i, v),
            _ => false,
        };
        m.judge_error = v
            .is_none()
            .then(|| "the judge's reply has no verdict for this mover".to_string());
        m.verdict = v;
    }
    rec.stage = Stage::Done;
    rec.work_attempts = 0;
    rec.at = iso(now);
    if !put_if_seen(store, &rec, &mut seen).await? {
        return Ok(changed(&key));
    }
    let passed = rec.movers.iter().filter(|m| m.passed).count();
    Ok(closed(
        format!(
            "insight {key}: judged {} lines, {passed} passed; done",
            insights.len()
        ),
        others,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderError;
    use chrono::Duration;
    use futures::FutureExt;
    use serde_json::{json, Value};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, VecDeque};

    const RECAP: &str = include_str!("testdata/shadow/recap_2026-11-06.json");
    const RECAP_KEY: &str = "recap/2026-11-06.json";
    const PREMARKET: &str = include_str!("testdata/shadow/premarket_2026-11-12.json");
    const AMD: &str = include_str!("testdata/shadow/earnings_AMD_0000002488-26-000188.json");
    const CHTR: &str = include_str!("testdata/shadow/earnings_CHTR_0001091667-26-000072.json");
    const CALENDAR: &str = include_str!("testdata/shadow/calendar.json");
    const REPLIES: &str = include_str!("testdata/shadow/recap_2026-11-06.replies.json");
    const TRIAL_PACKS: &str = include_str!("testdata/shadow/recap_2026-11-06.packs.json");

    /// 17:00 ET on Friday 2026-11-06, after the recap.
    fn recap_now() -> DateTime<Utc> {
        "2026-11-06T22:00:00Z".parse().unwrap()
    }
    /// 08:40 ET on 2026-11-12, after the 8:35 update.
    fn premarket_now() -> DateTime<Utc> {
        "2026-11-12T13:40:00Z".parse().unwrap()
    }

    fn cfg() -> Config {
        config(None, None, None).unwrap()
    }

    /// The step's store in memory. `ops` lists every call, one per
    /// subrequest it would be in the Worker.
    #[derive(Default)]
    struct Mem {
        posts: RefCell<Vec<PostRef>>,
        objects: RefCell<BTreeMap<String, String>>,
        calendar: Vec<(String, String, String)>,
        checks: RefCell<Vec<CheckRef>>,
        kv: RefCell<BTreeMap<String, String>>,
        ops: RefCell<Vec<String>>,
        /// Keys written, R2 and kv.
        writes: RefCell<Vec<String>>,
        /// The message is killed right after the next R2 write whose text
        /// holds this: the write lands, then nothing more runs.
        kill_after: RefCell<Option<&'static str>>,
    }

    impl Mem {
        fn op(&self, s: String) {
            self.ops.borrow_mut().push(s);
        }
        fn record(&self, facts_key: &str) -> Option<Record> {
            self.objects
                .borrow()
                .get(&record_key(facts_key))
                .map(|t| serde_json::from_str(t).unwrap())
        }
        /// The attempt count alone (the counter also holds when it was written).
        fn attempts(&self, facts_key: &str) -> Option<String> {
            self.objects
                .borrow()
                .get(&attempts_key(facts_key))
                .map(|t| read_attempts(t).0.to_string())
        }
        fn calls(&self, month: &str) -> Option<String> {
            self.kv
                .borrow()
                .get(&format!("insight_calls:{month}"))
                .cloned()
        }
    }

    #[async_trait(?Send)]
    impl InsightStore for Mem {
        async fn insight_posts(&self, since: &str) -> Result<Vec<PostRef>, String> {
            self.op("d1 posts".into());
            Ok(self
                .posts
                .borrow()
                .iter()
                .filter(|p| p.trading_date.as_str() >= since)
                .cloned()
                .collect())
        }
        async fn get_text(&self, key: &str) -> Result<Option<String>, String> {
            self.op(format!("r2 get {key}"));
            Ok(self.objects.borrow().get(key).cloned())
        }
        async fn put_text(&self, key: &str, text: &str) -> Result<(), String> {
            self.op(format!("r2 put {key}"));
            self.writes.borrow_mut().push(key.into());
            self.objects.borrow_mut().insert(key.into(), text.into());
            let kill = self.kill_after.borrow().is_some_and(|m| text.contains(m));
            if kill {
                self.kill_after.borrow_mut().take();
                std::future::pending::<()>().await;
            }
            Ok(())
        }
        async fn calendar_rows(
            &self,
            tickers: &[String],
            from: &str,
            to: &str,
        ) -> Result<Vec<(String, String, String)>, String> {
            self.op("d1 calendar".into());
            Ok(self
                .calendar
                .iter()
                .filter(|(t, d, s)| {
                    tickers.contains(t) && (from..=to).contains(&d.as_str()) && s != "quarantined"
                })
                .cloned()
                .collect())
        }
        async fn earnings_checks(
            &self,
            tickers: &[String],
            from: &str,
            to: &str,
        ) -> Result<Vec<CheckRef>, String> {
            self.op("d1 checks".into());
            Ok(self
                .checks
                .borrow()
                .iter()
                .filter(|c| {
                    tickers.contains(&c.ticker) && (from..=to).contains(&c.filing_date.as_str())
                })
                .cloned()
                .collect())
        }
        async fn kv_get(&self, key: &str) -> Result<Option<String>, String> {
            self.op(format!("d1 kv get {key}"));
            Ok(self.kv.borrow().get(key).cloned())
        }
        async fn kv_claim(&self, key: &str, cap: u32, _now: &str) -> Result<Option<u32>, String> {
            self.op(format!("d1 kv claim {key}"));
            let n: u32 = self.kv.borrow().get(key).map_or(0, |v| v.parse().unwrap());
            if n >= cap {
                return Ok(None);
            }
            self.writes.borrow_mut().push(key.into());
            self.kv.borrow_mut().insert(key.into(), (n + 1).to_string());
            Ok(Some(n + 1))
        }
    }

    /// The recap of 2026-11-06 as the trial stored it: its bundle, the two
    /// earnings checks it read, and the calendar rows of its movers.
    fn recap_store() -> Mem {
        let m = Mem {
            calendar: serde_json::from_str::<Vec<(String, String, String)>>(CALENDAR).unwrap(),
            ..Default::default()
        };
        m.posts.borrow_mut().push(PostRef {
            content_type: "recap".into(),
            trading_date: "2026-11-06".into(),
            slug: "recap-2026-11-06".into(),
            facts_key: RECAP_KEY.into(),
        });
        let mut o = m.objects.borrow_mut();
        o.insert(RECAP_KEY.into(), RECAP.into());
        o.insert("earnings/amd.json".into(), AMD.into());
        o.insert("earnings/chtr.json".into(), CHTR.into());
        drop(o);
        *m.checks.borrow_mut() = vec![
            CheckRef {
                ticker: "AMD".into(),
                filing_date: "2026-11-03".into(),
                accepted_at: "2026-11-03T21:16:27Z".into(),
                facts_key: "earnings/amd.json".into(),
            },
            CheckRef {
                ticker: "CHTR".into(),
                filing_date: "2026-11-06".into(),
                accepted_at: "2026-11-06T12:00:54Z".into(),
                facts_key: "earnings/chtr.json".into(),
            },
        ];
        m
    }

    fn premarket_post(facts_key: &str) -> PostRef {
        PostRef {
            content_type: "premarket".into(),
            trading_date: "2026-11-12".into(),
            slug: "premarket-2026-11-12".into(),
            facts_key: facts_key.into(),
        }
    }

    /// The 2026-11-12 brief and its 8:35 update (the same bundle under both
    /// keys), the post row pointing at the update.
    fn premarket_store() -> Mem {
        let m = Mem::default();
        m.posts
            .borrow_mut()
            .push(premarket_post("premarket/2026-11-12-update.json"));
        let mut o = m.objects.borrow_mut();
        o.insert("premarket/2026-11-12.json".into(), PREMARKET.into());
        o.insert("premarket/2026-11-12-update.json".into(), PREMARKET.into());
        drop(o);
        m
    }

    enum Answer {
        Value(Value),
        Fail(ProviderError),
        /// Never answers: the message is killed during the call.
        Hang,
        /// Answers after other messages ran.
        Later(Value),
    }

    /// A model double. `strict` is the method it expects: the judge's.
    struct Llm {
        strict: bool,
        answers: RefCell<VecDeque<Answer>>,
        calls: Cell<usize>,
    }

    impl Llm {
        fn new(strict: bool, answers: Vec<Answer>) -> Llm {
            Llm {
                strict,
                answers: RefCell::new(answers.into()),
                calls: Cell::new(0),
            }
        }
        fn writer(answers: Vec<Answer>) -> Llm {
            Llm::new(false, answers)
        }
        fn judge(answers: Vec<Answer>) -> Llm {
            Llm::new(true, answers)
        }
        /// Fails the test if called.
        fn none(strict: bool) -> Llm {
            Llm::new(strict, vec![])
        }
        async fn answer(&self, strict: bool) -> Result<Value, ProviderError> {
            assert_eq!(
                strict, self.strict,
                "the writer is lenient, the judge strict"
            );
            self.calls.set(self.calls.get() + 1);
            let a = self
                .answers
                .borrow_mut()
                .pop_front()
                .expect("no model call expected");
            match a {
                Answer::Value(v) => Ok(v),
                Answer::Fail(e) => Err(e),
                Answer::Hang => std::future::pending().await,
                Answer::Later(v) => {
                    tokio::task::yield_now().await;
                    Ok(v)
                }
            }
        }
    }

    #[async_trait(?Send)]
    impl LlmProvider for Llm {
        async fn complete_json(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
            self.answer(false).await
        }
        async fn complete_json_strict(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
            self.answer(true).await
        }
    }

    fn replies() -> Value {
        serde_json::from_str(REPLIES).unwrap()
    }
    fn writer_reply() -> Answer {
        Answer::Value(replies()["writer"].clone())
    }
    fn judge_reply() -> Answer {
        Answer::Value(replies()["judge"].clone())
    }
    fn refusal() -> Answer {
        Answer::Fail(ProviderError::Parse(
            "text after the JSON object: On reflection, claim 2 is not supported.".into(),
        ))
    }

    async fn run(m: &Mem, now: DateTime<Utc>, writer: &Llm, judge: &Llm) -> Tick {
        insight_tick(now, m, writer, judge, &cfg(), false)
            .await
            .unwrap()
    }

    /// The recap's write and check steps: its record waits for the judge.
    async fn to_written(m: &Mem) {
        let writer = Llm::writer(vec![writer_reply()]);
        run(m, recap_now(), &writer, &Llm::none(true)).await;
        run(m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(m.record(RECAP_KEY).unwrap().stage, Stage::Written);
    }

    const REPLIED: &str = r#""stage":"replied""#;
    const WRITTEN: &str = r#""stage":"written""#;
    const JUDGED: &str = r#""stage":"judged""#;

    #[test]
    fn mode_reads_off_shadow_and_refuses_anything_else() {
        assert_eq!(mode(None), Ok(Mode::Off));
        assert_eq!(mode(Some("off")), Ok(Mode::Off));
        assert_eq!(mode(Some("shadow")), Ok(Mode::Shadow));
        for bad in ["on", "Shadow", " shadow", "true", ""] {
            assert!(mode(Some(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(
            mode(Some("on")).unwrap_err(),
            "unknown INSIGHTS_MODE: on (use off or shadow)"
        );
    }

    /// The Worker ships in shadow, capped at 300 model calls a month (Rick's
    /// go: shadow on 2026-11-17, the cap raised from the first run's 10 on
    /// 2026-11-18). Changing either is his call.
    #[test]
    fn the_worker_ships_in_shadow_capped_at_300_calls() {
        let toml = include_str!("../../../generator/wrangler.toml");
        let set =
            |name: &str| -> Vec<&str> { toml.lines().filter(|l| l.starts_with(name)).collect() };
        assert_eq!(set("INSIGHTS_MODE"), [r#"INSIGHTS_MODE = "shadow""#]);
        assert_eq!(mode(Some("shadow")), Ok(Mode::Shadow));
        assert_eq!(
            set("INSIGHT_MONTHLY_CALLS"),
            [r#"INSIGHT_MONTHLY_CALLS = "300""#]
        );
        assert_eq!(config(None, None, Some("300")).unwrap().cap, 300);
    }

    #[test]
    fn config_defaults_to_claude_and_300_calls() {
        assert_eq!(
            config(None, None, None),
            Ok(Config {
                writer: "claude",
                judge: "claude",
                writer_model: "claude-sonnet-5-5",
                judge_model: "claude-sonnet-5-5",
                cap: 300,
            })
        );
        let c = config(Some("claude"), Some("deepseek"), Some(" 40 ")).unwrap();
        assert_eq!(
            (c.judge, c.judge_model, c.cap),
            ("deepseek", "deepseek-flash", 40)
        );
        assert!(config(Some("gpt"), None, None)
            .unwrap_err()
            .contains("INSIGHT_WRITER_PROVIDER"));
        assert!(config(None, Some("x"), None)
            .unwrap_err()
            .contains("INSIGHT_JUDGE_PROVIDER"));
        assert!(config(None, None, Some("lots")).is_err());
    }

    #[test]
    fn record_keys_follow_the_facts_key() {
        assert_eq!(
            record_key("premarket/2026-11-17-update.json"),
            "insights/premarket/2026-11-17-update.json"
        );
        assert_eq!(
            record_key("premarket/2026-11-17.json"),
            "insights/premarket/2026-11-17.json"
        );
        assert_eq!(
            record_key("recap/2026-11-17.json"),
            "insights/recap/2026-11-17.json"
        );
        assert_eq!(
            attempts_key("recap/2026-11-17.json"),
            "insights/recap/2026-11-17.attempts"
        );
    }

    #[test]
    fn sweeps_run_in_two_windows_on_trading_days_and_never_when_off() {
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        // Tuesday 2026-11-17 (EST: ET = UTC-5).
        for (t, due) in [
            ("2026-11-17T13:05:00Z", false),
            ("2026-11-17T13:10:00Z", true),
            ("2026-11-17T14:30:00Z", true),
            ("2026-11-17T14:35:00Z", false),
            ("2026-11-17T21:10:00Z", true),
            ("2026-11-17T22:30:00Z", true),
            ("2026-11-17T22:35:00Z", false),
            // EDT: 08:10 ET is 12:10Z.
            ("2026-10-29T12:10:00Z", true),
            // Saturday, and Thanksgiving.
            ("2026-11-21T13:30:00Z", false),
            ("2026-11-26T13:30:00Z", false),
        ] {
            assert_eq!(sweep_due(at(t)), due, "{t}");
            assert_eq!(sweep_wanted(Some("shadow"), at(t)), due, "{t}");
            assert_eq!(sweep_wanted(Some("bad"), at(t)), due, "{t}");
        }
        // Off (unset or "off"): never, at any minute of a week.
        let start = at("2026-11-16T00:00:00Z");
        for m in 0..7 * 24 * 60 {
            let t = start + Duration::minutes(m);
            assert!(!sweep_wanted(None, t) && !sweep_wanted(Some("off"), t));
        }
    }

    /// The step reads the same earnings checks and calendar rows the trial
    /// read, so its packs for the recap of 2026-11-06 are the trial's.
    #[tokio::test]
    async fn the_packs_match_the_trials_on_a_real_recap() {
        let m = recap_store();
        // Decoys the step must leave unread: an older AMD check, and a CHTR
        // check accepted after the close the recap describes.
        m.checks.borrow_mut().extend([
            CheckRef {
                ticker: "AMD".into(),
                filing_date: "2026-11-04".into(),
                accepted_at: "2026-11-06T21:30:00Z".into(),
                facts_key: "earnings/late.json".into(),
            },
            CheckRef {
                ticker: "AMD".into(),
                filing_date: "2026-11-03".into(),
                accepted_at: "2026-11-03T20:00:00Z".into(),
                facts_key: "earnings/older.json".into(),
            },
        ]);
        let t = run(
            &m,
            recap_now(),
            &Llm::writer(vec![writer_reply()]),
            &Llm::none(true),
        )
        .await;
        assert!(t.follow_up, "{t:?}");
        let rec = m.record(RECAP_KEY).unwrap();
        let trial: Vec<Pack> = serde_json::from_str(TRIAL_PACKS).unwrap();
        let packs: Vec<Pack> = rec.movers.iter().map(|x| x.pack.clone()).collect();
        assert_eq!(packs, trial);
        let ops = m.ops.borrow();
        assert!(ops.contains(&"r2 get earnings/amd.json".to_string()));
        assert!(
            !ops.iter()
                .any(|o| o.contains("late") || o.contains("older")),
            "{ops:?}"
        );
    }

    /// The whole flow on the real recap: a write stage, then a judge stage
    /// in a second message; the record's JSON is pinned (Task 5's fixture).
    #[tokio::test]
    async fn a_recap_is_written_then_judged_and_the_record_is_pinned() {
        let m = recap_store();
        let writer = Llm::writer(vec![writer_reply()]);
        let judge = Llm::judge(vec![judge_reply()]);
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert_eq!(
            t,
            Tick {
                line: "insight recap/2026-11-06.json: 5 eligible, writer replied; checks queued"
                    .into(),
                follow_up: true,
            }
        );
        let replied = m.record(RECAP_KEY).unwrap();
        assert_eq!(replied.stage, Stage::Replied);
        assert!(replied.writer_reply.is_some());
        assert!(replied.movers.iter().all(|x| x.insight.is_none()));
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert_eq!(
            t,
            Tick {
                line: "insight recap/2026-11-06.json: 5 eligible, 3 written, 3 passed the checks; judge queued".into(),
                follow_up: true,
            }
        );
        let written = m.record(RECAP_KEY).unwrap();
        assert_eq!(written.stage, Stage::Written);
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 0));
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert_eq!(
            t.line,
            "insight recap/2026-11-06.json: judged 3 lines, 3 passed; done"
        );
        assert!(!t.follow_up);
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 1));
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(rec.stage, Stage::Done);
        assert_eq!((rec.calls, rec.judge_attempts), (2, 1));
        assert_eq!(m.calls("2026-11").as_deref(), Some("2"));
        // Nothing more to do.
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert_eq!(t.line, "insight: nothing to do");
        let json = m.objects.borrow()[&record_key(RECAP_KEY)].clone();
        let pinned: Value =
            serde_json::from_str(include_str!("testdata/shadow/record.json")).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), pinned);
    }

    #[test]
    fn the_record_shape_is_pinned() {
        let v: Value = serde_json::from_str(include_str!("testdata/shadow/record.json")).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "at",
                "calls",
                "content_type",
                "facts_key",
                "judge_attempts",
                "judge_model",
                "judge_reply",
                "judge_version",
                "movers",
                "post_slug",
                "skipped_reason",
                "stage",
                "trading_date",
                "version",
                "work_attempts",
                "writer_model",
                "writer_reply",
                "writer_version"
            ]
        );
        let mover: Vec<&str> = v["movers"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            mover,
            [
                "check",
                "insight",
                "judge_error",
                "pack",
                "passed",
                "ticker",
                "verdict",
                "write_error"
            ]
        );
        assert_eq!(v["version"], "insight-record/v1");
        assert_eq!(v["stage"], "done");
        assert_eq!(v["writer_version"], WRITER_VERSION);
        assert_eq!(v["judge_version"], JUDGE_VERSION);
    }

    /// Review Focus 1: the 8:35 update rewrites the 8:00 post with a new
    /// bundle. It gets its own record; the 8:00 record stays.
    #[tokio::test]
    async fn the_835_update_gets_its_own_record_and_the_800_one_stays() {
        let m = premarket_store();
        m.posts.borrow_mut()[0] = premarket_post("premarket/2026-11-12.json");
        let none = || Answer::Value(json!({"insights": []}));
        let at_800 = "2026-11-12T13:05:00Z".parse().unwrap();
        run(&m, at_800, &Llm::writer(vec![none()]), &Llm::none(true)).await;
        run(&m, at_800, &Llm::none(false), &Llm::none(true)).await;
        let base = m.objects.borrow()["insights/premarket/2026-11-12.json"].clone();
        assert_eq!(
            m.record("premarket/2026-11-12.json").unwrap().stage,
            Stage::Done
        );
        // 8:35: the post row now points at the update's bundle.
        m.posts.borrow_mut()[0] = premarket_post("premarket/2026-11-12-update.json");
        let t = run(
            &m,
            premarket_now(),
            &Llm::writer(vec![none()]),
            &Llm::none(true),
        )
        .await;
        assert!(
            t.line
                .starts_with("insight premarket/2026-11-12-update.json:"),
            "{t:?}"
        );
        let update = m.record("premarket/2026-11-12-update.json").unwrap();
        assert_eq!(update.facts_key, "premarket/2026-11-12-update.json");
        assert_eq!(update.post_slug, "premarket-2026-11-12");
        assert_eq!(
            m.objects.borrow()["insights/premarket/2026-11-12.json"],
            base
        );
        assert_eq!(
            m.record("premarket/2026-11-12.json").unwrap().facts_key,
            "premarket/2026-11-12.json"
        );
    }

    /// Review Focus 1, the other order: the 8:00 record still waits for its
    /// judge when the update lands. The next tick judges the 8:00 record
    /// (found by its fixed key), and the one after writes the update's.
    #[tokio::test]
    async fn a_written_800_record_is_still_judged_after_the_update() {
        let m = premarket_store();
        // A written record for the 8:00 bundle, with the recap's lines.
        let r = recap_store();
        to_written(&r).await;
        let mut base = r.record(RECAP_KEY).unwrap();
        base.facts_key = "premarket/2026-11-12.json".into();
        base.content_type = "premarket".into();
        m.objects.borrow_mut().insert(
            "insights/premarket/2026-11-12.json".into(),
            serde_json::to_string(&base).unwrap(),
        );
        let judge = Llm::judge(vec![judge_reply()]);
        let t = run(&m, premarket_now(), &Llm::none(false), &judge).await;
        assert!(
            t.line
                .starts_with("insight premarket/2026-11-12.json: judged"),
            "{t:?}"
        );
        assert_eq!(
            m.record("premarket/2026-11-12.json").unwrap().stage,
            Stage::Done
        );
        assert!(m.record("premarket/2026-11-12-update.json").is_none());
        let t = run(
            &m,
            premarket_now(),
            &Llm::writer(vec![Answer::Value(json!({"insights": []}))]),
            &Llm::none(true),
        )
        .await;
        assert!(
            t.line
                .starts_with("insight premarket/2026-11-12-update.json:"),
            "{t:?}"
        );
    }

    /// The lead's ruling of 2026-11-17: a `written` 8:00 record is still
    /// judged after the 8:35 update has published and has its own record,
    /// whether the update's record is done or also waits for its judge (the
    /// older one first).
    #[tokio::test]
    async fn a_written_800_record_is_judged_beside_the_updates_own_record() {
        let r = recap_store();
        to_written(&r).await;
        let written = r.record(RECAP_KEY).unwrap();
        let as_key = |key: &str, stage: Stage| {
            let mut rec = written.clone();
            rec.facts_key = key.into();
            rec.content_type = "premarket".into();
            rec.stage = stage;
            serde_json::to_string(&rec).unwrap()
        };
        for update_stage in [Stage::Done, Stage::Written] {
            let m = premarket_store();
            m.objects.borrow_mut().insert(
                "insights/premarket/2026-11-12.json".into(),
                as_key("premarket/2026-11-12.json", Stage::Written),
            );
            let update = as_key("premarket/2026-11-12-update.json", update_stage);
            m.objects.borrow_mut().insert(
                "insights/premarket/2026-11-12-update.json".into(),
                update.clone(),
            );
            let judge = Llm::judge(vec![judge_reply(), judge_reply()]);
            let t = run(&m, premarket_now(), &Llm::none(false), &judge).await;
            assert_eq!(
                t.line, "insight premarket/2026-11-12.json: judged 3 lines, 3 passed; done",
                "{update_stage:?}"
            );
            assert_eq!(
                m.record("premarket/2026-11-12.json").unwrap().stage,
                Stage::Done
            );
            // The update's record is untouched by the 8:00 judge stage.
            assert_eq!(
                m.objects.borrow()["insights/premarket/2026-11-12-update.json"],
                update
            );
            let t = run(&m, premarket_now(), &Llm::none(false), &judge).await;
            match update_stage {
                Stage::Done => assert_eq!(t.line, "insight: nothing to do"),
                Stage::Replied | Stage::Judged => unreachable!(),
                Stage::Written => {
                    assert!(t
                        .line
                        .starts_with("insight premarket/2026-11-12-update.json: judged"));
                    assert_eq!(
                        m.record("premarket/2026-11-12-update.json").unwrap().stage,
                        Stage::Done
                    );
                }
            }
        }
    }

    /// The cap's boundary, as the D1 statement keeps it: at cap - 1 one more
    /// call is claimed, at the cap none, and a second claim at once finds the
    /// cap.
    #[tokio::test]
    async fn at_one_below_the_cap_one_call_is_allowed_and_at_the_cap_none() {
        let m = recap_store();
        m.kv.borrow_mut()
            .insert("insight_calls:2026-11".into(), "299".into());
        let writer = Llm::writer(vec![writer_reply()]);
        let t = run(&m, recap_now(), &writer, &Llm::none(true)).await;
        assert!(t.follow_up, "{t:?}");
        assert_eq!(writer.calls.get(), 1);
        assert_eq!(m.calls("2026-11").as_deref(), Some("300"));
        // The check step makes no call, at the cap or not.
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert!(t.line.ends_with("judge queued"), "{t:?}");
        // Now at the cap: the judge step makes no call and uses no try.
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert!(t.line.starts_with("monthly cap of 300"), "{t:?}");
        assert_eq!(m.record(RECAP_KEY).unwrap().judge_attempts, 0);
        assert_eq!(m.calls("2026-11").as_deref(), Some("300"));
        // The claim itself: one below the cap gives the last call, then none.
        let s = Mem::default();
        s.kv.borrow_mut().insert("k".into(), "1".into());
        assert_eq!(s.kv_claim("k", 2, "t").await, Ok(Some(2)));
        assert_eq!(s.kv_claim("k", 2, "t").await, Ok(None));
        assert_eq!(s.kv.borrow()["k"], "2");
    }

    /// A record a Friday left open (`replied`, `written` or `judged`: the
    /// last sweep ended before the next step) is finished by Monday's first
    /// sweeps: the step looks back to the previous trading day, not the
    /// calendar day before. The same across a market holiday.
    #[tokio::test]
    async fn a_friday_record_is_finished_on_monday_and_across_a_holiday() {
        for (day, sweep) in [
            // Friday 2026-11-06, then Monday 2026-11-09 at 08:10 ET.
            ("2026-11-06", "2026-11-09T13:10:00Z"),
            // Wednesday 2026-11-25, then Friday 2026-11-27 after Thanksgiving.
            ("2026-11-25", "2026-11-27T13:10:00Z"),
        ] {
            for left in [Stage::Replied, Stage::Written, Stage::Judged] {
                let m = recap_store();
                m.posts.borrow_mut()[0].trading_date = day.into();
                let writer = Llm::writer(vec![writer_reply()]);
                run(&m, recap_now(), &writer, &Llm::none(true)).await;
                if left != Stage::Replied {
                    run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
                }
                if left == Stage::Judged {
                    *m.kill_after.borrow_mut() = Some(JUDGED);
                    let j = Llm::judge(vec![judge_reply()]);
                    let killed =
                        insight_tick(recap_now(), &m, &Llm::none(false), &j, &cfg(), false)
                            .now_or_never();
                    assert!(killed.is_none());
                }
                assert_eq!(m.record(RECAP_KEY).unwrap().stage, left, "{day}");
                let at: DateTime<Utc> = sweep.parse().unwrap();
                let judge = Llm::judge(vec![judge_reply()]);
                let mut last = String::new();
                for _ in 0..3 {
                    last = run(&m, at, &Llm::none(false), &judge).await.line;
                    if m.record(RECAP_KEY).unwrap().stage == Stage::Done {
                        break;
                    }
                }
                assert_eq!(
                    last, "insight recap/2026-11-06.json: judged 3 lines, 3 passed; done",
                    "{day} {left:?}"
                );
                assert_eq!(writer.calls.get(), 1);
            }
        }
        // A record two trading days back is out of the window.
        let m = recap_store();
        to_written(&m).await;
        let t = run(
            &m,
            "2026-11-10T13:10:00Z".parse().unwrap(),
            &Llm::none(false),
            &Llm::none(true),
        )
        .await;
        assert_eq!(t.line, "insight: no brief or recap to read");
    }

    /// Review M1, the reviewer's sequence: ticks X and Y both judge a
    /// `written` record; X stores its paid reply (`judged`) and is killed;
    /// Y's call fails. Y must not write its stale copy over X's reply, and
    /// the next tick confirms X's reply without paying the judge again.
    #[tokio::test]
    async fn a_failed_judge_never_overwrites_another_runs_stored_reply() {
        let m = recap_store();
        to_written(&m).await;
        let x = Llm::judge(vec![Answer::Later(replies()["judge"].clone())]);
        let y = Llm::judge(vec![Answer::Fail(ProviderError::Http(529))]);
        // Y's failure comes after X's reply is stored: Y yields once first.
        struct Slow<'a>(&'a Llm);
        #[async_trait(?Send)]
        impl LlmProvider for Slow<'_> {
            async fn complete_json(&self, s: &str, u: &str) -> Result<Value, ProviderError> {
                self.0.complete_json(s, u).await
            }
            async fn complete_json_strict(&self, s: &str, u: &str) -> Result<Value, ProviderError> {
                for _ in 0..4 {
                    tokio::task::yield_now().await;
                }
                self.0.complete_json_strict(s, u).await
            }
        }
        *m.kill_after.borrow_mut() = Some(JUDGED);
        let (no_writer, c) = (Llm::none(false), cfg());
        let x_tick = insight_tick(recap_now(), &m, &no_writer, &x, &c, false);
        let y_slow = Slow(&y);
        let y_tick = insight_tick(recap_now(), &m, &no_writer, &y_slow, &c, false);
        let x_killed = futures::future::select(Box::pin(x_tick), Box::pin(y_tick)).await;
        let y_line = match x_killed {
            futures::future::Either::Right((y, _x_pending)) => y.unwrap().line,
            futures::future::Either::Left(_) => panic!("X is killed after storing its reply"),
        };
        assert_eq!(
            y_line,
            "insight recap/2026-11-06.json: record changed by another run; left to it"
        );
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(rec.stage, Stage::Judged);
        assert_eq!(rec.judge_reply, Some(replies()["judge"].clone()));
        assert_eq!((x.calls.get(), y.calls.get()), (1, 1));
        // The next tick confirms the stored reply: no third judge call.
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(
            t.line,
            "insight recap/2026-11-06.json: judged 3 lines, 3 passed; done"
        );
        assert_eq!(m.calls("2026-11").as_deref(), Some("3"));
    }

    /// Review M1, the lease: a sweep leaves a record or an attempt counter
    /// another message wrote within 5 minutes to that message; the chain's
    /// own messages carry on, and after the lease a sweep takes it up.
    #[tokio::test]
    async fn a_sweep_leaves_a_record_touched_within_the_lease_alone() {
        let m = recap_store();
        run(
            &m,
            recap_now(),
            &Llm::writer(vec![writer_reply()]),
            &Llm::none(true),
        )
        .await;
        let (no_writer, no_judge, c) = (Llm::none(false), Llm::none(true), cfg());
        let sweep = |at: DateTime<Utc>| insight_tick(at, &m, &no_writer, &no_judge, &c, true);
        let soon = recap_now() + Duration::minutes(4);
        assert_eq!(sweep(soon).await.unwrap().line, "insight: nothing to do");
        assert_eq!(m.record(RECAP_KEY).unwrap().stage, Stage::Replied);
        let later = recap_now() + Duration::minutes(5);
        let t = sweep(later).await.unwrap();
        assert!(t.line.ends_with("judge queued"), "{t:?}");
        // An attempt counter in its lease: a write in flight is not doubled.
        let m = recap_store();
        let killed = insight_tick(
            recap_now(),
            &m,
            &Llm::writer(vec![Answer::Hang]),
            &Llm::none(true),
            &cfg(),
            false,
        )
        .now_or_never();
        assert!(killed.is_none());
        let t = insight_tick(soon, &m, &Llm::none(false), &Llm::none(true), &cfg(), true)
            .await
            .unwrap();
        assert_eq!(t.line, "insight: nothing to do");
        let writer = Llm::writer(vec![writer_reply()]);
        let t = insight_tick(later, &m, &writer, &Llm::none(true), &cfg(), true)
            .await
            .unwrap();
        assert!(t.follow_up && writer.calls.get() == 1, "{t:?}");
        assert_eq!(m.attempts(RECAP_KEY).as_deref(), Some("2"));
    }

    /// Review L2: a record of another version is left alone, and the judge
    /// step stamps the judge that answered.
    #[tokio::test]
    async fn another_versions_record_is_left_alone_and_the_judge_is_stamped() {
        let m = recap_store();
        to_written(&m).await;
        let mut other = m.record(RECAP_KEY).unwrap();
        other.version = "insight-record/v2".into();
        let text = serde_json::to_string(&other).unwrap();
        m.objects
            .borrow_mut()
            .insert(record_key(RECAP_KEY), text.clone());
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(t.line, "insight: nothing to do");
        assert_eq!(m.objects.borrow()[&record_key(RECAP_KEY)], text);
        let m = recap_store();
        to_written(&m).await;
        let mut old = m.record(RECAP_KEY).unwrap();
        old.judge_version = "insight-judge/v0".into();
        old.judge_model = "old".into();
        m.objects
            .borrow_mut()
            .insert(record_key(RECAP_KEY), serde_json::to_string(&old).unwrap());
        run(
            &m,
            recap_now(),
            &Llm::none(false),
            &Llm::judge(vec![judge_reply()]),
        )
        .await;
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(
            (rec.judge_version.as_str(), rec.judge_model.as_str()),
            (JUDGE_VERSION, "claude-sonnet-5-5")
        );
    }

    /// Review L3: a message that closes an older record asks for one more
    /// when another bundle still has no record.
    #[tokio::test]
    async fn closing_a_record_queues_one_more_when_a_bundle_waits() {
        let m = recap_store();
        to_written(&m).await;
        m.posts.borrow_mut().push(PostRef {
            content_type: "recap".into(),
            trading_date: "2026-11-06".into(),
            slug: "other".into(),
            facts_key: "recap/2026-11-06-other.json".into(),
        });
        let t = run(
            &m,
            recap_now(),
            &Llm::none(false),
            &Llm::judge(vec![judge_reply()]),
        )
        .await;
        assert!(t.line.ends_with("3 passed; done") && t.follow_up, "{t:?}");
        // With nothing else waiting, no follow-up.
        let m = recap_store();
        to_written(&m).await;
        let t = run(
            &m,
            recap_now(),
            &Llm::none(false),
            &Llm::judge(vec![judge_reply()]),
        )
        .await;
        assert!(!t.follow_up, "{t:?}");
    }

    /// Review R2, the reviewer's wedge probe: a `written` record whose head
    /// reads but whose body does not is left alone and named in the line;
    /// the newer bundle behind it still gets written, every tick.
    #[tokio::test]
    async fn a_record_with_an_unreadable_body_never_blocks_the_others() {
        let m = premarket_store();
        m.posts.borrow_mut()[0] = premarket_post("premarket/2026-11-12.json");
        m.posts.borrow_mut().push(PostRef {
            content_type: "recap".into(),
            trading_date: "2026-11-11".into(),
            slug: "r".into(),
            facts_key: "recap/2026-11-11.json".into(),
        });
        let bad = r#"{"version":"insight-record/v1","stage":"written","at":"2026-11-11T21:00:00Z","judge_attempts":0,"work_attempts":0,"movers":"not a list"}"#;
        m.objects
            .borrow_mut()
            .insert("insights/recap/2026-11-11.json".into(), bad.into());
        let writer = Llm::writer(vec![Answer::Value(json!({"insights": []}))]);
        let t = insight_tick(premarket_now(), &m, &writer, &Llm::none(true), &cfg(), true)
            .await
            .unwrap();
        assert!(
            t.line
                .starts_with("insight premarket/2026-11-12.json: 1 eligible, writer replied"),
            "{t:?}"
        );
        assert!(
            t.line
                .contains("unreadable record left alone: recap/2026-11-11.json: invalid type"),
            "{t:?}"
        );
        assert_eq!(
            m.record("premarket/2026-11-12.json").unwrap().stage,
            Stage::Replied
        );
        assert_eq!(m.objects.borrow()["insights/recap/2026-11-11.json"], bad);
        // And on: the check step of the newer bundle, the bad record untouched.
        let t = run(&m, premarket_now(), &Llm::none(false), &Llm::none(true)).await;
        assert!(
            t.line.starts_with("insight premarket/2026-11-12.json:"),
            "{t:?}"
        );
        assert_eq!(m.objects.borrow()["insights/recap/2026-11-11.json"], bad);
    }

    /// Review R5: a bundle whose write tries are used up is not waiting, so
    /// closing a record does not queue a message that only finds nothing to
    /// do.
    #[tokio::test]
    async fn a_given_up_bundle_does_not_count_as_waiting() {
        let m = recap_store();
        to_written(&m).await;
        m.posts.borrow_mut().push(PostRef {
            content_type: "recap".into(),
            trading_date: "2026-11-06".into(),
            slug: "other".into(),
            facts_key: "recap/2026-11-06-other.json".into(),
        });
        m.objects.borrow_mut().insert(
            attempts_key("recap/2026-11-06-other.json"),
            "3 2026-11-06T21:00:00Z".into(),
        );
        let t = run(
            &m,
            recap_now(),
            &Llm::none(false),
            &Llm::judge(vec![judge_reply()]),
        )
        .await;
        assert!(t.line.ends_with("3 passed; done") && !t.follow_up, "{t:?}");
    }

    /// The lead's CPU ruling: a message killed right after the writer's
    /// reply is stored is redelivered and goes on from the stored reply; the
    /// writer is never paid twice.
    #[tokio::test]
    async fn a_kill_after_the_writer_reply_is_stored_never_repays_it() {
        let m = recap_store();
        *m.kill_after.borrow_mut() = Some(REPLIED);
        let writer = Llm::writer(vec![writer_reply()]);
        let killed =
            insight_tick(recap_now(), &m, &writer, &Llm::none(true), &cfg(), false).now_or_never();
        assert!(killed.is_none(), "killed after the reply was stored");
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!((rec.stage, rec.calls), (Stage::Replied, 1));
        assert_eq!(rec.writer_reply, Some(replies()["writer"].clone()));
        // Redelivered: the checks run from the stored reply, no writer call.
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert!(
            t.line.ends_with("3 passed the checks; judge queued"),
            "{t:?}"
        );
        assert_eq!(writer.calls.get(), 1);
        assert_eq!(m.calls("2026-11").as_deref(), Some("1"));
    }

    /// The same for the judge: killed right after its reply is stored, the
    /// next delivery confirms the stored verdicts; the judge is never paid
    /// twice.
    #[tokio::test]
    async fn a_kill_after_the_judge_reply_is_stored_never_repays_it() {
        let m = recap_store();
        to_written(&m).await;
        *m.kill_after.borrow_mut() = Some(JUDGED);
        let judge = Llm::judge(vec![judge_reply()]);
        let killed =
            insight_tick(recap_now(), &m, &Llm::none(false), &judge, &cfg(), false).now_or_never();
        assert!(killed.is_none(), "killed after the reply was stored");
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(
            (rec.stage, rec.judge_attempts, rec.work_attempts),
            (Stage::Judged, 1, 1)
        );
        assert_eq!(rec.judge_reply, Some(replies()["judge"].clone()));
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(
            t.line,
            "insight recap/2026-11-06.json: judged 3 lines, 3 passed; done"
        );
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!((rec.stage, rec.calls), (Stage::Done, 2));
        assert_eq!(judge.calls.get(), 1);
        assert_eq!(m.calls("2026-11").as_deref(), Some("2"));
    }

    /// One queue message delivered three times in a row, each delivery
    /// killed right after it stored its result (the writer's reply, the
    /// checked lines, the judge's reply), as an unacknowledged message is
    /// redelivered: each model call is paid once, and a sweep finishes it.
    #[tokio::test]
    async fn a_message_delivered_three_times_pays_each_call_once() {
        let m = recap_store();
        let writer = Llm::writer(vec![writer_reply()]);
        let judge = Llm::judge(vec![judge_reply()]);
        for (marker, stage) in [
            (REPLIED, Stage::Replied),
            (WRITTEN, Stage::Written),
            (JUDGED, Stage::Judged),
        ] {
            *m.kill_after.borrow_mut() = Some(marker);
            let killed =
                insight_tick(recap_now(), &m, &writer, &judge, &cfg(), false).now_or_never();
            assert!(killed.is_none(), "{marker}");
            assert_eq!(m.record(RECAP_KEY).unwrap().stage, stage);
        }
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert!(t.line.ends_with("3 passed; done"), "{t:?}");
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 1));
        assert_eq!(m.calls("2026-11").as_deref(), Some("2"));
        assert_eq!(m.attempts(RECAP_KEY).as_deref(), Some("1"));
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert_eq!(t.line, "insight: nothing to do");
    }

    /// A code-only step killed every time (a cold compile over the CPU
    /// limit) is given up after three tries, without a model call.
    #[tokio::test]
    async fn a_check_step_killed_three_times_is_given_up() {
        let m = recap_store();
        run(
            &m,
            recap_now(),
            &Llm::writer(vec![writer_reply()]),
            &Llm::none(true),
        )
        .await;
        for n in 1..=3 {
            // Killed right after counting its try (the record is rewritten
            // with the try and still reads `replied`).
            *m.kill_after.borrow_mut() = Some(REPLIED);
            let killed = insight_tick(
                recap_now(),
                &m,
                &Llm::none(false),
                &Llm::none(true),
                &cfg(),
                false,
            )
            .now_or_never();
            assert!(killed.is_none());
            assert_eq!(m.record(RECAP_KEY).unwrap().work_attempts, n);
        }
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(
            t.line,
            "insight recap/2026-11-06.json: the code checks given up after 3 tries"
        );
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(rec.stage, Stage::Done);
        assert_eq!(
            rec.skipped_reason.as_deref(),
            Some("the code checks did not finish in 3 tries")
        );
        assert_eq!(m.calls("2026-11").as_deref(), Some("1"));
    }

    /// Each step compiles only the pattern groups it needs, and the steps
    /// that pay compile theirs before the call (`warm_*`), checked in a
    /// fresh process per step.
    #[test]
    fn each_step_warms_its_own_pattern_groups() {
        assert_eq!(warm_write(), 5);
        assert_eq!(warm_check(), 18 + 4);
        assert_eq!(warm_judge(), 4 + 1);
    }

    /// Review Focus 2: a message killed after the writer call went out. The
    /// attempt and the call are already counted; the next sweep retries,
    /// and the counts grow by what that retry does, nothing more.
    #[tokio::test]
    async fn a_message_killed_mid_call_is_counted_once_and_retried() {
        let m = recap_store();
        let writer = Llm::writer(vec![Answer::Hang, writer_reply()]);
        let judge = Llm::judge(vec![Answer::Hang, judge_reply()]);
        let killed = insight_tick(recap_now(), &m, &writer, &judge, &cfg(), false).now_or_never();
        assert!(killed.is_none(), "the writer call hangs");
        assert_eq!(m.attempts(RECAP_KEY).as_deref(), Some("1"));
        assert_eq!(m.calls("2026-11").as_deref(), Some("1"));
        assert!(m.record(RECAP_KEY).is_none());
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert!(t.follow_up, "{t:?}");
        assert_eq!(m.attempts(RECAP_KEY).as_deref(), Some("2"));
        assert_eq!(m.calls("2026-11").as_deref(), Some("2"));
        assert_eq!(writer.calls.get(), 2);
        run(&m, recap_now(), &writer, &judge).await;
        // The judge step killed mid-call: its try and the month's call are
        // counted; the record's `calls` holds the calls whose outcome it has.
        let killed = insight_tick(recap_now(), &m, &writer, &judge, &cfg(), false).now_or_never();
        assert!(killed.is_none());
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(
            (rec.stage, rec.judge_attempts, rec.calls),
            (Stage::Written, 1, 1)
        );
        assert_eq!(m.calls("2026-11").as_deref(), Some("3"));
        run(&m, recap_now(), &writer, &judge).await;
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(
            (rec.stage, rec.judge_attempts, rec.calls),
            (Stage::Done, 2, 2)
        );
        assert_eq!(m.calls("2026-11").as_deref(), Some("4"));
        assert_eq!((writer.calls.get(), judge.calls.get()), (2, 2));
    }

    #[tokio::test]
    async fn writes_give_up_after_three_attempts() {
        let m = recap_store();
        let writer = Llm::writer(vec![
            Answer::Fail(ProviderError::Http(529)),
            Answer::Fail(ProviderError::Http(529)),
            Answer::Fail(ProviderError::Http(529)),
        ]);
        for n in 1..=3 {
            let t = run(&m, recap_now(), &writer, &Llm::none(true)).await;
            assert!(t.line.ends_with(&format!("attempt {n} of 3")), "{t:?}");
        }
        let t = run(&m, recap_now(), &writer, &Llm::none(true)).await;
        assert_eq!(t.line, "insight: nothing to do");
        assert_eq!(writer.calls.get(), 3);
        assert!(m.record(RECAP_KEY).is_none());
    }

    /// Review Focus 3, in the step: a strict refusal is a failed judge call
    /// recorded as such, with no line passed. The record stays `written`
    /// and a later tick completes it with exactly one more judge call.
    #[tokio::test]
    async fn a_failed_judge_call_leaves_a_written_record_a_later_tick_completes() {
        let m = recap_store();
        let writer = Llm::writer(vec![writer_reply()]);
        let judge = Llm::judge(vec![refusal(), judge_reply()]);
        run(&m, recap_now(), &writer, &judge).await;
        run(&m, recap_now(), &writer, &judge).await;
        let t = run(&m, recap_now(), &writer, &judge).await;
        assert!(
            t.line
                .contains("judge call failed: parse error: text after the JSON object"),
            "{t:?}"
        );
        assert!(!t.follow_up);
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(rec.stage, Stage::Written);
        assert!(rec.movers.iter().all(|x| !x.passed));
        let sent: Vec<&MoverRecord> = rec
            .movers
            .iter()
            .filter(|x| x.check.is_empty() && x.insight.is_some())
            .collect();
        assert_eq!(sent.len(), 3);
        assert!(sent.iter().all(|x| x
            .judge_error
            .as_deref()
            .is_some_and(|e| e.contains("text after the JSON object"))));
        run(&m, recap_now(), &writer, &judge).await;
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(rec.stage, Stage::Done);
        assert_eq!(rec.movers.iter().filter(|x| x.passed).count(), 3);
        assert!(rec.movers.iter().all(|x| x.judge_error.is_none()));
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 2));
        assert_eq!(rec.calls, 3);
    }

    #[tokio::test]
    async fn three_judge_failures_end_done_with_nothing_passed() {
        let m = recap_store();
        let writer = Llm::writer(vec![writer_reply()]);
        let judge = Llm::judge(vec![
            refusal(),
            Answer::Fail(ProviderError::Http(500)),
            refusal(),
        ]);
        // Write, check, then three judge tries.
        for _ in 0..5 {
            run(&m, recap_now(), &writer, &judge).await;
        }
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(rec.stage, Stage::Done);
        assert_eq!(rec.judge_attempts, 3);
        assert_eq!(
            rec.skipped_reason.as_deref(),
            Some("the judge failed 3 times")
        );
        assert!(rec.movers.iter().all(|x| !x.passed));
        assert_eq!((writer.calls.get(), judge.calls.get()), (1, 3));
        // A record whose third try was killed is closed without a call.
        let mut open = rec.clone();
        open.stage = Stage::Written;
        m.objects
            .borrow_mut()
            .insert(record_key(RECAP_KEY), serde_json::to_string(&open).unwrap());
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert!(t.line.ends_with("judge given up after 3 tries"), "{t:?}");
        assert_eq!(m.record(RECAP_KEY).unwrap().stage, Stage::Done);
    }

    /// Review Focus 4: no eligible mover. A final record says so; no model
    /// call is made and the month's call count is untouched.
    #[tokio::test]
    async fn a_post_with_no_eligible_mover_gets_a_record_and_no_call() {
        let m = premarket_store();
        let mut facts: Value = serde_json::from_str(PREMARKET).unwrap();
        for side in ["gainers", "losers"] {
            for mv in facts[side].as_array_mut().unwrap() {
                mv["reasons"] = json!([]);
            }
        }
        for k in [
            "premarket/2026-11-12.json",
            "premarket/2026-11-12-update.json",
        ] {
            m.objects.borrow_mut().insert(k.into(), facts.to_string());
        }
        let t = run(&m, premarket_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(
            t,
            Tick {
                line: "insight premarket/2026-11-12-update.json: no eligible mover; done".into(),
                follow_up: false,
            }
        );
        let rec = m.record("premarket/2026-11-12-update.json").unwrap();
        assert_eq!(rec.stage, Stage::Done);
        assert_eq!(rec.skipped_reason.as_deref(), Some("no eligible mover"));
        assert_eq!(rec.calls, 0);
        assert!(rec.movers.is_empty());
        assert!(m.kv.borrow().is_empty(), "{:?}", m.kv.borrow());
    }

    /// Review Focus 5: the publish-queued message and a sweep at once. Both
    /// may pay the writer; one record stands, the first one written.
    #[tokio::test]
    async fn two_messages_at_once_leave_one_record() {
        let m = recap_store();
        let mut other = replies()["writer"].clone();
        other["insights"][0]["en"]["hook"] = json!("Another hook entirely");
        let first = Llm::writer(vec![Answer::Later(replies()["writer"].clone())]);
        let second = Llm::writer(vec![Answer::Later(other)]);
        let (a, b) = futures::future::join(
            insight_tick(recap_now(), &m, &first, &Llm::none(true), &cfg(), false),
            insight_tick(recap_now(), &m, &second, &Llm::none(true), &cfg(), false),
        )
        .await;
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(a.follow_up, "{a:?}");
        assert_eq!(
            b,
            Tick {
                line: "insight recap/2026-11-06.json: record already written by another run".into(),
                follow_up: false,
            }
        );
        assert_eq!((first.calls.get(), second.calls.get()), (1, 1));
        let rec = m.record(RECAP_KEY).unwrap();
        assert_eq!(rec.writer_reply, Some(replies()["writer"].clone()));
        let puts = m
            .ops
            .borrow()
            .iter()
            .filter(|o| *o == "r2 put insights/recap/2026-11-06.json")
            .count();
        assert_eq!(puts, 1);
        assert_eq!(m.calls("2026-11").as_deref(), Some("2"));
        run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        // Two judges at once: one done record, never overwritten.
        let j1 = Llm::judge(vec![Answer::Later(replies()["judge"].clone())]);
        let j2 = Llm::judge(vec![Answer::Later(json!({"verdicts": []}))]);
        let (a, b) = futures::future::join(
            insight_tick(recap_now(), &m, &Llm::none(false), &j1, &cfg(), false),
            insight_tick(recap_now(), &m, &Llm::none(false), &j2, &cfg(), false),
        )
        .await;
        assert!(a.unwrap().line.ends_with("3 passed; done"));
        assert_eq!(
            b.unwrap().line,
            "insight recap/2026-11-06.json: record changed by another run; left to it"
        );
        assert_eq!(
            m.record(RECAP_KEY)
                .unwrap()
                .movers
                .iter()
                .filter(|x| x.passed)
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn the_cap_stops_before_a_call_and_says_so_once_a_month() {
        let m = recap_store();
        m.kv.borrow_mut()
            .insert("insight_calls:2026-11".into(), "300".into());
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(
            t.line,
            "monthly cap of 300 model calls reached for 2026-11; insights stop until next month"
        );
        // Nothing was tried: no attempt used, no facts read.
        assert_eq!(m.attempts(RECAP_KEY), None);
        assert!(!m
            .ops
            .borrow()
            .iter()
            .any(|o| o == &format!("r2 get {RECAP_KEY}")));
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(t.line, "at the monthly cap; skipped");
        // A cap reached between the check and the call: the claim stops it.
        let m = recap_store();
        m.kv.borrow_mut()
            .insert("insight_calls:2026-11".into(), "1".into());
        let cfg1 = config(None, None, Some("1")).unwrap();
        let t = insight_tick(
            recap_now(),
            &m,
            &Llm::none(false),
            &Llm::none(true),
            &cfg1,
            false,
        )
        .await
        .unwrap();
        assert!(
            t.line.starts_with("monthly cap of 1 model calls reached"),
            "{t:?}"
        );
        // A judge stage at the cap makes no call and uses no try.
        let m = recap_store();
        to_written(&m).await;
        m.kv.borrow_mut()
            .insert("insight_calls:2026-11".into(), "300".into());
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert!(t.line.starts_with("monthly cap of 300"), "{t:?}");
        assert_eq!(m.record(RECAP_KEY).unwrap().judge_attempts, 0);
        // The next month starts again.
        let december: DateTime<Utc> = "2026-12-01T13:30:00Z".parse().unwrap();
        assert_eq!(calls_key(december), "insight_calls:2026-12");
    }

    #[tokio::test]
    async fn a_missing_facts_bundle_uses_one_attempt() {
        let m = recap_store();
        m.objects.borrow_mut().remove(RECAP_KEY);
        let t = run(&m, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        assert_eq!(
            t.line,
            "insight recap/2026-11-06.json: facts missing; attempt 1 of 3"
        );
        assert_eq!(m.attempts(RECAP_KEY).as_deref(), Some("1"));
        assert!(m.record(RECAP_KEY).is_none());
        assert!(m.kv.borrow().is_empty());
    }

    /// Every write lands under `insights/` or in kv `insight_calls:*`.
    #[tokio::test]
    async fn the_step_writes_only_its_own_keys() {
        let m = recap_store();
        let writer = Llm::writer(vec![Answer::Fail(ProviderError::Http(500)), writer_reply()]);
        let judge = Llm::judge(vec![refusal(), judge_reply()]);
        for _ in 0..5 {
            run(&m, recap_now(), &writer, &judge).await;
        }
        let p = premarket_store();
        run(
            &p,
            premarket_now(),
            &Llm::writer(vec![Answer::Value(json!({"insights": []}))]),
            &Llm::none(true),
        )
        .await;
        let c = recap_store();
        c.kv.borrow_mut()
            .insert("insight_calls:2026-11".into(), "300".into());
        run(&c, recap_now(), &Llm::none(false), &Llm::none(true)).await;
        for s in [&m, &p, &c] {
            let w = s.writes.borrow();
            assert!(!w.is_empty());
            for k in w.iter() {
                assert!(
                    k.starts_with("insights/") || k.starts_with("insight_calls:"),
                    "{k}"
                );
            }
        }
    }

    /// The most a tick can read: six bundles of today and the previous trading day, the
    /// two newest given up (two attempt reads before the third), and a
    /// brief whose eight movers each have an earnings check.
    #[tokio::test]
    async fn the_worst_case_tick_stays_under_thirty_subrequests() {
        let m = Mem::default();
        let amd: Value = serde_json::from_str(AMD).unwrap();
        let facts: Value = serde_json::from_str(PREMARKET).unwrap();
        let mut refs = vec![];
        for side in ["gainers", "losers"] {
            for mv in facts[side].as_array().unwrap() {
                let t = mv["quote"]["symbol"].as_str().unwrap().to_string();
                let mut e = amd.clone();
                e["ticker"] = json!(t);
                e["filing_date"] = json!("2026-11-11");
                e["accepted_at"] = json!("2026-11-11T21:00:00Z");
                let key = format!("earnings/{t}.json");
                m.objects.borrow_mut().insert(key.clone(), e.to_string());
                refs.push(CheckRef {
                    ticker: t,
                    filing_date: "2026-11-11".into(),
                    accepted_at: "2026-11-11T21:00:00Z".into(),
                    facts_key: key,
                });
            }
        }
        assert_eq!(refs.len(), 8);
        *m.checks.borrow_mut() = refs;
        let post = |ct: &str, d: &str, key: &str| PostRef {
            content_type: ct.into(),
            trading_date: d.into(),
            slug: format!("{ct}-{d}"),
            facts_key: key.into(),
        };
        *m.posts.borrow_mut() = vec![
            post("recap", "2026-11-12", "recap/2026-11-12.json"),
            post(
                "premarket",
                "2026-11-12",
                "premarket/2026-11-12-update.json",
            ),
            post("recap", "2026-11-11", "recap/2026-11-11.json"),
            post(
                "premarket",
                "2026-11-11",
                "premarket/2026-11-11-update.json",
            ),
        ];
        {
            let mut o = m.objects.borrow_mut();
            o.insert("premarket/2026-11-12.json".into(), PREMARKET.into());
            for k in ["recap/2026-11-12.json", "premarket/2026-11-12-update.json"] {
                o.insert(attempts_key(k), "3".into());
            }
            let done = serde_json::to_string(&Record {
                version: RECORD_VERSION.into(),
                stage: Stage::Done,
                judge_attempts: 1,
                post_slug: "x".into(),
                content_type: "recap".into(),
                trading_date: "2026-11-11".into(),
                facts_key: "x".into(),
                at: "2026-11-11T22:00:00Z".into(),
                writer_version: WRITER_VERSION.into(),
                judge_version: JUDGE_VERSION.into(),
                writer_model: "m".into(),
                judge_model: "m".into(),
                movers: vec![],
                writer_reply: None,
                judge_reply: None,
                work_attempts: 0,
                calls: 2,
                skipped_reason: None,
            })
            .unwrap();
            for k in [
                "recap/2026-11-11.json",
                "premarket/2026-11-11-update.json",
                "premarket/2026-11-11.json",
            ] {
                o.insert(record_key(k), done.clone());
            }
        }
        let now: DateTime<Utc> = "2026-11-12T22:00:00Z".parse().unwrap();
        let writer = Llm::writer(vec![Answer::Value(json!({"insights": []}))]);
        let t = insight_tick(now, &m, &writer, &Llm::none(true), &cfg(), false)
            .await
            .unwrap();
        assert!(
            t.line
                .starts_with("insight premarket/2026-11-12.json: 8 eligible"),
            "{t:?}"
        );
        let ops = m.ops.borrow().clone();
        let count = |p: &str| ops.iter().filter(|o| o.starts_with(p)).count();
        assert_eq!(count("d1 posts"), 1);
        // Six record reads and the re-read before the write.
        let gets = |suffix: &str| {
            ops.iter()
                .filter(|o| o.starts_with("r2 get insights/") && o.ends_with(suffix))
                .count()
        };
        assert_eq!(gets(".json"), 6 + 1);
        // Two given-up counters, then the one tried, and its write.
        assert_eq!(gets(".attempts"), 3);
        assert_eq!(count("r2 put insights/premarket/2026-11-12.attempts"), 1);
        assert_eq!(count("r2 get earnings/"), 8);
        // Model calls, and the follow-up queue send a written record makes.
        let total = ops.len() + writer.calls.get() + 1;
        assert_eq!(total, 28, "{ops:#?}");
    }
}
