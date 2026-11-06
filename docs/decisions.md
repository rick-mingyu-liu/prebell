# Decisions

Things Rick has decided, with the date and his words where they matter, so
they are never asked twice. Things decided *not* to do go in `declined.md`.

## 2026-11-04

- **Frontend hosting: Cloudflare Workers, not Vercel.** The web app reads D1
  and R2 through bindings; Vercel would need HTTP adapters and its Hobby plan
  bars commercial use. Rick chose "Cloudflare Workers now".
- **Site look: "B · the front page".** A daily paper: warm off-white, serif
  headlines, lead story with a cover, movers as a ruled column, no pills, no
  boxes. Rick: "i think this one ace over: B". Dark mode is dropped.
  Design: `docs/superpowers/specs/2026-11-04-front-page-redesign-design.md`.
- **Cover images are AI-generated** (Workers AI, Flux Schnell, free
  allocation), composed with the logo and two verified figures at render
  time. Rick: "AI-generated scenes". **Superseded on 2026-11-05** by "Covers
  are data graphics, not AI scenes" below.
- **Author block** at the end of every article and on About, AlphaLab style,
  short bio, links: rickmingyuliu.com, GitHub, LinkedIn. Real photo intended;
  GitHub avatar is only a placeholder.
- **Email: a weekly digest, like AlphaLab, not a daily email.** Rick:
  "weekly digest like AlphaLab". Provider Buttondown (free to 100
  subscribers) behind our own `/api/subscribe` route. Signup ships with the
  redesign; sending is a separate later project.
- **Home keeps a recent-posts list and briefs keep previous/next links;
  the home premarket/recap tabs and the earnings strip on post pages are
  removed.** (Rick, via the builder-check question.)
- **Generator LLM stays DeepSeek** (`--var LLM_PROVIDER:deepseek` on deploy)
  until the Max-plan API credit is claimed, then switch to Claude.

## 2026-11-05

- **Section order follows the story: macro → themes → movers (AlphaLab's
  digest order); approved by Rick 2026-11-05 when he approved the
  themes-and-macro design.**
- **Free-plan budget: 4 movers per side instead of 150-symbol Yahoo
  batches** (controller's call after the 150-symbol live check could not
  run; reversible in one constant once a live check from another network
  passes).
- **The not-advice statement lives only on the legal page** ("Terms: nature of content" / "条款：内容性质声明"), modelled on AlphaLab's 内容性质声明; no footer or per-post disclaimer. Rick, 2026-11-05: "just include the not advice section only in legal and privacy policy page instead of showing everywhere".
- **Covers are data graphics, not AI scenes** (replaces the 2026-11-04
  AI-generated covers entry): the generator draws the story's own figures as
  an SVG chart in the paper palette, a brief's biggest movers and an earnings
  check's recent revenue, with no Workers AI call. Rick, choosing among four
  mockups: "A · Data graphic"; and, of the AI covers: "they should be more
  about the market instead of just the dark gray shadows". Design:
  `docs/superpowers/specs/2026-11-05-data-covers-design.md`.
- **Author photo in colour, not greyscale.** Rick, 2026-11-05: "do not make my photo black and white, original colorful". The `.author-photo` grayscale filter is gone; the file itself was always colour.
- **The site's domain is `prebell.site`**, served by the `prebell-web` Worker as a custom domain. www redirects to the apex (a second custom domain plus a Next redirect, no dashboard rule), and the old workers.dev URL stays up. Rick, 2026-11-05: "i purchased the domain name: prebell.site".
- **Contact addresses are on the domain, as with TermPocket**: `privacy@prebell.site` (privacy requests), `support@prebell.site` (terms questions, EDGAR `SEC_CONTACT`), forwarded to the owner's Gmail by Cloudflare Email Routing. Rick, 2026-11-05: "do the email address into the same way we handle as termpocket, use the site one".

## 2026-11-06

- **The affiliate disclosure is the partner strip's label**: "Affiliate partners" / "推广合作" replaces "From our partners" plus the sentence on the right, and the terms ("sources and links") say PreBell may earn a referral fee through those links. Rick asked "can we remove this note?"; the disclosure must stay where readers see the links, so it was folded into the label (Rick chose "Fold into the label").
- **Earnings: fix the failures first, then widen coverage** beyond the S&P 100: all ~520 companies (S&P 500 ∪ Nasdaq-100) on the calendar, full write-ups only for the largest. Rick chose "Fix failures, then widen" on 2026-11-06 after a week where 6 of 14 detected reports did not publish.
- **An earnings check publishes without revenue when the release states none** (AIG, ConocoPhillips): revenue stays required when the release gives it, so a figure that fails verification still blocks and retries. Ruled under Rick's "Fix failures, then widen" (2026-11-06) (the spec left required sections to the designer); reversible in `missing_required`.
- **The premarket brief publishes at 8:00 ET and is updated at 8:35 ET** with the 8:30 economic data: the update rebuilds the brief from fresh facts and rewrites the same post (same URL, "Updated" time shown), only when both languages draft; otherwise the 8:00 brief stands. Rick asked "is it also possible to trigger the articles slightly earlier?" and chose "8:00 brief + 8:35 update" over a plain 8:00 move and over keeping 8:30. Design: `docs/superpowers/specs/2026-11-06-premarket-update-design.md`.
