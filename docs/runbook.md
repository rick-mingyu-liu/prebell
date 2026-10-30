# PreBell runbook

Operational notes for the owner. Plan 1 Task 13 adds the deploy, cron and rerun sections to this file.

## LLM provider

Every LLM call goes through one provider: reason extraction while building facts, and the EN/ZH drafts. The generator Worker picks the provider from config when each queue batch starts. Claude is the primary provider and the default; DeepSeek is the fallback.

| `LLM_PROVIDER` | API | Secret | Default model |
|---|---|---|---|
| `claude` (default) | `https://api.anthropic.com/v1/messages` | `ANTHROPIC_API_KEY` | `claude-sonnet-5-5` |
| `deepseek` (fallback) | `https://api.deepseek.com/chat/completions` | `DEEPSEEK_API_KEY` | `deepseek-chat` |

- `LLM_PROVIDER`, the optional `LLM_MODEL` and `LLM_EFFORT` are plain vars in `crates/generator/wrangler.toml` under `[vars]`. If `LLM_MODEL` is unset or empty, the provider's default model is used. If `LLM_PROVIDER` is unset, it is `claude`.
- `LLM_EFFORT` (Claude only) sets `output_config.effort` on every request: `low` (default), `medium` or `high`. `none` leaves the field out; use it for a model that does not support effort. Any other value is a config error: see the next point. DeepSeek ignores it.
- The secrets are set only with `wrangler secret put` (production) or in `crates/generator/.dev.vars` (local, git-ignored). They are never committed.
- An unknown `LLM_PROVIDER` or `LLM_EFFORT` is a config error. The premarket and recap jobs then never run, and each one raises one `error` alert per ET day, keyed `config:<job>:<date>`. The alert reads like `premarket: config error: parse error: unknown LLM_EFFORT: hgih (use low, medium, high or none)`. It names the variable and its value, never a key. The competitor job does not use the LLM and keeps running. Fix the var and deploy.
- The number check (`numbers::check`) and quote anchoring run the same way whichever provider is used. Switching provider does not loosen them.

## Claude (default)

### Set up (owner only; this deploys)

1. `cd crates/generator && pnpm exec wrangler secret put ANTHROPIC_API_KEY` and paste the key.
2. In `crates/generator/wrangler.toml`, `LLM_PROVIDER = "claude"` and `LLM_EFFORT = "low"` are already set. To use a different model, add `LLM_MODEL = "<model id>"`, for example `claude-opus-5-5`. If that model does not support effort, also set `LLM_EFFORT = "none"`.
3. Deploy the generator (`pnpm exec wrangler deploy` in `crates/generator`). Vars take effect on deploy.
4. Check the next run (and time it: compare `updated_at` with 08:30 ET): `pnpm exec wrangler d1 execute prebell --remote --command "select run_key,status,attempts,last_error from runs order by updated_at desc limit 3"`.

Claude thinks before answering, so a build (up to 11 extraction calls plus the EN and ZH drafts) may take longer than on DeepSeek. Effort `low` keeps that short. A build older than 15 minutes counts as stale and a second one starts. If the first runs take more than about 10 minutes, tell the developer: a faster `LLM_MODEL` is the option. Raise `LLM_EFFORT` to `medium` or `high` only if draft quality needs it.

### Local dev

Put the key in `crates/generator/.dev.vars`:

```
LLM_PROVIDER=claude
ANTHROPIC_API_KEY=sk-ant-...
```

Values in `.dev.vars` override `[vars]` under `wrangler dev`. Then follow the local steps in `docs/plan-1-notes.md`. A missing key logs `secret ANTHROPIC_API_KEY is not set; provider calls will fail`.

## Fall back to DeepSeek

Use this if Claude is down, the spend limit is reached or the key is revoked, and the run cannot wait.

1. `cd crates/generator && pnpm exec wrangler secret put DEEPSEEK_API_KEY` (skip if it is still set).
2. In `crates/generator/wrangler.toml`, set `LLM_PROVIDER = "deepseek"` and remove `LLM_MODEL` if you set it to a Claude model. `LLM_EFFORT` can stay; DeepSeek ignores it.
3. Deploy the generator (`pnpm exec wrangler deploy` in `crates/generator`).

Both secrets can stay set the whole time. To go back, set `LLM_PROVIDER = "claude"` and deploy.

### What Claude failures look like

Where a failure shows up depends on which call failed.

- **Reason extraction** (the first calls in a build): errors are not recorded. If every extraction fails — wrong key, rate limit, outage, or refusals — no mover keeps a reason, the required movers section is missing, and `runs.last_error` reads just `movers`. That is the same thing you see when the news has no usable reasons, so use `pnpm exec wrangler tail` in `crates/generator` to see the HTTP status. The run stays `pending` and retries every 5 minutes until 09:25 ET, then it is skipped and an error alert is raised.
- **EN draft:** each draft gets two attempts per tick. If both fail, `last_error` reads `en: llm: <error>` and the run retries on later ticks until 09:25 ET as above.
- **ZH draft:** EN still publishes. ZH is stored as `pending` ("中文版稍后推出") and `last_error` reads `zh: llm: <error>`. Later firings retry ZH, hourly after the morning window. A failure at or after 18:00 ET marks ZH `failed` and raises one warn alert.

Every HTTP error of 400 or above also writes one line to `wrangler tail` with the error type and request id, never the key or the body: `claude http <status>: error.type=<type> request_id=<id>`. Quote the `request_id` to Anthropic support.

The `<error>` part:

| Error | Meaning |
|---|---|
| `http status 400` | Bad request, or the organization or workspace spend limit was reached (the API returns 400 for both). `wrangler tail` shows `claude http 400: error.type=invalid_request_error request_id=req_...`; the message is not logged, so check the Claude Console usage and spend limit first, then the request (`LLM_MODEL`, `LLM_EFFORT`). If the limit is the cause, raise it or fall back to DeepSeek. |
| `http status 401` | Wrong or missing `ANTHROPIC_API_KEY`. |
| `http status 402` | `billing_error`: a billing or payment problem. Fix it in the Claude Console. |
| `http status 403` | `permission_error`: the key has no access to this model or workspace. Check the key's workspace and the organization's model access in the Claude Console. |
| `http status 404` | `not_found_error`: almost always a bad `LLM_MODEL` id. Fix or remove `LLM_MODEL` and redeploy. |
| `http status 429` | Rate limit. |
| `http status 529` / `5xx` | Anthropic overloaded or unavailable; usually clears on a later tick. |
| `parse error: truncated` | The reply hit `max_tokens` (16000) or the model's context window (`model_context_window_exceeded`). It is never used as partial JSON. |
| `parse error: refusal: <category>` | The model declined the request. Facts are only saved after a successful EN draft, so a refused build leaves nothing in R2 to inspect; tell the developer the date and category. |
| `parse error: stop_reason: <x>` | The turn ended unfinished in some other way. It is never used. |
| `parse error: ...` (other) | The reply was not a JSON object. |

## Free plan

The generator runs on the Workers Free plan for now. The limits that matter:

| Limit (Free) | Value | Where it bites |
|---|---|---|
| Cron triggers per account | 5 | `crates/generator/wrangler.toml` uses 3 (a test in `schedule.rs` keeps it at 3). |
| CPU time per invocation | 10 ms | Each queue message is its own invocation (`max_batch_size = 1`). Waiting on the network is not CPU time; JSON parsing and drafting checks are. |
| Fetch subrequests per invocation | 50 | Yahoo, Tavily, the LLM, EDGAR, the competitor feed and the alert webhook. |
| Subrequests to internal services | 1,000 | D1 and R2 binding calls. The [limits page](https://developers.cloudflare.com/workers/platform/limits/) counts calls to R2, KV and D1 as subrequests and gives Free a separate 1,000 row for internal services, so they are not counted in `subrequests=`. |
| Queue operations per day | 10,000 | About 650 cron messages on a weekday (premarket, recap and earnings every 5 minutes) plus earnings builds, at about 3 operations each. |

### Reading `subrequests=`

After each queue message, `pnpm exec wrangler tail` in `crates/generator` shows one line like `subrequests=27 job=premarket`. Alert delivery logs its own line, `subrequests=N job=alerts`. Add the two lines to get the invocation's total. Delivery posts at most 45 minus what the job used; the other alerts stay unsent for the next invocation. Most firings show `subrequests=0`: the clock decides nothing is due, and only D1 is read.

- A premarket build is about 1 Yahoo cookie, 1 crumb, 11 v7 batches and 4 futures charts, plus 1 Tavily search and 1 LLM call per mover, 1 macro search and its LLM call, and 2-4 draft calls.
- A recap build is 4 index charts, the cookie, the crumb and 11 v7 batches, plus the same news and LLM work.
- An earnings poll (`job=earnings`) is 1-3 requests for EDGAR's current 8-K feed (usually 1: it reads back only to the previous poll). A build (`job=earnings:build`) is the filing index, the exhibit, up to 5 XBRL `companyconcept` documents, the extraction and 2-4 draft calls: about 12, at most about 14 with a Chinese retry. The calendar (`job=earnings:calendar`) is the Yahoo cookie, crumb and 3 v7 batches plus up to 5 announcement searches with an LLM call each: about 15.
- When a run retries, `runs.last_error` ends with `subrequests=N`: the count when the facts were built, before the drafts. Read it with `pnpm exec wrangler d1 execute prebell --remote --command "select run_key,last_error from runs order by updated_at desc limit 3"`.
- Redirect hops count toward the 50 but are not visible here, so treat 45 or more as at the limit. Past 50, fetches fail with a network error, and the build shows them as `net=` or `search_err=` in its diagnostics.

### CPU overruns

An invocation over 10 ms of CPU is killed. In `wrangler tail` it shows with outcome `exceededCpu`. If that happens, or if `subrequests=` gets near 50, the fix is Workers Paid ($5/month): 5 minutes of CPU (30 seconds by default) and 10,000 subrequests per invocation. No code change is needed.

## Earnings

- Before the first deploy, apply the D1 migrations: `pnpm exec wrangler d1 migrations apply prebell --remote` (in `crates/generator`). It applies `0003_earnings.sql` even though `0004_competitor.sql` is already applied: D1 tracks each migration by name.
- Create the queue before the first deploy: `pnpm exec wrangler queues create prebell-earnings`. Earnings run only on it (one consumer, one message at a time, one retry), so EDGAR is never hit in parallel.
- On the first deploy, check one detected filing's time: compare `earnings_filings.accepted_at` (UTC) with the "Accepted" time on its `-index.htm` page (ET). They should differ by exactly the ET offset (4 h EDT, 5 h EST). The feed's time was checked once against an index page before the merge.
- `SEC_CONTACT` must be a real contact email: SEC blocks anonymous clients. With the placeholder, every earnings step is a config error with one `error` alert per ET day, keyed `config:earnings:<date>`.
- SEC answering 403/429 stops that poll and raises one `error` alert per ET day (`sec-throttled:<date>`); the next firing tries again.
- `EARNINGS_CALENDAR_SEED = "off"` stops reading Yahoo for calendar dates (for example while Yahoo throttles; a 429 shows as `yahoo_429=N` in the `job=earnings:calendar` log line). Company announcements and EDGAR filings still update the calendar.

## Logos

Company logos (earnings strip, calendar chips, movers rows) and partner logos (sidebar) are static SVGs in `apps/web/public/logos/`, served as Workers static assets. Pages never fetch a logo from a third party. The files are generated from the pinned `simple-icons` devDependency (CC0 icon data) by `apps/web/scripts/build-logos.mjs`. The same script writes the manifest `apps/web/src/lib/logos.generated.ts` and `apps/web/public/logos/LICENSES.md`. Never edit those three by hand.

- **Add a company logo:** find the brand's slug at simpleicons.org. Add `TICKER: "slug"` to `COMPANIES` in the script (the ticker must be in `crates/core/data/watchlist.tsv`). Run `pnpm logos` in `apps/web`, then look at the tile at 18px in light and dark. Commit the SVG, manifest and `LICENSES.md` together. Use only the company's own mark, and check its brand guidelines link in `LICENSES.md`.
- **Refresh all logos:** bump `simple-icons` (exact version) in `apps/web/package.json`. Set `FETCHED` in the script to the day you do it, then run `pnpm logos`. If a brand was removed upstream (usually because the brand asked), the script stops with `simple-icons X has no "slug"`. Remove that ticker from `COMPANIES` too, so its chip goes back to text.
- **A logo from somewhere else** (an official press kit, for example): Simple Icons doesn't cover it, so the script can't build it. Put the optimized SVG in `public/logos/companies/{TICKER}.svg` on a 32px rounded tile like the generated ones, and add the ticker to the manifest. Also add a row to `LICENSES.md` with the URL, the date and the terms. The script deletes and rebuilds both folders and the manifest, so first add the hand-made file to the script (copied in, not generated).
- **Partner logos** (`PARTNERS` in the script) are cropped wordmarks: `{id}.svg` is black for the light sidebar and `{id}-dark.svg` is white for the dark one. Set the paths as `logo` / `logoDark` on the partner in `apps/web/src/lib/partners.ts`. A partner without `logo` shows its name as text.
- Without a logo, a ticker shows the plain text chip, and in movers rows a grey lettered tile. Nothing breaks if a file is missing; the tests check that every manifest entry exists.
