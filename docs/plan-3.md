# Plan 3 — Earnings detection, health checks, charts and calendar

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a watchlist company files an 8-K with Item 2.02 on EDGAR, the generator finds it within minutes, pulls the Exhibit 99.1 press release, extracts the headline figures with verbatim quotes that code checks against the exhibit, adds GAAP history from XBRL `companyfacts`, draws GAAP-only SVG charts, and publishes an English + Simplified Chinese "earnings health check" that passes the number check. A calendar page shows upcoming report dates, marked confirmed only with primary proof.

**Split:** Plan 3a = Tasks 1–7 and 10 (watchlist, EDGAR, detection, extraction, drafting, run state machine, Worker wiring). Plan 3b = Tasks 8, 9 and 11–12 (XBRL trend, charts, calendar, web pages). Both ship on `feat/plan-3-earnings`.

**Architecture:** Same shape as Plan 1. All logic lives in `crates/core` (pure Rust, tested natively with fakes); `crates/generator` only wires Workers bindings to the core traits. Earnings code sits in its own module (`core::earnings`) with its own store trait (`EarningsStore`), so the shared `Store`, `run.rs` and `clock.rs` stay untouched apart from the small changes listed under "Shared-code changes". The site reads the new tables from D1 and the chart SVGs from R2.

**Tech Stack:** Plan 1's stack plus `plotters` 0.3 (`default-features = false`, `svg_backend` only) for charts.

**Spec:** `docs/spec.md` §3 goals 3–4, §7 earnings detection + calendar, §8 rules. Plan 1 (`docs/plan-1.md`) conventions apply unless restated.

## Global Constraints

- Everything from Plan 1's Global Constraints (languages, ET schedule, UTC storage, missing data never guessed, EN-first/ZH-pending, no AlphaLab content, secrets only via `wrangler secret put`).
- **SEC fair access:** every EDGAR request sends `User-Agent: PreBell/0.1 (+https://prebell.ai; <SEC_CONTACT>)` where `SEC_CONTACT` is the existing wrangler var. Requests are sequential with a pause of at least **125 ms** between them (≤ 8 req/s per invocation). Polling is spread: one tick polls one batch of at most **21** CIKs, so the 102 domestic watchlist companies cycle in 5 one-minute ticks.
- **Polling window:** weekdays **06:00 ≤ ET < 22:00** (spec: ~6 AM–9 PM, plus one hour of slack for late filers). Builds and ZH retries run in the same window.
- **Detection rule:** form exactly `8-K` (not `8-K/A`), `items` contains `2.02`, accepted within the last **96 h** (covers a Saturday filing picked up Monday), accession not seen before. 6-K/20-F filers are `foreign` in the watchlist: calendar only, never polled, never auto-written.
- **Quote-anchored figures:** a press-release figure is kept only if (a) its quote is ≥ 20 chars and appears in the fetched exhibit text (`anchor::verify_quote`), (b) its `amount` string appears inside that quote, and (c) `amount` parses to exactly one number. Anything else is `Missing { reason }`. A negative value needs a parenthesised amount, a minus sign, or the word "loss" in the quote.
- **No consensus, no beat/miss, no estimates.** The Yahoo calendar's EPS-estimate column is never parsed or stored. Prompts forbid "beat", "miss", "expectations", "consensus", "estimates"; drafts containing those words (EN) or 预期/预估/共识/超出/不及 (ZH) are rejected like an unbacked number.
- **Charts are GAAP only:** quarterly GAAP values from XBRL (`us-gaap` duration facts of 80–100 days, from 10-Q/10-K) plus the current quarter's GAAP figure from the press release. Non-GAAP appears only as text for the current quarter. Q4 is never derived (FY − 9M); a missing quarter is a gap.
- **Charts never measure text:** `plotters` on `wasm32-unknown-unknown` measures text through `web_sys::window()`, which does not exist in a Worker. Charts use only `SVGBackend` + `DrawingArea` primitives (`Rectangle`, `PathElement`, `Text` with explicit anchors). No `ChartBuilder`, `configure_mesh`, `titled` or `estimate_text_size`.
- **Required sections (earnings check):** revenue, at least one EPS figure (GAAP or non-GAAP), and a filing link. Optional: net income, margins, cash flow, guidance, highlights → "n/a" / "暂缺".
- **Late, not thin:** a filing whose required figures are missing retries every 5 min until **6 h** after acceptance, then the run is `failed` with one `error` alert. ZH retries continue until **24 h** after acceptance, then ZH is `failed` with one `warn` alert.
- Post: template `earnings:v1`, `content_type = 'earnings'`, run key `earnings:{TICKER}:{accession}`, slug `{ticker}-{fiscal-period}-earnings` (e.g. `aapl-q4-fy2026-earnings`, `brk-b-fy2026-earnings` for a full-year release); when the fiscal period is not verified the slug uses the ET filing date (`aapl-2026-10-30-earnings`); a slug already owned by another run gets `-{last 6 digits of accession}`.
- Every number an article publishes passes `numbers::check`: LLM text against the facts (with one retry), and the final rendered markdown (code tables included, link targets excluded) against the same allow-list as a guard. Chart label strings are checked the same way.
- Calendar: seeded from Yahoo's earnings calendar (behind `CalendarSource`); `confirmed` only with primary proof — an EDGAR 8-K Item 2.02 (`reported`) or a quote-anchored company announcement whose URL host is the company's own domain or a major press-release wire; conflicting dates go to `quarantined` with a reason, `checked_at` and `next_check_at`, and are not shown publicly.
- Schema changes only in `migrations/0003_earnings.sql`. Never edit `0001`.

## Shared-code changes (kept minimal; called out in the PR)

- `crates/core/src/lib.rs`: add `pub mod earnings; pub mod edgar; pub mod yahoo_calendar;`.
- `crates/core/src/premarket/draft.rs`: `one_line`, `is_setext_underline`, `fmt_num` become `pub(crate)` (reused by earnings drafting). No behaviour change.
- `crates/core/Cargo.toml`: add `plotters`.
- `crates/generator/src/lib.rs`: dispatch on the cron string and on `Job.job` (`"premarket"` unchanged, new `"earnings"`); build the EDGAR/calendar providers.
- `crates/generator/src/store.rs`: none; the new `EarningsStore` impl lives in `crates/generator/src/earnings_store.rs`.
- `crates/generator/wrangler.toml`: one more cron `"* * * * *"` and a second queue `prebell-earnings` (producer `EARNINGS` + its own consumer). The queue must be created before deploy (`wrangler queues create prebell-earnings`).
- `apps/web`: `tests/d1.ts` loads every `migrations/*.sql` in order; layout nav gets an "Earnings" link; the post page renders charts; home page gets a "Latest earnings checks" list; `wrangler.jsonc` gets the `FACTS` R2 binding (read-only use) for chart SVGs.

## Review Focus

1. **Fair access and subrequest budget:** never more than 21 submissions requests per tick, ≥ 125 ms apart, descriptive UA on every EDGAR request (Task 3, 7).
2. **No duplicate posts:** two ticks seeing the same accession, queue redelivery, or a re-run → one `earnings_filings` row, one post (Task 7).
3. **Quote anchoring is the only path to a published figure:** a figure whose quote is paraphrased, whose amount is not in the quote, or whose scale is wrong never reaches facts (Task 5).
4. **GAAP-only charts and the same-quarter trap:** when the company files its 10-Q the same day (e.g. Microsoft), the latest XBRL quarter *is* the current quarter and the release point must not be added twice (Task 8).
5. **Calendar honesty:** an aggregator date is never shown as confirmed; a conflict is quarantined, not silently overwritten (Task 11).
6. **wasm32:** `cargo build -p prebell-core --target wasm32-unknown-unknown` and `worker-build --release` pass; charts never call text layout (Task 9).

---

## File Structure

```
migrations/0003_earnings.sql
crates/core/
  data/watchlist.tsv                  # S&P 100 + a few foreign filers: ticker, CIK, filer kind, name, domains
  prompts/earnings_extract_v1.md      # figure extraction (quote-anchored)
  prompts/earnings_v1_en.md, earnings_v1_zh.md
  src/edgar.rs                        # Filings trait + Edgar<H> (submissions, filing index, documents, companyfacts), Pause
  src/yahoo_calendar.rs               # CalendarSource impl over Yahoo's earnings calendar HTML
  src/earnings/
    mod.rs          # window, batches, re-exports
    watchlist.rs    # WatchEntry, watchlist(), poll_batch()
    detect.rs       # new 8-K Item 2.02 filings from a submissions doc
    facts.rs        # EarningsFacts, Figure, gate, numbers allow-list
    extract.rs      # LLM extraction + verification against the exhibit text
    xbrl.rs         # GAAP quarterly trend from companyfacts
    charts.rs       # SVG charts (plotters)
    draft.rs        # EN/ZH drafting with number check and markup filter
    store.rs        # EarningsStore trait, rows, MemEarningsStore
    run.rs          # poll tick + build tick (state machine)
    calendar.rs     # CalendarSource, reconcile(), refresh tick, IR proof
  tests/fixtures/edgar_*.json|htm, yahoo_earnings_calendar.html, deepseek_earnings_extract.json
crates/generator/src/earnings_store.rs   # EarningsStore on D1
apps/web/
  src/lib/earnings.ts                    # calendar week, ticker archive, charts, latest earnings
  src/app/charts/[...key]/route.ts       # serves chart SVGs from R2
  src/app/[locale]/earnings/page.tsx     # calendar week view
  src/app/[locale]/t/[ticker]/page.tsx   # archive by ticker
  src/components/Charts.tsx, CalendarWeek.tsx
  tests/earnings.test.ts
```

---

### Task 1: Schema `0003_earnings.sql` + this plan

**Files:** Create `docs/plan-3.md`, `migrations/0003_earnings.sql`. (No smoke file under `migrations/`: wrangler applies every `.sql` there. The web tests load the schema instead, Task 12.)

**Interfaces (tables later tasks use by these exact names):**
- `earnings_filings(accession TEXT PK, cik INTEGER, ticker TEXT, form TEXT, items TEXT, filing_date TEXT, report_date TEXT, accepted_at TEXT, primary_doc TEXT, run_key TEXT UNIQUE, state TEXT CHECK IN ('open','done'), detected_at TEXT)` + index `(state, accepted_at)`, `(ticker)`.
- `post_charts(post_id INTEGER REFERENCES posts(id), name TEXT, r2_key TEXT, alt_en TEXT, alt_zh TEXT, sort INTEGER, PRIMARY KEY(post_id, name))`.
- `earnings_calendar(ticker TEXT, report_date TEXT, time_slot TEXT CHECK IN ('bmo','amc','unknown'), status TEXT CHECK IN ('unconfirmed','confirmed','reported','quarantined'), source TEXT CHECK IN ('yahoo','ir','edgar'), proof_url TEXT, proof_quote TEXT, conflict_reason TEXT, first_seen_at TEXT, checked_at TEXT, next_check_at TEXT, updated_at TEXT, PRIMARY KEY(ticker, report_date))` + index `(report_date)`.
- `kv(key TEXT PK, value TEXT, updated_at TEXT)` — small generator state (calendar refresh date).

- [ ] **Step 1:** Write the plan (this file). Commit `docs: Plan 3 earnings plan`.
- [ ] **Step 2:** Write the migration. A scratch smoke script inserts one row in each table (plus a run/post for the FK) and selects them joined.
- [ ] **Step 3: Verify** `wrangler d1 migrations apply prebell --local -c crates/generator/wrangler.toml --persist-to <scratch>` applies 0001 and 0003; the smoke script returns the joined row (local only).
- [ ] **Step 4: Commit** `feat(db): earnings schema`.

---

### Task 2: Watchlist and polling window (`core::earnings::{watchlist, mod}`)

**Files:** Create `crates/core/data/watchlist.tsv`, `src/earnings/mod.rs`, `src/earnings/watchlist.rs`.

**Interfaces:**
- `enum Filer { Domestic, Foreign }`; `struct WatchEntry { ticker: &'static str, cik: u64, filer: Filer, name: &'static str, domains: Vec<&'static str> }`
- `fn watchlist() -> Vec<WatchEntry>`; `fn find(ticker: &str) -> Option<WatchEntry>`; `fn by_cik(cik: u64) -> Option<WatchEntry>`
- `fn cik10(cik: u64) -> String` (`320193` → `"0000320193"`)
- `fn in_window(now: DateTime<Utc>) -> bool` — Mon–Fri, 06:00 ≤ ET < 22:00.
- `fn poll_batch(now: DateTime<Utc>) -> Vec<WatchEntry>` — domestic entries chunked by `BATCH = 21`; chunk index = (minutes since ET midnight) mod chunk count.

- [ ] **Step 1: Failing tests:** watchlist has ≥ 100 domestic entries, unique tickers and CIKs, every domestic entry has a domain; `cik10(320193) == "0000320193"`; `in_window` false at 05:59 ET and 22:00 ET and Saturday, true at 06:00 ET on both sides of the DST change (2026-10-30 10:00Z EDT, 2026-11-02 11:00Z EST); five consecutive minutes' `poll_batch` cover every domestic entry exactly once and no batch exceeds 21; foreign entries (e.g. `TM`) never appear in a batch.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core earnings::watchlist` PASS.
- [ ] **Step 5: Commit** `feat(core): earnings watchlist and polling window`.

---

### Task 3: EDGAR provider (`core::edgar`)

**Files:** Create `src/edgar.rs`; fixtures `tests/fixtures/edgar_submissions_AAPL.json`, `edgar_index_AAPL.htm`, `edgar_ex991_AAPL.htm`, `edgar_companyfacts_AAPL.json` (hand-written in the documented shapes; see the PR for why they are not recorded).

**Interfaces:**
- `#[async_trait(?Send)] trait Pause { async fn pause(&self, ms: u64); }`
- `struct RecentFiling { accession: String, form: String, items: String, filing_date: String, report_date: String, accepted_at: DateTime<Utc>, primary_doc: String }`; `struct Submissions { cik: u64, name: String, tickers: Vec<String>, recent: Vec<RecentFiling> }`
- `struct FilingDoc { name: String, doc_type: String, url: String }`
- `#[async_trait(?Send)] trait Filings { async fn submissions(&self, cik: u64) -> Result<Submissions, ProviderError>; async fn filing_docs(&self, cik: u64, accession: &str) -> Result<Vec<FilingDoc>, ProviderError>; async fn document_text(&self, url: &str) -> Result<String, ProviderError>; async fn company_facts(&self, cik: u64) -> Result<CompanyFacts, ProviderError>; }`
- `struct Edgar<H: Http, P: Pause> { http, pause, user_agent }` with `Edgar::new(http, pause, contact) -> Result<Self, ProviderError>` (empty contact → `Parse("SEC_CONTACT is empty")`). URLs: `https://data.sec.gov/submissions/CIK{cik10}.json`, `https://www.sec.gov/Archives/edgar/data/{cik}/{acc-no-dashes}/{acc}-index.htm`, `https://data.sec.gov/api/xbrl/companyfacts/CIK{cik10}.json`.
- `fn filing_url(cik, accession) -> String` (the `-index.htm` page, linked from posts); `fn exhibit_991(docs: &[FilingDoc]) -> Option<&FilingDoc>` (type `EX-99.1`, else the first `EX-99*` HTML/text exhibit); `fn html_to_text(html: &str) -> String` (drops `<script>/<style>`, turns block tags and `<td>` into separators, decodes entities via the anchor normaliser rules, collapses whitespace).
- `CompanyFacts` keeps only the concepts Task 8 needs (typed serde struct; other concepts are skipped without allocation — the real file is several MB).

- [ ] **Step 1: Failing tests (FakeHttp):** submissions fixture → `recent` zipped from the columnar arrays, `accepted_at` parsed; every request carries the UA with the contact and goes through `pause(≥125)` except the first; 403/429 → `Http(status)`; HTML index fixture → `exhibit_991` finds `EX-99.1` with an absolute URL; `html_to_text` keeps "Total net sales 94,930" as one line of text and decodes `&#8217;` and `&nbsp;`; empty contact → error.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core edgar` PASS.
- [ ] **Step 5: Commit** `feat(core): EDGAR filings provider`.

---

### Task 4: Detection (`core::earnings::detect`)

**Files:** Create `src/earnings/detect.rs`.

**Interfaces:**
- `fn earnings_filings(sub: &Submissions, now: DateTime<Utc>) -> Vec<&RecentFiling>` — form `8-K`, items split on `,` contains exactly `2.02`, `now - accepted_at ≤ 96h`.
- `fn ticker_matches(sub: &Submissions, entry: &WatchEntry) -> bool` — the submissions doc lists the watchlist ticker (BRK.B ↔ `BRK-B`), guarding a wrong CIK in the data file.

- [ ] **Step 1: Failing tests:** an `8-K/A` with 2.02 is ignored; items `"2.020"`/`"12.02"` are not 2.02; a 2.02 8-K accepted 97 h ago is ignored; `"2.02,9.01"` is detected; ticker mismatch is reported.
- [ ] **Step 2–4:** FAIL → implement → PASS. **Step 5: Commit** `feat(core): detect 8-K Item 2.02 filings`.

---

### Task 5: Earnings facts + quote-anchored extraction (`core::earnings::{facts, extract}`)

**Files:** Create `src/earnings/facts.rs`, `src/earnings/extract.rs`, `prompts/earnings_extract_v1.md`.

**Interfaces:**
- `enum Unit { Usd, UsdPerShare, Percent }`; `struct Figure { value: f64 /* base units */, unit: Unit, amount: String, quote: String }`
- `struct Metrics { revenue, net_income, eps_gaap, eps_non_gaap, gross_margin, operating_margin, operating_cash_flow, free_cash_flow: Field<Figure> }`
- `struct Note { quote: String, summary_en: String }` (guidance, highlights)
- `struct FiscalPeriod { quarter: Option<u8>, fiscal_year: i32, quote: String }`
- `struct QuarterPoint { period_end: NaiveDate, value: f64, form: String, filed: NaiveDate }`; `struct Trend { revenue: Vec<QuarterPoint>, eps_diluted: Vec<QuarterPoint>, net_income: Vec<QuarterPoint> }`
- `struct EarningsFacts { ticker, company, cik: u64, accession, filing_date: NaiveDate, accepted_at: DateTime<Utc>, filing_url, exhibit_url: Field<String>, fiscal_period: Field<FiscalPeriod>, metrics: Metrics, guidance: Field<Vec<Note>>, highlights: Field<Vec<Note>>, trend: Field<Trend> }`
- `fn missing_required(f: &EarningsFacts) -> Vec<&'static str>` → `"revenue"`, `"eps"`, `"filing"`.
- `fn numbers_in_facts(f: &EarningsFacts) -> Vec<f64>` — figure values, every number in kept quotes (both signs when unsigned), trend values, the fiscal year and filing year ± 1.
- `async fn extract(llm: &dyn LlmProvider, exhibit_text: &str, company: &str, ticker: &str) -> Extracted` where `Extracted { fiscal_period, metrics, guidance, highlights }`; LLM errors → every field `Missing`.
- `fn verify_figure(text: &str, unit: Unit, raw: &RawFigure) -> Result<Figure, String>` (pure; the rules in Global Constraints).

- [ ] **Step 1: Failing tests:** revenue `"$94.9 billion"` quoted verbatim → `value == 94.9e9`; table amount `"94,930"` with scale `millions` → `94.93e9`; quote not in exhibit → Missing; amount not in quote → Missing; `"$(0.25)"` → `-0.25`; "negative" without parentheses/minus/"loss" → Missing; EPS `"$1.85"` is `UsdPerShare`; margin `"47.2%"` is `Percent`; `missing_required` → `["eps"]` with only revenue; fiscal period quote must contain the year digits; facts JSON round-trips.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core earnings::extract earnings::facts` PASS.
- [ ] **Step 5: Commit** `feat(core): quote-anchored earnings figures`.

---

### Task 6: Drafting EN/ZH (`core::earnings::draft`)

**Files:** Create `src/earnings/draft.rs`, `prompts/earnings_v1_en.md`, `prompts/earnings_v1_zh.md`. Modify `src/premarket/draft.rs` (visibility only).

**Interfaces:**
- `const TEMPLATE_ID: &str = "earnings:v1"`, `DISCLAIMER_EN`, `DISCLAIMER_ZH`.
- Headings EN `"Results"`, `"Margins and cash flow"`, `"Guidance"`, `"What else stood out"`; ZH `"业绩概览"`, `"利润率与现金流"`, `"业绩指引"`, `"其他要点"`.
- `fn render_markdown(d: &Draft, f: &EarningsFacts) -> String` — key-figures table from facts (n/a / 暂缺 for Missing), LLM sections, "Sources" with the 8-K and the press release, disclaimer.
- `async fn write(llm, f, lang) -> Result<(Draft, String), Vec<String>>` — same contract as `premarket::draft::write` (one retry with the problem list); also rejects forbidden consensus words and fails if the full rendered markdown (link targets removed) fails `numbers::check`.
- `fn slug(f: &EarningsFacts) -> String`, `fn period_label(f, lang) -> Option<String>` ("Fiscal Q4 2026" / "2026财年第四季度"). The company name and ticker are masked before the full-page number check ("3M" is a name, not 3 million).

- [ ] **Step 1: Failing tests (FakeLlm):** clean draft → Ok and markdown contains the revenue figure formatted from facts and both source links; unbacked "12%" then clean → Ok after 2 calls with "12%" in the second prompt; "beat estimates" twice → Err; a section with `[x](http…)` → Err; ZH draft with 亿 amounts matching facts → Ok; slug `aapl-q4-fy2026-earnings`, `brk-b-...`, and date fallback.
- [ ] **Step 2–4:** FAIL → implement → PASS. **Step 5: Commit** `feat(core): bilingual earnings drafting`.

---

### Task 7: Earnings store + run state machine (`core::earnings::{store, run}`)

**Files:** Create `src/earnings/store.rs`, `src/earnings/run.rs`.

**Interfaces:**
- Rows: `FilingRow` (mirrors `earnings_filings`), `ChartRow` (mirrors `post_charts`), `CalendarRow` (mirrors `earnings_calendar`).
- `#[async_trait(?Send)] trait EarningsStore { insert_filing(&FilingRow) -> Result<bool /*inserted*/>; open_filings() -> Result<Vec<FilingRow>>; set_filing_state(accession, state); slug_owner(slug) -> Result<Option<String /*run_key*/>>; set_charts(post_id, &[ChartRow]); calendar_for(ticker) -> Result<Vec<CalendarRow>>; calendar_between(from, to) -> Result<Vec<CalendarRow>>; upsert_calendar(&CalendarRow); delete_calendar(ticker, date); kv_get(key); kv_set(key, value, now) }`, all `Result<_, String>`; `MemEarningsStore` for tests.
- `async fn poll_tick(now, es: &dyn EarningsStore, store: &dyn Store, filings: &dyn Filings) -> PollOutcome { checked, detected: Vec<String>, errors: Vec<String> }` — outside window ⇒ nothing; for each `poll_batch` entry: submissions → ticker guard → each detected filing: `insert_filing` (INSERT OR IGNORE), create the `pending` run, mark the calendar entry `reported`. One company's error never stops the batch.
- `async fn build_tick(now, es, store, filings, llm) -> Vec<(String, TickOutcome)>` — for each open filing (oldest first; at most **1 build and 1 ZH retry** per tick, budgeted separately so a failing ZH draft never holds up new posts; ZH retries ≥ 5 min apart; a stale `building` run past the deadline fails with one alert instead of rebuilding; the build deadline is 6 h after acceptance but at least 1 h after detection; the caller gates it to the polling window): run `published` & ZH `pending` ⇒ ZH retry (24 h cap); run `building` and fresh (< 15 min) ⇒ skip; `pending` and last attempt < 5 min ago ⇒ skip; else build: docs → EX-99.1 → text → extract → companyfacts trend → facts → gate (missing ⇒ `pending` or, past 6 h, `failed` + alert) → charts → EN/ZH → write post, translations, sources (8-K + exhibit), charts, facts JSON → run `published`; filing `done` when EN and ZH are final (published/failed) or the run failed.
- Uses Plan 1's `TickOutcome`.

- [ ] **Step 1: Failing tests (MemStore + MemEarningsStore + fakes):** same accession seen by two polls ⇒ one filing row, one run; a fake that 429s one CIK still checks the rest; a full build publishes one post with slug, template `earnings:v1`, two charts, sources = [8-K, EX-99.1], both translations; rebuild of the same filing does not create a second post; missing revenue at +5 h ⇒ `pending`, at +6 h ⇒ `failed` + one alert; EN ok + ZH bad ⇒ ZH `pending` and next tick fills it; a slug collision with another run gets the accession suffix; at most one build per tick.
- [ ] **Step 2–4:** FAIL → implement → PASS. **Step 5: Commit** `feat(core): earnings run state machine`.

---

### Task 8: GAAP trend from XBRL (`core::earnings::xbrl`)

**Files:** Create `src/earnings/xbrl.rs`.

**Interfaces:**
- `fn trend(cf: &CompanyFacts) -> Trend` — revenue from the first of `Revenues`, `RevenueFromContractWithCustomerExcludingAssessedTax`, `RevenuesNetOfInterestExpense`, `SalesRevenueNet` with ≥ 1 discrete quarter (most recent period end wins ties); EPS from `EarningsPerShareDiluted` (`USD/shares`); net income from `NetIncomeLoss`. Discrete quarter = `start`..`end` of 80–100 days, form `10-Q` or `10-K`(/A); per `end` keep the latest `filed`. Last 8 quarters, ascending.
- `fn release_already_in_trend(points: &[QuarterPoint], filing_date: NaiveDate) -> bool` — per series: the latest point was `filed ≥ filing_date` and ended ≤ 75 days before it (a late amendment of an older quarter does not count). Revenue merges the candidate tags by period end (first-listed tag wins a quarter both report).

- [ ] **Step 1: Failing tests:** 9-month YTD and annual facts are ignored; a restated quarter keeps the latest `filed`; no Q4 is derived; a company with only `RevenueFromContract…` uses it; same-day 10-Q ⇒ `release_already_in_trend == true`.
- [ ] **Step 2–4:** FAIL → implement → PASS. **Step 5: Commit** `feat(core): GAAP quarterly trend from companyfacts`.

---

### Task 9: SVG charts (`core::earnings::charts`)

**Files:** Create `src/earnings/charts.rs`; modify `crates/core/Cargo.toml` (`plotters = { version = "0.3", default-features = false, features = ["svg_backend"] }`).

**Interfaces:**
- `struct Chart { name: &'static str /* "revenue" | "eps" */, svg: String, alt_en: String, alt_zh: String, labels: Vec<String> }`
- `fn charts(f: &EarningsFacts) -> Vec<Chart>` — bars per quarter (XBRL, x label `YYYY-MM` of the period end; one language-neutral SVG serves EN and ZH), plus the press-release GAAP value as the last bar labelled `8-K` unless that series' latest XBRL quarter already is the release quarter; a missing quarter is an empty slot; a chart needs ≥ 2 bars; value labels on the first and last bar only; transparent background; revenue in $B with one decimal, EPS in $ with two decimals; negative bars drawn below a zero line. Labels are code-formatted from facts values and checked with `numbers::check` against `numbers_in_facts` (a failing chart is dropped, never published).
- R2 key: `charts/earnings/{ticker}/{accession}/{name}.svg`.

- [ ] **Step 1: Failing tests:** output starts with `<svg`, contains one `<rect` per bar, contains `8-K` only when the release quarter is not already in the series; gaps for missing quarters; non-GAAP EPS never appears; < 2 points ⇒ no chart; every label passes `numbers::check`; text is XML-escaped.
- [ ] **Step 2–4:** FAIL → implement → PASS; `cargo build -p prebell-core --target wasm32-unknown-unknown` PASS.
- [ ] **Step 5: Commit** `feat(core): GAAP-only SVG earnings charts`.

---

### Task 10: Generator wiring

**Files:** Create `crates/generator/src/earnings_store.rs`; modify `crates/generator/src/lib.rs`, `wrangler.toml`.

**Interfaces:**
- `impl EarningsStore for D1Store` (SQL per Task 1 tables; `insert_filing` is `INSERT … ON CONFLICT(accession) DO NOTHING RETURNING accession`).
- `struct WorkerPause` (`worker::Delay`).
- `scheduled`: cron `"* * * * *"` ⇒ if `earnings::in_window(now)` send `Job { job: "earnings", at: <cron time> }` to a **separate queue** `prebell-earnings` (binding `EARNINGS`, consumer `max_batch_size = 1`, `max_concurrency = 1`), so a minutes-long build never delays the premarket job and never runs twice in parallel against EDGAR; other crons ⇒ `"premarket"` on `JOBS` as before. `Job` gains an optional `at` (the cron time picks the poll batch; a message older than 3 min skips its poll but still builds). Earnings refuse to run with the placeholder `SEC_CONTACT`.
- `queue`: `"premarket"` ⇒ Plan 1 `tick`; `"earnings"` ⇒ `poll_tick`, then `calendar::refresh_tick`, then `build_tick`; alerts delivered after the batch as before. A warning is logged when `SEC_CONTACT` is the placeholder.

- [ ] **Step 1:** `worker-build --release` → builds. `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 2: Commit** `feat(generator): earnings poll, build and calendar jobs`.

---

### Task 11: Earnings calendar (`core::earnings::calendar`, `core::yahoo_calendar`)

**Files:** Create `src/earnings/calendar.rs`, `src/yahoo_calendar.rs`, fixture `tests/fixtures/yahoo_earnings_calendar.html`.

**Interfaces:**
- `enum TimeSlot { Bmo, Amc, Unknown }`; `struct Seed { ticker: String, date: NaiveDate, slot: TimeSlot }`
- `#[async_trait(?Send)] trait CalendarSource { async fn day(&self, date: NaiveDate) -> Result<Vec<Seed>, ProviderError>; }` — `YahooCalendar<H>` fetches `https://finance.yahoo.com/calendar/earnings?day=YYYY-MM-DD&offset=N&size=100` (≤ 3 pages), maps columns by header text (Symbol, Earnings Call Time), never reads estimate columns.
- `struct Proof { url: String, quote: String, date: NaiveDate, source: Source }` where `Source { Ir, Edgar }`.
- `fn reconcile(existing: &[CalendarRow], seed: Option<&Seed>, proof: Option<&Proof>, today: NaiveDate, now: DateTime<Utc>) -> Vec<Change>`, `enum Change { Upsert(CalendarRow), Delete { ticker, date } }` — pure rules:
  1. EDGAR proof for date D ⇒ D `reported` (source `edgar`); any other upcoming row of that ticker within ±21 days of D is deleted.
  2. IR proof for D and no conflicting upcoming row ⇒ D `confirmed` (source `ir`).
  3. IR proof for D but the aggregator says E ≠ D ⇒ both collapse to one `quarantined` row at D with reason `"aggregator says E; company says D"`, `next_check_at = now + 12h`.
  4. Aggregator seed E, existing `confirmed` row D ≠ E within ±21 days ⇒ D becomes `quarantined` (reason as above); E is not inserted.
  5. Aggregator seed E, existing `unconfirmed` row D ≠ E ⇒ delete D, insert E `unconfirmed` (aggregator moved its own guess).
  6. Aggregator seed matching an existing row ⇒ refresh `checked_at`/slot only; status never downgraded.
  7. Quarantined row whose aggregator now agrees with its proof ⇒ `confirmed`, reason cleared.
  8. Quarantined row with proof, when the aggregator was read in full and no longer lists the ticker in the window ⇒ `confirmed` (its conflicting date is gone). A seen-again conflict moves `next_check_at` on.
  A proof dated before today or more than 21 days from the row it checks is ignored.
- `fn proof_from_hit(entry: &WatchEntry, hit: &SearchHit, quote: &str, date: NaiveDate) -> Option<Proof>` — host is one of the entry's domains (or a subdomain) or `businesswire.com`, `prnewswire.com`, `globenewswire.com` (wire quotes must also say results/earnings/conference call); `verify_quote(hit.content, quote)`; the normalised quote names the company (or its ticker in the `(NYSE: X)` / `(NASDAQ: X)` form: bare tickers like PM, NOW, T are ordinary words) and contains D in a recognised date form ("October 30, 2026", "Oct. 30, 2026", "Sept. 30, 2026", "10/30/2026"), not glued to a preceding digit.
- `async fn refresh_tick(now, es, src: &dyn CalendarSource, ns: &dyn NewsSearch, llm: &dyn LlmProvider) -> Result<RefreshOutcome, String>` — once per ET day at ≥ 06:00 (kv `calendar_refreshed_on`): fetch the next 10 trading days, reconcile each watchlist ticker; then IR-check at most **5** `unconfirmed` rows dated within 7 days and not checked in the last 20 h (kv `ir_checked:{ticker}:{date}`); newly quarantined rows add one `warn` alert; one `warn` alert when every aggregator day fails. Yahoo pages are read until a page is shorter than the first (or under 20 rows), with cumulative offsets.

- [ ] **Step 1: Failing tests:** each reconcile rule above; Yahoo fixture → seeds for watchlist tickers only with slots mapped; 429 → error; `proof_from_hit` rejects a non-company host, a paraphrased quote and a quote with another date; refresh runs once per day.
- [ ] **Step 2–4:** FAIL → implement → PASS. **Step 5: Commit** `feat(core): earnings calendar with primary-source confirmation`.

---

### Task 12: Web pages

**Files:** Create `apps/web/src/lib/earnings.ts`, `src/app/charts/[...key]/route.ts`, `src/app/[locale]/earnings/page.tsx`, `src/app/[locale]/t/[ticker]/page.tsx`, `src/components/Charts.tsx`, `src/components/CalendarWeek.tsx`, `tests/earnings.test.ts`; modify `tests/d1.ts`, layout nav, post page, home page, `messages/*.json`, `wrangler.jsonc`, `cloudflare-env.d.ts`, `globals.css`.

**Interfaces:**
- `weekStart(date: string): string` (Monday, YYYY-MM-DD); `listCalendarWeek(db, monday): Promise<CalendarDay[]>` — 5 weekdays, entries with `status ∈ {unconfirmed, confirmed, reported}` (quarantined hidden), each `{ ticker, slot, status, postSlug | null }`.
- `getPostCharts(db, slug): Promise<{ name, src, altEn, altZh }[]>` (`src = /charts/<r2 key without "charts/">`).
- `listTickerPosts(db, ticker, lang)`, `listTickerCalendar(db, ticker)`, `listLatestEarnings(db, lang, n)`.
- `GET /charts/[...key]`: only keys matching `earnings/[A-Z0-9-]+/[0-9-]+/(revenue|eps).svg`; served from R2 `FACTS` at `charts/<key>` with `Content-Type: image/svg+xml`, `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'`, `X-Content-Type-Options: nosniff`, long cache; anything else 404.
- `/[locale]/earnings?week=YYYY-MM-DD`: week view, previous/next week links, confirmed vs unconfirmed vs reported marked, times as "Before open / After close / Time n/a".
- `/[locale]/t/[ticker]`: earnings posts for the ticker + its calendar rows; unknown ticker (not in a filing or calendar row) → 404.

- [ ] **Step 1: Failing tests:** `weekStart('2026-10-29') == '2026-10-26'`; week view hides quarantined and links a reported entry to its post; `getPostCharts` maps keys; ticker archive returns only that ticker's earnings posts in `lang` (ZH pending excluded from lists).
- [ ] **Step 2–4:** FAIL → implement → `pnpm --filter web test`, `typecheck`, `lint` PASS.
- [ ] **Step 5: Commit** `feat(web): earnings calendar, ticker archive and charts`.

---

### Task 13: Whole-branch review and verification

- [ ] Review the branch diff against this plan and the spec; fix findings.
- [ ] All must pass: `cargo test -p prebell-core`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo build -p prebell-core --target wasm32-unknown-unknown`; `worker-build --release` (in `crates/generator`); `pnpm --filter web test`, `typecheck`, `lint`; `cargo fmt --all --check`.
- [ ] Push `feat/plan-3-earnings`; PR into `feat/plan-1`.

## Parallelization

- Wave 1: Task 1. Wave 2: Tasks 2, 3, 12-scaffold. Wave 3: Tasks 4, 5, 8 (need 2, 3). Wave 4: Tasks 6, 9 (need 5, 8). Wave 5: Task 7 → Task 11 → Task 10 → Task 12 verification → Task 13.
