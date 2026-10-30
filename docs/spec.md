# Bilingual daily market site — design

Date: 2026-10-29. Status: draft for owner review.
Name: PreBell (盘前铃), domain prebell.ai

## 1. Intent

Build our own automated investing-news site, in **English and Simplified Chinese**, that does the daily reading for people who follow US stocks but don't want to read filings and news themselves. They should come back every day. The model is AlphaLab (alphalab.site), whose machinery we reverse-engineered in `notes/findings.md`; we take its *architecture and ideas*, not its text, name, or look.

What makes ours different from AlphaLab:
- **North America first.** All schedules run on US Eastern time. AlphaLab's schedule is built around a Taiwan breakfast (market recap at 7 PM ET) and has **no premarket content at all**. Ours leads with a premarket brief at ~8:30 AM ET, which also lands at ~8:30–9:30 PM in Beijing for Chinese readers.
- **Faster, source-backed earnings.** We detect earnings releases from SEC EDGAR filings (8-K Item 2.02) instead of waiting for a nightly batch.
- **Numbers that are checked.** Every number in every published text is checked against the stored data before publishing.

## 2. Readers and success

- Readers: people interested in US stocks who want the news summarized for them. Mostly North America; a mix of mainland Chinese readers. Primary language English; Simplified Chinese second.
- Success: readers return daily; content publishes on time every trading day without a human touching it; no published number contradicts its source.

## 3. Goals

1. **Premarket brief** (the headline product), weekdays ~8:30 AM ET:
   - index futures (ES, NQ, YM, RTY) and their changes;
   - top premarket gainers and losers among liquid US stocks, each with the news behind the move;
   - overnight earnings and macro news;
   - a short "what to watch today" list (scheduled data releases, earnings before the open).
2. **After-close recap**, weekdays ~4:45 PM ET: index closes, notable movers with reasons, after-hours earnings to watch. Shorter than the premarket brief. (Kept by default; owner hasn't objected.)
3. **Earnings health checks** for a watchlist of large companies: triggered when the company's 8-K Item 2.02 appears on EDGAR. Covers revenue, margins, EPS (GAAP vs non-GAAP), cash flow, guidance, with charts, citing the filing and press release.
4. **Earnings calendar page** (upcoming dates, before open / after close / unknown), confirmed from primary sources.
5. **English + Simplified Chinese** for every piece, at `/en/...` and `/zh/...`.
6. **Competitor watch** (internal only): daily read of AlphaLab's public RSS feed for coverage gaps.

## 4. Non-goals (not at launch)

- Crypto digest, AI tutorials, the tools section, affiliate banners, newsletter, social auto-posting. (Can come later, using the same pipeline.)
- User accounts, comments, paywall.
- A CMS or editing UI. Posts are created by the pipeline; fixes happen by re-running or by an admin script.
- China-specific delivery (ICP license, China network). Chinese readers get the same global site.
- Real-time quotes or live tickers on the page.

## 5. Anti-goals (failure even if it "works")

- Publishing a number the data doesn't support, or one where the model filled in a gap. Missing data is shown as missing, never guessed (AlphaLab's own rule, which we keep and enforce in code).
- Any AlphaLab text reaching our content, even rewritten or translated. Competitor watch reads titles, tickers, categories and timestamps only, and never feeds generation directly.
- Reading as investment advice. Every page carries a "not investment advice" disclaimer in its language.
- Silent failure: a job that fails must not publish a half-empty post (AlphaLab shipped a ~1.3k-char crypto digest on 2026-10-20); it alerts instead.

## 6. Owner's decisions (as stated)

- **Front end:** Next.js.
- **Platform:** Cloudflare.
- **Generator / backend:** Rust.
- **AI:** Claude API (owner, updated 2026-10-29: Anthropic API key with launch credit). Launch waits for the Claude provider (Plan 5). DeepSeek stays implemented as a fallback. The provider must be swappable by config (`LLM_PROVIDER`).
- **Data:** start free like AlphaLab — Yahoo Finance (unofficial), SEC EDGAR, web search for news — and move to a paid market-data provider once the site proves itself. The source must be swappable.
- **Audience:** North America primary; mainland China as a mix → Simplified Chinese; no China-specific delivery work.
- **Competitor watch:** yes, AlphaLab's public feed only, metadata only.

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
- **Storage:** D1 for posts, translations, facts index, sources, runs, calendar, competitor items. R2 for facts JSON snapshots, article bodies if large, and chart SVGs.
- **Generator:** Rust compiled to WebAssembly with `workers-rs` (owner: Rust). Scheduled by Cron Triggers; heavy work goes through a Queue so each job runs in its own invocation. Cron times are UTC, so the Worker is triggered at both candidate UTC times and checks the current America/New_York time in code to handle daylight saving (DST ends 2026-11-01).
  - Fallback if a needed Rust crate won't run under WebAssembly: run the same Rust code as a native binary on a schedule (e.g., GitHub Actions or a small VM) writing to D1/R2 through Cloudflare's API. Builder decides if and when.
- **Provider interfaces (traits in Rust):**
  - `LlmProvider`: Claude (owner, primary) with DeepSeek as an implemented fallback. Config selects provider and model.
  - `MarketData`: Yahoo Finance unofficial endpoints first (owner); paid provider later. Covers quotes incl. premarket/after-hours, futures, index levels, movers screens.
  - `NewsSearch`: a web search API (Brave Search or Tavily — designer's call) because our code needs the full article text to verify quotes. Kept on Tavily after the Claude switch; Claude's web-search tool returns results to the model rather than raw text to our code.
  - `Filings`: SEC EDGAR (`data.sec.gov/submissions`, full-text/filing index). Must send a descriptive User-Agent with contact email and stay under 10 requests/second.
- **Earnings detection:** during US hours (~6 AM–9 PM ET, weekdays), poll EDGAR submissions for watchlist companies every few minutes; a new 8-K with Item 2.02 queues an earnings job using the filing and its Exhibit 99.1 press release. Foreign filers (6-K, e.g., Toyota, Barclays, Santander) have no Item 2.02: at launch they're tracked only on the calendar, not auto-written.
- **Earnings calendar:** seed upcoming dates from an aggregator (Yahoo's earnings calendar at launch); mark each entry `confirmed` only after primary proof (company IR announcement or EDGAR); entries with conflicting dates go to a quarantine list with reason and check times (AlphaLab's `pending_dates` idea).
- **Competitor watch:** daily cron reads `https://www.alphalab.site/feed.xml`, stores title, link, published time, category and any tickers found in the title; a report lists tickers/topics they covered that we didn't in the last 24h. Admin-only (a protected page or a log/notification). Never used as generation input.

## 8. Content pipeline rules

1. **Facts first.** Each job builds one facts JSON (prices, changes, filing figures, news items with URLs) and stores it. Both languages are written from the same facts.
   - **Quote-anchored extraction (designer's call).** Figures that only exist in prose or untyped tables (8-K Exhibit 99.1 press releases, news articles) are extracted by the LLM into the facts *together with a verbatim quote* from the source. Code then checks that the quote appears in the fetched source text and contains the number; a figure that fails is dropped as missing. This makes "checked" mean: every published number is traceable by code to the source text it came from, not just to another model's output.
   - Structured sources skip extraction: prices from `MarketData`, prior-quarter trend data from EDGAR XBRL `companyfacts` (GAAP only). Charts mixing quarters use GAAP figures from XBRL plus the current quarter's GAAP figures from the press release; non-GAAP appears only as text for the current quarter.
   - News numbers (deal sizes, price targets, "rose 12% last week") must also be in the facts via the same quote-anchored extraction; there is no exemption.
   - **No consensus / beat-miss at launch (designer's call):** there is no reliable free estimates source. Earnings checks report results and guidance only. Revisit with a paid provider.
2. **Write each language separately** from the facts, using per-template prompts in each language. Not translate-after.
3. **Number check in code.** Extract every number from each draft, match it to a facts value (allowing defined rounding and unit conversions such as "92.28 billion" ↔ "922.8 億"), and fail the draft if any number has no match. On failure: retry once with the error list; if it still fails, don't publish that language and alert.
4. **Missing data stays missing.** If a source returns nothing for a field, the facts JSON marks it missing and the template prints "n/a" / "暂缺"; the prompt forbids filling it.
   - **Required sections (designer's call).** A post only publishes if its required sections have data; optional sections may show n/a. Premarket brief: required = movers (with at least one news reason) and futures; optional = macro, watch list. Recap: required = index closes; optional = movers, after-hours. Earnings check: required = revenue, EPS, a link to the filing; optional = segments, cash flow, guidance.
   - **Late, not thin (owner).** If required data is missing at the scheduled time, the job keeps retrying (every ~5 min). The premarket brief may publish up to **9:25 AM ET**; after that it is skipped for the day and an alert is sent. The recap may publish up to 7 PM ET under the same rule.
   - **One language can go first (owner).** If English passes the number check and Chinese still fails after its retry, English publishes on time. The Chinese page for that post shows a "translation coming / 中文版稍后推出" notice (excluded from search and from the hreflang pair until filled), and the job keeps retrying Chinese until it passes, then fills it in. The reverse case (Chinese passes, English fails) holds both and alerts, since English is primary. Posts therefore have a per-language status (`published`, `pending`, `failed`).
5. **Cite sources.** Every news claim links its URL; every earnings post links the EDGAR filing and the press release.
6. **Versioned templates.** Each content type has a template ID and version (e.g., `premarket:v1`) stored on the post, so changes can be traced (AlphaLab does this with an HTML comment).
7. **Idempotent runs.** A run is keyed by (content type, ET trading date, ticker if any). Re-running replaces the post instead of creating a duplicate (AlphaLab's duplicate `-2` slugs came from this).
8. **Market calendar.** Premarket and recap skip NYSE holidays and handle early-close days.

## 9. Pages

- Home (per language): today's premarket brief at the top; latest recap; latest earnings checks; link to the calendar.
- Post page: title, ET timestamp, body, charts, sources, disclaimer, language switch.
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
- Watchlist size for earnings checks at launch (suggest S&P 100).
- Alerting channel for failures.
- Whether to use the native-binary fallback for any part of the generator.

## 12. Open for the owner

- Site name and domain: PreBell (盘前铃), prebell.ai (domain not yet bought). Use a config value for the EDGAR User-Agent contact email.
- Whether premarket/recap pages are indexed by search engines (AlphaLab hides its digests with `noindex`). Designer's default: index them.

## 13. Build approach (owner)

Implementation will be split across parallel subagents (site, generator core, data providers, earnings/calendar, competitor watch), with a separate review pass.
