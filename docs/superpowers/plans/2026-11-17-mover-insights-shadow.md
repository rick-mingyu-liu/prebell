# Mover Insights, Plan 2: The Shadow Week

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run mover insights inside the generator after every published premarket brief, premarket update and recap, store every line and verdict in R2, and show readers nothing; then give Rick one page to read at the end of the week.

**Architecture:** A new queue job `insight` reads a just-published post's facts bundle, builds the evidence packs (`insight::pack`), calls the writer and the code checks (`insight::write`, `insight::check`), then the judge (`insight::judge`) in strict mode, and writes one audit record to R2. It runs in its own queue message, is queued when a content job publishes (as `cards` is), and is swept by later firings, bounded by an attempts counter and a monthly call cap. A switch `INSIGHTS_MODE` (`off` | `shadow`) gates it. The trial tool gains a `shadow` command that downloads the week's records and builds the labelling page.

**Tech Stack:** Rust (`prebell-core`, `prebell-generator` on workers-rs), Cloudflare Queues, D1, R2; the native `tools/insight-trial`.

**Spec:** `docs/superpowers/specs/2026-11-13-mover-insights-design.md` (approved 2026-11-13). Plan 1 (`docs/superpowers/plans/2026-11-13-mover-insights-trial.md`) built the core module and the trial; read its "after trial run" sections, they changed the rules.

## Global Constraints

- Readers see nothing: no change to `apps/web`, the MCP server, posts, translations or any table the site reads. Plan 3 shows insights.
- `INSIGHTS_MODE` is `off` unless set; `shadow` runs the step; any other value is a config error, alerted once per ET day under the key `config:insight:<date>` (the existing `config_error` path), and the step does nothing.
- Writer and judge are Claude (`claude-sonnet-5-5`, effort `low`), chosen by `INSIGHT_WRITER_PROVIDER` and `INSIGHT_JUDGE_PROVIDER`, both defaulting to `claude`, independent of `LLM_PROVIDER`. Rick's labels picked Claude as the judge (15 of 15 agreement, against DeepSeek's 14). With a missing key the step logs and skips; it never falls back to another provider.
- The judge is strict: a judge reply with any text after its JSON is a failed judge call, never a pass (decided 2026-11-16).
- Spend: at most `INSIGHT_MONTHLY_CALLS` model calls per calendar month (default 300, about $3 at the trial's measured $0.008–0.02 per call), counted in kv `insight_calls:YYYY-MM` before each call. At the cap the step stops for the month and logs once.
- Cloudflare free plan: the step runs only in its own `insight` queue message, never inside a build; at most 2 model calls per message (one writer, one judge). R2 and D1 reads are bounded (at most 8 movers, at most one earnings bundle per mover).
- A failure never alerts, never retries a brief, never changes a post. Attempts are counted in R2 before each try (`insights/<stem>.attempts`, at most 3), as `cover::card` does.
- People rule, Rick 2026-11-17 ("Allow it if attributed"): a line may repeat a claim about a named person, including an allegation, only when it names the outlet that reported it. This replaces the lead's rule of 2026-11-13.
- Everything Plan 1's code checks enforce still holds: numbers and dates from cited evidence, outlets named in a reporting frame, no ratings, price targets, consensus wording, advice or relative time.
- Commits end with the trailer lines the lead gives; no TODO placeholders, skipped tests or stubs.

## Review Focus

1. **The 8:35 update.** It rewrites the 8:00 post with a new facts bundle. Expected: a second insight record for the update's facts, keyed by its own facts stem; the 8:00 record stays. Test in Task 3.
2. **A message killed mid-way** (CPU or wall-clock limit after the writer call). Expected: the attempt is already counted, the month's call count already includes the writer call, and the next sweep retries up to the limit without double-counting. Test in Task 3.
3. **A judge reply with a note after its JSON.** Expected: a failed judge call recorded as such, no line counted as passed. Test in Task 2.
4. **A post with no eligible mover.** Expected: a record saying so, no model call, no attempts used. Test in Task 3.
5. **Two messages for the same post at once** (the publish-queued message and a sweep). Expected: one record stands; at worst the calls are paid twice, never two different records under one key. Test in Task 3.

---

### Task 1: The people rule and the label-key fix

**Files:** `crates/core/src/insight/write.rs`, `judge.rs`, `check.rs`, `tools/insight-trial/src/main.rs`, `docs/superpowers/specs/2026-11-13-mover-insights-design.md`, `docs/decisions.md`.

- Writer prompt (`insight-writer/v6`): replace the people rule with Rick's: a claim about a named person, including an allegation, may be repeated only when the line names the outlet that reported it, in a reporting frame; never stated as PreBell's own claim.
- Judge prompt (`insight-judge/v3`): replace the `no_personal_allegation` flag with `personal_claims_attributed` (true when every claim about a named person in either language is attributed to its outlet). A missing flag reads as false.
- Code: remove the personal-allegation word list from `check`. A line that names a person and uses one of those words must still carry an attributing frame for the cited news item (the existing attribution rule already requires it); add a test that "据Benzinga报道，…婚外情…" passes and the same claim with no outlet fails.
- Tool: `line_key` uses the record's own `writer_version`, not the `WRITER_VERSION` constant. Test: Rick's real `labels.json` shape against a v4 record matches.
- Docs: the spec's people rule; `docs/decisions.md` gets the 2026-11-17 entries (Rick's people rule, Claude as judge by measured agreement, this plan's rulings).

### Task 2: Strict judge replies

**Files:** `crates/core/src/providers.rs`, `crates/core/src/llm.rs`, `claude.rs`, `deepseek.rs`, `crates/core/src/insight/judge.rs`.

**Interfaces — Produces:**

```rust
// providers.rs, on LlmProvider
/// As `complete_json`, but a reply with any text after its JSON object is
/// `ProviderError::Parse("text after the JSON object: <snippet>")`.
/// Default: `complete_json` (for test doubles).
async fn complete_json_strict(&self, system: &str, user: &str) -> Result<Value, ProviderError>;
```

- Claude and DeepSeek implement it through `llm::parse_reply`, turning `Some(dropped)` into the error. `judge::judge` calls it.
- Tests: a judge reply of a pass followed by "On reflection, claim 2 is not supported" is an error for both providers; a clean reply passes; `complete_json` is unchanged.

### Task 3: The `insight` step in core

**Files:** create `crates/core/src/insight/step.rs`; modify `crates/core/src/insight/mod.rs`, `crates/core/src/schedule.rs` (`JobKind::Insight`, job name `insight`, sent to `JOBS_QUEUE`), `crates/core/src/jobs.rs` (`JobRunner::insight`, `JobRunner::queue_insight`; queue it beside `queue_card_if` when a content job's outcome is `Published` or `PublishedEnOnly`), `crates/core/src/store.rs` (any read the step needs).

**Interfaces — Produces:**

```rust
pub enum Mode { Off, Shadow }
pub fn mode(raw: Option<&str>) -> Result<Mode, String>;      // None or "off" → Off
pub const MONTHLY_CALLS_DEFAULT: u32 = 300;
pub const MAX_ATTEMPTS: u32 = 3;
pub const RECORD_VERSION: &str = "insight-record/v1";
/// R2 key of a post's record: `insights/<facts key with ".json" removed>.json`,
/// e.g. `insights/premarket/2026-11-17-update.json`.
pub fn record_key(facts_key: &str) -> String;
/// One `insight` message: the newest premarket or recap post of the last
/// 2 ET days whose facts have no record and fewer than MAX_ATTEMPTS tries.
pub async fn insight_tick(now, store, writer: &dyn LlmProvider, judge: &dyn LlmProvider, cap: u32) -> Result<String, String>;
```

The record (JSON, one per facts bundle): `version`, `post_slug`, `content_type`, `trading_date`, `facts_key`, `at`, `writer_version`, `judge_version`, `writer_model`, `judge_model`, and per eligible mover `{ticker, pack, insight|null, write_error|null, check: [..], verdict|null, judge_error|null, passed}`, plus `calls` and `skipped_reason|null`.

Decisions:
- The pack inputs come from stored data only: the post's facts bundle (R2), the earnings calendar rows for the movers' tickers (D1, one query), and an earnings check bundle for a mover only when `posts` has a published earnings post for that ticker filed within the pack's window (D1 then R2). No network fetch other than the two model calls.
- Order inside a tick: pick the post; count the attempt; build packs; if none eligible, write the record and stop; check the month's cap for 2 calls; count and call the writer; run `check`; count and call the judge (strict) on the insights that passed `check`; write the record. A record already present (written by a concurrent message) is not overwritten.
- `TickOutcome` is untouched; the step reports a log line.

- [ ] Tests (in-memory store, mock providers): each Review Focus item; `mode` parsing; the cap stopping before a call and logging once per month; a missing facts bundle using one attempt; the record's JSON shape pinned; the step never writes outside `insights/` and `kv` `insight_calls:*`.

### Task 4: The generator

**Files:** `crates/generator/src/lib.rs`, `crates/generator/wrangler.toml`.

- Wire `JobRunner::insight` and `queue_insight` (send `{"job":"insight"}` to `JOBS`). Build the writer and judge from their own settings and `ANTHROPIC_API_KEY`; a config error (bad mode, unknown provider) goes through `config_error` with key `config:insight:<date>`.
- The crons queue one `insight` message per firing during 08:10–09:30 ET and 16:10–17:30 ET on trading days, only when the mode is `shadow` (a cheap sweep: one D1 read when there is nothing to do).
- `wrangler.toml`: add `INSIGHTS_MODE = "off"` with a comment. Turning it to `shadow` is a deploy-config change and Rick's call.
- Log each tick's line through `console_log!` and each dropped reply note through the existing warn sink.
- [ ] `cargo check -p prebell-generator --target wasm32-unknown-unknown --locked`; `cargo test -p prebell-generator --locked`.

### Task 5: The shadow review page

**Files:** `tools/insight-trial/src/main.rs`, `src/page.rs`, `README.md`, a read-only `fetch-shadow.sh`.

- `fetch-shadow.sh FROM TO DIR`: lists `insights/` records for trading dates FROM..TO from R2 via the posts' facts keys (read-only `wrangler` calls), saves them to DIR.
- `shadow --records DIR --out DIR`: converts records into the tool's `lines.jsonl` shape and writes `label.html` with every judge-passed line (no sampling) plus every line the judge refused, verdicts hidden; `score` then reports the bar (at least 50 judge-passed lines read, none marked wrong).
- [ ] Tests from a fixture record built by Task 3's code.

### Task 6: Run it (the lead, with Rick's go)

- [ ] Merge after review; deploy the generator with `INSIGHTS_MODE = "off"`; confirm nothing changes.
- [ ] With Rick's go, set `INSIGHTS_MODE = "shadow"` and deploy. Watch the first day's records, calls and CPU time (Workers dashboard) and report.
- [ ] After five trading days, run `fetch-shadow.sh` and `shadow`, send Rick the page, and score it.
