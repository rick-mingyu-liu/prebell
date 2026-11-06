-- 0008_macro_events.sql: the macro events calendar (FOMC decisions and minutes,
-- the jobs report, CPI, elections, market closures), shown beside earnings and
-- in the premarket brief's "What to watch". Additive; nothing existing changes.
-- `id` is source:series:period ("bls:cpi:2026-10") and stays when a date moves;
-- `series` is free text so later kinds need no migration. Every row carries the
-- official page that lists its date (`proof_url`) and, once out, the release.
CREATE TABLE macro_events (
  id TEXT PRIMARY KEY,
  series TEXT NOT NULL,
  ref_period TEXT,
  variant TEXT,
  -- UTC ISO 8601 with Z; NULL for all-day events (elections, closures).
  starts_at TEXT,
  -- The ET calendar day the event shows on, YYYY-MM-DD.
  et_date TEXT NOT NULL,
  tier INTEGER NOT NULL CHECK (tier IN (1, 2, 3)),
  status TEXT NOT NULL CHECK (status IN ('scheduled', 'confirmed', 'moved', 'postponed', 'released')),
  proof_url TEXT NOT NULL,
  release_url TEXT,
  -- The previous et_date while status = 'moved'.
  moved_from TEXT,
  -- Consecutive live checks that did not list the event (2 = postponed).
  missing_count INTEGER NOT NULL DEFAULT 0,
  first_seen_at TEXT NOT NULL,
  checked_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX idx_macro_events_date ON macro_events(et_date);
