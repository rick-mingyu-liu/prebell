-- 0009_ipo_watch.sql: IPO watch (design docs/superpowers/specs/2026-11-11-ipo-watch-design.md,
-- contract docs/superpowers/specs/2026-11-11-ipo-watch-contract.md). Additive: new tables
-- only, nothing existing changes. Written only by the generator; the site reads.

-- One row per tracked IPO (crates/core/data/ipo-watch.tsv), once EDGAR lists a
-- registration filing for it. `first_trade_date` is the ET day shown on the
-- calendar, NULL when no date can be sourced ("date not set").
CREATE TABLE ipo_watch (
  slug TEXT PRIMARY KEY,
  cik INTEGER NOT NULL,
  name TEXT NOT NULL,
  name_zh TEXT,
  status TEXT NOT NULL CHECK (status IN ('filed', 'range_set', 'priced', 'listed', 'withdrawn')),
  exchange TEXT,
  symbol TEXT,
  -- The filing whose cover facts (ipo_facts) are current.
  facts_accession TEXT,
  first_trade_date TEXT,
  date_status TEXT CHECK (date_status IN ('reported', 'expected', 'confirmed')),
  date_source TEXT CHECK (date_source IN ('news', 'filing', 'trading')),
  date_source_name TEXT,
  date_source_url TEXT,
  date_quote TEXT,
  date_as_of TEXT,
  checked_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  -- A date always comes with its status and source, in these pairs only.
  CHECK ((first_trade_date IS NULL) = (date_status IS NULL)),
  CHECK ((date_status IS NULL) = (date_source IS NULL)),
  CHECK (date_status IS NULL
    OR (date_status = 'reported' AND date_source = 'news')
    OR (date_status = 'expected' AND date_source = 'filing')
    OR (date_status = 'confirmed' AND date_source IN ('filing', 'trading'))),
  CHECK (date_source_url IS NULL OR date_source_url LIKE 'https://%' OR date_source_url LIKE 'http://%')
);
CREATE INDEX idx_ipo_watch_date ON ipo_watch(first_trade_date);

-- Every filing of a tracked company that the timeline shows.
CREATE TABLE ipo_filings (
  accession TEXT PRIMARY KEY,
  slug TEXT NOT NULL,
  form TEXT NOT NULL,
  filing_date TEXT NOT NULL,
  index_url TEXT NOT NULL,
  doc_url TEXT,
  -- none: timeline only; pending: its cover is to be read; read; failed (gave up).
  read_state TEXT NOT NULL CHECK (read_state IN ('none', 'pending', 'read', 'failed')),
  attempts INTEGER NOT NULL DEFAULT 0,
  first_seen_at TEXT NOT NULL,
  CHECK (index_url LIKE 'https://%'),
  CHECK (doc_url IS NULL OR doc_url LIKE 'https://%')
);
CREATE INDEX idx_ipo_filings_slug ON ipo_filings(slug, filing_date);

-- Cover facts of a filing, each with the sentence it came from (verified by code
-- to be in the document).
CREATE TABLE ipo_facts (
  accession TEXT NOT NULL,
  kind TEXT NOT NULL,
  slug TEXT NOT NULL,
  value_num REAL,
  value_text TEXT,
  quote TEXT NOT NULL,
  PRIMARY KEY (accession, kind)
);
CREATE INDEX idx_ipo_facts_slug ON ipo_facts(slug);

-- "As reported" news items: a quote that code found word for word in the
-- article, with its outlet, link and publication day. One per (slug, kind, url).
CREATE TABLE ipo_items (
  slug TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('valuation', 'expected_date', 'company_move', 'view')),
  url TEXT NOT NULL,
  outlet TEXT NOT NULL,
  published_on TEXT NOT NULL,
  quote TEXT NOT NULL,
  statement_en TEXT,
  statement_zh TEXT,
  speaker TEXT,
  value_usd REAL,
  value_date TEXT,
  found_at TEXT NOT NULL,
  PRIMARY KEY (slug, kind, url),
  CHECK (url LIKE 'https://%')
);
CREATE INDEX idx_ipo_items_slug ON ipo_items(slug, kind, published_on);

-- After listing: one row per regular session read by the recap (Yahoo).
CREATE TABLE ipo_trading (
  slug TEXT NOT NULL,
  session_date TEXT NOT NULL,
  symbol TEXT NOT NULL,
  open REAL,
  close REAL NOT NULL,
  volume INTEGER,
  ipo_price REAL,
  change_pct REAL,
  source_url TEXT NOT NULL,
  fetched_at TEXT NOT NULL,
  PRIMARY KEY (slug, session_date),
  CHECK (source_url LIKE 'https://%')
);
