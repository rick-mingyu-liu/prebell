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
#    checks, the earnings calendar and the macro events (read-only: D1
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
`--only recap_2026-11-12`). Each `run` starts `out/lines.jsonl` afresh; the
spend ledger is kept.

## Output

- `out/lines.jsonl`: one object per mover that had a pack: `post`, `ticker`,
  `pack` (the evidence the models saw), `insight` (or null), `write_error`,
  `check` (the code checks' reasons; empty means it went to the judges),
  `judges.<name>` (`verdict`, `passed`), `published_if.<name>` (whether the
  line would publish with that judge), and the writer and judge versions.
- `out/posts.jsonl`: per post, how many movers it lists and how many were
  eligible.
- `out/spend.jsonl`: one line per model call.
- `out/seeded.jsonl`: one object per wrong variant: `post`, `ticker`, `kind`,
  the variant `insight`, its `check` result and, when the checks let it
  through, each judge's answer (a judge "caught" it when it did not pass it).
- `out/label.html`: about 60 lines in random order (fixed seed): every line a
  judge said no to or the judges disagreed on, then judge-passed lines at
  random. `--cards N` changes the number. Choices are kept in the browser's
  storage; "Save labels" downloads `labels.json`
  (`[{"post", "ticker", "label"}]`, label `fine`, `wrong_fact` or
  `advice_or_informal`).
- `out/report.md` (from `score`): each judge's agreement with Rick's labels
  (a judge pass counts as "Fine"), the judge-passed lines Rick marked wrong in
  full, seeded recall overall and by kind, what the code checks refused,
  eligibility, spend, the judge to choose (higher agreement, ties broken by
  seeded recall) and whether the bar is met (no judge-passed line marked
  wrong, seeded recall at least 90%).
