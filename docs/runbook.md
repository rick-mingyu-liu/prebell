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

- A premarket build is: cookie 1, crumb 1, 11 v7 batches, 4 futures, at most 8 mover searches and at most 8 extractions, up to 3 macro searches and 1 extraction, at most 1 theme search and 1 extraction, 2-4 drafts. The facts stage stays at 40 or fewer (`BUDGET_SOFT` is 40); the whole invocation is at most 43, or 44 on the Yahoo 401-refresh path, where the guard drops the theme search to absorb the refresh. Four movers per side is the free-plan budget choice. 150-symbol Yahoo batches, which would bring back five per side, wait for a live check from another network.
- A recap build is: 4 index charts, cookie 1, crumb 1, 11 v7 batches, at most 8 mover searches and at most 8 extractions, up to 3 macro searches and 1 extraction, the after-hours search and extraction, at most 1 theme search and 1 extraction (the recap usually has no room for it), 2-4 drafts. The facts stage stays at 40 or fewer with after hours included and the theme search skipped; the whole invocation is at most 43, or 44 on the Yahoo 401-refresh path, where the guard drops the second and third macro queries to absorb the refresh. It also reads which earnings checks were published that day: 1 D1 `posts` query plus 1 R2 facts read per earnings post. Those are internal-service subrequests, so they do not show in `subrequests=`.
- An earnings poll (`job=earnings`) is 1-3 requests for EDGAR's current 8-K feed (usually 1: it reads back only to the previous poll). A build (`job=earnings:build`) is the filing index, the exhibit, up to 5 XBRL `companyconcept` documents, the extraction and 2-4 draft calls: about 11, at most about 13 with a Chinese retry. The calendar (`job=earnings:calendar`) is the Yahoo cookie, crumb and 3 v7 batches plus up to 5 announcement searches with an LLM call each: about 15.
- When a run retries, `runs.last_error` ends with `subrequests=N`: the count when the facts were built, before the drafts. Read it with `pnpm exec wrangler d1 execute prebell --remote --command "select run_key,last_error from runs order by updated_at desc limit 3"`.
- Redirect hops count toward the 50 but are not visible here, so treat 45 or more as at the limit. Past 50, fetches fail with a network error, and the build shows them as `net=` or `search_err=` in its diagnostics.

### Reading the diagnostics line

Each build logs one `diagnostics:` line as soon as its facts are built, before the drafts, so the line is there even when a build dies in them; a retrying run stores it in `runs.last_error`. Premarket, recap and earnings builds that get as far as the cover step then log a second, short line, `cover=ok|err|none`. Besides the quote, mover and search counts, these show why news was dropped:

- `hedged=` on the `reasons` part counts mover reasons dropped because `summary_en` hedged its cause ("which could support the gain"). In code the counter is `reasons_hedged`.
- `news_noise=` counts search hits dropped as noise before any LLM call: MarketBeat filing alerts, 13F, holdings and institutional-ownership pages, "trims position"-style titles, analyst consensus-rating alerts ("Has Consensus Price Target of", "Average Rating of"), and Tavily's own company-profile pages (`tavily.com/knowledge/`). Each mover and macro news query asks Tavily for `NEWS_EXTRA_HITS` (2) more hits than it keeps, so stale and noise hits do not use up a mover's slots; after filtering, the prompt still gets at most the usual number of articles. `news_stale=` and `news_noise=` count every dropped hit, including the extra ones.
- `reported=N reported_err=M` (recap only, last part) counts the earnings checks published today that the recap links as already reported. `reported_err=M` above 0 means that many D1 or R2 reads failed; the recap still publishes and lists the checks it could read.
- `macro queries=Q hits=H items=I` (premarket and recap) covers the macro section. `queries=` is how many of the three macro searches ran: 3 normally, fewer when the budget guard dropped the later ones (see "Budget guard" below), and 1 when the first search failed with HTTP 401, 403 or 429 (Tavily auth or quota), since the others would fail the same way. `hits=` is the hits left after merging the searches, dropping duplicates, stale and noise hits, and capping at 8. `items=` is the macro items that passed the quote check and are in the facts, at most 5. `items=0` means the brief has no macro section; with `hits=0` there was nothing fresh to read, with `hits=` above 0 the extraction kept nothing.
- `themes found=N kept=K searched=0|1` (premarket and recap) covers the themes section. `found=` counts the industry groups that qualified as a theme, `kept=` the ones shown after the cap of 3. `searched=1` means the top theme had no reason from its movers' news and its own search ran; `searched=0` means no theme formed, the top theme already had a reason, or the budget guard skipped the search. `found=0` means the brief has no themes section and the front page shows no themes block.
- `cover=ok|err|none` (premarket, recap and earnings builds; its own line after the cover write, once per build that publishes; never in `runs.last_error` or the stored facts) says what happened to the post's cover graphic. `ok`: the SVG was stored in R2 as `covers/<slug>.svg`. `err`: the R2 write failed; the post still publishes and shows the flat tile, and a separate `cover write failed: <error>` line follows with the R2 error (also never in `runs.last_error`). `none`: there was nothing to draw, so the post shows the flat tile. That means no non-zero mover for a brief, fewer than two revenue quarters and fewer than two diluted EPS quarters in the last 8 slots for an earnings check, or a value label that failed the number check.

### Tuning themes

A theme is an industry group from `crates/core/data/universe.txt` whose members move together. The rules are constants in `crates/core/src/themes.rs`:

| Constant | Premarket | Recap |
|---|---|---|
| `min_move`: a member counts at this move or more | 2.0% | 3.0% |
| `min_median`: the counting members' median move | 2.5% | 3.5% |
| `THEME_MIN_MEMBERS`: counting members on one side | 3 | 3 |

A group also needs the counting members to be at least half of its members that moved more than 0.5%, and a group with 3 or more counting members on both sides is no theme. Dual share classes count once. Themes rank by median times members; `THEME_MAX` keeps 3, and each theme lists up to 6 members (`THEME_LIST_MEMBERS`) and "and N more".

To see what nearly qualified, read the stored facts. Each brief's facts object is in the `prebell-facts` R2 bucket at `premarket/YYYY-MM-DD.json` or `recap/YYYY-MM-DD.json`, and its `theme_candidates` array has one entry for every group with at least two members beyond ±1.5% (`THEME_STAT_MOVE`), up to 15:

```
cd crates/generator
pnpm exec wrangler r2 object get prebell-facts/recap/2026-11-05.json --remote --pipe | jq '.theme_candidates, [.themes[] | {group, median_pct, more}]'
```

Each entry has `group`, `present` (members with a quote), `up` and `down` (members at or beyond +1.5% and −1.5%), and `median_up` / `median_down` (the median move on each side). A group with `down: 2` and `median_down: -4.0` moved hard but missed on member count; a group with `down: 4` and `median_down: -2.2` had the breadth but most likely missed on the median (the theme rules count members at 2.0% or 3.0%, not 1.5%, so the stats are a wider net). After a week of briefs, change the constants if the same near misses keep showing on days the market plainly had that story; the tests in `themes.rs` pin the values, so update them in the same commit.

### Budget guard

The facts stage keeps optional work under `BUDGET_SOFT` (40 fetch subrequests, `crates/core/src/budget.rs`), which leaves room for the drafts and the Yahoo 401-refresh path. Before each optional step the build checks that what it has used plus the step's cost is 40 or less. What goes first, in order: the top theme's search, then the third macro query, then the second. The first macro query, the quotes, the mover searches and the drafts always run; the recap also keeps room for its after-hours search. In the diagnostics the guard shows as `searched=0` with `found=` above 0 and as `macro queries=` below 3. Neither is proof on its own: `searched=0` with `found=` above 0 also happens when the top theme already has a reason from its movers' news, and `macro queries=1` also follows a 401, 403 or 429 on the first macro search (then `search_err=` is above 0). At the nominal premarket usage the theme search fits; at the nominal recap usage it is skipped and all three macro queries run.

### CPU overruns

An invocation over 10 ms of CPU is killed. In `wrangler tail` it shows with outcome `exceededCpu`. If that happens, or if `subrequests=` gets near 50, the fix is Workers Paid ($5/month): 5 minutes of CPU (30 seconds by default) and 10,000 subrequests per invocation. No code change is needed.

## Cover graphics

Each published post's cover background is an SVG chart of its own figures, drawn by the generator in the paper palette and stored in the `prebell-facts` bucket as `covers/<slug>.svg`. A brief draws its biggest movers as bars: up to 4 gainers above up to 4 losers, with a move that rounds to 0.00% skipped. An earnings check draws its revenue series as bars, or its diluted EPS series when revenue has fewer than two quarters in its last 8 slots. It shows the last 8 slots in grey. When the release has a quarterly figure, that bar is this release's quarter, drawn last in ink with its value labelled. Otherwise all bars are grey and no value is labelled. There is no AI call, no credentials and no extra subrequest, and nothing in the cover step can fail a post.

- The Workers AI scene path is gone. `CF_ACCOUNT_ID` in `crates/generator/wrangler.toml` is now unused. The `CF_AI_TOKEN` secret is unused too; the owner may delete it with `cd crates/generator && pnpm exec wrangler secret delete CF_AI_TOKEN`. Old `covers/*.png` and `covers/*.jpg` objects in R2 are harmless.
- Whether a cover was drawn shows in the `cover=ok|err|none` line (see "Reading the diagnostics line"). A post without a graphic shows the flat light tile.
- On the site, the article and front-page lead covers show the tag, both cover lines and the amber bar above the chart. Small covers (list rows and the front page's earnings checks) show the tag and the chart only; a flat tile keeps its lines.
- **Backfill for posts without cover lines:** `scripts/backfill-covers.sh` (repo root) reads the remote D1 and the `prebell-facts` bucket, recomputes each post's cover lines with the `cover-lines` binary, and writes one `UPDATE` per post and language to `backfill-covers.sql` (git-ignored). It never runs the SQL; the owner reviews it and applies it with `cd crates/generator && pnpm exec wrangler d1 execute prebell --remote --file ../../backfill-covers.sql`, and rerunning the script afterwards lists only the rows still missing lines.
- **Backfill for posts without a graphic:** `scripts/backfill-graphics.sh` (repo root) takes every published post whose `cover_key` is empty or not an `.svg`, downloads its stored facts, draws the graphic with the `cover-graphic` binary, uploads it to `prebell-facts` as `covers/<slug>.svg` (this writes to the remote R2 bucket) and writes the matching `UPDATE` statements to `backfill-graphics.sql` (git-ignored, SVGs in `backfill-graphics/`). A post whose facts give no graphic gets `cover_key = NULL`, so its old scene image stops showing under the paper styling and it shows the flat tile; the next run lists it again and writes the same statement. It never runs the SQL; the owner applies it with `cd crates/generator && pnpm exec wrangler d1 execute prebell --remote --file ../../backfill-graphics.sql`. A post whose download, render or upload failed gets no statement and shows up again on the next run.

## Web site (`prebell-web`)

The site is the OpenNext Worker in `apps/web` (`wrangler.jsonc`, name `prebell-web`). It reads the same D1 database and R2 bucket as the generator: posts and the `kv` row `index_line` (the masthead's index line) from D1, facts, charts and cover graphics from R2 (`/covers/<slug>.svg` serves the R2 object `covers/<slug>.svg`, under `Content-Security-Policy: sandbox` so an SVG can never run script; older `.png` and `.jpg` keys are served too; any other path there, or a missing object, is a 404).

- `BUTTONDOWN_API_KEY` is the site's one secret: the Buttondown API key behind the weekly-digest signup. The owner sets it with `cd apps/web && pnpm exec wrangler secret put BUTTONDOWN_API_KEY` (it lands on `prebell-web`). Until it is set, no page shows the signup form (home, post pages and About check for the key on every request) and `/api/subscribe` returns 503; the form appears once the key is set, with no redeploy. Nothing else depends on it. An address that is already subscribed gets "You're already on the list." instead of the confirmation line. A Buttondown failure logs only `buttondown subscribe failed, status <code>`, never the body or the key.
- **Domain `prebell.site`** (registered at Namecheap 2026-11-05). The site's custom domain is in `apps/web/wrangler.jsonc` (`routes`), and `SITE_URL` is `https://prebell.site`. The `--var SITE_URL:...` override on deploy is no longer needed. One-time setup (owner only):
  1. Cloudflare dashboard → Add a domain → `prebell.site` → Free plan. Delete any imported records that point at Namecheap parking.
  2. Namecheap → Domain List → `prebell.site` → Nameservers → Custom DNS: the two nameservers Cloudflare shows. Wait for the zone to show **Active**. This usually takes minutes and can take up to 24 hours.
  3. Deploy the site (step 3 below). The first deploy with the route creates the custom domain and its certificate.
  4. Nothing else for www: the deploy also makes `www.prebell.site` a custom domain, and `next.config.ts` permanently redirects (308) every www URL to the apex.
  5. Check: `curl -sI https://prebell.site/en` returns 200, and `curl -sI https://www.prebell.site/en` returns 308 to `https://prebell.site/en`. Canonical URLs in a page's `<head>` start with `https://prebell.site`.
- **Email on `prebell.site`** works the same way as TermPocket: role addresses on the domain forward to the owner's Gmail. PreBell sends no mail of its own (Buttondown sends the digest), so there is no `noreply@`. The addresses are:
  - `privacy@prebell.site`, named in the privacy policy for data requests.
  - `support@prebell.site`, named in the terms, and the generator's `SEC_CONTACT` (EDGAR User-Agent).

  The DNS is on Cloudflare, so forwarding is Cloudflare Email Routing (free), not Namecheap's. One-time setup (owner only):
  1. Cloudflare → `prebell.site` → Email → Email Routing → Get started. Delete the imported Namecheap `eforward*` MX records and their SPF TXT when asked, and let Email Routing add its own MX and SPF.
  2. Destination addresses: add the owner's Gmail and click the verification link Cloudflare emails to it.
  3. Routing rules: add custom addresses `privacy` and `support`, each sending to that Gmail. Turn on the catch-all only if wanted.
  4. Check: send a message to each address from another account and confirm it arrives.
  Do this before deploying the generator with `SEC_CONTACT = "support@prebell.site"`, so SEC can reach the address.
- **Deploy order** (owner only): first apply any new D1 migrations, then deploy the generator, then the site.
  1. `cd crates/generator && pnpm exec wrangler d1 migrations apply prebell --remote`. Migration `0006_covers.sql` adds `posts.cover_key` and `post_translations.cover_line1` / `cover_line2`; both Workers read them, so it must run before either deploys.
  2. `pnpm exec wrangler deploy` in `crates/generator`.
  3. `cd apps/web && pnpm exec opennextjs-cloudflare build && pnpm exec opennextjs-cloudflare deploy`.
- **One-off for the data-graphic covers change:** deploy the site (step 3) before the generator (step 2) this one time. The new site serves `.svg` covers and draws them on paper. A new generator deployed first would point posts at `covers/<slug>.svg`, which the old site answers with a 404. Then run the graphics backfill above. Later deploys go back to the order above.
- **Local preview:** the site's preview uses the generator's local state (`--persist-to ../../crates/generator/.wrangler/state`), so the local D1 needs the migrations too. From `crates/generator`: `pnpm exec wrangler d1 migrations apply prebell --local`, then `pnpm exec wrangler d1 execute prebell --local --file ../../db/seed-preview.sql` and `bash ../../db/load-preview-facts.sh`. Then `cd ../../apps/web && pnpm preview` (add `--port <n>` if 8787 is taken). Without migration 0006 the post queries fail with a `no such column` error.

## Earnings

- Apply the D1 migrations before the first deploy: `pnpm exec wrangler d1 migrations apply prebell --remote` (in `crates/generator`).
- Create the queue before the first deploy: `pnpm exec wrangler queues create prebell-earnings`. Earnings run only on it (one consumer, one message at a time, one retry), so EDGAR is never hit in parallel.
- On the first deploy, check one detected filing's time: compare `earnings_filings.accepted_at` (UTC) with the "Accepted" time on its `-index.htm` page (ET). They should differ by exactly the ET offset (4 h EDT, 5 h EST). The feed's time was checked once against an index page before the merge.
- **Retries:** a run whose required data is missing retries every 5 minutes until 6 hours after acceptance, or 1 hour after detection if that is later, and never gives up before its third build, so a backlog cannot fail a filing on its first try. A press release over 2 MB (filings styled inline, such as Duke's and Simon's) is read up to its first 2 MB instead of failing.
- `SEC_CONTACT` must be a real contact email: SEC blocks anonymous clients. With the placeholder, every earnings step is a config error with one `error` alert per ET day, keyed `config:earnings:<date>`.
- SEC answering 403/429 stops that poll and raises one `error` alert per ET day (`sec-throttled:<date>`); the next firing tries again.
- `EARNINGS_CALENDAR_SEED = "off"` stops reading Yahoo for calendar dates (for example while Yahoo throttles; a 429 shows as `yahoo_429=N` in the `job=earnings:calendar` log line). Company announcements and EDGAR filings still update the calendar.

## Logos

Company logos (the white tag on earnings-check covers) and partner logos are static files in `apps/web/public/logos/`, served as Workers static assets. Pages never fetch a logo from a third party. `apps/web/scripts/build-logos.mjs` (`pnpm logos` in `apps/web`) builds every file, the manifest `apps/web/src/lib/logos.generated.ts` and `apps/web/public/logos/LICENSES.md`. Never edit those by hand. For each watchlist ticker the script takes the first of:

1. **Simple Icons** (`COMPANIES` in the script): a CC0 glyph from the pinned `simple-icons` devDependency, drawn as a 32px SVG tile.
2. **Wikimedia Commons via Wikidata:** the script finds the company's Wikidata item by the SEC CIK in `watchlist.tsv` (or `WIKIDATA` in the script) and takes its P8972 "small logo", else its P154 "logo". Files in `COMMONS` in the script go first. Only files Commons lists as public domain, CC0, CC BY or CC BY-SA are used. A logo wider than 2.2:1 loses to a site icon, and one wider than 4.5:1 is never used.
3. **The company's own site icon:** the largest `apple-touch-icon` / `icon` / `favicon.ico` image of at least 48px linked from the homepage (the domains in `watchlist.tsv`, then Wikidata's P856).

Tiers 2 and 3 become 64px PNG tiles: a full-bleed app icon fills the tile, and anything else sits on a white tile (near-black when the mark is too pale for white). `SKIP` in the script keeps a ticker as a text chip on purpose. Partner marks (`PARTNERS`) are square tiles shown before the partner's name: OKX from Simple Icons, the others from each partner's own site or newsroom.

- **Run it:** downloads go out one per second with the User-Agent `PreBell logo build (https://prebell.site)`. Definitive answers (2xx, and 4xx other than 408/429) are cached in `apps/web/node_modules/.cache/logos`, so a rerun gives the same files; `pnpm logos --offline` uses only the cache. Delete the cache to fetch everything again (a full run takes about 8 minutes). The script prints each tier's tickers, the ones left without a logo, and why a Commons file was skipped. Before committing, look at every new or changed tile at 18px on the white cover tag.
- **When it fails:** the script builds in `apps/web/node_modules/.cache/logos-build` and moves the result into `public/logos/` and `src/lib/logos.generated.ts` only when the whole run succeeds. A failed run leaves the committed files as they were. It stops with an error when Wikidata, Commons or a partner file doesn't answer (a timeout, 429 or 5xx is never cached, so just run it again later). It also stops when a ticker that has a logo in the committed manifest would lose it; pass `--allow-shrink` only if that is intended. Company sites that fail the same way are reported at the end and retried on the next run.
- **Browser caching:** `apps/web/public/_headers` serves `/logos/*` with a one-year `immutable` Cache-Control, so a tile replaced under the same file name may keep showing the old image to returning visitors for up to a year.
- **Add or fix a company logo:** if Simple Icons has the brand, add `TICKER: "slug"` to `COMPANIES`. Otherwise, if Wikidata picks a wordmark that is too wide, find the company's own square mark on Commons and add `TICKER: "File name.svg"` to `COMMONS` (the licence check still applies). If the result is unreadable at 18px, add the ticker to `SKIP` with the reason. Use only the company's own current mark.
- **Refresh all logos:** bump `simple-icons` (exact version) in `apps/web/package.json`, set `FETCHED` in the script to that day, delete the cache and run `pnpm logos`. If a brand was removed upstream (usually because the brand asked), the script stops with `simple-icons X has no "slug"`. Remove that ticker from `COMPANIES` too.
- **Partner logos:** set `url` (the partner's own file) and `page` (where it is linked) in `PARTNERS`, run `pnpm logos`, then set `logo` on the partner in `apps/web/src/lib/partners.ts`. The partner strip shows names only, so a logo is kept on disk and in the `/api/partners` response but not drawn; `logoDark` is optional.
- Without a logo, a cover's tag shows the ticker as text. Nothing breaks if a file is missing; the tests check that every manifest entry exists, and that every file has a `LICENSES.md` row.
