# Mover Insights, Plan 2: The Shadow Stage

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run mover insights inside the generator after every published premarket brief, premarket update and recap, store every line and verdict in R2, and show readers nothing; and give Rick a review page he can open on any day, which adds up until the bar is met.

**Architecture:** A new queue job `insight` reads a just-published post's facts bundle, builds the evidence packs (`insight::pack`), calls the writer and the code checks (`insight::write`, `insight::check`), then the judge (`insight::judge`) in strict mode, and writes one audit record to R2. It runs in its own queue message, is queued when a content job publishes (as `cards` is), and is swept by later firings, bounded by an attempts counter and a monthly call cap. A switch `INSIGHTS_MODE` (`off` | `shadow`) gates it. The trial tool gains a `shadow` command that downloads every record so far and builds a cumulative labelling page.

**Tech Stack:** Rust (`prebell-core`, `prebell-generator` on workers-rs), Cloudflare Queues, D1, R2; the native `tools/insight-trial`.

**Spec:** `docs/superpowers/specs/2026-11-13-mover-insights-design.md` (approved 2026-11-13). Plan 1 (`docs/superpowers/plans/2026-11-13-mover-insights-trial.md`) built the core module and the trial; read its "after trial run" sections, they changed the rules.

## Global Constraints

- Readers see nothing: no change to `apps/web`, the MCP server, posts, translations or any table the site reads. Plan 3 shows insights.
- `INSIGHTS_MODE` is `off` unless set; `shadow` runs the step; any other value is a config error, alerted once per ET day under the key `config:insight:<date>` (the existing `config_error` path), and the step does nothing.
- Writer and judge are Claude (`claude-sonnet-5-5`, effort `low`), chosen by `INSIGHT_WRITER_PROVIDER` and `INSIGHT_JUDGE_PROVIDER`, both defaulting to `claude`, independent of `LLM_PROVIDER`. Rick's labels picked Claude as the judge (15 of 15 agreement, against DeepSeek's 14). With a missing key the step logs and skips; it never falls back to another provider.
- The judge is strict: a judge reply with any text after its JSON is a failed judge call, never a pass (decided 2026-11-16).
- Spend: at most `INSIGHT_MONTHLY_CALLS` model calls per calendar month (default 300: $2.40 to $6 a month at the trial's measured $0.008 to $0.02 a call), counted in kv `insight_calls:YYYY-MM` before each call. At the cap the step stops for the month and logs once.
- Cloudflare free plan: the step runs only in its own `insight` queue message, never inside a build; at most 2 model calls per message (one writer, one judge). R2 and D1 reads are bounded (at most 8 movers, at most one earnings bundle per mover).
- How long shadow runs: the trial gave about 2 judge-passed lines a trading day (14 over 7 days), so the spec's bar of 50 lines read takes about five weeks, not one. Shadow runs until the bar is met or Rick lowers it (the spec makes the bar his to change). Nothing in this plan assumes a one-week end.
- A failure never alerts, never retries a brief, never changes a post. Attempts are counted in R2 before each try (`insights/<stem>.attempts`, at most 3), as `cover::card` does.
- People rule, Rick 2026-11-17 ("Allow it if attributed"): a line may repeat a claim about a named person, including an allegation, only when it names the outlet that reported it. This replaces the lead's rule of 2026-11-13.
- Everything Plan 1's code checks enforce still holds: numbers and dates from cited evidence, outlets named in a reporting frame, no ratings, price targets, consensus wording, advice or relative time.
- Commits end with the trailer lines the lead gives; no TODO placeholders, skipped tests or stubs.

## Review Focus

1. **The 8:35 update.** It rewrites the 8:00 post with a new facts bundle. Expected: a second insight record for the update's facts, keyed by its own facts stem; the 8:00 record stays. Test in Task 3.
2. **A message killed mid-way** (CPU or wall-clock limit after the writer call). Expected: the attempt is already counted, the month's call count already includes the writer call, and the next sweep retries up to the limit without double-counting. Test in Task 3.
3. **A judge reply with a note after its JSON.** Expected: a failed judge call recorded as such, no line counted as passed. Test in Task 2.
4. **A post with no eligible mover.** Expected: a final `done` record saying so (`skipped_reason`), no model call and no month-count use; the attempt is counted first, as for any try (the lead's ruling, 2026-11-17). Test in Task 3.
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
/// The default is an error (`ProviderError::Parse("strict replies are not
/// supported by this provider")`), never `complete_json`: a provider that
/// forgets to override it must not turn the last check permissive. Test
/// doubles that act as a judge implement it.
async fn complete_json_strict(&self, system: &str, user: &str) -> Result<Value, ProviderError>;
```

- Claude and DeepSeek implement it through `llm::parse_reply`, turning `Some(dropped)` into the error. `judge::judge` calls it.
- Tests: a judge reply of a pass followed by "On reflection, claim 2 is not supported" is an error for both providers; a clean reply passes; `complete_json` is unchanged.

### Task 3: The `insight` step in core

**Files:** create `crates/core/src/insight/step.rs`; modify `crates/core/src/insight/mod.rs`, `crates/core/src/schedule.rs` (`JobKind::Insight`, job name `insight`, sent to its own queue `INSIGHT`, `prebell-insight`, one message at a time: Rick's choice after review M4; the queue is created as a separate step on the owner's go before the deploy that carries it, because wrangler 4.154.0 would otherwise create it on deploy by itself), `crates/core/src/jobs.rs` (`JobRunner::insight`, `JobRunner::queue_insight`; queue it beside `queue_card_if` when a content job's outcome is `Published` or `PublishedEnOnly`), `crates/core/src/store.rs` (any read the step needs).

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
/// 2 trading days (today and the previous trading day, so a Friday record is finished on Monday) whose facts have no record and fewer than MAX_ATTEMPTS tries.
pub async fn insight_tick(now, store, writer: &dyn LlmProvider, judge: &dyn LlmProvider, cap: u32) -> Result<String, String>;
```

The record (JSON, one per facts bundle): `version`, `stage` (`replied` | `written` | `judged` | `done`), `judge_attempts`, `work_attempts`, `writer_reply|null`, `judge_reply|null`, `post_slug`, `content_type`, `trading_date`, `facts_key`, `at`, `writer_version`, `judge_version`, `writer_model`, `judge_model`, and per eligible mover `{ticker, pack, insight|null, write_error|null, check: [..], verdict|null, judge_error|null, passed}`, plus `calls` and `skipped_reason|null`.

Decisions:
- The pack inputs come from stored data only: the post's facts bundle (R2), the earnings calendar rows for the movers' tickers (D1, one query), and an earnings check bundle for a mover only when `posts` has a published earnings post for that ticker filed within the pack's window (D1 then R2). No network fetch other than the two model calls.
- Steps, each in its own message unless noted, so a paid reply is stored before any heavy work and each message compiles only the patterns it needs, before it pays (the lead's CPU ruling, 2026-11-17, after Task 6 measured a 27 ms native first call):
  - **Write** (no record yet): pick the post; count the attempt; compile the pack patterns (`warm_write`); build packs; if none eligible, write a final record (`stage: "done"`, `skipped_reason`) and stop; check the month's cap; claim and call the writer; store its reply as it came (`stage: "replied"`, `writer_reply`); queue one more `insight` message. No code check runs here.
  - **Check** (`replied`): count a try in `work_attempts` (at most 3, then `done`); compile the check and number patterns (`warm_check`); parse the stored reply, run `check`; write `stage: "written"` (or `"done"` when no insight passed `check`); queue one more message.
  - **Judge** (`written`): check the month's cap; count a judge attempt in `judge_attempts` (at most 3); compile the confirmation patterns (`warm_judge`: numbers and date marks); claim and call the judge (strict); store its reply (`stage: "judged"`, `judge_reply`, the first confirmation try counted); then, in the same message, confirm the verdicts and write `stage: "done"`. A failed judge call (a strict refusal, an HTTP error) leaves `stage: "written"` with `judge_error` set, for the next sweep; after 3 it becomes `stage: "done"` with every line `passed: false`.
  - **Verdicts** (`judged`, only when the judge step was killed after storing the reply): count a try in `work_attempts` (at most 3); confirm the stored verdicts; write `stage: "done"`.
  - A tick does the next step of the oldest unfinished record first, else the write step of the newest post without a record. A redelivered message (one killed before its ack) finds the stored reply and goes on from it: no paid call is made twice.
  - A `done` record is never overwritten. Two messages racing on the write stage may both pay the writer; the second sees the record and does not overwrite it.
- `TickOutcome` is untouched; the step reports a log line.

- [ ] Tests (in-memory store, mock providers): each Review Focus item; a judge failure leaving a `written` record that a later tick completes with exactly one more judge call and no writer call; three judge failures ending in `done` with nothing passed; `mode` parsing; the cap stopping before a call and logging once per month; a missing facts bundle using one attempt; the record's JSON shape pinned; the step never writes outside `insights/` and `kv` `insight_calls:*`.

### Task 4: The generator

**Files:** `crates/generator/src/lib.rs`, `crates/generator/wrangler.toml`.

- Wire `JobRunner::insight` and `queue_insight` (send `{"job":"insight"}` to `INSIGHT`). Build the writer and judge from their own settings and `ANTHROPIC_API_KEY`; a config error (bad mode, unknown provider) goes through `config_error` with key `config:insight:<date>`.
- The crons queue one `insight` message per firing during 08:10–09:30 ET and 16:10–17:30 ET on trading days, only when the mode is `shadow` (a cheap sweep: one D1 read when there is nothing to do).
- `wrangler.toml`: add `INSIGHTS_MODE = "off"` with a comment. Turning it to `shadow` is a deploy-config change and Rick's call.
- Log each tick's line through `console_log!` and each dropped reply note through the existing warn sink.
- [ ] `cargo check -p prebell-generator --target wasm32-unknown-unknown --locked`; `cargo test -p prebell-generator --locked`.

### Task 5: The shadow review page

**Files:** `tools/insight-trial/src/main.rs`, `src/page.rs`, `README.md`, a read-only `fetch-shadow.sh`.

- `fetch-shadow.sh FROM TO DIR`: for each trading date FROM..TO, tries the three fixed keys `insights/premarket/<date>.json`, `insights/premarket/<date>-update.json` and `insights/recap/<date>.json` with read-only `wrangler r2 object get` (a missing key is skipped). It must not go through `posts.facts_r2_key`: after the 8:35 update the post row points at the update's facts, and the 8:00 record would be missed.
- `shadow --records DIR --out DIR [--labels FILE]`: converts every record into the tool's `lines.jsonl` shape and writes a cumulative `label.html` with every judge-passed line (no sampling) plus every line the judge refused, verdicts hidden. Lines already in `--labels` are shown as done, so Rick only reads new ones. The 8:00 brief and its 8:35 update often carry the same mover: when both have a line for one ticker on one date, the page shows both but `score` counts one toward the bar (the update's).
- `score` reports the bar from all labels so far: judge-passed lines read (counting one per ticker per date for premarket), any marked wrong, and "not yet assessable" below 50.
- [ ] Tests from a fixture record built by Task 3's code.

### Task 6: Measure the step's CPU (builder, no model calls)

The Worker allows 10 ms of CPU per invocation, and the step compiles dozens of regular expressions (pack, checks, judge-support confirmation) that a cold isolate pays for on its first call.

- [ ] A native release timing in `tools/insight-trial` (`time` subcommand): over the 13 stored bundles in `facts/` and the stored final-run lines and verdicts, time the write stage's own work (packs + `check` on the stored insights) and the judge stage's own work (`passed` on the stored verdicts), first call in a fresh process and warm, per post. Print the worst and the median. No model calls.
- [ ] Report the numbers. Native time understates Wasm; if the worst first-call figure is above about 2 ms native, say so and propose what to cut (lazy statics already compiled once per isolate, fewer patterns, or splitting `check` across messages) before the shadow switch.

### Task 7: Run it (the lead, with Rick's go)

- [ ] Dry-run the new prompts offline first: writer v6 and judge v3 have never been sent to a model (run 1 of the trial passed 3 of 29 lines with untested rules). One `run` of the trial tool into a fresh output folder, about $0.25, inside what is left of the trial's $3 cap ($1.63). Read every line and every refusal before going on.
- [ ] Merge after review; deploy the generator with `INSIGHTS_MODE = "off"`; confirm nothing changes.
- [ ] With Rick's go, set `INSIGHTS_MODE = "shadow"` and deploy. Watch the first day's records, calls and CPU time (Workers dashboard) and report.
- [ ] Build the review page for Rick whenever he asks and at least weekly; score it each time; stop shadow only on his word.
