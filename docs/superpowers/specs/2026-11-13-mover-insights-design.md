# Mover insights: a hook and a "what to watch" line on big movers

Status: design, awaiting Rick's approval. Nothing is built.

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
  that report: the check's headline figures and its guidance and outlook
  quotes, verbatim;
- the theme group it moved with, when the facts have one;
- dated events PreBell already stores: the company's next or latest report
  date from the earnings calendar, and the macro releases of the next
  seven days.

The prompt tells the writer to use only the pack and not what it
remembers.

### Which movers

A mover is eligible when its pack has at least one verified news reason or
a reported-results entry. A mover with only a theme fallback or no news
gets no insight: there is nothing to ground a reading in. At most the
eight movers a brief already lists (four up, four down).

### What the writer returns

One call per post covers every eligible mover in both languages. Per
mover: `hook_en`, `watch_en`, `hook_zh`, `watch_zh`, and for each language
the ids of the evidence items the line rests on.

- Hook: at most about eight words (about 14 Chinese characters), no
  numbers.
- Watch line: one sentence, at most about 35 words (about 70 Chinese
  characters). It names something concrete to track (a figure the company
  gave, a dated event, a stated plan) and may say why it matters. Formal
  register. No advice, no price prediction, no "analysts expect" unless an
  evidence item says so and is cited.

### Code checks (before the judge)

- Every number and date passes the existing `numbers::check` against the
  pack.
- Cited evidence ids exist and belong to this mover.
- A refusal list of advice and prediction wording in both languages (buy,
  sell, should, will rise, target price, 建议买入, 看涨 and the like).
- Length limits; the Chinese line contains Chinese; no markup or links.

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
  line, labelled "What to watch" / "后续关注". The existing English reason
  stays as it is. The home page's "Top movers" shows the hook.
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

## Open questions for Rick

1. Approve the model spend (estimate above), starting with the offline
   trial.
2. After the shadow stage passes, lines publish with no human reading each
   one. That is the point of the judge, and it is also the risk.
