# Plan 1 notes

## Task 11: generator Worker, local verification (2026-10-29)

Done with no API keys. `crates/generator/.dev.vars` (git-ignored) had empty
`TAVILY_API_KEY`, `DEEPSEEK_API_KEY` and `ALERT_WEBHOOK_URL`, plus
`FORCE_NOW_UTC`. 2026-10-29 is a Thursday in EDT, so the window time is
`T12:35:00Z`. Commands were run from `crates/generator` with wrangler 4.154.0.

- `worker-build --release`: builds (`index.js 28.0kb`).
- `pnpm exec wrangler dev --test-scheduled --local`: starts. It lists the bindings
  JOBS (queue, local), DB (D1, local), FACTS (R2, local) and the vars.
- `curl localhost:8787/health` returns `ok` (200). Any other path returns 404.
- The local queue consumer does run under `wrangler dev`, so no direct-tick workaround was needed.

### Run 1: in the window (`FORCE_NOW_UTC=2026-10-29T12:35:00Z`)

`curl "http://localhost:8787/__scheduled?cron=*/5+12-14+*+*+1-5"` printed `Ran scheduled event`. Dev log:

```
cron */5 12-14 * * 1-5: queued premarket job
▲ [WARNING] secret TAVILY_API_KEY is not set; provider calls will fail
▲ [WARNING] secret DEEPSEEK_API_KEY is not set; provider calls will fail
job {}: Retrying("movers")
[wrangler:info] QUEUE prebell-jobs 1/1 (5103ms)
```

`wrangler d1 execute prebell --local --command "select run_key,status,attempts,last_error from runs"`:
`premarket:2026-10-29 | pending | 1 | movers`. Yahoo returned no usable
movers from this machine, so the required `movers` field was missing and the
run stayed pending for a retry. Core does not record the Yahoo HTTP status.

The `job {}` in that log led to a fix. The message had been a
`serde_json::Value`, which serde-wasm-bindgen sends as a JS `Map`, so the body
arrived empty. The body is now a `Job { job }` struct, and run 2 logs `job premarket`.

### Run 2: past the deadline, with a webhook (`FORCE_NOW_UTC=2026-10-29T13:30:00Z`)

`ALERT_WEBHOOK_URL` pointed at a local stub (`127.0.0.1:9876`) that answers 204.

```
cron */5 12-14 * * 1-5: queued premarket job
job premarket: Skipped
[wrangler:info] QUEUE prebell-jobs 1/1 (34ms)
stub received: application/json {"content":"error: premarket:2026-10-29: premarket brief skipped for 2026-10-29: not ready by 09:25 ET (movers)"}
```

- `runs` row: `premarket:2026-10-29 | skipped | 1 | movers`.
- `alerts` row: `id=1, level=error, sent=1, created_at=2026-10-29T18:09:06Z`. `created_at` is the real clock, not the forced one.

### SQL spot checks

These were run directly against local D1 and the test rows were deleted afterwards:

- Running the `upsert_post` statement (`ON CONFLICT(run_key) DO UPDATE … RETURNING id`) twice returned the same id both times.
- The `post_translations` upsert updated the row in place.
- D1 enforces the foreign keys: a translation for a post id that does not exist is rejected.

### Not verified (needs real keys)

The publish path was not exercised end to end. That path covers
`upsert_post`, `upsert_translation`, `add_sources` (a DELETE and INSERTs in
one D1 batch) and the R2 `put_facts`/`get_facts`.

Local D1 also already held a run row (`premarket-2026-01-05`) and a post from
Task 1's schema check. Both were left alone.

## Yahoo batching and run diagnostics (2026-10-29)

The first live run sent one Yahoo chart request per symbol: 522 requests in
about 10 seconds. Yahoo answered HTTP 429, no movers survived, and the run
ended `Retrying("movers")` without saying why.

- `premarket_quotes` now uses the v7 quote endpoint
  (`/v7/finance/quote?symbols=…&crumb=…`), 50 symbols per request, sent one
  after another. The cookie comes from `fc.yahoo.com` (a 404 there is normal)
  and the crumb from `/v1/test/getcrumb`. Both are cached for the life of the
  `Yahoo` struct and refreshed once on a 401 or 403. `quote()` (the 4
  futures) still uses the per-symbol chart endpoint.
- Field mapping: `last = preMarketPrice`, `change_pct = preMarketChangePercent`
  (percent units), `prev_close = regularMarketPrice`, which at 8:30 AM ET is
  still the prior session's close. A symbol without `preMarketPrice` (no
  premarket trades) or absent from the response is `Empty`.
- **Stale guard (review ruling):** `preMarketTime` must convert to
  America/New_York on the current ET trading date, before 09:30 ET.
  Otherwise the symbol is `Empty`, because a thin name can still carry an
  earlier day's premarket fields. If `preMarketPrice − preMarketChange`
  differs from `regularMarketPrice` by more than 0.5% of the price, the symbol
  is `Stale` (counted as `stale=`) rather than re-anchored. `marketState` is
  reported as a diagnostic only (`market_state PRE=…`) and gates nothing,
  since it varies across the PREPRE, PRE and CLOSED windows.
- A 200 response with an empty or null `quoteResponse.result` and a non-null
  `error` fails that chunk as `Parse("yahoo error <code>")`, with the code
  sanitized, and is reported as `yahoo_error=<code>`.
- **Mid-run 429:** after a chunk gets HTTP 429, the remaining chunks are not
  requested and their symbols are `Skipped` (`skipped=`, not `http429=`).
  Symbols from chunks that succeeded earlier are still used, so a run can
  publish from a partial universe. This is deliberate: movers are the top
  five per side among the quotes that arrived, and the diagnostics show how
  many symbols were skipped.
- **Liquidity ruling:** v7 has no premarket volume, so `volume` is `None`.
  When `volume` is `None` the mover filter skips the 50,000-share premarket
  volume test and keeps only the `last ≥ $5` floor. Every name in the
  universe is an S&P 500 or Nasdaq-100 large cap.
- The fixtures (`yahoo_quote_batch.json`, `yahoo_getcrumb.txt`) are hand-written
  from the documented response shapes used by yfinance. Yahoo was not
  fetched live, because this IP was throttled.
- **Diagnostics:** `PremarketFacts.diagnostics` (serde default, omitted when
  empty, never sent to the LLM) holds short counters such as
  `quotes ok=480 http429=0 http_other=0 empty=42 stale=0 parse=0 net=0 skipped=0`,
  `yahoo_session=ok refreshes=0` (or `cookie_fail`, `cookie_http<code>`,
  `crumb_fail`, `crumb_http<code>`), `market_state PRE=522`, an optional
  `yahoo_error=<code>`, `movers liquid=470 gainers=5 losers=5`, `reasons proposed=14 kept=9`,
  `search_err=0 llm_err=0` and `futures ok=4 err=0`. When required sections are
  missing, `last_error` is the missing list, then ` | `, then the counters
  joined by `; `, e.g. `movers | quotes ok=0 http429=522 …`. The generator logs
  `diagnostics: …` once per build tick.

## Yahoo live check (2026-10-29, blocked)

Attempted at 20:27 ET on Thursday 2026-10-29 (00:27 UTC on 10-30), from a
Claude Code cloud session. Outside the 4:00–9:30 ET window, so premarket
fields would have been stale even if the calls had gone through.

- **Requests that reached Yahoo: 0.** The session's egress proxy refused the
  CONNECT to `fc.yahoo.com:443` and `query1.finance.yahoo.com:443` with HTTP
  403 (a network-policy denial, not a Yahoo answer). 8 attempts were made
  (fc, getcrumb, one 50-symbol v7 batch, one bad-crumb v7, 4 charts), all
  refused at the proxy.
- **Nothing was confirmed or refuted.** Assumptions a–h (premarket fields
  and units, `preMarketTime` in epoch seconds, the regular-market fields,
  `marketState` values, the earnings-date fields, the `quoteResponse.error`
  shape and the bad-crumb answer, whether fc.yahoo.com sets A3 directly, and
  the chart `meta`/`timestamp`/`indicators.quote[0]` shapes) are still
  unverified.
- **Nothing changed in the parser or fixtures.** The fixtures stay
  hand-written.
- `scripts/yahoo-live-check.sh OUT_DIR` repeats the check (8 requests, 2 s
  apart, the same User-Agent as `yahoo.rs`) from a network that can reach
  Yahoo. It prints statuses, Set-Cookie names and the crumb's length and
  character set, never the cookie or crumb values, and stops at the first
  429. Run it between 4:00 and 9:30 AM ET to see live premarket fields.
