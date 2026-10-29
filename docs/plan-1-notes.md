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
