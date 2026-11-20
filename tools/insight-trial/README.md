# insight-trial

The offline trial for mover insights (spec:
`docs/superpowers/specs/2026-11-13-mover-insights-design.md`, plan:
`docs/superpowers/plans/2026-11-13-mover-insights-trial.md`). It runs the
production insight code in `prebell_core::insight` over saved fact bundles:
the evidence packs, one Claude writer call per post, the code checks, and one
judge call per judge (Claude and DeepSeek) over the lines that pass the checks.
Nothing here touches the Worker or the site.

A native crate outside the workspace: it has its own `Cargo.lock` and
builds into its own `tools/insight-trial/target/`. Its `.cargo/config.toml`
pins that, overriding a `target-dir` an enclosing checkout's
`.cargo/config.toml` may set (the main checkout's local one points every
worktree at `~/.cache/prebell-target`). The `CARGO_TARGET_DIR` variable
would still win over both, so every command below starts with
`env -u CARGO_TARGET_DIR`.

## Spend

Every model call goes through `src/http.rs`, which prices it and refuses a
call that would pass `--budget` (USD, default 3.0). The cap spans runs: each
call is appended to `OUT/spend.jsonl` (time, model, role, tokens, USD), and a
later run with the same `--out` starts from what that file records. When the
cap is reached the tool stops, keeps what it has written, prints the spend so
far and exits non-zero.

Keys are read from `--dev-vars PATH` (only `ANTHROPIC_API_KEY` and
`DEEPSEEK_API_KEY`) or the environment. They stay in memory and are never
printed or written.

## Commands

From the repository root:

```sh
# 1. Save the last two weeks of briefs and recaps, three weeks of earnings
#    checks, the earnings calendar (every status but quarantined, as
#    [ticker, date, status]) (read-only: D1
#    SELECTs and R2 gets).
tools/insight-trial/fetch-facts.sh tools/insight-trial/facts

# 2. Writer, checks and both judges over every saved brief and recap.
cd tools/insight-trial
env -u CARGO_TARGET_DIR cargo run --release -- run \
    --facts facts --out out --dev-vars ../../crates/generator/.dev.vars --budget 3.0
```

Then, in the same directory:

```sh
# 3. Seeded wrong lines: up to five broken variants of each line that passed
#    the checks (a swapped number, a flipped direction, a swapped outlet, a
#    number changed in the Chinese line only, an advice clause), each judged
#    by both judges. Shares the spend ledger and cap with `run`.
env -u CARGO_TARGET_DIR cargo run --release -- seed \
    --out out --dev-vars ../../crates/generator/.dev.vars --budget 3.0

# 4. The labelling page for Rick: out/label.html (no network, verdicts hidden).
env -u CARGO_TARGET_DIR cargo run --release -- page --out out

# 5. When labels.json comes back from the page's "Save labels" button:
env -u CARGO_TARGET_DIR cargo run --release -- score --out out --labels ~/Downloads/labels.json
```

`--dev-vars` names the file that holds the keys: in the main checkout,
`crates/generator/.dev.vars` (a git worktree has none, so give its full path).
The ledger lives in `--out`, so a run into a new folder starts it empty.
To keep one cap across folders (trial run 1 is kept in `out-v1/`, ignored by
git as `out-*/`), copy the earlier `spend.jsonl` into the new folder first,
or lower `--budget` by what the earlier folder spent.

`--only SUBSTR` keeps the posts whose file name contains it (for example
`--only recap_2026-11-12`).

Re-runs keep what was paid for: `run` skips a post already complete in
`out/lines.jsonl`, and finishes one a budget stop left part-done by calling
only its missing judges; `seed` skips a post already in `out/seeded.jsonl`.
A judge reply that fails to parse is asked for once more at once, within
the budget, and the result is marked `retried`. The judge is strict, as in
the Worker: a judge reply with any text after its JSON is a failed call
(retried once the same way), never a pass. A judge call that still
failed for any reason (the budget, a network error, a reply that would not
parse) is made again on a re-run. `seed` writes a post
only when all its calls are done: a post the budget cuts part-way is not
written (its finished calls stay in the ledger) and is redone on a re-run.
`--fresh` redoes everything. `--plan` (on `run` and `seed`) prints the calls
the step would make and their expected cost, and makes none; without it the
plan is printed first and the step refuses to start when the expected cost
would pass what is left of `--budget`. `seed` instead seeds a fixed-seed
random sample of whole posts that fits what is left, and says which posts
and how many wrong lines and controls that sends (`--plan` prints the same).
The expected cost uses the average
output of earlier calls in the ledger, or an assumed 500 to 600 tokens per
mover plus 1,500 for Claude's thinking; each call still reserves its
worst case (16,000 output tokens for Claude, 4,096 for DeepSeek) before it
is sent.

## Output

- `out/lines.jsonl`: one object per mover that had a pack: `post`, `ticker`,
  `pack` (the evidence the models saw), `insight` (or null), `write_error`,
  `check` (the code checks' reasons; empty means it went to the judges),
  `judges.<name>` (`verdict`, `passed`), `published_if.<name>` (whether the
  line would publish with that judge), and the writer and judge versions.
- `out/posts.jsonl`: per post, how many movers it lists and how many were
  eligible.
- `out/spend.jsonl`: one line per model call.
- `out/seeded.jsonl`: one object per wrong variant or control: `post`,
  `ticker`, `kind` (`control` is the original line sent again), the
  `insight`, its `check` result, each judge's answer when the checks let it
  through, `original_passed` per judge, and the original line's `line_key`.
  Each row records what it changed (`change`: "$28.00 ↔ $7.54", "raised →
  cut"). A number seed exchanges or replaces like with like (money with
  money, a per-share figure with a per-share figure, a percentage with a
  percentage), never a year, a date, a label or a range's end, and changes
  the Chinese line to match; when no such change exists it is skipped and
  counted.
  Each judge call holds at most one row per mover, under the mover's own
  ticker, and each mover's control goes in a different call from its
  variants. A wrong line counts against a judge only when that judge passed
  the original; a failed call or an omitted mover counts neither way.
- `out/label.html`: about 60 lines in random order (fixed seed): every line a
  judge said no to or the judges disagreed on, then judge-passed lines at
  random. `--cards N` changes the number (the page warns when the lines it
  must show are more). Each card shows the whole pack, the items the line
  cites marked, and a company figure's source text. Choices are kept in the
  browser's storage, keyed by the line's text, so a re-run's new lines start
  blank; "Save labels" downloads `labels.json`
  (`[{"post", "ticker", "key", "label"}]`, label `fine`, `wrong_fact` or
  `advice_or_informal`).
- `out/report.md` (from `score`): each judge's agreement with Rick's labels
  (a judge pass counts as "Fine"), the judge-passed lines Rick marked wrong in
  full, seeded recall overall and by kind (with the kinds that were skipped
  because every change would still be true), the flip rate on controls of
  lines the judge had passed and its consistent rejections apart, how many
  results were left out (failed calls, omitted movers, labels or seeds for
  older text), what the code checks refused, eligibility, spend, the judge
  to choose (higher agreement, ties broken by seeded recall), how many
  judge-passed lines were labelled, whether the trial's part of the bar
  holds (none marked wrong, seeded recall at least 90%), and the limits of
  the trial. The bar's 50 labelled judge-passed lines cannot be reached
  with this trial's 34 eligible movers (final run): the report calls the bar "not yet
  assessable" until they are, and they accumulate through the shadow stage.

## The shadow stage's review

While the Worker runs with `INSIGHTS_MODE = "shadow"`, it stores one record
per facts bundle in R2: `insights/premarket/<date>.json` (the 8:00 brief),
`insights/premarket/<date>-update.json` (the 8:35 update) and
`insights/recap/<date>.json`. No model call is made here.

```sh
# 1. Save every record from the first shadow day to today (read-only R2 gets
#    of the three fixed keys per weekday; a done record already saved is
#    kept, a missing key is skipped).
tools/insight-trial/fetch-shadow.sh 2026-11-18 "$(date +%F)" tools/insight-trial/shadow

# 2. Every record as lines.jsonl, and a cumulative labelling page.
cd tools/insight-trial
env -u CARGO_TARGET_DIR cargo run --release -- shadow \
    --records shadow --out out-shadow --labels ~/Downloads/labels.json

# 3. The bar, from every label so far.
env -u CARGO_TARGET_DIR cargo run --release -- score \
    --out out-shadow --labels ~/Downloads/labels.json
```

`shadow` shows every line the judge answered for, passed or refused (no
sampling), verdicts hidden, new ones first in a fixed random order; lines
labelled before (matched by `line_key`, the writer version and the text)
are shown at the end, done, with their label, and "Save labels" saves them
again with the new ones, so one `labels.json` always holds every label.
Leave `--labels` off the first time. Lines the code checks refused and
lines the judge has not answered yet (a record still `written`, or a
failed judge call) are left out. The 8:00 brief and its 8:35 update often
carry the same mover: the page shows both. A line the Worker's rewrite
wrote is marked "rewritten", with the writer's refused line and its check
failures folded under it; it is labelled like any other line (it is what
readers would get), and `score` also counts rewritten lines apart.

`score` on the shadow folder reports the reading part of the bar: the
judge-passed lines read, counting a mover once per trading date across the
brief and its update, and any marked wrong. Below 50 read it says "not yet
assessable". The bar's seeded-recall part comes from the trial's seed run
and its own report.

### Replaying the rewrite on stored records (spends money)

The Worker gives a line the code checks refuse one rewrite call
(`insight-rewrite/v1`, the writer's own rules plus the refused line and the
exact check failures). `rewrite` replays it on saved records, for lines the
checks refused before the Worker had it: per record with a line the current
checks refuse, one rewrite call (Claude, the Worker's writer) over those
lines only, the code checks on what it returns, then one Claude judge call
over the lines that now pass. It writes `OUT/rewrite.jsonl` (per refused
line: the writer's line and its refusals, the rewrite's line, its refusals,
the verdict, passed) and skips records already there. Only
`ANTHROPIC_API_KEY` is needed. It shares `run`'s spend ledger and cap: a
new `--out` starts its own ledger.

```sh
# The calls and their cost, no key read and no call made:
env -u CARGO_TARGET_DIR cargo run --release -- rewrite \
    --records shadow --out out-rewrite --plan
# The replay (Rick's go first: it pays for real calls):
env -u CARGO_TARGET_DIR cargo run --release -- rewrite \
    --records shadow --out out-rewrite \
    --dev-vars ../../crates/generator/.dev.vars --budget 1.0
```

The time command does not run the rewrite: a record whose checks refused a
line shows "waits for the rewrite (not timed)" in its judge cell.

## CPU timing

The Worker allows 10 ms of CPU per invocation. `time` measures the insight
step's own work natively, with no model call: for each stored brief and
recap in `--facts`, it runs the real `insight::step::insight_tick` over an
in-memory store with the stored replies from `--lines` (the final run's
lines as the writer's, Claude's verdicts as the judge's): the write stage
(reading the facts and earnings checks, the packs, the code checks, the
record) and the judge stage (the judge-support confirmation, the record),
each first in a fresh process (every regular expression compiled on first
use, as in a cold isolate) and then warm (the median of 20 runs). It prints
a table per post, then the worst and the median of each column. Native
time understates Wasm.

```sh
env -u CARGO_TARGET_DIR cargo run --release -- time --facts facts --lines out/lines.jsonl
```

`--dump DIR` also writes each step's starting snapshot for
`tools/insight-cpu-wasm`, which times the same steps in V8.
