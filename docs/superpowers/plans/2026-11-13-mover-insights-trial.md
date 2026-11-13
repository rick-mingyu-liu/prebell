# Mover Insights, Plan 1: Core Logic and the Offline Trial

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the insight logic (evidence pack, writer, code checks, judge) in `prebell-core` and a local tool that runs it over the last two weeks of stored briefs, so Rick can read real lines and the judge can be chosen by measurement.

**Architecture:** A new pure module `crates/core/src/insight/` holds everything the Worker will later reuse; nothing in the Worker calls it yet. A standalone native crate `tools/insight-trial/` (outside the workspace, like `tools/llm-eval` on branch `eval/claude-vs-deepseek`) reads saved fact bundles, calls the production providers through a recording HTTP layer with a hard spend cap, and writes a results file, a labelling page for Rick and a judge score report.

**Tech Stack:** Rust (`prebell-core`, no new dependencies in core), a native tool with `reqwest` + `tokio`, one static HTML page with inline script, `wrangler` for read-only fetches of stored facts.

**Spec:** `docs/superpowers/specs/2026-11-13-mover-insights-design.md`. Read it first; this plan argues from it.

This is Plan 1 of 3. Plan 2 (the Worker's `insight` step in shadow mode) and Plan 3 (showing insights on the site) are written after the trial, because the prompts, the refusal list and the choice of judge come out of it.

## Global Constraints

- No Worker behaviour changes in this plan: `crates/generator` is not edited, and `prebell-core` must still pass `cargo check -p prebell-generator --target wasm32-unknown-unknown --locked`.
- No new dependency in `crates/core/Cargo.toml`. The tool has its own `Cargo.toml` and `Cargo.lock` and declares an empty `[workspace]`.
- House rule: the model never supplies a number, a date or a fact. Every number in a line must pass `numbers::check` against the pack; every date must be a date in the pack.
- The writer is always Claude (`claude-sonnet-5-5`, effort `low`). Judges compared: Claude (`claude-sonnet-5-5`) and DeepSeek (the provider default in `crates/core/src/deepseek.rs`).
- Limits, verbatim from the spec: hook at most 8 English words / 14 Chinese characters, no digits; watch line one sentence, at most 40 English words / 80 Chinese characters.
- Labels on the page later: "Watch next" / "后续关注" (not used in this plan, but do not name anything "what to watch" in user-facing strings).
- A line rejected in either language is dropped in both.
- Keys come only from `--dev-vars PATH` or the environment, stay in memory, and are never printed, logged or written. Default spend cap `--budget 3.0` (USD) across runs, enforced before each call.
- No live Yahoo requests. No remote D1 or R2 writes. Read-only remote reads are allowed.
- Rust builds use `CARGO_TARGET_DIR=~/.cache/prebell-target` for the workspace; before trusting a test run, `touch crates/core/src/lib.rs` and confirm the `Compiling prebell-core (...)` line names this worktree. The tool builds into its own `tools/insight-trial/target` (git-ignored).
- Commits end with the two trailer lines the lead gives; no TODO placeholders, skipped tests or stubs.

## Review Focus

1. **One number, two meanings.** A pack holds "$4.2 billion" as revenue and as a buyback; a line that swaps them passes the number check. Expected: the judge's support step fails it (the quoted support does not say what the line says). Pinned by the seeded set in Task 6 and a judge-parse test in Task 4.
2. **Invisible characters in quotes.** Stored quotes contain zero-width spaces (a live Reuters quote reads `on \u{200b}the Nasdaq`). Expected: support confirmation and outlet matching ignore them. Test in Task 4.
3. **Writer goes off-list.** The reply names a ticker that has no pack, repeats one, or omits one. Expected: unknown and repeated entries are discarded with a reason, omitted movers simply have no insight, nothing panics. Test in Task 3.
4. **Chinese dates and numbers.** "11月19日", "2026年11月19日", full-width digits and Chinese numerals (七十). Expected: dates are recognised and checked against the pack; a line using Chinese numerals for a figure is refused. Tests in Task 2.
5. **Outlet names.** "The Wall Street Journal" vs "WSJ"; "Reuters" vs "路透社". Expected: attribution passes with any accepted name for that outlet in the line's language, and fails when the outlet is missing or a different outlet is named. Tests in Task 2.

---

### Task 1: Types and the evidence pack

**Files:**
- Create: `crates/core/src/insight/mod.rs`, `crates/core/src/insight/pack.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod insight;`)
- Test: inline `#[cfg(test)]` in `pack.rs`

**Interfaces:**
- Produces (in `insight/mod.rs`):

```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind { Move, News, AnalystAction, CompanyFigure, CompanyGuidance, Theme, ReportDate, MacroEvent }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Evidence {
    pub id: String,                 // "e1", "e2", … unique within a pack
    pub kind: EvidenceKind,
    pub text: String,               // what the model reads; verbatim source text for News/AnalystAction/CompanyGuidance, code-written for CompanyFigure
    pub outlet: Option<String>,     // English outlet name, News and AnalystAction only
    pub url: Option<String>,
    pub date: Option<chrono::NaiveDate>,
    pub quote: Option<String>,      // the verbatim source when `text` is code-written (CompanyFigure); serialised only when present
    pub unconfirmed: bool,          // a ReportDate the company has not confirmed; serialised only when true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pack { pub ticker: String, pub name: String, pub evidence: Vec<Evidence> }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Line { pub hook: String, pub watch: String, pub cites: Vec<String> }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Insight { pub ticker: String, pub en: Line, pub zh: Line }
```

- Produces (in `insight/pack.rs`):

```rust
pub struct PackInputs<'a> {
    pub session: crate::cover::Session,
    pub trading_date: chrono::NaiveDate,
    pub as_of_utc: chrono::DateTime<chrono::Utc>,     // when the post's facts were built
    pub movers: &'a [crate::facts::Mover],            // gainers then losers, as listed
    pub themes: &'a [crate::facts::Theme],
    pub checks: &'a [crate::earnings::facts::EarningsFacts], // PreBell's published checks, any ticker
    pub calendar: &'a [(String, chrono::NaiveDate, ReportStatus)],  // (ticker, report date, status) rows near trading_date
    pub macro_events: &'a [(chrono::NaiveDate, String)], // (date, event name), next seven days
}
/// Packs for the eligible movers only, in mover order.
pub fn packs(i: &PackInputs) -> Vec<Pack>;
/// The outlet's English name: from `ipo::news::OUTLETS` when the host is listed, else the host without "www.".
pub fn outlet_name(url: &str) -> Option<String>;
/// True when a news reason is an analyst rating action (upgrade, downgrade, initiation, price target).
pub fn is_analyst_action(r: &crate::facts::NewsReason) -> bool;
/// Whether a guidance quote itself names the period it applies to.
pub fn names_period(quote: &str) -> bool;
pub enum ReportStatus { Reported, Confirmed, Unconfirmed } // earnings_calendar.status; quarantined rows are never read
```

Decisions the implementer cannot make alone:
- Evidence order per pack: Move, then each news reason (News or AnalystAction; `text` is the reason's `quote`, `outlet` from `outlet_name`), then for a matching earnings check (same ticker, `filing_date` within 3 calendar days before or on `trading_date`, and `accepted_at` before the premarket post's `as_of_utc`, or before the regular-session close for a recap): one `CompanyFigure` per verified headline metric, its `text` written by code from the verified fields ("Revenue: $18.52 billion, reported for Q1 of fiscal 2027.": metric label, `fmt_value` of the value and unit, the period from `fiscal_period`, the word "reported") and the verbatim `quote` kept in `Evidence.quote`; and one `CompanyGuidance` per guidance `Note` whose verbatim `quote` names its period (`names_period`: a quarter, a half, a fiscal or calendar year, "full year"), text = `quote` (`summary_en` is model-written and never used; a note without a period is left out); then Theme (when the mover is a member of a theme), ReportDate (calendar rows for the ticker within 100 days: "<name> reported results on <date>", "<name>'s report date is confirmed for <date>", or for an unconfirmed row "Expected report date, not confirmed by the company: <date>" with `unconfirmed` set), MacroEvent (each event in the next seven days). An identical text is stored once per outlet (two outlets carrying it are two items).
- Eligible means: at least one `News`, `CompanyFigure` or `CompanyGuidance` item. `AnalystAction`, `Theme`, `ReportDate`, `MacroEvent` and `Move` alone do not make a mover eligible.
- `is_analyst_action` (revised after review): reads the reason's quote and summary, not its headline. Unambiguous rating wording counts alone (`price target`, `price objective`, `initiated/started coverage`, `to buy from hold`, `overweight`, `equal-weight`, `sector weight`, `buy rating` and the like); words with other meanings (`upgrade`, `downgrade`, `outperform`, `target`, `upside`, `rating`) count only beside a named broker or "analyst", and a broker that is the mover itself does not count.
- Strip the invisible characters `ipo::news::visible` strips from every `text` before storing it (widen that function to `pub(crate)` if it is private; do not copy its character list).

- [ ] **Step 1: Write the failing tests** in `pack.rs`: `eligible_with_a_news_reason`, `not_eligible_with_only_a_theme_fallback`, `not_eligible_with_only_an_analyst_action`, `a_mover_with_news_and_a_report_gets_its_check_figures_and_guidance`, `a_check_older_than_three_days_is_left_out`, `outlet_name_uses_the_list_then_the_host` (asserts `reuters.com` → `"Reuters"`, `https://www.marketscreener.com/x` → `"marketscreener.com"`), `analyst_actions_are_recognised` (an upgrade title is, an earnings title is not), `evidence_ids_are_unique_and_ordered`, `invisible_characters_are_stripped` (input `"on \u{200b}the Nasdaq"` → `"on the Nasdaq"`).
- [ ] **Step 2: Run** `cargo test -p prebell-core --locked insight::pack` and confirm they fail to compile or fail.
- [ ] **Step 3: Implement** the types and `packs`, `outlet_name`, `is_analyst_action`. Build test fixtures with the existing test helpers for `Mover` and `EarningsFacts` where the crate has them (search `fn mover(` and `fn facts(` in tests).
- [ ] **Step 4: Run** the same command; expect all pass. Run `cargo check -p prebell-generator --target wasm32-unknown-unknown --locked`; expect success.
- [ ] **Step 5: Commit** "Insight: evidence pack and eligibility".

### Task 2: Code checks

**Files:**
- Create: `crates/core/src/insight/check.rs`
- Test: inline in `check.rs`

**Interfaces:**
- Consumes: `Pack`, `Evidence`, `EvidenceKind`, `Insight`, `Line` (Task 1); `crate::numbers::check(text, lang, allowed)`; `crate::Lang`.
- Produces:

```rust
pub const HOOK_WORDS_EN: usize = 8;
pub const HOOK_CHARS_ZH: usize = 14;
pub const WATCH_WORDS_EN: usize = 40;
pub const WATCH_CHARS_ZH: usize = 80;
/// Empty when the insight may go to the judge; else one reason per failure, each starting "en:" or "zh:".
pub fn check(pack: &Pack, i: &Insight) -> Vec<String>;
/// Accepted names for an outlet in a language ("Reuters" → zh ["路透社", "路透", "Reuters"]).
pub fn outlet_names(outlet: &str, lang: crate::Lang) -> Vec<String>;
/// Calendar dates written in `text` ("Nov 19", "November 19, 2026", "11月19日", "2026年11月19日", "2026-11-19"), year taken from `year_of` when absent.
pub fn dates_in(text: &str, lang: crate::Lang, year_of: chrono::NaiveDate) -> Vec<chrono::NaiveDate>;
```

Rules `check` applies to each language's line (hook and watch):
1. Lengths per the constants; hook has no digit (ASCII or full-width) and no Chinese numeral used as a figure; watch is one sentence (at most one sentence-ending mark, at the end); no Markdown, URL or line break; the zh line contains at least one CJK character and the en line none.
2. `cites` is not empty, every id exists in this pack, and at least one cited item is not `AnalystAction`.
3. Numbers: `numbers::check(hook + " " + watch, lang, allowed)` where `allowed` is every number `numbers::extract` finds in the cited items' `text`. Chinese numerals for a figure (regex `[一二三四五六七八九十百千万亿两]+(美元|亿|万|%|％|个百分点)`) are refused.
4. Dates: every date `dates_in` finds must equal the `date` of some evidence item in the pack or a date `dates_in` finds in a cited item's `text`.
5. Relative time is refused: en `next (week|month|quarter|year)`, `tomorrow`, `this (week|month)`, `soon`, `in the coming (days|weeks|months)`; zh `下周`, `下个月`, `明天`, `本周`, `近期`, `未来几(天|周|个月)`.
6. Fiscal labels (`Q[1-4]`, `fiscal 20\d\d`, `FY\d\d`, `第[一二三四]季度`, `\d{4}财年`) must appear in a cited item's `text` (compare case-insensitively; a zh label is accepted when its English equivalent is in the cited text: 第三季度 ↔ Q3 / third quarter, 2027财年 ↔ fiscal 2027).
7. Attribution: for every cited `News` or `AnalystAction` item, one of `outlet_names(outlet, lang)` is in the watch line.
8. Refusal list, whole words, case-insensitive. en: `buy`, `sell`, `hold`, `should`, `must`, `recommend`, `will rise`, `will fall`, `will rally`, `will drop`, `set to surge`, `price target`, `target price`, `overweight`, `underweight`, `outperform`, `underperform`, `analysts expect`, `undervalued`, `overvalued`, `bullish`, `bearish`. zh: `买入`, `卖出`, `持有`, `建议`, `应当`, `应该`, `目标价`, `看涨`, `看跌`, `看好`, `看空`, `增持`, `减持`, `跑赢`, `跑输`, `低估`, `高估`, `分析师预计`, `将上涨`, `将下跌`.

- [ ] **Step 1: Write the failing tests**, one per rule plus the Review Focus cases: `a_clean_insight_passes`; `lengths_are_enforced_in_both_languages`; `a_hook_with_a_digit_is_refused`; `two_sentences_are_refused`; `a_number_not_in_the_cited_items_is_refused` (line says "$5.1 billion", cited text says "$4.2 billion"); `chinese_numerals_for_a_figure_are_refused` ("七十美元"); `full_width_digits_are_checked` ("７０美元" with 70 allowed passes, "７１美元" fails); `chinese_and_english_dates_are_recognised` (asserts `dates_in("将于11月19日", Zh, 2026-11-12) == [2026-11-19]` and the four English/ISO shapes); `a_date_not_in_the_pack_is_refused`; `relative_time_is_refused_in_both_languages`; `a_fiscal_label_must_be_in_the_cited_text` (zh "第三季度" accepted against "third quarter"); `news_must_name_its_outlet` (passes with "Reuters reports", with "据路透社报道", with "WSJ" for The Wall Street Journal; fails with no outlet and with a different outlet); `citing_only_an_analyst_action_is_refused`; `refusal_words_are_refused_even_when_the_evidence_uses_them`; `unknown_cite_ids_are_refused`; `failures_say_which_language`.
- [ ] **Step 2: Run** `cargo test -p prebell-core --locked insight::check`; expect failure.
- [ ] **Step 3: Implement** `check`, `outlet_names` (a table for the 25 outlets in `ipo::news::OUTLETS` with their common Chinese names and short forms; an outlet outside the table is accepted by its English name or host in both languages) and `dates_in`.
- [ ] **Step 4: Run** the tests; expect pass. Re-run the wasm check.
- [ ] **Step 5: Commit** "Insight: code checks".

### Task 3: Writer

**Files:**
- Create: `crates/core/src/insight/write.rs`
- Test: inline in `write.rs`

**Interfaces:**
- Consumes: `Pack`, `Insight`, `Line`; `crate::providers::{LlmProvider, ProviderError}`.
- Produces:

```rust
pub const WRITER_VERSION: &str = "insight-writer/v1";
pub fn prompt(packs: &[Pack]) -> (String, String);             // (system, user)
/// One entry per usable reply item; items for unknown or repeated tickers, or with a missing field, become Err(reason).
pub fn parse(reply: &serde_json::Value, packs: &[Pack]) -> Vec<Result<Insight, String>>;
pub async fn write(llm: &dyn LlmProvider, packs: &[Pack]) -> Result<Vec<Result<Insight, String>>, ProviderError>;
```

Decisions:
- One call for all packs. The user message is JSON: `{"movers":[{"ticker","name","evidence":[{"id","kind","text","outlet","date"}]}]}`. Reply shape: `{"insights":[{"ticker","en":{"hook","watch","cites":[]},"zh":{"hook","watch","cites":[]}}]}`. The writer may omit a mover when the evidence gives it nothing concrete to track.
- The system prompt states, in this order: the task (a hook and one sentence on what to track next and why it matters, PreBell's own reading, formal register); use only the evidence, never memory; every number and date must appear in a cited item, copied in the same form; a fact from a `news` or `analyst_action` item is attributed to its `outlet` by name in the sentence, a `company_*` item is stated as the company's; an analyst action may be mentioned but never its rating word or price target, and the thing to track must come from another item; no advice, no price prediction, no relative time words; the limits from Global Constraints; the Chinese line says the same thing as the English one; reply with JSON only.
- `write` calls `llm.complete_json(system, user)` once and does not retry; an empty `packs` returns `Ok(vec![])` without calling.

- [ ] **Step 1: Write the failing tests**: `the_prompt_carries_every_pack_and_no_urls` (the user JSON has each ticker and evidence id; URLs are not sent); `the_system_prompt_states_the_limits` (contains "8 words", "40 words", "14", "80"); `parse_keeps_good_items`; `parse_rejects_unknown_and_repeated_tickers` (Review Focus 3: reply with a ticker that has no pack, and one ticker twice → the unknown and the second are `Err`, the rest `Ok`); `parse_rejects_a_missing_language`; `an_omitted_mover_is_not_an_error`; `a_non_object_reply_is_one_error`; `no_packs_means_no_call` (a mock `LlmProvider` that panics if called).
- [ ] **Step 2: Run** `cargo test -p prebell-core --locked insight::write`; expect failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the tests and the wasm check; expect pass.
- [ ] **Step 5: Commit** "Insight: writer prompt and reply parsing".

### Task 4: Judge

**Files:**
- Create: `crates/core/src/insight/judge.rs`
- Test: inline in `judge.rs`

**Interfaces:**
- Consumes: `Pack`, `Insight`; `LlmProvider`.
- Produces:

```rust
pub const JUDGE_VERSION: &str = "insight-judge/v1";
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Claim { pub claim: String, pub evidence_id: String, pub support: String }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Verdict {
    pub ticker: String,
    pub en_claims: Vec<Claim>, pub zh_claims: Vec<Claim>,
    pub en_supported: bool, pub zh_supported: bool,
    pub no_advice: bool, pub formal: bool, pub same_meaning: bool,
    pub notes: String,
}
pub fn prompt(packs: &[Pack], insights: &[Insight]) -> (String, String);
pub fn parse(reply: &serde_json::Value, insights: &[Insight]) -> Vec<Verdict>;
/// True when every claim's `support` is found verbatim in the named evidence item of this pack.
pub fn support_confirmed(pack: &Pack, claims: &[Claim]) -> bool;
/// The final answer for one mover: every flag true, at least one claim per language, and support confirmed for both.
pub fn passed(pack: &Pack, insight: &Insight, v: &Verdict) -> bool; // also: every number and date of each line within that language's confirmed support
pub async fn judge(llm: &dyn LlmProvider, packs: &[Pack], insights: &[Insight]) -> Result<Vec<Verdict>, ProviderError>;
```

Decisions:
- The judge sees the same pack JSON as the writer plus the lines. It is told: you are checking, not rewriting; split each watch line and hook into its factual claims; for each claim give the evidence id and copy the exact words that support it; if no item supports a claim, set `*_supported` false; PreBell's framing of why something matters is allowed only when it follows from a supported claim; flag advice or price prediction; flag an informal or promotional tone; compare the Chinese line with the English one.
- `support_confirmed` compares after stripping invisible characters (`ipo::news::visible`), collapsing whitespace and lowercasing ASCII. A `support` shorter than 12 characters (6 for text that is mostly CJK) is not confirmed, so a judge cannot pass a claim on a single common word.
- A mover the judge leaves out of its reply has no `Verdict` and therefore does not pass.
- No retry. An empty `insights` returns `Ok(vec![])` without calling.

- [ ] **Step 1: Write the failing tests**: `parse_reads_a_full_verdict`; `a_mover_missing_from_the_reply_has_no_verdict`; `support_must_be_verbatim_in_the_named_item` (exact quote passes; a paraphrase fails; the right words under the wrong evidence id fail); `support_ignores_invisible_characters_and_spacing` (Review Focus 2: evidence text `"on \u{200b}the  Nasdaq on November 19"`, support `"on the Nasdaq on November 19"` → confirmed); `a_too_short_support_is_not_confirmed`; `passed_needs_every_flag_and_confirmed_support_in_both_languages`; `a_swapped_meaning_fails_on_support` (Review Focus 1: pack text "revenue of $4.2 billion … a $1.0 billion buyback", claim "a $4.2 billion buyback" with support "$4.2 billion" → too short → not confirmed; with support "a $4.2 billion buyback" → not verbatim → not confirmed); `no_insights_means_no_call`.
- [ ] **Step 2: Run** `cargo test -p prebell-core --locked insight::judge`; expect failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the whole core suite (`cargo test -p prebell-core --locked`), `cargo fmt --all -- --check` and the wasm check; expect pass.
- [ ] **Step 5: Commit** "Insight: judge prompt, verdict parsing and support confirmation".

### Task 5: The trial tool, run mode

**Files:**
- Create: `tools/insight-trial/Cargo.toml`, `tools/insight-trial/.gitignore` (`target/`, `out/`, `facts/`), `tools/insight-trial/src/main.rs`, `tools/insight-trial/src/http.rs`, `tools/insight-trial/fetch-facts.sh`, `tools/insight-trial/README.md`

**Interfaces:**
- Consumes: everything Tasks 1–4 produce; `prebell_core::llm::llm_from_env`; `prebell_core::facts::PremarketFacts`, `prebell_core::recap::RecapFacts`, `prebell_core::earnings::facts::EarningsFacts`.
- Produces: `out/lines.jsonl`, one JSON object per mover that had a pack:

```json
{"post":"recap_2026-11-12","ticker":"ANET","pack":{…},"insight":{…}|null,"write_error":null|"…",
 "check":[…],"judges":{"claude":{"verdict":{…}|null,"passed":true},"deepseek":{…}},"published_if":{"claude":true,"deepseek":false}}
```

  and `out/spend.jsonl` (one line per call: time, model, role `writer|judge`, tokens, USD).

Decisions:
- `http.rs` is the recording HTTP layer, spend cap, price table, ledger and key loading from `tools/llm-eval/src/main.rs` on branch `eval/claude-vs-deepseek` (`git show eval/claude-vs-deepseek:tools/llm-eval/src/main.rs`): copy `prices`, `deepseek_peak`, `Call`, `Budget`, `read_ledger`, `RecordingHttp`, `read_usage`, `price_call`, `load_keys`, unchanged apart from module visibility. Copy that crate's `Cargo.toml` dependencies.
- CLI: `cargo run --release -- run --facts DIR --out DIR --dev-vars PATH [--budget 3.0] [--only SUBSTR]`. `--facts` holds files named as `fetch-facts.sh` saves them: `premarket_<date>.json`, `recap_<date>.json`, `earnings_<TICKER>_<accession>.json`, plus `calendar.json` (`[[ticker, "YYYY-MM-DD", status], …]`, every status but quarantined) and `events.json` (`[["YYYY-MM-DD", name], …]`).
- For each brief or recap bundle: build `PackInputs` (every `earnings_*` bundle as `checks`), `packs`, one writer call, `check` each insight, then one judge call per judge provider over the insights that passed `check`, `passed` per mover. A post with no eligible mover makes no call.
- `fetch-facts.sh` (read-only): lists `facts_r2_key` for posts with `content_type IN ('premarket','recap')` and `trading_date >= date('now','-14 day')`, and for `content_type='earnings'` in the last 21 days, with `wrangler d1 execute prebell --remote --json`, saves each with `wrangler r2 object get prebell-facts/<key> --remote --file`, and writes `calendar.json` and `events.json` from the calendar and macro-events tables (read their schemas in `migrations/0003_earnings.sql` and `0008_macro_events.sql`). It runs from `crates/generator` so wrangler finds its config.
- The budget is checked before every call; when it would be passed the tool stops, keeps what it has written and exits non-zero with a message naming the spend so far.
- Nothing printed or written ever includes a request header or a key.

- [ ] **Step 1: Write the failing tests** in `main.rs`: `a_post_with_no_eligible_mover_makes_no_call` and `the_budget_stops_the_run_and_keeps_results` (mock HTTP that counts calls; a cap of 0.0 stops before the first call and `lines.jsonl` exists), `lines_record_every_stage` (mock replies for writer and one judge → one line with insight, empty check, verdict and `passed`), `a_check_failure_never_reaches_the_judge`.
- [ ] **Step 2: Run** `cd tools/insight-trial && cargo test`; expect failure.
- [ ] **Step 3: Implement** the crate, the script and a README with the exact commands.
- [ ] **Step 4: Run** `cargo test` and `cargo build --release` in the tool directory; expect pass. Confirm the workspace is untouched: `git status` shows no change to the root `Cargo.lock`.
- [ ] **Step 5: Commit** "insight-trial: run writer and judges over saved facts".

### Task 6: Seeded wrong lines, the labelling page and the score

**Files:**
- Create: `tools/insight-trial/src/seed.rs`, `tools/insight-trial/src/page.rs`, `tools/insight-trial/src/score.rs`
- Modify: `tools/insight-trial/src/main.rs` (subcommands `seed`, `page`, `score`), `tools/insight-trial/README.md`

**Interfaces:**
- Consumes: `out/lines.jsonl` (Task 5).
- Produces: `out/seeded.jsonl` (wrong lines with each judge's verdict), `out/label.html`, and from `score`: `out/report.md`.

Decisions:
- `seed`: from each insight that passed `check`, make up to five wrong variants, each still passing `check` where possible, tagged with its kind: `number_swap` (two numbers in the English line exchanged, or one replaced by another number in the same cited item), `direction_flip` (raised↔cut, above↔below, increase↔decrease, 上调↔下调, 高于↔低于, 增长↔下降), `outlet_swap` (the named outlet replaced by another outlet present in the pack, with the cite kept), `zh_only_number` (a number changed in the Chinese line only), `formal_advice` (one of five fixed formal sentences that avoid every refusal word appended in both languages, for example "Investors may wish to add exposure ahead of that date." / "投资者不妨在此之前提高配置。"). Variants that `check` already refuses are counted separately and not sent to a judge. Then one judge call per provider per post over the variants.
- `page`: one self-contained `label.html` (inline CSS and script, no network) listing about 60 lines: every real line where a judge said no or the two judges disagreed, then judge-passed lines chosen at random with a fixed seed until 60. Order shuffled, verdicts hidden. Each card shows the mover, the English and Chinese hook and line, and the cited evidence texts with their outlets. Three choices per card: "Fine", "Wrong fact", "Advice or not formal". A "Save labels" button downloads `labels.json` (`[{"post","ticker","label"}]`). Choices persist in `localStorage` wrapped in try/catch so a reload keeps them.
- `score --labels FILE`: for each judge, agreement with Rick (judge pass ↔ "Fine"), the lines a judge passed that Rick marked wrong (listed in full), seeded recall overall and by kind, how many lines `check` refused and why (top reasons), how many movers were eligible out of all movers, and total spend from `spend.jsonl`. It ends by naming the judge with higher agreement, ties broken by seeded recall, and states whether the spec's bar is met (no judge-passed line marked wrong; seeded recall at least 0.9).

- [ ] **Step 1: Write the failing tests**: `each_seed_kind_changes_the_line_it_claims_to` (five small fixtures); `a_seed_with_no_number_to_swap_is_skipped`; `the_page_hides_verdicts_and_lists_at_most_sixty` (the HTML contains no `"passed"` and no judge name; 60 cards from 200 lines; all disagreements included); `the_page_is_self_contained` (no `http://` or `https://` other than inside evidence text, no `<script src`); `score_reports_agreement_recall_and_the_bar` (a fixture with 10 labels and 10 seeded lines gives the expected numbers and names the expected judge); `score_lists_judge_passed_lines_marked_wrong`.
- [ ] **Step 2: Run** `cargo test` in the tool directory; expect failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test`; expect pass.
- [ ] **Step 5: Commit** "insight-trial: seeded wrong lines, labelling page and judge score".

### Task 7: Run the trial

This task spends money (approved by Rick, 2026-11-13) and is run by the lead or with the lead's go-ahead, not as part of an unattended build.

- [ ] **Step 1:** `tools/insight-trial/fetch-facts.sh tools/insight-trial/facts` and confirm it saved the briefs, recaps and earnings bundles and wrote `calendar.json` and `events.json`.
- [ ] **Step 2:** `cargo run --release -- run --facts facts --out out --dev-vars <path to the repo's env file> --budget 3.0`, then `seed`, then `page`. Report the spend from `out/spend.jsonl`, how many movers were eligible, how many lines survived `check`, and each judge's pass count.
- [ ] **Step 3:** Read twenty lines yourself before handing over. If the writer's lines are systematically refused by one rule (for example attribution), fix the prompt, bump `WRITER_VERSION`, and re-run within the budget; record what changed.
- [ ] **Step 4:** Give Rick `out/label.html`. When `labels.json` comes back, run `score` and bring him `out/report.md` with a recommendation on the judge and on whether to write Plan 2.

## Changes after the review of PR #67 (2026-11-13, the lead's rulings)

The review came back "fix first" (4 high, 9 medium, 10 low findings). The interfaces and rules above are updated where these changed them:

- **Company figures are written by code** from the verified value, unit and fiscal period; the verbatim quote is kept in `Evidence.quote` and never allows a number or confirms support. A table row's quote ("Total revenue 18,517 14,883") holds other periods' cells.
- **Guidance needs its period in its own quote** (`names_period`); `Note` has no period field and `summary_en` is model-written. The earnings module's stored schema is unchanged.
- **A check counts only when it was out before the post**: accepted before a premarket post's `as_of_utc`, or before the close a recap describes.
- **Unconfirmed report dates are included, marked**: `fetch-facts.sh` keeps every status but quarantined, `calendar.json` rows are `[ticker, date, status]`, and a line that cites (or writes) an unconfirmed date must hedge it with `expected`, `scheduled` or `unconfirmed` (en) or `预计`, `预期`, `暂定`, `未确认` (zh) (check rule 9).
- **Code checks, widened**: numbers never come from an analyst action; M/D dates, bare days, weekdays, Chinese-numeral dates, years with 年 and bare months are checked or refused; figures in words (ten percent, 百分之, 三成, 倍) are refused; fiscal labels include 4Q, H1/H2, "fourth-quarter", 四季度, 上/下半年; more relative-time words in both languages; target, rating, consensus and prediction words are refused in both languages, hyphenated forms included, with only named compounds (sell-off, buy-side, sell-side, buy-back) exempt; "hold", 持有, Fortune and 市场观察 are refused only where they are advice or attribution; semicolons are refused and abbreviations ("St.", "Inc.") are not sentence ends.
- **The judge's pass** also needs every number and date of each line inside that language's confirmed support spans, and a generic span ("the company said") confirms nothing.
- **Seeding and scoring**: one row per mover per judge call, the original line sent as a control in another round, a wrong line counted against a judge only when that judge passed its original, failed calls and omitted movers left out and counted, the false-reject rate on controls reported, `outlet_swap` trading two cited outlets so it reaches the judges, and the bar also needs 50 judge-passed lines read. `seed` prints its planned calls and expected cost and refuses to start past the cap.
- **Re-runs**: `run` skips posts already complete unless `--fresh` and finishes a part-done post without paying for its writer again; labels and seeds made for older text are left out of the score.
- **The page** shows the whole pack with the cited items marked, and keys saved choices by the line's text and writer version. `report.md` has a "Limits of this trial" section (today's calendar and macro rows are used for past posts).

## Changes after the second review (2026-11-13, the lead's rulings)

- **Refusals target advice senses, not bare words**: "buy" and "sell" are refused as ratings, imperatives and advice to investors and allowed in deals ("agreed to sell"); "target" is refused next to a price, the stock or "$", or beside a cited analyst action, and allowed in plans ("targets a launch", "margin target"); "hold" as before. Consensus wording is refused in both languages (analysts' or Street estimates, beat or missed estimates, market estimates, consensus, 分析师预期, 市场预期, 一致预期). `check::refused_wording` is public and pinned against every stored mover reason of 2026-11-04 to 2026-11-12 (`insight/testdata/reasons_2026_11.tsv`).
- **An unconfirmed date is hedged in its own clause**, in both languages.
- **Evidence text**: Markdown links keep only their text and bare URLs are dropped, so no URL reaches a model; the move names its trading date.
- **Seeds must be wrong**: a replaced number differs from the original by at least 5% at the line's own precision, in both languages; a range's two ends are never swapped with each other; a flip is skipped when the line already holds both words of the pair. A kind that could only make a true line is recorded as skipped and counted, never sent.
- **The $3 cap covers `run` and `seed` together**: `seed` plans from the real `lines.jsonl` and, when the expected cost passes what is left, seeds a fixed-seed random sample of whole posts that fits, saying which and how many wrong lines and controls that sends (`--plan` prints the same). The per-call cap is unchanged.
- **Scoring**: a control counts as a flip only when that judge passed the line in `run`; consistent rejections are reported apart. The report gives the number of judge-passed lines labelled and calls the bar "not yet assessable" below 50 (the offline trial has 37 eligible movers; the rest come from the shadow stage), with the trial's own part of the bar on its own line.
- **Re-runs** retry any judge call that failed, not only one the budget stopped. The tool pins its build directory to `tools/insight-trial/target` in its own `.cargo/config.toml`.

