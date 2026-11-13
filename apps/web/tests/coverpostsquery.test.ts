import Database from "better-sqlite3";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { migrations } from "./d1";

/**
 * The share cards' query lives in the generator (`COVER_POSTS` in crates/generator/src/store.rs), which
 * has no SQLite in its tests, so it is checked here against the real schema and the IPO preview seeds:
 * the card's kicker names an earnings check's filing ticker, or an IPO write-up's company name and symbol
 * from its IPO's watch row, and the posts are read through idx_posts_trading_date.
 */
const dir = fileURLToPath(new URL("../../../migrations/", import.meta.url));
const db = fileURLToPath(new URL("../../../db/", import.meta.url));
const rust = readFileSync(fileURLToPath(new URL("../../../crates/generator/src/store.rs", import.meta.url)), "utf8");
const literal = /pub\(crate\) const COVER_POSTS: &str =\s*"([\s\S]*?)";/.exec(rust);
// A Rust string's `\` + newline drops the newline and the next line's indent.
const COVER_POSTS = (literal?.[1] ?? "").replace(/\\\n\s*/g, "");

function database() {
  const sqlite = new Database(":memory:");
  for (const f of migrations) sqlite.exec(readFileSync(dir + f, "utf8"));
  // Anthropic (symbol ANT) with its range_set write-up and cover; Harbor Bio (HRBO) tracked.
  sqlite.exec(readFileSync(db + "seed-ipo-preview.sql", "utf8"));
  sqlite.exec(readFileSync(db + "seed-ipo-articles-preview.sql", "utf8"));
  const runs = ["recap:2026-11-11", "earnings:2026-11-11:1", "ipo:harbor-bio:filed:9999990002-26-000001", "ipo:gone:filed:1"];
  for (const r of runs) sqlite.exec(`INSERT INTO runs VALUES ('${r}','x','2026-11-11','published',1,NULL,'t','t');`);
  sqlite.exec(`
    INSERT INTO posts (run_key, content_type, template_id, trading_date, slug, published_at, cover_key) VALUES
      ('recap:2026-11-11', 'recap', 'recap:v1', '2026-11-11', 'recap-2026-11-11', '2026-11-11T21:00:00Z', 'covers/recap-2026-11-11.svg'),
      ('earnings:2026-11-11:1', 'earnings', 'earnings:v1', '2026-11-11', 'amd-q3', '2026-11-11T20:00:00Z', 'covers/amd-q3.svg'),
      ('ipo:harbor-bio:filed:9999990002-26-000001', 'ipo', 'ipo-filed', '2026-11-09', 'harbor-bio-ipo-filing-2026-11-09', '2026-11-09T22:00:00Z', 'covers/harbor-bio-ipo-filing-2026-11-09.svg'),
      ('ipo:gone:filed:1', 'ipo', 'ipo-filed', '2026-11-09', 'gone-ipo-filing-2026-11-09', '2026-11-09T21:00:00Z', 'covers/gone-ipo-filing-2026-11-09.svg');
    INSERT INTO earnings_filings VALUES ('a1',2488,'AMD','8-K','2.02','2026-11-11','2026-09-30','x','d','earnings:2026-11-11:1','done','x');
  `);
  return sqlite;
}

describe("COVER_POSTS", () => {
  it("names an IPO write-up's company and symbol, and an earnings check's filing ticker", () => {
    expect(COVER_POSTS).toContain("ipo_watch");
    type Row = { cover_key: string; content_type: string; ticker: string | null; ipo_name: string | null; ipo_symbol: string | null };
    const rows = database().prepare(COVER_POSTS).all({ 1: 30, 2: "2026-10-29" }) as Row[];
    expect(Object.fromEntries(rows.map((r) => [r.cover_key, [r.content_type, r.ticker, r.ipo_name, r.ipo_symbol]]))).toEqual({
      "covers/anthropic-ipo-price-range-2026-11-10.svg": ["ipo", null, "Anthropic", "ANT"],
      "covers/recap-2026-11-11.svg": ["recap", null, null, null],
      "covers/amd-q3.svg": ["earnings", "AMD", null, null],
      "covers/harbor-bio-ipo-filing-2026-11-09.svg": ["ipo", null, "Harbor Bio", "HRBO"],
      // An IPO no longer tracked: no name or symbol (its kicker is "IPO" and the date).
      "covers/gone-ipo-filing-2026-11-09.svg": ["ipo", null, null, null],
    });
  });

  it("reads posts through the trading-date index and each IPO by its primary key", () => {
    const plan = (
      database()
        .prepare(`EXPLAIN QUERY PLAN ${COVER_POSTS}`)
        .all({ 1: 30, 2: "2026-10-29" }) as { detail: string }[]
    ).map((r) => r.detail);
    expect(plan.join("\n")).toMatch(/SEARCH p USING INDEX idx_posts_trading_date/);
    expect(plan.join("\n")).toMatch(/SEARCH w USING INDEX sqlite_autoindex_ipo_watch_1 \(slug=\?\)/);
  });
});
