# IPO watch: design

Date: 2026-11-11. Status: Rick approved the picture below in conversation and said "go ahead and build it". Everything marked "my call" is the controller's choice, not his, and he can reverse it.

## Intent

PreBell's calendar shows earnings and economy dates but not big IPOs. Rick wants the big ones tracked, starting with Anthropic.

His words:

- "i am wondering if we can also keep track of the big ipo news, like anthropic, recently. our calender does not show those, which i want to include"
- Asked what he went looking for when the Anthropic IPO news came out: "i would say is the volume, and their recent move. and their valuation and date of ipo. i want to include the news like: they recently also release their office applications like words, ppt etc, but with ai. which can be seen as a potential for more ai powered saas software coming up from them, and also their most recent valuation of 1.4 trillion instead of 2 trillion about a month ago etc"
- After the playback in "What Rick agreed to": "yeah that's right, go ahead and build it"

Why it fits PreBell: an IPO leaves a paper trail at the SEC, which the generator already reads. So the offering's own facts can carry the same proof the earnings checks do: the sentence from the filing and its link.

Why now: Anthropic's amended S-1 of 2026-11-10 set a price range; press reports put pricing and the first trading day within days. A first version that is on the calendar before it lists is worth more than a fuller one after.

## What Rick agreed to (the playback he answered "that's right" to)

An "IPO watch" for a big upcoming listing, one place that shows:

1. **The date** it is expected to start trading, marked unconfirmed until it is priced.
2. **The valuation and how it has changed**, each figure with its date and where it was reported.
3. **Recent moves**: what the company has done lately (his example: AI office applications), each a short dated item with its source.
4. **Volume**: before listing, the size of the deal; after listing, how much it traded and how the price moved. (The reading of "volume" as these two things was my guess in the playback; he did not correct it.)

And two rules, stated to him as my calls under the site's existing rule:

- Facts that are not in SEC filings (valuation, product news, an expected date) are stated as reported by a named source, with the article's own sentence and link, never as PreBell's own number.
- A "what it could mean" line appears only when a cited article says it, quoted and attributed. PreBell does not write views itself.

## Goals

1. A tracked IPO appears on the market calendar: on its date when one is known (unconfirmed or confirmed), and in a short "IPO watch" list on the calendar page when no date is known yet.
2. Each tracked IPO has its own page in English and Chinese with: the offering's facts from the latest SEC filing (exchange, symbol, shares offered, price range, then the final price), each with its source sentence and filing link; a filing timeline; the valuation as reported, over time; recent company moves; and after listing, the first day's price move and volume.
3. Every figure on the page is either in an SEC filing (with its sentence) or in a quoted sentence from a linked article. Nothing is computed by the model.
4. It updates by itself: new filings are picked up within the hour on trading days; news items once or twice a day.

## Non-goals (v1)

- Finding IPOs automatically. v1 tracks a short hand-kept list (see "Design"). "Big" is a judgement and there are a handful a year.
- A written article about the IPO in the brief's style, alerts, or email. The weekly digest may list a tracked IPO's date later; not in v1.
- Financial statements from the prospectus (revenue, losses). The S-1's numbers are a later step with their own checks.
- Analyst estimates, "fair value", allocation or how-to-buy guidance, or any advice.
- A new cron, a new Worker, or any paid service. Free plan only.

## Anti-goals (a failure even if it works)

- A valuation or date shown as fact with no source, or a number the model worked out (for example a valuation multiplied out from shares and price). Code may compute a number only from two stored, sourced numbers, and the page then shows both inputs (for example first-day change against the IPO price).
- A stale or wrong date presented as confirmed. A date from a news report is "unconfirmed" until the pricing filing; a date PreBell cannot source is not shown.
- A view or prediction in PreBell's own voice ("this could mean...", "a strong debut is likely").
- A news sweep that blows the free-plan limits (50 outbound requests per invocation, the Tavily monthly allowance, the model budget) or slows the earnings polls.
- A quote that is not word-for-word in the fetched article, or a "reported by" that names the wrong outlet.

## What is known today (checked 2026-11-11 against SEC EDGAR)

- Anthropic, PBC. CIK `0002130357`. EDGAR lists ticker `ANT`. SIC: prepackaged software. Delaware.
- Filings (from `https://data.sec.gov/submissions/CIK0002130357.json`): DRS 2026-06-01, five DRS/A through 2026-10-09, **S-1 2026-10-26** (accession 0001628280-26-067168), **S-1/A 2026-11-02** (0001628280-26-068676), **S-1/A 2026-11-10** (0001628280-26-072484, primary document `anthropicpbc-sx1ax2.htm`, 14.8 MB).
- The 2026-11-10 amendment's cover, which sits in the first part of the document, says, word for word:
  - "Subject to Completion. Dated November 10, 2026."
  - "Class A Common Stock 825,000,000 Shares"
  - "We are selling 801,351,354 shares of Class A common stock and the selling stockholders identified in this prospectus are selling an additional 23,648,646 shares of Class A common stock."
  - "It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00"
  - "We have applied to list our Class A common stock on the Nasdaq Global Select Market under the symbol “ANT.”"
- The amendment also has a filing-fees exhibit (`exfilingfees.htm`), which normally states the maximum aggregate offering price as a number.
- Not in any filing: a valuation figure, or the first trading day. Press reports (unverified) speak of about $1.4 trillion, pricing around 2026-11-13 and listing around 2026-11-16, and a last private round at $965 billion in May 2026. These are exactly the "as reported" items.

Save a trimmed copy of that cover (the first ~300 KB of the primary document) and the submissions JSON as test fixtures. Fetch them once with the generator's SEC client conventions (its `User-Agent` with `SEC_CONTACT`), not in a loop.

## Design

### Which IPOs (my call)

A hand-kept list in the repo, `crates/core/data/ipo-watch.tsv` (name, slug, CIK, and optional notes), seeded with Anthropic. Adding one is a one-line change. Everything else about an IPO comes from EDGAR and the news, not from this file.

### The offering's facts, from EDGAR

- Read the company's submissions JSON for each tracked CIK (one request each). New forms that matter: `S-1`, `S-1/A` (and `F-1`, `F-1/A`), `424B4` (final prospectus: the price), `8-A12B` (registration on the exchange), `EFFECT` (the SEC declared it effective), `CERT` (the exchange certified the listing), `RW` (withdrawn).
- On a new registration statement or prospectus, fetch only the first part of the primary document (the cover), the way `events/check.rs` reads a prefix of the BLS page; never the whole 15 MB.
- Extract, each with the sentence it came from and a check that the sentence is in the document: exchange and symbol; total shares offered (and the company / selling-stockholder split when stated); the price range; on a 424B4 the final price per share; the cover date. Use the approach the earnings extractor already uses for "figure plus verbatim quote" (`crates/core/src/earnings/extract.rs`, `anchor.rs`); whether that is code patterns or a model call with code verification is the builder's choice. If a model is used, the number checker applies.
- Deal size: use the filing-fees exhibit's stated maximum aggregate offering price if it parses cleanly with its sentence; otherwise show shares and price range and no dollar total. Never multiply it out in the model.
- Status, by code from the forms seen: `filed` (S-1, no range) → `range_set` (range on the cover) → `priced` (424B4) → `listed` (first trading day reached) ; `withdrawn` on RW.

### The date

- Before pricing there is no SEC source for the first trading day. The date shown is the most recent one a news item reports (see below), marked unconfirmed, with its source.
- Once a 424B4 is filed (or an `EFFECT` plus `8-A12B`), the first trading day is taken as the next trading day after pricing by `clock`, marked confirmed only if a filing or the exchange states it; otherwise still "expected", sourced to the filing date. The builder decides the exact rule and writes it in the decisions file; the bar is that "confirmed" never rests on a guess.
- No date known: the IPO sits in the "IPO watch" list with "date not set".

### As reported: valuation, expected date, recent moves, views

- Once or twice a day per tracked IPO that is not yet 30 days past listing (my call on cadence: with the premarket build and after the close), one news sweep: a Tavily search (reuse `tavily.rs`), fetch the top few articles, and one model call that returns items of four kinds: `valuation`, `expected_date`, `company_move`, `view`.
- Each item is: kind; a one-sentence statement in English; the outlet's name; the article URL and its publication date; a verbatim quote; and for `valuation` and `expected_date` the value in structured form.
- Code then checks each item before it is stored, with the machinery the mover reasons use (`anchor.rs`, `numbers.rs`): the quote is word-for-word in the fetched article; every number in the statement is in the quote; the outlet matches the URL's site. An item that fails is dropped, not repaired.
- A `view` is stored only as an attributed direct quote ("X wrote: '...'"). It is never paraphrased into PreBell's voice. Cap how many show (my call: the three most recent).
- Items are kept with their dates so the valuation section reads as a history: "$2 trillion, reported by A on Oct 12" then "$1.4 trillion, reported by B on Nov 10". PreBell prints both; it does not write "fell 30%".
- Dedupe by URL and kind; cap what is kept per IPO.
- Chinese: the statement gets a Chinese version written from the same item and number-checked like other Chinese text on the site; quotes stay verbatim in their original language.
- Budget: the builder counts current Tavily use per month and model calls per day from the code and states what this adds, and caps the sweep so it cannot exceed the free Tavily allowance or 50 outbound requests in one invocation. Each sweep is its own queue message.

### After listing

- When the ticker trades, the recap's quote read already has what is needed for a stock; add tracked, recently listed tickers to what the recap fetches (one more symbol, no new request if the existing batch has room).
- The page then shows, for the first trading day and the most recent one: open, close, volume, and the change against the IPO price from the 424B4 (computed by code from two stored numbers, both shown). Yahoo is the source, as for movers, and is named.
- No live Yahoo requests from the development machine: use fixtures.

### Where it runs

No new cron. Use the existing crons and queues: the filing check rides the earnings poll's cadence on trading days (or an hourly gate on it); the news sweep is queued by the premarket and recap schedule. Keep every invocation under 50 outbound requests and keep the earnings polls' own budget unchanged.

A switch, `IPO_WATCH_ENABLED` in the generator's `wrangler.toml` (my call: committed as `"true"`, since Rick asked for the feature; the deploy is still his). Off means no filing checks and no sweeps; the pages keep showing what is stored.

### Storage

One migration (the next number after `0008`): what the calendar and the page need to read by date and by slug, plus the items. Keep to additive tables; no change to existing ones. The facts with their quotes may live in D1 rows or in an R2 bundle like the earnings facts; builder's choice, following whichever existing pattern fits. Rick applies the migration before the deploy.

### The site (`apps/web`)

- **Calendar** (`/[locale]/earnings`): a tracked IPO shows on its date among that day's events, with the same unconfirmed/confirmed treatment the calendar already uses, and links to its page. Above or beside the week, a short "IPO watch" list of tracked IPOs (name, status, date or "date not set").
- **IPO page** (`/[locale]/ipo/[slug]`), in the plain style of the existing pages:
  1. Name, symbol, exchange, status, and the date line.
  2. The offering: shares, price range or final price, deal size if sourced, each with "from the S-1/A filed Nov 10, 2026" and the filing link; the source sentence available the way the earnings check shows quotes.
  3. Timeline of filings with dates and links.
  4. Valuation as reported (dated, attributed, linked).
  5. Recent moves (dated, attributed, linked).
  6. What others say (attributed quotes), only when there are any.
  7. After listing: the first-day and latest numbers.
  8. The site's disclaimer; a line that pre-listing figures are as reported by the named sources.
- English and Chinese, hreflang pairs, sitemap entries, share metadata, like the other pages. No change to the masthead navigation in v1 (my call); the calendar is the way in.
- If the facts for an IPO are missing or unreadable the page still renders what it has and says what is not available. An unknown slug is a 404.

### Not in v1, noted so they are not forgotten

- The premarket brief's "What to watch" naming a tracked IPO on its day (cheap if the IPO date is also written as a calendar event; do it only if it falls out of the design without new risk).
- The MCP `earnings_calendar` tool returning IPO dates.
- The weekly digest listing next week's tracked IPO.

## Constraints

- Cloudflare Workers Free: 50 outbound requests per invocation, about 10 ms CPU nominal, five crons (all used), D1 and R2 free allowances.
- SEC: the generator's existing client, its `User-Agent`, its pacing (stay under 10 requests a second in total with the earnings polls).
- "Never consensus", "say only what can be proven", number checker and quote anchoring all apply.
- The Next.js version differs from older ones: read `apps/web/node_modules/next/dist/docs/` before writing routes.
- No live Yahoo requests from this Mac; no `wrangler deploy`, `wrangler secret` or remote D1 commands by agents.

## Left to the builder

- Table and JSON shapes, file layout, job names, the exact extraction method, cache keys.
- The exact rule for "confirmed" dates, within the bar above.
- Whether the IPO date is also written as a row the brief's watch list can read.
- Test fixtures: the Anthropic submissions JSON and S-1/A cover prefix, a made-up 424B4 cover in the same shape for the priced state, article fixtures for each item kind including ones that must be rejected (quote not in article, number not in quote, a view paraphrased).

## Done means

- `cargo test -p prebell-core`, the generator wasm check and `cargo fmt --check` pass; `pnpm test`, `pnpm typecheck`, `pnpm lint` pass in `apps/web`.
- From the Anthropic fixtures, the pipeline produces: status `range_set`, Nasdaq Global Select Market, `ANT`, 825,000,000 shares, $70.00 to $78.00, each with its sentence, and the filing timeline.
- A local preview shows the calendar entry and the IPO page in both languages, at desktop and phone width, with seeded data.
- The runbook says how to add an IPO to the list, how to switch the feature off, what it costs in Tavily searches and model calls, and the deploy order (migration, generator, web).
- A separate reviewer has read the diff.
