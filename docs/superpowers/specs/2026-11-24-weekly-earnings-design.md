# The Friday earnings week post

Date: 2026-11-24. Status: Rick chose this form on 2026-11-24 (`docs/decisions.md`). The details below are waiting for his answers to the open questions. Nothing is built until then.

This design replaces the live week page proposed earlier the same day, for three reasons:
- The critic's review showed that the live page mostly repeated the dated calendar.
- Its "last quarter" figures would be empty through January, and never exist for foreign filers.
- Its generator state had a race.

Rick then chose "Friday weekly post (Recommended)". The design was revised the same day after the critic's second review: the post gets its own guard, a stored window, a correction path that works, its date sources shown, and a minimum size.

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
- Run key `week:<friday>`; slug `earnings-week-<friday>`, for example `earnings-week-2026-12-04`.
- `trading_date` is the Friday.
- The facts bundle is `week/<friday>.json` in R2. It holds every row and number the page states (below).

**Written by code, no model.** The title, summary, cover lines and body are templates per language, filled from the bundle, as the IPO write-ups are (`crates/core/src/ipo/writeup.rs`). Chinese has its own templates; it is not translated from the English.

### The bundle

| Field | What it holds |
|---|---|
| `friday`, `next_monday` | The run's dates. Every date in a build comes from these, never from the build day |
| `cutoff` | The time of the first successful build; kept on every rebuild |
| `window` | The first and last report dates of the checks it lists |
| `listed` | The slugs of the checks it lists |
| `checks[]` | Slug, ticker, name, trading date, period label, the `en` and `zh` cover lines exactly as stored in `post_translations`, the `zh` status, and the market-cap rank |
| `expected[]` | Ticker, name, tier, date, slot, status, `source`, `proof_url` and `checked_at`, from `earnings_calendar` |
| `events[]` | Series, date, time and official link, from the digest's `gather` |
| `counts` | Checks, expected, confirmed, write-up companies expected, and companies per day |

### Sections

**1. "Earnings checks published Nov 30–Dec 4" / "11月30日至12月4日发布的财报速评"**
- **Which checks:**
  - the English text is published and `published_at <= cutoff`;
  - the slug is not in the previous published week bundle's `listed`;
  - the report date is on or after the previous week post's Monday (for the first post, on or after this week's Monday).
- **Why this rule.** Rebuilding a check or a week post overwrites `published_at`; this rule does not move when that happens. A check published after the build lands in the next post. A failed or missing week is covered by the next post, and the heading names the dates it spans (`window`).
- **Order:** market value from the dated snapshot, the digest's rule. A company missing from the snapshot, or at 0 there, comes after the rest, by ticker. ANT is at 0, so it sorts last.
- **One line per check:**
  - display name and ticker;
  - the period, by the search-title rule of 2026-11-19: "Q3 2026" only when that calendar quarter ended within 120 days before the check's date, else "Q3 fiscal 2027";
  - the check's two cover lines in that language;
  - a link to `/{lang}/p/<slug>`.
- **Cover lines:** a line with no digit (`American International Group results`) is left out.
- **Chinese translation failed** (EMR and COP today): the line keeps its Chinese cover lines and links to `/en/p/<slug>`, marked "（英文）". The Chinese page of a failed translation redirects to the Chinese home page (`resolvePost`).
- **Chinese translation pending:** the line links to `/zh/p/<slug>`, which shows the "coming soon" notice.

**2. "Expected next week, as of Fri Dec 4, 18:05 ET" / "下周预计公布（截至美东时间12月4日周五18:05）"**
- **Layout:** Monday to Friday, then slot: before open, after close, time not given. A day with no reports reads "No reports expected." / "暂无。"
- **Write-up companies** come first, one line each, with the status in words: "confirmed" or "date not confirmed" (`日期已确认` / `日期未确认`).
- **Calendar-only companies:** one line per slot and status, for example "Not confirmed: Copart (CPRT), NIO (NIO)", so a name is never more than a line from its status. A `reported` row dated next week (unlikely on a Friday) reads "reported" / "已发布".
- **Links:** every name links to its ticker page; every calendar row has one (`tickerKnown`).
- **Left out:** quarantined rows, and any row dated before `next_monday`, so the stale "not yet matched to a filing" rows never appear.
- **Economy & Fed:** one line for next week, each event linking to its official page.
- **Where the dates come from:** the section ends with a sentence in the calendar's own words (`earnings.legend`, both languages, without its tier sentence): "Confirmed: the company announced the date. Unconfirmed: the date comes from an aggregator and may change. Reported: the results are filed with the SEC." A test keeps the Rust copy equal to the web message.

**3. "The calendar, updated live →" / "实时日历 →"**, linking to `/{lang}/earnings?week=<next_monday>`.

### Title, summary, cover lines

**Titles** (checks named largest first, at most three):

| Case | English | Chinese |
|---|---|---|
| 3 or more checks, with companies expected | `Earnings this week: Adobe, Broadcom, Costco; 8 expected next week` | `本周财报：Adobe、Broadcom、Costco；下周预计8家` |
| 2 checks | `Earnings this week: Salesforce and Marvell; …` | `本周财报：Salesforce和Marvell；…` |
| 1 check | `Earnings this week: Marvell; …` | `本周财报：Marvell；…` |
| none expected next week | the clause after `;` is dropped | the clause after `；` is dropped |
| no check | `Earnings next week: 14 companies expected from Nov 30` | `下周财报：11月30日起预计14家` |

**Summary:** `PreBell checked 3 earnings reports published Nov 30–Dec 4. 8 companies are expected to report Dec 7–11, as of Friday 18:05 ET; 1 date is confirmed.` The confirmed clause has three forms: "no date is confirmed", "1 date is", "2 dates are". Singular and plural also apply to the report and company counts.

**Cover lines:** `3 checks this week` / `本周 3 篇速评`, and `8 expected next week` / `下周预计 8 家`.

Every variant is pinned by a fixture.

### Minimum, empty weeks and holidays

- **Minimum:** publish only with at least 2 checks in section 1, or at least 2 write-up companies expected next week. Otherwise the run is `skipped`, with a log line and no alert, and its checks appear in the next post that publishes. On today's data this keeps Dec 4, 11 and 18, and drops the Christmas and New Year Fridays.
- **Holiday Fridays** (Dec 25, 2026; Jan 1, 2027) follow the same rule. Polls still run on them, because `in_window` checks only the weekday and the hour.
- **Holiday calendar refresh:** `refresh_due` has no trading-day test. The builder confirms whether the calendar job refreshes on a holiday; if it does not, the build waits for the 21:45 ET fallback. A test pins both Fridays.

### The guard

`writeup::verify_page` cannot be used:
- it takes `&IpoArticleFacts` (`writeup.rs:1740`);
- it rejects every amount written with a unit as "not exact" (`writeup.rs:1765-1775`), which every cover line is.

The week post gets its own `week::verify_page`, built from the same parts: `numbers::extract`, `numbers::check_allowed`, and `writeup`'s date pattern (exported). It works in three steps:
1. **Cover lines:** each one on the page must equal, character for character, a line stored in the bundle for that slug, and is then removed. The bundle copied it from `post_translations`, where the check's own number check passed it.
2. **Names:** company names and tickers are removed as whole words. 3M (`MMM`) is a write-up company with a digit in its name.
3. **Everything else:** each remaining number must be a bundle field: a date, the "as of" time, a count, a fiscal year or a slot time.

This proves the page is the bundle, copied exactly. That a date is true rests on its stored source, which section 2 states. A page that fails is not published. It retries, then fails with an alert, as the IPO write-ups do.

### Publishing, never half

As in `ipo::article` (`article.rs:18-25`):
1. Both languages are rendered and checked.
2. The bundle goes to R2.
3. The post row is upserted without text. It stays invisible, because the site shows a post only through a published translation.
4. Both translations land in one transaction through `revise_post`.

A failure before step 4 leaves nothing visible, and the next try writes into the same post.

### Correcting a post

A post is frozen, so a correction is a rebuild:
1. The owner fixes the data (a calendar row or a check) and sets `runs.status = 'pending'` for `week:<friday>`. The runbook gives the statement.
2. Every poll in the window, on any weekday, reads `runs` for a pending `week:*` run that already has a post: one read on the primary-key range `'week:'` to `'week;'`. When it finds one, it queues `week` for that run.
3. The rebuild takes every date from the run's `friday` and its stored `cutoff`, so it describes the same week.
4. It does not call `upsert_post`, which overwrites `published_at` (`store.rs:146-155`). It publishes through `revise_post`, which sets `revised_at` and keeps `published_at` (`store.rs:157-159`), so the page shows "Updated".
5. It writes a new cover stem, `earnings-week-<friday>-r<n>`. `cards_tick` skips a stem whose PNG exists (`card.rs:242`), so only a new stem gets a new card.

**A listed check revised later** is the one case where a post would show a figure that its check no longer states. Each Friday build:
- reads the last four published week bundles (four R2 reads);
- runs one D1 query for any of their `listed` checks whose `COALESCE(revised_at, published_at)` is later than that bundle's `cutoff`. A rebuilt check gets a new `published_at`, a revised one `revised_at`, so both are caught;
- raises a once-only warn alert, `week-stale:<friday>`, naming the slug. The owner decides whether to rebuild that week.

**To take a post down at once**, set both translations to `failed`.

## What being a post brings (traced in the code)

| Where | In the code | Change needed |
|---|---|---|
| Home recent list | `listRecentBriefs` takes every type but `earnings` (`earnings.ts:300-312`), and `loadHome` merges it | none |
| Archive | The default view binds `IN (?, ?)` for premarket and recap (`posts.ts:229`); other types appear only under their filter | `week` in `CONTENT_TYPES`; `types.week` = "Earnings week" / "财报周报"; the default view gets a third bind, with its query-plan test (lead's call) |
| Post page | `loadBrief` loads no facts for the type, and the body renders as Markdown. `BriefView.tsx:104` shows "✓ Every figure traced to the filing" for any type that is neither a brief nor an IPO, which is false for aggregator dates | a `week` line: "✓ Figures from PreBell's earnings checks. Dates as of the time shown below: confirmed ones announced by the company, the rest from an aggregator." / "✓ 数字出自盘前铃财报速评。日期以下文所示时间为准：已确认的由公司公布，其余来自第三方汇总。" |
| Prev/next | briefs only (`getAdjacentPosts`, skipped when `briefKind` is null) | none |
| Cover and card | `COVER_POSTS` (`store.rs:621-628`) has no type filter, so the first card renders with no change. The kicker is the date alone (`card.rs:223-226`) | the card task: `cover::week`, and a kicker case. The kicker needs `CoverPost` to carry `content_type`: the query selects it, but the struct keeps it only for IPOs |
| Sitemap, JSON-LD | Every published post (`sitemap.ts`); `NewsArticle` (`seo.ts`) | none |
| MCP | `latest_brief` binds premarket and recap only (`mcptools.ts:966`) | none; it stays out |
| Digest | The picks read `content_type = 'earnings'`; "Full calendar →" links `/en/earnings?week=<monday>` (`render.rs:199`) | none; the digest is unchanged (lead's call) |
| Edge cache | Posts are cached for 300 s (`next.config.ts`) | none |

**Before the card task ships**, a week post has no SVG cover, so it shows the flat tile and the default card.

**`cover::week`** uses Latin text only, as the card fonts do. It shows next week as five day bars: each bar's height is the number of companies expected that day, labelled with the day and the count, with the largest expected ticker above it. All of these numbers are in `counts`. When no company is expected next week, it shows this week's checks as ticker tiles instead.

## The calendar's weekend turn (split out)

This ships now as its own small PR, which the lead started separately. The undated `/earnings` uses a new `readingWeek(today)` in `earnings.ts` instead of `weekStart(todayEt())`: the coming week from Saturday 00:00 ET. The "back to this week" test uses it too.

On Saturday and Sunday the kicker reads "Market calendar · Coming week" / "市场日历 · 下周", and the button reads "Coming week" / "下周". Dated `?week=` pages, the range, the noindex distance and the home strip are unchanged. The post depends on it only through the link in section 3.

## Running it, and costs

**Trigger.** It runs on the same poll as the digest, under the digest's conditions:
- Friday from 18:00 ET (`digest::due`);
- that day's calendar refresh has finished, or it is 21:45 ET or later (`closing`);
- the run `week:<friday>` is not `published`, `skipped` or `failed`. A `failed` run is never queued again except through the correction path.

Corrections are queued on any weekday, as described above.

**Running**
- The message goes on `prebell-earnings-build`, serial with the earnings builds as `ipo:article` is. A check build already in flight finishes first; a test pins a check published at 18:02 appearing in an 18:05 build.
- The switch is `WEEK_POST_ENABLED`, exactly `"true"`, independent of `DIGEST_ENABLED`.
- Run state is the `runs` row, with the IPO write-ups' lease and retries (5, 15, 30 and 60 minutes).
- Retries can run out when polls stop at 22:00 ET. So from 21:55 ET (`digest::LAST_TRY`), a run that is still neither published nor skipped raises a once-only error alert: "week post for <friday> not published".
- A Friday evening on which every poll fails queues nothing, as for the digest.
- No new cron.

**Costs**
- **Model calls:** zero. **Fetch subrequests:** zero.
- **D1 per Friday build**, about 10 queries:
  - run state;
  - the previous bundle's post;
  - the checks;
  - next week's calendar, through `digest::gather(now, monday, …)`;
  - events;
  - the stale-check query;
  - the post upsert and the `revise_post` transaction.
- **The digest's `gather`** is extended to keep each calendar-only name with its status. Today `Slot.others` is a count, and `Company.confirmed` folds `reported` into confirmed. The email's output stays the same, pinned by `the_fixture_week_renders_exactly`.
- **D1 per poll:** one indexed `runs` read for corrections.
- **R2:** up to 5 reads (the previous bundles) and 2 writes (the bundle, and the cover once the card task ships).
- **Queue:** one message a week, plus at most four retries.
- **CPU:** the digest is a separate message, so the two never share one invocation's 10 ms CPU or 50 subrequests. The week build's CPU is templating and the guard over at most about 60 checks and 200 names. The builder profiles a 60-check, 200-company fixture with `--cpu-prof`, and reads `cpuTime` in `wrangler tail` on the first live run.
- **Storage:** about 20 KB a week, plus the card.

## Risks

| Risk | Handling |
|---|---|
| A check publishes after the build | `cutoff` and `listed`: it goes into the next published post, under its dates |
| A listed check is revised after the post | The `week-stale` alert, and a rebuild through the correction path |
| Unconfirmed dates in a frozen post | "as of" in the heading and summary, a status on every line, the source sentence, the reworded ✓ line, and the live calendar link |
| "Not yet matched to a filing" rows (PFE, NVO, TM, TECH, NBIS, MNSO, ZTO) | Section 2 starts at `next_monday`, so they never appear. Their wording on the calendar is a separate fix. Pfizer's row probably predates PreBell's first feed read; it needs a one-off correction after checking EDGAR |
| A peak week of about 60 checks and 200 names | About 15 KB of Markdown, no cut-off, ordered by market value, three names in the title. A test renders the fixture once through the site's `PostBody` |
| A cover line that differs from its check's summary | Walmart's cover line says 1888.3亿美元 and its Chinese summary 1888.4亿美元. The post shows the cover line, as the check's own cover does |
| DST in "as of" | Fixtures on an EST Friday (2026-12-04) and an EDT Friday (2027-03-19) |
| SEO | One dated URL a week with a title that names companies; no duplicate of a calendar page |

## Distribution and the stop rule

**Each Friday evening or Saturday**, Rick:
- opens the Chinese post and reads it;
- shares its dated link in the groups he uses, and the English link on X;
- where links show no preview, posts a screenshot of section 2 instead, which carries its "as of" heading.

Once the card task ships, he posts after the log line `job cards: card og/earnings-week-<friday>….png`, so that no platform caches the default card.

**Two-minute preview test**, before the card task is built:
- Rick pastes an existing check that has a card (for example `https://prebell.site/zh/p/adi-q4-fy2026-earnings`) into a WeChat chat and into X's composer.
- He notes whether the image, title and description show.
- If WeChat shows no image, the card task waits, and the Chinese route is the screenshot.

**Measure.** Cloudflare Web Analytics uses no cookies, so it cannot count returning visitors. The proxies are:
- weekly page views of `/zh/p/earnings-week-*` and `/en/p/earnings-week-*`;
- Saturday-to-Monday visits to `/zh/earnings` and `/en/earnings`;
- referrers.

**Stop rule.** December's posts run, but the clock starts at the first January post (Jan 8). After six posts from then, compare views per post, and weekend calendar visits against the average for the same weekdays. If neither rose, switch `WEEK_POST_ENABLED` off and keep the published posts. Comparing December with February would measure the earnings season, not the post.

## Out of scope

- A live week page and "Changes since Friday".
- Prior-quarter figures (see the appendix).
- Chinese company names.
- The IPO watch in the post.
- MCP.
- Any change to the digest.

## Open questions for Rick

1. **Where will you post it, and will you post every week?** This decides whether the card task is built.
   - *Recommended:* the Chinese link in WeChat groups and the English one on X, each Friday or Saturday, after the two-minute preview test.
2. **What counts as success, and when to stop.**
   - *Recommended:* the measure above, and the six-post stop rule counted from Jan 8.

## Calls the lead made

These are recorded here, not open:
- Friday build and Saturday turn, replacing "each Sunday" in the approved direction.
- Type `week`, with the labels "Earnings week" / "财报周报".
- `week` in the archive's default view.
- `prebell-earnings-build` as the queue.
- The digest unchanged.

Still the lead's, with the card task: the kicker (`EARNINGS WEEK · 2026-12-04` recommended) and the cover graphic as described.

## Build plan

Each task is one builder pass, then a separate review. The weekend turn is split out (above).

**1. Generator: the rows.**

Work:
- A Rust port of `calendarAligned` and `periodFromSlug`, with the TypeScript tests' cases: Nvidia, Disney, ConocoPhillips, and a slug named by date.
- The bundle type.
- The section 1 query from `cutoff` and `listed`.
- `gather` keeping each calendar-only name and its status.

*Accept when* unit tests cover:
- a check published after the build, which appears in the next post only;
- a check re-run after it was listed, which is not listed twice;
- a failed previous week, whose range is labelled;
- a failed and a pending Chinese translation;
- a cover line with no digit;
- two checks on one date;
- ANT's place in the order;
- a quarantined row;
- mixed statuses in one slot, and a `reported` row next week;
- `the_fixture_week_renders_exactly` still passing, unchanged.

**2. Generator: the post.**

Work:
- The EN and ZH templates.
- `week::verify_page`.
- The publish order.
- The trigger.
- The correction path and the `week-stale` check.
- The 21:55 alert.
- `WEEK_POST_ENABLED`.

*Accept when:*
- Fixtures render exactly for:
  - every title and summary variant;
  - a week with no checks;
  - a week below the minimum, which is skipped;
  - 60 checks with 200 companies;
  - Dec 25 and Jan 1;
  - an EST and an EDT "as of";
  - the source sentence, in both languages.
- The guard accepts exact cover lines, rejects one with a changed digit, rejects a planted number, and passes 3M.
- A Tuesday correction rebuilds last Friday's post with last Friday's dates, sets `revised_at`, keeps `published_at` and writes a new cover stem.
- A `failed` run is never queued again.
- The 21:55 alert fires once.
- A revised listed check raises `week-stale` once.
- A failure retries, then fails with an alert.
- A word-boundary test finds no `beat`, `miss`, `estimate`, `consensus` or `预期` in the templates.
- `schedule.rs` still asserts two crons.
- The CPU profile of the 60-check fixture is in the PR.
- `cargo test -p prebell-core` and the wasm build pass in the shared target dir.

**3. Site.**

Work:
- `week` in `CONTENT_TYPES`, with `types.week`.
- The `BriefView` ✓ line.
- The default view of `listPosts` with three types.

*Accept when:*
- Render tests show a week post in both languages, in the home recent list and in the archive.
- The 60-check body renders through `PostBody`.
- `tests/queryplan.test.ts` covers `listPosts` with three types.
- `tests/sitemap.test.ts`, lint and `opennextjs-cloudflare build` pass.

**4. The card (after Rick's preview test).**

Work:
- `cover::week`.
- The kicker case, with `content_type` in `CoverPost`.
- The en dash checked in the embedded font, or a hyphen used instead.

*Accept when:*
- A fixture SVG renders with `card-png`.
- A correction's new stem gets a new card.
- The cards tests pass.

**5. Docs.**
- A runbook section "Earnings week post": what runs, the switch, the alerts, the two correction cases (a calendar row fixed, a listed check revised) with their statements, and posting after the card.
- Rick's answers in `docs/decisions.md`.

**Deploy order** (owner only, no migration):
1. The site (task 3), after the weekend-turn PR.
2. The generator, with `WEEK_POST_ENABLED = "true"`.
3. On the first Friday, check the `job week` log line and the post in both languages.
4. Task 4, on its own, after the preview test.

## Appendix: prior-quarter figures from XBRL, for later

From the critic's estimate:
- **Requests:** 2 to 4 EDGAR `companyconcept` requests per domestic company, cached per ticker and period in R2, so each company is fetched once a quarter.
- **Volume:** about 50 requests in a quiet week and 340 to 680 in a peak week, each message under 50 subrequests.

Prerequisites:
- `accn` in `FactEntry`, so each figure links to its filing.
- Only 10-Q and 10-K facts of 80 to 100 days.
- A period end within 120 days before the report.
- Figures labelled "as filed", with the revenue tag named.
- Formatting by the generator's cover-line formatter.

It cannot cover foreign filers (so no Chinese ADR), or a preceding fiscal Q4. Build it only after the week post shows that readers come back.
