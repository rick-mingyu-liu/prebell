# Plan 4 — Competitor Watch (internal only) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Once a day, at about 7:30 AM ET, the generator reads AlphaLab's public RSS feed, stores only each item's title, link, published time, category and the tickers in its title, compares the last 24 hours against what PreBell published, and reports the gaps as one webhook digest plus an admin-only page. None of this data ever reaches article generation.

**Architecture:** Pure logic lives in a new `core::competitor` module (feed parser, ticker extraction, report and digest, a daily-run function behind a `CompetitorStore` trait), tested natively with fixtures and an in-memory store. The generator Worker adds one cron, dispatches a `competitor` queue job, and implements `CompetitorStore` on D1. The digest is inserted as an `info` row in the existing `alerts` table, so the existing webhook delivery (1900-char cap) sends it unchanged. The generator also stores the computed report as JSON in D1; the Next.js admin page only reads and renders it, so the comparison logic exists once, in Rust.

**Tech Stack:** As Plan 1. No new crates: the RSS parser is a small hand-written scanner; dates use `chrono`'s RFC 2822 parser.

**Spec:** `docs/spec.md` §3 goal 6, §5 (anti-goals), §7 (Competitor watch). Executors read both.

## Global Constraints

- Feed: `https://www.alphalab.site/feed.xml` (public RSS 2.0). One GET per day in production. Tests never hit the network: they use the hand-written fixture `crates/core/tests/fixtures/alphalab_feed.xml` in the feed's shape (CDATA titles, `dc:creator`, RFC 2822 `pubDate`), with invented titles.
- Stored per item: **title, link, published time (UTC), category, tickers in the title**. Never descriptions, `content:encoded`, authors or bodies. The parser never returns them: `FeedItem` has no field to hold them.
- **Never feeds generation.** `premarket/`, `run.rs`, `facts.rs`, `store.rs` and the prompts never import or read `competitor` code or tables. A core test enforces this by scanning those sources.
- No AlphaLab text in any of our content. Titles appear only in the internal digest and the admin page.
- Schedule: crons `31 11 * * *` and `31 12 * * *` (daily, both DST offsets; two crons because the schedule parser can't read comma lists). The run is due only when the ET time is in `[07:30, 08:00)`, so exactly one firing per day counts (EDT: 11:31Z; EST: 12:31Z). Minute 31 keeps it off every premarket cron minute, so each firing is its own scheduled event. One report per ET date: an existing report for that date makes later firings a no-op.
- Report window: their items published in the 24 hours before the run (`now - 24h ≤ t < now`). Our posts (EN published, by `published_at`) are compared from 00:00 ET of the previous NYSE trading day, or 24h if that is earlier, so weekend and Monday reports still see Friday's posts (`our_window_start`).
- Digest: one `alerts` row, `level = 'info'`, `run_key = 'competitor:YYYY-MM-DD'`; built to at most `DIGEST_MAX_CHARS = 1850` characters so `alerts::deliver`'s `"{level}: {run_key}: "` prefix keeps it under its 1900-char cap without truncating mid-line. Never includes the admin token.
- Fetch failure, non-2xx, or a body that isn't RSS: an `error` alert, no report for that day; the next day runs normally.
- Admin page: `/admin/competitor?token=…`, checked in constant time against the `ADMIN_TOKEN` secret (unset ⇒ always 404); wrong token ⇒ 404. `noindex, nofollow` meta and `X-Robots-Tag`, `Cache-Control: private, no-store`, `Referrer-Policy: no-referrer`. Not linked from any public page, English only.
- Secrets only via `wrangler secret put` / `.dev.vars`: new `ADMIN_TOKEN` (web Worker).

## Review Focus

1. **Generation isolation.** No path from `competitor_items`/`competitor_reports` or `core::competitor` into facts, prompts or drafts. Test in Task 5.
2. **Only metadata stored.** Descriptions/`content:encoded` (even with `<title>` or `<category>` text inside their CDATA) never leak into stored fields. Test in Task 2.
3. **DST.** On 2026-10-30 (EDT) only 11:30Z counts; on 2026-11-02 (EST) only 12:30Z counts; a second firing the same ET day is a no-op. Test in Task 5.
4. **Ticker false positives.** `AI`, `CEO`, `GDP`, `Q3`, `ETF`, `FOMC` are not tickers; `MSFT`, `(KLAC)`, `$NVDA`, `（TSM）`, `(NASDAQ: AAPL)`, `BRK.B` are. Test in Task 3.
5. **Digest size.** A report with hundreds of gaps still yields one message ≤ 1850 chars, ending with an "and N more" line. Test in Task 4.

---

## File Structure

```
migrations/0004_competitor.sql            # competitor_items, competitor_reports
crates/core/src/competitor/
  mod.rs        # competitor_decision, CompetitorStore, MemCompetitorStore, run_competitor
  feed.rs       # parse_feed: RSS → Vec<FeedItem> (metadata only)
  tickers.rs    # tickers_in(text) → Vec<String>
  report.rs     # build_report, Report, digest
crates/core/tests/fixtures/alphalab_feed.xml   # hand-written, invented titles
crates/generator/src/competitor.rs        # CompetitorStore on D1
crates/generator/src/lib.rs               # cron + queue dispatch (modified)
crates/generator/wrangler.toml            # + crons "31 11 * * *", "31 12 * * *" (modified)
apps/web/src/lib/competitor.ts            # getLatestReport, listRecentItems, tokenMatches
apps/web/src/app/admin/layout.tsx         # separate root layout, noindex
apps/web/src/app/admin/competitor/page.tsx
apps/web/next.config.ts                   # /admin headers (modified)
apps/web/cloudflare-env.d.ts              # ADMIN_TOKEN (modified)
apps/web/tests/d1.ts                      # load every migration (modified)
apps/web/tests/competitor.test.ts
```

---

### Task 1: D1 schema

**Files:** Create `migrations/0004_competitor.sql`.

**Interfaces:**
- Produces:
  - `competitor_items(link TEXT PK, title TEXT NOT NULL, published_at TEXT /* UTC ISO Z, NULL if pubDate missing */, category TEXT /* categories joined by ', ' */, tickers TEXT NOT NULL DEFAULT '' /* space-separated */, first_seen_at TEXT NOT NULL, last_seen_at TEXT NOT NULL)` + index on `published_at`.
  - `competitor_reports(report_date TEXT PK /* ET date */, window_start TEXT, window_end TEXT, report_json TEXT, digest TEXT, created_at TEXT)` (all NOT NULL).
- File number 0004 reserves 0002/0003 for Plans 2 and 3; D1 applies migrations in name order, so the gap is harmless.

- [ ] **Step 1:** Write the migration with a header comment: internal only, never read by generation.
- [ ] **Step 2: Verify** with SQLite: apply `0001` then `0004`, insert one item and one report, select them; insert the same `link` twice with `ON CONFLICT(link) DO UPDATE` → one row.
- [ ] **Step 3: Commit** `feat(db): competitor watch tables`.

---

### Task 2: RSS parser (`core::competitor::feed`)

**Files:** Create `crates/core/src/competitor/feed.rs`, `crates/core/tests/fixtures/alphalab_feed.xml`.

**Interfaces:**
- Produces:
  - `struct FeedItem { title: String, link: String, published: Option<DateTime<Utc>>, categories: Vec<String> }` (`Clone, Debug, PartialEq`).
  - `enum FeedError { NotRss }` — body has no `<rss` / `<channel`.
  - `fn parse_feed(xml: &str) -> Result<Vec<FeedItem>, FeedError>`.
- Rules: scan `<item>` elements (skipping comments and CDATA so markup inside CDATA is never a tag); read only the item's direct `title`, `link`, `pubDate`, `category` children; CDATA unwrapped verbatim, entities decoded outside CDATA (`&amp; &lt; &gt; &quot; &apos; &#N; &#xN;`); whitespace trimmed and collapsed; items without a title or link are dropped; an unparseable `pubDate` gives `published: None`.

- [ ] **Step 1: Failing tests:** fixture → 5 items; item 1 title (CDATA) and link exact, `published == 2026-10-29T11:05:00Z` from `Wed, 29 Oct 2026 19:05:00 +0800`; `GMT` dates parse; two `<category>` elements → both, in order; a `<description><![CDATA[<title>X</title><category>Y</category>]]></description>` placed before `<title>` does not change title or categories; entity title `S&amp;P` → `S&P`; `"<html>…"` → `Err(NotRss)`; item without link dropped; bad `pubDate` → `None`.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core competitor::feed` PASS.
- [ ] **Step 5: Commit** `feat(core): metadata-only RSS parser for competitor watch`.

---

### Task 3: Ticker extraction (`core::competitor::tickers`)

**Files:** Create `crates/core/src/competitor/tickers.rs`.

**Interfaces:**
- Produces: `fn tickers_in(text: &str) -> Vec<String>` — unique, in first-seen order.
- Rules: an ASCII token is a maximal run of `[A-Za-z0-9.]` (trailing `.` dropped), so CJK text bounds it (`微软MSFT财报` → `MSFT`).
  - `$` prefix (`$NVDA`, `$F`): 1–5 uppercase letters, optional `.X` class suffix → ticker.
  - In parentheses, ASCII or full-width (`(KLAC)`, `（TSM）`), optionally `EXCHANGE:` first (`NASDAQ`, `NYSE`, `AMEX`, `NYSEARCA`, `OTC`) → 1–5 uppercase letters (+ `.X`), not in `STOPWORDS` (which includes `A`, `B`) → ticker.
  - Bare token: 2–5 uppercase letters (+ optional `.X`) in the S&P 500 ∪ Nasdaq-100 universe (`data/universe.txt`; so `AWS`, `HPC`, `BTC`, `TW` are not tickers), not in `STOPWORDS` (`AI`, `CEO`, `CFO`, `US`, `GDP`, `CPI`, `PCE`, `FOMC`, `FED`, `ETF`, `EPS`, `IPO`, `SEC`, `NYSE`, `NASDAQ`, `ET`, `AM`, `PM`, `YOY`, `QOQ`, `USD`, `EV`, `GPU`, `HBM`, …). Tokens containing digits (`Q3`, `FY2027`) are never tickers. A bare word before a parenthesized ticker is the company name (`KLA (KLAC)`), but not before a parenthesized stopword (`MSFT (AI)` → `[MSFT]`, `AAPL (NASDAQ)` → `[AAPL]`).

- [ ] **Step 1: Failing tests:** `"Why MSFT and $NVDA moved"` → `[MSFT, NVDA]`; `"KLA (KLAC) beats"` → `[KLAC]`; `"台积电（TSM）"` → `[TSM]`; `"(NASDAQ: AAPL)"` → `[AAPL]`; `"BRK.B rises."` → `[BRK.B]`; `"AI capex, CEO says GDP Q3 FY2027 ETF FOMC"` → `[]`; `"Ford ($F)"` → `[F]`; a lone `"F"` → `[]`; duplicates collapse.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core competitor::tickers` PASS.
- [ ] **Step 5: Commit** `feat(core): ticker extraction from titles`.

---

### Task 4: Report and digest (`core::competitor::report`)

**Files:** Create `crates/core/src/competitor/report.rs`.

**Interfaces:**
- Consumes: `tickers_in`.
- Produces (`Serialize, Deserialize, Clone, Debug, PartialEq`; JSON shape is read by `apps/web/src/lib/competitor.ts`):
  - `struct StoredItem { link, title, published_at: String, category: String, tickers: Vec<String> }`
  - `struct OurPost { slug: String, text: String /* EN title + summary + body + source titles */ }`
  - `struct ItemRef { title, link, category, published_at }`
  - `struct TickerGap { ticker: String, items: Vec<ItemRef> }`
  - `struct Report { report_date, window_start, window_end, their_item_count: usize, our_post_count: usize, our_slugs: Vec<String>, uncovered: Vec<TickerGap>, covered: Vec<String>, topics: Vec<ItemRef> }`
  - `fn build_report(report_date: NaiveDate, window_start, window_end: DateTime<Utc>, theirs: &[StoredItem], ours: &[OurPost]) -> Report`
  - `const DIGEST_MAX_CHARS: usize = 1850;` `fn digest(r: &Report) -> String`
- Rules: our coverage = `tickers_in` over each `OurPost.text` (a ticker in our tables counts). `uncovered` = their tickers not in our coverage, sorted by number of their items (desc) then ticker; `covered` = their tickers we also have, sorted. `topics` = their items with no ticker (we can't match these by code; listed for a human). Digest: header line (date, counts), `Not covered:` lines `TICKER ×n — title (link)` (title cut to 80 chars), `Topics without tickers:` lines `[category] title`, then `Covered by both: …`; lines that don't fit are replaced by `…and N more (see /admin/competitor)`; never over `DIGEST_MAX_CHARS`.

- [ ] **Step 1: Failing tests:** their `MSFT`, `KLAC`, `NVDA` vs our post text containing `| NVDA |` → uncovered `[MSFT, KLAC]` (MSFT has 2 items, first), covered `[NVDA]`; an untickered item is in `topics`; 300 gap tickers → `digest` ≤ 1850 chars and ends with an `…and N more` line where N + listed = 300; an empty report digest says no items; JSON round-trip equals original.
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core competitor::report` PASS.
- [ ] **Step 5: Commit** `feat(core): competitor gap report and digest`.

---

### Task 5: Daily run (`core::competitor`)

**Files:** Create `crates/core/src/competitor/mod.rs`; modify `crates/core/src/lib.rs` (`pub mod competitor;`).

**Interfaces:**
- Consumes: `Http`, `clock::et`, Tasks 2–4.
- Produces:
  - `const FEED_URL: &str`, `fn run_key(d: NaiveDate) -> String` = `"competitor:YYYY-MM-DD"`.
  - `enum CompetitorDecision { NotNow, Due { report_date: NaiveDate } }`, `fn competitor_decision(now: DateTime<Utc>) -> CompetitorDecision` — due iff ET time in `[07:30, 08:00)`, any day of the week.
  - `trait CompetitorStore` (`async_trait(?Send)`, `Result<_, String>`): `has_report(date) -> bool`, `upsert_items(items: &[NewItem], seen_at: &str)` (`NewItem { link, title, published_at: Option<String>, category, tickers }`), `items_between(start, end) -> Vec<StoredItem>`, `our_posts_between(start, end) -> Vec<OurPost>`, `put_report(report: &Report, digest: &str, created_at: &str)`, `add_alert(run_key, level, msg)`. `struct MemCompetitorStore` for tests.
  - `enum CompetitorOutcome { Nothing, AlreadyDone, Reported { items: usize, uncovered: usize }, Failed(String) }`
  - `async fn run_competitor(now, http: &dyn Http, timer: &dyn Timer, store: &dyn CompetitorStore) -> CompetitorOutcome` — GET `FEED_URL` with the neutral `User-Agent: PreBell (+https://prebell.ai)` (no label, no email), raced against `timer.sleep(15s)`, body capped at 5 MB (`Http::get_capped`); stored title ≤ 500 chars and category ≤ 100 (cut), link ≤ 2,048 (else the item is dropped); on non-2xx / timeout / oversize / network / `NotRss` / store error: `error` alert, `Failed`; else upsert items (≤ 200), read the 24h window back from the store, build report, `put_report`, `info` alert with the digest, `Reported`.

- [ ] **Step 1: Failing tests (FakeHttp + MemCompetitorStore):** `competitor_decision` at 2026-10-30T11:30Z (EDT) and 2026-11-02T12:30Z (EST) is `Due`, at 2026-10-30T12:30Z and 2026-11-02T11:30Z is `NotNow`, Saturday 2026-10-31T11:30Z is `Due`; a run stores 5 items with only metadata and writes one report and one `info` alert ≤ 1850 chars; a second run the same day → `AlreadyDone`, still one report/alert; HTTP 503 → `Failed`, one `error` alert, no report; HTML body → `Failed`; a re-fetched item keeps one row. **Isolation guard:** `include_str!` of `premarket/mod.rs`, `premarket/draft.rs`, `run.rs`, `facts.rs`, `store.rs` and both prompt files contain no `competitor` / `alphalab` (case-insensitive).
- [ ] **Step 2–4:** FAIL → implement → `cargo test -p prebell-core competitor` PASS.
- [ ] **Step 5: Commit** `feat(core): daily competitor watch run`.

---

### Task 6: Generator wiring

**Files:** Create `crates/generator/src/competitor.rs`; modify `crates/generator/src/lib.rs`, `crates/generator/wrangler.toml`.

**Interfaces:**
- Consumes: `run_competitor`, `CompetitorStore`, `D1Store`, `WorkerHttp`, `alerts::deliver`.
- Produces: `impl CompetitorStore for D1Store` (items upsert in one D1 batch, `ON CONFLICT(link) DO UPDATE` keeping `first_seen_at`; our posts = `posts` ⋈ EN `post_translations` with `status='published'` and `published_at` in the window, plus that post's `sources.title`); `COMPETITOR_CRONS = ["31 11 * * *", "31 12 * * *"]`; `scheduled` sends `{"job":"competitor"}` for those crons and `{"job":"premarket"}` otherwise; `queue` dispatches on `job`, running `run_competitor(now(&env), …)` for `competitor`, then delivers alerts as before. Premarket providers are built only for premarket jobs.

- [ ] **Step 1:** `cargo clippy --workspace --all-targets -- -D warnings`, `worker-build --release` → builds.
- [ ] **Step 2: Verify locally (no network to alphalab needed for the dispatch check):** `wrangler d1 migrations apply prebell --local`; `wrangler dev --test-scheduled --local` with `FORCE_NOW_UTC=2026-10-30T11:30:00Z`; `curl "…/__scheduled?cron=31+11+*+*+*"` → log `job competitor: …`. Record the outcome in the PR.
- [ ] **Step 3: Commit** `feat(generator): daily competitor watch job`.

---

### Task 7: Admin page (`apps/web`)

**Files:** Create `apps/web/src/lib/competitor.ts`, `apps/web/src/app/admin/layout.tsx`, `apps/web/src/app/admin/competitor/page.tsx`, `apps/web/tests/competitor.test.ts`; modify `apps/web/tests/d1.ts`, `apps/web/next.config.ts`, `apps/web/cloudflare-env.d.ts`.

**Interfaces:**
- Consumes: `competitor_reports`, `competitor_items`; report JSON shape from Task 4.
- Produces: `tokenMatches(given: unknown, expected: string | undefined): boolean` (false when expected unset/empty or given isn't a string; constant-time over equal lengths); `getLatestReport(db): Promise<CompetitorReport | null>`; `listRecentItems(db, limit = 100): Promise<CompetitorItem[]>` newest first. Page: 404 unless the token matches; otherwise shows the latest report (gaps with their item links, topics, covered list, our posts) and recent items, times in ET. Admin root layout sets `robots: { index: false, follow: false }`. External links use `rel="noreferrer noopener"`.

- [ ] **Step 1: Failing tests:** `tokenMatches("abc","abc")` true; `("abd","abc")`, `("abc","")`, `("abc",undefined)`, `(["abc"],"abc")` false; `getLatestReport` returns the newest `report_date` with parsed `uncovered`; `listRecentItems` orders by `published_at` desc and splits `tickers`; `memoryD1` applies every migration.
- [ ] **Step 2–4:** `pnpm --filter web test` FAIL → implement → PASS; `typecheck`, `lint` clean.
- [ ] **Step 5: Commit** `feat(web): admin-only competitor watch page`.

---

## Parallelization (for subagent execution)

- **Wave 1 (parallel):** Tasks 1, 2, 3.
- **Wave 2:** Task 4 (needs 3) → Task 5 (needs 2, 4).
- **Wave 3 (parallel):** Task 6 (needs 1, 5), Task 7 (needs 1, 4's JSON shape).
