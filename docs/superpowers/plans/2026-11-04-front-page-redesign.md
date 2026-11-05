# Front-Page Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn PreBell into a daily paper: light paper palette, serif headlines, a composed cover per story with an AI-generated scene, an author block, a weekly-digest signup, and a front-page home, without changing what the generator writes.

**Architecture:** The generator gains a cover step at publish (two verified figure lines per language into D1, one optional Workers AI scene into R2, an index-line snapshot into `kv`). The web app gets new data fields through its existing D1 queries, a covers route like the charts route, a subscribe route calling Buttondown, and a restyled component tree on a rewritten `globals.css`. Tasks 1–4 are Rust and ship first so the web can be tested against real columns; Tasks 5–11 are the web.

**Tech Stack:** Rust (workers-rs 0.8.7, `base64` 0.22), Next.js 16.4 App Router on OpenNext for Cloudflare, `next-intl`, plain CSS with custom properties, Vitest 5 with `better-sqlite3` in-memory D1, `@fontsource-variable/newsreader`.

**Spec:** `docs/superpowers/specs/2026-11-04-front-page-redesign-design.md` (read the Amendments section too).

**Branch:** `feat/front-page` (already holds the spec at bc38faf). Commit per task. Do not merge, deploy or push to main.

## Global Constraints

- Free tiers only: no new paid services; Workers AI stays inside the 10,000 neurons/day allocation; Buttondown free plan.
- Workers Free CPU: the web Worker adds no per-request work beyond one `kv` read and the extra columns; covers are static R2 objects served by a route with `Cache-Control: public, max-age=86400, s-maxage=86400`.
- The generator never waits on or fails for a scene: one attempt, errors swallowed, `cover_key` null on failure.
- No derived numbers on the site: cover lines carry only verified figures, no percentages for earnings.
- Palette tokens exactly: paper `#faf8f3`, ink `#1a1a1a`, muted `#666`, rule `#d9d5cc`, section rule `#222`, up `#15803d`, down `#b91c1c`, kicker `#8a1c1c`, cover bar `#e8a317`. No other colours. No dark mode. No pills, tiles, filled chips, shadows or gradients except the cover shade.
- Fonts: Newsreader (variable, latin) for masthead, h1–h3, section titles and story headlines; Inter for everything else; Chinese headlines use `"Songti SC", "Noto Serif CJK SC", "Source Han Serif SC", serif`. No CJK web font. No Google Fonts request.
- Negative numbers use U+2212 "−" in web and cover text.
- Author: name "Rick Liu"; bio EN "AI & software engineer, University of Waterloo. Built PreBell to read earnings the way an analyst would, without the analyst."; links `https://rickmingyuliu.com`, `https://github.com/rick-mingyu-liu`, `https://www.linkedin.com/in/rick-mingyu-liu/`; photo `/author.jpg`, greyscale via CSS; if the file is absent the block renders without an image.
- Signup copy EN: heading "Get the weekly digest"; line "One email a week: the week's briefs and earnings checks. No spam, unsubscribe anytime."; button "Subscribe"; success "Check your inbox to confirm."; error "Something went wrong. Please try again."; manage link text "Already subscribed? Manage preferences" → `https://buttondown.com/portal`.
- ZH copy for new strings is written by the implementer in the same register as `zh.json`; the task reviewer checks it reads naturally.
- Tests: core `cargo test -p prebell-core --locked` from the repo root (438 now); web `cd apps/web && pnpm test` (238 now) and `pnpm typecheck`; generator `cargo check -p prebell-generator --target wasm32-unknown-unknown --locked`; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` clean.
- `apps/web/AGENTS.md`: Next 16.4 differs from older Next; read `node_modules/next/dist/docs/` before writing a page or route.
- Commit messages end with the two attribution trailers used on this branch (see `git log -1 --format=%B bc38faf`).

## Review Focus

1. A ZH retry or a pending-ZH write must not null the EN or ZH cover lines: `upsert_translation` overwrites the whole row. Pinned in Task 3 (`retry_zh_keeps_cover_lines`, earnings `zh_retry_keeps_cover_lines`).
2. A post with `cover_key` null and lines null (pre-launch post) must render a tag-only flat cover everywhere, not crash or show "null". Pinned in Task 7 (`Cover` tests) and Task 10 (archive row).
3. The subscribe route must never echo Buttondown's error text or the API key, and must treat "already subscribed" (HTTP 400 with that code) as success. Pinned in Task 7.
4. The covers route must reject keys outside `covers/<slug>.(png|jpg)` and never read arbitrary R2 keys. Pinned in Task 5.
5. The index line must omit itself when `kv` has no `index_line`, and must label futures as futures. Pinned in Task 5 (`loadIndexLine`) and Task 6 (`IndexLine`).

---

### Task 1: Schema and store rows for covers and the index snapshot

**Files:**
- Create: `migrations/0006_covers.sql`
- Modify: `crates/core/src/store.rs` (PostRow, TranslationRow, `Store` trait, MemStore)
- Modify: `crates/core/src/run.rs:1086` (FailStore forwards the new methods)
- Modify: `crates/generator/src/store.rs` (PostRec, TrRec, upsert_post, upsert_translation, get_post_by_run, posts_on, get_translation, new `put_bytes`, `kv_set`)
- Modify: every `PostRow { … }` / `TranslationRow { … }` literal listed in the inventory (core run.rs 377, 389, 461, 756; recap/mod.rs 962, 1477, 1758, 1788; earnings/run.rs 369, 614, 629, 642, 733, 1449; store.rs 148; generator store.rs 58, 252) to add the new fields
- Modify: `apps/web/tests/*.test.ts(x)` seeds that use positional `INSERT INTO post_translations VALUES (…)` (pages, briefpage, homeload, earnings, posts and shared helpers) → column lists
- Test: `crates/core/src/store.rs` tests, `apps/web/tests/d1.ts` (no change) via `pnpm test`

**Interfaces:**
- Produces: `PostRow.cover_key: Option<String>`; `TranslationRow.cover_line1: Option<String>`, `cover_line2: Option<String>`;
  `Store::put_bytes(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), String>`;
  `Store::kv_set(&self, key: &str, value: &str, now: &str) -> Result<(), String>`;
  `MemStore.bytes: RefCell<BTreeMap<String, (Vec<u8>, String)>>`, `MemStore.kv: RefCell<BTreeMap<String, String>>`.
- D1 columns: `posts.cover_key TEXT`, `post_translations.cover_line1 TEXT`, `post_translations.cover_line2 TEXT`.

- [ ] **Step 1: Write the migration**

```sql
-- 0006_covers.sql
ALTER TABLE posts ADD COLUMN cover_key TEXT;
ALTER TABLE post_translations ADD COLUMN cover_line1 TEXT;
ALTER TABLE post_translations ADD COLUMN cover_line2 TEXT;
```

- [ ] **Step 2: Run the web tests to see the positional seeds fail**

Run: `cd apps/web && pnpm test`
Expected: failures with "table post_translations has 10 columns but 7 values were supplied".

- [ ] **Step 3: Convert every positional `INSERT INTO post_translations` seed to a column list** (`(post_id, lang, status, title, summary, body_md, updated_at)`), then run `pnpm test` → 238 pass.

- [ ] **Step 4: Write the failing core tests** in `crates/core/src/store.rs`:

```rust
#[tokio::test] async fn mem_store_round_trips_cover_fields() {
    // upsert_post with cover_key Some("covers/x.png") then get_post_by_run → same;
    // upsert_translation with cover_line1 Some("Revenue $10.4B"), cover_line2 None then get_translation → same
}
#[tokio::test] async fn mem_store_put_bytes_and_kv_set() {
    // put_bytes("covers/x.png", &[1,2,3], "image/png") → bytes map holds (vec![1,2,3], "image/png");
    // kv_set("index_line", "{}", "2026-11-04T00:00:00Z") → kv["index_line"] == "{}"
}
```

- [ ] **Step 5: Run them to verify they fail** (`cargo test -p prebell-core mem_store_` → compile errors on the missing fields/methods).

- [ ] **Step 6: Add the fields and methods**: `PostRow.cover_key`, `TranslationRow.cover_line1/2` (both `Option<String>`, `#[serde(default)]` if the structs derive Deserialize), the two trait methods, MemStore + FailStore impls, and fill every literal with `None`. In `crates/generator/src/store.rs`: add the columns to the INSERT list, placeholders, `ON CONFLICT … SET cover_key = excluded.cover_key` (and the two lines), the bind arrays via `opt()`, the `PostRec`/`TrRec` decoders and the three SELECT lists; `put_bytes` = `self.facts.put(key, bytes.to_vec()).http_metadata(HttpMetadata { content_type: Some(content_type.into()), ..Default::default() }).execute().await`; `kv_set` reuses the `EarningsStore` body (call a shared private fn).

- [ ] **Step 7: Run all checks**

Run: `cargo test -p prebell-core --locked && cargo check -p prebell-generator --target wasm32-unknown-unknown --locked && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: 440 core tests pass; check and clippy clean.

- [ ] **Step 8: Commit** `feat: cover columns, index snapshot kv and binary put in the store`

---

### Task 2: Cover composition in core (lines, index snapshot, scene prompt, maker trait)

**Files:**
- Create: `crates/core/src/cover.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod cover;`)
- Modify: `crates/core/src/run.rs` (`Content` trait gains `cover_lines` and `index_snapshot`)
- Modify: `crates/core/src/premarket/mod.rs`, `crates/core/src/recap/mod.rs` (implement them)
- Modify: `crates/core/src/earnings/draft.rs` (make `fmt_value` and `period_label` `pub`; add nothing else)
- Test: `crates/core/src/cover.rs` tests, premarket/recap tests

**Interfaces:**
- Produces, in `cover.rs`:
  ```rust
  pub struct CoverLines { pub en: [String; 2], pub zh: [String; 2] }
  #[async_trait(?Send)]
  pub trait CoverMaker {
      /// One attempt. Ok(bytes) is a PNG or JPEG; Err is logged and ignored.
      async fn scene(&self, prompt: &str) -> Result<Vec<u8>, String>;
  }
  pub struct NoCover;            // scene() → Err("cover maker not configured")
  pub fn fmt_short_usd(value: f64) -> String;        // 10_400_000_000.0 → "$10.4B"; 850_000_000.0 → "$850M"; 2_300_000.0 → "$2.3M"; 7.08 (per share) handled by fmt_eps
  pub fn fmt_eps(value: f64) -> String;              // 7.08 → "$7.08"; -0.5 → "−$0.50"
  pub fn fmt_signed_pct(p: f64) -> String;           // 3.03 → "+3.03%"; -23.67 → "−23.67%" (U+2212)
  pub fn earnings_lines(f: &EarningsFacts) -> CoverLines;
  pub fn brief_lines(indexes: &[(String, Field<Quote>)], gainers: &[Mover], losers: &[Mover], futures: bool) -> CoverLines;
  pub fn image_ext(bytes: &[u8]) -> Option<&'static str>;  // "png" for 89 50 4E 47, "jpg" for FF D8 FF, else None
  pub fn scene_prompt(subject: &str) -> String;      // the Flux prompt with the fixed style suffix and negative cues
  #[derive(Serialize, Deserialize)] pub struct IndexSnapshot { pub kind: String, pub as_of_utc: String, pub entries: Vec<IndexEntry> }
  #[derive(Serialize, Deserialize)] pub struct IndexEntry { pub symbol: String, pub last: f64, pub change_pct: f64 }
  pub fn index_snapshot(kind: &str, as_of: DateTime<Utc>, quotes: &[(String, Field<Quote>)]) -> IndexSnapshot;  // Missing quotes skipped
  pub const INDEX_LINE_KEY: &str = "index_line";
  ```
- `Content` trait additions (run.rs):
  ```rust
  fn cover_lines(&self, f: &Self::Facts) -> CoverLines;
  fn index_snapshot(&self, f: &Self::Facts) -> IndexSnapshot;
  fn scene_subject(&self, f: &Self::Facts) -> String;   // premarket/recap: weekday-rotated scene family
  ```
- Earnings: `pub fn earnings_scene_subject(f: &EarningsFacts) -> String` in `cover.rs` ("the industry of <company>" descriptor built from the company name; builder's wording).

Line rules (from the spec): earnings line 1 `Revenue $10.4B` (ZH `营收 104亿美元` using `fmt_value(.., Lang::Zh)`); line 2 `GAAP EPS $7.08` or `Non-GAAP EPS $1.85` when GAAP is missing, or the period label (`Q3 FY2026 results` / `2026财年第三季度`) when both EPS are missing. Briefs: line 1 = the entry with the largest `|change_pct|` among present quotes, labelled by `index_label(symbol, futures)` (`^DJI`→"Dow", `^GSPC`→"S&P 500", `^NDX`/`^IXIC`→"Nasdaq", `^RUT`→"Russell 2000"; futures: `ES=F`→"S&P futures", `NQ=F`→"Nasdaq futures", `YM=F`→"Dow futures", `RTY=F`→"Russell futures") + space + `fmt_signed_pct`; line 2 = `"{top_gainer_symbol} {pct}, {top_loser_symbol} {pct}"` (one side omitted when that list is empty; empty string when both are). ZH labels: 道指 / 标普500 / 纳指 / 罗素2000, futures add 期货.

- [ ] **Step 1: Write the failing tests** in `cover.rs`:

```rust
#[test] fn short_usd_formats_billions_and_millions()   // asserts the four examples above plus 1_000_000_000.0 → "$1B", 999_000_000.0 → "$999M"
#[test] fn signed_pct_uses_unicode_minus()
#[test] fn eps_formats_negative_with_unicode_minus()
#[test] fn image_ext_detects_png_jpg_and_rejects_other()
#[test] fn earnings_lines_prefer_gaap_then_non_gaap_then_period()  // three EarningsFacts fixtures
#[test] fn brief_lines_pick_largest_index_move_and_top_movers()    // recap fixture: Dow +3.03 largest → "Dow +3.03%", "MOS +17.37%, FIS −23.67%"
#[test] fn brief_lines_label_futures()                              // premarket fixture → "S&P futures −1.14%"
#[test] fn index_snapshot_skips_missing_quotes()
#[test] fn scene_prompt_contains_no_people_no_text_cues()
```

- [ ] **Step 2: Run to verify they fail** (`cargo test -p prebell-core cover::`).
- [ ] **Step 3: Implement `cover.rs`** and the `Content` methods in premarket (`futures: true`) and recap (`false`); `scene_subject` for briefs rotates `["a trading floor seen from above", "a stock exchange building exterior at dawn", "a financial district skyline at dusk", "a newsroom desk with printed pages", "a harbour of container ships"]` by `trading_date.weekday()`.
- [ ] **Step 4: Run tests** → pass; clippy, fmt clean.
- [ ] **Step 5: Commit** `feat(core): cover lines, index snapshot and scene prompt`

---

### Task 3: Covers in the publish paths

**Files:**
- Modify: `crates/core/src/run.rs` (`Hooks` gains `cover: &'a dyn CoverMaker`; `build`, `translation_zh`, `retry_zh`, `reconcile`)
- Modify: `crates/core/src/earnings/run.rs` (`build_tick` and `build` gain `cover: &dyn CoverMaker`; `empty_translation`, `retry_zh`, `zh_failed` carry lines)
- Modify: `crates/core/src/recap/mod.rs` and premarket `tick_with_hooks` call sites and tests that construct `Hooks`
- Test: run.rs, recap/mod.rs, earnings/run.rs tests

**Interfaces:**
- `pub struct Hooks<'a> { pub log, pub extra, pub cover: &'a dyn CoverMaker }`
- `pub async fn build_tick(now, es, store, filings, llm, cover: &dyn CoverMaker) -> …`
- Behaviour in brief `build` (run.rs ~L346–416), in order: facts built → `lines = c.diagnostics(&facts)`; before `hooks.log`, run the scene step and push `format!("cover={}", outcome)` where outcome ∈ `ok|skip|err` (`skip` when `scene()` returns `Err` containing "not configured", `err` otherwise); scene step = `hooks.cover.scene(&scene_prompt(&c.scene_subject(&facts)))` → on Ok bytes with a known ext, `store.put_bytes(&format!("covers/{slug}.{ext}"), &bytes, mime)`; `cover_key` = that key on success else None → `PostRow.cover_key`; `lines = c.cover_lines(&facts)` → EN row gets `lines.en`, published ZH row gets `lines.zh`, pending ZH row gets `lines.zh` too; after `upsert_post`, `store.kv_set(INDEX_LINE_KEY, &serde_json::to_string(&c.index_snapshot(&facts))?, &ts)`. A `put_bytes` or `kv_set` error is logged through `hooks.log` and does not fail the build.
- `retry_zh` and `reconcile` must pass the stored ZH lines through: read the existing ZH translation row and copy its `cover_line1/2` into the new row (they were written at build), or recompute from the facts they already load (`C::Facts` is available in `retry_zh`); pick the recompute, it is one call.
- Earnings `build` (earnings/run.rs ~L596–662): same scene step after `put_facts` and before `upsert_post`, subject `earnings_scene_subject(&facts)`; lines from `earnings_lines(&facts)`; `empty_translation` gains `lines: Option<[String;2]>`; `retry_zh` and `zh_failed` recompute from the facts JSON they re-read (`get_facts`) or carry from the existing row; no `kv_set`, no diagnostics line; failures go through `console`-free paths (the Worker logs).

- [ ] **Step 1: Write the failing tests**

run.rs (premarket, using the existing fake `Content` and `MemStore`, with a fake `CoverMaker` recording prompts):
```rust
#[tokio::test] async fn build_stores_cover_lines_scene_and_index_snapshot()
  // cover maker returns PNG magic bytes → posts.cover_key == "covers/premarket-2026-11-04.png", bytes map has it with "image/png",
  // en row lines == fake lines.en, zh row lines == lines.zh, kv["index_line"] parses to IndexSnapshot with kind "premarket",
  // logged diagnostics contain "cover=ok"
#[tokio::test] async fn build_publishes_without_scene_when_maker_fails()
  // maker Err("boom") → cover_key None, run published, line "cover=err"; NoCover → "cover=skip"
#[tokio::test] async fn retry_zh_keeps_cover_lines()
  // after build with pending ZH, retry_zh publishes ZH → zh row has lines.zh (not None)
```
earnings/run.rs:
```rust
#[tokio::test] async fn earnings_build_writes_cover_key_and_lines()
#[tokio::test] async fn earnings_zh_retry_keeps_cover_lines()
#[tokio::test] async fn earnings_build_publishes_when_scene_fails()
```
recap/mod.rs: `recap_build_writes_index_snapshot_with_closes` (kind "recap", four entries).
Existing tests that assert full diagnostics lines (run.rs:820, recap/mod.rs:770, 909, 1570) gain the `cover=skip` line where they use `NoCover`.

- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement** as described. Keep the scene step in one private fn in `cover.rs`: `pub async fn make_scene(store: &dyn Store, maker: &dyn CoverMaker, slug: &str, subject: &str) -> (Option<String>, &'static str)` returning `(cover_key, outcome)`, used by both brief and earnings builds.
- [ ] **Step 4: Run** core tests, clippy, fmt → pass (expect ~448).
- [ ] **Step 5: Commit** `feat(core): covers and the index snapshot at publish`

---

### Task 4: Workers AI cover client in the generator

**Files:**
- Create: `crates/generator/src/cover.rs`
- Modify: `crates/generator/Cargo.toml` (`base64 = "0.22"`)
- Modify: `crates/generator/src/lib.rs` (build the maker, pass it in `Hooks` and `build_tick`)
- Modify: `crates/generator/wrangler.toml` (`CF_ACCOUNT_ID` var `ed6569531d89c3e00705807340c7b75f`)
- Modify: `docs/runbook.md` (secrets list gains `CF_AI_TOKEN`, deploy note, `cover=` in the diagnostics section)
- Test: `cargo check` for wasm; a unit test for the response parser that does not touch the network

**Interfaces:**
- ```rust
  pub struct WorkersAiCover<'a, H: Http> { http: &'a H, account_id: String, token: String }
  impl<'a, H: Http> WorkersAiCover<'a, H> { pub fn new(http: &'a H, account_id: String, token: String) -> Self }
  #[async_trait(?Send)] impl<H: Http> CoverMaker for WorkersAiCover<'_, H> { async fn scene(&self, prompt: &str) -> Result<Vec<u8>, String> }
  pub fn parse_image(body: &str) -> Result<Vec<u8>, String>   // {"success":true,"result":{"image":"<b64>"}} → bytes; anything else → Err with the API's `errors[0].message` if present, never the token
  ```
- Request: `POST https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/run/@cf/black-forest-labs/flux-1-schnell`, headers `Authorization: Bearer {token}`, body `{"prompt": prompt, "steps": 4}`.
- In `Runner::llm`-style fashion: `fn cover(&self) -> Box<dyn CoverMaker + '_>` returns `WorkersAiCover` when `secret_or_empty(env, "CF_AI_TOKEN")` is non-empty and `text(env, "CF_ACCOUNT_ID")` is set, else `NoCover`. On `Err` from `scene`, `console_warn!("cover: {e}")` from the Worker side (wrap the maker in a logging adapter in lib.rs).

- [ ] **Step 1: Write the failing parser test** `parse_image_decodes_base64_and_reports_api_errors` (two fixtures: success body with the base64 of the 8 PNG magic bytes; failure body `{"success":false,"errors":[{"message":"quota"}]}` → Err containing "quota").
- [ ] **Step 2: Run** `cargo test -p prebell-generator parse_image` → fails to compile.
- [ ] **Step 3: Implement** `cover.rs` and the wiring; `Hooks { log, extra, cover: &maker }`; `build_tick(.., &maker)`.
- [ ] **Step 4: Run** `cargo test -p prebell-generator`, the wasm `cargo check`, clippy, fmt → clean.
- [ ] **Step 5: Update `docs/runbook.md`**: secrets (`CF_AI_TOKEN`: Cloudflare API token with Workers AI "Read" permission, set with `wrangler secret put` by the owner), var `CF_ACCOUNT_ID`, the `cover=ok|skip|err` line, and "covers live in R2 under `covers/<slug>.<ext>`".
- [ ] **Step 6: Commit** `feat(generator): Workers AI scene for covers`

---

### Task 5: Web data layer: cover fields, ticker, covers route, index line

**Files:**
- Modify: `apps/web/src/lib/posts.ts` (`PostView`, `PostHead`, `PostListItem` gain `coverKey: string | null`, `coverLines: [string, string] | null`, `ticker: string | null`; `HEAD_COLUMNS`, `FROM_VIEW`, `listPosts`, `getAdjacentPosts`, `toListItem`; every SELECT gains `p.cover_key, t.cover_line1, t.cover_line2` and `LEFT JOIN earnings_filings f ON f.run_key = p.run_key` with `f.ticker`)
- Modify: `apps/web/src/lib/earnings.ts` (`EARNINGS_LIST`, `listRecentBriefs` select the same fields; `ListRow` gains them)
- Create: `apps/web/src/lib/indexline.ts`
- Create: `apps/web/src/lib/covers.ts` and `apps/web/src/app/covers/[...key]/route.ts`
- Modify: `apps/web/cloudflare-env.d.ts` (`BUTTONDOWN_API_KEY?: string`)
- Test: `apps/web/tests/posts.test.ts`, `earnings.test.ts`, new `indexline.test.ts`, `covers.test.ts`

**Interfaces:**
- ```ts
  // lib/indexline.ts
  export interface IndexEntryView { symbol: string; last: number; changePct: number; dir: Dir }
  export interface IndexLineView { kind: "premarket" | "recap"; asOfUtc: string; entries: IndexEntryView[] }
  export function parseIndexLine(json: unknown): IndexLineView | null   // validates shape; null on anything off
  export async function loadIndexLine(db: D1Like): Promise<IndexLineView | null>  // SELECT value FROM kv WHERE key='index_line'
  // lib/covers.ts
  export const COVER_KEY = /^covers\/[a-z0-9-]{1,80}\.(png|jpg)$/;
  export async function coverResponse(bucket: R2Like, segments: string[]): Promise<Response>  // 404 "not found" on bad key or missing; Content-Type image/png|image/jpeg; Cache-Control public, max-age=86400, s-maxage=86400; X-Content-Type-Options nosniff
  ```
- `coverLines` is `[cover_line1, cover_line2]` when `cover_line1` is non-null (line 2 may be ""), else null.

- [ ] **Step 1: Write the failing tests**
  - posts.test.ts: `lists cover key, lines and ticker on heads, lists and neighbours` (seed one earnings post with `earnings_filings` row and cover columns, one brief without → `ticker` "AMGN"/null, `coverLines` tuple/null, `coverKey`).
  - earnings.test.ts: `latest earnings carry ticker and cover fields`.
  - indexline.test.ts: `parseIndexLine accepts the generator shape and rejects junk`; `loadIndexLine returns null without the kv row`.
  - covers.test.ts: `serves png and jpg with cache headers`, `rejects keys outside covers/`, `404 when missing`.
- [ ] **Step 2: Run** `pnpm test` → the new tests fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm test && pnpm typecheck` → pass.
- [ ] **Step 5: Commit** `feat(web): cover fields, covers route and the index line snapshot`

---

### Task 6: Web foundation: fonts, tokens, masthead, index line, footer

**Files:**
- Modify: `apps/web/package.json` (add `@fontsource-variable/newsreader`), `pnpm-lock.yaml`
- Modify: `apps/web/src/lib/fonts.ts` (export `newsreader` via `localFont`, variable `--font-serif`)
- Rewrite: `apps/web/src/app/globals.css` (tokens from Global Constraints; base; `.masthead`, `.dateline`, `.site-nav`, `.index-line`, `.rule`, `.rule-strong`, `.kicker`, `.num.up/.down/.flat`, `.cover*`, `.table`, `.story-*`, `.author*`, `.signup*`, `.partners-strip`, `.site-footer`; one `@media (max-width: 720px)` block; delete the dark block, `--*-pill`, `.pill`, `.tile*`, `.tlogo-none`, `.side-*`, `.tp-*`, `.estrip*`, `.tabs`, `.ptab`, `.panel*`, `.ecard*`, `.grid-portal`, `.grid-article`)
- Modify: `apps/web/src/app/[locale]/layout.tsx` (masthead + dateline + nav + `IndexLine`; footer: disclaimer, About, lang switch, no RSS), `global-not-found.tsx` (both font variables)
- Create: `apps/web/src/components/Masthead.tsx`, `IndexLine.tsx`
- Modify: `apps/web/messages/en.json`, `zh.json` (new namespace `paper`: `dateline.premarket` "Before the bell", `dateline.recap` "After the close", `dateline.earnings` "Earnings check", `index.asOf` "as of {time}", `index.futures` suffix handling; and `author.*`, `signup.*`, `partners.from` "From our partners", `home.*` keys used later; remove `termpocket` and `site.wordmark` rich key)
- Test: `apps/web/tests/layout.test.tsx` (new), `components.test.tsx` adjustments

**Interfaces:**
- `Masthead({ locale, dateline }: { locale: Locale; dateline: string })` server component: `<header className="masthead">` with `<a class="masthead-name">PreBell <small>盘前铃</small></a>` (ZH: 盘前铃 first), `<p class="dateline">{dateline}</p>`, `<nav class="site-nav">` (Briefs, Earnings, Calendar, About, LangSwitch).
- `IndexLine({ line, locale }: { line: IndexLineView | null; locale: Locale })`: renders nothing when `line` is null; otherwise `<p className="index-line">` with each entry `label level <span class="num up|down">▲1.58%</span>` using existing `index.*` messages for labels, `fmtPrice`, `fmtPct`, and the as-of ET time via `formatEtTime`.
- `layout.tsx` computes `dateline` from the date in ET (`formatDate(todayEt(), lang)` long form) and loads `loadIndexLine(getDb())` once per request.

- [ ] **Step 1: Write the failing tests**
  - layout.test.tsx: `renders masthead, dateline, nav and index line in order` (renderToStaticMarkup of `Masthead` + `IndexLine` with a fixture line), `omits the index line without a snapshot`, `labels futures entries as futures` (`ES=F` → message `index.es`).
  - A test that `globals.css` contains no `#00c805`, `#ff5000`, `prefers-color-scheme`, `.pill` (string assertions on the file).
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.** `pnpm add @fontsource-variable/newsreader` in apps/web. Write the CSS in sections with a one-line comment per section. Keep `.post-body` styles for markdown (headings in serif, tables ruled, `img` grayscale filter for `.charts img`).
- [ ] **Step 4: Run** `pnpm test && pnpm typecheck` → pass (expect deletions of tests that pinned removed CSS classes to come in Task 11; if a test breaks now because its subject no longer exists, update it in this task and say so in the report).
- [ ] **Step 5: Commit** `feat(web): paper palette, Newsreader, masthead and index line`

---

### Task 7: Cover, author block, signup and partner strip components

**Files:**
- Create: `apps/web/src/components/Cover.tsx`, `AuthorBlock.tsx`, `SignupForm.tsx` ("use client"), `PartnersStrip.tsx`
- Create: `apps/web/src/app/api/subscribe/route.ts`, `apps/web/src/lib/subscribe.ts`
- Modify: `apps/web/next.config.ts` (headers: `/api/subscribe` → `private, no-store`)
- Modify: `apps/web/src/components/PartnerLinks.tsx` (restyle to the strip; keep the client fetch of `/api/partners`)
- Modify: `apps/web/src/lib/partners.ts` (remove `TERMPOCKET_URL` use from components; keep the constant for About)
- Add: `apps/web/public/author.jpg` (download `https://github.com/rick-mingyu-liu.png` once, resize to 256px square, commit; the spec calls it a placeholder for a real photo)
- Test: `apps/web/tests/cover.test.tsx`, `author.test.tsx`, `subscribe.test.ts`, `partners.test.ts` (strip)

**Interfaces:**
- ```tsx
  export interface CoverProps { coverKey: string | null; lines: [string, string] | null; tag: { kind: "logo"; ticker: string } | { kind: "label"; text: string }; size: "lead" | "article" | "small"; alt: string }
  export default function Cover(p: CoverProps): JSX.Element
  // <figure class="cover cover-{size}"> [<img class="cover-bg" src="/{coverKey}" alt="" loading="lazy">] <div class="cover-shade"/> <div class="cover-in"> <span class="cover-tag">logo img or text</span> [<p class="cover-stat">line1<br/>line2</p><span class="cover-bar"/>] </div></figure>
  // no img when coverKey null; no stat/bar when lines null; logo tag falls back to the ticker text when companyLogo(ticker) is null
  export default function AuthorBlock({ locale }: { locale: Locale })   // server component; reads author.* messages; links from Global Constraints; <img src="/author.jpg"> only when `hasAuthorPhoto` (a build-time constant set by checking public/author.jpg with fs in lib/author.ts), else no image
  export default function SignupForm({ source, locale }: { source: "post" | "home" | "about"; locale: Locale })  // posts JSON {email, source, lang} to /api/subscribe; honeypot input name "website" hidden; states idle|loading|done|error
  export default function PartnersStrip({ locale })  // wraps PartnerLinks in <aside class="partners-strip">
  // lib/subscribe.ts
  export function validEmail(s: string): boolean
  export async function subscribe(apiKey: string, email: string, tags: string[], fetchImpl = fetch): Promise<"ok" | "already" | "error">
  // POST https://api.buttondown.com/v1/subscribers, Authorization: Token {apiKey}, body {email_address, tags, type: "unactivated"}; 201 → ok; 400 whose body mentions "already" → already; else error (log status only)
  // route.ts: POST → 400 on bad JSON/invalid email/filled honeypot; 503 when BUTTONDOWN_API_KEY is unset; 200 {ok:true} for ok|already; 502 {ok:false} for error. Never include upstream text.
  ```

- [ ] **Step 1: Write the failing tests**
  - cover.test.tsx: `renders scene, tag logo and two lines for an earnings cover`; `renders a flat tag-only cover without key or lines`; `falls back to the ticker text when no logo exists`.
  - author.test.tsx: `renders name, bio, three links and the archive link in both locales`; `public/author.jpg exists and is under 60 KB`.
  - subscribe.test.ts: `validEmail`; `subscribe maps 201 to ok, already-subscribed 400 to already, 500 to error` (fake fetch); `POST /api/subscribe rejects honeypot and bad email with 400`, `returns 503 without a key`, `never echoes upstream error text` (mock `getCloudflareContext`).
  - partners.test.ts: replace the Sidebar assertions with `PartnersStrip renders the heading and the disclosure, no partner hosts server-side`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm test && pnpm typecheck` → pass.
- [ ] **Step 5: Commit** `feat(web): cover, author block, weekly-digest signup and partner strip`

---

### Task 8: The front page

**Files:**
- Modify: `apps/web/src/lib/home.ts` (`HomeData` → `{ lead: PostHead | null; leadFacts: FactsView | null; earnings: PostListItem[]; recent: PostListItem[]; reporting: StripDay | null }`; `loadHome(db, bucket, lang, today, onError)` with no `requested` tab; lead = later-published of premarket/recap (premarket wins a tie); `reporting = stripDay(db, lead?.contentType === "premarket" ? today : addDays(today, 1), { exact: false, lang })`; `recent` = `listRecentBriefs(db, lang, 8)` minus the lead, plus `listLatestEarnings` beyond the three shown, sorted by publishedAt desc, sliced to 8)
- Rewrite: `apps/web/src/components/HomeView.tsx`
- Modify: `apps/web/src/app/[locale]/page.tsx` (drop `searchParams.tab`)
- Delete: `apps/web/src/components/EarningsStrip.tsx` only if no longer imported anywhere after Task 9; else keep until Task 11
- Test: `apps/web/tests/home.test.tsx`, `homeload.test.ts`

**Interfaces:**
- `HomeView({ data, locale }: { data: HomeData; locale: Locale })` renders `<div class="front">` with `<section class="front-lead">` (kicker `dateline.*`, `Cover size="lead"` with label tag, serif `<h1>` title, summary `<p>`, "Read the full brief" link), `<section class="front-earnings">` (rule-strong + "Earnings checks" title; up to three `Cover size="small"` with logo tags and serif headlines linking to the posts; `home.noEarnings` when empty), `<section class="front-recent">` (ruled rows: kicker, title, date), `<aside class="front-side">` with `MoversTable` (Task 9's component, compact variant, from `leadFacts`) and `<section class="front-reporting">` (heading "Reporting {date}", before-open and after-close lists of tickers linking to `/t/{ticker}`).
- `selectTab` is removed; `?tab=` is ignored.

- [ ] **Step 1: Write the failing tests**
  - homeload.test.ts: replace the tab tests with `leads with the later-published brief and reads only its facts`, `reporting comes from tomorrow for a recap lead and today for a premarket lead`, `recent excludes the lead and is capped at eight`.
  - home.test.tsx: `renders lead, earnings row, recent list, movers column and reporting block in order`, `renders "No clear news" quietly in the movers column`, `shows the empty earnings message`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement** (MoversTable may be created here and reused in Task 9: `MoversTable({ movers, locale, variant: "column" | "table", showPrice })`).
- [ ] **Step 4: Run** → pass.
- [ ] **Step 5: Commit** `feat(web): the front page`

---

### Task 9: The article page

**Files:**
- Rewrite: `apps/web/src/components/BriefView.tsx` (article layout; no `EarningsStrip`, no `Sidebar`)
- Modify: `apps/web/src/lib/brief.ts` (`BriefData` drops `strip`; keeps `facts`, `adjacent`, `charts`)
- Modify: `apps/web/src/components/NumbersBlock.tsx` → delete; `MoversList.tsx` → delete after `MoversTable` covers it; `Pill.tsx` → delete; `Charts.tsx` restyle (caption in muted, `img` class `chart`)
- Modify: `apps/web/src/components/PostBody.tsx` (no change to behaviour; ensure tables get `class="table"`)
- Test: `apps/web/tests/briefpage.test.tsx`, `pages.test.tsx`

**Interfaces:**
- `BriefView({ data, locale })` renders `<article class="story">`: kicker (`dateline.*` + date, or "Earnings check · {ticker}" for earnings), serif `<h1>`, `<p class="story-meta">` (published ET time; for briefs the `checked` tag `<span class="checked">✓ {brief.checked}</span>`; earnings: the same tag with `earnings.checkedFiling` text), `Cover size="article"` (label tag for briefs, logo tag for earnings), `<div class="story-body">` = for briefs: summary paragraph, `PostBody` of `trimStory(bodyMd)`, then `<h3>` movers title + `MoversTable variant="table" showPrice`, then the body's remaining sections; for earnings: `PostBody` untouched + `Charts`; then `Sources` list (existing markup restyled), `Disclaimer`, `AuthorBlock`, `SignupForm source="post"`, adjacency links for briefs (`prev`/`next`), `PartnersStrip`.
- ZH `MoversTable` omits the reason column.

- [ ] **Step 1: Write the failing tests** (replace the ones that pin the strip, grid and sidebar): `renders kicker, title, meta with the checked tag, cover, body, movers table, sources, author block, signup and prev/next in order`; `renders an earnings check with the logo cover, body table untouched, grey charts, no movers`; `ZH brief table has no reason column`; `loadBrief no longer loads the strip`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → pass.
- [ ] **Step 5: Commit** `feat(web): the article page`

---

### Task 10: Lists, calendar, ticker, about, 404

**Files:**
- Modify: `apps/web/src/components/PostList.tsx` (rows: `Cover size="small"` at 160px left, kicker, serif title, summary, date; no type tag)
- Modify: `apps/web/src/app/[locale]/archive/page.tsx`, `earnings/page.tsx`, `t/[ticker]/page.tsx` (drop `Sidebar`; keep filters/pager/week nav restyled as plain links; the calendar grid keeps its chips but restyled as ruled rows with ticker links and bmo/amc labels, no coloured badges except the `reported` tick)
- Modify: `apps/web/src/app/[locale]/about/page.tsx` (existing three paragraphs; "Who is behind it" rule + `AuthorBlock`; one line about TermPocket with `TERMPOCKET_URL`; `SignupForm source="about"`)
- Modify: `apps/web/src/components/CalendarWeek.tsx`
- Modify: `not-found.tsx`, `global-not-found.tsx` (paper styles)
- Test: `apps/web/tests/pages.test.tsx`, `components.test.tsx`, `about.test.tsx` (new)

- [ ] **Step 1: Write the failing tests**: `archive rows show a small cover (flat when the post has none), kicker, title and date`; `about page renders the author block, the TermPocket line and the signup`; `calendar week renders ruled day rows without badges`.
- [ ] **Step 2: Run** → fail.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → pass.
- [ ] **Step 5: Commit** `feat(web): lists, calendar, about and 404 in the paper style`

---

### Task 11: Remove the old shell, full verification, docs

**Files:**
- Delete: `apps/web/src/components/Sidebar.tsx`, `EarningsStrip.tsx` (strip is gone from both pages), `Pill.tsx`, `NumbersBlock.tsx`, `MoversList.tsx`, `TickerLogo.tsx` placeholder branch (keep the logo `<img>` helper if `Cover` uses it), `LangSwitch` pill class
- Modify: tests that referenced them (`components.test.tsx`, `partners.test.ts` Sidebar/PartnerBoxes blocks)
- Modify: `apps/web/messages/*.json` (remove `termpocket`, `home.briefTabs`, `home.tabMovers`, `home.tabEarnings`, `home.panelTabs`, `home.moversInBrief`, `earnings.strip*`; keep everything still referenced — grep before deleting)
- Modify: `docs/runbook.md` (web secrets: `BUTTONDOWN_API_KEY`; migration 0006 in the deploy steps; the local preview needs `wrangler d1 migrations apply prebell --local` from `crates/generator`)
- Modify: `apps/web/README.md` or `README.md` if either describes the old layout

- [ ] **Step 1: Grep for every deleted symbol and message key** (`rg "Sidebar|EarningsStrip|Pill|NumbersBlock|MoversList|tlogo-none|termpocket|briefTabs" apps/web/src apps/web/tests`) → remove or replace each use.
- [ ] **Step 2: Run everything**

Run: `cd apps/web && pnpm test && pnpm typecheck && pnpm cf:build`; repo root: `cargo test -p prebell-core --locked && cargo check -p prebell-generator --target wasm32-unknown-unknown --locked && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: all green; web test count stated in the report.

- [ ] **Step 3: Local preview check.** From `crates/generator`: `pnpm exec wrangler d1 migrations apply prebell --local`; from `apps/web`: `pnpm preview` and GET `/en`, `/zh`, `/en/archive`, `/en/earnings`, `/en/about`, one `/en/p/<slug>` seeded by `db/seed-preview.sql` (update that seed with cover columns and a `kv` `index_line` row). Save a screenshot of `/en` and `/en/p/<slug>` to the plan's workspace and state in the report that no pill, tile or neon colour remains.
- [ ] **Step 4: Commit** `chore(web): remove the dashboard shell; docs for covers and signup`

---

## Self-review notes

- Spec coverage: palette/type/masthead/index line (T6), home (T8), post page (T9), lists/about/404 (T10), cover composition and scenes (T2–T4, T7), author block (T7), signup (T7), partner strip (T7), footer (T6), kv snapshot (T1–T3, T5–T6), migration and seeds (T1), diagnostics `cover=` (T3), runbook (T4, T11), decisions.md already done (bc38faf).
- Interfaces: `CoverLines`, `CoverMaker`, `IndexSnapshot` (T2) are consumed by T3 and T4; `PostListItem.coverKey/coverLines/ticker` (T5) by T7–T10; `Cover` (T7) by T8–T10; `MoversTable` (T8) by T9; `IndexLineView` (T5) by T6.
- Proportion: the plan decides files, names, signatures, copy and tests; bodies are left to the implementer except the SQL migration and the API shapes.
