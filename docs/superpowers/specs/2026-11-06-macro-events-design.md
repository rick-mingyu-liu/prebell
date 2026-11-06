# Macro events on the calendar and in "What to watch"

Date: 2026-11-06. Rick: "we can also add the big events into the calender.
such as the us midterm election, the Fed's news in advance". Scope, Rick
2026-11-06: "for now just only the market-moving events (Fed decisions,
jobs, CPI, elections). we will decide if we want to add the fuller later".
Design only; open decisions marked **[Rick]**, each with a recommendation.

## Goals

- **v1 kinds: FOMC decisions (statement, press conference), the monthly
  jobs report (BLS Employment Situation), CPI, US federal elections.**
  Recommended addition: FOMC minutes (**[Rick] 1**).
- The calendar shows them next to earnings, each with ET time, importance
  tier and an official proof link, as every earnings date already has.
- The brief's "What to watch" (stub: `crates/core/src/premarket/mod.rs:710`,
  `crates/core/src/facts.rs:97`) lists today's events, so it can publish.
- A moved or postponed date (a shutdown) shows as such, never a silent
  overwrite; a release that came out links to it.
- The model, table and glossary hold the later kinds (GDP, PCE, PPI, JOLTS,
  claims, retail, housing, Fed speeches, rebalances, ...) with no schema
  change: adding one is a source parser, a glossary row and a tier.

Non-goals: consensus forecasts; event detail pages; unscheduled news (see
"Later: breaking news"); any calendar outside the US.

## v1 sources (fetched 2026-11-06 from the build machine)

All are US government works (public domain, 17 U.S.C. 105): dates, titles
and the released figures may be republished.

| Kind | Official source | Format | Published ahead | Fetch notes |
|---|---|---|---|---|
| FOMC meetings (statement 2:00 PM ET, press conference 2:30 PM, `*` = with projections) | federalreserve.gov/monetarypolicy/fomccalendars.htm | HTML, `div.fomc-meeting` rows (`__month`, `__date` "27-28", `*`) per year panel | 2027 listed (page updated 2026-10-28); 2028 note: Jan 25-26. "Each meeting date is tentative until confirmed at the meeting immediately preceding it" | 166 KB; no robots.txt; 200 to a plain GET |
| FOMC minutes (2:00 PM, ~3 weeks after) | federalreserve.gov/newsevents/YYYY-month.htm (slugs vary: `2026-04.htm`, `2026-may.htm`; follow the links on /newsevents/calendar.htm) | HTML "FOMC Meetings" block | a year+ (Dec 2026 page updated 2025-06-24): 2026-11-18, 2026-12-30 | ~80 KB/month |
| Fed released proof | statement `/newsevents/pressreleases/monetaryYYYYMMDDa.htm`; feeds/press_monetary.xml | deterministic / RSS | at release | verified for 2026-10-28 |
| Jobs report, CPI (8:30 AM) | bls.gov/schedule/news_release/empsit.htm and cpi.htm; bls.gov/schedule/news_release/bls.ics | HTML table (reference month, date, time) / ICS (TZID US-Eastern, UID, SEQUENCE, no URL) | pages: jobs through 2027-12-03, CPI through late 2027; ICS through 2026-12-30 only | **403 to browser and default UAs; 200 with a UA naming a contact.** Worker egress untested |
| BLS released proof | bls.gov/news.release/archives/{empsit,cpi}_MMDDYYYY.htm | deterministic HTML | at release | verified empsit_11062026, cpi_10142026 |
| Federal elections | 2 U.S.C. 7 (Tuesday after the first Monday in November, even years; uscode.house.gov), fec.gov | statute | years | manual file. 2026-11-03 was three days ago; next 2028-11-07 |

Upcoming v1 dates: jobs 2026-12-04, 2027-01-08; CPI 2026-11-10, 2026-12-10;
FOMC 2026-12-08/09 (projections), 2027-01-26/27; minutes 2026-11-18.

How often they change: rarely, but in bulk. The 2025-10-01..11-12 funding
lapse cancelled six weeks of DOL claims releases (notice on
oui.doleta.gov/unemploy/claims_arch.asp) and BEA's release JSON still shows
the gap. FOMC dates move only from tentative to confirmed, or in an
emergency. The current funding CR is reported (secondary sources) to run to
2026-12-11, so a lapse would hit the jobs report of 2027-01-08 at the
earliest; the "postponed" path exists for exactly this.

## Later sources (enough to show the model extends)

| Kind | Source, format, lead | Note |
|---|---|---|
| PPI, JOLTS, ECI | same BLS per-release pages + ICS | JOLTS page shows only the next date |
| GDP, PCE | apps.bea.gov/API/signup/release_dates.json (UTC, through 2027-12) + BEA ICS (estimate names); released: apps.bea.gov/rss/rss.xml | GDP and PCE now share dates |
| Retail, durables, housing starts | census.gov/economic-indicators/calendar-listview(-2026).html, HTML, full year | indicator.xml is stale (2021) |
| Jobless claims | rule (Thu 8:30) + DOL exception list (through 2027-11-24) | |
| Fed speeches/testimony | Fed month pages, ~1 week ahead, Board members only | |
| ISM, UMich | ISM page behind reCAPTCHA; UMich no forward schedule | both copyright; dates only |
| OPEX, S&P rebalance, refunding, funding deadlines | rules / S&P DJI press / Treasury statements / congress.gov, manual | debt-ceiling X-dates are estimates: never |
| Market closed / early close | `crates/core/src/clock.rs:26-49` | already the trading-day source |

## Event model

```
MacroEvent {
  id: "bls:cpi:2026-10" | "fed:fomc:2026-12" | "fed:minutes:2026-10"
      | "us:election:2028-11-07"  (source:series:period; stable on a move)
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
  it moved/confirmed. **released**: the release URL answers 200 or the RSS
  item appears; never inferred from the clock. Elections: scheduled only.

Tiers: v1 kinds are all tier 1, minutes tier 2. Later kinds get 2 or 3
(PPI, retail, claims 2; housing, durables 3). Tiers are a code table, so
the page and the brief filter by tier, not by kind.

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
`series` is free text (no CHECK), so later kinds need no migration.
Additive; Rick applies it to remote D1 before deploying (as with 0007).

## Ingestion: committed snapshot + small daily live check

1. **Snapshot** `scripts/build-events.mjs` -> `crates/core/data/events.tsv`
   (like `scripts/build-watchlist.mjs`: contact UA, cache, `--offline`,
   deterministic bytes). v1 reads the FOMC page, the Fed month pages
   (minutes), BLS `empsit.htm` and `cpi.htm`, and
   `crates/core/data/events-manual.tsv` (elections; a row without an
   official `proof_url` stops the run). One parser per source, so a later
   kind is one more parser. Prints a moved/new/removed diff against the
   committed file for review. Covers today .. +13 months. Run monthly and
   whenever a shutdown looms (`docs/runbook.md`).
2. **Load**: the generator embeds the TSV (`include_str!`, as the watchlist)
   and a daily `events:check` job upserts rows whose content changed (one
   D1 read of the window, one batch of changed rows). Wiring follows
   `crates/core/src/schedule.rs:6` and the once-a-day KV pattern of
   `crates/core/src/earnings/calendar.rs:642`; the queue is the builder's.
3. **Live check, same job, next 10 trading days only**: BLS ICS (81 KB) and
   the FOMC page's current-year panel (166 KB page): 2 subrequests, 2-3 D1
   queries. Diff by id -> confirmed / moved / postponed. A fetch that fails
   or parses to nothing is a diagnostic, never a status change. The builder
   measures CPU against the free plan's 10 ms (scan only the needed panel).
4. **Released check**: on the existing `*/5` cron, for an event in the last
   45 minutes not yet released, one GET of its release URL (BLS archive
   pattern; Fed `monetaryYYYYMMDDa.htm`; minutes via press_monetary.xml).
   At most two a day.

If the Worker cannot reach BLS (UA or IP block), BLS rows stay `scheduled`
from the snapshot and link the schedule page instead of the release.

## Calendar page

`/[locale]/earnings` (`apps/web/src/app/[locale]/earnings/page.tsx`) gains a
sibling query `listEventsWeek(db, monday)` (one D1 query, tiers 1-2). Each
day with events gets an **"Economy & Fed" / "经济与美联储"** group **above**
the earnings slots (`apps/web/src/components/CalendarWeek.tsx`), by time:
`8:30 AM  October CPI  [Major]  Schedule ↗` -> once released `✓ Release ↗`;
`2:00 PM  FOMC rate decision (with projections)`, then `2:30 PM` press
conference; moved: "Moved from Dec 10"; postponed: "Postponed" + schedule
link. Tier 1 gets a text badge ("Major" / "重要"), not colour alone, as the
page already does. All-day events (elections) show without a time. The
legend gains one sentence on proof and status. No event detail page: the
official release is the detail. In v1 most weeks have zero to two events.

## Premarket "What to watch"

- `watch_list: Field<Vec<String>>` becomes `Field<Vec<WatchItem>>`
  (`{id, et_time, title, tier, status, release_url}`, title in the brief's
  language from the glossary). Old R2 facts hold `Missing`, which still
  deserialises.
- Built from today's tier 1-2 events (plus, **[Rick] 4**, write-up earnings
  due after today's close). Empty -> `Missing`, and the section drops as it
  does now (`crates/core/src/premarket/draft.rs:58-70`). With v1 kinds alone
  that is about 3 trading days a month.
- Digits in times and periods ("8:30", "2026") must be allowed figures:
  extend `numbers_in_facts` (`crates/core/src/facts.rs:171`) with each
  item's hour, minute and period year/month.
- **8:00 brief**: "October CPI at 8:30 AM ET". **8:35 update**: runs the
  released check first, so the item reads "released at 8:30" with the link;
  the figure reaches the text only via the existing macro news search.
  The update keeps running every trading day (the 2026-11-06 update design):
  gating it on the calendar would skip the 8:30 releases v1 does not track.

## Bilingual glossary (code table, never LLM-translated)

| key | EN | ZH |
|---|---|---|
| fomc (v1) | FOMC rate decision | 联邦公开市场委员会（FOMC）议息会议 |
| fomc_presser / sep (v1) | Fed Chair press conference / with projections (dot plot) | 美联储主席新闻发布会 / 含经济预测（点阵图） |
| fomc_minutes (rec.) | FOMC minutes | FOMC会议纪要 |
| jobs (v1) | Jobs report (nonfarm payrolls) | 非农就业报告 |
| cpi (v1) | CPI (consumer prices) | 消费者物价指数（CPI） |
| election / midterm (v1) | US general election / US midterm elections | 美国大选 / 美国中期选举 |
| later | PPI; JOLTS; claims; GDP advance/second/third; PCE; retail sales; Beige Book | 生产者物价指数（PPI）; 职位空缺（JOLTS）; 每周初请失业金人数; GDP初值/修正值/终值; 个人收入与支出（PCE物价指数）; 零售销售; 美联储褐皮书 |

Periods: EN "October CPI", "October jobs report"; ZH "10月CPI", "10月非农就业报告".
Titles name offices, not people ("Fed Chair"), so a new Chair needs no edit.

## Later: breaking news (out of scope; interface only)

Rick's separate idea (sudden news added to posts) would need a trigger
source (e.g. Fed `press_all.xml`), an unscheduled `macro_events` row
(status `released`, no prior schedule) and a post revision path; the
premarket update's `posts.revised_at` + per-revision facts key
(`2026-11-06-premarket-update-design.md`) is that primitive, appending a
dated block instead of rebuilding. Nothing in v1 blocks it.

## Decisions

Decided, Rick 2026-11-06: v1 kinds are "only the market-moving events (Fed
decisions, jobs, CPI, elections). we will decide if we want to add the
fuller later".

Open, for Rick:
1. **FOMC minutes in v1.** Rec: yes, tier 2. Same Fed source family, dates
   a year ahead, 2:00 PM releases that move rates; one extra parser.
2. **Figures.** Rec: never consensus/forecasts (Bloomberg, LSEG, Dow Jones
   polls are licensed). v1 shows no actuals either, only the release link;
   actuals from the official release (public domain) can follow with
   anchored extraction like earnings.
3. **Page name.** Rec: keep the `/earnings` URL; H1 "Market calendar" /
   "市场日历" (the nav already says Calendar / 日历). User-visible wording.
4. **Earnings in "What to watch".** Rec: yes, write-up-tier companies
   reporting after today's close (already in D1, no new fetch); without it
   the section shows on only ~3 days a month.
