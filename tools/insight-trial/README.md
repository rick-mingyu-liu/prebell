# insight-trial

The offline trial for mover insights (spec:
`docs/superpowers/specs/2026-11-13-mover-insights-design.md`, plan:
`docs/superpowers/plans/2026-11-13-mover-insights-trial.md`). It runs the
production insight code in `prebell_core::insight` over saved fact bundles:
the evidence packs, one Claude writer call per post, the code checks, and one
judge call per judge (Claude and DeepSeek) over the lines that pass the checks.
Nothing here touches the Worker or the site.

A native crate outside the workspace: it builds into its own `target/` and
has its own `Cargo.lock`. Do not point it at a shared `CARGO_TARGET_DIR`
(the commands below unset it).

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
#    [ticker, date, status]) and the macro events (read-only: D1
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
`--only SUBSTR` keeps the posts whose file name contains it (for example
`--only recap_2026-11-12`).

Re-runs keep what was paid for: `run` skips a post already complete in
`out/lines.jsonl`, and finishes one a budget stop left part-done by calling
only its missing judges; `seed` skips a post already in `out/seeded.jsonl`.
`--fresh` redoes everything. `--plan` (on `run` and `seed`) prints the calls
the step would make and their expected cost, and makes none; without it the
plan is printed first and the step refuses to start when the expected cost
would pass what is left of `--budget`. The expected cost uses the average
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
  full, seeded recall overall and by kind, the false-reject rate on
  controls, how many results were left out (failed calls, omitted movers,
  labels or seeds for older text), what the code checks refused,
  eligibility, spend, the judge to choose (higher agreement, ties broken by
  seeded recall), whether the bar is met (at least 50 judge-passed lines
  read, none marked wrong, seeded recall at least 90%), and the limits of
  the trial.
