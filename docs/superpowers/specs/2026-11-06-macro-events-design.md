# Macro events on the calendar and in "What to watch"

Date: 2026-11-06. Rick: "we can also add the big events into the calender.
such as the us midterm election, the Fed's news in advance". Design only;
decisions marked **[Rick]** need him, each with a recommendation.

## Goals

- The calendar shows scheduled market-moving events (Fed, main data
  releases, closures, elections, dated political deadlines) next to
  earnings, each with ET time, importance tier and official proof link.
- The brief's "What to watch" (stub: `crates/core/src/premarket/mod.rs:710`,
  `crates/core/src/facts.rs:97`) lists today's events, so it publishes.
- A moved or postponed date (a shutdown) shows as such, never a silent
  overwrite; a release that came out links to it.

Non-goals: consensus forecasts; event detail pages; unscheduled news (see
"Later: breaking news"); any calendar outside the US.

## Sources (fetched 2026-11-06 from the build machine)

All but ISM, UMich and S&P are US government works (public domain, 17 U.S.C.
105): dates, titles and the released figures may be republished.

| Kind | Official source | Format | Published ahead | Fetch notes |
|---|---|---|---|---|
| FOMC meetings (stmt 2:00 PM, presser 2:30 PM, `*` = SEP) | federalreserve.gov/monetarypolicy/fomccalendars.htm | HTML, `div.fomc-meeting` rows per year panel | 2027 listed (page updated 2026-10-28); 2028 note: Jan 25-26. "Tentative until confirmed at the meeting immediately preceding it" | 166 KB; no robots.txt; 200 to plain GET |
| FOMC minutes, Beige Book, Board speeches/testimony | federalreserve.gov/newsevents/YYYY-month.htm (slugs vary: `2026-04.htm`, `2026-may.htm`; follow links on /newsevents/calendar.htm) | HTML, JS-free fallback | Minutes/Beige Book a year+ (Dec 2026 page updated 2025-06-24); speeches ~1 week (on 11-06 the Nov page lists through 11-13). Board members only, no Reserve Bank presidents | ~80 KB/month |
| Fed "released" proof | federalreserve.gov/feeds/press_monetary.xml; statement URL `/newsevents/pressreleases/monetaryYYYYMMDDa.htm` | RSS / deterministic | at release | verified for 2026-10-28 |
| BLS: jobs, CPI, PPI, JOLTS, ECI (8:30; JOLTS 10:00) | bls.gov/schedule/news_release/bls.ics (all BLS) and bls.gov/schedule/news_release/{empsit,cpi,ppi,jolts,eci}.htm | ICS (TZID US-Eastern, UID+SEQUENCE, no URL) / HTML table | ICS: through 2026-12-30 only. Per-release pages: CPI, PPI, jobs, ECI through late 2027; JOLTS only the next date (2026-12-01) | **403 to browser and default UAs; 200 with a UA naming a contact.** Worker egress untested |
| BLS "released" proof | bls.gov/news.release/archives/{empsit,cpi,ppi,jolts,eci}_MMDDYYYY.htm | deterministic HTML | at release | verified empsit_11062026, cpi_10142026 |
| BEA: GDP (adv/2nd/3rd), Personal Income & Outlays (PCE) 8:30 | apps.bea.gov/API/signup/release_dates.json; bea.gov/news/schedule/ics/online-calendar-subscription.ics | JSON (UTC instants, no estimate name) / ICS (names "GDP (Second Estimate)...") | through 2027-12-22 | 12 KB / 43 KB; GDP and PCE now share dates (e.g. 2026-11-25) |
| BEA "released" proof | apps.bea.gov/rss/rss.xml (item title = release title, link = release page) | RSS | at release | |
| Census: retail sales, durable goods, housing starts (8:30) | census.gov/economic-indicators/calendar-listview.html (2027), .../calendar-listview-2026.html | HTML table, row id `AYYYYMMDDHHMM` | full calendar year; 2027 already up | ~90 KB; indicator.xml feed is stale (2021) |
| Jobless claims (Thu 8:30) | oui.doleta.gov/unemploy/claims_arch.asp; latest release dol.gov/ui/data.pdf | rule + HTML exception list | exceptions listed through 2027-11-24 (Wed 2026-11-25, 2027-11-10, 2027-11-24) | dol.gov newsroom pages 403 |
| ISM PMIs (mfg 1st business day, services 3rd, 10:00) | ismworld.org .../rob-report-calendar/ | HTML behind reCAPTCHA | year | not scrapable; PMI figures are ISM copyright |
| UMich sentiment (prelim/final 10:00) | data.sca.isr.umich.edu/releases.php | HTML list of past reports | no forward schedule found | copyright + usage agreement |
| Treasury quarterly refunding | each refunding statement names the next date (home.treasury.gov press releases) | prose | ~3 months | manual |
| Monthly OPEX / quarterly expiration; S&P 500 rebalance | rule: 3rd Friday (Thursday if a holiday); rebalance effective before Monday's open after the 3rd Friday of Mar/Jun/Sep/Dec, announced ~2 weeks earlier by S&P DJI press release (Dec 2026: Dec 18 ref, Dec 21 effective, expected) | rule | n/a | S&P names are trademarks; date only |
| Market closed / early close | `crates/core/src/clock.rs:26-49` (2026-2027) | Rust table | already | already the trading-day source |
| Federal elections | 2 U.S.C. 7 (Tuesday after the first Monday in November, even years): 2026-11-03 (three days ago), next 2028-11-07; fec.gov | statute | years | manual |
| Funding deadlines | the enacted CR text on congress.gov (reported: FY2027 CR through 2026-12-11; verify before entering) | prose | weeks | manual; debt-ceiling "X-dates" are CBO/BPC estimates, not dates: excluded |

How often they change: rarely, but in bulk. The 2025-10-01..11-12 funding
lapse cancelled six weeks of claims releases (DOL notice on claims_arch.asp)
and BEA's JSON still shows the gap (trade: 2025-09-04, then 2025-11-19). A
lapse after 2026-12-11 would hit PPI (12-15), retail (12-16), GDP/PCE
(12-23). FOMC dates move only on tentative-to-confirmed or emergencies.

## Event model

```
MacroEvent {
  id: "bls:cpi:2026-10" | "fed:fomc:2026-12" | "dol:claims:2026-11-25"
      (source:series:reference period; stable when the date moves)
  series: glossary key (titles EN/ZH come from code, not the row)
  ref_period: "2026-10" | "2026-Q3" | null;  variant: advance|second|third|sep|null
  starts_at: UTC instant, null = all-day;  et_date: YYYY-MM-DD;  tier: 1|2|3
  status: scheduled | confirmed | moved | postponed | released
  proof_url: official schedule page;  release_url: the release, once out
  moved_from: previous et_date while moved
}
```

- **scheduled**: on the agency's published schedule (primary proof already).
  **confirmed**: re-seen in the live source within 10 trading days; FOMC
  also needs the preceding meeting to have happened (the Fed's own rule).
- **moved**: same id on another date; "Moved from Dec 10" until it passes.
- **postponed**: missing from a source that otherwise parsed, on **two**
  consecutive checks (one bad parse never cancels); a later sighting makes
  it moved/confirmed. **released**: the release URL answers 200 or the RSS
  item appears; never inferred from the clock.

Tiers (v1): **1** FOMC decision (+SEP/presser), jobs, CPI, GDP advance, PCE
month, Chair testimony, federal election, funding deadline. **2** PPI,
retail sales, JOLTS, ECI, claims, FOMC minutes, Chair speech, GDP 2nd/3rd,
quarterly expiration, S&P rebalance, market closed/early close. **3**
durables, housing starts, Beige Book, monthly OPEX. Tiers are a code table.

## Storage: a new table, not `earnings_calendar`

`earnings_calendar` (`migrations/0003_earnings.sql:35-49`) is keyed
`(ticker, report_date)` with CHECKs on `time_slot` (bmo/amc/unknown) and
`source` (yahoo/ir/edgar); reusing it means fake tickers and widened CHECKs
that the web queries (`apps/web/src/lib/earnings.ts:136-151`) and
reconciliation would have to filter everywhere. Recommend
`migrations/0008_macro_events.sql`:

```sql
CREATE TABLE macro_events (
  id TEXT PRIMARY KEY, series TEXT NOT NULL, ref_period TEXT, variant TEXT,
  starts_at TEXT, et_date TEXT NOT NULL,
  tier INTEGER NOT NULL CHECK (tier IN (1,2,3)),
  status TEXT NOT NULL CHECK (status IN
    ('scheduled','confirmed','moved','postponed','released')),
  proof_url TEXT NOT NULL, release_url TEXT, moved_from TEXT,
  missing_count INTEGER NOT NULL DEFAULT 0,
  first_seen_at TEXT NOT NULL, checked_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX idx_macro_events_date ON macro_events(et_date);
```
Additive; Rick applies it to remote D1 before deploying (as with 0007).

## Ingestion: committed snapshot + small daily live check

1. **Snapshot** `scripts/build-events.mjs` -> `crates/core/data/events.tsv`
   (like `scripts/build-watchlist.mjs`: contact UA, cache, `--offline`,
   deterministic bytes). Reads the FOMC page, Fed month pages, BLS
   per-release pages + ICS, BEA JSON + ICS (for the estimate names), Census
   2026/2027 lists, the DOL rule + exceptions, the OPEX/rebalance rules, and
   `crates/core/data/events-manual.tsv` (elections, funding deadlines,
   refunding; a row without an official `proof_url` stops the run). Prints a
   moved/new/removed diff against the committed file for review. Covers
   today .. +13 months. Run monthly and whenever a shutdown looms
   (`docs/runbook.md`).
2. **Load**: the generator embeds the TSV (`include_str!`, as the watchlist)
   and a daily `events:check` job upserts rows whose content changed (one
   D1 read of the window, one batch of changed rows). Closures and early
   closes are written from `clock.rs` tables, one source of truth. Wiring
   follows `crates/core/src/schedule.rs:6` and the once-a-day KV pattern of
   `crates/core/src/earnings/calendar.rs:642`; the queue is the builder's.
3. **Live check, same job, next 10 trading days only**: BLS ICS, BEA JSON,
   FOMC page (current-year panel only), current Census list, current Fed
   month page: 5 subrequests, under 50 D1 queries. Diff by id -> confirmed /
   moved / postponed. A fetch that fails or parses to nothing is a
   diagnostic, never a status change. CPU: ~400 KB of HTML in Wasm is close
   to the free plan's 10 ms; the builder measures and, if needed, splits
   sources across messages or narrows each parse to its panel.
4. **Released check**: on the existing `*/5` cron, for any event in the last
   45 minutes not yet released, one GET of its release URL (BLS archive
   pattern, Fed `monetaryYYYYMMDDa.htm`, BEA RSS, Census indicator page,
   `dol.gov/ui/data.pdf` dated by its text). At most a few per day.

If the Worker cannot reach BLS (UA or IP block), BLS rows stay `scheduled`
from the snapshot and link the schedule page instead of the release.

## Calendar page

`/[locale]/earnings` (`apps/web/src/app/[locale]/earnings/page.tsx`) gains a
sibling query `listEventsWeek(db, monday)` (one D1 query, tiers 1-2; tier 3
folded behind "Show N more" as long slots already are,
`apps/web/src/components/CalendarWeek.tsx:9`). Each day: an **"Economy &
Fed" / "经济与美联储"** group **above** the earnings slots, events by time:
`8:30 AM  October CPI  [Major]  Schedule ↗` -> once released `✓ Release ↗`;
moved: "Moved from Dec 10"; postponed: "Postponed" + schedule link. Tier 1
gets a text badge ("Major" / "重要"), not colour alone, matching the page's
existing rule. All-day events show without a time. The legend gains one
sentence on proof and status. No event detail page in v1: the official
release is the detail.

## Premarket "What to watch"

- `watch_list: Field<Vec<String>>` becomes `Field<Vec<WatchItem>>`
  (`{id, et_time, title, tier, status, release_url}`, title in the brief's
  language from the glossary). Old R2 facts hold `Missing`, which still
  deserialises.
- Built from today's tier 1-2 events (plus, **[Rick] 6**, write-up earnings
  due after today's close). Empty -> `Missing`, and the section drops as it
  does now (`crates/core/src/premarket/draft.rs:58-70`).
- Digits in times and periods ("8:30", "Q3", "2026") must be allowed
  figures: extend `numbers_in_facts` (`crates/core/src/facts.rs:171`) with
  each item's hour, minute and period year/quarter/month.
- **8:00 brief**: "CPI for October at 8:30 AM ET". **8:35 update**: runs the
  released check first, so items read "released at 8:30" with the link;
  the figure itself reaches the text only via the existing macro news search.
  With the calendar known, **[Rick] 4**: run the update only on days with a
  tier 1-2 event at or before 8:30.

## Bilingual glossary (code table, never LLM-translated)

| key | EN | ZH |
|---|---|---|
| fomc | FOMC rate decision | 联邦公开市场委员会（FOMC）议息会议 |
| fomc_presser / sep | Fed Chair press conference / with projections (dot plot) | 美联储主席新闻发布会 / 含经济预测（点阵图） |
| fomc_minutes / beige_book | FOMC minutes / Beige Book | FOMC会议纪要 / 美联储褐皮书 |
| chair_speech / chair_testimony | Fed Chair speaks / testifies to Congress | 美联储主席讲话 / 国会证词 |
| jobs / claims | Jobs report (nonfarm payrolls) / Weekly jobless claims | 非农就业报告 / 每周初请失业金人数 |
| cpi / ppi | CPI (consumer prices) / PPI (producer prices) | 消费者物价指数（CPI） / 生产者物价指数（PPI） |
| jolts / eci | JOLTS job openings / Employment Cost Index | 职位空缺（JOLTS） / 就业成本指数（ECI） |
| gdp (advance/second/third) | GDP, advance / second / third estimate | GDP初值 / 修正值 / 终值 |
| pce | Personal income & spending (PCE prices) | 个人收入与支出（PCE物价指数） |
| retail / durables / starts | Retail sales / Durable goods orders / Housing starts | 零售销售 / 耐用品订单 / 新屋开工 |
| quad_witching / opex | Quarterly options expiration / Monthly options expiration | 季度期权到期日（三巫日） / 月度期权到期日 |
| sp_rebalance | S&P 500 quarterly rebalance takes effect | 标普500季度成分股调整生效 |
| closed / early_close | Market closed / Early close (1:00 PM) | 美股休市 / 美股提前收盘（13:00） |
| election / midterm | US general election / US midterm elections | 美国大选 / 美国中期选举 |
| funding_deadline | Federal funding deadline (shutdown risk) | 联邦政府拨款到期（停摆风险） |

Periods: EN "October CPI", "Q3 GDP, advance estimate"; ZH "10月CPI", "三季度GDP初值".
Titles name offices, not people ("Fed Chair"), so a new Chair needs no edit.

## Later: breaking news (out of scope; interface only)

Rick's separate idea (sudden news added to posts) would need a trigger
source (e.g. Fed `press_all.xml`), an unscheduled `macro_events` row
(status `released`, no prior schedule) and a post revision path; the
premarket update's `posts.revised_at` + per-revision facts key
(`2026-11-06-premarket-update-design.md`) is that primitive, appending a
dated block instead of rebuilding. Nothing in v1 blocks it.

## Decisions for Rick

1. **v1 event kinds.** Rec: Fed (FOMC, SEP, presser, minutes, Chair
   speeches/testimony), jobs, CPI, PPI, JOLTS, ECI, GDP, PCE, retail,
   durables, housing starts, claims, closures, elections, funding deadlines
   (manual, congress.gov proof), quarterly expiration + S&P rebalance. Defer
   ISM (captcha), UMich (no forward schedule), refunding, other Fed speakers.
2. **Figures.** Rec: never consensus/forecasts (Bloomberg, LSEG, Dow Jones
   polls are licensed). v1 shows no actuals either, only the release link;
   actuals from the official release (public domain) can follow with
   anchored extraction like earnings.
3. **Page name.** Rec: keep the `/earnings` URL; H1 "Market calendar" /
   "市场日历" (the nav already says Calendar / 日历). User-visible wording.
4. **8:35 update only on 8:30-release days.** Rec: yes (saves Tavily
   searches on quiet days; reverses part of the 2026-11-06 design's
   "runs every trading day").
5. **Political deadlines.** Rec: funding deadlines yes, neutral wording,
   enacted-law proof only; debt-ceiling X-dates no (estimates).
6. **Earnings in "What to watch".** Rec: yes, write-up-tier companies
   reporting after today's close (already in D1, no new fetch).
