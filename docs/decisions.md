# Decisions

Things Rick has decided, with the date and his words where they matter, so
they are never asked twice. Things decided *not* to do go in `declined.md`.

## 2026-11-04

- **Frontend hosting: Cloudflare Workers, not Vercel.** The web app reads D1
  and R2 through bindings; Vercel would need HTTP adapters and its Hobby plan
  bars commercial use. Rick chose "Cloudflare Workers now".
- **Site look: "B · the front page".** A daily paper: warm off-white, serif
  headlines, lead story with a cover, movers as a ruled column, no pills, no
  boxes. Rick: "i think this one ace over: B". Dark mode is dropped.
  Design: `docs/superpowers/specs/2026-11-04-front-page-redesign-design.md`.
- **Cover images are AI-generated** (Workers AI, Flux Schnell, free
  allocation), composed with the logo and two verified figures at render
  time. Rick: "AI-generated scenes". **Superseded on 2026-11-05** by "Covers
  are data graphics, not AI scenes" below.
- **Author block** at the end of every article and on About, AlphaLab style,
  short bio, links: rickmingyuliu.com, GitHub, LinkedIn. Real photo intended;
  GitHub avatar is only a placeholder.
- **Email: a weekly digest, like AlphaLab, not a daily email.** Rick:
  "weekly digest like AlphaLab". Provider Buttondown (free to 100
  subscribers) behind our own `/api/subscribe` route. Signup ships with the
  redesign; sending is a separate later project.
- **Home keeps a recent-posts list and briefs keep previous/next links;
  the home premarket/recap tabs and the earnings strip on post pages are
  removed.** (Rick, via the builder-check question.)
- **Generator LLM stays DeepSeek** (`--var LLM_PROVIDER:deepseek` on deploy)
  until the Max-plan API credit is claimed, then switch to Claude.

## 2026-11-05

- **Section order follows the story: macro → themes → movers (AlphaLab's
  digest order); approved by Rick 2026-11-05 when he approved the
  themes-and-macro design.**
- **Free-plan budget: 4 movers per side instead of 150-symbol Yahoo
  batches** (controller's call after the 150-symbol live check could not
  run; reversible in one constant once a live check from another network
  passes).
- **The not-advice statement lives only on the legal page** ("Terms: nature of content" / "条款：内容性质声明"), modelled on AlphaLab's 内容性质声明; no footer or per-post disclaimer. Rick, 2026-11-05: "just include the not advice section only in legal and privacy policy page instead of showing everywhere".
- **Covers are data graphics, not AI scenes** (replaces the 2026-11-04
  AI-generated covers entry): the generator draws the story's own figures as
  an SVG chart in the paper palette, a brief's biggest movers and an earnings
  check's recent revenue, with no Workers AI call. Rick, choosing among four
  mockups: "A · Data graphic"; and, of the AI covers: "they should be more
  about the market instead of just the dark gray shadows". Design:
  `docs/superpowers/specs/2026-11-05-data-covers-design.md`.
- **Author photo in colour, not greyscale.** Rick, 2026-11-05: "do not make my photo black and white, original colorful". The `.author-photo` grayscale filter is gone; the file itself was always colour.
- **The site's domain is `prebell.site`**, served by the `prebell-web` Worker as a custom domain. www redirects to the apex (a second custom domain plus a Next redirect, no dashboard rule), and the old workers.dev URL stays up. Rick, 2026-11-05: "i purchased the domain name: prebell.site".
- **Contact addresses are on the domain, as with TermPocket**: `privacy@prebell.site` (privacy requests), `support@prebell.site` (terms questions, EDGAR `SEC_CONTACT`), forwarded to the owner's Gmail by Cloudflare Email Routing. Rick, 2026-11-05: "do the email address into the same way we handle as termpocket, use the site one".

## 2026-11-06

- **The affiliate disclosure is the partner strip's label**: "Affiliate partners" / "推广合作" replaces "From our partners" plus the sentence on the right, and the terms ("sources and links") say PreBell may earn a referral fee through those links. Rick asked "can we remove this note?"; the disclosure must stay where readers see the links, so it was folded into the label (Rick chose "Fold into the label").
- **Earnings: fix the failures first, then widen coverage** beyond the S&P 100: all ~520 companies (S&P 500 ∪ Nasdaq-100) on the calendar, full write-ups only for the largest. Rick chose "Fix failures, then widen" on 2026-11-06 after a week where 6 of 14 detected reports did not publish.
- **An earnings check publishes without revenue when the release states none** (AIG, ConocoPhillips): revenue stays required when the release gives it, so a figure that fails verification still blocks and retries. Ruled under Rick's "Fix failures, then widen" (2026-11-06) (the spec left required sections to the designer); reversible in `missing_required`.
- **The premarket brief publishes at 8:00 ET and is updated at 8:35 ET** with the 8:30 economic data: the update rebuilds the brief from fresh facts and rewrites the same post (same URL, "Updated" time shown), only when both languages draft; otherwise the 8:00 brief stands. Rick asked "is it also possible to trigger the articles slightly earlier?" and chose "8:00 brief + 8:35 update" over a plain 8:00 move and over keeping 8:30. Design: `docs/superpowers/specs/2026-11-06-premarket-update-design.md`.
- **Earnings widening, the shape** (Rick, 2026-11-06, choosing among options from `docs/superpowers/specs/2026-11-06-earnings-widen-design.md`):
  - Write-ups for the S&P 100 plus every company at or above $100B market cap in a dated snapshot ("S&P 100 + $100B+"); the rest of the S&P 500 ∪ Nasdaq-100 is calendar only.
  - About 15 hand-picked large foreign companies outside both indexes (Enbridge, Sony, Ferrari, Ryanair, ...) join the calendar, calendar only ("~15 hand-picked").
  - A calendar-only company shows its name, a ✓ once reported and the SEC filing link; its ticker page is noindex until it has a post ("Row + SEC link").
  - Earnings builds get their own queue, `prebell-earnings-build` ("Yes, add it").
  - English company names on ZH pages for v1 (controller's call; reversible).
- **Earnings widening, builder's rulings** (under Rick's "S&P 100 + $100B+" and "~15 hand-picked", 2026-11-06): the market-cap snapshot comes from Nasdaq's stock screener (Yahoo's v7 quote answered 429 to the build machine); the hand-picked foreign list is the six already on the calendar (BCS, NVO, SAN, SAP, TM, TSM) plus Enbridge, Sony, Ferrari, Ryanair, Suncor, Canadian Natural, TC Energy, Manulife, Brookfield, Itaú and Teva (17); one share class per company (GOOGL, FOXA, NWSA); a filer is foreign only when it files 20-F/40-F/6-K and no 10-Q. Reversible in `crates/core/data/watchlist-overrides.tsv` and `scripts/build-watchlist.mjs`.
- **Macro events on the calendar, v1 kinds: FOMC decisions, the jobs report, CPI and US federal elections**; the fuller set (GDP, PCE, PPI, claims, retail, Fed speeches, ...) is decided later. Rick, 2026-11-06: "for now just only the market-moving events (Fed decisions, jobs, CPI, elections). we will decide if we want to add the fuller later". Design: `docs/superpowers/specs/2026-11-06-macro-events-design.md`.
- **Macro events, Rick's answers** (2026-11-06, on `docs/superpowers/specs/2026-11-06-macro-events-design.md`): FOMC minutes are in v1 ("Yes, include", tier 2); figures are "Link only for now" (never consensus, no actual figures in v1, the release link only); the calendar heading is "Market calendar" / "市场日历" with the `/earnings` URL kept; "What to watch" adds the write-up-tier companies reporting after that day's close ("Yes, add them").
- **The privacy page states the site uses Cloudflare Web Analytics** (no cookies, aggregate visit data only), and the link-preview / search descriptions for the calendar, ticker pages and the default share card are as drafted on `feat/reader-basics`. Rick approved both "as written" on 2026-11-06, after choosing to grow daily readers ("make it a tool for people to access and earn some DAUs") from both Chinese and English social ("i gonna post on both").
- **Every company on the calendar is a tile** (logo, or a ticker tile), write-up companies first, the name beneath on wide screens; the text rows are gone. Rick, 2026-11-06: "so weird, what about just assign everyone of them a label?", then chose "Tiles for all". This replaces the calendar-only text row from "Row + SEC link": the SEC filing link moves to the ticker page, one click from the tile.
- **Calendar legend wording for the tiles**: "…every other company's page links to its SEC filing." / "…其余公司的页面链接至其 SEC 申报文件。" Rick approved "Yes, use the new wording" on 2026-11-06.
- **The market calendar is a week board with wide company cards**: five day columns side by side on a desktop (one day per screen with day chips on a phone), before open and after close as sub-columns, the Economy & Fed strip at the top of each day, and every company as a white card with its logo fitted inside (a wordmark across the card, a square icon beside the ticker and name) or its ticker and name set in type. Rick asked "how come alphasite's look more polished than ours" and chose "Week board + wide cards" on 2026-11-06. This replaces "the name beneath on wide screens" in the tiles entry above; the tiles-for-all rule stands.
- **Design rules narrowed for the board** (builder's ruling under "Week board + wide cards", 2026-11-06): soft shadows are allowed on the calendar's company cards only, and the board has its own 1000px breakpoint whose block may hold calendar rules only; 720px stays the one site-wide breakpoint. Both are asserted in `apps/web/tests/layout.test.tsx`.
- **Calendar key line above the board**: "☀ Before open · ☾ After close · ✓ Reported · dashed card: date not confirmed" / "☀ 盘前 · ☾ 盘后 · ✓ 已发布 · 虚线框：日期未确认", one quiet line with the cards' own tick and a small dashed swatch (decorative; the words carry the meaning). Rick approved it "as written" on 2026-11-06.
- **About 30 large Chinese companies listed in the US join the earnings calendar, calendar only** (dates, and a ✓ with the SEC filing link once reported: 28 of them file 20-F/6-K, which have no Item 2.02, so they get no write-up, and their ✓ comes from the 6-K results release (see the 6-K detection entry below); Yum China files 10-Q/8-K, so its 8-K gives it the ✓). Rick, 2026-11-06: "Yes, ~30 of them". The list (US exchange-listed, by US market cap and reader interest on 2026-11-06): BABA, PDD (already in the Nasdaq-100), JD, BIDU, NTES, TCOM, TME, BILI, NIO, XPEV, LI, FUTU, TIGR, BZ, YMM, VIPS, ZTO, HTHT, BEKE, TAL, EDU, YUMC, QFIN, LU, HSAI, WB, IQ, GDS, MNSO, ATAT. Left out: Zeekr (taken private), Luckin (OTC), Tencent and Xiaomi (OTC / Hong Kong). English names only, as for v1. In `crates/core/data/watchlist-overrides.tsv`; reversible there.
- **Foreign filers' reports are detected from their 6-Ks**, so they also get "a ✓ once reported and the SEC filing link" (Rick's calendar-only decision, 2026-11-06, "Row + SEC link"; built at the controller's request the same day; review fixes the same day). The poll reads EDGAR's current 6-K feed, takes a 6-K that a watched 20-F/6-K filer files from 2 days before to 7 days after its calendar date, and reads the opening (32 KB) of its results document, one 6-K per `earnings:6k` message (queued by the poll, at most 3 a poll) so its CPU never adds to the poll's. It marks the release date reported (its dateline, else the 6-K's date when later than the calendar's), with the 6-K as proof, only when the document states results for a period that ended within 100 days, reported figures follow, the company is named, and no notice wording comes up to the end of that sentence (board or audit-committee meetings, "will be held", announcement or release dates, "scheduled to", "expected to be released", pre-announcements, profit alerts and "preliminary", delivery updates, monthly revenue, meetings, buybacks, reconciliations); 6-K/As never count. Coverage is partial (15 of 17 results releases in a 49-filing review corpus; no wrong ticks there). No nightly per-company fallback: per-company submissions JSON is large to parse within 10 ms of CPU, and the feed already covers every 6-K. In `crates/core/src/earnings/foreign.rs`; reversible there.
- **The calendar board is compact and keeps to the content column**: the board and its header sit in the site's column (the width of the masthead rule, about 1088px) instead of spanning the window. On a desktop the cards are about 96×44 with an 80×28 logo box (about 2/3 of before), with tighter gaps and smaller day headings and half labels; the five days and their two halves still fit from 1000px. The phone keeps its one-day row with slightly smaller cards. No wording changed. Rick, 2026-11-06, on the live board: "much better, but hope this section can be smaller in size".
- **Unconfirmed calendar cards are dashed, in full colour**: the dashed border is the only mark of an unconfirmed date, as in the key line; the logo, ticker and name are no longer faded or greyed, and the card has the normal white ground (the key's dashed swatch too). Rick, 2026-11-06: "Dashed, full-colour logo".
- **The generator writes with Claude Sonnet 5.5 at effort low**, ending the DeepSeek stopgap (see "Generator LLM stays DeepSeek" above). The deploy command no longer passes `--var LLM_PROVIDER:deepseek`, so `wrangler.toml`'s `LLM_PROVIDER = "claude"` and `LLM_EFFORT = "low"` apply. DeepSeek (`deepseek-flash`, thinking off) stays the fallback, used by redeploying with `--var LLM_PROVIDER:deepseek`. Rick, 2026-11-06: "api credit is claimed, we now get 100. we can do the switch and the merge". Why Sonnet, from the 2026-11-06 eval on 18 of the week's bundles:
  - first-pass with the fixed checker: Sonnet 96%, DeepSeek 92%, Opus 89%;
  - Chinese first-pass: Sonnet 94%, DeepSeek 86%;
  - no rule breaks for Sonnet, 4 for DeepSeek;
  - about $23–32 a month at current volume, against the $100 credit;
  - Opus costs 2.5× as much and is no more accurate.
