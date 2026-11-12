# Bilingual daily market site — design

Date: 2026-10-29. Status: draft for owner review.
Name: PreBell (盘前铃), domain prebell.site (bought 2026-11-05; planned as prebell.ai)

## 1. Intent

Build an automated investing-news site, in **English and Simplified Chinese**, that does the daily reading for people who follow US stocks but don't want to read filings and news themselves. They should come back every day.

What defines it:
- **North America first.** All schedules run on US Eastern time. It leads with a premarket brief at ~8:00 AM ET, which also lands at ~8:00–9:00 PM in Beijing for Chinese readers, updated at ~8:35 AM ET once the 8:30 economic releases are out.
- **Source-backed earnings.** We detect earnings releases from SEC EDGAR filings (8-K Item 2.02) as they are filed.
- **Numbers that are checked.** Every number in every published text is checked against the stored data before publishing.

## 2. Readers and success

- Readers: people interested in US stocks who want the news summarized for them. Mostly North America; a mix of mainland Chinese readers. Primary language English; Simplified Chinese second.
- Success: readers return daily; content publishes on time every trading day without a human touching it; no published number contradicts its source.

## 3. Goals

1. **Premarket brief** (the headline product), weekdays ~8:00 AM ET, rebuilt once from ~8:35 AM ET (the update, below). Sections in this order, macro first because the order follows the story:
   - "Futures" / "期货": index futures (ES, NQ, YM, RTY) and their changes;
   - "Macro & overnight" / "宏观与隔夜": up to five overnight macro and market news items;
   - "Themes" / "板块动向": industry groups whose members moved together, each as a code-rendered list line ("**Semiconductors** ▼ median −4.20%: NVDA −5.20%, AMD −4.80%, AVGO −4.50%, MRVL −4.30%, ON −4.10%, MCHP −3.90%, and 2 more", with up to six members listed before the count) and a short paragraph;
   - "Biggest premarket movers" / "盘前异动": top premarket gainers and losers among liquid US stocks, each with the news behind the move;
   - "What to watch" / "今日关注": a short list (scheduled data releases, earnings before the open).

   **The update (owner, 2026-11-06: "8:00 brief + 8:35 update").** 8:30 ET is when CPI, the jobs report, jobless claims, GDP and retail sales come out, so the 8:00 brief is written before the number. From 8:35 ET a second build rebuilds the whole brief from fresh facts and rewrites the same post at the same URL, EN and ZH together, and the post page shows "Updated 8:35 AM ET" next to its publish time. It writes only when the number check passes in both languages; otherwise the 8:00 brief stands, with no alert. It is not needed, and skipped, when the brief itself went out at or after 8:30 ET. Design: `docs/superpowers/specs/2026-11-06-premarket-update-design.md`.
2. **After-close recap**, weekdays ~4:45 PM ET: "Index closes" / "指数收盘", "Macro & after the close" / "宏观与盘后", "Themes" / "板块动向" (same list line and paragraphs), "Notable movers" / "个股异动" with reasons, "After-hours earnings to watch" / "盘后财报关注". Shorter than the premarket brief. (Kept by default; owner hasn't objected.)
3. **Earnings health checks** for the largest companies (the S&P 100 plus every S&P 500 ∪ Nasdaq-100 company at or above $100B in a dated market-cap snapshot): triggered when the company's 8-K Item 2.02 appears on EDGAR. Covers revenue, margins, EPS (GAAP vs non-GAAP), cash flow, guidance, with charts, citing the filing and press release.
4. **Earnings calendar page** (upcoming dates, before open / after close / unknown), confirmed from primary sources, for the whole S&P 500 ∪ Nasdaq-100 plus about 15 hand-picked large foreign companies (Rick, 2026-11-06) and about 30 large Chinese companies listed in the US (Rick, 2026-11-06, "Yes, ~30 of them"), all calendar only. A foreign (20-F/6-K) filer gets its ✓ and filing link from its 6-K results release (EDGAR's current 6-K feed, near the calendar date) when the release's wording is recognised, which covers most but not all of them; a domestic filer such as Yum China gets them from the 8-K. Every company is a tile (its logo, or its ticker), write-up companies first, the name beneath on wide screens and a ✓ once reported; a calendar-only tile links to its ticker page, which carries the SEC filing and is noindex until it has a check (Rick, 2026-11-06: "Tiles for all").
5. **English + Simplified Chinese** for every piece, at `/en/...` and `/zh/...`.
6. **Weekly digest email** (owner, 2026-11-04 "weekly digest like AlphaLab"; shape 2026-11-06; one English email 2026-11-10, "just english"; `docs/decisions.md`). One English email a week to every subscriber, saved Friday evening as a Buttondown draft that Rick reviews and sends (meant for Sunday about 6 PM ET). It works on Buttondown's free plan: no tags and no audience filter. Signup (`/api/subscribe`) is on English and Chinese pages (the Chinese form says the email is in English; Rick, 2026-11-12) and sends Buttondown only the address.
   - **Content, plain text-like:** an opening line; three picks from the past week's earnings checks whose English page is published (a title must not be empty), one per company (its newest check), the largest companies first by the market-cap snapshot (a company missing from it last, ties by ticker), each its title linked to the English page and its one-line summary (fewer when fewer exist; with none the section and its mention in the opening line are left out), and a link to the earnings archive; then the week ahead: for each day Monday to Friday, the Economy & Fed events (time, glossary title, a link to the schedule or release), closures and early closes, and the earnings calendar by slot (before open, after close, time n/a), with write-up companies named, largest first, "(unconfirmed)" on an aggregator's date and every other company counted, and a link to `/en/earnings?week=<Monday>`; then one question inviting a reply, the last line before the sign-off "Rick"; then a "not investment advice" footer. Only stored data, never an LLM: titles and summaries are the published pages' own, event titles come from the events glossary, names from the watchlist, and every other word from `crates/core/src/digest/copy.rs` (Rick redrafts it there). No comparison with analyst estimates. The subject is "Week ahead: Nov 9–13".
   - **When:** no cron of its own. A Friday earnings poll from 18:00 ET, once that day's calendar refresh has finished, queues one `digest` message on `prebell-earnings`, and does so again on each poll until 22:00 ET until kv `digest_built_for` holds next Monday's date. From 21:45 ET it is queued even if the refresh has not finished, and the draft is then built from the calendar as it stands with a `digest-stale` warning. A run from 21:55 ET that saves no draft alerts `digest-missed`, once a week. A message on any other day does nothing. It is gated by `DIGEST_ENABLED`, which is off unless exactly `"true"` (no surrounding spaces).
   - **Sending:** Rick sends each draft ("Draft first, I click send"); `DIGEST_MODE = "scheduled"` is the later switch to Sunday 18:00 New York time, using that Sunday's own UTC offset, and stays off. With `DIGEST_ENABLED = "true"` and the generator's `BUTTONDOWN_API_KEY`, `digest::buttondown::Buttondown` creates the draft with `status: "draft"`, a pinned `X-API-Version` and one idempotency key per week, and schedules it only when Buttondown's answer says it is a draft. Without the key the run is a dry run that sends nothing. The sender name is the newsletter's "from name" setting in Buttondown. Sending is one `Sender` trait, so another provider would change only that.
   - **Budget:** 0 fetches as a dry run, 1 for a draft, 2 when scheduled, and about 6 D1 queries, in its own message. Operations are in `docs/runbook.md` ("Weekly digest").
7. **IPO watch** (owner, 2026-11-11: "go ahead and build it"; design `docs/superpowers/specs/2026-11-11-ipo-watch-design.md`, data contract `2026-11-11-ipo-watch-contract.md`, rulings in `docs/decisions.md`). A hand-kept list of big upcoming US listings (`crates/core/data/ipo-watch.tsv`, Anthropic first) on the market calendar and on a page per IPO in both languages: the offering's facts from the latest SEC filing cover (exchange, symbol, shares and their split, the underwriters' option, the price range or final price), each with its sentence and filing link; the filing timeline; valuations, recent company moves and attributed views as reported, each a word-for-word quote from a linked, dated article; the expected first trading day (reported, expected or confirmed, by a code rule); after listing, Yahoo's closes against the IPO price. No model writes a figure, no view or prediction is PreBell's own, and no new cron: the earnings poll queues an hourly `ipo:check` and one `ipo:sweep` a trading day per IPO (at most 30 Tavily searches a month), behind `IPO_WATCH_ENABLED`. Operations and costs are in `docs/runbook.md` ("IPO watch").

## 4. Non-goals (not at launch)

- Crypto digest, AI tutorials, the tools section, affiliate banners, a daily newsletter (the weekly digest is goal 6), social auto-posting. (Can come later, using the same pipeline.)
- User accounts, comments, paywall.
- A CMS or editing UI. Posts are created by the pipeline; fixes happen by re-running or by an admin script.
- China-specific delivery (ICP license, China network). Chinese readers get the same global site.
- Real-time quotes or live tickers on the page.

## 5. Anti-goals (failure even if it "works")

- Publishing a number the data doesn't support, or one where the model filled in a gap. Missing data is shown as missing, never guessed, and this is enforced in code.
- Reading as investment advice. Every page carries a "not investment advice" disclaimer in its language.
- Silent failure: a job that fails must not publish a half-empty post; it alerts instead.
- Republishing third-party text. Articles are written from our own stored facts; news sources are cited and quoted only as short attributed excerpts; their text is never reproduced as our content.

## 6. Owner's decisions (as stated)

- **Front end:** Next.js.
- **Platform:** Cloudflare.
- **Generator / backend:** Rust.
- **AI:** Claude API (owner, updated 2026-10-29: Anthropic API key with launch credit). Launch waits for the Claude provider (Plan 5). DeepSeek stays implemented as a fallback. The provider must be swappable by config (`LLM_PROVIDER`).
- **Data:** start free — Yahoo Finance (unofficial), SEC EDGAR, web search for news — and move to a paid market-data provider once the site proves itself. The source must be swappable.
- **Audience:** North America primary; mainland China as a mix → Simplified Chinese; no China-specific delivery work.

## 7. Architecture (designer's calls unless marked owner)

```
Cron Triggers (UTC, DST-aware gate) ─┐
EDGAR poller (every 2–5 min, market hours) ─┤
                                      ▼
                    Rust Worker "generator" (workers-rs)
                      │  enqueue jobs → Cloudflare Queue
                      ▼
          job: gather facts → facts JSON (R2) + rows (D1)
               → LLM draft EN, LLM draft ZH (from the same facts)
               → number check (code) → charts (SVG, plotters) → R2
               → publish: D1 post rows → revalidate site cache
                                      ▼
              Next.js on Cloudflare Workers (OpenNext)
              reads D1 / R2, serves /en and /zh
```

- **Site:** Next.js App Router on Cloudflare Workers via `@opennextjs/cloudflare` (owner: Next.js + Cloudflare; adapter is designer's call). i18n with `next-intl`, routes `/en/...` and `/zh/...`, `hreflang` alternates, language switch on every page. Pages render from D1/R2 on the server and are cached; publishing triggers revalidation of the affected paths.
- **Storage:** D1 for posts, translations, facts index, sources, runs, calendar. R2 for facts JSON snapshots, article bodies if large, and chart SVGs.
- **Generator:** Rust compiled to WebAssembly with `workers-rs` (owner: Rust). Scheduled by Cron Triggers; heavy work goes through a Queue so each job runs in its own invocation. Cron times are UTC, so the Worker is triggered at both candidate UTC times and checks the current America/New_York time in code to handle daylight saving (DST ends 2026-11-01).
  - Fallback if a needed Rust crate won't run under WebAssembly: run the same Rust code as a native binary on a schedule (e.g., GitHub Actions or a small VM) writing to D1/R2 through Cloudflare's API. Builder decides if and when.
- **Provider interfaces (traits in Rust):**
  - `LlmProvider`: Claude (owner, primary) with DeepSeek as an implemented fallback. Config selects provider and model.
  - `MarketData`: Yahoo Finance unofficial endpoints first (owner); paid provider later. Covers quotes incl. premarket/after-hours, futures, index levels, movers screens.
  - `NewsSearch`: a web search API (Brave Search or Tavily — designer's call) because our code needs the full article text to verify quotes. Kept on Tavily after the Claude switch; Claude's web-search tool returns results to the model rather than raw text to our code.
  - `Filings`: SEC EDGAR (`data.sec.gov/submissions`, full-text/filing index). Must send a descriptive User-Agent with contact email and stay under 10 requests/second.
- **Earnings detection:** during US hours (~6 AM–9 PM ET, weekdays), poll EDGAR's current 8-K feed every few minutes (one feed for every company, so the cost does not grow with the watchlist); a new 8-K with Item 2.02 by a write-up company queues an earnings job using the filing and its Exhibit 99.1 press release, and one by a calendar-only company marks its calendar date reported. Foreign filers (6-K, e.g., Toyota, Barclays, Santander) have no Item 2.02: they're not auto-written; the poll also reads EDGAR's current 6-K feed, and a watched foreign filer's 6-K results release near its calendar date marks that date reported. The watchlist (`crates/core/data/watchlist.tsv`) is generated from SEC data by `scripts/build-watchlist.mjs`.
- **Earnings calendar:** seed upcoming dates from an aggregator (Yahoo's earnings calendar at launch); mark each entry `confirmed` only after primary proof (company IR announcement or EDGAR); entries with conflicting dates go to a quarantine list with reason and check times. Dates are kept for the next 15 NYSE trading days (from a Friday, the next two full weeks), including Yahoo's own estimates, which show as unconfirmed and never quarantine a company's date (2026-11-06).

## 8. Content pipeline rules

1. **Facts first.** Each job builds one facts JSON (prices, changes, filing figures, news items with URLs) and stores it. Both languages are written from the same facts.
   - **Quote-anchored extraction (designer's call).** Figures that only exist in prose or untyped tables (8-K Exhibit 99.1 press releases, news articles) are extracted by the LLM into the facts *together with a verbatim quote* from the source. Code then checks that the quote appears in the fetched source text and contains the number; a figure that fails is dropped as missing. This makes "checked" mean: every published number is traceable by code to the source text it came from, not just to another model's output.
   - Structured sources skip extraction: prices from `MarketData`, prior-quarter trend data from EDGAR XBRL `companyfacts` (GAAP only). Charts mixing quarters use GAAP figures from XBRL plus the current quarter's GAAP figures from the press release; non-GAAP appears only as text for the current quarter.
   - News numbers (deal sizes, price targets, "rose 12% last week") must also be in the facts via the same quote-anchored extraction; there is no exemption.
   - **No consensus / beat-miss at launch (designer's call):** there is no reliable free estimates source. Earnings checks report results and guidance only. Revisit with a paid provider.
2. **Write each language separately** from the facts, using per-template prompts in each language. Not translate-after.
3. **Number check in code.** Extract every number from each draft, match it to a facts value (allowing defined rounding and unit conversions such as "92.28 billion" ↔ "922.8 億"), and fail the draft if any number has no match. On failure: retry once with the error list; if it still fails, don't publish that language and alert.
4. **Missing data stays missing.** If a source returns nothing for a field, the facts JSON marks it missing; the prompt forbids filling it. A missing required field keeps the post from publishing. A missing optional field or section is **omitted** from the page, never shown as "n/a" / "暂缺", and the prose does not mention it. Code, not the LLM, decides which sections exist and lists them at the end of the drafting prompt. The futures and index tables have fixed rows, so a missing contract or index keeps its "n/a" / "暂缺" cell.
   - **Required sections (designer's call).** A post only publishes if its required sections have data; optional sections are left out when empty. Premarket brief: required = movers (with at least one news reason) and futures; optional = macro, themes, watch list. Recap: required = index closes; optional = macro, themes, movers, after-hours. Earnings check: required = revenue (unless the release states none, as insurers' and some energy companies' do; ruled 2026-11-06), EPS, a link to the filing; optional = segments, cash flow, guidance. The earnings key-figures table lists only the figures the release gave.
   - **Already reported (recap).** When our earnings check for a filing accepted after today's close is already published, the recap's after-hours section ends with a code-rendered line, "Already reported after the close: [Apple (AAPL)](/en/p/aapl-q4-fy2026-earnings)." (ZH "已于盘后发布财报：…"), and the after-hours news extraction skips that company, so the recap never says it is still to report.
   - **Late, not thin (owner).** If required data is missing at the scheduled time, the job keeps retrying (every ~5 min). The premarket brief may publish up to **9:25 AM ET**; after that it is skipped for the day and an alert is sent. The premarket update retries the same way until 9:25 AM ET, then is skipped without an alert, since the brief already stands. The recap may publish up to 7 PM ET under the same rule.
   - **One language can go first (owner).** If English passes the number check and Chinese still fails after its retry, English publishes on time. The Chinese page for that post shows a "translation coming / 中文版稍后推出" notice (excluded from search and from the hreflang pair until filled), and the job keeps retrying Chinese until it passes, then fills it in. The reverse case (Chinese passes, English fails) holds both and alerts, since English is primary. Posts therefore have a per-language status (`published`, `pending`, `failed`).
5. **Cite sources.** Every news claim links its URL; every earnings post links the EDGAR filing and the press release.
6. **Versioned templates.** Each content type has a template ID and version (e.g., `premarket:v1`) stored on the post, so changes can be traced.
7. **Idempotent runs.** A run is keyed by (content type, ET trading date, ticker if any). Re-running replaces the post instead of creating a duplicate.
8. **Market calendar.** Premarket and recap skip NYSE holidays and handle early-close days.

## 9. Pages

- Home (per language): today's premarket brief at the top; latest recap; latest earnings checks; link to the calendar.
- Post page: title, ET timestamp (and the update time once the premarket update rewrote it), body, charts, sources, disclaimer, language switch.
- Archives: by type (premarket / recap / earnings) and by ticker.
- Earnings calendar: week view with navigation, confirmed vs unconfirmed marked.
- About + disclaimer.
- Self-hosted fonts and Cloudflare Web Analytics (works for Chinese readers too; avoids Google services).

## 10. Constraints

- Cost (owner, 2026-10-29): **everything runs on free tiers until the site has real daily active users.** That means Cloudflare Workers Free (5 cron triggers per account, 10 ms CPU and 50 subrequests per invocation), free data sources and free API allowances. Design within those limits; when one is hit, prefer a free-tier workaround (batching, fewer schedules, splitting work into smaller queued steps) and present a paid upgrade only as an option with its cost. AI spend comes from the owner's Anthropic API credit; cost per brief is measured on the first live run.
- Yahoo Finance's terms don't permit scraping; the owner accepts this for launch. The `MarketData` interface exists so we can leave quickly if it breaks or blocks us.
- EDGAR fair-access rules (User-Agent, ≤10 req/s).
- All displayed times in US Eastern; stored in UTC.

## 11. Left to the builder

- Exact D1 schema, crate choices, prompt wording, chart styling, visual design of the site.
- Which liquid-stock universe defines "premarket movers" (e.g., S&P 500 + Nasdaq 100 + names above a volume threshold).
- Watchlist size for earnings checks at launch (suggest S&P 100). Widened on 2026-11-06 (decisions.md): S&P 100 + $100B write-ups, S&P 500 ∪ Nasdaq-100 on the calendar.
- Alerting channel for failures.
- Whether to use the native-binary fallback for any part of the generator.

## 12. Open for the owner

- Site name and domain: PreBell (盘前铃), prebell.site (bought 2026-11-05). Use a config value for the EDGAR User-Agent contact email.
- Whether premarket/recap pages are indexed by search engines. Designer's default: index them.

## 13. Build approach (owner)

Implementation will be split across parallel subagents (site, generator core, data providers, earnings/calendar), with a separate review pass.
