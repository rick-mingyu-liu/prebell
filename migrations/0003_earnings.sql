-- Plan 3: earnings detection, health checks, charts and calendar.

-- One row per detected 8-K Item 2.02 filing. Doubles as the build queue:
-- `state = 'open'` until the run reaches a final state.
CREATE TABLE earnings_filings (
  accession TEXT PRIMARY KEY,
  cik INTEGER NOT NULL,
  ticker TEXT NOT NULL,
  form TEXT NOT NULL,
  items TEXT NOT NULL,
  filing_date TEXT NOT NULL,
  report_date TEXT NOT NULL,
  accepted_at TEXT NOT NULL,
  primary_doc TEXT NOT NULL,
  run_key TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('open','done')),
  detected_at TEXT NOT NULL
);
CREATE INDEX idx_earnings_filings_state ON earnings_filings(state, accepted_at);
CREATE INDEX idx_earnings_filings_ticker ON earnings_filings(ticker);

-- Chart SVGs (stored in R2 under `r2_key`) shown on a post.
CREATE TABLE post_charts (
  post_id INTEGER NOT NULL REFERENCES posts(id),
  name TEXT NOT NULL,
  r2_key TEXT NOT NULL,
  alt_en TEXT NOT NULL,
  alt_zh TEXT NOT NULL,
  sort INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (post_id, name)
);

-- Upcoming and past report dates. `confirmed` needs primary proof (company
-- announcement or EDGAR); `quarantined` rows are hidden from the site.
CREATE TABLE earnings_calendar (
  ticker TEXT NOT NULL,
  report_date TEXT NOT NULL,
  time_slot TEXT NOT NULL CHECK (time_slot IN ('bmo','amc','unknown')),
  status TEXT NOT NULL CHECK (status IN ('unconfirmed','confirmed','reported','quarantined')),
  source TEXT NOT NULL CHECK (source IN ('yahoo','ir','edgar')),
  proof_url TEXT,
  proof_quote TEXT,
  conflict_reason TEXT,
  first_seen_at TEXT NOT NULL,
  checked_at TEXT NOT NULL,
  next_check_at TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (ticker, report_date)
);
CREATE INDEX idx_earnings_calendar_date ON earnings_calendar(report_date);

-- Small generator state (e.g. the ET date of the last calendar refresh).
CREATE TABLE kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
