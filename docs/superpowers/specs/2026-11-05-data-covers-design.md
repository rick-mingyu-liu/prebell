# Data-graphic covers

Date: 2026-11-05. Owner: Rick. Status: draft for approval.

## Intent and the why

Every PreBell story has a cover: two bold lines with the story's key figures over a background. Today the background is an AI scene (Flux Schnell on Workers AI), dimmed to 55%, desaturated and shaded, which reads as "dark grey shadows" and says nothing about the market. Rick, 2026-11-05: "they should be more about the market instead of just the dark gray shadows"; after seeing four mockups built from the day's real brief he chose **A, the data graphic**: the generator draws the story's own figures as a chart, in the paper palette, and that chart is the cover.

The cover becomes a small, true picture of the day: the biggest moves for a brief, the revenue trend for an earnings check. Every cover is different because the market is. No AI image, no Workers AI call, no prompt tuning, one fetch fewer per post.

## Goals

- Briefs (premarket, recap) get a cover graphic of their biggest movers, drawn from the verified facts.
- Earnings checks get a cover graphic of the company's recent revenue, drawn from the filing's XBRL trend the site already charts.
- The cover is one SVG per post, written to R2 at publish time, language-neutral (tickers and numbers only), served by the existing `/covers/` route and shown by the existing `Cover` component on the front page, the lists and the article.
- The HTML overlay keeps the two cover lines (per language), the edition tag or company logo and the amber bar; its colours switch from white-on-dark to ink-on-paper.
- Existing posts are backfilled from their stored facts, as the lines and scenes were.
- The Flux path is removed: no `CoverMaker` trait, no `WorkersAiCover`, no scene prompts, no `CF_AI_TOKEN`.

## Non-goals

- Open Graph / social share images (a later project; the SVG is a good start for it).
- Bilingual text inside the SVG; the overlay already handles language.
- Charts beyond the one graphic per story type; the earnings page keeps its existing chart set.
- Deleting the old `covers/*.jpg|png` objects from R2 (harmless; a later cleanup).

## Anti-goals

- A cover that states a figure the facts do not contain, or a rounding that differs from the article (same formatting helpers as the lines and tables).
- Illegible thumbnails: at 160 px wide the graphic must still read as a chart (bars, direction, colour); labels may be too small to read there, and that is fine because the two lines carry the message.
- A cover that fails the publish. A failed SVG write is logged and the post renders the flat tile.
- Dark covers. The graphic's background is paper (`#faf8f3`), the ink is `#1a1a1a`, up is `#15803d`, down is `#b91c1c`, grey bars are `#d9d5cc`, captions `#666`. No gradients, no shade.

## What the reader sees

**Brief cover (premarket or recap).** Paper background. The lower part is a diverging bar chart of the brief's movers: gainers (positive `change_pct`) sorted by move descending, then losers (negative) sorted by move ascending, at most 4 of each, colour by the sign of `change_pct` (a zero or non-finite move is skipped). A vertical axis line at the centre, green bars to the right for gainers, red bars to the left for losers, bar length proportional to |move| with the largest move at a fixed length; the ticker beside the axis, the signed move at the bar's end via `fmt_signed_pct` (`+20.49%`, `−11.77%`, U+2212, two decimals, the same as the lines and the table). A small caption bottom-left: `BIGGEST PREMARKET MOVES` or `BIGGEST MOVES AT THE CLOSE` (uppercase, letter-spaced, grey). In SVG units the chart occupies the lower 58% of the 640×360 viewBox and keeps 24 px clear at both sides; the upper 42% is empty paper for the HTML lines.

**Earnings cover.** Paper background. The lower part is a bar series of the company's quarterly revenue: the same series the earnings page's revenue chart draws (`earnings/charts.rs`: the XBRL trend plus this release's 8-K bar via `release_value`, empty slots for missing quarters, at most the last 8 slots including the release), oldest left, grey bars with the release quarter in ink, the release value labelled above it (`$11.3B`, via `fmt_short_usd`), the oldest slot's period end labelled below its bar as `YYYY-MM` (`QuarterPoint` carries only `period_end`). Caption: `REVENUE, LAST N QUARTERS`. The cover follows the chart's own rules: its labels must pass `check` against `numbers_in_facts`, else no cover. When the revenue series has fewer than 2 values, fall back to diluted EPS (`trend.eps_diluted` plus the release EPS, caption `DILUTED EPS, LAST N QUARTERS`, values via `fmt_eps`); when both are short, write no SVG (flat tile with the logo). The series code is shared with `charts.rs`, not copied.

**Front page, lists, article.** Unchanged layout. The `Cover` component shows the SVG as its `<img>` background with `object-fit: contain; object-position: center bottom`, no opacity, no filter: the covers have fixed heights and variable widths (the list slot is about 1.45:1, the lead about 2.5:1), and `contain` keeps the whole chart visible on paper that matches the SVG's own background, so nothing is cropped. The two lines move to the top-left, directly under the tag (`.cover-in` becomes `justify-content: flex-start` with the tag in flow above the lines), with the amber bar under them; `.cover` background becomes `--paper`, the shade is removed, the lines are ink, the tag gets a `--rule` border on every cover (it is white on paper now). The flat tile (`cover-flat`) stays for posts without a cover key and gets the same top-left lines.

## Architecture (designer's calls unless marked owner)

- **Renderer:** `crates/core/src/cover/graphic.rs` (the existing `cover.rs` becomes `cover/mod.rs` with the lines, snapshot and formatting that stay). `pub fn brief_graphic(movers: &[CoverMover], session: Session) -> String` and `pub fn earnings_graphic(f: &EarningsFacts) -> Option<String>` return SVG text on a `0 0 640 360` viewBox, `width="640" height="360"`, `xmlns` set, all text XML-escaped, numbers formatted with the existing helpers, `font-family="Inter, system-ui, sans-serif"`. `CoverMover { symbol: String, change_pct: f64 }`; `Session { Premarket, Recap }`. Non-finite moves are skipped; zero movers returns `None` (flat tile).
- **Storage:** R2 key `covers/<slug>.svg`, content type `image/svg+xml`, written by the same publish step that writes the lines (`run.rs` for briefs, `earnings/run.rs` for earnings). `posts.cover_key` stores it; no migration (the column exists from 0006).
- **Web:** `apps/web/src/lib/covers.ts` `COVER_KEY` accepts `svg` and maps it to `image/svg+xml`; `Content-Security-Policy: sandbox` and `X-Content-Type-Options: nosniff` on the response so an SVG can never run script (it is an `<img>` anyway). `globals.css` cover block retokened as above. `Cover.tsx` unchanged except the removed opacity/filter now living in CSS.
- **Removal:** `CoverMaker`, `NoCover`, `LoggedCover`, `WorkersAiCover` (`crates/generator/src/cover.rs`), `make_scene`, `scene_prompt*`, `*_scene_subject`, `image_ext`, the `cover=ok|skip|err` log line (replaced by `cover=ok|err|none` after the SVG write, now for earnings builds too), the `--prompt` option of `crates/core/src/bin/cover-lines.rs`, `scripts/backfill-scenes.sh`, the `CF_AI_TOKEN` / `CF_ACCOUNT_ID` handling in `generator/src/lib.rs` (the var may stay in `wrangler.toml`; the owner deletes the secret with `wrangler secret delete CF_AI_TOKEN` when convenient). Dropping the `cover` parameter changes `build_tick` and its ~40 test call sites; that is mechanical.
- **Backfill:** `crates/core/src/bin/cover-graphic.rs` (`cover-graphic <content_type> <facts.json>`) prints the SVG; `scripts/backfill-graphics.sh` (replaces `backfill-scenes.sh`) lists every published post with a facts key from remote D1, skips those whose `cover_key` already ends in `.svg`, reads each facts object from R2, renders, uploads `covers/<slug>.svg` with `wrangler r2 object put --remote --content-type image/svg+xml`, and writes `backfill-graphics.sql` (`UPDATE posts SET cover_key = 'covers/<slug>.svg' WHERE slug = '<slug>'`) for the owner to apply. Posts whose graphic is `None` keep their current key. `scripts/backfill-covers.sh` (the lines) is unchanged.
- **Budget:** one fetch fewer per brief and per earnings build. Runbook formula lines drop "1 cover scene"; totals fall by one (premarket whole invocation ≤ 43 normal / ≤ 44 on the 401 path; recap the same; earnings build "about 12" becomes "about 11").
- **CPU:** the SVG is string formatting over at most 8 bars or 8 quarters; well under a millisecond.

## Constraints

- Free plan only; no new bindings, no new secrets.
- Every number in the SVG goes through `fmt_signed_pct`, `fmt_short_usd` or `fmt_eps`; the same values appear in the article.
- Tests: core (`cargo test -p prebell-core --locked`), wasm check, clippy `-D warnings`, fmt; web `pnpm test`, `pnpm typecheck`, `pnpm lint`, `pnpm cf:build`. SVG tests assert structure (counts of `<rect>`, colours per direction, U+2212 in labels, escaping of `&`/`<`, the viewBox) and the `None` cases.
- Deploy: generator then web; no migration. Backfill SQL applied by the owner.

## Left to the builder

- Exact geometry (bar heights, gaps, font sizes) within the stated proportions; the mockup at the chosen direction is the reference.
- Whether the brief caption includes the session time; the lines already carry the date.
- How `cover/mod.rs` is split; keep public names that other modules use.
- How the bar-series code is shared between the earnings chart and the cover (a shared series builder with two renderers is the expected shape).

## Builder-check answers (2026-11-05, my calls)

- Lines sit top-left under the tag; the chart fills the lower 58% of the viewBox; the image is `contain`ed, not cropped.
- Movers are coloured by the sign of `change_pct` and sorted by size within each side; two decimals everywhere.
- The earnings cover reuses the chart's series (release bar, gaps, number check) and labels periods `YYYY-MM`.
- The backfill selects every post with facts and skips `.svg` keys; the lines backfill script stays.
- The tag gets a rule border on every cover; earnings builds gain the `cover=` log line.

## Owner's decisions (as stated)

- 2026-11-05, choosing among four mockups: "A · Data graphic".
- 2026-11-05: "they should be more about the market instead of just the dark gray shadows".
