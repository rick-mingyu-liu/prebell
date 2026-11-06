# Earnings: widen coverage to S&P 500 ∪ Nasdaq-100

Date: 2026-11-06. Status: design for a builder to plan from; items marked
**[Rick]** need the owner's call before building.

## Intent

docs/decisions.md:56: "fix the failures first, then widen coverage beyond the
S&P 100: all ~520 companies (S&P 500 ∪ Nasdaq-100) on the calendar, full
write-ups only for the largest." Fixes shipped in #33. For Nov 2-6 the D1
calendar had 19 rows (15 `reported`, 4 `unconfirmed`, queried 2026-11-06);
AlphaLab listed 131 companies for the same week.

## Today, measured

- `crates/core/data/watchlist.tsv`: 109 rows (102 domestic, 7 foreign),
  compiled in (`watchlist.rs:8`). `universe.txt`: 518 rows; 6 watchlist
  tickers are not in it (BCS NVO SAN SAP TM TSM), 415 universe tickers are not
  on the watchlist. Three universe pairs share a CIK (GOOG/GOOGL, FOX/FOXA,
  NWS/NWSA), and `watchlist_shape` requires unique CIKs (`watchlist.rs:119`).
  New watchlist: 109 + 415 − 3 = **521 rows**, plus any extra ADRs.
- Remote D1: 27 calendar rows in total, 15 filings, 13 posts.

## 1. Data: generate watchlist.tsv

New `scripts/build-watchlist.mjs` (Node, no deps, same habits as
`apps/web/scripts/build-logos.mjs`: cache of definitive answers, `--offline`,
build into a temp dir and replace the file only when the whole run passes).

Inputs, all in git:
- `crates/core/data/universe.txt` (membership; already maintained by hand).
- New `crates/core/data/watchlist-overrides.tsv`: hand-checked values that
  win over generated ones: display name, domains, tier, share class to keep,
  and the extra ADR rows. Seed it from today's 109 rows so nothing hand-checked
  is lost.

Fetches (User-Agent `PreBell watchlist build support@prebell.site`, ≤5 req/s,
half SEC's 10/s fair-access limit):
- `https://www.sec.gov/files/company_tickers.json` once: ticker → CIK, SEC
  title. SEC writes class shares with a dash (`BRK-B`, `BF-B`); map to dots.
  A universe ticker missing from it fails the run (no guessing).
- `https://data.sec.gov/submissions/CIK##########.json` per CIK (~535 calls,
  ~2 min): `name`, `filings.recent.{form,items,acceptanceDateTime}`.

Derived per row:
- `filer`: `domestic` if any 10-Q in the last 15 months, else `foreign`. Do not
  assume from the country: Linde, Chubb, Accenture, Medtronic, Eaton file
  10-Q/8-K; ASML, ARM, PDD, SHOP, TRI, FER file 6-K/20-F/40-F (all in
  universe.txt:500-520).
- `has_202` (script report only): any 8-K with item 2.02 in 15 months. A
  domestic row without one (Berkshire publishes with its 10-Q) is printed for
  review: it will never be detected, only seeded.
- `sec`: always the SEC title (today optional, `watchlist.rs:27`).
- `domains`: only needed for IR proof (`calendar.rs:401`, own-domain path).
  Write-up tier: keep the hand-checked ones; for new tier rows take Wikidata
  P856 by CIK (the lookup build-logos.mjs already does) and review by hand.
  Calendar tier: may be empty. Relax `watchlist.rs:120-122` to the tier.
- `tier`: `full` or `calendar` (section 4).

Column layout: append `tier` as column 7 and always fill column 6 (`sec`).
`build-logos.mjs:463-468` reads columns 1, 2, 4, 5 by position, so appending
keeps it working. Output sorted by ticker, so a rerun on the same cache is
byte-identical. Keep total rows ≤ 550 (11 Yahoo batches; same cap as
`universe_is_at_most_550`, `universe.rs:137`).

Share classes: one row per CIK, keep GOOGL, FOXA, NWSA (the watchlist already
keeps GOOGL only). `/t/GOOG` then 404s; the builder may alias it in the web
ticker normalizer (optional).

## 2. Detection: EDGAR cost does not scale with the watchlist

The poll reads EDGAR's all-company current 8-K feed (`edgar.rs:192-197`), 1-3
pages back to the watermark (`run.rs:49`, `run.rs:128`), and filters by CIK in
memory (`detect.rs:109-121`). Cost is the same at 109 or 550 rows. The only
growth is D1 work per matched filing (`record_filing`, 2-3 binding calls).

Change: in `poll_tick` (`run.rs:162-172`), call `record_filing` for every
domestic match, but `insert_filing` + `upsert_run` (the build queue) only for
`tier = full`. A calendar-tier filing marks its calendar row `reported` with
`proof_url` = the filing index (`calendar.rs:301`) and nothing else.

Foreign filers have no Item 2.02; they stay calendar-only from Yahoo seeds.
A 6-K feed would add 1-3 requests per poll and 6-Ks are not tagged as
earnings, so it is out of scope.

## 3. Calendar refresh at 535 tickers

Cost today (runbook.md:96): cookie, crumb, 3 v7 batches, ≤5 announcement
searches + 5 LLM calls ≈ 15 fetches.

| Item | Today (109) | Widened (~535) | Limit |
|---|---|---|---|
| Yahoo v7 batches (`QUOTE_BATCH` 50, `yahoo.rs:19,138`) | 3 | 11 | — |
| Yahoo session + 401 refresh (`yahoo.rs:143-151`) | 2 (+3) | 2 (+3) | — |
| IR searches + LLM (`IR_CHECKS` 5, `calendar.rs:28`) | 10 | 10 | — |
| Fetch subrequests, worst case | 18 | 26 | 50 (runbook.md:86) |
| D1 binding calls (see below) | ~60 | ~900 at season peak | 1,000 (runbook.md:87) |
| Tavily searches/month (5 × 21 days) | 105 | 105 | plan quota |

Yahoo: 11 v7 batches from a Worker already run twice a day for the movers
(runbook.md:93), so the request pattern and CPU for parsing are proven. The
calendar runs once a day at 06:00 ET (`calendar_refreshed_on` was written at
11:01Z today), away from the premarket's Yahoo burst.

**D1 is the real limit.** Per seeded ticker, `seed_from` calls `calendar_for`
(`calendar.rs:605`), then `apply` calls it again per change
(`calendar.rs:536`) and upserts. An agreeing seed still changes `checked_at`
and `updated_at` (`calendar.rs:224,236`), so every seed is a write. With
~300 tickers dated in the 10-trading-day window at season peak, that is ~900
calls plus rule 8 and IR checks, near the 1,000 internal-service cap. Fix:
- One ranged read: `calendar_between(today − SAME_EVENT_DAYS, last +
  SAME_EVENT_DAYS)`, grouped by ticker in memory. `reconcile` only looks at
  rows ≥ today and `reported` rows within 21 days (`calendar.rs:162-174`), so
  this range is enough.
- Skip writes whose only change is `checked_at`/`updated_at` (the site never
  reads them), and send the rest as `db.batch()` in chunks of ≤100 (the pattern
  in `earnings_store.rs:189`).
- Result: ~3-6 D1 calls a day, ~600 rows read, tens of rows written (free:
  5M reads, 100k writes a day).

IR proof search: limit candidates (`calendar.rs:657-670`) to `tier = full`.
Otherwise the five daily slots go to calendar-only names that never get a
write-up. Keep `IR_CHECKS` at 5, so Tavily use stays the same. Calendar-tier
rows stay `unconfirmed` (muted) until their 8-K marks them `reported`.

Index churn: `seed_from` only reconciles watchlist tickers (`calendar.rs:574`),
so a dropped company's future `unconfirmed` Yahoo row would stay forever.
Delete `unconfirmed`/`yahoo` rows in the window whose ticker is off the
watchlist; keep `reported` history.

## 4. Write-up tier and build capacity

**[Rick] Tier rule.** Recommendation: `full` = today's S&P 100 rows ∪ any
universe company with market cap ≥ $100B in a dated snapshot (Yahoo v7
`marketCap`, fetched by the script from a laptop, stored in the overrides
file with its date, refreshed with the quarterly index refresh). Expected
~150 companies; the script's first run gives the exact count. Alternatives:
S&P 100 ∪ Nasdaq-100 (deterministic, but pulls in sub-$30B NDX names and
leaves out big S&P 500 names like McKesson or Eaton), or a fixed top-N.
Cost: each write-up is 4 LLM calls (runbook.md:96); 150 vs 102 companies is
about +50% earnings LLM spend. Calendar-tier rows cost no LLM.

Peak load (estimate; confirm against next season's D1): ~150 tier companies
over ~6 weeks, ~30% in the busiest week → ~45 that week, ~12-15 on the peak
day, ~8-10 before the open, clustered 06:30-08:00 ET.

How builds run now:
- Each queue message is its own invocation with its own 50-fetch budget
  (`max_batch_size = 1`, wrangler.toml:48; loop in `generator/src/lib.rs:268`).
  A build is ≤12 fetches (`run.rs:1355`). **Subrequests do not limit builds
  per tick.**
- A poll sends one Build per newly detected filing, else one while any filing
  is open (`run.rs:215-219`). Each Build does at most one build
  (`run.rs:277-279`).
- The earnings queue is serial (`max_concurrency = 1`, wrangler.toml:49) and
  carries the polls too. A build takes 1-5 min (`run.rs:31`). Ten filings in
  one 07:00 poll mean ~10 × 3 min = 30 min of builds. Polls queued behind them
  are more than 5 min late and get dropped (`run.rs:198`, `MAX_POLL_LAG_MINUTES`).
  So detection pauses during a burst, and later filings wait.
- Retries drain at one per 5-min firing when nothing new arrives. The 6 h
  deadline (`run.rs:36`) gives 72 firings, enough for ~10 filings × 3
  attempts. Capacity is fine; latency is the problem.

Proposal:
1. **[Rick] (deploy config)** Move builds to their own queue
   `prebell-earnings-build` (`max_batch_size = 1`, `max_concurrency = 1`), so
   polls never wait behind builds. EDGAR then sees two clients at once; set
   build-side pacing to 250 ms (`GAP_MS`, `edgar.rs:22`; ≤4 req/s) beside the
   poll's 125 ms (≤8 req/s, 1-3 requests). Worst case stays near 10 req/s, and
   the poll's burst is 3 requests. Needs `wrangler queues create`. Queue ops
   stay far under 10k/day (runbook.md:88: ~3 per message).
2. With no new filings, a poll sends `min(due open filings, 3)` Builds instead
   of 1, so retries of a burst finish within ~15 min, not ~45.
3. Keep one build per message and the 15-min lease (`run.rs:32`).

Calendar-only companies on the site **[Rick]**, recommendation: a calendar
row with ticker, company name, slot and ✓ when reported, plus "SEC filing ↗"
to `proof_url` (the 8-K index, a primary source). No write-up, no logo. Their
`/t/[ticker]` page lists dates only and is `noindex` while it has no post
(~370 thin pages otherwise).

## 5. Web

- **Names.** The calendar shows tickers only (`CalendarWeek.tsx:39-42`). At
  130 rows a week names are needed. The watchlist script also writes
  `apps/web/src/lib/companies.generated.ts` (ticker → name, tier; ~25 KB),
  like `logos.generated.ts`. No D1 read. **[Rick]** Chinese pages show English
  names in v1 (recommended); a curated ZH name list is a separate task.
- **Layout** (`/earnings`, server-rendered, no new client JS): per day,
  sub-groups Before open / After close / Time TBA (the query already orders by
  slot, `earnings.ts:110`). Within a slot, `full` tier first, then by ticker.
  Show the first 12 rows per slot and put the rest in a native `<details>`
  "Show N more". Same cap as the home strip, which already truncates with "+N
  more" (`HomeView.tsx:24,85-102`). Optional `?only=checks` filter.
- **Query.** `listCalendarWeek` joins `POSTS_BY_TICKER_DATE` over every
  earnings post (`earnings.ts:74-81,102-119`). Measured on remote D1 today it
  read 112 rows to return 18, and it grows with every post. Add
  `p.trading_date BETWEEN ? AND ?` inside the subquery (the `POSTS_ON_DATE`
  pattern, `earnings.ts:84-92`), and select `c.proof_url` for the "SEC
  filing" link. `listTickerCalendar` (`earnings.ts:171-179`): same fix, by
  ticker.
- **Logos:** filter `build-logos.mjs` to `tier = full` (otherwise ~410 more
  fetches and licence reviews; a full run is ~8 min today).

## 6. Risks and review focus

1. **Wrong CIK / name collisions.** `same_company` matches the first
   significant word (`detect.rs:66-74`). At 535 names many share it:
   American (AXP, AMT, AEP, AIG, AWK), General (GD, GIS, GM), Public (PEG,
   PSA), International (IBM, IP, IFF), United (UPS, URI, UAL), Texas (TXN,
   TPL). A swapped CIK between two of them would pass. Since `sec` is now
   always the SEC title, require the feed filer name to equal it token by
   token. Keep the first-word rule only as a display-name sanity test. A
   rename then shows up as a `watchlist-cik:` alert, which is the right
   signal to refresh.
2. **Wire proof lookalikes** (`calendar.rs:417-424`): generic names (Target,
   Ball, Visa) need the exchange-ticker form. This only affects the `full`
   tier now; add a test per tier name shorter than 5 letters.
3. **Share classes:** GOOG/FOX/NWS are dropped, and BRK.B / BF.B go through the
   dot↔dash mapping both ways (SEC and Yahoo, `yahoo_calendar.rs:57`).
4. **Foreign filers that file 8-K** (filed as domestic by data) and
   domestic filers without 2.02 (BRK): the script's report must be read
   before merging. Expect a handful of each.
5. **Index churn:** the universe is hand-refreshed. Rerun the script with it,
   and the stale-row cleanup in section 3 handles leavers. A company moving
   `full` → `calendar` mid-season finishes its open builds.
6. **CPU (10 ms):** `find`/`watchlist()` re-parse the TSV on every call
   (`watchlist.rs:73-85`); fine at 535 rows but parse once (`OnceCell`) while
   touching it. The refresh's per-ticker loop goes away with the batched read.
7. **Tests to update:** `watchlist_shape` (count, domains per tier),
   `every_watchlist_name_matches_its_sec_name` (all rows),
   `a_build_stays_under_40_subrequests` (unchanged), a new refresh test that
   counts D1 calls for 300 seeds, and a poll test where a calendar-tier
   filing marks `reported` and queues no run.

## Decisions (Rick, 2026-11-06; docs/decisions.md)

1. Write-up tier: S&P 100 + every company at or above $100B in a dated
   market-cap snapshot ("S&P 100 + $100B+"). The snapshot of 2026-11-06 gives
   139 rows (102 + 37).
2. About 15 hand-picked large foreign companies outside both indexes, calendar
   only: the six already listed (BCS NVO SAN SAP TM TSM) plus ENB SONY RACE
   RYAAY SU CNQ TRP MFC BN ITUB TEVA, 17 in all. ENB and TEVA file 10-Q/8-K, so
   their 8-Ks mark the calendar too.
3. Calendar-only companies: row + name + ✓ + SEC filing link; ticker page
   noindex until it has a check.
4. Builds on their own queue, `prebell-earnings-build`.
5. English company names on ZH pages for v1 (controller's call; reversible).

Built as designed, with these rulings: the market caps come from Nasdaq's
stock screener (Yahoo's v7 quote answered 429 to the build machine), and a
filer is foreign only when it files 20-F/40-F/6-K and no 10-Q (so a new
spin-off without a 10-Q yet is domestic).
