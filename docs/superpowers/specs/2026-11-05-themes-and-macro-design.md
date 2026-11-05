# Themes and macro: the brief covers the day's story, not just ten tickers

Date: 2026-11-05. Status: draft for builder check, then owner approval.

## Intent

Rick's words on the 2026-11-05 premarket: "how come we did not have anything
related to SanDisk, Micron etc. they are doing so bad recently. and we did
not say about the Trump midterm elections at all. they are all huge news."

Today's brief reports the five biggest gainers and losers by percentage out
of about 420 liquid names, plus a one-query "Macro & overnight" section. A
group of twelve memory and semiconductor names each down 3 to 6 percent is
invisible unless one of them makes the top five. The election got one clause
from a single WSJ live-blog hit. Readers want the story of the morning: which
groups are moving together and why, and what the big news is.

This adds two things to both briefs (premarket and after-close recap):

- **Themes.** Clusters of movers by industry group, detected in code from
  the quotes the brief already has, with the reasons it already fetched.
- **Macro that covers the day.** Three targeted news queries instead of one
  generic one, up to five items, and the summary leads with the biggest
  story.

## Goals

- When several names in one industry move the same way, the brief names
  the group, lists the members with their moves, and says why when a reason
  is known.
- On a day with a market-wide story (election, Fed, oil, a data print), the
  summary says so in its first sentence and the macro section carries it.
- Everything stays inside the Free plan: no more than 2 extra subrequests
  per brief, no per-request CPU added to the web Worker.
- Every figure in the new text is verified the way all figures are today.

## Non-goals

- Per-stock fundamentals, analyst ratings, or sector ETFs.
- A separate "sectors" page or a sector heatmap.
- Changing how the ten top movers are picked or written up.
- Replacing Yahoo or Tavily, or adding a data vendor.

## Anti-goals

- Themes that are noise: two random names in a sector that happen to be up.
  A theme must be a real cluster (rules below) or it must not appear.
- Invented aggregates. Any number the brief prints about a theme is computed
  in code and verified, never estimated by the model.
- Going over the subrequest budget and losing the whole brief. Themes and
  the wider macro step must degrade to nothing before they can cost the
  publish.
- Stale or wrong industry labels. A name filed under the wrong group is
  worse than no theme.

## Constraints

- Free plan: 50 fetch subrequests per invocation. Today's premarket uses
  about 39 at facts time and 41 to 43 after the drafts (runbook). The
  budget for this feature is +2 at facts time, guarded.
- Free plan CPU is nominally 10 ms per invocation; today's premarket used
  261 ms and completed. Theme detection must be linear in the number of
  quotes and allocate little; no per-theme sorting of the whole universe.
- Yahoo v7 quotes carry no sector or industry. The universe file is the only
  place to hold them, as static data.
- Generator diagnostics tests pin line indexes; the web `trim.test.ts` pins
  the section headings from static fixtures. New sections and lines shift
  them and the plan must list the updates.

## The industry map (decided)

`crates/core/data/universe.txt` gains two tab-separated columns after the
name: `Sector` (one of the 11 GICS sectors, English) and `Group` (a short
industry group label, 25 to 60 distinct values across the 522 tickers, e.g.
"Memory & storage", "Semiconductors", "Semiconductor equipment", "Cloud &
enterprise software", "Payments", "Regional banks", "Managed care",
"Pharma", "Biotech", "Oil & gas producers", "Oilfield services",
"Utilities", "Homebuilders", "Airlines", "Cruise lines", "Discount
retail", "Apparel & luxury", "Restaurants", "Media & streaming",
"Telecom", "Defense", "Rail & trucking", "Chemicals", "Steel & metals",
"REITs", "Insurers", "Asset managers", "Exchanges & data"…). Each group
has a Chinese label in a small map in code (the builder writes both lists
together).

The builder fills the two columns with model knowledge and a review pass;
the commit lists the groups and their member counts. Tests: every ticker
has a sector and a group; sectors are exactly the GICS eleven; group count
in range; no group with fewer than 2 members (singletons cannot form a
theme and should be merged into a neighbour).

`universe::group(ticker) -> Option<&'static str>` and
`universe::sector(ticker)` are added; the parser tolerates the old two-column
lines.

## Theme detection (decided)

Input: the liquid quotes the brief already has (premarket: all liquid
universe quotes with a premarket move; recap: liquid closes).

For each group: members present, members up, members down, and the move of
each. A **theme** is a group where at least 3 members moved the same
direction by at least `THEME_MIN_MOVE` (premarket 2.0%, recap 3.0%), those
members are at least half of the group's present members with any move
beyond ±0.5%, and the median move of the qualifying members is at least
`THEME_MIN_MEDIAN` (premarket 2.5%, recap 3.5%). Direction is the sign of
the qualifying members; a group with 3 up and 3 down is not a theme.

Themes are ranked by `median |move| × qualifying members`, and at most
`THEME_MAX` = 3 are kept. Each theme carries: group label, direction,
qualifying members sorted by |move| (ticker, name, move; up to 6 listed,
the rest counted as "and N more"), median move, and reasons.

Reasons: a theme reuses the `NewsReason`s of any qualifying member that is
also one of the ten top movers (already fetched, zero cost). If the top
theme has no such member, one Tavily search `"{group} stocks {direction
word} today"` plus one extraction call are made for it, under the budget
guard; the other themes never search. This is the +2.

Facts: `PremarketFacts.themes: Vec<Theme>` and `RecapFacts.themes`, with
`#[serde(default)]`; `Theme { group, group_zh, direction, members:
Vec<ThemeMember>, median_pct, more: usize, reasons: Vec<NewsReason> }`,
`ThemeMember { ticker, name, change_pct }`. `numbers_in_facts` gains every
member move and median so the model may cite them.

## Macro that covers the day (decided)

Replace the single query with three, run in order, each through the
existing `fresh_news` filter:

1. `"stock market futures today"` (as today; premarket) /
   `"stock market today closing"` (recap).
2. `"Wall Street markets news today"` — the day's headline story in
   non-futures framing (elections, policy, geopolitics, a big data print).
3. `"Treasury yields oil dollar Federal Reserve today"` — rates, energy,
   FX.

Hits are merged, de-duplicated by URL, capped at `MACRO_HITS_TOTAL` = 8
(dated first, as `fresh_news` already orders), and sent to one extraction
call asking for up to `MACRO_ITEMS` = 5 items, each verified against its
quote as today. Cost: 3 searches + 1 extraction = 4, versus 2 today, so
+2. The third query is skipped under the budget guard. The recap gets the
same macro section ("Macro & after the close" / "宏观与盘后"), new for
recap, placed before the after-hours section.

The prompts gain one rule for the summary: "If the macro items or a theme
describe a market-wide story, the first sentence of the summary names it."

## Budget guard (decided)

`build_facts` receives a `Budget` (a closure returning the subrequests used
so far, from the Worker's `CountingHttp`, the same source as the
`subrequests=` line). Optional work runs only while `used < BUDGET_SOFT` =
42: the third macro query, then the theme search, in that order of
sacrifice (the theme search is the first dropped). The diagnostics gain
`themes found=N kept=K searched=0|1` and `macro queries=3 hits=H items=I`.
Drafts still need 2 to 4 calls, so the ceiling stays under 48 on the
normal path and the 401-refresh path (+3) still fits.

## What the reader sees

**Brief page (markdown):** a new section "Themes" / "板块动向" between
the movers table and the macro section, present only when at least one
theme exists. The code renders a compact list first (one line per theme:
"**Memory & storage** ▼ median −4.1%: MU −4.1%, SNDK −6.0%, WDC −3.9%, and
2 more"), then the model writes one short paragraph per theme using only
the facts' figures and the theme's reasons; themes with no reason get a
single sentence that states the move without a cause (no "no clear news"
boilerplate in prose; the list line already carries the figures). The macro
section becomes "Macro & overnight" with up to five items, as today in
form.

**Front page:** under "Top movers" in the right column, a "Themes" rule
with one line per theme (label, arrow, median, first three tickers) linking
to the brief. Read from the facts the home page already loads for the lead.
When there are no themes the block is absent.

**Summary:** leads with the market-wide story when there is one.

## Decisions and calls (mine unless marked Rick)

- Themes come from static industry groups in the universe file, not from
  Yahoo (no field) and not from the model (unverifiable). (Zero fetches.)
- The thresholds above are my starting values; the plan pins them as
  constants with tests, and the runbook explains how to tune them.
- Themes reuse mover reasons and search at most once, for the top theme
  only. Everything else is numbers-only. (Budget.)
- Three macro queries, one extraction, five items, +2 subrequests, third
  query dropped first under the guard.
- The recap gains a macro section for symmetry; the after-hours section
  stays.
- Both briefs get themes; the front page shows them in the movers column.
- Rick asked for this on 2026-11-05 after seeing memory stocks and the
  midterms missing. (Rick.)

## Left to the builder

- The exact group list and the per-ticker assignments (with the review pass
  and the Chinese labels).
- Theme ranking tie-breaks and the "and N more" wording in both languages.
- The prompt wording for the themes paragraphs and the summary rule.
- Where `Budget` is threaded (through `Hooks`/`build_facts` arguments like
  `CoverMaker` was).
- Diagnostics line placement and the test updates it forces.

## Amendments after the builder check (2026-11-05, my calls)

- **The budget arithmetic was wrong.** The cover scene is one more fetch
  after the drafts, so today's premarket worst case is 44 and the recap,
  which has no macro section today, would gain +4 to +6, reaching 50 on the
  401-refresh path. The feature cannot ship on top of today's quote stage.
  Fix: **cut the quote stage, not the features.** Yahoo v7 accepts far more
  than 50 symbols per request; `QUOTE_BATCH` goes from 50 to 150 (and the
  response cap `MAX_QUOTE_BYTES` from 1 MiB to 2 MiB), so 522 symbols cost
  4 batches instead of 11, saving 7 subrequests per brief. The plan's first
  task is a live check of a 150-symbol v7 request (size, latency, no 429)
  before anything else is built; if Yahoo rejects it, the fallback is
  `MOVERS_PER_SIDE` 5 → 4 and a theme search only in the premarket, and
  Rick is told. With the saving, the new ceiling is about 43 normal / 46 on
  the 401 path for both briefs, under the runbook's 45 "at the limit" line
  on the normal path. The guard becomes `used + cost <= BUDGET_SOFT` with
  `BUDGET_SOFT` = 40 at facts time, and the order of sacrifice is: theme
  search, then the third macro query, then the second macro query.
- **SanDisk is not in the universe.** `universe.txt` is "S&P 500 ∪
  Nasdaq-100 as of 2026-06". The plan adds a universe refresh task: add the
  names that joined either index since June 2026 (SNDK among them) and
  remove the names that left, each addition with its source noted in the
  commit, staying at or below 550 tickers so the batch count holds. This
  changes the pool the movers come from, not the rule; the non-goal above
  is about the rule.
- **Acceptance is unit tests; the 2026-11-05 case cannot be replayed.**
  Stored facts keep only the ten movers, and v7 premarket quotes are
  live-only. To make the thresholds tunable, the facts gain
  `theme_candidates: Vec<ThemeStat>` (group, present, up, down, median
  move of the up side and of the down side) for every group with at least
  two members beyond ±1.5%, capped at 15 entries; the diagnostics line
  carries `themes found=N kept=K searched=0|1`. After a week the runbook
  shows how to read them and adjust the constants.
- `universe::group`/`sector` use a map built once (the existing
  `company_name` scans the file per call; the plan replaces that too).
- Recap: the rendered theme line's figures go into `table_values`, "and N
  more" counts above 10 are allowed, and member names go through
  `blank_names`, so the recap's rendered-body number check passes.

- **Section order follows the story, as AlphaLab's digest does** (their
  2026-11-04 digest runs macro → industry radar → unusual moves). Premarket
  body order becomes: Futures table, "Macro & overnight", "Themes",
  "Biggest premarket movers" (table + prose), "What to watch" when present.
  Recap: Index closes, "Macro & after the close", "Themes", "Notable
  movers", after-hours. The front page is unchanged (lead summary, movers
  column with the themes block under it). This moves the macro section
  above the movers table in `render_markdown` and reorders `all_headings`.
- Approved by Rick on 2026-11-05 ("can we merge it in and edit today's
  article"); today's article cannot be regenerated because premarket quotes
  are live-only.

## Open for Rick

- None blocking. The thresholds are tunable after a week of real briefs.
