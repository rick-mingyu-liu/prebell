# Premarket brief at 8:00 ET, updated at 8:35 ET

Date: 2026-11-06. Rick asked "is it also possible to trigger the articles
slightly earlier? or current we can only do it around 8:30", and chose
**"8:00 brief + 8:35 update"** over a plain move to 8:00 and over keeping
8:30.

## Why

The brief goes out at 8:30 ET today. Moving it to 8:00 gets it to readers
(and to Beijing evenings) half an hour sooner, but 8:30 ET is when CPI, the
jobs report, jobless claims, GDP and retail sales come out, so an 8:00 brief
is written before the number. A second pass at 8:35 rebuilds the brief once
the release is out, so the day's brief is both early and complete.

## Goals

- The premarket brief publishes from **08:00 ET** (window 08:00-09:25,
  unchanged end).
- From **08:35 ET** a second build, the **update**, rebuilds the whole brief
  from fresh facts (futures, movers, macro news, themes) and **rewrites the
  same post at the same URL**, EN and ZH together.
- The post shows when it was updated ("Updated 8:35 AM ET" / ZH equivalent)
  next to its publish time.

## Non-goals

- No economic calendar: the update does not know whether a release is due
  today. It runs every trading day. (`watch_list` stays "not available in
  v1".)
- No diff, changelog or second post. Readers see one brief per day.
- No change to the recap, earnings checks or the cron triggers (the existing
  `*/5 10-23 * * MON-FRI` UTC covers 08:00 and 08:35 ET in both offsets).

## Anti-goals (a failure even if it runs)

- The update must never leave the brief **worse** than the 8:00 version: no
  pending/blank ZH, no half-updated post, no lost post when the update fails.
- Never two posts for one day, never a slug conflict.
- No alert noise: a failed or unnecessary update is not an error; the 8:00
  brief stands.

## Decided behaviour

### Base brief (`premarket:YYYY-MM-DD`)
Unchanged state machine; only `premarket_window` starts at 08:00 instead of
08:30. Everything that keys off the window (ZH retry cadence, tests, docs)
follows.

### Update (`premarket-update:YYYY-MM-DD`, its own run row)
- **Window 08:35-09:25 ET** on trading days. Before 08:35: nothing.
- **Precondition:** the base run is `published`. If not yet, do nothing this
  tick (the base may still publish late). At the deadline with no base, mark
  the update `skipped` quietly.
- **Not needed:** if the base post was published at or after **08:30 ET**
  (a late base build already saw the 8:30 data), mark the update `skipped`
  with `last_error = "base published after 08:30"`, no build.
- **Build:** same facts builder, gate and drafts as the base, with a fresh
  subrequest budget (its own queue message/invocation).
- **All or nothing:** the update writes only when the gate passes **and
  both EN and ZH drafts succeed**. Otherwise it records `attempts` and
  `last_error` and retries next firing, untouched post. Past 09:25 it is
  marked `skipped` with **no alert**.
- **On success** it writes, onto the base post (`posts.run_key` stays
  `premarket:DATE`, same slug): fresh facts under a **new key**
  `premarket/DATE-update.json`, written to R2 first, with the post's
  `facts_r2_key` switched to it inside the same D1 batch as the text, so
  the post and its facts flip together and the 8:00 facts stay intact
  (a failed batch only leaves an unreferenced R2 object; ruled 2026-11-06,
  replacing "same facts key, overwritten"), both translations, sources
  (replace), the front-page index line, and a new cover at a **new key**
  per attempt, `covers/<slug>-update-<attempt>.svg`, with the post's
  `cover_key` pointing to it (covers are served with a one-day cache, so a
  key the site may serve must never be rewritten; ruled 2026-11-06). A
  failed cover write retries like a failed draft, leaving the 8:00 cover. `published_at` keeps the base time;
  the new `posts.revised_at` gets the update time. The update run becomes
  `published`.
- **Reconcile must not misfire:** the base post already exists and is
  published, so the base's "EN live but run not published" repair must not
  mark the update published without building. (`posts.revised_at` set is
  the update's own proof: a rewrite that landed before its run write was
  lost marks the run published with no second build; ruled 2026-11-06.)
- **ZH race:** if the base ZH is still `pending` and the base's ZH retry is
  drafting from the 8:00 facts when the update lands, the retry must not
  overwrite the update's ZH (re-check the post's revision before writing,
  or an equivalent guard). Implemented as a conditional write: the ZH row
  lands only while the post's `revised_at` is the one the retry read.

### Schema
Migration `0007_post_revised_at.sql`: `ALTER TABLE posts ADD COLUMN
revised_at TEXT;` (nullable, additive; nothing existing changes). Rick
applies it to remote D1 before deploying.

### Web
- The post page shows the update time next to the publish time when
  `revised_at` is set ("Updated 8:35 AM ET"; ZH in the site's existing
  date/time style). Lists and the front page keep sorting by
  `published_at`.

### Cost
One more full premarket build per trading day: roughly 11-12 Tavily
searches and 3 LLM calls (EN, ZH, extraction). Rick should check the
Tavily plan quota; this is about +230 searches/month.

## Left to the builder

- Whether the update is a second `Content` impl with extra trait hooks, or a
  dedicated `tick_update` beside the shared state machine: whichever keeps
  the base path unchanged and the tests readable.
- How the job is enqueued from the cron (new job kind in `jobs.rs` /
  `schedule.rs`, keeping the cron/job sync test green).
- Exact wording of the "Updated" line, matching the site's existing time
  format in both languages.
