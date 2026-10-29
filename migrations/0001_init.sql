CREATE TABLE runs (
  run_key TEXT PRIMARY KEY,
  content_type TEXT NOT NULL,
  trading_date TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending','building','published','skipped','failed')),
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE posts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_key TEXT NOT NULL UNIQUE REFERENCES runs(run_key),
  content_type TEXT NOT NULL,
  template_id TEXT NOT NULL,
  trading_date TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE,
  facts_r2_key TEXT,
  published_at TEXT
);
CREATE INDEX idx_posts_trading_date ON posts(trading_date);

CREATE TABLE post_translations (
  post_id INTEGER NOT NULL REFERENCES posts(id),
  lang TEXT NOT NULL CHECK (lang IN ('en','zh')),
  status TEXT NOT NULL CHECK (status IN ('published','pending','failed')),
  title TEXT,
  summary TEXT,
  body_md TEXT,
  updated_at TEXT,
  PRIMARY KEY (post_id, lang)
);
CREATE INDEX idx_post_translations_lang_status ON post_translations(lang, status);

CREATE TABLE sources (
  post_id INTEGER NOT NULL REFERENCES posts(id),
  url TEXT NOT NULL,
  title TEXT,
  quote TEXT
);

CREATE TABLE alerts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_key TEXT,
  level TEXT NOT NULL,
  message TEXT NOT NULL,
  created_at TEXT NOT NULL,
  sent INTEGER NOT NULL DEFAULT 0
);
