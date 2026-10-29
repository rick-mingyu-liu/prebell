# Plan 2 — After-Close Recap (EN/ZH) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every NYSE trading day the generator builds an after-close recap (index closes, notable movers with quote-anchored news reasons, after-hours earnings to watch), writes it in English and Simplified Chinese from the same facts, checks every number in code, and publishes it under the same "late, not thin" and EN-first rules as the premarket brief. The site shows the latest recap next to the latest premarket brief, and the archive filters by type.

**Architecture:** Plan 1's premarket-only pieces become content-agnostic where they were only incidentally premarket: `clock` gets a `Window` per content type and one `decide` function; `run` gets a `Content` trait (facts type, window, build, gate, write, sources) and one generic state machine `run::tick_content`, with `run::tick` kept as the premarket entry point. The recap lives in a new `core::recap` module (facts builder + drafting) and reuses the premarket drafting machinery (parse, markup filter, number check, retry). A new `core::schedule` maps each cron expression to the jobs it enqueues, so recap firings do not multiply premarket ZH retries. The site generalizes "latest premarket" to "latest post of a type".

**Tech Stack:** unchanged from Plan 1.

**Spec:** `docs/spec.md` (§3 goal 2, §8). Plan 1: `docs/plan-1.md`. Executors read all three.

**Later plans (not this one):** Plan 3 earnings detection + health checks + calendar (in progress in parallel; touches `run.rs`/`store.rs` too) · Plan 4 competitor watch · Plan 5 Claude API.

## Global Constraints

- Everything in Plan 1's Global Constraints still holds (languages, ET display / UTC storage, secrets, no AlphaLab content, disclaimer, provider traits).
- Recap: scheduled **4:45 PM ET** on full trading days and **1:45 PM ET** on early-close days (1:00 PM close); may publish late up to **7:00 PM ET** on both; retry every **5 minutes**; at 7:00 PM skip the day and alert. Closed days: nothing.
- Required section: **index closes** — S&P 500 (`^GSPC`), Nasdaq 100 (`^NDX`), Dow Jones Industrial Average (`^DJI`), Russell 2000 (`^RUT`). All four must be present and for today's session. Optional: notable movers (with reasons when anchored), after-hours earnings to watch → "n/a" / "暂缺".
- Template `recap:v1`, `content_type = "recap"`, slug `recap-YYYY-MM-DD`, run key `recap:YYYY-MM-DD`, facts at R2 `recap/YYYY-MM-DD.json`.
- Same number check (`numbers::check`) on the LLM text, plus a check of the rendered article body (tables included, source list excluded) against the facts. Same markup filter. Same EN-first / ZH-pending rule; recap ZH retries until **22:00 ET**, then `failed` + one warn alert (premarket keeps 18:00 ET).
- No schema change: `content_type` is free text and every needed column exists, so there is no `migrations/0002_recap.sql`.
- Shared code changes stay minimal and are listed in the PR (Plan 3 edits `run.rs` and `store.rs` in parallel). `run::tick`'s signature and `Store` are unchanged.
- No real API keys: provider tests use hand-written fixtures built from documented response shapes.

## Review Focus

1. **DST and the UTC day boundary.** Under EST, 19:00 ET is 00:00 UTC the next day. The 7:00 PM skip and the 22:00 ET ZH give-up both need firings in UTC 00–03 on UTC weekdays Tue–Sat. Tests in Tasks 1 and 7.
2. **Early-close days** (2026-11-27, 2026-12-24): window opens 13:45 ET; deadline still 19:00 ET. Tests in Task 1 and Task 6.
3. **Stale closes.** Yahoo can return the previous session's close; a close quote whose `regularMarketTime` is not on the trading date is an error, so it never publishes as today's close. Test in Task 2.
4. **Numbers in the rendered tables** are checked too; source link titles are not (they are verbatim citations). Test in Task 5.
5. **Generalization must not change premarket behavior.** All Plan 1 tests stay green unchanged; premarket ZH retry cadence is unchanged by the new crons. Tests in Tasks 6 and 7.

---

## File Structure

```
crates/core/src/
  clock.rs            # + Window, decide(), premarket_window(), recap_window(), recap_decision()
  providers.rs        # + MarketData::close_quote / close_quotes (default impls)
  yahoo.rs            # + close quotes from chart meta; '^' percent-encoded in the URL
  premarket/mod.rs    # extraction helpers made pub(crate) for reuse
  premarket/draft.rs  # generic write_with(); helpers pub(crate)
  recap/mod.rs        # RecapFacts, missing_required, numbers_in_facts, build_facts
  recap/draft.rs      # headings, render_markdown, write (with rendered-body check)
  run.rs              # Content trait, tick_content(), Premarket; tick() unchanged
  schedule.rs         # jobs_for_cron()
  lib.rs
crates/core/prompts/recap_v1_en.md, recap_v1_zh.md
crates/core/tests/fixtures/yahoo_chart_GSPC_close.json, yahoo_chart_NVDA_close.json
crates/generator/src/lib.rs, wrangler.toml
apps/web/src/lib/posts.ts, src/app/[locale]/page.tsx, archive/page.tsx, p/[slug]/page.tsx,
  src/components/PostList.tsx, messages/{en,zh}.json, tests/posts.test.ts
docs/plan-2.md
```

---

### Task 1: Per-type windows in the clock (`core::clock`)

**Files:** Modify `crates/core/src/clock.rs`.

**Interfaces:**
- Produces:
  - `struct Window { start: NaiveTime, end: NaiveTime }` (ET, `start <= t < end` is in window).
  - `fn decide(now_utc, window: impl Fn(DayKind) -> Window) -> SlotDecision` — `Closed` ⇒ `NotToday`; else `TooEarly` / `InWindow` / `PastDeadline` by the day's window.
  - `fn premarket_window(DayKind) -> Window` = 08:30–09:25 (all trading days).
  - `fn recap_window(DayKind) -> Window` = 16:45–19:00 on `Full`, 13:45–19:00 on `EarlyClose`.
  - `fn recap_decision(now_utc) -> SlotDecision`; `premarket_decision` becomes `decide(now, premarket_window)`.

- [ ] **Step 1: Failing tests**
```rust
#[test] fn recap_window_edt_and_est() {
    // Thu 2026-10-29 EDT: 20:45Z = 16:45 ET
    assert_eq!(recap_decision(utc("2026-10-29T20:44:59Z")), SlotDecision::TooEarly);
    assert!(matches!(recap_decision(utc("2026-10-29T20:45:00Z")), SlotDecision::InWindow{..}));
    // Mon 2026-11-02 EST: 21:45Z = 16:45 ET; 20:45Z = 15:45 ET
    assert_eq!(recap_decision(utc("2026-11-02T20:45:00Z")), SlotDecision::TooEarly);
    assert!(matches!(recap_decision(utc("2026-11-02T21:45:00Z")), SlotDecision::InWindow{..}));
}
#[test] fn recap_deadline_crosses_utc_midnight_in_est() {
    // Fri 2026-11-06 18:59:59 ET = Sat 23:59:59Z; 19:00 ET = Sat 00:00Z
    assert_eq!(recap_decision(utc("2026-11-06T23:59:59Z")), SlotDecision::InWindow{ trading_date: d("2026-11-06") });
    assert_eq!(recap_decision(utc("2026-11-07T00:00:00Z")), SlotDecision::PastDeadline{ trading_date: d("2026-11-06") });
}
#[test] fn recap_early_close() {
    // Fri 2026-11-27 EST, 1 PM close: window opens 13:45 ET = 18:45Z
    assert_eq!(recap_decision(utc("2026-11-27T18:44:00Z")), SlotDecision::TooEarly);
    assert!(matches!(recap_decision(utc("2026-11-27T18:45:00Z")), SlotDecision::InWindow{..}));
    assert_eq!(recap_decision(utc("2026-11-26T22:00:00Z")), SlotDecision::NotToday); // Thanksgiving
}
```
- [ ] **Step 2:** `cargo test -p prebell-core clock` → FAIL. **Step 3:** implement; Plan 1 clock tests unchanged. **Step 4:** PASS.
- [ ] **Step 5: Commit** `feat(core): per-content-type windows in the ET clock`.

---

### Task 2: Close quotes (`MarketData::close_quote`, Yahoo)

**Files:** Modify `crates/core/src/providers.rs`, `crates/core/src/yahoo.rs`; create fixtures `yahoo_chart_GSPC_close.json`, `yahoo_chart_NVDA_close.json` (hand-written from the documented v8 chart shape: `meta.regularMarketPrice`, `meta.regularMarketTime`, `meta.regularMarketVolume`, `meta.chartPreviousClose`).

**Interfaces:**
- Produces in `providers.rs` (default methods so other `MarketData` impls keep compiling):
  - `async fn close_quote(&self, symbol: &str, session: NaiveDate) -> Result<Quote, ProviderError>` — default `Err(Empty)`.
  - `async fn close_quotes(&self, symbols: &[String], session: NaiveDate) -> Vec<(String, Result<Quote, ProviderError>)>` — default calls `close_quote` in order.
- Yahoo: same chart URL with the symbol's `^` written as `%5E`; `last = regularMarketPrice`, `prev_close = chartPreviousClose`, `volume = regularMarketVolume`, `change_pct` computed; `regularMarketTime` (in the exchange time zone) must fall on `session`, else `Err(Parse("stale …"))`; `close_quotes` in chunks of 20 like `premarket_quotes`.

- [ ] **Step 1: Failing tests:** GSPC fixture → `last == 5850.12`, `prev_close == 5820.4`, change computed; NVDA → `volume == Some(210_000_000)`; same fixture with session 2026-10-30 → `Err(Parse(_))`; URL requested for `^GSPC` contains `/chart/%5EGSPC?`; 429 → `Err(Http(429))`; `close_quotes` keeps order.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core yahoo` PASS.
- [ ] **Step 5: Commit** `feat(core): regular-session close quotes`.

---

### Task 3: Generic drafting core (`core::premarket::draft`)

**Files:** Modify `crates/core/src/premarket/draft.rs`, `crates/core/src/premarket/mod.rs`.

**Interfaces:**
- Produces (`pub(crate)`):
  - `async fn write_with(llm, system: &str, facts_json: &str, allowed: &[f64], headings: &[&str], lang, render: impl Fn(&Draft) -> String) -> Result<(Draft, String), Vec<String>>` — the Plan 1 loop (parse, markup filter, number check, one retry with the problems appended). `write` becomes a call to it.
  - `fmt_num`, `fmt_pct`, `cell`, `na`, `render_sources(reasons, lang) -> String`.
  - In `premarket/mod.rs`: `system_prompt`, `articles_prompt`, `extract` made `pub(crate)` for recap reuse.
- Rule: no behavior change; every Plan 1 draft and premarket test passes untouched.

- [ ] **Step 1: Failing test:** `write_with` with custom headings `["A","B"]` and a custom render closure returns the closure's output and rejects a reply with headings `["A"]` (malformed, retried).
- [ ] **Step 2–4:** FAIL → refactor → `cargo test -p prebell-core` PASS (all Plan 1 tests too).
- [ ] **Step 5: Commit** `refactor(core): reusable drafting loop`.

---

### Task 4: Recap facts and builder (`core::recap`)

**Files:** Create `crates/core/src/recap/mod.rs`; modify `lib.rs`.

**Interfaces:**
- Consumes: `MarketData::close_quote(s)`, `NewsSearch`, `LlmProvider`, `premarket::{universe, yahoo_symbol, company_name}`, the `pub(crate)` extraction helpers, `facts::{Field, Quote, Mover, NewsReason, quote_numbers}`, `clock::trading_day`.
- Produces:
  - `struct RecapFacts { trading_date, as_of_utc, early_close: bool, indexes: Vec<(String, Field<Quote>)>, gainers: Vec<Mover>, losers: Vec<Mover>, after_hours: Field<Vec<NewsReason>> }` (serde, Clone, Debug, PartialEq).
  - `fn missing_required(&RecapFacts) -> Vec<&'static str>` → `["indexes"]` when any index is `Missing`.
  - `fn numbers_in_facts(&RecapFacts) -> Vec<f64>` — year±1, index and mover quote fields, reason quote numbers.
  - `async fn build_facts(md, ns, llm, trading_date, now) -> RecapFacts`.
- Rules: indexes `S&P 500`=`^GSPC`, `Nasdaq 100`=`^NDX`, `Dow Jones`=`^DJI`, `Russell 2000`=`^RUT`. Movers = universe close quotes with `last ≥ 5` and session `volume ≥ 1_000_000`, top 5 / bottom 5 by `change_pct`, each searched (`"{name} ({ticker}) stock"`, max 3) and explained by quote-anchored reasons (max 2). After-hours = search `"earnings after the bell {Month D, YYYY}"` (max 5) → quote-anchored items (max 5) of companies the article says report after today's close. Provider errors → `Missing`.

- [ ] **Step 1: Failing tests (fakes):** all four indexes present ⇒ `missing_required == []`; one index 429 ⇒ `["indexes"]`; movers sorted/capped and the volume filter applied; unanchored reason dropped; after-hours search error ⇒ `Missing`; an EarlyClose date sets `early_close`; index symbols are exactly the four above; JSON round trip.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core recap` PASS.
- [ ] **Step 5: Commit** `feat(core): recap facts builder`.

---

### Task 5: Recap drafting (`core::recap::draft`)

**Files:** Create `crates/core/src/recap/draft.rs`, `crates/core/prompts/recap_v1_en.md`, `recap_v1_zh.md`.

**Interfaces:**
- Produces: `TEMPLATE_ID = "recap:v1"`; `fn headings(Lang) -> [&str; 3]` = EN `"Index closes"`, `"Notable movers"`, `"After-hours earnings to watch"`; ZH `"指数收盘"`, `"个股异动"`, `"盘后财报关注"`; `fn render_markdown(&Draft, &RecapFacts) -> String` (title, summary, early-close note when set, index table, movers table, LLM sections, sources, disclaimer); `async fn write(llm, &RecapFacts, Lang) -> Result<(Draft, String), Vec<String>>` via `write_with`, then `numbers::check` over the rendered markdown up to the sources heading.
- Prompt rules (both files): Plan 1's rules, recap headings, "the session closed early at 1 PM ET" only when `early_close` is true, and after-hours items only as the facts state them.

- [ ] **Step 1: Failing tests (FakeLlm):** clean EN draft ⇒ `Ok`, markdown contains `| S&P 500 | 5,850.12 | +0.51% |`; unbacked `"7.5%"` then clean ⇒ two calls; two bad ⇒ `Err(["7.5%"])`; ZH renders `DISCLAIMER_ZH` and `## 指数收盘`; early close renders the note in both languages; a source title with an unbacked number still passes; a facts value that the table would render but that is not in the allow-list fails (rendered-body check is live).
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core recap` PASS.
- [ ] **Step 5: Commit** `feat(core): bilingual recap drafting with number check`.

---

### Task 6: Generic run state machine (`core::run`)

**Files:** Modify `crates/core/src/run.rs`, `crates/core/src/recap/mod.rs`.

**Interfaces:**
- Produces:
  - `#[async_trait(?Send)] trait Content { type Facts: Serialize + DeserializeOwned; fn content_type(&self) -> &'static str; fn template_id(&self) -> &'static str; fn label(&self) -> &'static str; fn window(&self, DayKind) -> Window; fn zh_give_up(&self) -> NaiveTime; async fn build_facts(&self, NaiveDate, DateTime<Utc>) -> Self::Facts; fn missing_required(&self, &Self::Facts) -> Vec<&'static str>; async fn write(&self, &Self::Facts, Lang) -> Result<(Draft, String), Vec<String>>; fn sources(&self, &Self::Facts) -> Vec<NewsReason>; }`, with provided `run_key(date)` = `"{type}:{date}"`, `slug(date)` = `"{type}-{date}"`, `facts_key(date)` = `"{type}/{date}.json"`.
  - `async fn tick_content<C: Content>(now, store, &C) -> TickOutcome` — Plan 1's logic, parameterized.
  - `struct Premarket<'a> { md, ns, llm }` implementing `Content`; `tick(now, store, md, ns, llm)` = `tick_content(now, store, &Premarket{..})` (signature unchanged).
  - `struct Recap<'a> { md, ns, llm }` in `recap/mod.rs`; `recap::tick(now, store, md, ns, llm)`.
- Rules: skip alert reads `"{label} skipped for {date}: not ready by {HH:MM} ET ({why})"` (premarket text unchanged); ZH give-up premarket 18:00, recap 22:00.

- [ ] **Step 1: Failing tests (MemStore + fakes):** recap at 16:45 ET publishes one post with `template_id "recap:v1"`, slug `recap-2026-10-29`, facts at `recap/2026-10-29.json`; a second tick ⇒ `Nothing`; one index 429 ⇒ `Retrying("indexes")` until 19:00 ET (EST, 00:00Z next UTC day) ⇒ `Skipped`, one error alert mentioning `19:00 ET`, no post; early-close day publishes at 13:45 ET; EN ok + ZH bad ⇒ `PublishedEnOnly`, ZH still `pending` at 21:55 ET, `failed` with one warn at 22:00 ET; premarket and recap on the same date keep separate runs and posts.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core` PASS (all Plan 1 run tests unchanged).
- [ ] **Step 5: Commit** `feat(core): content-agnostic run state machine and recap run`.

---

### Task 7: Cron schedule and generator wiring

**Files:** Create `crates/core/src/schedule.rs`; modify `crates/generator/src/lib.rs`, `crates/generator/wrangler.toml`.

**Interfaces:**
- Produces: `const CRONS: &[(&str, &[&str])]` and `fn jobs_for_cron(cron) -> &'static [&'static str]` (unknown cron ⇒ every job, logged by the Worker).
- Crons (UTC; `clock` decides what each firing means):
  - `*/5 12-14 * * 1-5` → premarket (unchanged)
  - `0 15-23 * * 1-5` → premarket (unchanged: hourly ZH retries)
  - `*/5 17-23 * * 1-5` → recap: early-close window from 17:45Z (EDT) / 18:45Z (EST), full-day window from 20:45Z (EDT) / 21:45Z (EST)
  - `*/5 0 * * 2-6` → recap: EST 19:00–19:55 ET (deadline skip) and the first hour of ZH retries
  - `0 1-3 * * 2-6` → recap: hourly ZH retries until the 22:00 ET give-up under both offsets
- Worker: `scheduled` enqueues one `Job` per job name for the firing cron; `queue` dispatches `"premarket"` → `run::tick`, `"recap"` → `recap::tick`, unknown → error log + ack.

- [ ] **Step 1: Failing tests:** `CRONS` keys equal the `crons` array in `crates/generator/wrangler.toml` (read with `include_str!`); every recap window minute in EDT and EST for full and early-close days, the 19:00 ET deadline and the 22:00 ET give-up is matched by some recap cron (simulated with a tiny 5-field matcher over each minute of 2026-10-29, 2026-11-02, 2026-11-06, 2026-11-27); premarket jobs are only produced by premarket crons.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core schedule` PASS; `worker-build --release` builds.
- [ ] **Step 5: Commit** `feat(generator): recap cron and job dispatch`.

---

### Task 8: Site — latest recap on home, archive by type

**Files:** Modify `apps/web/src/lib/posts.ts`, `src/app/[locale]/page.tsx`, `archive/page.tsx`, `p/[slug]/page.tsx`, `src/components/PostList.tsx`, `messages/en.json`, `messages/zh.json`, `tests/posts.test.ts`.

**Interfaces:**
- Produces: `type ContentType = "premarket" | "recap"`, `CONTENT_TYPES`, `parseContentType(raw): ContentType | null`; `getLatestPost(db, lang, type)` (`getLatestPremarket` kept as a wrapper); `listPosts(db, lang, page, type?)`; `PostView.contentType`.
- Pages: home shows the latest premarket brief (as now), then a "Latest after-close recap" card (title, ET date, summary, link; pending ZH shows the pending title + notice link); the recent list excludes both. Archive `?type=premarket|recap` (invalid ⇒ all), filter links, pager keeps the filter, canonical/hreflang include the filter. Post-list items show the type label. Pending titles use the post's type ("After-close recap, {date}" / "收盘回顾 {date}").

- [ ] **Step 1: Failing tests:** `getLatestPost(db,'en','recap')` returns the newest recap and never a premarket; `listPosts(db,'en',1,'recap')` only recaps; `listPosts` with no type returns both; `parseContentType('earnings') === null`; `getPostBySlug` returns `contentType`.
- [ ] **Step 2:** `pnpm --filter web test` → FAIL. **Step 3:** implement. **Step 4:** test, typecheck, lint PASS.
- [ ] **Step 5: Commit** `feat(web): latest recap on home, archive filter by type`.

---

## Parallelization (for subagent execution)

- **Wave 1 (parallel):** Tasks 1, 2, 3, 8.
- **Wave 2:** Task 4 (needs 2, 3) → Task 5 (needs 3, 4) → Task 6 (needs 1, 5) → Task 7 (needs 6).
