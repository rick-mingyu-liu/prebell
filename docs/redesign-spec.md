# PreBell site redesign: design

Status: draft for owner review, 2026-10-30. Scope: `apps/web` only.

## Why

The first site worked but looked like a default blog. Every number on it is verified, yet gains and losses looked the same and the page didn't show the market at a glance. The owner's direction: **Robinhood's clean look with green/red numbers, in a portal layout: earnings strip under the header, daily brief as the main block, partner links in a right sidebar.** The reference mockups are in `.superpowers/brainstorm/98568-1793328857/content/` (`homepage-portal-v2.html`, `brief-page.html`; earlier options in the same folder).

## Goals

- A reader sees the market in about five seconds: index futures, the top movers with green/red pills, and the reason for each move.
- The brief page reads in this order: **numbers, then movers, then the story** (owner's choice).
- PreBell looks like its own brand (black/white plus Robinhood green #00C805 / red #FF5000). It borrows a portal *structure*, not another site's look.
- Partner revenue: broker and crypto invite links in the sidebar, shown only where they are allowed, with a clear affiliate disclosure.
- Cross-promotion for TermPocket (the owner's other product).
- English and Simplified Chinese stay full equals in layout.

## Non-goals

- No crypto content section. The crypto exchanges appear only as sidebar invite links.
- No AI articles, FIRE calculator or other tool sections (possible later).
- No backend changes except the optional chart, which is deferred below. The generator, D1 schema, crons and number checks don't change.
- Company and partner logos are bundled static files (see `docs/runbook.md` → Logos); a ticker without a licensed logo shows a text chip. Logos identify companies only (nominative use) and are never fetched from third parties at runtime.

## Anti-goals (would make this a failure even if it "works")

- Showing a number that doesn't come from the stored facts, or a color that contradicts the sign (green on a negative change).
- Crypto or broker invite links shown to visitors in mainland China, or Wealthsimple shown outside Canada.
- Affiliate links without the disclosure, or without `rel="sponsored"`.
- Pages that become slow or break the Workers Free limits (each page view must stay a handful of D1/R2 reads).
- Copying another site's visual identity, wording or logos.

## Data the pages use

- **D1** (already used): posts, translations (`title`, `summary`, `body_md`), the earnings calendar and the earnings checks.
- **R2 `FACTS` bucket** (already bound in `apps/web/wrangler.jsonc`, currently used only for chart SVGs): each post's facts JSON at `posts.facts_r2_key`. Premarket facts (`crates/core/src/facts.rs::PremarketFacts`) carry `futures: [(label, Field<Quote>)]`, `gainers`/`losers: Mover{quote{symbol,last,prev_close,change_pct}, name, reasons[{url,title,quote,summary_en}]}`, `macro_items` and `as_of_utc`. Recap facts (`crates/core/src/recap/mod.rs::RecapFacts`) carry `indexes`, `gainers`, `losers`, `after_hours` and `early_close`. The new numbers and movers blocks render from these facts. If the facts object is missing or malformed, the page falls back to the current markdown rendering, so it never shows an error.
- `Field` is serialized by serde as `{"Value": ...}` or `{"Missing": {"reason": ...}}`. A missing value renders as "—" in muted text, never as a colored number.
- **Mover reasons are English-only** (`summary_en`). On ZH pages the movers list shows ticker, name, price and pill, with no reason line. (A Chinese reason needs backend work and is tracked separately.) The full story body still comes from the ZH `body_md`.
- **The S&P intraday chart is deferred.** No premarket price series is stored yet, and fetching one costs a Yahoo subrequest (a brief currently uses 41 of 50). The redesign reserves the spot in the layout. It ships with the big number and change only, and adding the chart is a separate backend change.

## Pages

### Shared shell (all public pages)
- **Header:** black bar with the wordmark "Pre**Bell**" ("Bell" in green) and 盘前铃 underneath, nav (Briefs/简报, Earnings/财报, Calendar/日历, Archive/归档, About/关于) and a pill-shaped language switch that keeps the current page.
- **Earnings strip** under the header on the homepage and brief pages: "Earnings today · {date}", split into before-open / after-close / time-unconfirmed groups from the calendar, each ticker a chip linking to its ticker page or earnings check. Reported entries that have a check link to it. Ends with "Full calendar →". Hidden when the day has no entries. It scrolls horizontally on phones.
- **Right sidebar** (desktop ≥1024px; on smaller screens it moves below the main content):
  - "Brokers / 券商": **moomoo** `https://j.moomoo.com/0J1BGf` (hidden for CN), **Wealthsimple** `https://wealthsimple.com/invite/QDBQ5Z` (shown only for CA).
  - "Crypto exchanges / 加密交易所": **OKX** `https://okx.com/join/AIAFMP3U`, **Crypto.com** `https://crypto.com/app/smqqda7396` (both hidden for CN).
  - A box that ends up with no links is not rendered.
  - Disclosure under the boxes (EN: "Affiliate links: PreBell may earn a reward if you sign up. Not investment advice." ZH: "推广链接：通过链接注册，PreBell 可能获得奖励。不构成投资建议。").
  - Links open in a new tab with `rel="sponsored noopener noreferrer"`.
  - **TermPocket card:** dark navy card, "TermPocket", one line of copy (EN: "From the PreBell team. Plan your goals and habits with Pocky, your AI assistant." ZH: "来自 PreBell 团队。用 AI 助手 Pocky 规划你的目标与习惯。"), and a button linking to `https://www.termpocket.com`. Shown in every country.
- **Country:** read from Cloudflare's request country (`cf-ipcountry` header / `request.cf.country`). Unknown country = show everything except Wealthsimple. All partner data lives in one typed module (e.g. `src/lib/partners.ts`) with `allowedIn(country)` rules, so links can be changed in one place.
- **Footer:** the disclaimer, plus small About and language links.

### Homepage (`/[locale]`)
Three columns on desktop (main-left, main-middle, sidebar); single column on mobile in this order: brief block, movers, earnings checks, sidebar.
- **Main-left, the brief block:**
  - Tabs: "Premarket brief / 盘前简报" and "After-close recap / 收盘回顾" (whichever is latest is selected by default; switching tabs needs no client JS beyond a simple toggle, or uses two links).
  - S&P 500 futures label with time (ET), big number, and absolute and % change in green/red.
  - Reserved chart area (empty until the chart ships, so no layout jump later).
  - Three tiles for Nasdaq 100, Dow and Russell 2000 (value plus colored %).
  - Headline, summary, "Read the full brief →".
  - For the recap the same block uses the index closes.
- **Main-middle:** tabs "Top movers / 涨跌榜" and "Earnings checks / 财报体检".
  - Top movers: the top 3 gainers and top 3 losers, interleaved by size of move or grouped up/down (builder's call). Each row has the ticker bold, the reason line (EN only), and a pill with the %.
  - Earnings checks: the latest earnings-check card(s) with title and summary, linking to the check.
  - "All earnings checks →".
- **Recent posts** list below (as today), restyled.
- Pending ZH translation keeps its current notice behavior.

### Brief page (`/[locale]/p/[slug]`, premarket and recap)
1. Breadcrumb (Briefs › Premarket › date), publish time in ET, a green **"✓ Numbers checked against source data / ✓ 数字已与源数据核对"** badge, and the language pill.
2. **Numbers:** the big S&P futures number (premarket) or S&P close (recap) with the change; the other three as compact rows or tiles.
3. **Movers:** every gainer and loser from the facts, with ticker, company name, reason line (EN), price and a green/red pill. A mover without a reason shows "No clear news" in muted text (EN only).
4. **Story:** headline (h1) and summary, then `body_md` rendered **with its leading H1, summary paragraph and the markdown tables removed** (they are now shown above), so the prose sections, Sources and disclaimer remain. Percentages in the prose are colored green/red when they carry an explicit sign or follow "up/rose/gained" vs "down/fell/dropped" (EN) or 涨/跌 (ZH). Only plain coloring, never changing the text. If this is unreliable, color only explicitly signed numbers.
5. Previous/next brief links.
6. Earnings-check posts keep their current layout (charts), restyled with the new tokens.

### Other pages
Archive, Earnings calendar, ticker pages (`/t/[ticker]`), About and not-found use the new shell, typography and colors, and lists use the movers-row style where it fits.

## Visual system
- **Theme:** follows the reader's system setting.
  - Light: white page, black header, #111 text.
  - Dark: #000 page, #fff text, the same header.
  - Up = #00C805 (text on light uses the darker #00A804 for contrast; pills use #00C805 with black text). Down = #FF5000 (text on light #E64800; pills #FF5000 with black text). Unchanged/missing = muted grey.
- **Type:** Inter (already bundled), tabular numbers for all prices and percentages. Big number 30–36px bold, headlines 22–26px extra-bold, body 15–16px.
- Rounded corners (6–12px), light grey tiles, thin dividers, generous whitespace. Links aren't blue-underlined; the green accent is used for "Read more →" style links.
- Accessibility: color is never the only signal (signs +/− always shown), contrast meets WCAG AA in both themes, and the pills keep the text sign.

## Calls made by Claude (owner to confirm while reading)
1. Theme follows system (light and dark), not a single fixed theme.
2. The S&P chart is deferred (backend change, one more Yahoo request); the layout reserves its space.
3. The movers block reads the R2 facts JSON instead of parsing markdown tables, with a markdown fallback.
4. The written "Biggest premarket movers" section stays in the story for now, because it carries detail the one-line reasons don't (EPS figures, guidance). It may be trimmed after the redesign ships.
5. The country rules: moomoo hidden in CN, Wealthsimple only in CA, OKX/Crypto.com hidden in CN; unknown country shows everything except Wealthsimple. The owner should still check each program's terms.
6. TermPocket copy as written above.

## Decisions from the builder check
1. **Country gating vs caching.** Pages stay cacheable and country-neutral. The partner boxes are a small client island that fetches an uncached `/api/partners` route. The route reads `cf-ipcountry` / `request.cf.country`, returns only the links allowed for that country, sends `Cache-Control: private, no-store` and does no D1/R2 work. Without JS, or while loading, the broker and crypto boxes don't render (fails closed). The TermPocket card and the disclosure are static. A gated link never appears in server-rendered HTML.
2. **Nav targets.**
   - Briefs/简报 → `/archive` (premarket + recap, existing filters).
   - Earnings/财报 → `/archive?type=earnings`: extend `parseContentType` and the archive query to accept `earnings`. "All earnings checks →" also links here.
   - Calendar/日历 → the existing `/earnings` weekly calendar page.
   - About/关于.
   - There is no separate "Archive" item; Briefs is the archive.
3. **Derived numbers.**
   - The point change is computed in the UI as `last − prev_close`, rounded to 2 decimals (the same precision as `last`).
   - Sign and color come from `change_pct`; a value that rounds to 0.00% renders grey with no sign.
   - Prices use 2 decimals with thousands separators, and percentages use 2 decimals with an explicit sign.
4. **Which facts feed the homepage.**
   - The brief block defaults to the most recently published of the latest premarket and latest recap. The movers panel always follows the brief tab that is selected.
   - When the ZH translation is pending, numbers and movers still render, since facts are language-neutral. Only the headline/summary/story area shows the existing pending notice.
4b. **Mover counts.** The homepage shows up to 3 per side; the brief page shows all of them (up to 5 per side).
5. **Earnings strip date.**
   - On the homepage: the next trading day with calendar entries, starting from today's ET date, labelled with that date. On a weekend it shows Monday, labelled "Earnings Mon, Nov 2".
   - On a brief page: that brief's trading date.
   - The strip is hidden when there are no entries.
6. **Local preview fixtures.** The seed's R2 keys don't match what the generator writes (`premarket/{date}.json`). Add facts fixtures at the generator's real key format, plus a script to load them into local R2, so the preview shows the new blocks.
7. **Index and futures matching** uses `Quote.symbol` (ES=F, NQ=F, YM=F, RTY=F; ^GSPC, ^IXIC, ^DJI, ^RUT for the recap), not the English labels. Chinese display names (标普500期货 etc.) come from i18n messages.

## Left to the builder
Component breakdown, CSS approach (the app uses plain CSS in `globals.css`; keep it plain CSS with custom properties, no new UI framework), exact spacing, tab mechanics, test coverage for parsing facts, country gating and markdown trimming, and visual QA screenshots of the homepage and brief page in both themes and at phone width.
