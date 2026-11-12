# IPO on the front page, and IPO write-ups

Date: 2026-11-11. Status: approved direction, details below are the builder's to follow.

## Why

Rick, 2026-11-11: "i want the ipo section to be in the main page and have some write up articles as well if major things happened. just showing in the calender is way too lowkey and people may dismiss".

The IPO watch (PR #61, #62) stores a tracked IPO's filing facts, attributed news items and trading sessions, and shows them on `/[locale]/ipo/<slug>` and as one calendar entry. A reader who opens the home page does not learn that Anthropic is going public. This work puts the IPO on the home page and publishes a short write-up each time a filing proves a step forward.

Rick's decision on what triggers a write-up (2026-11-11, from three options): "Filing milestones (Recommended)", described to him as: "A write-up when SEC filings prove a step: goes public with its filing, sets or changes the price range, prices the deal, first trading day closes, or withdraws. About five per IPO. Reported news (valuation, date, company moves) is quoted inside each piece but doesn't trigger one."

## Goals

1. The home page shows an IPO section whenever a tracked IPO is active, in both languages, visible without scrolling on a desktop screen and before the movers on a phone.
2. A write-up in English and Chinese is published for each milestone, appears in the home page's recent list and the archive like any other post, and is linked from the home section and the IPO page.
3. Every write-up passes the same kind of code check as an earnings write-up before it is published. Publishing nothing is an allowed outcome.

## Non-goals

- No write-up triggered by news alone (a reported valuation, a reported date, a product launch).
- No change to what the news sweep stores or how it checks quotes.
- No IPO content in the MCP server or the weekly digest.
- No new paid service, no extra Tavily searches, no migration unless the builder finds one unavoidable (then stop and report).

## Anti-goals

- A write-up that states a valuation, a listing date, demand for the deal, or a view in PreBell's own voice. Those may appear only as attributed, word-for-word quotes from stored news items.
- Any sentence that tells the reader what to do, predicts the first-day price, or compares the deal with "expectations".
- A home section that shows an IPO that is withdrawn, or one that listed more than 30 days ago.
- A number that is in neither the facts bundle nor a quoted news item.

## Milestones

One write-up per milestone, each proven by stored data, never by news:

| Milestone | Proven by | Key |
|---|---|---|
| `filed` | first public registration statement (S-1 or F-1; a confidential draft does not count) | accession |
| `range_set` | first cover with a price range | accession |
| `range_changed` | a later amendment whose price range or share count differs from the last one written up | accession |
| `priced` | final prospectus (424B4 or 424B1) with the offering price | accession |
| `listed` | the first trading session has closed and its row is stored | session date |
| `withdrawn` | withdrawal filing (RW) | accession |

Rules:
- A milestone is written up once. The run key is `ipo:<ipo_slug>:<milestone>:<key>`, so a retry or a redelivered message cannot publish twice.
- At launch, only the latest milestone already reached is written up (for Anthropic: `range_set`, the S-1/A of 2026-11-10). Earlier milestones are not backfilled. The write-up states the filing's own date; it does not pretend to be news of today.
- A milestone found more than 7 days after its filing or session is skipped with a `warn` alert, not published late.
- An amendment that changes nothing on the cover is not a milestone.

## The write-up

Facts bundle (stored in R2 like other posts), gathered by code:
- the cover facts with their filing sentences, the filing's form, date and EDGAR link;
- for `range_changed`: the previous range and share count, from the previous write-up's facts;
- the filing timeline;
- figures derived by code and labelled as derived: the raise at the low and high end of the range (shares offered x price), and after pricing, the raise at the offering price. The model may not derive anything itself;
- for `listed`: the first session's open, close, volume and the change from the offering price, as stored;
- the stored news items for this IPO (valuation, date, company moves, views), each with its outlet, link, date and exact quote.

The model receives that bundle and nothing else, and writes the English and the Chinese version separately, as the earnings write-ups do.

Checks before publishing (code, not the model):
- Every number in the draft is in the bundle (the existing number checker and its rounding rules).
- The headline states the milestone with a figure from the bundle (for `range_set`: the price range).
- Anything taken from a news item appears as an attributed quote: the outlet is named and the quoted words are word-for-word from the stored item. A valuation, a listing date, or a view outside such a quote rejects the draft. In the Chinese version the quote stays in its original English, with the attribution in Chinese.
- Banned outright: advice, price predictions, "oversubscribed" or other demand claims not inside a quote, "expected" comparisons, and beat/miss language.
- One retry with the rejection reason, then no publish and an alert, with later attempts on the schedule the earnings write-ups use.

Length and shape: about the size of an earnings write-up; what happened and the figures first, then what the filing says about the offering, then "What has been reported" with the quoted news items if any exist, then the next step in the process stated generally (for example, that a final prospectus follows pricing) without a date unless a stored reported date is quoted.

Every write-up's stored body (`body_md`) ends with the standing line that it is generated automatically from SEC filings and public news reports, for information only, and is not investment advice, as the other generators' bodies do. The site does not render it on the post: the not-advice statement lives only on the legal page (Rick, 2026-11-05, `docs/decisions.md` line 41). Corrected 2026-11-11, the lead's ruling.

## Contract between the generator and the site

- `posts.content_type = 'ipo'`; `posts.template_id = 'ipo-<milestone>'`.
- `runs.run_key` and `posts.run_key` = `ipo:<ipo_slug>:<milestone>:<key>`. The site finds an IPO's write-ups by run-key range (`run_key >= 'ipo:<slug>:' AND run_key < 'ipo:<slug>;'`), so no new column or table is needed.
- `posts.slug` is unique and readable, for example `anthropic-ipo-price-range-2026-11-10`.
- `posts.trading_date` is the filing date (the session date for `listed`).
- `posts.facts_r2_key` = `ipo/<ipo_slug>/<milestone>-<key>.json`, holding at least `ipo_slug`, `milestone`, `name`, `symbol`, and the bundle above.
- `post_translations` rows for `en` and `zh` as for other posts.

The builder of the generator side writes the exact facts JSON shape into `docs/superpowers/specs/2026-11-11-ipo-articles-contract.md` first and tells the site builder when it is committed.

## The home section

- Shown when at least one tracked IPO is active: not withdrawn, and not more than 30 days past its first session (the same rule the news sweep uses). No active IPO, no section and no empty box.
- For each active IPO: name, symbol and exchange, the status label the IPO page uses, the price range (or the offering price once priced), the first trading day with the same wording and confirmed/reported distinction as the IPO page (or "date not set"), the latest reported valuation with its outlet when one is stored, the headline of the latest write-up linking to it, and a link to the full IPO page.
- After listing: the latest stored close and its change from the offering price, as the IPO page shows them.
- Placement is the designer's to propose with screenshots at 1440, 768 and 390 px, within goal 1. It should look like part of the paper, not an advert.
- It reads from D1 only, tolerates the IPO tables being missing or empty, and adds as few reads as possible to the home page (the home page's query-plan tests must keep passing).

Elsewhere on the site:
- `ipo` joins the content types: the recent list, the archive and its filter, the type label ("IPO" in both languages), the sitemap.
- The write-up's page uses the normal post page, with a link to the IPO page.
- The IPO page lists its write-ups, newest first.

## Switches and cost

- `IPO_ARTICLES_ENABLED`, exactly `"true"`, in the generator's `wrangler.toml`. The home section needs no switch; it shows what is stored.
- Cost: two model calls per milestone and language pair, about five milestones per IPO; a few cents each.
- Free-plan limits: the build runs as its own queue message like an earnings build, within 50 outbound requests.

## Calls made by Claude, for Rick to overrule

- Only the latest milestone is written up at launch; nothing older is backfilled.
- A milestone discovered more than 7 days late is skipped.
- The raise at each end of the range is computed by code and shown as derived.
- Quotes stay in English inside the Chinese write-up.
- The home section hides an IPO 30 days after it lists.

## Left to the builders

Module layout, prompt wording, the exact check implementation (reuse the earnings checker where it fits), the section's design, and test fixtures. Raise anything that would change what a reader sees and is not covered here.
