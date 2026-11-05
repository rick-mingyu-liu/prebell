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
- An unknown `LLM_PROVIDER` or `LLM_EFFORT` is a config error. The premarket and recap jobs then never run, and each one raises one `error` alert per ET day, keyed `config:<job>:<date>`. The alert reads like `premarket: config error: parse error: unknown LLM_EFFORT: hgih (use low, medium, high or none)`. It names the variable and its value, never a key. Fix the var and deploy.
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
| Cron triggers per account | 5 | `crates/generator/wrangler.toml` uses 2 (a test in `schedule.rs` keeps it at 2). |
| CPU time per invocation | 10 ms | Each queue message is its own invocation (`max_batch_size = 1`). Waiting on the network is not CPU time; JSON parsing and drafting checks are. |
| Fetch subrequests per invocation | 50 | Yahoo, Tavily, the LLM, EDGAR and the alert webhook. |
| Subrequests to internal services | 1,000 | D1 and R2 binding calls. The [limits page](https://developers.cloudflare.com/workers/platform/limits/) counts calls to R2, KV and D1 as subrequests and gives Free a separate 1,000 row for internal services, so they are not counted in `subrequests=`. |
| Queue operations per day | 10,000 | About 650 cron messages on a weekday (premarket, recap and earnings every 5 minutes) plus earnings builds, at about 3 operations each. |

### Reading `subrequests=`

After each queue message, `pnpm exec wrangler tail` in `crates/generator` shows one line like `subrequests=27 job=premarket`. Alert delivery logs its own line, `subrequests=N job=alerts`. Add the two lines to get the invocation's total. Delivery posts at most 45 minus what the job used; the other alerts stay unsent for the next invocation. Most firings show `subrequests=0`: the clock decides nothing is due, and only D1 is read.

- A premarket build is about 1 Yahoo cookie, 1 crumb, 11 v7 batches and 4 futures charts, plus 1 Tavily search and 1 LLM call per mover, 1 macro search and its LLM call, and 2-4 draft calls.
- A recap build is 4 index charts, the cookie, the crumb and 11 v7 batches, plus the same news and LLM work. It also reads which earnings checks were published that day: 1 D1 `posts` query plus 1 R2 facts read per earnings post. Those are internal-service subrequests, so they do not show in `subrequests=`.
- An earnings poll (`job=earnings`) is 1-3 requests for EDGAR's current 8-K feed (usually 1: it reads back only to the previous poll). A build (`job=earnings:build`) is the filing index, the exhibit, up to 5 XBRL `companyconcept` documents, the extraction and 2-4 draft calls: about 12, at most about 14 with a Chinese retry. The calendar (`job=earnings:calendar`) is the Yahoo cookie, crumb and 3 v7 batches plus up to 5 announcement searches with an LLM call each: about 15.
- When a run retries, `runs.last_error` ends with `subrequests=N`: the count when the facts were built, before the drafts. Read it with `pnpm exec wrangler d1 execute prebell --remote --command "select run_key,last_error from runs order by updated_at desc limit 3"`.
- Redirect hops count toward the 50 but are not visible here, so treat 45 or more as at the limit. Past 50, fetches fail with a network error, and the build shows them as `net=` or `search_err=` in its diagnostics.

### Reading the diagnostics line

Each build logs one `diagnostics:` line as soon as its facts are built, before the drafts, so the line is there even when a build dies in them; a retrying run stores it in `runs.last_error`. Premarket and recap builds that get as far as the scene step then log a second, short line, `cover=ok|skip|err`. Besides the quote, mover and search counts, these show why news was dropped:

- `hedged=` on the `reasons` part counts mover reasons dropped because `summary_en` hedged its cause ("which could support the gain"). In code the counter is `reasons_hedged`.
- `news_noise=` counts search hits dropped as noise before any LLM call: MarketBeat filing alerts, 13F, holdings and institutional-ownership pages, "trims position"-style titles, analyst consensus-rating alerts ("Has Consensus Price Target of", "Average Rating of"), and Tavily's own company-profile pages (`tavily.com/knowledge/`). Each mover and macro news query asks Tavily for `NEWS_EXTRA_HITS` (2) more hits than it keeps, so stale and noise hits do not use up a mover's slots; after filtering, the prompt still gets at most the usual number of articles. `news_stale=` and `news_noise=` count every dropped hit, including the extra ones.
- `reported=N reported_err=M` (recap only, last part) counts the earnings checks published today that the recap links as already reported. `reported_err=M` above 0 means that many D1 or R2 reads failed; the recap still publishes and lists the checks it could read.
- `cover=ok|skip|err` (premarket and recap only, its own line after the scene step; never in `runs.last_error` or the stored facts) says what happened to the post's cover scene. `ok`: the Workers AI image was stored in R2 under `covers/<slug>.<ext>`. `skip`: no scene maker is configured (`CF_AI_TOKEN` or `CF_ACCOUNT_ID` is missing), so the post has the plain cover. `err`: the scene call failed, the image was neither PNG nor JPEG, or the R2 write failed; the post still publishes with the plain cover. When the scene call itself failed, `wrangler tail` also shows `cover: <message>`, for example the API's quota or auth error. Earnings posts get covers the same way but log no `cover=` line. Scene prompts never name the company: an earnings scene is one of eight generic, people-free and text-free settings picked by a stable hash of the ticker, and a brief scene rotates by weekday through five such settings.

### CPU overruns

An invocation over 10 ms of CPU is killed. In `wrangler tail` it shows with outcome `exceededCpu`. If that happens, or if `subrequests=` gets near 50, the fix is Workers Paid ($5/month): 5 minutes of CPU (30 seconds by default) and 10,000 subrequests per invocation. No code change is needed.

## Cover images (Workers AI)

Each published post gets a generated background scene (Flux Schnell on Workers AI, one request per post, no retry). Without the credentials below, posts publish with the plain cover and the log says `cover=skip`.

- `CF_ACCOUNT_ID` is a plain var in `crates/generator/wrangler.toml` under `[vars]`. It is the Cloudflare account id, not a secret.
- `CF_AI_TOKEN` is a secret: a Cloudflare API token with the Workers AI "Read" permission. The owner sets it with `cd crates/generator && pnpm exec wrangler secret put CF_AI_TOKEN` (production) or in `crates/generator/.dev.vars` (local). It is never committed and never appears in an error or log line.
- Both take effect on the next deploy. A missing token or account id logs `secret CF_AI_TOKEN or var CF_ACCOUNT_ID is not set; covers will be plain` once per queue message.
- A failed scene is logged as `cover: workers ai: <message>` (or `workers ai http <status>` when the body has no message) and never fails the post. One scene is one fetch subrequest: it counts toward the 50-subrequest limit but is not included in the logged `subrequests=`, which is read before the scene is made.

## Web site (`prebell-web`)

The site is the OpenNext Worker in `apps/web` (`wrangler.jsonc`, name `prebell-web`). It reads the same D1 database and R2 bucket as the generator: posts and the `kv` row `index_line` (the masthead's index line) from D1, facts, charts and cover scenes from R2 (`/covers/<slug>.png` serves the R2 object `covers/<slug>.png`; any other path there, or a missing object, is a 404).

- `BUTTONDOWN_API_KEY` is the site's one secret: the Buttondown API key behind the weekly-digest signup. The owner sets it with `cd apps/web && pnpm exec wrangler secret put BUTTONDOWN_API_KEY` (it lands on `prebell-web`). Until it is set, no page shows the signup form (home, post pages and About check for the key on every request) and `/api/subscribe` returns 503; the form appears once the key is set, with no redeploy. Nothing else depends on it. An address that is already subscribed gets "You're already on the list." instead of the confirmation line. A Buttondown failure logs only `buttondown subscribe failed, status <code>`, never the body or the key.
- **Deploy order** (owner only): first apply any new D1 migrations, then deploy the generator, then the site.
  1. `cd crates/generator && pnpm exec wrangler d1 migrations apply prebell --remote`. Migration `0006_covers.sql` adds `posts.cover_key` and `post_translations.cover_line1` / `cover_line2`; both Workers read them, so it must run before either deploys.
  2. `pnpm exec wrangler deploy` in `crates/generator`.
  3. `cd apps/web && pnpm exec opennextjs-cloudflare build && pnpm exec opennextjs-cloudflare deploy`.
- **Local preview:** the site's preview uses the generator's local state (`--persist-to ../../crates/generator/.wrangler/state`), so the local D1 needs the migrations too. From `crates/generator`: `pnpm exec wrangler d1 migrations apply prebell --local`, then `pnpm exec wrangler d1 execute prebell --local --file ../../db/seed-preview.sql` and `bash ../../db/load-preview-facts.sh`. Then `cd ../../apps/web && pnpm preview` (add `--port <n>` if 8787 is taken). Without migration 0006 the post queries fail with a `no such column` error.

## Earnings

- Apply the D1 migrations before the first deploy: `pnpm exec wrangler d1 migrations apply prebell --remote` (in `crates/generator`).
- Create the queue before the first deploy: `pnpm exec wrangler queues create prebell-earnings`. Earnings run only on it (one consumer, one message at a time, one retry), so EDGAR is never hit in parallel.
- On the first deploy, check one detected filing's time: compare `earnings_filings.accepted_at` (UTC) with the "Accepted" time on its `-index.htm` page (ET). They should differ by exactly the ET offset (4 h EDT, 5 h EST). The feed's time was checked once against an index page before the merge.
- `SEC_CONTACT` must be a real contact email: SEC blocks anonymous clients. With the placeholder, every earnings step is a config error with one `error` alert per ET day, keyed `config:earnings:<date>`.
- SEC answering 403/429 stops that poll and raises one `error` alert per ET day (`sec-throttled:<date>`); the next firing tries again.
- `EARNINGS_CALENDAR_SEED = "off"` stops reading Yahoo for calendar dates (for example while Yahoo throttles; a 429 shows as `yahoo_429=N` in the `job=earnings:calendar` log line). Company announcements and EDGAR filings still update the calendar.

## Logos

Company logos (the white tag on earnings-check covers) and partner logos are static files in `apps/web/public/logos/`, served as Workers static assets. Pages never fetch a logo from a third party. `apps/web/scripts/build-logos.mjs` (`pnpm logos` in `apps/web`) builds every file, the manifest `apps/web/src/lib/logos.generated.ts` and `apps/web/public/logos/LICENSES.md`. Never edit those by hand. For each watchlist ticker the script takes the first of:

1. **Simple Icons** (`COMPANIES` in the script): a CC0 glyph from the pinned `simple-icons` devDependency, drawn as a 32px SVG tile.
2. **Wikimedia Commons via Wikidata:** the script finds the company's Wikidata item by the SEC CIK in `watchlist.tsv` (or `WIKIDATA` in the script) and takes its P8972 "small logo", else its P154 "logo". Files in `COMMONS` in the script go first. Only files Commons lists as public domain, CC0, CC BY or CC BY-SA are used. A logo wider than 2.2:1 loses to a site icon, and one wider than 4.5:1 is never used.
3. **The company's own site icon:** the largest `apple-touch-icon` / `icon` / `favicon.ico` image of at least 48px linked from the homepage (the domains in `watchlist.tsv`, then Wikidata's P856).

Tiers 2 and 3 become 64px PNG tiles: a full-bleed app icon fills the tile, and anything else sits on a white tile (near-black when the mark is too pale for white). `SKIP` in the script keeps a ticker as a text chip on purpose. Partner marks (`PARTNERS`) are square tiles shown before the partner's name: OKX from Simple Icons, the others from each partner's own site or newsroom.

- **Run it:** downloads go out one per second with the User-Agent `PreBell logo build (https://prebell.ai)`. Definitive answers (2xx, and 4xx other than 408/429) are cached in `apps/web/node_modules/.cache/logos`, so a rerun gives the same files; `pnpm logos --offline` uses only the cache. Delete the cache to fetch everything again (a full run takes about 8 minutes). The script prints each tier's tickers, the ones left without a logo, and why a Commons file was skipped. Before committing, look at every new or changed tile at 18px on the white cover tag.
- **When it fails:** the script builds in `apps/web/node_modules/.cache/logos-build` and moves the result into `public/logos/` and `src/lib/logos.generated.ts` only when the whole run succeeds. A failed run leaves the committed files as they were. It stops with an error when Wikidata, Commons or a partner file doesn't answer (a timeout, 429 or 5xx is never cached, so just run it again later). It also stops when a ticker that has a logo in the committed manifest would lose it; pass `--allow-shrink` only if that is intended. Company sites that fail the same way are reported at the end and retried on the next run.
- **Browser caching:** `apps/web/public/_headers` serves `/logos/*` with a one-year `immutable` Cache-Control, so a tile replaced under the same file name may keep showing the old image to returning visitors for up to a year.
- **Add or fix a company logo:** if Simple Icons has the brand, add `TICKER: "slug"` to `COMPANIES`. Otherwise, if Wikidata picks a wordmark that is too wide, find the company's own square mark on Commons and add `TICKER: "File name.svg"` to `COMMONS` (the licence check still applies). If the result is unreadable at 18px, add the ticker to `SKIP` with the reason. Use only the company's own current mark.
- **Refresh all logos:** bump `simple-icons` (exact version) in `apps/web/package.json`, set `FETCHED` in the script to that day, delete the cache and run `pnpm logos`. If a brand was removed upstream (usually because the brand asked), the script stops with `simple-icons X has no "slug"`. Remove that ticker from `COMPANIES` too.
- **Partner logos:** set `url` (the partner's own file) and `page` (where it is linked) in `PARTNERS`, run `pnpm logos`, then set `logo` on the partner in `apps/web/src/lib/partners.ts`. The partner strip shows names only, so a logo is kept on disk and in the `/api/partners` response but not drawn; `logoDark` is optional.
- Without a logo, a cover's tag shows the ticker as text. Nothing breaks if a file is missing; the tests check that every manifest entry exists, and that every file has a `LICENSES.md` row.
