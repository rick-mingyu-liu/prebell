# The Friday earnings week post

Date: 2026-11-24. Status: Rick chose this form on 2026-11-24 (`docs/decisions.md`). The details below are waiting for his answers to the open questions. Nothing is built until then.

This design replaces the live week page proposed earlier the same day, for three reasons:
- The critic's review showed that the live page mostly repeated the dated calendar.
- Its "last quarter" figures would be empty through January, and never exist for foreign filers.
- Its generator state had a race.

Rick then chose "Friday weekly post (Recommended)".

## Why

Rick's choice and its option text are in `docs/decisions.md` (2026-11-24). The aim is a new, dated, shareable item every week, built on the most-viewed page (`/zh/earnings`) and on the checks readers prefer. Distribution is Rick's part.

What the first posts will hold, from live D1 on 2026-11-24:
- Checks per week so far: 15, 3, 9 and 3.
- Companies expected in the next three weeks: 14, 8 and 3.
- Write-up companies among them: MRVL and CRM; then ADBE, AVGO and COST; then ORCL.

So the December posts are short, and the January season makes them long.

## The post

**Type and keys**
- `posts.content_type = 'week'`, `template_id = 'week-v1'`.
- Run key `week:<friday>`.
- Slug `earnings-week-<friday>`, for example `earnings-week-2026-12-04`.
- `trading_date` is the Friday.
- The facts bundle is `week/<friday>.json` in R2, holding every row the page states.
- The cover is `covers/earnings-week-<friday>.svg`.

**Written by code, no model.** The title, summary, cover lines and body are templates per language, filled from the same rows, as the IPO write-ups are (`crates/core/src/ipo/writeup.rs`). Chinese has its own templates, not a translation.

**Guard.** Every number and date on the page must be one the bundle holds, checked by the existing page guard that `writeup::verify_page` uses. A page that fails is not published. It is retried and then failed with an alert, as IPO write-ups are.

**Never half-published.** Both languages are rendered and checked first. Then the run, the post and both translations are written in one D1 batch, as `ipo::article` does.

### Sections

**1. "Earnings checks this week" / "本周财报速评"**
- Which checks: every earnings check whose English text was published after the previous week post's `published_at`, up to this build. The first post starts from that week's Monday, 00:00 ET.
- A check published after a Friday build appears in the next post, under its own date. None is lost and none appears twice.
- Order: market value, from the dated snapshot (`crates/core/data/marketcap-snapshot.tsv`, which the digest already reads).
- One line per check, with:
  - the company's display name and ticker;
  - the period, by the search-title rule of 2026-11-19 ("Q3 2026" only when that calendar quarter ended within 120 days before the check's date, else "Q3 fiscal 2027");
  - the check's two cover lines in that language;
  - a link to `/{lang}/p/<slug>`.
- A cover line with no digit (`American International Group results`) is left out.
- A check whose Chinese translation is `failed` (EMR and COP today) keeps its Chinese cover lines on the Chinese line, but links to `/en/p/<slug>`, marked "（英文）". The Chinese page of a failed translation redirects to the Chinese home page (`posts.ts`).
- A `pending` Chinese translation links to `/zh/p/<slug>` (its "coming soon" notice).

**2. "Expected next week, as of Fri Dec 4, 18:05 ET" / "下周预计公布（截至美东时间12月4日周五18:05）"**
- Layout: Monday to Friday, then slots (before open, after close, time not given). A day with no reports reads "No reports expected." / "暂无。"
- Write-up companies first, each on its own line with its status in words: "confirmed" or "date not confirmed" (`日期已确认` / `日期未确认`).
- Calendar-only companies follow, one line per slot and status, for example "Not confirmed: Copart (CPRT), NIO (NIO)". No status is ever more than a line from its name.
- Every name links to its ticker page (every calendar row has one, `tickerKnown`); quarantined rows are left out.
- No row dated before the coming Monday is listed, so the stale "not yet matched to a filing" rows never appear in a post.
- One "Economy & Fed" line for next week (FOMC, CPI, jobs), from the source the digest uses.

**3.** "The calendar, updated live →" / "实时日历 →" to `/{lang}/earnings?week=<next monday>`.

### Title, summary, cover lines

- **Title**: the three largest checks of the week and a count, for example `Earnings this week: Adobe, Broadcom, Costco; 8 expected next week` / `本周财报：Adobe、Broadcom、Costco；下周预计8家`.
- **Title with no check**: `Earnings next week: 14 companies expected from Nov 30` / `下周财报：11月30日起预计14家`.
- **Summary**: `PreBell checked 3 earnings reports this week. 8 companies are expected to report from Dec 7 to Dec 11, as of Friday 18:05 ET; 1 date is confirmed.` The Chinese template states the same facts.
- **Cover lines**: `3 checks this week` / `本周 3 篇速评`, and `8 expected next week` / `下周预计 8 家`.

### Empty weeks and holiday Fridays

- The post publishes when the window holds at least one check, or next week has at least one company expected.
- Otherwise the week's run is `skipped`, with a log line and no alert.
- Holiday Fridays (Dec 25, 2026; Jan 1, 2027) follow the same rule. Polls still run on them, because `in_window` checks only the weekday and the hour.
- `refresh_due` has no trading-day test; the builder confirms whether the calendar job refreshes on a holiday (if not, the post waits for the 21:45 ET fallback). A test pins both Fridays.

### Correcting a post

The post is frozen, so a wrong post is rebuilt:
1. The owner fixes the data (a calendar row, or a check).
2. The owner sets the run back to `pending`.
3. The next eligible poll rebuilds it. The posts upsert (`ON CONFLICT(run_key) DO UPDATE`, `crates/generator/src/store.rs`) keeps the slug and sets `revised_at`, so the page shows "Updated" and the cards job renders a new card.

To take a post down at once, set both translations to `failed`; the runbook gives the statements.

## What being a post brings (traced in the code)

| Where | What happens | Change needed |
|---|---|---|
| Home recent list | `listRecentBriefs` takes every type but `earnings` (`apps/web/src/lib/earnings.ts`); `loadHome` merges it | none |
| Archive | The default view binds `premarket` and `recap` only (`listPosts`, `posts.ts`); other types appear under their filter | `week` in `CONTENT_TYPES`; `types.week` = "Earnings week" / "财报周报" in both message files; the default view is the lead's call |
| Post page | `BriefView` shows `earnings.checkedFiling` ("✓ Every figure traced to the filing") for any type that is neither a brief nor `ipo`. That is false here, because next week's dates are aggregator data | A `week` case with its own line: "✓ Figures from PreBell's earnings checks; dates as of the time shown" / "✓ 数字出自盘前铃财报速评；日期以所示时间为准" |
| Prev/next | Briefs only (`getAdjacentPosts`) | none |
| Cover and share card | `cards_tick` scans every published post with an SVG cover (`COVER_POSTS`, `store.rs`), so the card renders with no change. The kicker for a post without a ticker is the date alone (`card.rs`) | The build draws `cover::week`; the kicker case is the lead's call |
| Sitemap | Every published post (`sitemap.ts`) | none |
| JSON-LD | `NewsArticle` for every post (`seo.ts`) | none |
| MCP | `latest_brief` binds `premarket`/`recap`; the earnings tools join `earnings_filings` | none; the post stays out |
| Digest | The picks read `content_type = 'earnings'` | none (see question 3) |
| Edge cache | Posts are cached 300 s (`next.config.ts`) | none |

**Cover graphic** (`cover::week`, Latin text only, as the card fonts are):
- Next week as five day bars: each bar's height is the number of companies expected that day, labelled with the day and the count, with the largest expected ticker above it.
- Every number drawn is in the bundle.
- A week with no company expected draws this week's checks as ticker tiles instead.

## The calendar's weekend turn

In `apps/web/src/app/[locale]/earnings/page.tsx`:
- `monday()` uses `weekStart(todayEt())` for the undated page. It changes to a new `readingWeek(today)` in `earnings.ts`: `weekStart(today)` from Monday to Friday, and `weekStart(today) + 7` on Saturday and Sunday (ET).
- The "back to this week" test (`start !== weekStart(today)`) uses `readingWeek` too.
- Dated `?week=` pages, the week range and the home strip are unchanged.

On Saturday and Sunday:
- The undated page's kicker reads "Market calendar · Coming week" / "市场日历 · 下周".
- The week button reads "Coming week" / "下周".


## Running it, and costs

**Trigger.** The digest's conditions, on the same poll:
- Friday from 18:00 ET (`digest::due`);
- that day's calendar refresh has finished, or it is 21:45 ET or later (`closing`);
- the run `week:<friday>` is neither `published` nor `skipped`.

The poll queues a `week` message on `prebell-earnings-build`, serial with the earnings builds as `ipo:article` is, so a late Friday check builds first. The switch is `WEEK_POST_ENABLED`, exactly `"true"`, independent of `DIGEST_ENABLED`. Run state is the `runs` row, with the IPO write-ups' lease and retries (5, 15, 30 and 60 minutes, then `failed` with an alert). A Friday evening on which every poll fails queues nothing, as for the digest. No new cron.

**Costs.** Zero model calls and zero fetch subrequests. About 8 D1 queries: run state, the previous post's time, the checks in the window, next week's calendar, events and one write batch. Next week's calendar comes through the digest's `gather`, extended to keep the calendar-only names it now only counts; the email is unchanged, pinned by `the_fixture_week_renders_exactly`. Two R2 writes (bundle and cover). One queue message a week, at most four retries. The digest is a separate message, so the two never share one invocation's 10 ms or 50 subrequests. CPU is templating and the guard over at most about 60 checks and 200 names. The builder profiles a 60-check, 200-company fixture with `--cpu-prof` and reads `cpuTime` in `wrangler tail` on the first live run. The card render is the existing cost, in the cards job. Storage is about 20 KB a week plus the card.

## Risks

| Risk | Handling |
|---|---|
| A check publishes after the build | The window is "since the previous post", so the next post lists it under its own date |
| Unconfirmed dates in a frozen post | "as of <Friday time> ET" in the section heading and the summary; a status in words on every line; a link to the live calendar |
| "Not yet matched to a filing" rows (PFE, NVO, TM, TECH, NBIS, MNSO, ZTO) | Section 2 starts at next Monday, so a post never lists a past date. Their wording on the calendar is a separate fix. Pfizer's Nov 3 row probably predates PreBell's first feed read and needs a one-off correction after a check of EDGAR |
| A peak week of about 60 checks | One line per check, about 150 characters, ordered by market value: about 9 KB of Markdown. The title names three. No cut-off; a test renders the 60-check fixture |
| A cover line differs from its check's summary | Walmart: cover line 1888.3亿美元, Chinese summary 1888.4亿美元. The post shows the cover line, as the check's own cover does |
| SEO | One new dated URL a week with a company-naming title; no duplicate of a calendar page |

## Distribution and the stop rule

**Each Friday evening or Saturday**, Rick:
- opens the Chinese post and reads it;
- shares its dated link in the groups he uses, and the English link on X;
- where links show no preview, posts a screenshot of section 2 instead, with its "as of" heading.

He posts after the log line `job cards: card og/earnings-week-<friday>.png`, so that no platform caches the default card.

**Two-minute preview test**, before the card work counts as done:
- Rick pastes an existing check that has a card (for example `https://prebell.site/zh/p/adi-q4-fy2026-earnings`) into a WeChat chat and into X's composer.
- He notes whether the image, title and description show.
- If WeChat shows no image, nothing more is spent on cards, and the Chinese route is the screenshot.

**Measure.** Cloudflare Web Analytics uses no cookies, so it cannot count returning visitors. The proxies:
- weekly page views of `/zh/p/earnings-week-*` and `/en/p/earnings-week-*`;
- Saturday to Monday visits to `/zh/earnings` and `/en/earnings`;
- referrers.

**Stop rule.** After six weekly posts, compare those numbers with the six weeks before the first post. If neither weekly-post views nor weekend calendar visits rose, switch `WEEK_POST_ENABLED` off and keep the published posts. January's season lifts traffic anyway, so this is a judgement made with the numbers.

## Out of scope

- A live week page.
- "Changes since Friday".
- Prior-quarter figures (appendix).
- Chinese company names.
- The IPO watch in the post.
- MCP.
- Any change to the digest without Rick's yes.

## Open questions for Rick

1. **Where will you post it, and every week?** This decides whether the card work matters.
   - *Recommended*: the Chinese link in WeChat groups and the English one on X, each Friday or Saturday, after the two-minute preview test.
2. **What counts as success, and when to stop.**
   - *Recommended*: the measure and the six-post stop rule above.
3. **The digest.** Its "Full calendar →" (`FULL_CALENDAR`, `digest/copy.rs`) links to next week's live calendar, which is still the right target for an email read on Sunday, so re-pointing it is not proposed. A link to the post would need the post built first, and shown only when it was not skipped.
   - *Recommended*: leave the email unchanged for v1.
   - Or add one line linking to the post when it is published, in your wording.

## Calls for the lead

- Build on Friday from 18:00 ET; the calendar turns at Saturday 00:00 ET. This replaces "each Sunday" in the approved direction.
- Content type `week`, labels "Earnings week" / "财报周报", slug `earnings-week-<friday>`.
- Card kicker `EARNINGS WEEK · 2026-12-04`, a new case in `card.rs`; without it, the kicker is the date alone.
- The cover graphic as described.
- Holiday Fridays follow the empty-week rule rather than always skipping.
- `week` in the archive's default view with `premarket` and `recap` (recommended, since Rick's option said "shows on ... archive"), or only under its filter.
- Queue `prebell-earnings-build` (recommended) or `prebell-jobs`.

## Build plan

Each task is one builder pass, then a separate review.

1. **Generator: the rows.** A Rust port of `calendarAligned` and `periodFromSlug` with the TypeScript tests' cases (Nvidia, Disney, ConocoPhillips, a date-named slug); the checks-in-window query; `digest::gather` keeping calendar-only names. *Accept when* unit tests cover a check published after the previous build (it lands in the next post), failed and pending Chinese translations, a digit-less cover line, two checks on one date, a quarantined row and mixed statuses in one slot, and `the_fixture_week_renders_exactly` passes unchanged.
2. **Generator: the post.** Templates (EN, ZH), bundle, guard, one-batch publish, `cover::week`, the trigger, `WEEK_POST_ENABLED`. *Accept when* fixture weeks render exactly (normal; no checks; no companies, skipped; 60 checks with 200 companies; Dec 25; Jan 1), the guard rejects a planted number, a failure retries and then fails with an alert, a `published` run is never rebuilt, `schedule.rs` still asserts two crons, a word-boundary test finds no `beat`, `miss`, `estimate`, `consensus` or `预期` in the templates, the 60-check CPU profile is in the PR, and `cargo test -p prebell-core` and the wasm build pass in the shared target dir.
3. **Site.** `week` in `CONTENT_TYPES`, `types.week`, the `BriefView` checked line, the archive call; `readingWeek` and the weekend kicker and button. *Accept when* render tests show a week post in both languages, in the home recent list and the archive; calendar tests pass at Fri 23:59, Sat 00:00 and Sun 23:59 ET; `tests/queryplan.test.ts` and `tests/sitemap.test.ts` pass; lint and `opennextjs-cloudflare build` pass.
4. **Docs.** A runbook section "Earnings week post" (what runs, the switch, alerts, the correction statements, posting after the card); Rick's answers in `docs/decisions.md`.

**Before the card work is called done**: the preview test.

**Deploy order** (owner only, no migration): the site first (it knows `week` and the weekend turn), then the generator with `WEEK_POST_ENABLED = "true"`. On the first Friday, check the `job week` log line, the post in both languages and its card in a WeChat and an X preview.

## Appendix: prior-quarter figures from XBRL, for later

From the critic's estimate: 2 to 4 EDGAR `companyconcept` requests per domestic company, cached per ticker and period in R2 (once a quarter). That is about 50 requests in a quiet week and 340 to 680 in a peak week, each message under 50 subrequests. Prerequisites: `accn` in `FactEntry` so each figure links to its filing; 80 to 100 day 10-Q/10-K facts only; a period end within 120 days before the report; labelled "as filed", with the revenue tag named; formatted by the generator's cover-line formatter. It cannot cover foreign filers (no Chinese ADR) or a preceding fiscal Q4. Build it only after the week post shows readers come back.
