# PreBell MCP server: design

Date: 2026-11-10. Status: approved in conversation by Rick (see "What Rick decided").

## Intent

Let a person's own AI agent read PreBell. The first user is Rick with his own agent; the picture he gave: in the morning he asks the agent which companies are about to report earnings, and how premarket is moving.

Why this and not more of the site: for a human reader the English brief is a commodity, but an agent is the one reader for whom PreBell's checking matters. A figure that arrives with the sentence it came from and its SEC filing link is something an agent can cite. So the server is earnings-first.

Why now: the data is already structured and stored for the site, so this is a thin read-only layer, about a day of work.

## What Rick decided

- Scope is "Small, earnings-first": the earnings calendar, the earnings checks with their sources, and the latest brief as a time-stamped snapshot. No live quotes. (Chosen 2026-11-10 from three options; the others were to park MCP, and a full version with live premarket quotes.)
- The premarket limit is accepted: the brief exists only from 8:00 ET and is rebuilt at 8:35 ET. The server serves that snapshot with its time on it and never fetches a fresh quote.
- The defaults in "Design" below were shown to him as my calls and he answered "ok".
- He did not say which agent he will connect first. The server is built to the MCP standard and checked with a standard client; Muse and Grok's bot are unverified.

## Goals

1. An agent can answer "who reports earnings on these days?" for any day or week on the calendar, optionally for a list of tickers.
2. An agent can answer "has this company reported, and what were the numbers?" with each figure's source sentence and the filing link.
3. An agent can read the latest premarket brief or after-close recap, and can tell from the answer how old it is and whether today's is out yet.
4. Every answer says where it came from and when, so an agent can cite it and cannot mistake a snapshot for a live quote.

## Non-goals

- Live or on-demand quotes, or any data the site does not already store.
- Writing anything: no subscriptions, alerts, watchlists or accounts.
- Sign-in or tokens. Everything served is already public on prebell.site.
- A database migration, a new Worker, or a new paid feature. Free plan only.
- A listing in an MCP registry or directory. That is a later step and Rick's to do.
- Estimates, beat/miss or advice of the server's own. It adds nothing to what was published. A published mover reason may itself quote a news article's wording about expectations; that passes through unchanged, as on the page.

## Anti-goals (a failure even if it works)

- An agent presents an 8:35 snapshot as the current price. Every brief answer carries `as_of` and a plain sentence saying it is a snapshot.
- The server can be used to exhaust the site's free-plan quota and take prebell.site down.
- A response carries more of the Yahoo-derived data than the public page shows (for example the full quote table behind the movers).
- A tool returns a number that the page would not show, or returns text that bypassed the site's checks. The server reads only published posts and their stored fact bundles.

## Design

### Where it lives (my call)

One route in the existing site Worker (`apps/web`, Next.js on Cloudflare via OpenNext): `POST https://prebell.site/mcp`. It reuses the site's data layer (`apps/web/src/lib/earnings.ts`, `events.ts`, `posts.ts`, `facts.ts`, `brief.ts`, `db.ts`) and its D1 (`DB`) and R2 (`FACTS`) bindings. No new service.

### Protocol (my call; the builder confirms details against the current MCP specification before writing code)

- MCP over Streamable HTTP, stateless: each POST carries one JSON-RPC message and gets one `application/json` response. No sessions, no server-sent events, no server-initiated messages.
- Methods: `initialize`, `ping`, `tools/list`, `tools/call`, and the `notifications/initialized` notification (accepted, 202, no body). Anything else gets JSON-RPC "method not found".
- `GET /mcp` returns a short plain-text description of the server and how to add it to a client (so a person who opens the address in a browser sees what it is). No site navigation or footer wording changes in v1.
- Whether to hand-write this small handler or use the official TypeScript SDK is the builder's choice, decided by what runs cleanly in a Next.js route handler on Workers with no new heavy dependency. TermPocket's read-only MCP server (`~/Documents/MindGo/backend`, see its `test/mcp.test.ts`) is prior art for the hand-written shape.
- Protocol version, capability shape, tool annotations (read-only hints), structured output (`structuredContent` with an output schema) and the recommended `Origin` handling for a public remote server all come from the current specification, not from memory.

### Tools (all read-only)

Names and fields are a starting point; the builder may adjust names to the site's vocabulary but not the meaning.

1. `earnings_calendar` — who reports and when.
   - Input: `from` and `to` (ET dates, optional; default the current week, Monday to Friday; at most 15 trading days), `tickers` (optional list, at most 50), `lang` (`en` | `zh`, default `en`).
   - Output per day: companies with ticker, name, date, slot (`before_open` | `after_close` | `unknown`), status (`unconfirmed` | `confirmed` | `reported`), whether PreBell writes a check for it, the check's URL when published, and the SEC filing link when the calendar has one; plus that day's economy events (Fed decision, jobs, CPI, elections, closures) with status and their official link.
   - Meaning of the statuses is stated in the tool description, as on the calendar page: unconfirmed means the date comes from an aggregator.
2. `earnings_report` — one company's latest results.
   - Input: `ticker`, `lang`.
   - Output when PreBell has a published check: company, fiscal period, filing date, title, summary, key figures (metric, value, unit, the amount as the company wrote it, and the verbatim source sentence), guidance and highlight notes with their quotes, the share move when the facts hold one (with its session and time), the check's URL, the filing and press-release links, and `published_at`. The full article text is returned only when `full_text` is true.
   - Output otherwise: whether the company has reported and its filing link if so, or its next calendar date; and a plain statement that PreBell writes no check for this company (calendar-only tier) or has none yet.
   - An unknown ticker is a normal tool result saying so, not a protocol error.
3. `latest_brief` — the newest premarket brief or after-close recap.
   - Input: `kind` (`premarket` | `recap`, optional; default whichever is newest), `lang`.
   - Output: title, summary, `published_at`, `updated_at`, `as_of` (the facts' own time), the index or futures lines, the movers exactly as the page shows them (ticker, name, percent move, the reason with its source link, or the code-written fallback), the themes, the "what to watch" list, and the post URL; plus `is_today` and, when today's brief is not out, a sentence saying so and when the next one is due (8:00 ET on a trading day).
   - Never more market data than the published page shows.

Every result carries: `as_of` or `published_at`, a `source` URL on prebell.site, and one fixed sentence: informational, generated automatically from public data, not investment advice, not real-time.

### Protecting the site (my call, to be confirmed by the builder against what the Free plan offers)

- Inputs are validated and capped (ticker pattern, date range, list length, request body at most 16 KB); a JSON-RPC batch is rejected or capped at a small number.
- Each tool does a small, bounded number of D1 reads and at most one R2 read.
- Responses for the same normalized arguments are cached for a short time (about 60 seconds) with the Workers Cache API, so repeated identical calls do not reach D1.
- Rate limiting per client address: use the Workers rate-limit binding if it is available on the Free plan; if it is not, write the exact Cloudflare dashboard rule for `/mcp` into `docs/runbook.md` for Rick to switch on, and say so in the report. No paid feature either way.
- One log line per call (`mcp tool=… cache=hit|miss ms=…`, never the caller's address) so usage can be counted from the Worker's logs. No database table.

### Text safety

Tool output is data for someone else's model. Return stored fields as plain text; do not add instructions, and do not pass through anything the site's own pipeline would not publish. Mention in the server's description that reasons and summaries are machine-written from the linked sources.

## Constraints

- Cloudflare Workers Free: about 10 ms CPU per request, 100,000 requests a day shared with the site, D1 and R2 free allowances. No new cron, queue or migration.
- This Next.js version (16.x) differs from older ones: read the bundled docs in `apps/web/node_modules/next/dist/docs/` before writing the route. (`apps/web/AGENTS.md` is generated by `next dev` and is not in the repo.)
- English and Chinese, using the fields the site already stores for each language.
- No live requests to Yahoo, the SEC or BLS from this route, ever.

## Left to the builder

- File layout under `apps/web/src`, the handler's internal structure, exact field names, and the JSON schemas.
- How earnings fact bundles are read from R2 for `earnings_report` (the web does not parse them today; the Rust types in `crates/core/src/earnings/facts.rs` are the shape).
- Cache key design and TTL within "about a minute".
- Tests: unit tests for the JSON-RPC handler and each tool against the test D1 helper (`apps/web/tests/d1.ts`) and fixture fact bundles; an end-to-end check against the local preview with a real MCP client or the MCP Inspector's CLI.
- `docs/runbook.md` section (how to connect a client, how to count calls, the rate-limit rule) and a README line.

## Done means

- `pnpm test` and `pnpm typecheck` pass in `apps/web`; the Rust checks are unaffected.
- A standard MCP client connects to the local preview, lists three tools and gets a correct answer from each, in English and Chinese.
- A malformed request, an oversized body, an unknown method, an unknown tool, bad arguments and an unknown ticker each get the right kind of error or result.
- Deploy is the web Worker only, by Rick.

## Rulings after the builder check (2026-11-10, the controller's calls; each reversible)

A fresh reader checked this document against the code (`reports/mcp-builder-check.md` in the session scratchpad). None of its questions changes what gets built for Rick, so they are settled here.

1. **"What to watch" in `latest_brief`** comes from the stored facts' `watch_list` (the data the published line was rendered from), parsed leniently from the Rust shape in `crates/core/src/events/watch.rs` (title, time, status, link, figures when present). The field is omitted when the facts hold none. `latest_brief` also takes `full_text`, like `earnings_report`, to return the article body.
2. **Chinese** (`lang=zh`): title, summary and (with `full_text`) the article come from the Chinese translation. Figures use stable keys (`revenue`, `eps_gaap`, ...) plus a label in the requested language from a small table in TypeScript that mirrors `label_en`/`label_zh` in `crates/core/src/earnings/facts.rs`, with a test that keeps the two in sync (as the events glossary test does). Quotes stay verbatim, in English. Fields that exist only in English keep an explicit `_en` name (`summary_en`, a mover's `reason_en`).
3. **Earnings fact bundles**: confirm in the generator code how an earnings post records its facts key before relying on it. Every field added after launch is optional (`share_move`, `cause`, `scale`, `current`). A check whose bundle is missing or unreadable returns its title, summary, period and links, and says the figures are not available. Add an earnings facts fixture to the web tests, built from the Rust test fixture.
4. **Dates**: the range cap is 21 calendar days. `is_today` compares the post's trading date with today's ET date. "Next brief due" is 8:00 ET on the next weekday that is not a market closure when the events table has closure rows, else "the next trading day" with no date.
5. **A stale brief is a result, not an error**: the newest published brief with `is_today: false` and the not-out-yet sentence. Empty only when no brief exists.
6. **Market data in `latest_brief`** is limited to what the page prints: a mover's last price and percent change, not the previous close or volume.
7. **Calendar**: the tool needs its own date-range and ticker query; the week helper is fixed to Monday-Friday. The filing link is what the calendar exposes today (reported rows with an EDGAR link). Events are the tiers the calendar page shows.
8. **Off switch**: a `MCP_ENABLED` var in `apps/web/wrangler.jsonc` (on by default); when it is not `"true"` the route answers 404, so the server can be turned off with one redeploy flag.
9. **Rate limiting does not block the build.** If the Free plan has no rate-limit binding, the cache and input caps ship, the exact dashboard rule goes into the runbook and the PR, and switching it on before or with the deploy is Rick's step. POST responses are cached under a synthetic GET key of the normalized arguments.

