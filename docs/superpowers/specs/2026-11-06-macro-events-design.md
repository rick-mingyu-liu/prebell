# Macro events on the calendar and in "What to watch"

Date: 2026-11-06. Rick: "we can also add the big events into the calender.
such as the us midterm election, the Fed's news in advance". Scope, Rick
2026-11-06: "for now just only the market-moving events (Fed decisions,
jobs, CPI, elections). we will decide if we want to add the fuller later".
Design only; open decisions marked **[Rick]**, each with a recommendation.

## Goals

- **v1 kinds: FOMC decision day (statement 2:00 PM ET + press conference,
  projections flag), the jobs report (BLS Employment Situation), CPI, US
  federal elections**, plus closures/early closes, free from `clock.rs`
  (designer's call: no fetch, no new source; drop if unwanted).
- The calendar shows them next to earnings, each with ET time, importance
  tier and an official proof link, as every earnings date already has.
- The brief's "What to watch" (stub: `crates/core/src/premarket/mod.rs:710`,
  `crates/core/src/facts.rs:97`) lists today's events, so it can publish.
- A moved or postponed date (a shutdown) shows as such, never a silent
  overwrite; a release that came out links to it.
- The model, table and glossary hold every later kind (see "Later") with
  no schema change: adding one is a parser, a glossary row and a tier.

Non-goals: consensus forecasts; event detail pages; unscheduled news (see
"Later: breaking news"); any calendar outside the US.

## v1 sources (fetched 2026-11-06 from the build machine)

All are US government works (public domain, 17 U.S.C. 105): dates, titles
and the released figures may be republished.

| Kind | Official source | Format | Published ahead | Fetch notes |
|---|---|---|---|---|
| FOMC meetings (statement 2:00 PM ET, press conference 2:30 PM, `*` = with projections) | federalreserve.gov/monetarypolicy/fomccalendars.htm | HTML, `div.fomc-meeting` rows (`__month`, `__date` "27-28", `*`) per year panel | 2027 listed (page updated 2026-10-28); 2028 note: Jan 25-26. "Each meeting date is tentative until confirmed at the meeting immediately preceding it" | 166 KB; no robots.txt; 200 to a plain GET |
| Fed released proof | statement `/newsevents/pressreleases/monetaryYYYYMMDDa.htm` | deterministic HTML | at release | verified for 2026-10-28 |
| Jobs report, CPI (8:30 AM) | bls.gov/schedule/news_release/empsit.htm and cpi.htm | HTML table: reference month, release date, time | jobs through 2027-12-03, CPI through late 2027 (the all-BLS `bls.ics` runs only to 2026-12-30: not used) | **403 to browser and default UAs; 200 with a UA naming a contact** (e.g. `PreBell (https://prebell.site; support@prebell.site)`, as the watchlist script). Worker egress untested |
| BLS released proof | bls.gov/news.release/archives/{empsit,cpi}_MMDDYYYY.htm | deterministic HTML | at release | verified empsit_11062026, cpi_10142026 |
| Federal elections | 2 U.S.C. 7 (Tuesday after the first Monday in November, even years; uscode.house.gov), fec.gov | statute | years | manual file |
| Market closed / early close | `crates/core/src/clock.rs:26-49` (2026-2027) | Rust table | already | no fetch; proof link: nyse.com hours and calendars page |

Upcoming v1 dates: CPI 2026-11-10, 2026-12-10; jobs 2026-12-04,
2027-01-08; FOMC 2026-12-08/09 (projections), 2027-01-26/27. Elections:
the 2026 midterm was 2026-11-03, three days ago, so the only future row is
2028-11-07. The manual elections file stays (one line per election), but
plainly it adds almost nothing to the calendar or the brief until 2028.

How often they change: rarely, but in bulk. The 2025-10-01..11-12 funding
lapse cancelled six weeks of DOL claims releases (oui.doleta.gov notice).
The current CR reportedly runs to 2026-12-11, so a lapse would first hit
the jobs report of 2027-01-08: the "postponed" path is for this. The Fed is
self-funded; FOMC dates move only tentative -> confirmed, or in emergencies.

## Later (not v1; shows the model holds them)

| Kind | Source, format, lead | Note |
|---|---|---|
| FOMC minutes, Beige Book (2:00 PM) | federalreserve.gov/newsevents/YYYY-month.htm (slugs vary: `2026-04.htm`, `2026-may.htm`), a year+ ahead; released: feeds/press_monetary.xml | minutes ~3 weeks after a meeting |
| Chair speeches/testimony | same month pages, ~1 week ahead, Board members only | |
| PPI, JOLTS, ECI | same BLS per-release pages | JOLTS page shows only the next date |
| GDP, PCE | apps.bea.gov/API/signup/release_dates.json (UTC, through 2027-12) + BEA ICS (estimate names); released: apps.bea.gov/rss/rss.xml | GDP and PCE now share dates |
| Retail, durables, housing starts | census.gov/economic-indicators/calendar-listview(-2026).html, HTML, full year | indicator.xml is stale (2021) |
| Jobless claims | rule (Thu 8:30) + DOL exception list (through 2027-11-24) | |
| ISM, UMich | ISM page behind reCAPTCHA; UMich no forward schedule | both copyright; dates only |
| Funding deadlines | enacted CR on congress.gov, manual | debt-ceiling X-dates are estimates: never |
| OPEX, S&P rebalance, refunding | rules / S&P DJI press / Treasury statements, manual | |

## Event model

```
MacroEvent {
  id: "bls:cpi:2026-10" | "fed:fomc:2026-12" | "us:election:2028-11-07"
      | "nyse:closed:2026-11-26"  (source:series:period; stable on a move)
  series: glossary key (titles EN/ZH come from code, not the row)
  ref_period: "2026-10" | "2026-Q3" | null;  variant: sep|advance|...|null
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
  it moved/confirmed. **released**: the release URL answers 200; never
  inferred from the clock. Elections and closures: scheduled only.

Tiers: FOMC, jobs, CPI, elections tier 1; closures tier 2. Later kinds get
2 or 3 (minutes, PPI, retail, claims 2; housing, durables 3). Tiers are a
code table, so the page and the brief filter by tier, not by kind.

## Storage: a new table, not `earnings_calendar`

`earnings_calendar` (`migrations/0003_earnings.sql:35-49`) is keyed
`(ticker, report_date)` with CHECKs on `time_slot` (bmo/amc/unknown) and
`source` (yahoo/ir/edgar); reusing it means fake tickers and widened CHECKs
that the web queries (`apps/web/src/lib/earnings.ts:136-151`) and
reconciliation would filter everywhere. `migrations/0008_macro_events.sql`:

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
`series` is free text (no CHECK): later kinds need no migration. Additive;
Rick applies it to remote D1 before deploying (as with 0007).

## Ingestion: committed snapshot + small daily live check

1. **Snapshot** `scripts/build-events.mjs` -> `crates/core/data/events.tsv`
   (like `scripts/build-watchlist.mjs`: contact UA, cache, `--offline`,
   deterministic bytes). v1 fetches 3 pages (FOMC calendar, BLS
   `empsit.htm`, `cpi.htm`) and reads `crates/core/data/events-manual.tsv`
   (elections; a row without an official `proof_url` stops the run). One
   parser per source: a later kind is one more parser. Prints a
   moved/new/removed diff for review; covers today .. +13 months; run
   monthly and whenever a shutdown looms (`docs/runbook.md`).
2. **Load**: the generator embeds the TSV (`include_str!`, as the watchlist)
   and a daily `events:check` job upserts changed rows (one D1 read of the
   window, one batch) and generates closures from `clock.rs`, no fetch.
   Wiring follows `crates/core/src/schedule.rs:6` and the once-a-day KV
   pattern of `crates/core/src/earnings/calendar.rs:642`.
3. **Live check, same job, next 10 trading days**: the same 3 pages with
   the contact UA: **3 subrequests** of 50, 2-3 D1 queries. Diff by id ->
   confirmed / moved / postponed; a failed or empty parse is a diagnostic,
   never a status change. CPU: measure it; ~200 KB of HTML is small next
   to the ~620 ms a recap invocation already uses here. Not a blocker.
4. **Released check**: on the existing `*/5` cron, for an event in the last
   45 minutes not yet released, one GET of its release URL (BLS archive
   pattern; Fed `monetaryYYYYMMDDa.htm`). Per trading day: 3 + 0-2 fetches.

If the Worker cannot reach BLS (UA or IP block), jobs and CPI rows stay
`scheduled` from the snapshot and link the schedule page, not the release.

## Calendar page

`/[locale]/earnings` (`apps/web/src/app/[locale]/earnings/page.tsx`) gains a
sibling query `listEventsWeek(db, monday)` (one D1 query, tiers 1-2). Each
day with events gets an **"Economy & Fed" / "经济与美联储"** group **above**
the earnings slots (`apps/web/src/components/CalendarWeek.tsx`), by time:
`8:30 AM  October CPI  [Major]  Schedule ↗` -> once released `✓ Release ↗`;
`2:00 PM  FOMC rate decision (with projections)`, then `2:30 PM` press
conference; moved: "Moved from Dec 10"; postponed: "Postponed" + schedule
link. Tier 1 gets a text badge ("Major" / "重要"), not colour alone, as the
page already does. All-day events (elections, closures) show without a
time. The legend gains one sentence on proof and status. No event detail
page: the official release is the detail. Most weeks: zero to two events.

## Premarket "What to watch"

- `watch_list: Field<Vec<String>>` becomes `Field<Vec<WatchItem>>`
  (`{id, et_time, title, tier, status, release_url}`, title in the brief's
  language from the glossary). Old R2 facts hold `Missing`, which still
  deserialises.
- Built from today's tier 1 events (plus, **[Rick] 6**, write-up earnings
  due after today's close). Empty -> `Missing`, and the section drops as it
  does now (`crates/core/src/premarket/draft.rs:58-70`). With v1 kinds
  alone that is about 3 trading days a month (jobs, CPI, ~2/3 FOMC).
- Digits in times and periods ("8:30", "2026") must be allowed figures:
  extend `numbers_in_facts` (`crates/core/src/facts.rs:171`) with each
  item's hour, minute and period year/month.
- **8:00 brief**: "October CPI at 8:30 AM ET". **8:35 update**: runs the
  released check first, so the item reads "released at 8:30" with the link;
  the figure reaches the text only via the existing macro news search.

## Bilingual glossary (code table, never LLM-translated)

| key | EN | ZH |
|---|---|---|
| fomc | FOMC rate decision | 联邦公开市场委员会（FOMC）议息会议 |
| fomc_presser / sep | Fed Chair press conference / with projections (dot plot) | 美联储主席新闻发布会 / 含经济预测（点阵图） |
| jobs | Jobs report (nonfarm payrolls) | 非农就业报告 |
| cpi | CPI (consumer prices) | 消费者物价指数（CPI） |
| election / midterm | US general election / US midterm elections | 美国大选 / 美国中期选举 |
| closed / early_close | Market closed / Early close (1:00 PM) | 美股休市 / 美股提前收盘（13:00） |
| later | FOMC minutes; PPI; JOLTS; claims; GDP advance/second/third; PCE; retail sales | FOMC会议纪要; 生产者物价指数（PPI）; 职位空缺（JOLTS）; 每周初请失业金人数; GDP初值/修正值/终值; 个人收入与支出（PCE物价指数）; 零售销售 |

Periods: EN "October CPI"; ZH "10月CPI". Titles name offices, not people.

## Later: breaking news (out of scope; interface only)

Rick's separate idea (sudden news added to posts) would need a trigger
(e.g. Fed `press_all.xml`), an unscheduled `macro_events` row (`released`,
no prior schedule) and a revision path: the premarket update's
`posts.revised_at` + per-revision facts key, appending a dated block
instead of rebuilding. Nothing in v1 blocks it.

## Decisions

1. **Decided, Rick 2026-11-06:** "for now just only the market-moving
   events (Fed decisions, jobs, CPI, elections). we will decide if we want
   to add the fuller later". Closures ride along (designer's call, free).

Open, for Rick:
2. **Figures.** Rec: never consensus/forecasts (Bloomberg, LSEG, Dow Jones
   polls are licensed). v1 shows no actual jobs/CPI numbers either, only
   the release link; actuals from the BLS release (public domain) can
   follow with anchored extraction like earnings.
3. **Page name.** Rec: keep the `/earnings` URL; H1 "Market calendar" /
   "市场日历" (the nav already says Calendar / 日历). User-visible wording.
4. **8:35 update gated on the calendar.** Rec: no for v1; keep it every
   trading day. v1 tracks only two 8:30 releases (jobs, CPI), so gating
   would skip claims, PPI, GDP and retail mornings. Revisit with "Later".
5. **Political events beyond elections.** Rec: none in v1 (Rick's scope).
   If the fuller set is chosen later: funding deadlines with enacted-law
   proof and neutral wording; debt-ceiling X-dates never (estimates).
6. **Earnings in "What to watch".** Rec: yes, write-up-tier companies
   reporting after today's close (already in D1, no new fetch); without it
   the section shows on only ~3 days a month.
