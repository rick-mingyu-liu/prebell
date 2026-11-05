# Themes and Macro Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Both briefs name the industry groups moving together (Themes) and cover the day's market-wide story (three macro queries, five items, summary leads with it), inside the Free plan's subrequest budget.

**Architecture:** The Yahoo quote stage shrinks from 11 to 4 requests (150 symbols per batch) to pay for the new work. The universe file gains sector and industry-group columns; a pure `themes` module clusters the liquid quotes by group; `build_facts` gains a budget guard and a three-query macro step; the drafts reorder sections (macro, themes, movers) and render a code-written theme list the model may not alter. The web front page reads `themes` from the facts it already loads.

**Tech Stack:** Rust (crates/core, crates/generator on workers-rs), Next.js 16 (apps/web), Vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-11-05-themes-and-macro-design.md` (read its Amendments).

**Branch:** `feat/themes-macro` from main. Commit per task. Do not merge, deploy or push to main.

## Global Constraints

- Free plan: 50 fetch subrequests per invocation. Target after this plan: premarket ≤ 43 normal / ≤ 46 on the Yahoo 401-refresh path; recap the same. `BUDGET_SOFT = 40` at facts time; the guard is `used + cost <= BUDGET_SOFT`; sacrifice order: theme search, third macro query, second macro query.
- Every figure the model writes must already be in `numbers_in_facts`; aggregates (median moves) are computed in code and added there. Negative numbers use U+2212 in rendered text, as the existing renderers do.
- Constants (exact): `QUOTE_BATCH = 150`, `MAX_QUOTE_BYTES = 2 MiB`; `THEME_MIN_MOVE` premarket 2.0 / recap 3.0; `THEME_MIN_MEDIAN` premarket 2.5 / recap 3.5; `THEME_MIN_MEMBERS = 3`; `THEME_MAX = 3`; `THEME_LIST_MEMBERS = 6`; `THEME_STAT_MOVE = 1.5`, `THEME_STATS_MAX = 15`; `MACRO_ITEMS = 5`, `MACRO_HITS_TOTAL = 8`, `MACRO_HITS` per query stays 5.
- Macro queries (exact): premarket `["stock market futures today", "Wall Street markets news today", "Treasury yields oil dollar Federal Reserve today"]`; recap `["stock market today closing", "Wall Street markets news today", "Treasury yields oil dollar Federal Reserve today"]`.
- Headings (exact). Premarket EN: "Futures", "Macro & overnight", "Themes", "Biggest premarket movers", "What to watch"; ZH: "期货", "宏观与隔夜", "板块动向", "盘前异动", "今日关注". Recap EN: "Index closes", "Macro & after the close", "Themes", "Notable movers", "After-hours earnings to watch"; ZH: "指数收盘", "宏观与盘后", "板块动向", "个股异动", "盘后财报关注". Macro and Themes sections exist only when they have items/themes.
- Universe file stays ≤ 550 tickers; sectors are exactly the 11 GICS sectors; every ticker has a group; no group has fewer than 2 members; 25–60 groups.
- Tests: `cargo test -p prebell-core --locked` (477 now), `cargo check -p prebell-generator --target wasm32-unknown-unknown --locked`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`; web `cd apps/web && pnpm test && pnpm typecheck && pnpm lint` (263+ now).
- Commit trailers: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_01C33suxkNuxRbEfR6iUqCky`.

## Review Focus

1. A theme must never form from mixed directions or from fewer than 3 qualifying members; pinned in Task 4.
2. The budget guard must drop optional work in the stated order and never let the facts stage exceed `BUDGET_SOFT`; pinned in Task 5 and Task 7 (`theme_search_is_skipped_when_budget_is_tight`).
3. The rendered theme line's figures must pass the recap's rendered-body number check (table_values, blank_names, counts > 10); pinned in Task 8 (`recap_with_themes_passes_rendered_number_check`).
4. Old stored facts without `themes`/`theme_candidates`/`macro_items` (recap) must still deserialize; pinned in Task 7 (`facts_without_theme_fields_deserialize`).
5. A 150-symbol Yahoo batch must not be silently truncated or rejected; pinned by Task 1's live check and its unit test on chunking.

---

### Task 1: Shrink the quote stage (live check first)

**Files:** Modify `crates/core/src/yahoo.rs` (`QUOTE_BATCH`, `MAX_QUOTE_BYTES`, chunk tests), `docs/runbook.md` (subrequest formula), `scripts/yahoo-live-check.sh` (add a `--batch N` option if absent).

**Interfaces:** Produces nothing new; `v7_quotes` behaviour unchanged except batch size.

- [ ] **Step 1: Live check.** From this Mac, run the existing live check with 150 symbols from `crates/core/data/universe.txt` (first 150) against the v7 endpoint with cookie and crumb, twice, and record: HTTP status, response bytes, elapsed ms, and the count of quotes returned. Record the numbers in the task report. Expected: 200, under 2 MiB, 150 quotes. If Yahoo returns fewer quotes than requested or 4xx, STOP and report BLOCKED with the evidence (the controller decides the fallback).
- [ ] **Step 2: Failing test** `quote_batches_cover_522_symbols_in_4_requests` in yahoo.rs: 522 symbols → 4 chunks (150,150,150,72).
- [ ] **Step 3:** Set the constants; run the test; run the full core suite.
- [ ] **Step 4:** Runbook: the premarket/recap subrequest formula line becomes "cookie 1, crumb 1, 4 v7 batches, 4 futures (premarket) / 4 index charts (recap), ≤10 mover searches + ≤10 extractions, 3 macro searches + 1 extraction, ≤1 theme search + 1 extraction, 2–4 drafts, 1 cover scene" with the totals from Global Constraints.
- [ ] **Step 5: Commit** `perf(core): 150-symbol Yahoo quote batches`

---

### Task 2: Universe refresh

**Files:** Modify `crates/core/data/universe.txt`; `crates/core/src/premarket/universe.rs` tests.

- [ ] **Step 1:** Determine S&P 500 and Nasdaq-100 membership changes since 2026-06 from model knowledge and the SEC/press announcements you can cite (list each addition and removal with the effective date and the source in the commit body). SNDK must be present. Keep the union ≤ 550.
- [ ] **Step 2:** Failing tests: `universe_contains_recent_additions` (SNDK and two more named additions), `universe_is_at_most_550`.
- [ ] **Step 3:** Edit the file; update the header line to the new as-of date; run tests.
- [ ] **Step 4: Commit** `data: refresh the mover universe to 2026-11 membership`

---

### Task 3: Industry map

**Files:** Modify `crates/core/data/universe.txt` (columns 3–4), `crates/core/src/premarket/universe.rs`; Create `crates/core/src/premarket/groups_zh.rs` (group → Chinese label).

**Interfaces:** Produces `pub fn group(ticker: &str) -> Option<&'static str>`, `pub fn sector(ticker: &str) -> Option<&'static str>`, `pub fn group_zh(group: &str) -> &'static str` (falls back to the English label), `pub fn groups() -> impl Iterator<Item = &'static str>`; all backed by a `OnceLock<HashMap<&'static str, Entry>>` built once; `company_name` moves onto the same map.

- [ ] **Step 1:** Failing tests: `every_ticker_has_sector_and_group`, `sectors_are_the_gics_eleven` (exact set), `group_count_in_range` (25..=60), `no_singleton_groups`, `group_zh_has_a_label_for_every_group`, `lookups_use_the_map_once` (call `group` 10_000 times under 50 ms in debug is enough to show no per-call parse).
- [ ] **Step 2:** Fill the two columns for every ticker (model knowledge + a review pass; merge singletons into the nearest group); write `groups_zh.rs`.
- [ ] **Step 3:** Parser accepts 2, 3 or 4 columns; implement the map; run tests.
- [ ] **Step 4: Commit** `feat(core): sector and industry group for every universe ticker`

---

### Task 4: Theme detection

**Files:** Create `crates/core/src/themes.rs`; Modify `crates/core/src/lib.rs`, `crates/core/src/facts.rs` (types).

**Interfaces:**
```rust
// facts.rs
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ThemeMember { pub ticker: String, pub name: String, pub change_pct: f64 }
pub struct Theme { pub group: String, pub group_zh: String, pub direction: Direction /* Up | Down */, pub members: Vec<ThemeMember>, pub more: usize, pub median_pct: f64, pub reasons: Vec<NewsReason> }
pub struct ThemeStat { pub group: String, pub present: usize, pub up: usize, pub down: usize, pub median_up: Option<f64>, pub median_down: Option<f64> }
// themes.rs
pub struct ThemeRules { pub min_move: f64, pub min_median: f64 }
pub const PREMARKET_RULES: ThemeRules; pub const RECAP_RULES: ThemeRules;
pub fn detect(quotes: &[Quote], rules: &ThemeRules) -> (Vec<Theme>, Vec<ThemeStat>)
```
`detect` groups by `universe::group`, applies the spec's rules, ranks by `median × members`, keeps `THEME_MAX`, lists `THEME_LIST_MEMBERS` members by |move| with `more` for the rest, fills `group_zh`, leaves `reasons` empty; stats for groups with ≥ 2 members beyond ±`THEME_STAT_MOVE`, capped at `THEME_STATS_MAX` by present count. Medians round to 2 decimals.

- [ ] **Step 1:** Failing tests: `three_members_same_direction_form_a_theme`, `mixed_directions_do_not`, `two_members_do_not`, `half_rule_rejects_a_group_mostly_flat`, `themes_rank_by_median_times_members_and_cap_at_three`, `members_listed_by_move_with_more_count`, `stats_cover_groups_with_two_strong_movers_capped_at_fifteen`, `median_is_rounded`.
- [ ] **Step 2:** Implement; clippy; commit `feat(core): theme detection from grouped quotes`

---

### Task 5: Budget guard

**Files:** Modify `crates/core/src/run.rs` (`Hooks.budget: &'a dyn Fn() -> usize`, `Content::build_facts(&self, date, now, budget: &dyn Fn() -> usize)`), premarket and recap impls, `crates/generator/src/lib.rs` (wire `|| self.http.count()`), all test call sites (`&|| 0`).

**Interfaces:** `pub const BUDGET_SOFT: usize = 40;` and `pub fn allows(used: usize, cost: usize) -> bool` in `crates/core/src/budget.rs`.

- [ ] Failing tests: `allows_within_soft_budget`, `build_facts_receives_the_budget` (a fake `Content` records the value), generator compiles for wasm. Implement; commit `feat(core): subrequest budget guard for optional work`

---

### Task 6: Macro that covers the day

**Files:** Modify `crates/core/src/premarket/mod.rs` (`macro_items` with queries, dedupe, cap, budget), `crates/core/src/recap/mod.rs` (new `macro_items` call and `RecapFacts.macro_items: Field<Vec<NewsReason>>` with `#[serde(default = missing)]`), `Diag` (`macro_queries`, `macro_hits`, `macro_items_kept`), prompts `crates/core/prompts/{premarket,recap}_v1_{en,zh}.md` (summary rule only here; section lists in Task 8).

**Interfaces:** `pub const PREMARKET_MACRO_QUERIES: [&str; 3]`, `pub const RECAP_MACRO_QUERIES: [&str; 3]`; `async fn macro_items(ns, llm, queries: &[&str], now, d, budget) -> Field<Vec<NewsReason>>`.

- [ ] Failing tests: `macro_merges_three_queries_dedupes_urls_and_caps_hits` (fake NewsSearch returns overlapping URLs → ≤ 8 hits, one extraction call), `macro_skips_later_queries_when_budget_is_tight` (budget returns 39 → only query 1), `macro_keeps_up_to_five_items`, recap `recap_facts_have_macro_items`, diagnostics line `macro queries=3 hits=8 items=5` placed after the `search_err=` line (update the index-pinned tests listed in the inventory: premarket/mod.rs ~1474–1558, run.rs ~952–1586, recap/mod.rs ~772–1549).
- [ ] Implement; commit `feat(core): three macro queries and five items in both briefs`

---

### Task 7: Themes in the build

**Files:** Modify `crates/core/src/premarket/mod.rs` and `recap/mod.rs` (`build_facts`: detect themes from liquid quotes, attach reasons from top movers, top-theme search under budget), `facts.rs` (`PremarketFacts.themes`, `theme_candidates` with `#[serde(default)]`; `RecapFacts` same; `numbers_in_facts` adds member moves and medians), `Diag` (`themes_found`, `themes_kept`, `theme_searched`).

**Interfaces:** `async fn theme_reasons(ns, llm, theme: &mut Theme, movers: &[Mover], now, d, budget)`; search query `format!("{} stocks {} today", group, if up {"rally"} else {"fall"})`.

- [ ] Failing tests: `themes_reuse_reasons_of_top_movers`, `top_theme_without_mover_reason_searches_once`, `theme_search_is_skipped_when_budget_is_tight`, `only_the_top_theme_searches`, `facts_without_theme_fields_deserialize`, `numbers_in_facts_include_theme_moves_and_medians`, diagnostics `themes found=N kept=K searched=0|1` after the macro line (update pinned tests again).
- [ ] Implement; commit `feat(core): themes with reused reasons and one guarded search`

---

### Task 8: Drafts and rendering

**Files:** Modify `crates/core/src/premarket/draft.rs`, `crates/core/src/recap/draft.rs`, the four prompt files, `crates/core/src/run.rs` test `draft_reply`.

**Interfaces:** `pub fn render_themes(themes: &[Theme], lang: Lang) -> String` (one line per theme: `**{group}** ▲/▼ median {±x.xx%}: T1 +a.aa%, T2 −b.bb%, …, and N more` / ZH `**{group_zh}** ▲ 中位数 +x.xx%：…，另有 N 只`); `all_headings` and `headings()` carry the new order and the conditional "Themes"; `render_markdown` order: title, summary, futures/index table, Macro section, Themes list then Themes section, movers table then movers section, remaining sections, sources (now including theme reasons), disclaimer. Recap: theme line values added to `table_values`; member names to `blank_names`; counts above 10 in "and N more" allowed by adding `more` to the allowed numbers. Prompts: themes paragraphs rule ("one short paragraph per theme in the listed order; cite only the listed moves; a theme without a reason gets one sentence stating the move") and the summary rule from the spec.

- [ ] Failing tests: `headings_put_macro_then_themes_then_movers`, `themes_section_exists_only_with_themes`, `render_themes_en_and_zh`, `render_markdown_orders_macro_themes_movers`, `recap_with_themes_passes_rendered_number_check`, `prompts_state_the_summary_rule`, updated `draft_reply` and the heading-pinning tests from the inventory (premarket/draft.rs 641–1327, run.rs 768+, recap/draft.rs 332, recap/mod.rs 1425); regenerate `apps/web/tests/fixtures/premarket-2026-10-30.{en,zh}.md` only if `trim.test.ts` needs the new headings (it strips tables and is heading-agnostic; leave if green).
- [ ] Implement; commit `feat(core): macro, themes, movers order and the themes list`

---

### Task 9: Front-page themes block (web)

**Files:** Modify `apps/web/src/lib/facts.ts` (`ThemeView { group, direction, medianPct, members: {symbol, changePct}[], more }`, `FactsView.themes`), `apps/web/src/components/HomeView.tsx` (a "Themes" `.section-title` rule under the movers column with one line per theme: group, ▲/▼ with `.num`, median via `fmtPct`, first three tickers, linking to the lead post), messages (`home.themes` "Themes"/"板块动向", `home.themesMore` "+{n}"), globals.css (ruled rows, tokens only), tests (`facts.test.ts` parse with and without `themes`; `home.test.tsx` renders the block and omits it when empty).

- [ ] Failing tests → implement → `pnpm test && pnpm typecheck && pnpm lint` → commit `feat(web): themes block on the front page`

---

### Task 10: Docs, rehearsal, verification

**Files:** `docs/runbook.md` (new diagnostics lines, tuning section for theme thresholds reading `theme_candidates`, the budget guard), `docs/spec.md` (brief sections), `docs/decisions.md` (section order follows the story; approved 2026-11-05).

- [ ] Full checks (Global Constraints). Local rehearsal: `crates/generator` `wrangler dev --test-scheduled --local` with `.dev.vars` copied from the main checkout and `FORCE_NOW_UTC` set to a weekday 21:46Z (recap); trigger one tick; confirm the recap publishes with the macro and themes sections (or states why no theme formed), the diagnostics lines, and `subrequests=` ≤ 40 at facts time; restore `.dev.vars`. Save the rendered EN/ZH markdown to the plan workspace and quote the themes list and macro section in the report.
- [ ] Commit `docs: themes and macro runbook, spec and decisions`

## Self-review notes

- Spec coverage: quote stage (T1), universe refresh (T2), industry map (T3), detection rules and stats (T4), budget guard (T5), macro queries/items/recap section/summary rule (T6, T8), theme reasons and search (T7), section order and rendering and number checks (T8), front page (T9), docs/tuning/rehearsal (T10).
- Interfaces: `Theme`/`ThemeStat` (T4) consumed by T7–T9; `budget` (T5) by T6–T7; headings (T8) by web fixtures.
- Proportion: decisions and tests only; bodies left to implementers.
