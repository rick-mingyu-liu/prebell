# Plan 1 — Foundation + Premarket Brief (EN/ZH) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One working vertical slice: every weekday a Rust Worker builds a premarket brief from free data, writes it in English and Simplified Chinese with DeepSeek, checks every number in code, stores it in D1, and a Next.js site on Cloudflare serves it at `/en` and `/zh`.

**Architecture:** A Cargo workspace holds `core` (pure Rust, all logic, tested natively with `cargo test`) and `generator` (thin `workers-rs` Worker: cron, queue, D1/R2, HTTP). Providers are traits in `core`; the Worker supplies real HTTP, tests supply fakes. The site is a pnpm Next.js app deployed with `@opennextjs/cloudflare`, reading the same D1 database.

**Tech Stack:** Rust 1.97 + `worker` crate (workers-rs) + `serde`/`serde_json` + `chrono`/`chrono-tz`; Cloudflare Workers, Cron Triggers, Queues, D1, R2; DeepSeek API (OpenAI-compatible); Tavily search API; Yahoo Finance unofficial chart endpoints; Next.js App Router + `next-intl` + `@opennextjs/cloudflare`; Vitest; wrangler (as a dev dependency).

**Spec:** `docs/spec.md`. Executors read both.

**Later plans (not this one):** Plan 2 after-close recap · Plan 3 earnings detection (EDGAR 8-K Item 2.02) + health checks + charts + calendar · Plan 4 competitor watch · Plan 5 switch to Claude API.

## Global Constraints

- Repo: `~/Desktop/prebell`, git, default branch `main`. Name: PreBell (盘前铃), domain prebell.ai.
- Languages: `en` (primary) and `zh` (Simplified Chinese — use 亿/万, never 億/萬). Routes `/en/...`, `/zh/...`.
- All schedules and displayed times in **America/New_York**; everything stored in UTC (ISO 8601 `Z`).
- Premarket brief: scheduled **8:30 AM ET** weekdays that are NYSE trading days; may publish late up to **9:25 AM ET**; retry every **5 minutes**; after 9:25 skip the day and alert.
- Required sections for the premarket brief: **futures** (ES=F, NQ=F, YM=F, RTY=F) and **movers with at least one news reason**. Optional: macro, watch list → shown as "n/a" / "暂缺".
- Missing data is never guessed: facts mark it missing; prompts forbid filling it; the number check rejects any number not traceable to facts.
- If EN passes and ZH fails after one retry: publish EN, ZH status `pending`, ZH page shows "中文版稍后推出", excluded from search and hreflang until filled; keep retrying ZH. If EN fails: hold both, alert.
- Run key = (content type, ET trading date). Re-running replaces, never duplicates.
- Template ID stored on every post: `premarket:v1`.
- LLM: DeepSeek first (`https://api.deepseek.com/chat/completions`, model `deepseek-chat`, JSON output). Provider chosen by env var `LLM_PROVIDER=deepseek`.
- Data: Yahoo Finance unofficial endpoints behind the `MarketData` trait; Tavily behind `NewsSearch`.
- Every page shows a "not investment advice" disclaimer in its language. No Google Fonts or Google Analytics: self-hosted fonts, Cloudflare Web Analytics.
- Never use AlphaLab content in any prompt or output.
- Secrets only via `wrangler secret put` / `.dev.vars` (git-ignored): `DEEPSEEK_API_KEY`, `TAVILY_API_KEY`, `ALERT_WEBHOOK_URL`.

## Review Focus

1. **DST change day (2026-11-01 → 11-02 first weekday after).** Cron fires at both 12:30 and 13:30 UTC; exactly one counts as 8:30 ET. Test in Task 2.
2. **NYSE holidays and early-close days** (e.g., 2026-11-26 Thanksgiving closed, 2026-11-27 early close): no brief on closed days; brief still runs on early-close days. Test in Task 2.
3. **Yahoo returns empty / 429 / HTML from Cloudflare IPs.** Must not publish a thin post; the run retries until 9:25 then skips and alerts. Tests in Tasks 6 and 10.
4. **Chinese number formats** (亿, 万, 万亿, full-width ％ and ，, "上涨3.2%") and English ("$1.2 billion", "-0.45%", "1,234.5"): checker must normalize both and accept only facts-backed values. Tests in Task 4.
5. **Duplicate execution** (two cron firings, queue redelivery): one post per run key, statuses consistent. Test in Task 10.

---

## File Structure

```
prebell/
  Cargo.toml                         # workspace: crates/core, crates/generator
  package.json, pnpm-workspace.yaml  # apps/web + wrangler dev dep
  docs/spec.md, docs/plan-1.md
  migrations/0001_init.sql           # shared D1 schema
  crates/core/src/
    lib.rs
    clock.rs        # ET time, NYSE calendar, slot/deadline decisions
    facts.rs        # PremarketFacts, Field<T>, required-section gate
    numbers.rs      # number extraction + normalization (en/zh) + check
    anchor.rs       # quote-anchored extraction verification
    providers.rs    # traits LlmProvider, MarketData, NewsSearch, Http
    deepseek.rs     # LlmProvider impl
    yahoo.rs        # MarketData impl
    tavily.rs       # NewsSearch impl
    premarket/
      mod.rs        # build_facts(): futures, movers, news reasons
      universe.rs   # ticker universe + symbol mapping (BRK.B → BRK-B)
      draft.rs      # prompts, LLM JSON → Draft, render markdown, retry
    run.rs          # run state machine (pure): decide next action
    store.rs        # Store trait (D1 in generator, in-memory in tests)
  crates/core/tests/fixtures/        # recorded Yahoo/Tavily/DeepSeek JSON
  crates/generator/
    wrangler.toml
    src/lib.rs      # #[event(scheduled)], #[event(queue)], fetch → Http, D1 Store, alerts
  apps/web/
    wrangler.jsonc, open-next.config.ts, next.config.ts
    messages/en.json, messages/zh.json
    src/i18n/routing.ts, src/middleware.ts
    src/lib/db.ts, src/lib/posts.ts
    src/app/[locale]/layout.tsx, page.tsx, p/[slug]/page.tsx, archive/page.tsx, about/page.tsx
    src/components/{LangSwitch,Disclaimer,PostBody,PendingNotice}.tsx
    tests/posts.test.ts
```

---

### Task 1: Repo, workspace, D1 schema

**Files:**
- Create: `Cargo.toml`, `crates/core/Cargo.toml`, `crates/core/src/lib.rs`, `crates/generator/Cargo.toml`, `crates/generator/wrangler.toml`, `package.json`, `pnpm-workspace.yaml`, `.gitignore`, `migrations/0001_init.sql`, `docs/spec.md`, `docs/plan-1.md`
- Test: `migrations/smoke.sql`

**Interfaces:**
- Produces D1 tables (later tasks use these exact names):
  - `runs(run_key TEXT PK, content_type TEXT, trading_date TEXT, status TEXT, attempts INT, last_error TEXT, created_at TEXT, updated_at TEXT)` — status ∈ `pending|building|published|skipped|failed`
  - `posts(id INTEGER PK, run_key TEXT UNIQUE REFERENCES runs, content_type TEXT, template_id TEXT, trading_date TEXT, slug TEXT UNIQUE, facts_r2_key TEXT, published_at TEXT)`
  - `post_translations(post_id INT, lang TEXT CHECK(lang IN ('en','zh')), status TEXT CHECK(status IN ('published','pending','failed')), title TEXT, summary TEXT, body_md TEXT, updated_at TEXT, PRIMARY KEY(post_id, lang))`
  - `sources(post_id INT, url TEXT, title TEXT, quote TEXT)`
  - `alerts(id INTEGER PK, run_key TEXT, level TEXT, message TEXT, created_at TEXT, sent INT DEFAULT 0)`
- Slug format: `premarket-YYYY-MM-DD`.

- [ ] **Step 1:** `git init ~/Desktop/prebell`; copy the spec to `docs/spec.md` and this plan to `docs/plan-1.md`; `rustup target add wasm32-unknown-unknown`; `cargo install worker-build`.
- [ ] **Step 2:** Workspace `Cargo.toml` with members `crates/core`, `crates/generator`; `core` deps: `serde`(derive), `serde_json`, `chrono`, `chrono-tz`, `async-trait`, `regex`, `thiserror`; `generator` deps: `worker`, `core` (path), crate-type `cdylib`. `package.json` at root with devDependency `wrangler`.
- [ ] **Step 3:** Write `migrations/0001_init.sql` with the tables above plus indexes on `posts(trading_date)`, `post_translations(lang,status)`. Write `migrations/smoke.sql` inserting one run, one post, two translations, and selecting them joined.
- [ ] **Step 4:** `wrangler.toml` for generator: name `prebell-generator`, `main = "build/worker/shim.mjs"`, build command `worker-build --release`, D1 binding `DB` (database `prebell`), R2 binding `FACTS` (bucket `prebell-facts`), queue producer+consumer `prebell-jobs` binding `JOBS`, crons `["*/5 12-14 * * 1-5","0 15-23 * * 1-5"]` (every 5 min across both DST offsets of the 8:30–9:25 ET window; hourly afterwards for ZH retries; `clock` decides what each firing means), vars `LLM_PROVIDER="deepseek"`, `SEC_CONTACT="placeholder@example.com"`.
- [ ] **Step 5: Verify** `pnpm exec wrangler d1 migrations apply prebell --local` then `pnpm exec wrangler d1 execute prebell --local --file migrations/smoke.sql` → returns 2 rows. `cargo test -p prebell-core` → 0 tests, OK. `cd crates/generator && worker-build --release` → builds.
- [ ] **Step 6: Commit** `chore: workspace, D1 schema, worker skeleton`.

---

### Task 2: ET clock and NYSE calendar (`core::clock`)

**Files:** Create `crates/core/src/clock.rs`; Test: same file `#[cfg(test)]`.

**Interfaces:**
- Produces:
  - `fn et(now_utc: DateTime<Utc>) -> DateTime<Tz>`
  - `fn trading_day(date: NaiveDate) -> DayKind` where `enum DayKind { Closed, Full, EarlyClose }` — hardcoded NYSE holidays/early closes for 2026 and 2027 (from the NYSE published list).
  - `fn premarket_decision(now_utc: DateTime<Utc>) -> SlotDecision` where `enum SlotDecision { NotToday, TooEarly, InWindow { trading_date: NaiveDate }, PastDeadline { trading_date: NaiveDate } }`. Window = 08:30 ≤ ET < 09:25.

- [ ] **Step 1: Failing tests**
```rust
#[test] fn dst_day_only_one_cron_counts() {
    // Mon 2026-11-02 is EST (UTC-5): 13:30Z = 08:30 ET in window; 12:30Z = 07:30 ET too early
    assert!(matches!(premarket_decision(utc("2026-11-02T13:30:00Z")), SlotDecision::InWindow{..}));
    assert_eq!(premarket_decision(utc("2026-11-02T12:30:00Z")), SlotDecision::TooEarly);
    // Fri 2026-10-30 is EDT (UTC-4): 12:30Z = 08:30 ET
    assert!(matches!(premarket_decision(utc("2026-10-30T12:30:00Z")), SlotDecision::InWindow{..}));
}
#[test] fn holidays() {
    assert_eq!(trading_day(d("2026-11-26")), DayKind::Closed);      // Thanksgiving
    assert_eq!(trading_day(d("2026-11-27")), DayKind::EarlyClose);
    assert_eq!(trading_day(d("2026-12-25")), DayKind::Closed);
    assert_eq!(trading_day(d("2026-10-31")), DayKind::Closed);      // Saturday
    assert_eq!(premarket_decision(utc("2026-11-26T13:30:00Z")), SlotDecision::NotToday);
}
#[test] fn deadline() {
    assert!(matches!(premarket_decision(utc("2026-10-30T13:24:59Z")), SlotDecision::InWindow{..})); // 09:24:59 ET
    assert!(matches!(premarket_decision(utc("2026-10-30T13:25:00Z")), SlotDecision::PastDeadline{..}));
}
```
- [ ] **Step 2:** `cargo test -p prebell-core clock` → FAIL (not defined).
- [ ] **Step 3:** Implement with `chrono-tz::America::New_York`.
- [ ] **Step 4:** `cargo test -p prebell-core clock` → PASS.
- [ ] **Step 5: Commit** `feat(core): ET clock and NYSE calendar`.

---

### Task 3: Facts model and required-section gate (`core::facts`)

**Files:** Create `crates/core/src/facts.rs`.

**Interfaces:**
- Produces (all `Serialize, Deserialize, Clone, Debug, PartialEq`):
  - `enum Field<T> { Value(T), Missing { reason: String } }`
  - `struct Quote { symbol: String, last: f64, prev_close: f64, change_pct: f64, volume: Option<u64> }`
  - `struct NewsReason { url: String, title: String, quote: String, summary_en: String }` — `quote` is verbatim source text (verified by Task 5).
  - `struct Mover { quote: Quote, name: String, reasons: Vec<NewsReason> }`
  - `struct PremarketFacts { trading_date: NaiveDate, as_of_utc: DateTime<Utc>, futures: Vec<(String /*label*/, Field<Quote>)>, gainers: Vec<Mover>, losers: Vec<Mover>, macro_items: Field<Vec<NewsReason>>, watch_list: Field<Vec<String>> }`
  - `fn missing_required(f: &PremarketFacts) -> Vec<&'static str>` → returns `"futures"` if every futures entry is Missing, `"movers"` if no mover in gainers∪losers has ≥1 reason. Empty vec = publishable.
  - `fn numbers_in_facts(f: &PremarketFacts) -> Vec<f64>` — every numeric value (last, prev_close, change_pct, volume) plus numbers inside quotes.

- [ ] **Step 1: Failing tests:** `missing_required` returns `["futures"]` when all 4 futures are Missing; `["movers"]` when movers exist but all `reasons` empty; `[]` when one future present and one mover has a reason; JSON round-trip of a full `PremarketFacts` equals original.
- [ ] **Step 2:** run → FAIL. **Step 3:** implement. **Step 4:** `cargo test -p prebell-core facts` → PASS.
- [ ] **Step 5: Commit** `feat(core): premarket facts model and gate`.

---

### Task 4: Number extraction and check (`core::numbers`)

**Files:** Create `crates/core/src/numbers.rs`.

**Interfaces:**
- Produces:
  - `struct Num { raw: String, value: f64, decimals: u8, is_percent: bool }`
  - `fn extract(text: &str, lang: Lang) -> Vec<Num>` where `enum Lang { En, Zh }` (defined here, re-exported from `lib.rs`).
  - `fn check(text: &str, lang: Lang, allowed: &[f64]) -> Result<(), Vec<String>>` — Err lists each `raw` with no match.
- Rules:
  - Ignore dates (`2026-10-29`, `Oct 29`, `10月29日`, `2026年`), clock times (`8:30`, `9:25 AM`, `上午8:30`), fiscal labels (`Q3`, `FY2027`, `第三季`), ordinals in lists (`1.` at line start), and ticker-embedded digits.
  - Multipliers: en `thousand/K`=1e3, `million/M`=1e6, `billion/B/bn`=1e9, `trillion/T`=1e12; zh `万`=1e4, `亿`=1e8, `万亿`=1e12. Accept full-width `％`, `，`, `．`, and signs `+ - − ▲ ▼ 上涨 下跌 涨 跌` (sign words affect sign only for matching).
  - Match: a draft number matches an allowed value if, after rounding the allowed value to the draft's `decimals` (in the draft's unit), they're equal; compare absolute values for percents so "下跌1.2%" matches -1.2.

- [ ] **Step 1: Failing tests (exact cases):**
```rust
check("ES futures +0.45% at 5,812.25", Lang::En, &[0.4512, 5812.25]) == Ok(())
check("Revenue hit $92.28 billion", Lang::En, &[92_279_000_000.0]) == Ok(())
check("营收922.8亿美元，增长19%", Lang::Zh, &[92_279_000_000.0, 18.97]) == Ok(())
check("NVDA下跌1.17％", Lang::Zh, &[-1.1734]) == Ok(())
check("TSLA rose 3.2%", Lang::En, &[2.9]) == Err(vec!["3.2%".into()])
check("2026-10-29 8:30 AM ET, Q3 FY2027 results", Lang::En, &[]) == Ok(())
check("2026年10月29日上午8:30，第三季", Lang::Zh, &[]) == Ok(())
check("市值1.2万亿美元", Lang::Zh, &[1.2e12]) == Ok(())
```
- [ ] **Step 2:** run → FAIL. **Step 3:** implement with `regex`. **Step 4:** `cargo test -p prebell-core numbers` → PASS.
- [ ] **Step 5: Commit** `feat(core): bilingual number extraction and check`.

---

### Task 5: Quote anchoring (`core::anchor`)

**Files:** Create `crates/core/src/anchor.rs`.

**Interfaces:**
- Produces: `fn verify_quote(source_text: &str, quote: &str) -> bool` — true if the quote appears in the source after normalizing whitespace, curly/straight quotes, and HTML entities (case-sensitive otherwise); quotes shorter than 20 chars return false.
  `fn verify_reason(source_text: &str, r: &NewsReason) -> bool` — `verify_quote` AND every number in `r.summary_en` (via `numbers::check` with `Lang::En`) is found in the quote's numbers.

- [ ] **Step 1: Failing tests:** quote with collapsed whitespace and `&amp;` → true; 19-char quote → false; quote not in text → false; reason whose summary says "12%" but quote says "10%" → false.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core anchor` PASS.
- [ ] **Step 5: Commit** `feat(core): quote-anchored verification`.

---

### Task 6: Provider traits + Yahoo `MarketData`

**Files:** Create `crates/core/src/providers.rs`, `crates/core/src/yahoo.rs`, `crates/core/tests/fixtures/yahoo_chart_ES=F.json`, `yahoo_chart_AAPL_pre.json`, `yahoo_429.html`.

**Interfaces:**
- Produces in `providers.rs` (all `#[async_trait(?Send)]`):
  - `trait Http { async fn get(&self, url: &str, headers: &[(&str,&str)]) -> Result<HttpResp, ProviderError>; async fn post_json(&self, url: &str, headers: &[(&str,&str)], body: &serde_json::Value) -> Result<HttpResp, ProviderError>; }`, `struct HttpResp { status: u16, body: String }`
  - `enum ProviderError { Http(u16), Network(String), Parse(String), Empty }`
  - `trait MarketData { async fn quote(&self, symbol: &str) -> Result<Quote, ProviderError>; async fn premarket_quotes(&self, symbols: &[String]) -> Vec<(String, Result<Quote, ProviderError>)>; }`
  - `trait NewsSearch { async fn search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>, ProviderError>; }`, `struct SearchHit { url: String, title: String, content: String }`
  - `trait LlmProvider { async fn complete_json(&self, system: &str, user: &str) -> Result<serde_json::Value, ProviderError>; }`
- Produces in `yahoo.rs`: `struct Yahoo<H: Http> { http: H }` implementing `MarketData` via `https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?interval=1m&range=1d&includePrePost=true`, with a browser-like `User-Agent`. Premarket `last` = latest bar before 09:30 ET today; `prev_close` = `meta.chartPreviousClose`; `change_pct` computed. `premarket_quotes` runs in chunks of 20 sequential-await groups (Workers subrequest budget).

- [ ] **Step 1:** Record the fixtures from the live endpoint (`curl` once; store bodies verbatim).
- [ ] **Step 2: Failing tests** with a `FakeHttp` mapping URL → fixture: ES=F fixture parses to a `Quote` with `prev_close == meta.chartPreviousClose`; status 429 → `Err(ProviderError::Http(429))`; HTML body with 200 → `Err(ProviderError::Parse(_))`; JSON with empty `result` → `Err(ProviderError::Empty)`.
- [ ] **Step 3–4:** implement → `cargo test -p prebell-core yahoo` PASS.
- [ ] **Step 5: Commit** `feat(core): provider traits and Yahoo market data`.

---

### Task 7: Tavily `NewsSearch` and DeepSeek `LlmProvider`

**Files:** Create `crates/core/src/tavily.rs`, `crates/core/src/deepseek.rs`, fixtures `tavily_search.json`, `deepseek_ok.json`, `deepseek_bad_json.json`.

**Interfaces:**
- Consumes: `Http`, `NewsSearch`, `LlmProvider`, `ProviderError` (Task 6).
- Produces: `struct Tavily<H> { http: H, api_key: String }` → POST `https://api.tavily.com/search` with `{query, max_results, topic:"news", days:2, include_raw_content:true}`; `SearchHit.content` = `raw_content` falling back to `content`.
  `struct DeepSeek<H> { http: H, api_key: String, model: String /* "deepseek-chat" */ }` → POST `https://api.deepseek.com/chat/completions` with `response_format: {"type":"json_object"}`, `temperature: 0.3`; returns parsed `choices[0].message.content` as JSON.
  `fn llm_from_env(provider: &str, ...) -> Box<dyn LlmProvider>` — only `"deepseek"` for now; unknown → error.

- [ ] **Step 1: Failing tests:** Tavily fixture → hits with non-empty `content`; DeepSeek ok fixture → `Value` object; content that isn't JSON → `ProviderError::Parse`; request body sent to FakeHttp contains `"response_format":{"type":"json_object"}` and `Authorization: Bearer <key>`.
- [ ] **Step 2–4:** implement → `cargo test -p prebell-core tavily deepseek` PASS.
- [ ] **Step 5: Commit** `feat(core): Tavily search and DeepSeek LLM providers`.

---

### Task 8: Build premarket facts (`core::premarket`)

**Files:** Create `crates/core/src/premarket/mod.rs`, `universe.rs`, `crates/core/data/universe.txt`.

**Interfaces:**
- Consumes: `MarketData`, `NewsSearch`, `LlmProvider`, `anchor::verify_reason`, facts types.
- Produces:
  - `fn universe() -> Vec<String>` (S&P 500 ∪ Nasdaq-100 tickers from `data/universe.txt`, `include_str!`), `fn yahoo_symbol(t: &str) -> String` (`BRK.B` → `BRK-B`).
  - `async fn build_facts(md: &dyn MarketData, ns: &dyn NewsSearch, llm: &dyn LlmProvider, trading_date: NaiveDate, now: DateTime<Utc>) -> PremarketFacts`
- Rules: movers = universe quotes with `last ≥ 5.0` and premarket `volume ≥ 50_000`, top 5 by `change_pct` and bottom 5; for each mover, search `"{name} ({ticker}) stock"` (max 3), then ask the LLM to return `{reasons:[{url,title,quote,summary_en}]}` using only the provided hits; keep only reasons passing `verify_reason` against that hit's content; macro = search `"stock market futures today"` → same extraction. Any provider error → that field `Missing{reason}`.

- [ ] **Step 1: Failing tests (fakes):** `yahoo_symbol("BRK.B") == "BRK-B"`; a mover whose LLM reason quote is not in the hit text ends with `reasons == []`; a stock at $3.10 is never a mover; MarketData returning `Err(Http(429))` for all futures → all futures `Missing`, and `missing_required` includes `"futures"`.
- [ ] **Step 2–4:** implement → `cargo test -p prebell-core premarket` PASS.
- [ ] **Step 5: Commit** `feat(core): premarket facts builder`.

---

### Task 9: Drafting EN/ZH with number check (`core::premarket::draft`)

**Files:** Create `crates/core/src/premarket/draft.rs`, `crates/core/prompts/premarket_v1_en.md`, `premarket_v1_zh.md`.

**Interfaces:**
- Consumes: `LlmProvider`, `PremarketFacts`, `numbers::{check, Lang}`, `facts::numbers_in_facts`.
- Produces:
  - `struct Draft { lang: Lang, title: String, summary: String, sections: Vec<(String /*heading*/, String /*markdown*/)> }`
  - `fn render_markdown(d: &Draft, f: &PremarketFacts) -> String` — code renders the futures and movers tables from facts (numbers never come from the LLM for tables); LLM sections follow; ends with sources list and disclaimer string (`DISCLAIMER_EN` / `DISCLAIMER_ZH` constants).
  - `async fn write(llm: &dyn LlmProvider, f: &PremarketFacts, lang: Lang) -> Result<(Draft, String /*markdown*/), Vec<String>>` — prompt with facts JSON; LLM returns `{title, summary, sections:[{heading, markdown}]}`; run `check` over title+summary+sections with `numbers_in_facts`; on failure retry once with the error list appended; second failure → `Err(errors)`.
- Prompt rules (both files): write from facts only; missing fields printed as "n/a" / "暂缺"; no investment advice; no numbers not in facts; ZH in Simplified Chinese using 亿/万; headings: EN "Futures", "Biggest premarket movers", "Macro & overnight", "What to watch"; ZH "期货", "盘前异动", "宏观与隔夜", "今日关注".

- [ ] **Step 1: Failing tests (FakeLlm returning scripted JSON):** clean draft → `Ok`; first reply with an unbacked "7.5%" then clean reply → `Ok` and FakeLlm saw 2 calls with the second prompt containing `"7.5%"`; two bad replies → `Err(["7.5%"])`; rendered markdown contains the facts' ES=F `last` exactly and contains `DISCLAIMER_ZH` for `Lang::Zh`.
- [ ] **Step 2–4:** implement → `cargo test -p prebell-core draft` PASS.
- [ ] **Step 5: Commit** `feat(core): bilingual premarket drafting with number check`.

---

### Task 10: Run state machine + Store (`core::run`, `core::store`)

**Files:** Create `crates/core/src/run.rs`, `crates/core/src/store.rs`.

**Interfaces:**
- Produces:
  - `trait Store { async fn get_run(&self, key: &str) -> Option<RunRow>; async fn upsert_run(&self, r: &RunRow); async fn upsert_post(&self, p: &PostRow) -> i64; async fn upsert_translation(&self, t: &TranslationRow); async fn add_sources(&self, post_id: i64, s: &[NewsReason]); async fn add_alert(&self, run_key: &str, level: &str, msg: &str); async fn put_facts(&self, key: &str, json: &str); }` with rows mirroring Task 1 columns; `struct MemStore` for tests.
  - `fn run_key(trading_date: NaiveDate) -> String` = `"premarket:YYYY-MM-DD"`.
  - `async fn tick(now: DateTime<Utc>, store: &dyn Store, md: &dyn MarketData, ns: &dyn NewsSearch, llm: &dyn LlmProvider) -> TickOutcome` — writes `posts.template_id = "premarket:v1"` and slug `premarket-YYYY-MM-DD` where `enum TickOutcome { Nothing, Published, PublishedEnOnly, Retrying(String), Skipped }`.
- Logic: `premarket_decision(now)` → `NotToday|TooEarly` ⇒ `Nothing`; run already `published` with ZH `published` ⇒ `Nothing`; run `published` with ZH `pending` ⇒ retry only ZH; `InWindow` ⇒ build facts → if `missing_required` non-empty ⇒ status `pending`, attempts+1, `Retrying`; else write EN and ZH → EN ok & ZH ok ⇒ both `published`; EN ok & ZH err ⇒ EN `published`, ZH `pending` (`PublishedEnOnly`); EN err ⇒ nothing published, `Retrying`; `PastDeadline` and not published ⇒ status `skipped`, alert `error` ⇒ `Skipped`. ZH retries continue after 9:25 until the end of that ET day.

- [ ] **Step 1: Failing tests with MemStore + fakes:** two `tick`s at the same InWindow time produce exactly one post row (Review Focus 5); all-429 market data at 08:30, 08:35 ⇒ `Retrying`, then at 09:25 ⇒ `Skipped` with one alert and no post (Review Focus 3); EN ok + ZH bad twice ⇒ `PublishedEnOnly`, next tick with good ZH ⇒ ZH `published`; Thanksgiving ⇒ `Nothing`.
- [ ] **Step 2–4:** implement → `cargo test -p prebell-core run` PASS.
- [ ] **Step 5: Commit** `feat(core): premarket run state machine`.

---

### Task 11: Generator Worker wiring

**Files:** Modify `crates/generator/src/lib.rs`.

**Interfaces:**
- Consumes: `core::run::tick`, `Store`, `Http`, providers.
- Produces: `struct WorkerHttp` (impl `Http` via `worker::Fetch`), `struct D1Store` (impl `Store` via `env.d1("DB")`, facts JSON to R2 `FACTS` at `premarket/YYYY-MM-DD.json`), `#[event(scheduled)]` → sends `{"job":"premarket"}` to queue `JOBS`; `#[event(queue)]` → runs `tick(Utc::now(), …)`; after `add_alert`, if `ALERT_WEBHOOK_URL` set, POST `{"content": msg}` (Discord-compatible) and mark `sent=1`. `#[event(fetch)]` → only `GET /health` returning `ok`.

- [ ] **Step 1:** `worker-build --release` → builds.
- [ ] **Step 2: Verify locally:** `.dev.vars` with real keys; `pnpm exec wrangler dev --test-scheduled --local` in `crates/generator`; `curl "http://localhost:8787/__scheduled?cron=*/5+12-14+*+*+1-5"`; then `wrangler d1 execute prebell --local --command "select status, attempts from runs"` → a row (status depends on the current ET time: outside the window expect no row).
- [ ] **Step 3: Verify the build path out of window:** add a dev-only env var `FORCE_NOW_UTC` read in the queue handler (ignored when unset) and run with `FORCE_NOW_UTC=<today>T12:35:00Z` (EDT) or `13:35Z` (EST) → `runs.status = published` and two `post_translations` rows, or `pending` with a logged reason. Record the outcome in `docs/plan-1-notes.md`.
- [ ] **Step 4: Commit** `feat(generator): cron, queue, D1/R2 store, alerts`.

---

### Task 12: Next.js site on Cloudflare (`apps/web`)

**Files:** Create everything under `apps/web/` listed in File Structure.

**Interfaces:**
- Consumes: D1 tables from Task 1 (binding `DB`, same database id as generator).
- Produces:
  - `getLatestPremarket(lang): Promise<PostView | null>`, `getPostBySlug(slug, lang): Promise<PostView | null>`, `listPosts(lang, page): Promise<PostListItem[]>` in `src/lib/posts.ts`, where `PostView = { slug, tradingDate, publishedAt, title, summary, bodyMd, status: 'published'|'pending', sources: {url,title}[], hasOtherLang: boolean }`.
  - Pages: `/[locale]` home (latest premarket brief, then list), `/[locale]/p/[slug]`, `/[locale]/archive?page=N`, `/[locale]/about`. Root `/` redirects to `/en`.
- Rules: ZH `pending` → page renders `PendingNotice` ("中文版稍后推出" + link to EN), `<meta name="robots" content="noindex">`, and no hreflang pair; published pairs emit `<link rel="alternate" hreflang="en|zh-Hans">`. Times shown as ET (`Intl.DateTimeFormat` with `timeZone: 'America/New_York'`). `Disclaimer` on every page from messages files. Markdown rendered server-side (`react-markdown` + `remark-gfm`). Fonts via `next/font/local` (Inter + Noto Sans SC subset files committed). Cloudflare Web Analytics beacon script in the layout (token from env `CF_BEACON_TOKEN`, omitted when unset). Archive by ticker is deferred to Plan 3. Responses `Cache-Control: public, s-maxage=60` (no ISR in Plan 1; designer's call replacing spec's revalidation step).

- [ ] **Step 1: Failing tests** `tests/posts.test.ts` (Vitest, D1 mocked with an in-memory SQLite via `better-sqlite3` loaded with `migrations/0001_init.sql`): `getPostBySlug('premarket-2026-10-29','zh')` on a pending row returns `status:'pending'`; `listPosts('zh',1)` excludes `failed`; `getLatestPremarket('en')` returns the newest `trading_date`.
- [ ] **Step 2:** `pnpm --filter web test` → FAIL. **Step 3:** implement. **Step 4:** PASS.
- [ ] **Step 5: Verify:** `pnpm --filter web exec opennextjs-cloudflare build && pnpm --filter web exec opennextjs-cloudflare preview` against the local D1 seeded by Task 11 → `/en` shows the brief with tables and disclaimer; `/zh/p/<slug>` shows Chinese or the pending notice; view-source shows hreflang only when both published.
- [ ] **Step 6: Commit** `feat(web): bilingual Next.js site on Cloudflare`.

---

### Task 13: Deploy and first live run

**Files:** Modify `crates/generator/wrangler.toml`, `apps/web/wrangler.jsonc` (real D1/R2/queue IDs); extend `docs/runbook.md` (created by Plan 5 with the LLM provider section).

- [ ] **Step 1:** Owner runs `pnpm exec wrangler login` (interactive). Create resources: `wrangler d1 create prebell`, `wrangler r2 bucket create prebell-facts`, `wrangler queues create prebell-jobs`; apply migrations `--remote`; set secrets `ANTHROPIC_API_KEY` (after Plan 5 merges; `LLM_PROVIDER="claude"`), `TAVILY_API_KEY`, `ALERT_WEBHOOK_URL`. `DEEPSEEK_API_KEY` is only needed if switching back to DeepSeek.
- [ ] **Step 2:** `wrangler deploy` (generator) and `opennextjs-cloudflare deploy` (web) to `*.workers.dev`. Confirm with the owner before each deploy.
- [ ] **Step 3: Verify:** next trading day at ~8:30–8:40 ET, `wrangler d1 execute prebell --remote --command "select r.status, t.lang, t.status from runs r join posts p using(run_key) join post_translations t on t.post_id=p.id"` → published rows; `https://prebell-web.<account>.workers.dev/en` shows the brief. If Yahoo blocks Cloudflare IPs (429 every attempt), record it in `docs/runbook.md` and stop: switching `MarketData` provider is a decision for the owner.
- [ ] **Step 4:** Extend `docs/runbook.md` (it already has the LLM provider section from Plan 5): secrets, cron times, how to rerun a day (`FORCE_NOW_UTC` is dev-only; production rerun = delete the run row and wait for the next tick), where alerts go.
- [ ] **Step 5: Commit** `chore: deploy config and runbook`.

---

## Parallelization (for subagent execution)

- **Wave 1:** Task 1.
- **Wave 2 (parallel):** Tasks 2, 3, 4, 6, 12-scaffold (web can build against the schema with seeded rows).
- **Wave 3 (parallel):** Tasks 5 (needs 3, 4), 7 (needs 6).
- **Wave 4:** Task 8 (needs 5, 6, 7) → Task 9 (needs 4, 8) → Task 10 (needs 2, 3, 9).
- **Wave 5:** Task 11 (needs 10), then Task 12 verification against real data, then Task 13.
