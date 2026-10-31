# Plan 6: Site Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the PreBell web UI as a Robinhood-style green/red portal (earnings strip, brief block, movers, partner sidebar) with a numbers → movers → story brief page.

**Architecture:** Only `apps/web` changes. Pages stay server-rendered and edge-cacheable. Structured numbers and movers come from each post's facts JSON in R2 (`FACTS` binding, key `posts.facts_r2_key`), parsed into a small typed view model. The story still comes from D1 `body_md`, with its title, summary and tables trimmed. Partner links are country-gated by a client island calling an uncached `/api/partners` route.

**Tech Stack:** Next.js 16 App Router on Cloudflare Workers (OpenNext 1.20), next-intl 4, react-markdown + remark-gfm, plain CSS with custom properties, vitest 5 + better-sqlite3 (in-memory D1) for tests.

**Spec:** `docs/redesign-spec.md` (read it; its "Decisions from the builder check" section is binding).

## Global Constraints
- Scope is `apps/web` (plus `db/` preview fixtures/scripts). Don't touch `crates/`, `migrations/` or any wrangler config except `apps/web/wrangler.jsonc` if a binding is strictly needed (it shouldn't be).
- Plain CSS in `src/app/globals.css` with custom properties. No new UI framework, CSS-in-JS or Tailwind. New npm dependencies only if unavoidable, and say why in the report.
- Colors: up `#00C805` (text on light `#00A804`), down `#FF5000` (text on light `#E64800`), pills use the bright color with black text, flat/missing = muted grey. Theme follows `prefers-color-scheme`: light page = white with a black header; dark page = `#000`.
- Every user-visible string goes in both `messages/en.json` and `messages/zh.json`.
- Numbers shown must come from stored facts or D1. The only derived value allowed is the point change `last − prev_close` (2 decimals). Sign and color come from `change_pct`; a value that rounds to 0.00 is grey with no sign. Prices use 2 decimals with thousands separators; percentages use 2 decimals with an explicit sign (use U+2212 "−" for display).
- `diagnostics` in facts JSON is internal and is never rendered.
- No gated partner link may appear in server-rendered HTML. Partner links use `target="_blank" rel="sponsored noopener noreferrer"`.
- Partner URLs, verbatim: moomoo `https://j.moomoo.com/0J1BGf`; Wealthsimple `https://wealthsimple.com/invite/QDBQ5Z`; OKX `https://okx.com/join/AIAFMP3U`; Crypto.com `https://crypto.com/app/smqqda7396`; TermPocket `https://www.termpocket.com`.
- Rules: moomoo hidden for `CN`; Wealthsimple only for `CA`; OKX and Crypto.com hidden for `CN`. An unknown or missing country shows everything except Wealthsimple.
- The existing safety rules in `PostBody` stay as they are: links only to the post's sources or same-site paths, no images, no raw HTML.
- Facts read failures (missing key, R2 error, bad JSON) fall back to the current markdown rendering. They never throw a page error.
- Verification for every task: `pnpm test`, `pnpm typecheck` and `pnpm lint` in `apps/web` pass. Tasks touching pages also run `pnpm build`.
- Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01C33suxkNuxRbEfR6iUqCky
  ```

## Review Focus
1. **ZH page while the ZH translation is pending.** Numbers and movers render; only the story area shows the pending notice (Task 6 and Task 7 tests).
2. **Facts with `Missing` fields** (for example Russell futures missing, or every mover without reasons): the number shows "—" in grey, and the reason line shows "No clear news" in EN or nothing in ZH (Task 3 tests).
3. **Old posts whose facts object is absent** (the seed rows point at `facts/premarket/...`, which doesn't exist): the page falls back to markdown without an error (Task 2 and Task 7 tests).
4. **A visitor from CN or CA, and one with no country header**: the right links appear or are hidden (Task 4 tests).
5. **Weekend homepage**: the earnings strip shows the next trading day that has entries, with that date label, or is hidden (Task 5 tests).

---

### Task 1: Visual system, shell and nav

**Files:**
- Modify: `apps/web/src/app/globals.css` (rewrite), `apps/web/src/app/[locale]/layout.tsx`, `apps/web/src/components/LangSwitch.tsx`, `apps/web/src/components/Disclaimer.tsx`, `apps/web/messages/en.json`, `apps/web/messages/zh.json`, `apps/web/src/lib/posts.ts` (`CONTENT_TYPES`/`parseContentType` and the list query accept `earnings`), `apps/web/src/app/[locale]/archive/page.tsx` (earnings filter tab)
- Test: `apps/web/tests/posts.test.ts`

**Interfaces:**
- Produces:
  - CSS tokens `--bg --fg --muted --line --tile --up --down --up-pill --down-pill --accent`.
  - Utility classes `.pill.up/.down/.flat`, `.num.up/.down/.flat`, `.tile`, `.row`, `.lbl`, `.grid-portal` (3 columns ≥1024px: `1fr 1fr 260px`; one column below) and `.grid-article` (`1fr 260px`; one column below).
  - `parseContentType` accepts `"earnings"`.
  - The header renders the wordmark (`Pre<b>Bell</b>` plus `盘前铃`), nav links Briefs→`/{locale}/archive`, Earnings→`/{locale}/archive?type=earnings`, Calendar→`/{locale}/earnings`, About→`/{locale}/about`, and a pill language switch.

- [ ] Step 1: Failing tests. `parseContentType("earnings") === "earnings"`; `listPosts` with type earnings returns only earnings posts (seed an earnings post in the in-memory D1, see `tests/d1.ts` and the existing `posts.test.ts`).
- [ ] Step 2: Run `pnpm test`, expect FAIL.
- [ ] Step 3: Implement the posts/archive change, the new nav, header, footer and messages (new keys `site.briefs`, `site.calendar`, `site.tagline_short` as needed; keep existing keys that other pages use), and rewrite `globals.css` to the visual system above, keeping existing class names used by other pages working (or restyling them).
- [ ] Step 4: `pnpm test && pnpm typecheck && pnpm lint && pnpm build` pass.
- [ ] Step 5: Commit `feat(web): visual system, header and nav`.

### Task 2: Facts view model and number formatting

**Files:**
- Create: `apps/web/src/lib/facts.ts`, `apps/web/src/lib/format.ts`, `apps/web/tests/facts.test.ts`, `apps/web/tests/format.test.ts`, `apps/web/tests/fixtures/recap-sample.json` (hand-built from `RecapFacts` in `crates/core/src/recap/mod.rs`)
- Existing fixture: `apps/web/tests/fixtures/premarket-2026-10-30.json` (real generator output)
- Create: `db/preview-facts/premarket-2026-10-27.json`, `db/preview-facts/premarket-2026-10-28.json` (matching the seed briefs' numbers), and `db/load-preview-facts.sh`, which uploads them to local R2 at `premarket/{date}.json` with `wrangler r2 object put prebell-facts/... --local` from `crates/generator`. Also modify `db/seed-preview.sql` so `facts_r2_key` uses `premarket/{date}.json`.

**Interfaces:**
- Produces:
  - `type Dir = "up" | "down" | "flat"`
  - `interface QuoteView { symbol: string; last: number; changePct: number; pointChange: number; dir: Dir }`
  - `interface IndexView { symbol: string; quote: QuoteView | null }` (null = Missing)
  - `interface MoverView { symbol: string; name: string; quote: QuoteView; reason: string | null; reasonUrl: string | null }`
  - `interface FactsView { kind: "premarket" | "recap"; tradingDate: string; asOfUtc: string; lead: IndexView; others: IndexView[]; gainers: MoverView[]; losers: MoverView[] }`
  - `parseFacts(kind, json: unknown): FactsView | null` (pure; null on any shape error)
  - `async loadFacts(bucket: R2Like | undefined, key: string | null, kind): Promise<FactsView | null>` (never throws)
  - `R2Like` is the minimal `{ get(key): Promise<{ text(): Promise<string> } | null> }`.
  - Lead index: `ES=F` (premarket) or `^GSPC` (recap). Others, in order: `NQ=F, YM=F, RTY=F` or `^IXIC, ^DJI, ^RUT`. Matching is by `Quote.symbol`; if the symbol is absent, fall back to the position in the array.
  - `format.ts`: `fmtPrice(n)` ("7,665.50"), `fmtPct(p)` ("+1.14%", "−2.68%", "0.00%"), `fmtPoint(n)` ("+24.75", "−88.75"), `dirOf(changePct): Dir` (flat when `Math.abs(round2(p)) === 0`).
  - Chinese index display names come from messages (`index.ES=F` → 标普500期货, etc.; recap 标普500, 纳指, 道指, 罗素2000).

- [ ] Step 1: Failing tests:
  - parse the real premarket fixture: lead `ES=F` last 7665.5, `changePct` ≈ −1.1445, dir "down", pointChange −88.75; others NQ/YM/RTY; 5 gainers and 5 losers; MDLZ reason non-empty with its URL.
  - parse the recap fixture.
  - `{"Missing":{...}}` futures give `quote: null`.
  - malformed JSON gives null.
  - `loadFacts` with a missing object, a throwing bucket or a null key gives null.
  - format cases including 0.004 → "0.00%" flat.
  - `diagnostics` is not present on the view.
- [ ] Step 2: Run, expect FAIL.
- [ ] Step 3: Implement.
- [ ] Step 4: Tests, typecheck and lint pass. Run `bash db/load-preview-facts.sh` against local state and confirm `wrangler r2 object get ... --local` returns the file.
- [ ] Step 5: Commit `feat(web): facts view model, number formatting, preview facts`.

### Task 3: Numbers and movers components

**Files:**
- Create: `apps/web/src/components/NumbersBlock.tsx`, `apps/web/src/components/MoversList.tsx`, `apps/web/src/components/Pill.tsx`, `apps/web/tests/components.test.tsx`
- Modify: messages (`movers.noNews` EN "No clear news"; ZH not shown), `globals.css` as needed

**Interfaces:**
- Consumes: `FactsView`, `QuoteView`, `MoverView` and the format functions from Task 2.
- Produces:
  - `<NumbersBlock facts={FactsView} variant="home" | "page" locale />`: the big lead number, point change and %, a label (EN "S&P 500 futures" / ZH "标普500期货"; recap "S&P 500" / "标普500") with the ET time from `asOfUtc`, a reserved `.chart-slot` (fixed height, empty), and the three other indexes as tiles on home and rows on the page.
  - `<MoversList movers={MoverView[]} locale limit? showPrice? />`: rows with bold ticker, muted name, a reason line only when `locale === "en"` (reason or muted "No clear news"), optional price, and a `<Pill>`.
  - `<Pill pct />`: shows the sign and %, class from `dirOf`.

- [ ] Step 1: Failing tests (`react-dom/server` `renderToStaticMarkup`):
  - the down pill has class `down` and text "−26.00%".
  - a flat mover is grey.
  - a missing index renders "—" with class `flat`.
  - EN with no reason shows "No clear news".
  - ZH shows no reason line.
  - the reason text is escaped (no HTML injection).
  - the lead label and time render.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: tests, typecheck and lint pass. Step 5: commit `feat(web): numbers and movers components`.

### Task 4: Partner links (country-gated) and TermPocket card

**Files:**
- Create: `apps/web/src/lib/partners.ts`, `apps/web/src/app/api/partners/route.ts`, `apps/web/src/components/Sidebar.tsx` (server: static boxes, disclosure, TermPocket), `apps/web/src/components/PartnerLinks.tsx` (`"use client"` island), `apps/web/tests/partners.test.ts`
- Modify: messages (box titles, disclosure, TermPocket copy, all verbatim from the spec), `next.config.ts` (`/api/partners` → `Cache-Control: private, no-store`)

**Interfaces:**
- Produces:
  - `type PartnerGroup = "brokers" | "crypto"`
  - `interface Partner { id: string; name: string; url: string; group: PartnerGroup; allowedIn(country: string | null): boolean }`
  - `partnersFor(country: string | null): Partner[]`
  - `countryOf(req: Request): string | null`: reads `cf-ipcountry` (ignores `XX`/`T1`/empty) and falls back to `request.cf?.country`.
  - `GET /api/partners` → `{ brokers: {id,name,url}[], crypto: {id,name,url}[] }` with `private, no-store`.
  - `<Sidebar locale />` renders the island, the disclosure and the TermPocket card. The island fetches on mount and renders a box only when its group is non-empty. It renders nothing before the response arrives and nothing on error.

- [ ] Step 1: Failing tests:
  - CN → no moomoo, OKX, Crypto.com or Wealthsimple.
  - CA → all four.
  - US → moomoo, OKX, Crypto.com.
  - null → the same as US.
  - `countryOf` with `XX`, missing or lowercase `ca` → normalized.
  - the route handler returns the no-store header and the grouped JSON.
  - Sidebar's server markup contains no partner URL (`renderToStaticMarkup` of `<Sidebar>` must not include `moomoo.com`, `okx.com`, `crypto.com` or `wealthsimple.com`) but does include the TermPocket URL.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: tests, typecheck, lint and build pass. Step 5: commit `feat(web): country-gated partner links and TermPocket card`.

### Task 5: Earnings strip

**Files:**
- Modify: `apps/web/src/lib/earnings.ts` (new query), messages
- Create: `apps/web/src/components/EarningsStrip.tsx`
- Test: `apps/web/tests/earnings.test.ts`

**Interfaces:**
- Consumes: the existing calendar tables and helpers in `earnings.ts` (`CalendarEntry`, `todayEt`, `addDays`).
- Produces:
  - `stripDay(db, fromDate: string, opts: { exact: boolean }): Promise<{ date: string; bmo: CalendarEntry[]; amc: CalendarEntry[]; unknown: CalendarEntry[] } | null>`. With `exact`, it uses only `fromDate` (brief pages). Otherwise it uses the first date ≥ `fromDate` with entries, looking at most 7 days ahead (homepage).
  - `<EarningsStrip day locale />` shows the label (EN "Earnings · Mon, Nov 2", or "Earnings today" when the date is today ET; ZH "财报 · 11月2日 周一" / "今日财报"), group labels (before open / after close / time TBD), and ticker chips. A chip links to the earnings post when `postSlug` is set, otherwise to `/{locale}/t/{ticker}`. It ends with "Full calendar →" to `/{locale}/earnings`. It renders nothing for null.

- [ ] Step 1: Failing tests:
  - weekend `fromDate` returns Monday's entries.
  - an exact date with no entries returns null.
  - grouping by slot.
  - reported entries carry `postSlug`.
  - the component renders chips and links.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: tests, typecheck and lint pass. Step 5: commit `feat(web): earnings strip`.

### Task 6: Homepage

**Files:**
- Modify: `apps/web/src/app/[locale]/page.tsx`, messages, `globals.css`
- Create (if useful): `apps/web/src/components/BriefTabs.tsx`
- Test: `apps/web/tests/home.test.tsx` (render the page's pure parts or a `HomeView` component with injected data)

**Interfaces:**
- Consumes: `getLatestPost`, `listRecentBriefs`, `listLatestEarnings` (existing); `loadFacts` (Task 2); `NumbersBlock`, `MoversList` (Task 3); `Sidebar` (Task 4); `stripDay`, `EarningsStrip` (Task 5).
- Produces: a homepage per the spec's Homepage section and decisions 4/4b.
  - Brief-block tabs Premarket/Recap: the default is whichever latest post has the later `published_at`. Use two links (`?tab=premarket|recap`) or a CSS-only toggle; no client state library.
  - The movers panel follows the selected tab, with 3 per side.
  - The middle column's second tab shows the latest earnings checks, with "All earnings checks →" linking to `/{locale}/archive?type=earnings`.
  - Recent posts follow, then the sidebar.
  - Without facts, the block shows the title, summary and link only, as today. With a pending ZH translation, numbers and movers still render and the story area shows `PendingNotice`.

- [ ] Step 1: Failing tests: render with facts (the lead number and pills appear); without facts (falls back to title and summary); ZH pending (numbers present and the notice present).
- [ ] Step 2: FAIL. Step 3: implement. Step 4: tests, typecheck, lint and build pass. Step 5: commit `feat(web): portal homepage`.

### Task 7: Brief page

**Files:**
- Modify: `apps/web/src/app/[locale]/p/[slug]/page.tsx`, `apps/web/src/lib/posts.ts` (`getAdjacentPosts(db, lang, publishedAt)` for previous/next published posts of type premarket or recap), messages
- Create: `apps/web/src/lib/trim.ts` (`trimStory(md: string): string`, which removes the leading `# ` title line, the first paragraph (the summary) and all GFM tables, and keeps every `## ` section, Sources and the closing disclaimer line), `apps/web/src/components/ColoredPct.tsx` or a react-markdown text renderer that wraps explicitly signed percentages (`+1.2%`, `-1.2%`, `−1.2%`) in `.num.up/.down`. Only explicitly signed numbers are colored; unsigned ones are left plain.
- Test: `apps/web/tests/trim.test.ts`, `apps/web/tests/briefpage.test.tsx`

**Interfaces:**
- Consumes: Tasks 2–5, `PostBody` (extended with an optional `colorSigned` prop that keeps all existing link and image rules).
- Produces the page order from the spec, for premarket and recap posts:
  1. breadcrumb, publish time in ET, the "✓ Numbers checked against source data / ✓ 数字已与源数据核对" badge and the language pill;
  2. `NumbersBlock variant="page"`;
  3. all movers, with price;
  4. h1 title, summary, `PostBody` of `trimStory(body_md)`;
  5. previous/next links.
  - The earnings strip uses the post's trading date with `exact`, and the sidebar is on the right.
  - Without facts, the full `body_md` renders as today (no trimming).
  - Earnings-check posts keep the current layout (charts) inside the new shell.
  - ZH pending keeps the numbers and movers, and the notice replaces the story.

- [ ] Step 1: Failing tests:
  - `trimStory` on the real 2026-10-30 EN and ZH bodies (copy them into fixtures from `<scratchpad>/live-2026-10-30.md`) leaves no `|` table rows, no `# ` title and no summary paragraph, but keeps `## Futures`, `## Sources` with all links, and the disclaimer.
  - "rose 0.42%" is not colored; "-1.14%" is colored down.
  - the source-link allowlist is still enforced after trimming.
  - adjacent posts work.
  - the page renders the badge and the order numbers → movers → title.
- [ ] Step 2: FAIL. Step 3: implement. Step 4: tests, typecheck, lint and build pass. Step 5: commit `feat(web): brief page numbers, movers, story`.

### Task 8: Restyle remaining pages

**Files:**
- Modify: `apps/web/src/app/[locale]/archive/page.tsx`, `earnings/page.tsx`, `t/[ticker]/page.tsx`, `about/page.tsx`, `not-found.tsx`, `src/app/global-not-found.tsx`, `src/components/PostList.tsx`, `CalendarWeek.tsx`, `Charts.tsx`, `PendingNotice.tsx`, `globals.css`

**Interfaces:**
- Consumes: the Task 1 tokens and classes and `Sidebar` (Task 4).
- Produces: every public page uses the new shell (the sidebar on archive, earnings and ticker pages), card/row styles for lists, and calendar chips matching the earnings strip. Behavior and data are unchanged; existing tests stay green.

- [ ] Step 1: Run the existing tests (they must stay green).
- [ ] Step 2: Restyle.
- [ ] Step 3: Tests, typecheck, lint and build pass.
- [ ] Step 4: Commit `feat(web): restyle archive, calendar, ticker and about pages`.

### Task 9: Visual QA

**Files:** fixes anywhere in `apps/web` as needed; screenshots saved outside the repo.

- [ ] Step 1: Prepare local data:
  ```
  cd crates/generator
  pnpm exec wrangler d1 migrations apply prebell --local
  pnpm exec wrangler d1 execute prebell --local --file ../../db/seed-preview.sql
  bash ../../db/load-preview-facts.sh
  ```
  The real 2026-10-30 brief is already in local state.
- [ ] Step 2: Start the site. Stop any server on :8787 first (`lsof -ti:8787 | xargs kill`), then run `cd apps/web && pnpm preview`.
- [ ] Step 3: Take headless Chrome screenshots (`"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --hide-scrollbars --window-size=W,H --screenshot=...`, with `--force-dark-mode` and `--blink-settings=preferredColorScheme=0` for dark) of:
  - `/en`, `/zh`, `/en/p/premarket-2026-10-30`, `/zh/p/premarket-2026-10-30`, `/zh/p/premarket-2026-10-28` (pending), `/en/archive?type=earnings`, `/en/earnings`;
  - at 1280px and 390px, in light and dark.
  - Save to `<scratchpad>/qa/`.
- [ ] Step 4: Compare against `.superpowers/brainstorm/98568-1793328857/content/homepage-portal-v2.html` and `brief-page.html`. Fix:
  - overflow or horizontal scroll at 390px;
  - contrast problems;
  - wrong colors versus signs;
  - leftover blue links;
  - misalignment.
- [ ] Step 5: Tests, typecheck, lint and build pass. Commit `fix(web): visual QA pass`. The report lists each screenshot path.
