# Mover insights: a hook and a "what to watch" line on big movers

Status: approved by Rick on 2026-11-13 ("approved", shown the summary with
the spend and unattended publishing marked as his decisions). A builder's
read-through then raised five questions; each is settled below and marked
"lead's call after the builder check". Nothing is built.

## Why

Rick, 2026-11-12: "so far the cotent is alright, but one thing concerns me
is the audience elasticity, like the content so far is dry and just the
numbers but without any future insights. for examples, the big movers, we
can say what to keep track in the future and see how alphasite handles
those".

How AlphaLab handles it (pages saved 2026-10-29, in the
`alpha-site-reverse` notes): each mover in its daily digest has three
parts. A short hook ("成長展望遭重新定價" on F5's 20% fall), the facts
with links, and one closing sentence in the site's own voice on what the
move means and what would confirm it, built from the company's own forward
numbers and hedged ("這些仍是未來目標，執行進度才是後續驗證重點"). Its
earnings posts stay backward-looking on purpose.

PreBell today: a mover has a one-line reason (a news summary in English,
or a code-written fallback). Chinese pages show no reason at all, because
reasons are English only.

## What Rick decided

- PreBell may state its own reading, in its own voice: "it is fine for
  whatever you write as long as it sounds formal. we are just saying our
  insights".
- An LLM writes it. Claude is the writer ("now we are using claude, and
  should be better").
- A judge model checks it in the backend ("you can also run a judge model
  in the backend to see if you think it is valid"). DeepSeek may be the
  judge ("you can also utilize the deepseek model for judge if necessary").
- Asked whether he wanted a hook and a "what to watch" line per big mover
  that reads like AlphaLab's, with every number and date checked against a
  source and the framing in PreBell's voice: "yes".

## Goals

1. Each eligible mover in the premarket brief, the 8:35 update and the
   recap carries a **hook** (a few words) and one **"what to watch"
   sentence**: what to track next and why it matters, formal in tone.
2. English and Chinese. Chinese pages gain something to read on each
   mover for the first time.
3. Every number and date in a line is in the mover's evidence, and a judge
   confirms that what the line *says* is supported by that evidence.
4. It is measured before it is trusted: a shadow stage where lines are
   written and judged but not shown, which Rick reads.

## Non-goals

- No price targets, no buy/sell/hold, no prediction of where a price goes.
- No new data source and no paid data. No analyst consensus.
- Earnings checks and IPO write-ups do not change in this version.
- The MCP server does not expose insights in this version.

## Anti-goals (it has failed if any of these happen)

- A false statement of fact published under PreBell's name.
- Wording a reader could take as investment advice.
- A brief that is late, blocked or failed because of this feature.
- Chinese and English lines that say different things.
- Model spend that grows without anyone deciding it should.

## Constraints

- Cloudflare free plan: 50 fetches and 10 ms CPU per Worker invocation.
  The brief build already uses most of its fetch budget
  (`crates/core/src/budget.rs`), so this work runs in its own queue
  message, never inside a build.
- Model calls cost money. Rick approves the spend before the shadow stage
  starts (see Cost).
- House rule, unchanged: code decides what is true. The model writes
  prose; it never supplies a number, a date or a fact.
- The "not investment advice" line stays on the legal page only (Rick,
  2026-11-05).

## Design (my calls unless marked as Rick's above)

### Evidence pack

For each mover the writer and the judge see the same pack and nothing
else, each item with an id:

- the move itself (ticker, name, percent, session) from the facts;
- its verified news reasons: title, outlet URL, the verbatim quote and the
  English summary (`NewsReason`);
- when the mover reported results and PreBell has an earnings check for
  that report, out before the post (before a brief's facts were built, or
  before the close a recap describes): the check's verified headline
  figures, each written by code from its verified value, unit and fiscal
  period ("Revenue: $18.52 billion, reported for Q1 of fiscal 2027.", the
  verbatim quote kept beside it for people), and its guidance quotes,
  verbatim, but only those whose own quote names the period they apply to
  (a guidance note has no period field, and its summary is model-written);
- the theme group it moved with, when the facts have one;
- dated events PreBell already stores: the company's next or latest report
  date from the earnings calendar with its status (reported, confirmed,
  or "Expected report date, not confirmed by the company"; quarantined
  dates are left out). Macro releases are not in a mover's pack (trial run
  1: they made filler, and the brief has its own macro section).
- each news item names its outlet: the 25 allowlisted outlets by name,
  common mover-news sites by their brand (Seeking Alpha, Benzinga,
  TipRanks, …), any other site by its host. An aggregator, search page or
  social platform (Google, Yahoo Finance, MSN, Bing, Stocktwits, Reddit, X,
  Facebook, YouTube) is not an outlet: its item has no
  outlet, cannot be cited for a fact and does not make a mover eligible.

The prompt tells the writer to use only the pack and not what it
remembers.

### Which movers

A mover is eligible when its pack has at least one verified news reason,
or PreBell has published an earnings check for the report that moved it.
A mover with only a theme fallback, only a link to a SEC filing, or no
news gets no insight: there is nothing to ground a reading in. At most the
eight movers a brief already lists (four up, four down).

Lead's call after the builder check: "reported results" means PreBell has
an earnings check for that report, because only then does the pack hold
figures and guidance. A mover with both a news reason and a report gets
its earnings check looked up too (one D1 read by ticker and report date;
`Fallback.post_slug` is only set when there is no news reason).

### What the writer returns

One call per post covers every eligible mover in both languages. Per
mover: `hook_en`, `watch_en`, `hook_zh`, `watch_zh`, and for each language
the ids of the evidence items the line rests on.

- Hook: at most eight words (20 characters in Chinese, raised from 14
  after trial run 1 for lines carrying Latin names), no numbers except in
  a name a cited item writes the same way ("GTA 6").
- Watch line: one sentence, at most 40 words (100 characters in Chinese,
  raised from 80). It names something specific to this company to track,
  from its evidence (a figure or target the company gave, a stated plan or
  timetable, a problem the evidence names, its report date), and says what
  would confirm or change the picture; never a market-wide or macro event.
  If the evidence offers nothing of that kind, the mover gets no line.
  Formal register. No advice, no price prediction.
- News items are the reasons PreBell's brief gives for the move, so a
  hook may tie the move to one.
- People (Rick, 2026-11-17: "Allow it if attributed"; this replaces the
  lead's rule of 2026-11-13): a line may repeat a claim about a named
  person, including an allegation, only when it names the outlet that
  reported it, in a reporting frame ("Benzinga reports …", "据Benzinga
  报道……"); never as PreBell's own claim. Naming an executive for what
  they said in their role is fine. Code has no word list for it: such a
  claim comes from a cited news item, whose outlet the attribution rule
  already requires; the judge's `personal_claims_attributed` flag (judge
  v3) catches a claim that is not attributed.
- An unconfirmed report date is never the company's own: in Chinese not
  "公司预计于…", but "财报日期暂定为…（未经公司确认）" or "市场预计其于…".
- An outlet is named as the pack gives it, in both languages, inside an
  attributing frame ("X reports", "according to X", "据X报道", "X称"); any
  case, or the host with its top-level domain, is also accepted. An
  ordinary word that happens to be a brand ("listed on the Nasdaq") is not
  attribution. Chinese
  figures use 万 and 亿, never 百万.

Lead's calls after the builder check:

- **Attribution.** The reading is PreBell's; the facts under it are not.
  A fact that comes from a news item is attributed to its outlet by name
  in the line ("Reuters reports …", "据路透社报道…"), as the IPO news items
  already are. A fact from the company's own release or filing is stated
  as the company's ("the company guided …"). Code checks that the outlet's
  name appears whenever a cited item is a news item. An outlet's mistake
  is then reported as that outlet's, not stated by PreBell.
- **Analyst actions.** Upgrades, downgrades and initiations are verified
  mover reasons today. A line may say that a named firm upgraded,
  downgraded or initiated coverage, attributed like any news fact. It
  never repeats the rating word ("Buy", "Overweight") or a price target,
  and "analysts expect" is not allowed. The thing to watch must be
  something else in the pack (a company figure, plan or dated event). A
  mover whose pack holds only an analyst action gets no insight.
- **Dates.** A date in a line must be a date in the pack. Relative times
  ("next week", "下周") are refused, because code cannot check them. A
  fiscal label ("Q3", "fiscal 2027") must appear in a cited evidence
  item. A line that cites or writes a report date the company has not
  confirmed must hedge it in the date's own clause ("expected",
  "scheduled", "unconfirmed"; 预计, 预期, 暂定, 未确认), or it is refused
  (lead's rulings after the reviews of PR #67).

### Code checks (before the judge)

- Every number passes the existing `numbers::check` against the pack.
  That function skips dates, clock times and fiscal labels on purpose, so
  this feature adds its own date check in both languages (see Dates
  above).
- When a cited evidence item is a news item, its outlet's name is in the
  line.
- Cited evidence ids exist and belong to this mover.
- A refusal list of advice and prediction wording in both languages (buy,
  sell, should, will rise, target price, rating words, 建议买入, 看涨 and
  the like). It applies even when the evidence itself uses the word.
- Length limits; the Chinese line contains Chinese; no markup or links.
- Only the judge can catch (code lets these through on purpose, after the
  third and fourth reviews): the same figure given a different meaning (a
  revenue figure stated as a buyback): code confirms that the judge's
  quoted words are verbatim and that the line's money and percentage
  figures sit inside them, but not that the words mean what the line says;
  a claim about a named person that is not attributed to its outlet (the
  judge's `personal_claims_attributed` flag); a reordered consensus phrase ("above the expectations of
  analysts"); 持有 read as "holds shares" when it is advice
  ("投资者可继续持有股份至11月19日"); a bare "target" in a line that does
  not cite the analyst item ("after Morgan Stanley lifted its target");
  also a colon joining a second clause, and an abbreviation hiding a
  sentence end.
- Added after the third review: "buy" or "sell" after "upgraded/downgraded
  … to", as a noun ("a compelling buy"), in "buying opportunity", or in
  any line citing an analyst action is refused; "will", "would" and "plan"
  no longer mark a deal; "the Nth" is a day unless a count follows ("the
  10th straight quarter"); an unconfirmed date's hedge must govern it
  (within six words before it in its clause or three after; in Chinese
  eight characters before or six after).
- Added after the second review: the refusal list targets advice senses,
  not bare words ("agreed to sell" and "targets a launch" pass; "a buy",
  "price target" and "investors may wish to buy" do not), and refuses
  comparisons with analysts' or the Street's estimates or the consensus.
- Added after the review of PR #67: no number from an analyst action
  (its target or rating); figures in words and dates that cannot be
  anchored (weekdays, a bare day, a month alone) are refused; the judge's
  quoted support must cover every number and date in the line.

A mover whose line fails a check is dropped; the writer is not asked
again.

### Judge

A second model call, with its own prompt, sees the pack and the lines that
survived the code checks. For each mover and language it answers:

1. Supported: for each factual claim in the line it must return the
   evidence id and the exact supporting words. Code then confirms those
   words are verbatim in that evidence item, so the judge cannot invent
   support.
2. No advice and no price prediction.
3. Formal tone.
4. The Chinese line says the same thing as the English one.

Any "no" drops that mover's insight in **both** languages, so the two
never differ. The judge can only remove lines; it never edits them.

Which model judges is a measured choice, not a guess: the shadow stage
runs both Claude (different prompt from the writer) and DeepSeek as judges
on the same lines, and the one that agrees better with Rick's own labels,
and catches more of a seeded set of deliberately wrong lines, becomes the
judge. Setting: `INSIGHT_JUDGE_PROVIDER`.

Measured in the offline trial (2026-11-17): on Rick's 15 labels Claude
agreed 15 of 15 and DeepSeek 14 of 15, so Claude is the judge
(`INSIGHT_JUDGE_PROVIDER` defaults to `claude`); the shadow stage runs one
judge, strict: a judge reply with any text after its JSON is a failed
call, never a pass.

Lead's calls after the builder check:

- **The writer is always Claude.** The step has its own setting
  (`INSIGHT_WRITER_PROVIDER`, default `claude`) and does not follow the
  site-wide `LLM_PROVIDER`. If the site is switched to DeepSeek as a
  fallback, or the Anthropic key is missing, the insight step skips; it
  does not write with DeepSeek.
- **Rick's labels.** The offline tool writes one local page listing lines
  in random order with the judges' verdicts hidden: every line a judge
  dropped or the two judges disagreed on, plus judge-passed lines, about
  60 in all (about 20 minutes of Rick's time). Each line shows its
  evidence and three choices (fine, wrong fact, reads as advice or is not
  formal). The page saves the labels to a file the tool reads back to
  score the judges. Rick may change the number.

### Where it runs

- A new job step, `insight`, queued after a premarket brief, a premarket
  update or a recap publishes. Its own invocation. It reads the post's
  stored facts bundle from R2, builds the packs (one D1 read for the
  calendar, R2 reads for any earnings checks), calls the writer, runs the
  code checks, calls the judge, and writes the result.
- The result is stored with the post's facts as a new optional field per
  mover, so old facts still parse. The web app renders it; it is not
  written into the article's Markdown.
- The 8:35 update rebuilds the facts, so its insights are computed afresh
  and the 8:00 ones are discarded with the old facts.
- Bounded like the share-card job: a small fixed number of attempts per
  post, then give up quietly. A failure logs a warning and never alerts,
  never retries a brief, never changes `published_at`.
- Every writer output, code-check result and judge verdict is saved to R2
  under `insights/<post slug>/…json`, so any published line can be audited
  later.

### On the page

- Brief and recap pages: under each eligible mover, the hook, then the
  line, labelled "Watch next" / "后续关注". (The premarket brief already
  has a section headed "What to watch" / "今日关注", so the per-mover label
  must not repeat it; wording for Rick to confirm with the screenshots.)
  The existing English reason stays as it is. The home page's "Top
  movers" shows the hook.
- A mover without an insight looks exactly as it does today.
- A small label near the first insight on a page says the lines are
  PreBell's reading, generated automatically. Wording for Rick to approve
  with the screenshots.

### Stages (`INSIGHTS_MODE = off | shadow | on`, default `off`)

1. **Offline trial.** A local tool (extending `llm-eval` on the `eval`
   branch) runs writer and both judges over the stored facts of the last
   two weeks of briefs and prints every line with its evidence and
   verdicts. Rick reads them. Costs a few dozen model calls.
2. **Shadow.** The Worker computes and stores insights for live briefs but
   the site does not show them. One week.
3. **On.** Shown on the site.

Bar to move from shadow to on (Rick's to change): Rick has read at least
50 judge-passed lines; none states a false fact or reads as advice; the
chosen judge catches at least 9 in 10 of the seeded wrong lines.

## Cost

Two model calls per post (writer, judge), three posts a trading day
(brief, update, recap): about six calls a day. The one measured figure we
have is the IPO test on 2026-11-11, three Claude calls for about $0.04.
At that rate this is about $2 to $3 a month; the offline trial will give
the real number before anything runs daily. The shadow stage doubles the
judge calls while both judges are compared. If Rick sets a monthly limit,
the step stops for the month when its own count of calls passes it.

## Left to the builder

Prompt wording; field and key names; the exact refusal list; page layout
and styles (with screenshots at phone and desktop widths, both languages);
the seeded wrong-line set; how the offline tool prints its table; how
attempts are counted.

## Rick's decisions

1. The model spend (estimate above), starting with the offline trial:
   approved, 2026-11-13.
2. After the shadow stage passes, lines publish with no human reading each
   one: approved, 2026-11-13.
