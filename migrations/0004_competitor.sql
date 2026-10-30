-- Competitor watch (Plan 4). Internal only: written by the generator's daily
-- competitor job and read by the admin page. Never read by article generation.
-- Only feed metadata is stored: no descriptions, bodies or authors.

CREATE TABLE competitor_items (
  link TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  -- UTC ISO 8601 with Z; NULL when the item's pubDate is missing or unparseable.
  published_at TEXT,
  -- The item's categories joined with ', '.
  category TEXT,
  -- Tickers found in the title, space-separated (e.g. 'MSFT NVDA').
  tickers TEXT NOT NULL DEFAULT '',
  first_seen_at TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
);
CREATE INDEX idx_competitor_items_published ON competitor_items(published_at);

-- One report per ET date; its presence makes later firings that day a no-op.
CREATE TABLE competitor_reports (
  report_date TEXT PRIMARY KEY,
  window_start TEXT NOT NULL,
  window_end TEXT NOT NULL,
  -- core::competitor::report::Report as JSON.
  report_json TEXT NOT NULL,
  digest TEXT NOT NULL,
  created_at TEXT NOT NULL
);
