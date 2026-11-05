# Data-Graphic Covers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every post's cover background becomes an SVG chart of the story's own verified figures (a brief's biggest movers, an earnings check's recent revenue), written to R2 at publish time, and the Flux/Workers AI scene path is removed.

**Architecture:** A new pure renderer `crates/core/src/cover/graphic.rs` turns facts into SVG text by string formatting (no plotters). The earnings cover reuses the earnings chart's bar series through a builder extracted from `earnings/charts.rs`. Both publish paths write `covers/<slug>.svg` through one helper that also yields the `cover=ok|err|none` log line. The web route serves `svg` under a sandbox CSP, and the cover CSS switches to ink on paper with the graphic `contain`ed.

**Tech Stack:** Rust (crates/core, crates/generator on workers-rs), Next.js 16 (apps/web), Vitest, cargo test, bash + wrangler for the backfill.

**Spec:** `docs/superpowers/specs/2026-11-05-data-covers-design.md` (binding; read it before any task).

**Branch:** `feat/data-covers` in `.worktrees/covers`. Commit per task. Do not merge, deploy, push to main, apply SQL or touch remote R2/D1.

## Global Constraints

- Free plan only. No new bindings, secrets or dependencies. The SVG is built with `format!`/`write!`, never plotters.
- SVG root, exact: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Inter, system-ui, sans-serif">`. Its first child, exact: `<rect x="0" y="0" width="640" height="360" fill="#faf8f3"/>`. It ends with `</svg>`.
- Every other rect is written exactly as `<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="{colour}"/>`, and every text as `<text x="{x:.1}" y="{y:.1}" …>{escaped}</text>`. The tests parse these shapes.
- Chart area: every bar rect and every text lies in `y ≥ 151.2` (the lower 58% of 360) and `y + height ≤ 360`; every rect in `24 ≤ x` and `x + width ≤ 616`.
- Colours, exact: paper `#faf8f3`, ink `#1a1a1a`, up `#15803d`, down `#b91c1c`, grey bars `#d9d5cc`, captions `#666`. Value and ticker labels are ink, so up and down appear only on bars. No gradients, no opacity, no filters.
- Numbers only through `fmt_signed_pct`, `fmt_short_usd` and `fmt_eps` in `crate::cover`. Negative values carry U+2212.
- All text goes through one XML escape helper (`&`, `<`, `>`, `"`, `'`).
- Captions, exact: `BIGGEST PREMARKET MOVES`, `BIGGEST MOVES AT THE CLOSE`, `REVENUE, LAST {n} QUARTERS`, `DILUTED EPS, LAST {n} QUARTERS`. They are grey, uppercase and letter-spaced.
- R2 key `covers/{slug}.svg`, content type `image/svg+xml`. The web key pattern is `^covers\/[a-z0-9-]{1,80}\.(png|jpg|svg)$`; old png/jpg keys stay valid.
- Log line `cover=ok|err|none`, its own line after the cover write, for premarket, recap and earnings builds. It is never in `runs.last_error` or the stored facts.
- Tests: `cargo test -p prebell-core --locked` (538 now), `cargo test --workspace --locked` (545 now), `cargo check -p prebell-generator --target wasm32-unknown-unknown --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --all --check`. Web, from `apps/web`: `pnpm test` (309 now), `pnpm typecheck`, `pnpm lint`, `pnpm cf:build`.
- Deploy (owner, after merge, not in this plan): **web first, then generator** (the web must accept `.svg` keys before the generator writes any; the old generator's png/jpg keys stay served). There is no migration. The owner applies `backfill-graphics.sql`.
- Commit trailers: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_01C33suxkNuxRbEfR6iUqCky`.

## Review Focus

1. A four-digit move or a five-letter ticker must keep its label inside the 24 px margins. Pinned in Task 2 by `brief_graphic_keeps_long_labels_inside_the_margins`.
2. When the last 8 slots of a gappy series start on an empty slot, the cover must start on a bar with a `YYYY-MM` label, never on a gap or `8-K`. Pinned in Task 3 by `earnings_graphic_keeps_the_last_eight_slots_and_starts_on_a_bar`.
3. A loss-making company's EPS fallback must hang its bars below a zero baseline and label `−$0.25` that passes the number check. Pinned in Task 3 by `earnings_graphic_draws_negative_eps_below_the_baseline`.
4. A flat series (all values zero) must render without `NaN` or `inf` in the SVG. Pinned in Task 3 by `earnings_graphic_with_an_all_zero_series_has_no_nan`.
5. Posts that keep an old `.png`/`.jpg` key must still be served and drawn, with no shade and no dimming. Pinned in Task 6 by the existing png/jpg route test and `renders an old scene key with the same paper styling`.

## File Structure

- `crates/core/src/earnings/charts.rs`: gains the shared `series()` builder; `charts()` draws from it.
- `crates/core/src/cover.rs` moves to `crates/core/src/cover/mod.rs`: lines, snapshot, formatting, `graphic_for_facts`, `store_graphic`. The Flux code leaves.
- `crates/core/src/cover/graphic.rs` (new): `CoverMover`, `Session`, `cover_movers`, `brief_graphic`, `earnings_graphic`.
- `crates/core/src/run.rs`, `crates/core/src/recap/mod.rs`, `crates/core/src/earnings/run.rs`: the publish paths write the SVG.
- `crates/core/src/store.rs`: `MemStore` gains `fail_put_bytes`.
- `crates/core/src/bin/cover-graphic.rs` (new) and `scripts/backfill-graphics.sh` (new). `scripts/backfill-scenes.sh` and `crates/generator/src/cover.rs` are deleted.
- `apps/web/src/lib/covers.ts`, `apps/web/src/components/Cover.tsx`, `apps/web/src/app/globals.css`: serve and show the SVG.
- `docs/runbook.md`, `docs/decisions.md`, `docs/declined.md`.

---

### Task 1: Shared earnings bar series

**Files:**
- Modify: `crates/core/src/earnings/charts.rs` (`Bar`, `bars`, `release_value`, `charts`, tests)

**Interfaces:**
- Produces, all `pub(crate)` in `crate::earnings::charts`:
  - `struct Bar { label: String, value: f64 }` with `pub(crate)` fields. The label is the period end as `YYYY-MM`, or `8-K` for the release bar.
  - `struct Series { name: &'static str, slots: Vec<Option<Bar>>, release: Option<f64> }` with `pub(crate)` fields. `name` is `"revenue"` or `"eps"`. `slots` is in time order, `None` is a missing quarter, and the first and last slots hold bars when the vector is non-empty. `release` is this 8-K's figure when it is the last slot.
  - `fn series(f: &EarningsFacts) -> Vec<Series>`: revenue then EPS, built with today's `bars` and `release_value` (`Metric::Revenue` on `trend.revenue`, `Metric::EpsGaap` on `trend.eps_diluted`). It is empty when `f.trend` is `Missing`.
  - `const MIN_BARS: usize = 2`.

- [ ] **Step 1: Write the failing test** `series_lists_revenue_then_eps_with_the_release_last` in the charts tests:

```rust
let s = series(&facts());
assert_eq!(s.iter().map(|s| s.name).collect::<Vec<_>>(), ["revenue", "eps"]);
assert_eq!(s[0].slots.len(), 3);
assert_eq!(s[0].slots[0].as_ref().unwrap().label, "2026-03");
assert_eq!(s[0].slots[2].as_ref().unwrap().label, "8-K");
assert_eq!(s[0].release, Some(102.5e9));
assert_eq!(s[1].release, Some(1.85));
let mut f = facts();
f.trend = Field::Missing { reason: "x".into() };
assert!(series(&f).is_empty());
```

- [ ] **Step 2: Run it to verify it fails.** Run `cargo test -p prebell-core --locked series_lists`. Expected: a compile error, `cannot find function series`.
- [ ] **Step 3: Implement `series` and rewrite `charts` to iterate it.** The labels, the number check, the alt texts (chosen by `name`) and the drawing stay exactly as they are. This is a refactor with no behaviour change.
- [ ] **Step 4: Run the tests.** Run `cargo test -p prebell-core --locked earnings::charts`, then the full core suite. Expected: all 12 existing chart tests pass unchanged, and core shows 539 passed.
- [ ] **Step 5: Commit** `refactor(core): share the earnings bar series`.

---

### Task 2: Brief cover graphic

**Files:**
- Move: `crates/core/src/cover.rs` to `crates/core/src/cover/mod.rs` with `git mv`. Add `pub mod graphic;` and `pub use graphic::{brief_graphic, cover_movers, CoverMover, Session};`.
- Create: `crates/core/src/cover/graphic.rs` with its tests.

**Interfaces:**
- Consumes: `fmt_signed_pct` from `crate::cover`, and `crate::facts::Mover`.
- Produces:
  - `#[derive(Debug, Clone, PartialEq)] pub struct CoverMover { pub symbol: String, pub change_pct: f64 }`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Session { Premarket, Recap }`
  - `pub fn cover_movers(gainers: &[Mover], losers: &[Mover]) -> Vec<CoverMover>`: gainers then losers, in list order, taking `quote.symbol` and `quote.change_pct`.
  - `pub fn brief_graphic(movers: &[CoverMover], session: Session) -> Option<String>`. Positive moves are sorted descending and negative moves ascending, with at most 4 of each. Moves that are non-finite or round to `0.00%` are skipped. It returns `None` when no move is left.

Layout follows the spec's "Brief cover" paragraph. Rows run top to bottom, gainers first, inside the chart area. A vertical axis line sits at x = 320. Bars start at the axis, green to the right and red to the left, and the largest |move| gets one fixed length. The ticker sits beside the axis on the side opposite the bar, and the `fmt_signed_pct` label sits at the bar's far end. The caption is bottom-left. The test helpers `rects(svg) -> Vec<(f64, f64, f64, f64, String)>` and `texts(svg) -> Vec<(f64, f64, String)>` parse the exact shapes from Global Constraints with `regex`, and `m(sym, pct)` builds a `CoverMover`.

- [ ] **Step 1: Write the failing tests** in `graphic.rs`:

```rust
#[test] fn brief_graphic_has_the_root_background_and_session_caption() {
    let svg = brief_graphic(&[m("MOS", 20.49), m("FIS", -11.77)], Session::Premarket).unwrap();
    assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360" width="640" height="360" font-family="Inter, system-ui, sans-serif">"#));
    assert!(svg.contains(r##"<rect x="0" y="0" width="640" height="360" fill="#faf8f3"/>"##) && svg.ends_with("</svg>"));
    assert!(svg.contains(">BIGGEST PREMARKET MOVES<") && svg.contains(">+20.49%<") && svg.contains(">−11.77%<"));
    for gone in ["gradient", "opacity", "filter"] { assert!(!svg.contains(gone)); }
    let recap = brief_graphic(&[m("MOS", 1.0)], Session::Recap).unwrap();
    assert!(recap.contains(">BIGGEST MOVES AT THE CLOSE<"));
}
#[test] fn brief_graphic_orders_and_colours_movers() // input B +3, L1 −2, A +9, L2 −7.5:
    // bars by ascending y are A, B, L2, L1; fills up, up, down, down;
    // up rects have x == 320.0, down rects have x + width == 320.0 (±0.1);
    // the value labels appear in the order "+9.00%", "+3.00%", "−7.50%", "−2.00%".
#[test] fn brief_graphic_caps_four_per_side() // 6 gainers G1..G6 (+6..+1) and 6 losers:
    // 4 up rects and 4 down rects; ">G5<", ">G6<" and the two smallest losers are absent.
#[test] fn brief_graphic_skips_zero_and_non_finite_moves() // ZERO 0.0, TINY −0.001, NANX NaN, INFX inf, KEEP 1.0:
    // one bar rect; none of ">ZERO<", ">TINY<", ">NANX<", ">INFX<"; "NaN" and "inf" absent;
    // brief_graphic(&[m("ZERO", 0.0)], _) and brief_graphic(&[], _) are None.
#[test] fn brief_graphic_scales_bars_inside_the_chart_area() // BIG +20, HALF +10, LOSS −20:
    // width(BIG) == width(LOSS) (±0.1); width(HALF) == width(BIG) / 2 (±0.2);
    // every non-paper rect is in 24 ≤ x, x + w ≤ 616, y ≥ 151.2, y + h ≤ 360; every text has y ≥ 151.2.
#[test] fn brief_graphic_keeps_long_labels_inside_the_margins() // GOOGL +1234.56, BRKBX −987.65:
    // the "+1234.56%" anchor x ≤ 526.0 (616 − 90: nine characters at 16 px);
    // the "−987.65%" anchor x ≥ 114.0, where the loser label is end-anchored.
#[test] fn brief_graphic_escapes_text() // m("A&B<", 2.0): contains "A&amp;B&lt;", not "A&B<".
#[test] fn cover_movers_takes_gainers_then_losers() // gainers [MOS +17.37], losers [FIS −23.67]
    // gives [CoverMover{MOS, 17.37}, CoverMover{FIS, −23.67}].
```

- [ ] **Step 2: Run them to verify they fail.** Run `cargo test -p prebell-core --locked cover::graphic`. Expected: a compile error for the missing items.
- [ ] **Step 3: Implement `cover_movers`, `brief_graphic` and the escape helper in `graphic.rs`.** Geometry is the builder's call within Global Constraints. Use the mockup at direction A as the reference.
- [ ] **Step 4: Run the tests.** Run `cargo test -p prebell-core --locked`. Expected: core shows 547 passed. Then run clippy and fmt from Global Constraints.
- [ ] **Step 5: Commit** `feat(core): brief cover graphic`.

---

### Task 3: Earnings cover graphic and `graphic_for_facts`

**Files:**
- Modify: `crates/core/src/cover/graphic.rs` and `crates/core/src/cover/mod.rs` (add `earnings_graphic` to the `pub use`; add `graphic_for_facts` and its tests).

**Interfaces:**
- Consumes: `series`, `Series`, `Bar` and `MIN_BARS` from Task 1; `numbers_in_facts` and `crate::numbers::check`; `fmt_short_usd` and `fmt_eps`.
- Produces:
  - `pub fn earnings_graphic(f: &EarningsFacts) -> Option<String>`, which calls the private `fn earnings_graphic_checked(f: &EarningsFacts, allowed: &[f64]) -> Option<String>` with `&numbers_in_facts(f)`.
  - `pub fn graphic_for_facts(content_type: &str, facts_json: &str) -> Result<Option<String>, String>` in `cover/mod.rs`. It takes the same content types and errors as `lines_for_facts`. `premarket` gives `brief_graphic(&cover_movers(&f.gainers, &f.losers), Session::Premarket)`, `recap` gives the same with `Session::Recap`, and `earnings` gives `earnings_graphic`.

Algorithm for `earnings_graphic_checked`:
1. Pick the revenue series if it has at least `MIN_BARS` bars, else the EPS series under the same rule, else return `None`.
2. Keep the last 8 slots, then drop leading `None`s. Return `None` if fewer than `MIN_BARS` bars remain.
3. The last slot is the highlight: an ink bar whose value label is `fmt_short_usd` for revenue or `fmt_eps` for EPS. This covers both a release bar and, when `release` is `None`, the latest quarter.
4. If `check(label, Lang::En, allowed)` fails, return `None`.
5. Every other bar is grey. The first slot's `Bar.label` is drawn below it, and `n = slots.len()` goes in the caption.
6. Bars rise from or hang below one zero baseline, and a zero span must not divide by zero.

- [ ] **Step 1: Write the failing tests.** They use `crate::earnings::facts::tests::{facts, fig}`, a local `qp(end, value)` helper and the Task 2 parsers.

```rust
#[test] fn earnings_graphic_draws_revenue_with_the_release_in_ink() {
    let svg = earnings_graphic(&facts()).unwrap();
    // 2026-03 and 2026-06 grey, the release in ink, rightmost.
    assert_eq!(rects(&svg).iter().filter(|r| r.4 == "#d9d5cc").count(), 2);
    assert_eq!(rects(&svg).iter().filter(|r| r.4 == "#1a1a1a").count(), 1);
    assert!(svg.contains(">$102.5B<") && svg.contains(">2026-03<") && svg.contains(">REVENUE, LAST 3 QUARTERS<"));
    assert!(!svg.contains("8-K") && !svg.contains(">2026-06<"));
}
#[test] fn earnings_graphic_falls_back_to_diluted_eps() // revenue Missing and trend.revenue truncated to 1:
    // contains ">DILUTED EPS, LAST 3 QUARTERS<" and ">$1.85<".
#[test] fn earnings_graphic_none_when_both_series_are_short()
    // trend = Field::Value(Trend::default()) gives None; trend = Missing gives None.
#[test] fn earnings_graphic_none_when_the_label_fails_the_check()
    // earnings_graphic_checked(&facts(), &[]) is None; with &numbers_in_facts(&facts()) it is Some.
#[test] fn earnings_graphic_keeps_the_last_eight_slots_and_starts_on_a_bar() {
    // revenue at 2024-09-28, then 2025-03-29, 2025-06-28, 2025-09-27, 2025-12-27, 2026-03-28, 2026-06-27
    // (about 100e9 each), plus the release: 9 slots with a gap second. The last 8 start on the gap.
    // Expect 7 bar rects, ">REVENUE, LAST 7 QUARTERS<", ">2025-03<", and no ">2024-09<".
}
#[test] fn earnings_graphic_draws_negative_eps_below_the_baseline() {
    // revenue Missing, trend.revenue cleared, eps_diluted [−0.40, −0.31],
    // eps_gaap = fig(−0.25, UsdPerShare, "$(0.25)", "diluted loss per share of $(0.25)").
    // Expect 3 bar rects sharing one y (the baseline), ">−$0.25<", and no "NaN".
}
#[test] fn earnings_graphic_with_an_all_zero_series_has_no_nan()
    // revenue Missing and cleared; eps_diluted [0.0, 0.0]; eps_gaap 0.0.
    // Expect Some, no "NaN", no "inf", and ">$0.00<".
// in cover/mod.rs tests:
#[test] fn graphic_for_facts_matches_the_renderers()
    // premarket_json() gives Ok(brief_graphic(&cover_movers(gainers, losers), Session::Premarket));
    // the earnings JSON of facts() gives Ok(earnings_graphic(&facts())).
#[test] fn graphic_for_facts_rejects_unknown_kinds_and_bad_json()
    // the errors of "weekly" and of "recap" with "not json" name the kind; "earnings" with premarket_json() is Err.
```

- [ ] **Step 2: Run them to verify they fail.** Run `cargo test -p prebell-core --locked graphic`. Expected: a compile error for the missing items.
- [ ] **Step 3: Implement `earnings_graphic`, `earnings_graphic_checked` and `graphic_for_facts`.**
- [ ] **Step 4: Run the tests.** Run `cargo test -p prebell-core --locked`. Expected: core shows 556 passed. Then run clippy and fmt.
- [ ] **Step 5: Commit** `feat(core): earnings cover graphic`.

---

### Task 4: `cover-graphic` binary and the graphics backfill

**Files:**
- Create: `crates/core/src/bin/cover-graphic.rs` and `scripts/backfill-graphics.sh` (executable).
- Modify: `.gitignore` (add `backfill-graphics/` and `backfill-graphics.sql`).

**Interfaces:**
- Consumes: `graphic_for_facts` from Task 3.
- Produces: `cover-graphic <content_type> <facts.json>`. It exits 0 and prints the SVG on success. It exits 3 with `no graphic for these facts` on stderr when the result is `None`, 1 with the error on stderr for a read or parse error, and 2 with `usage: cover-graphic <content_type> <facts.json>` on bad arguments. It follows the shape of `bin/cover-lines.rs`.

The script follows the shape of `scripts/backfill-covers.sh`: `set -euo pipefail`, it needs `jq`, `BUCKET=prebell-facts`, wrangler runs from `crates/generator`, and a temp dir is removed on exit.
- It selects `SELECT slug, content_type, facts_r2_key FROM posts WHERE published_at IS NOT NULL AND facts_r2_key IS NOT NULL AND (cover_key IS NULL OR cover_key NOT LIKE '%.svg')` from remote D1.
- It skips a slug that does not match `^[a-z0-9-]{1,80}$`. Otherwise it downloads the facts and renders to `backfill-graphics/<slug>.svg`.
- On exit 3 it logs `flat <slug>: no graphic (cover_key -> NULL)` and appends `UPDATE posts SET cover_key = NULL WHERE slug = '<slug>';` to `backfill-graphics.sql` (the spec: an old scene must not show under the paper styling). On any other failure it logs the reason and leaves the row alone.
- It uploads with `wrangler r2 object put "$BUCKET/covers/<slug>.svg" --file … --content-type image/svg+xml --remote`, checking for `Upload complete`.
- It appends `UPDATE posts SET cover_key = 'covers/<slug>.svg' WHERE slug = '<slug>';` to `backfill-graphics.sql`.
- It ends with the counts and the apply command `cd crates/generator && pnpm exec wrangler d1 execute prebell --remote --file ../../backfill-graphics.sql`. It never runs the SQL.

- [ ] **Step 1: Write the binary.**
- [ ] **Step 2: Smoke it against the preview facts.**

```bash
cargo build -q -p prebell-core --bin cover-graphic
./target/debug/cover-graphic premarket db/preview-facts/premarket-2026-10-27.json | head -c 40   # <svg xmlns="http://www.w3.org/2000/svg"
./target/debug/cover-graphic premarket db/preview-facts/premarket-2026-10-27.json | grep -o 'fill="#15803d"' | wc -l   # 2 (AAPL, AMD)
./target/debug/cover-graphic weekly db/preview-facts/premarket-2026-10-27.json; echo $?   # unknown content type: weekly / 1
./target/debug/cover-graphic; echo $?   # usage line / 2
```

- [ ] **Step 3: Write the script and check it.** Run `bash -n scripts/backfill-graphics.sh`, plus `shellcheck scripts/backfill-graphics.sh` if installed. Expected: no output.
- [ ] **Step 4: Run the suites.** Run `cargo test -p prebell-core --locked` and clippy. Expected: still 556 passed. Do not run the script against remote.
- [ ] **Step 5: Commit** `feat: cover-graphic binary and graphics backfill script`.

---

### Task 5: Publish paths write the SVG; the Flux path goes

**Files:**
- Modify: `crates/core/src/cover/mod.rs`. Add `store_graphic`. Remove `CoverMaker`, `NoCover`, `make_scene`, `image_ext`, `scene_prompt`, `scene_prompt_for_facts`, `BRIEF_SCENES`, `EARNINGS_SCENES`, `fnv1a`, `brief_scene_subject`, `earnings_scene_subject`, the `FakeCover` and `PNG` test helpers and their 11 tests. Update the module doc.
- Modify: `crates/core/src/store.rs`. Add `pub fail_put_bytes: Cell<bool>` to `MemStore`; when it is set, `put_bytes` returns `Err("r2: put_bytes failed")` and stores nothing.
- Modify: `crates/core/src/run.rs`. Change `Content`, `Premarket`, `Hooks`, `publish`, `tick_with_log`, the docs, `BudgetProbe` and the tests.
- Modify: `crates/core/src/recap/mod.rs`. Change the `Recap` impl, `tick_with_log` and the tests.
- Modify: `crates/core/src/earnings/run.rs`. Change `build_tick`, `build`, the imports and the tests (about 40 call sites).
- Modify: `crates/core/src/bin/cover-lines.rs`. Drop `--prompt` and the doc lines about it.
- Modify: `crates/generator/src/lib.rs` and `crates/generator/Cargo.toml` (drop `base64`), and refresh `Cargo.lock`.
- Modify: `crates/generator/wrangler.toml`. Change only the comment above `CF_ACCOUNT_ID` to `# Unused since covers became data graphics (2026-11-05); kept until the owner removes it.`
- Delete: `crates/generator/src/cover.rs` and `scripts/backfill-scenes.sh`. Delete the `backfill-scenes/` and `backfill-scenes.sql` lines in `.gitignore`.

**Interfaces:**
- Consumes: `brief_graphic`, `cover_movers`, `Session` and `earnings_graphic`.
- Produces:
  - `pub async fn store_graphic(store: &dyn Store, slug: &str, svg: Option<String>) -> (Option<String>, &'static str)` in `crate::cover`. `Some(svg)` is written with `put_bytes("covers/{slug}.svg", svg.as_bytes(), "image/svg+xml")`, which gives `(Some(key), "ok")` or, on a failed write, `(None, "err")`. `None` gives `(None, "none")`. It never fails the build.
  - `Content::cover_graphic(&self, f: &Self::Facts) -> Option<String>` replaces `scene_subject`. `Premarket` gives `brief_graphic(&cover_movers(&f.gainers, &f.losers), Session::Premarket)`, `Recap` gives the same with `Session::Recap`, and `BudgetProbe` gives `None`.
  - `Hooks { log, extra, budget }`: the `cover` field is gone.
  - `pub async fn build_tick(now: DateTime<Utc>, es: &dyn EarningsStore, store: &dyn Store, filings: &dyn Filings, llm: &dyn LlmProvider, log: &dyn Fn(&str)) -> Result<Vec<(String, TickOutcome)>, String>`. `log` replaces `cover`, is passed to `build`, and receives `cover=…` once per build that publishes.
  - `publish` in run.rs writes `store_graphic(store, &slug, c.cover_graphic(facts))` where `make_scene` was, then logs `cover={outcome}`. The earnings `build` does the same with `earnings_graphic(&facts)` after the slug is final.
  - Generator: delete `mod cover`, `LoggedCover`, `Runner::cover`, the cover imports and `cover:` in `Hooks`. The earnings build passes `&|m: &str| console_log!("{m}")` as `log`.

- [ ] **Step 1: Write the failing tests.**

```rust
// cover/mod.rs
#[tokio::test] async fn store_graphic_writes_the_svg_and_reports_the_outcome() {
    let s = MemStore::default();
    assert_eq!(store_graphic(&s, "recap-2026-11-04", Some("<svg/>".into())).await,
               (Some("covers/recap-2026-11-04.svg".to_string()), "ok"));
    assert_eq!(s.bytes.borrow()["covers/recap-2026-11-04.svg"], (b"<svg/>".to_vec(), "image/svg+xml".to_string()));
    assert_eq!(store_graphic(&s, "p", None).await, (None, "none"));
    s.fail_put_bytes.set(true);
    assert_eq!(store_graphic(&s, "q", Some("<svg/>".into())).await, (None, "err"));
    assert_eq!(s.bytes.borrow().len(), 1);
}
```

Rewrite these run.rs tests:
- `build_stores_cover_lines_scene_and_index_snapshot` becomes `build_stores_cover_lines_graphic_and_index_snapshot`. Assert key `covers/premarket-2026-11-04.svg`, content type `image/svg+xml`, bytes that start with `<svg` and contain `>AAPL<` and `>+8.00%<`, and `logs[1] == "cover=ok"`.
- `build_publishes_without_scene_when_maker_fails` becomes `build_publishes_without_cover_when_the_write_fails`. It is one case with `FailStore.fail_put_bytes` set. Assert `Published`, `cover_key == None`, empty bytes, lines and `index_line` present, and `logs[1] == "cover=err"`.
- In `build_logs_diagnostics_once` and in `diagnostics_are_logged_before_the_drafts_and_cover_after_the_scene` (rename its suffix to `_after_the_write`), the line `cover=skip` becomes `cover=ok`.
- In `premarket_cover_and_snapshot_come_from_the_futures`, the scene assertion becomes `assert!(c.cover_graphic(&f).unwrap().contains(">AAPL<"))`.

In recap: `build_logs_diagnostics_once` changes `cover=skip` to `cover=ok`. The Content test near line 2403 asserts that `cover_graphic` contains `>FIS<` and `>BIGGEST MOVES AT THE CLOSE<`.

In earnings:
- Every `&NoCover` becomes `&|_| {}`.
- `earnings_build_writes_cover_key_and_lines` captures logs. Assert key `covers/aapl-q4-fy2026-earnings.svg`, bytes equal to `earnings_graphic(&stored_facts(&base)).unwrap().into_bytes()` with `image/svg+xml`, and logs `["cover=ok"]`.
- `earnings_build_publishes_when_scene_fails` becomes `earnings_build_publishes_when_the_cover_write_fails`, with `base.fail_put_bytes.set(true)`. Assert `Published`, `cover_key == None`, empty bytes, lines present, and logs `["cover=err"]`.

- [ ] **Step 2: Run them to verify they fail.** Run `cargo test -p prebell-core --locked`. Expected: compile errors on `store_graphic`, `cover_graphic` and the `build_tick` arity.
- [ ] **Step 3: Implement the core changes and the removals listed under Files.**
- [ ] **Step 4: Update the generator.** Drop `base64` from `Cargo.toml`, then run `cargo check -p prebell-generator --target wasm32-unknown-unknown` once without `--locked` so `Cargo.lock` drops `base64` from the generator.
- [ ] **Step 5: Verify.**

```bash
cargo test -p prebell-core --locked            # 546 passed (556 − 11 + 1)
cargo test --workspace --locked                # 549 passed (generator keeps its 3 alert tests)
cargo check -p prebell-generator --target wasm32-unknown-unknown --locked
cargo clippy --workspace --all-targets --locked -- -D warnings && cargo fmt --all --check
git grep -nE "CoverMaker|NoCover|WorkersAiCover|LoggedCover|make_scene|scene_prompt|scene_subject|image_ext|CF_AI_TOKEN|backfill-scenes|base64" -- crates scripts .gitignore   # no output
```

- [ ] **Step 6: Commit** `feat: publish data-graphic covers; remove the Workers AI scene path`.

---

### Task 6: Web serves and shows the SVG

**Files:**
- Modify: `apps/web/src/lib/covers.ts`, `apps/web/src/components/Cover.tsx` (remove the `cover-shade` div and reword the docs from "scene" to "graphic"), `apps/web/src/app/globals.css` (the cover block at lines 92–112, plus the comments at 286–287), and the "scene" wording in the comments of `BriefView.tsx`, `HomeView.tsx` and `PostList.tsx`.
- Test: `apps/web/tests/covers.test.ts`, `cover.test.tsx`, `layout.test.tsx` and `posts.test.ts`.

**Interfaces:**
- Produces: `COVER_KEY = /^covers\/[a-z0-9-]{1,80}\.(png|jpg|svg)$/`. `coverResponse` maps `png` to `image/png`, `jpg` to `image/jpeg` and `svg` to `image/svg+xml`. Every cover response carries `Content-Security-Policy: sandbox`, `X-Content-Type-Options: nosniff` and the existing `Cache-Control`. The `Cover` props are unchanged.
- CSS, per the spec's "Front page, lists, article":
  - `.cover` gets `background: var(--paper); color: var(--ink)`.
  - `.cover-bg` gets `object-fit: contain; object-position: center bottom` and loses `opacity` and `filter`.
  - `.cover-in` gets `justify-content: flex-start`.
  - `.cover-tag` is in flow (no `position: absolute`, `top` or `left`) with `border: 1px solid var(--rule)` and a white background.
  - `.cover-stat` gets `color: var(--ink)`.
  - Delete every `.cover-shade` rule, `.cover-flat .cover-tag` and `.cover-flat .cover-stat`. `.cover-flat` keeps `background: var(--cover-flat); color: var(--ink)`, and the small-size tag keeps its padding and font size.
  - Rewrite the block comment to match.

- [ ] **Step 1: Write the failing tests.**

```ts
// covers.test.ts
it("serves svg as image/svg+xml in a sandbox", async () => {
  const res = await coverResponse(bucketWith({ "covers/premarket-2026-11-04.svg": "<svg/>" }), ["covers", "premarket-2026-11-04.svg"]);
  expect(res.headers.get("content-type")).toBe("image/svg+xml");
  expect(res.headers.get("content-security-policy")).toBe("sandbox");
  expect(res.headers.get("x-content-type-options")).toBe("nosniff");
  expect(res.headers.get("cache-control")).toBe("public, max-age=86400, s-maxage=86400");
});
// The png test also expects content-security-policy "sandbox".
// The rejects test drops "covers/a.svg" from the bad list, adds ["covers", "a.SVG"] and ["covers", "a.svgz"],
// and expects COVER_KEY.test("covers/a-1.svg") to be true.
// posts.test.ts: toCover keeps "covers/recap-2026-10-29.svg".
// cover.test.tsx: the main fixture key becomes "covers/aapl-q4.svg" (update the expected img and preload strings);
//   the main test expects no "cover-shade";
//   new: it("renders an old scene key with the same paper styling") checks that coverKey "covers/aapl-q4.png"
//   gives <img class="cover-bg" src="/covers/aapl-q4.png" and no "cover-shade".
// layout.test.tsx: the flat-cover test drops its shade and muted-border expectations;
//   new: it("draws covers on paper: contained graphic, tag in flow with a rule border, no shade"), where block(sel)
//   matches /^sel \{[^}]*\}/m:
//     .cover has "background: var(--paper)" and "color: var(--ink)";
//     .cover-bg has "object-fit: contain" and "object-position: center bottom", and neither "opacity" nor "filter";
//     .cover-in has "justify-content: flex-start";
//     .cover-tag has "border: 1px solid var(--rule)" and no "position: absolute";
//     .cover-stat has "color: var(--ink)"; css contains no "cover-shade".
```

- [ ] **Step 2: Run them to verify they fail.** Run `cd apps/web && pnpm test`. Expected: the new and changed cases fail.
- [ ] **Step 3: Implement the `covers.ts`, `Cover.tsx` and `globals.css` changes.**
- [ ] **Step 4: Run the suites.** Run `pnpm test`, `pnpm typecheck`, `pnpm lint` and `pnpm cf:build`. Expected: 312 tests pass and the other three are clean.
- [ ] **Step 5: Visual check (local only).** Set up the local preview as in the runbook's "Local preview" bullet. Render both preview briefs with `cover-graphic` into the scratch dir and put them to local R2 with `wrangler r2 object put prebell-facts/covers/premarket-<date>.svg --local --content-type image/svg+xml`. Then run `wrangler d1 execute prebell --local --command "UPDATE posts SET cover_key = 'covers/' || slug || '.svg' WHERE content_type = 'premarket'"` and `pnpm preview`. Using the `visual-design-qa` skill, screenshot the home lead, a list row and the article at 375 px and 1280 px. The whole chart must show, the lines must sit top-left under the tag without covering a bar, and the tag must be bordered. If the lines overlap the bars in the 160 px slot, shrink `.post-row .cover-stat` rather than moving the chart.
- [ ] **Step 6: Commit** `feat(web): serve SVG covers and draw them on paper`.

---

### Task 7: Docs

**Files:**
- Modify: `docs/runbook.md`, `docs/decisions.md` and `docs/declined.md`.
- Add: `docs/superpowers/plans/2026-11-05-data-covers.md`.

- [ ] **Step 1: Runbook.**
  - Premarket and recap formula lines: drop "and 1 cover scene". The totals become "at most 43, or 44 on the Yahoo 401-refresh path".
  - Earnings build: "about 12, at most about 14 with a Chinese retry" becomes "about 11, at most about 13".
  - Budget-guard paragraph: "the drafts, the cover scene and the Yahoo 401-refresh path" becomes "the drafts and the Yahoo 401-refresh path".
  - The diagnostics intro and the `cover=` bullet describe `cover=ok|err|none` for premarket, recap and earnings builds. `ok` means stored at `covers/<slug>.svg`. `err` means the R2 write failed and the post shows the flat tile. `none` means nothing to draw: no non-zero mover, fewer than two revenue and two EPS quarters, or a label that failed the number check.
  - Replace "## Cover images (Workers AI)" with "## Cover graphics". Say what each cover draws, that there are no credentials, and that `CF_ACCOUNT_ID` is unused. The owner may run `cd crates/generator && pnpm exec wrangler secret delete CF_AI_TOKEN`. Old `covers/*.png|jpg` objects are harmless. Keep the lines-backfill bullet. The scene-backfill bullet becomes a `scripts/backfill-graphics.sh` bullet matching Task 4, including that posts with no graphic keep their current key.
- [ ] **Step 2: Append to `docs/decisions.md`** under `## 2026-11-05`: "**Covers are data graphics, not AI scenes** (replaces the 2026-11-04 AI-generated covers entry): the generator draws the story's own figures as an SVG chart in the paper palette, a brief's biggest movers and an earnings check's recent revenue, with no Workers AI call. Rick, choosing among four mockups: "A · Data graphic"; and: "they should be more about the market instead of just the dark gray shadows". Design: `docs/superpowers/specs/2026-11-05-data-covers-design.md`."
- [ ] **Step 3: Append to `docs/declined.md`** a `## 2026-11-05` section: "**AI-generated cover scenes** (Flux Schnell on Workers AI). Rick: "they should be more about the market instead of just the dark gray shadows". Data graphics instead."
- [ ] **Step 4: Verify.** Run `git grep -nE "Workers AI|CF_AI_TOKEN|cover scene|backfill-scenes|cover=ok\|skip" docs/runbook.md`. Expected: matches only inside the new "## Cover graphics" section, where it says the scene path is gone (for example the `secret delete` line). Then run every suite from Global Constraints once more: core 546, workspace 549, web 312.
- [ ] **Step 5: Commit** `docs: data-graphic covers in the runbook, decisions and declined` with the plan file.
