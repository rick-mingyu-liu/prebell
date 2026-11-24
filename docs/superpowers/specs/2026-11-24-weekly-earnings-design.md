# Earnings this week: a page per week

Date: 2026-11-24. Status: the direction is approved (Rick, 2026-11-24, `docs/decisions.md`). This design waits for Rick's approval; nothing here is built until then. The open questions at the end are his.

## Why

From Cloudflare Web Analytics, bots excluded, over the 30 days to 2026-11-24: 210 visits and 500 page views; 82 visits in the last 7 days. The most-viewed page is the Chinese earnings calendar `/zh/earnings`, by a clear margin, then `/en`, `/en/earnings`, the Anthropic first-day article and `/zh/archive`. Rick's own write-up: "users seem more interested in the earnings checks than the daily briefs".

The proposal Rick approved on 2026-11-24 ("that's a good call. let's do that."): "a weekly "earnings this week" page, built automatically each Sunday from the calendar PreBell already verifies: who reports each day, what they reported last quarter, and links to each check as it publishes. In both languages, with a share card."

The point is a page that is new every week, is easy to share, and builds on the most-visited page. The constraint is distribution, not features: posting it in Chinese investor communities is Rick's part.

## What the stored data can do today (read 2026-11-24)

Three facts from the live database change the proposal. The design follows them.

1. **"What they reported last quarter" has almost no data yet.** PreBell has 30 earnings checks, one each for 30 companies. The first is dated 2026-11-03 (AMGN, AMD, BKNG, AIG) and the newest 2026-11-24 (MDT, ADI). No company has two. None of the 23 companies on the calendar from Nov 30 to Dec 18 has any check, because their previous reports came before PreBell existed. A company checked in November next reports about three months later, so the first week in which any listed company has an earlier check is about the week of 2027-02-01. That is an estimate from the usual reporting rhythm, not a stored date. After that, the column fills only for the write-up tier (140 companies). Calendar-only companies never get checks, so they never get figures this way. The page is designed so the column appears when the data exists and is absent, without a gap or a placeholder, until then. Figures from SEC XBRL history are an option for Rick (question 2).
2. **No cron fires on a Sunday.** The generator's two triggers are `*/5 10-23 * * MON-FRI` and `*/5 0-3 * * TUE-SAT` (UTC), and a test in `schedule.rs` keeps the count at two. "Built each Sunday" would need a third trigger (three of the five the free plan allows). This design builds on Friday evening instead, on the trigger the weekly digest already uses, and the stable "this week" link turns to the coming week at Saturday 00:00 ET. That changes the approved wording, so it is question 1.
3. **It launches in the off-season, and almost every date is unconfirmed.** The calendar has 121 rows (120 companies) since 2026-11-03, because Yahoo seeds only the next 15 trading days. Next week (Nov 30–Dec 4) lists 14 companies, and 13 of those dates are unconfirmed. The two weeks after list 8 and 3. Of the 121 rows as they stand, 2 are confirmed by a company announcement (`source = 'ir'`), 40 are Yahoo dates and 79 are reported from EDGAR. A report overwrites a row's status and source, so this counts the rows' state now, not how many were ever confirmed. This week (Nov 23–27) has 1 confirmed, 9 unconfirmed and 11 reported. Announcement checks reach only dates within 7 days, 5 a day (`IR_HORIZON_DAYS`, `IR_CHECKS` in `calendar.rs`), so a few of next week's dates may be confirmed by Friday, but most will not be. On this page, "expected" is therefore the normal state, not the exception. A peak-season week cannot be measured yet, since the calendar started after the October peak. The sizes below are estimates: up to about 200 companies in one week, of which up to about 60 are in the write-up tier.

Also relevant:

- **The weekly digest is running.** kv `digest_built_for = 2026-11-23` was written at 2026-11-20 23:01Z (Friday 18:01 ET). The digest writes that key only after Buttondown saves a draft; a dry run writes nothing. The English week-ahead email already lists next week's write-up companies by day and slot, largest first, and counts the rest. This page is its public, live, bilingual counterpart, not a replacement. The digest is unchanged unless Rick says otherwise (question 6).
- **Every earnings check already has two cover lines written by code in both languages**, in D1: `post_translations.cover_line1/2` (for example `Revenue $4.4B` / `GAAP EPS $3.40`, and `营收 43.7亿美元` / `GAAP每股收益 3.40美元`). They are composed by code from the verified figures. All 60 translations of the 30 checks have both lines. In three checks (AIG, ConocoPhillips, Berkshire) the first line is `<Company> results` / `<Company>业绩` with no figure, because the release states no revenue. The page uses the lines for both this week's results and the earlier check, so it needs no R2 read and no model.
- **Past dates with no report stay unconfirmed.** The reconcile rules never touch a row dated before today (`reconcile` in `calendar.rs`). So 7 past rows are still `unconfirmed`, with no report found: PFE Nov 3, NVO Nov 4, TM Nov 5, TECH Nov 9, NBIS Nov 12, MNSO and ZTO Nov 17. All are Yahoo dates, and all are calendar tier except Pfizer, which is write-up tier and has no check. A past week's page must not show them as still expected.
- No D1 history records a moved date. A Yahoo date that moves deletes the old `(ticker, report_date)` row and writes a new one (rule 5 in `reconcile`, `DELETE_CALENDAR` in `crates/generator/src/earnings_store.rs`). An 8-K writes a `reported` row on its ET acceptance date and deletes the ticker's other unreported rows within 21 days (`report_changes`, `SAME_EVENT_DAYS`), so an early or late report also leaves the week with no trace. The page can only say "moved since Friday" if Friday's list was saved somewhere.
- The calendar holds no rows dated on a Saturday or Sunday. Berkshire's Saturday release is dated on the Monday.

## Goals

1. A page for each week, in English and Chinese, listing every company on PreBell's calendar expected to report that week. Each date is shown as confirmed, unconfirmed or reported, in words.
2. As each report arrives, its row shows it within about a minute: write-up companies with the check's two figures and a link to the check, others with a link to the SEC filing.
3. When PreBell has an earlier check for a company, the row shows that check's period, date and two figures, and links to it.
4. A stable "this week" address and a share card for each week, so one link can be posted every weekend.
5. No model call. Every figure on the page is a cover line of a published check, and every date and status comes from the calendar.

## Non-goals

- No analyst estimates, no "beat" or "missed", no consensus, no expected figures, and no prediction of the share price reaction.
- No naming of the quarter a company is about to report before it reports. The calendar does not store it, and a guessed fiscal period is the kind of error the search-title ruling of 2026-11-19 exists to prevent.
- No new data source and no request to Yahoo beyond what already runs. The XBRL option (question 2) uses EDGAR, which PreBell already reads, and is not part of v1.
- No change to the weekly digest's content or behaviour without Rick's yes (question 6).
- No post row, so the page is not in the archive, the recent list or the MCP server (see "Why no post row").

## What the reader sees

### Address and title

- Dated page: `/{locale}/earnings/week/{monday}`, for example `/zh/earnings/week/2026-11-30`. It sits under `/earnings` so that it joins the most-visited section and keeps "earnings" in the address for search.
- Stable page: `/{locale}/earnings/week` renders the reading week in place: Monday to Friday, the current week; Saturday and Sunday, the coming week (the turn is at 00:00 ET Saturday). Its canonical URL is the dated page, so search engines index dated pages only. This is the address for the home page link and for bookmarks. Rick shares dated links, so a preview cached by WeChat or X stays true.
- Range: from the launch week (`FIRST_WEEK_PAGE`, set at build time) to the week after the reading week. A week outside the range answers 404 before anything is read, as the calendar does since PR #81. A past week stays up as the record of what was reported.
- `<title>` (fixed per URL): `Earnings, week of Nov 30–Dec 4, 2026 · PreBell` / `2026年11月30日至12月4日当周财报 · 盘前铃`.
- Heading kicker, which changes with today: "This week" / "Next week" / "Past week" (`本周` / `下周` / `往周`).
- Meta and Open Graph description: `Who is expected to report earnings from Nov 30 to Dec 4, 2026, with each date's status and links to PreBell's earnings checks as they publish.` / `2026年11月30日至12月4日预计公布财报的公司，标注日期是否确认，并在财报速评发布后附上链接。`

### Mock-up (English, the week of Nov 30, as the data stood on 2026-11-24)

```
EARNINGS · THIS WEEK
Week of Nov 30–Dec 4, 2026
14 companies · 1 date confirmed · 0 reported so far           [Share]
Times US Eastern. Unconfirmed dates come from an aggregator and may move.

Economy & Fed
  Fri Dec 4  8:30   Jobs report (November)                         ← listEvents, as on the calendar

Changes since Friday                               ← only when something changed; these two are made up
  Okta (OKTA) moved to Tue Dec 8, not confirmed.
  Kroger (KR) was added on Wed Dec 2.

Tuesday, Dec 1
  Before open
    Copart (CPRT) · NIO (NIO) — not confirmed                      ← calendar-only: names, ticker links
  After close
    Marvell Technology  MRVL                           Confirmed
      Earnings check after the filing.
    Also: NetApp (NTAP), Okta (OKTA), Trip.com (TCOM) — not confirmed

Wednesday, Dec 2
  After close
    Salesforce  CRM                                    Not confirmed
      Earnings check after the filing.

Thursday, Dec 3
  Before open
    Brown-Forman (BF.B), Dollar General (DG), Hormel Foods (HRL), Kroger (KR) — not confirmed
  After close
    Cooper Companies (COO), Ulta Beauty (ULTA), Veeva Systems (VEEV) — not confirmed

Monday, Nov 30 · Friday, Dec 4
  No reports expected.

See this week on the calendar →   ·   Next week →
```

Monday and Friday are shown in their place in the real page. The mock-up folds them to keep it short. The companies, dates, statuses and the jobs report are the stored rows of 2026-11-24; the two changes are made up to show the section.

A write-up row after its report (the figures are made up):

```
    Salesforce  CRM                                    ✓ Reported
      Revenue $10.3B · GAAP EPS $1.95                  Read the check →
```

and, from about February 2027, a write-up row with an earlier check (the figures here are illustrative):

```
    Advanced Micro Devices  AMD                        Not confirmed
      Earnings check after the filing.
      PreBell's last check: Q3 2026, Nov 3 — Revenue $9.2B · GAAP EPS $0.75 →
```

A calendar-only company after its report: `Dollar General (DG) ✓ Reported · SEC filing →`.

### Mock-up (Chinese)

```
财报 · 本周
2026年11月30日至12月4日当周
本周 14 家公司 · 1 家日期已确认 · 已发布 0 家                         [分享]
时间均为美东时间。未确认日期来自第三方汇总，可能变动。

经济与美联储
  12月4日周五 8:30  非农就业报告（11月）

自上周五以来的变动
  Okta（OKTA）改至12月8日周二，日期未确认。
  Kroger（KR）于12月2日周三加入。

12月1日 周二
  盘前
    Copart（CPRT）· NIO（NIO）— 日期未确认
  盘后
    Marvell Technology  MRVL                           已确认
      财报公布后发布速评。
    另有：NetApp（NTAP）、Okta（OKTA）、Trip.com（TCOM）— 日期未确认
...
    Salesforce  CRM                                    ✓ 已发布
      营收 103亿美元 · GAAP每股收益 1.95美元             阅读速评 →
      盘前铃上次速评：2026年第三季度，11月3日 — 营收 … →
```

Company names are the watchlist's English display names on both pages, as on the calendar today (`companies.generated.ts`). The Chinese strings are written for Chinese readers in `messages/zh.json`, reusing the calendar's terms (`盘前`, `盘后`, `日期未确认`, `已发布`, `阅读速评`, `SEC 文件`). They are not translations of the English.

### Rows, rules and sources

| What | Source | Rule |
|---|---|---|
| Who is listed | `earnings_calendar`, Mon–Fri of the week, status `unconfirmed`, `confirmed` or `reported` | Quarantined rows are hidden, as on the calendar. |
| Day and slot | `report_date`, `time_slot` | Before open, after close, then "Time not given" (`时间暂缺`). |
| Status | `status` | Always in words; "Confirmed" only for `confirmed`. Never "will report"; the line under an unconfirmed row says what PreBell will do, not what the company will do. |
| Order within a slot | write-up tier first, then market cap (the dated snapshot) | The digest's order. The site gets a cap rank in a generated file (below). |
| This week's result, write-up tier | the check's `cover_line1/2` in the page's language, plus the slug | Shown when the English check is published. A cover line with no digit (`AIG results`) is left out, so the row shows only the line with a figure. On the Chinese page a check whose Chinese text is pending shows the link (the post page's "coming soon") and the English cover lines are not substituted. |
| This week's result, calendar tier | `proof_url` (the EDGAR filing index) | `✓ Reported · SEC filing →`, the calendar's existing check on the URL shape. |
| Earlier check | the newest published earnings check of that ticker with `trading_date` before the week's Monday and at most 200 days before it | Labelled "PreBell's last check", never "last quarter": its own period (the search-title rule of 2026-11-19: "Q3 2026" when calendar-aligned, else "Q3 fiscal 2027"; from the slug; a slug named by filing date shows the date only), its date, its cover lines and a link. Absent, nothing is shown. |
| A date that passed with no report | `status` still `unconfirmed` or `confirmed`, `report_date` before today ET | "No report found for this date" / "未查到该日财报", with no promise of a check. It is counted apart from the expected companies in the header, so a past week never reads as still to come. |
| Changes since Friday | kv `week:<monday>` (Friday's list, below) against the live rows | Moved within the week: the new day. Moved out: the new date and status if a row exists, else "no longer on the calendar for this week". Reported before the week: "reported on Nov 27, before this week" with its check or filing link. Added after Friday: "added on <weekday>". |
| Economy & Fed | `listEvents` for the week | As on the calendar. |
| Counts in the header | the live rows | Companies, not rows, as the calendar counts. |

Big weeks: write-up companies always get a full row. Calendar-only companies in a slot are one line of names. Past 12 names, the line shows the first 12 by market cap and "and 31 more", which opens the rest (`<details>`, no script). Every name links to its ticker page.

Empty weeks (holidays, the end of December, the off-season): the header says "No company on PreBell's list is expected to report this week." / "本周盘前铃名单内暂无公司预计公布财报。", followed by the week's Economy & Fed rows and a link to the next week with a listed company (found by one indexed `MIN(report_date)` read, the strip's pattern). A week with no company is `noindex` and left out of the sitemap.

### How it updates during the week

Nothing is rebuilt. The site reads the calendar and the checks at render, so a report, a moved date or a newly seeded company appears on the next render: within the 60 s edge cache, per location, with the deploy version in the cache key (`apps/web/worker.ts`, PR #81). A check shows as soon as its English text is published. The generator adds only two things that cannot be read live: Friday's list, for "Changes since Friday", and the share card.

## Generator and site

### Site: all reading, at render

`apps/web/src/lib/week.ts` and the two routes under `apps/web/src/app/[locale]/earnings/week/`. Four reads, run in parallel with `Promise.all` as the calendar page runs its four (the site's `D1Like` has no `batch`):

1. The week's calendar rows with this week's check (slug, cover lines in `lang`): `listCalendarWeek`'s query, with the cover lines added to its post join.
2. The earlier checks: one query from `earnings_calendar` (the week's dates) through `earnings_filings` (by ticker) to `posts` and `post_translations`, the newest per ticker before Monday. It joins on the calendar rather than binding tickers, so a 200-company week stays one statement under D1's 100 bound values.
3. kv `week:<monday>` (one primary-key read).
4. `listEvents` for the week.

One more read runs only when something moved out of the week: those tickers' current rows (a few bound values). Another runs only for an empty week: the next date with a listed company.

No R2 read. The card's URL is in the kv snapshot once it exists; until then the page names `og-default.png`.

Market-cap order: `scripts/build-watchlist.mjs` also writes each ticker's rank by the dated snapshot into `companies.generated.ts` (a third field), read from `crates/core/data/marketcap-snapshot.tsv` with no network request. The builder checks that `--offline` reproduces the file byte for byte except for the new field.

### Generator: Friday's list and the card

A new `week` step on the jobs queue (`prebell-jobs`), like `ipo:sweep`: it needs no EDGAR, so it never holds up a poll. An earnings poll queues it:

- **Friday**, from 18:00 ET once that day's calendar refresh has finished, or from 21:45 ET whatever the refresh's state, until kv `week:<next Monday>` exists. These are the digest's conditions, so the list matches the email. It writes Friday's list for next week: ticker, date, slot and status for every listed row, about 30 bytes a row, at most about 6 KB.
- **Every trading day**, once after the calendar refresh (kv `week_checked_on=<ET date>`), for the current week, and on Friday for next week too: it works out the card's tickers from the live rows. If they differ from the card already drawn, it draws the cover and queues `cards`.

It is behind `WEEK_PAGE_ENABLED`, exactly `"true"`, in `crates/generator/wrangler.toml`. It is not tied to `DIGEST_ENABLED`, so switching the digest off leaves the page's list alone. Without the switch the page still renders, with no "Changes since Friday" and with the default card.

**No new cron trigger.** The step rides the existing polls. The `schedule.rs` test stays at two triggers.

**The card** reuses `cover::card`: a new `cover::week` draws the week's cover SVG and `card_svg` recomposes it at 1200×630 under the nameplate. The card keeps to `cover::card`'s rules: only Latin fonts are embedded (`card.rs`), so the card holds no Chinese, and one card serves both languages, as for every post. Platforms print the page's title beside it, in the page's language. Proposed content (question 5): kicker `EARNINGS · NOV 30–DEC 4`, and the up to six largest write-up companies expected that week as ticker tiles, with no day and no figure, so a move within the week never makes the card wrong. The file is `covers/week-<monday>-<hash8>.svg` and `og/week-<monday>-<hash8>.png`, where the hash is of the tickers drawn. A redrawn card therefore gets a new URL, which matters because WeChat and X cache images by URL. Both names fit `COVER_KEY` and `CARD_FILE` in `apps/web/src/lib/covers.ts`, and `/og/<file>` already serves them.

The `cards` job, after its post scan finds nothing missing, renders the newest week cover without a PNG. It uses the same attempts counter and give-up after 3 tries, then records the PNG's stem in kv `week:<monday>`, so the page names a card only once it exists. That adds one kv read per `cards` message.

### Storage

No migration. The step uses the existing `kv` table (`week:<monday>`, `week_checked_on`) and the existing `prebell-facts` bucket (`covers/week-*.svg`, `og/week-*.png`, `og/week-*.attempts`). There is one kv row per week, and two or three objects per drawing of a card.

### Why no post row

A post is an article: a `body_md`, translations with their own statuses, a slug, a trading date. It is listed by type in places this page should not reach on its own. `listRecentBriefs` takes every type that is not `earnings`, so a new type would land in the home page's recent list. Prev/next links, the archive, the MCP server and the sitemap's post entries also read posts. This page is a live view whose content changes every hour of its week, so it fits none of them. It gets explicit entries where it belongs:

- **Sitemap**: the dated week pages with at least one company, from the launch week to next week, within `INDEXED_WEEKS` (8) of today, in both languages, paired with hreflang. That takes one grouped read inside the sitemap's existing hourly cache.
- **Links**: the stable page from the calendar page's header ("This week as a list" / "本周列表"), and from the home page's earnings section if Rick says yes (question 4). The week page links back to the calendar's same week and to each ticker page. Ticker pages are unchanged.
- **Archive, recent list, MCP, RSS**: none. The site has no RSS feed. The MCP server's calendar tool already serves the same rows.

## Distribution

- **Share card**: as above. One per week, redrawn only when the companies on it change.
- **Stable link**: `/{locale}/earnings/week` for navigation; dated links for posting.
- **Open Graph and Twitter**: `shareMetadata` in the page's language, with `og:type=website`, the title and the description above, and the week's card.
- **Share button**: the calendar's `ShareButton` on the dated URL.
- **Posting**: Rick's part, from Friday evening, when next week's list exists, so the Chinese link can go to WeChat groups on Saturday or Sunday. The page does not depend on when he posts.
- **The digest**: unchanged. The proposal for Rick (question 6) is to add one line to the email's week-ahead section linking to `/en/earnings/week/<monday>` ("The full week, with each check as it publishes"). That is a change to his wording in `crates/core/src/digest/copy.rs` and its fixture test, so it is made only on his yes.

## Costs

- **Model calls: none.** All text is fixed strings in `messages/{en,zh}.json`, with figures that are the checks' own cover lines, which code wrote.
- **Site, per uncached render**: four D1 reads in parallel, plus one only when a date moved out of the week, or one only for an empty week. No R2 read. CPU: about the calendar page's, which renders the same rows as tiles. The builder measures both pages for a 200-row fixture week in the same run and reports the numbers. The page must not be slower than the calendar page. At about 12 visits a day most renders miss the 60 s edge cache, so the uncached cost is the one that counts.
- **Generator**: one `week` message per trading day, plus a few on Friday evening (about 10 queue operations a week against 10,000 a day). Each is two or three D1 reads, at most one kv write and, when the card changes, one R2 write and one queued `cards` message. No fetch subrequest. A card render is the existing cost, 3–4 ms warm and up to about 20 ms in a cold isolate (runbook, "Share cards"), in its own message, a few times a week at most.
- **Storage**: about 6 KB of kv a week; about 10 KB of SVG and 60 KB of PNG per card drawn.
- **Money**: none. Nothing is added on the free plan.

## Risks and how each is handled

| Risk | Handling |
|---|---|
| A wrong or moved date | The page is the calendar, read live. Unconfirmed is stated on every such row and in the header line. A move shows under "Changes since Friday". The calendar's own fixes (quarantine, the forced refresh in the runbook) correct the page with no extra step. |
| A company reports outside the week | Early, before Monday: the calendar marks its row reported on the filing date, and the page lists it under "Changes since Friday" as reported before the week, with the link. Late, after Friday: the same, with the new date. A row dated on a weekend would appear on the week of its Monday; none exists today. |
| No earlier check, or an old one | The earlier-check line is shown only when a published check exists within 200 days before Monday. It is labelled with its own period and date, never as "last quarter", so a missed quarter cannot be presented as the previous one. |
| Fiscal-period labels | The upcoming report's period is never named. An earlier check's period uses the search-title rule of 2026-11-19 (calendar quarter only when it ended within 120 days before the check's date, else fiscal), from the slug, with a test that includes Nvidia, Disney and ConocoPhillips cases. |
| A check whose Chinese text is pending | The Chinese page shows the link (the post's "coming soon") without English figures. |
| A past date with no report | Shown as "No report found for this date", never as expected (7 such rows exist today, Pfizer's among them). Why reports go undetected is the calendar's problem, not this page's. |
| A cover line that differs from the check's summary | The page shows the cover line, which code rounds from the verified value. The check's summary is model text that passed the number check. They can differ in the last digit: Walmart's Chinese cover line says `营收 1888.3亿美元`, and its Chinese summary says `1888.4亿美元`. The page matches the check's own cover, which the reader sees first on the check. The builder adds no figure the cover lines do not hold. The rounding difference is reported to the lead separately. |
| A week's page is wrong after publication | There is nothing to republish. Fixing the calendar row or the check fixes the page within 60 s. A wrong "Changes since Friday" line comes from the kv list: deleting `week:<monday>` removes the section, and the runbook gives the statement. A wrong card is redrawn by the daily check when the tickers differ. To force a redraw, delete the kv row's card stem and the next `week` message draws it again. Nothing is emailed by this feature, so no correction has to reach an inbox. |
| SEO duplication with the calendar | The week page is a text list with figures, earlier checks and changes, under its own title. The calendar is a board of tiles under "Market calendar · Week of …". Both stay indexed, with links to each other. If Search Console later shows them competing for the same queries, the fallback is `noindex` on the calendar's dated `?week=` pages (not on `/earnings` itself). That is a later decision, made from data. |
| CPU on a big week | The 200-row fixture is measured against the calendar page before merge. Calendar-only companies are name lists, not cards. |
| The card names a company that moved out | The card holds no days, and the daily check redraws it under a new URL. A preview already cached by a platform keeps the old image. That is why tickers are the only content and Rick shares the dated page. |
| The Friday step fails | The page works without it: no "Changes since Friday", default card. The step alerts once a week, `warn` and never `error`, keyed `week:<monday>`, since nothing a reader sees is wrong. |

## Out of scope for v1

- Prior-quarter figures for companies without an earlier check, from XBRL or any other source (question 2).
- The quarter being reported, consensus, estimates, guidance comparisons, share price reactions.
- Chinese company names on the page or the card.
- A week page in the MCP server, an RSS feed, JSON-LD beyond the existing site-wide markup.
- Any change to the digest (question 6), the calendar's board, or ticker pages, apart from one link from the calendar header.
- A "last week in review" article. The past week's page already lists every report with its figures and links.
- Weekend dates and pages for weeks more than one ahead.

## Open questions for Rick

1. **When the week turns.** (a) *Recommended*: next week's list and card are made Friday from 18:00 ET with the digest, and the "this week" link turns at 00:00 ET Saturday. No new cron, and the page is shareable all weekend. (b) Sunday morning, as first proposed: a third cron trigger, three of the free plan's five, for no data gain, since nothing in the calendar changes over a weekend.
2. **"What they reported last quarter".** (a) *Recommended for v1*: only from PreBell's own checks. The column stays empty until about February 2027, then fills for write-up companies. (b) Also from SEC XBRL quarterly history (GAAP revenue and diluted EPS from the last 10-Q or 10-K, the source behind the checks' charts): about 3–5 EDGAR requests per company per week, one message per company, no Yahoo. It leaves gaps for fiscal Q4s that companies tag only inside the annual total, and for foreign filers (no 10-Q). It would be a v2 build of its own, with its own check that each figure is the company's own filed value.
3. **Which companies.** (a) *Recommended*: every company on the calendar that week; write-up companies in full, the rest as name lists with their status and links. (b) Write-up companies only: a shorter page, but it drops most of the names Chinese readers look for (NIO and Trip.com are calendar-only).
4. **Where it is linked.** (a) *Recommended*: from the calendar page's header and one line in the home page's earnings section. (b) From the calendar page only. (c) Nowhere; direct links only.
5. **The share card.** (a) *Recommended*: the week's dates and up to six of the largest write-up companies expected, as tickers. (b) The dates only: never out of date, less to look at.
6. **The digest.** (a) *Recommended*: add one line to the email's week-ahead section linking to the week page; it is your wording, in `copy.rs`. (b) Leave the email as it is.

## Build plan

The site work (tasks 1–3) can ship before the generator work (task 4). The page works without Friday's list. Each task is one builder pass and one separate review.

1. **Week data (site).** `apps/web/src/lib/week.ts`: the week range (`FIRST_WEEK_PAGE` to the week after the reading week, the Saturday turn), the batched reads above, the earlier-check rule, and the "Changes since Friday" diff. Add the cap rank to `companies.generated.ts` through `scripts/build-watchlist.mjs`, with no network request.
   *Accept when*: unit tests over `tests/d1.ts` cover a normal week, an empty week, a 200-row week, a moved-out and a moved-in company, an early report, a missing and an old (over 200 days) earlier check, a pending Chinese check, a past date with no report, and a cover line with no figure. `tests/queryplan.test.ts` shows each statement starting from `idx_earnings_calendar_date`, `idx_earnings_filings_ticker` or a primary key, and none scanning `post_translations`. The regenerated `companies.generated.ts` differs only by the new field.
2. **The pages (site).** `/[locale]/earnings/week` and `/[locale]/earnings/week/[monday]`; the layout above; every string in `messages/en.json` and `messages/zh.json`; metadata (title, description, Open Graph, hreflang, canonical, `noindex` for empty and far weeks); a 404 outside the range before any read.
   *Accept when*: render tests for both languages over the task 1 fixtures; a test that no "beat", "miss", "estimate", "consensus", "预期", "超预期" or "不及预期" string appears in either messages file's new keys; screenshots at 1440, 768 and 390 px of a normal, an empty and a 200-row week, in both languages; and a CPU comparison with the calendar page on the 200-row fixture, with the numbers in the PR.
3. **Wiring (site).** Sitemap entries (one grouped read, inside the hourly cache); the calendar header link; the home link only if Rick says yes; the card from kv `week:<monday>`, else `og-default.png`.
   *Accept when*: `tests/sitemap.test.ts` covers week entries and leaves out empty weeks; the home page's query-plan tests still pass; `pnpm test`, the lint and `opennextjs-cloudflare build` pass.
4. **Friday's list and the card (generator).** The `week` step and its queueing from the poll; `WEEK_PAGE_ENABLED`; `cover::week`; the `cards` job extension; the weekly `warn` alert.
   *Accept when*: Rust tests for the step's timing (Friday 18:00 and 21:45 ET, once per week, once per trading day) and for the list's content from a fixture calendar; a fixture SVG for the card, and `card-png` renders it; `schedule.rs` still asserts two crons; `cargo test -p prebell-core` and the generator's wasm build pass. Build in the shared target dir (the untracked `.cargo/config.toml`, `docs/decisions.md` 2026-11-07), not a fresh `target/`.
5. **Docs.** A "Weekly earnings page" section in `docs/runbook.md`: what runs and when, the switch, the alerts, how to correct a week, costs. Rick's answers go into `docs/decisions.md` with his words.

**Test before deploy**: the local preview (`pnpm preview` with the seeded local D1, runbook "Web site") with a seeded week. Read-only `wrangler d1 execute --remote` SELECTs of the coming week, compared by hand with the rendered page.

**Deploy order** (owner only, as in the runbook): no migration. If task 4 ships with the site, deploy the generator first, then the site, so a card exists before the page names one. If tasks 1–3 ship alone, the site deploys alone, and the generator follows in its own PR. After deploy, check that `/{en,zh}/earnings/week` returns 200, that a week outside the range returns 404, and that the first Friday's `job week` log line appears and its card shows in a link preview.
