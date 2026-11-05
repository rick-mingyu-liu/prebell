# Front-page redesign: PreBell as a daily paper

Date: 2026-11-04. Status: draft for builder check, then owner approval.

## Intent

PreBell's current look reads as "AI-made": a black-and-neon-green dark mode,
green and orange pills on every number, boxes and tiles everywhere, letter
avatars where logos are missing, and an affiliate box in the sidebar. Rick
compared it to AlphaLab (alphalab.site) and The Batch (deeplearning.ai/the-batch)
and said what makes those trustworthy is the opposite: a clean light page
without strong contrast, black-and-white design, real images, and a person
standing behind the writing.

The redesign turns PreBell into a daily paper. The site keeps every feature it
has today (briefs, earnings checks, calendar strip, archive, ticker pages,
EN/ZH, partner links) and changes how they look and read. It adds two things
AlphaLab has and PreBell lacks: a cover image per story and an author block.
It also adds a newsletter signup for a weekly digest (capture only; sending is
a later project).

Rick chose direction "B · the front page" from three mockups
(`.superpowers/brainstorm/61247-1793837634/content/visual-direction.html`,
`article-pages.html`, `author-section.html`; the server directory is
git-ignored). The mockups are the visual reference for this document.

## Goals

- A reader landing on the home page sees a newspaper front page, not a
  dashboard: masthead, date line, a quiet index line, a lead story with a
  cover, a column of movers with reasons, and the day's earnings checks.
- Every post page reads as an article: one reading column, serif headline,
  cover, body, sources, author block, signup.
- Every story has a cover image composed from PreBell's own verified facts: a
  generated scene behind, the company logo or edition tag, and a two-line key
  figure.
- A reader can tell who is behind the site and reach him.
- A reader can leave an email for a weekly digest.

## Non-goals

- Sending the weekly digest. This project only captures subscribers.
- Open Graph / social share images. The cover is an HTML composition on the
  page; a rendered PNG for `og:image` is a later project.
- New content types, new data, or changes to what the generator writes. The
  prose, figures, sources and diagnostics stay as they are.
- A design system, component library, or Tailwind migration. The app uses
  plain CSS with custom properties today; keep that.
- Dark mode. Dropped at launch (see Decisions).

## Anti-goals (what would make this a failure even if it technically works)

- It still looks like a template or a crypto dashboard: pills, neon,
  tiles, gradients on text, glassmorphism, drop shadows on cards.
- The cover images look like AI slop: faces, text in the image, logos drawn
  by the model, saturated stock-photo colours. The scene must be quiet and
  the real content (logo, figures) must come from our data, not the model.
- Trust signals become loud. The "checked against source" tag, the
  disclaimer and the author block are quiet typographic elements, not badges.
- The Chinese edition is an afterthought: wrong fonts, broken line lengths,
  English-only cover text.
- Regressions: any current page, route, redirect, hreflang pair or
  accessibility contrast requirement breaks.

## Constraints

- Free tiers only (standing rule). Workers AI free allocation is 10,000
  neurons/day; Flux Schnell costs 4.80 neurons per 512×512 tile plus 9.60 per
  step, so a 1024×576 image at 4 steps is under 60 neurons. Our volume is two
  briefs plus a handful of earnings checks per day. Buttondown's free plan
  covers the first 100 subscribers.
- Cloudflare Workers Free: the web Worker renders under the 10 ms CPU limit
  today (brief render ~8.5 ms). The redesign must not add server-side work per
  request beyond reading one extra field from facts. Covers are static files
  served from R2, never generated at request time.
- Stack stays: Next.js 16 App Router on OpenNext for Cloudflare, `next-intl`
  for EN/ZH, plain CSS in `apps/web/src/app/globals.css` with custom
  properties, `next/font` for fonts. Rust generator in `crates/generator`
  (workers-rs) and `crates/core`.
- Accessibility: keep the current contrast discipline (text 4.5:1, UI
  boundaries 3:1). Muted green/red for numbers must still pass on the paper
  background.
- Logos: 98 company logos exist under `apps/web/public/logos/companies/`.
  Movers without a logo get no avatar at all (name and ticker only), never a
  lettered tile.
- The web app is a public repo candidate; no secrets in code. New secrets go
  on the Workers via `wrangler secret put`, which Rick runs.

## Visual design (decided)

Reference the mockups for proportions. Values here are the decisions.

**Palette.** Paper `#faf8f3`, ink `#1a1a1a`, muted text `#666`, rules
`#d9d5cc` (thin) and `#222` (section rules, masthead double rule), up
`#15803d`, down `#b91c1c`, kicker accent `#8a1c1c`, cover bar `#e8a317`.
Colour appears only in numbers, the kicker, and the cover bar. No pills, no
filled chips, no tiles. One theme.

**Type.** Headlines, masthead and section titles in a serif: Newsreader
(Google Fonts via `next/font`, variable, latin subset). Body, metadata,
tables and navigation in Inter (already loaded). Numbers use tabular
figures. Chinese pages: headlines in the system serif stack
(`"Songti SC", "Noto Serif CJK SC", "Source Han Serif SC", serif`), body in
the existing CJK sans stack; do not ship a CJK web font. Sizes: masthead 40px
desktop / 28px mobile; lead headline 36/28; article h1 32/26; body 16–17px
at 1.6–1.7 line height; metadata 12.5px.

**Masthead.** Centred "PreBell" in the serif with 盘前铃 beside it in small
grey, a date line underneath in small caps ("Wednesday, November 4, 2026 ·
After the close" / "Before the bell"), then navigation (Briefs, Earnings,
Calendar, About, 中文/EN) in the same small-caps line, and a double rule.
The Chinese edition shows the Chinese name first and the English second.

**Index line.** One centred grey line under the masthead on every page: S&P
500, Nasdaq 100, Dow, Russell 2000 with level and ▲/▼ percentage in muted
green/red, tabular figures, and the as-of time. The data comes from a small
snapshot the generator writes to the existing D1 `kv` table (key
`index_line`, JSON with the four indexes, their level, change, percentage
and as-of time, EN labels only; the web localizes) each time a brief
publishes, so every page pays one small D1 read and no R2 parse. If the
key is absent the line is omitted. On mobile it scrolls horizontally in
one line.

**Home (front page).** Two columns on desktop (about 1.6 : 1): left, the
lead story (latest brief): wide cover, serif headline, summary paragraph,
"Read the full recap" link; below it an "Earnings checks" section rule with
a row of up to three small covers and serif headlines for today's published
checks. Right, a ruled column: "Top movers" (name, ticker in grey, muted
percentage, one-line reason in grey; "No clear news" rendered in grey
italics), then "Reporting tomorrow" (before open / after close lists from the
calendar). The earnings strip that currently sits under the header becomes
this "Reporting" block; the calendar page keeps its own full view. Under the
earnings row, a "Recent" section rule and a ruled list of the last few
posts (the other edition of the day first, then earlier briefs and checks),
each a row with kicker, serif headline and date; this replaces the
premarket/recap `?tab=` switch, which is removed (old `?tab=` URLs still
render the page). The earnings strip is removed from post pages. On mobile
the columns stack: lead, movers, earnings, recent, reporting.

**Post page (brief or earnings check).** Masthead and index line, then one
680px reading column: kicker ("After-close recap · Nov 4, 2026" or
"Earnings check · Amgen (AMGN) · Q3 FY2026"), serif h1, metadata line
(published time; for earnings the filing date and accession) with a quiet
"✓ Numbers checked against source data" tag (bordered text, pale green
background, no icon other than the check), cover, body. Body keeps the
current sections and prose. Movers in a brief render as a ruled table:
company, reason in grey, close, change. The earnings key-figure table stays
exactly as the generator writes it in the body markdown (this quarter only);
no year-ago or change column, because the facts carry no verified
prior-year figure and the site derives no numbers of its own. The table's
CSS matches the mover table (thin rules, tabular figures, no fills).
Charts stay as the generator draws them; the web applies
`filter: grayscale(1)` to the chart images so the blue bars render in ink
grey on old and new posts alike, with no generator change. Then: Sources
block, disclaimer in italics, author block, signup block, partner strip,
footer. Brief pages keep their previous/next links at the end of the body.

**Lists (archive, earnings index, ticker page).** Same column, stories as
rows: small cover left (160px wide), kicker, serif headline, one-line
summary, date. No cards.

**Cover.** A 16:9 composition rendered in HTML/CSS: the scene image as
background (`object-fit: cover`, 55% opacity over near-black, desaturated),
a left-to-right dark gradient, a white tag top-left holding the company logo
(earnings) or the edition label (briefs: "Nov 4 · Recap" / "Nov 5 ·
Premarket"), and bottom-left two lines of bold white text with a short
amber bar under them. The two lines are localized per edition (EN/ZH).
Sizes: lead 280px tall, article 240px, small 110–150px; the text scales
with the container. If a story has no scene (generation failed or feature
off), the cover renders with a flat near-black background and the same
text, so the page never waits on the image.

**Author block.** At the end of every post page, after the disclaimer, and
on the About page under "Who is behind it". Greyscale round photo (72px),
committed under `apps/web/public/author.jpg` so the site never hotlinks
anything. A real head-and-shoulders photo of Rick is the intent; the
GitHub avatar is only the placeholder until he supplies one, and if no
photo is supplied by launch the block renders with no image rather than
the cartoon avatar. Name "Rick Liu" in
the serif, one line: "AI & software engineer, University of Waterloo. Built
PreBell to read earnings the way an analyst would, without the analyst."
Links: rickmingyuliu.com, GitHub (github.com/rick-mingyu-liu), LinkedIn
(linkedin.com/in/rick-mingyu-liu). A right-aligned "All briefs →" link to
the archive. Chinese edition: name in Latin letters, bio translated.

**Signup block.** Under the author block on post pages and in the home
footer area: "Get the weekly digest" heading, one line of copy ("One email a
week: the week's briefs and earnings checks. No spam, unsubscribe anytime."),
an email field and a Subscribe button in ink on paper, and after submission
an inline confirmation ("Check your inbox to confirm.") or error. Below it a
small "Already subscribed? Manage preferences" link to Buttondown's portal.

**Partner strip.** One line above the footer: "From our partners" in small
caps, the four partner names as plain links (moomoo, Wealthsimple, OKX,
Crypto.com; current geo rules unchanged), and "Affiliate links · not
investment advice" at the right. The sidebar box and the TermPocket card go
away; TermPocket becomes a line in the About page.

**Footer.** The existing disclaimer sentence, About, 中文/EN. No RSS link:
the site has no feed yet; a feed is a separate follow-up.

## Cover scenes (decided)

- Generated by the generator, once per post, at publish time, with Workers
  AI model `@cf/black-forest-labs/flux-1-schnell` via the Cloudflare REST API
  (`POST /accounts/{account}/ai/run/{model}`) using a scoped API token
  (`CF_AI_TOKEN`, Workers AI read/run) set as a Worker secret. Steps 4,
  size 1024×576.
- Prompt: a quiet, documentary, desaturated photograph of a place tied to the
  story's subject; no people, no faces, no text, no logos, no charts. The
  subject comes from facts: for earnings, the company's industry and
  products (a short descriptor the generator already knows or derives from
  the company name); for briefs, a trading-floor / city-skyline / exchange
  exterior family rotated by weekday. The exact prompt wording and negative
  prompt are the builder's.
- Stored in R2 `prebell-facts` under `covers/<slug>.png` (the model returns
  PNG; convert to JPEG only if a pure-Rust encoder is already a dependency,
  otherwise serve PNG).
- Cover data lives in D1, not in the facts JSON, so list pages and earnings
  pages (which load no facts today) can show covers with one query and no
  R2 parse. Migration `0006` adds `posts.cover_key TEXT NULL` and
  `post_translations.cover_line1 TEXT NULL`, `cover_line2 TEXT NULL`. The
  generator writes them at publish (the ZH lines when the ZH translation
  publishes). Existing list queries select the new columns.
- Two lines, composed by the generator from verified figures only, never a
  derived percentage: earnings: "Revenue $10.4B" (the extracted revenue
  figure, formatted in billions or millions as the facts' unit says) and
  "GAAP EPS $7.08" (non-GAAP EPS with its label when GAAP is missing);
  when either metric is missing, the fiscal period line ("Q3 FY2026
  results") takes its place, and when both are missing the second line is
  empty. Briefs: the biggest index move ("Dow +3.03%") and the top gainer
  and loser ("MOS +17.4%, FIS −23.7%"). Chinese lines use the same numbers
  with Chinese labels.
- Posts published before this ships have no cover columns. The web renders
  a flat near-black cover with only the tag (logo or edition label) and no
  lines for them; no backfill of lines or scenes. Lists may mix the two
  states; that is acceptable for the handful of posts that exist.
- Failure is silent to the reader: if the API errors, times out, or the
  neuron budget is exhausted, `scene_key` is null and the post publishes
  with the flat cover. One attempt per post, no retry loop, and the
  diagnostics line gains `cover=ok|skip|err`.
- The web serves covers through a route in the web Worker that reads the R2
  object and returns it with a long immutable cache header, the way charts
  are served today.

## Newsletter signup (decided)

- Provider: Buttondown, free plan, API key as web Worker secret
  `BUTTONDOWN_API_KEY`. Rick creates the account and the key.
- Web route `POST /api/subscribe` (no-store, like `/api/partners`) takes
  `{ email, source, lang }`, validates the email shape, rejects a filled
  honeypot field, and calls Buttondown's subscriber-create endpoint with
  the email and tags `prebell`, `lang:<en|zh>`, `source:<post|home|about>`.
  Already-subscribed is reported as success. Errors return a generic
  message; the Buttondown error text is logged, not shown.
- The form is a small client component; everything else on the page stays
  server-rendered.
- Double opt-in is Buttondown's confirmation email. The confirmation and
  the weekly send are configured in Buttondown, outside this project.
- `docs/decisions.md` gains the cadence decision (weekly digest, Rick,
  2026-11-04) and the provider.

## Content and copy rules

- Everything the generator writes stays as is. The redesign is the web app
  plus the cover step in the generator.
- "No clear news" stays as the honest label for a mover without a reason;
  render it quietly.
- Mover rows show name first, ticker second in grey. Logos are not used in
  mover rows.
- The EN and ZH editions have identical structure; copy lives in
  `apps/web/messages/en.json` and `zh.json` as today.
- Dates and times stay US Eastern.

## What exists today (for the builder)

- Routes: `src/app/[locale]/{page,about,archive,earnings,p/[slug],t/[ticker]}`.
- Components: HomeView, BriefView, PostBody, MoversList, NumbersBlock,
  Charts, EarningsStrip, CalendarWeek, PostList, Sidebar, PartnerLinks,
  Pill, TickerLogo, LangSwitch, Disclaimer, PendingNotice.
- CSS: `globals.css` with custom properties including a dark theme block
  under `prefers-color-scheme: dark`. 233 web tests (Vitest), CI runs them.
- Data: D1 (`posts`, `post_translations`, `sources`, `charts`, `runs`,
  `earnings_*`, calendar) and R2 facts JSON per post. Web reads via
  bindings in six files through `D1Like`/`R2Like`.
- Generator: `crates/core/src/{premarket,recap,earnings}` publish through
  `Store`; facts are written to R2 at publish.

## Left to the builder

- Component and file structure of the new pages, and whether to rewrite or
  restyle existing components. Keep one responsibility per file.
- Exact spacing, breakpoints (one mobile breakpoint around 720px is enough),
  and the horizontal-scroll treatment of the index line on mobile.
- The Flux prompt text, negative prompt, and the per-company subject
  descriptor source.
- How the cover text scales in the small variants.
- Test strategy for the new components and the subscribe route (mock
  Buttondown; never call it in tests).
- Whether the ticker page shows the index line (either is fine).

## Decisions and calls (mine unless marked Rick)

- Direction B, "the front page", over the newsletter and grid directions.
  (Rick, from the mockups.)
- Cover scenes are AI-generated, not a curated library. (Rick.)
- Author block placement: full block at the end of every article, AlphaLab
  style, with the short bio from the second mockup. (Rick.)
- Email: weekly digest like AlphaLab, not daily. (Rick.)
- Buttondown as provider, via our own `/api/subscribe` route, because
  AlphaLab's reverse-engineered form does the same and the free plan fits.
- Dark mode dropped: a paper has one look, and the current dark theme is
  the look Rick objected to. `prefers-color-scheme` is ignored.
- Newsreader for the serif: free, variable, designed for news reading,
  loads through `next/font` with no layout shift.
- Partner links move from a sidebar box to a footer strip.
- The TermPocket promo card is removed from the home page.
- Letter avatars are removed rather than replaced.
- Covers are composed in HTML, not baked into the image, so the ZH edition
  gets ZH text and a failed generation still yields a cover.
- One generation attempt per post, failing silently, so the publish path
  never waits on or breaks for an image.
- Cover lines and the scene key live in D1 (migration 0006), not in the
  facts JSON, so lists and earnings pages get covers from the query they
  already run. (Builder check.)
- No year-ago or change columns on the earnings table, and no percentage on
  the cover's revenue line: the facts carry no verified prior-year figure
  and the site must not derive numbers. (Builder check.)
- Charts are greyed with a CSS filter instead of redrawn in Rust, so old
  and new charts match and the generator's chart code is untouched.
  (Builder check.)
- The index line reads a `kv` snapshot the generator writes at brief
  publish, so every page can show it for one small D1 read. (Builder
  check.)
- No RSS link until a feed exists. (Builder check.)
- Posts from before launch get a tag-only flat cover; no backfill. (Builder
  check.)

- Home keeps a recent-posts list and brief pages keep previous/next links;
  the home tabs and the post-page earnings strip are removed. (Rick,
  2026-11-04, from the builder check's question.)
- Author photo is the GitHub avatar; links are site, GitHub and LinkedIn.
  (Rick.)

## Amendments after the code inventory (2026-11-04, my calls)

- **Index line data.** Premarket facts hold futures, recap facts hold index
  closes. The `kv` snapshot (`index_line`) is written by whichever brief
  publishes last and carries `kind` (`premarket` | `recap`), `as_of_utc`
  and four entries `{symbol, last, change_pct}`. The web labels by symbol
  with the existing `index.*` messages (so a premarket morning shows
  "S&P 500 futures" with the futures move) and prints the as-of time.
  `kv_set` is added to the `Store` trait; the D1 store already has the
  implementation under `EarningsStore`.
- **Cover lines for a premarket brief** use the biggest futures move
  ("S&P futures −1.14%") and the top gainer and loser, same shape as recap.
- **Scene image size.** Flux Schnell on Workers AI takes `prompt` and
  `steps` only; the image is whatever size the model returns (square). The
  cover crops it with `object-fit: cover`. The stored key keeps the real
  format: `covers/<slug>.png` or `.jpg` by magic bytes; `posts.cover_key`
  records it, and the covers route derives the content type from the
  extension.
- **Earnings kicker.** The web does not load earnings facts. The kicker is
  "Earnings check · <TICKER>" with the ticker from the `earnings_filings`
  join (list queries gain a `ticker` field, null for briefs); the company,
  fiscal period and filing line stay in the body's existing meta line.
- **Chinese mover reasons** do not exist in the facts (reasons are English
  summaries). The ZH movers table omits the reason column, as the ZH page
  does today. Not a regression; a ZH reason line is a separate follow-up.
- **"Reporting" block** uses the existing `stripDay` from the next trading
  day after the lead brief's date (from today when the lead is a
  premarket), non-exact, and shows the date it found in its heading.
- **Newsreader is self-hosted** via `@fontsource-variable/newsreader` and
  `next/font/local`, matching how Inter is loaded; no Google Fonts request.
- **`cover=` diagnostics** are appended to the logged line for briefs only
  (not stored in facts); earnings has no diagnostics line, so the Worker's
  cover client logs its own warn on failure.
- **Migration 0006** adds the three nullable columns; web test seeds that
  insert into `post_translations` positionally are converted to column
  lists in the same task.
- **Revenue is always present** on a published earnings post (the generator
  requires it), so the fiscal-period fallback applies to the EPS line only.

## Open for Rick

- Buttondown account and its API key, set on `prebell-web` as
  `BUTTONDOWN_API_KEY` before the signup form goes live. The form can ship
  behind a missing-key check that renders nothing until the key exists.
