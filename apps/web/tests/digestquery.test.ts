import Database from "better-sqlite3";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { migrations } from "./d1";

/**
 * The weekly digest's picks query lives in the generator (`DIGEST_POSTS` in
 * crates/generator/src/store.rs), which has no SQLite in its tests, so it is checked here against the
 * real schema: it must read the week through idx_posts_trading_date, never every published English
 * translation, with or without ANALYZE (D1 does not run it for you).
 */
const dir = fileURLToPath(new URL("../../../migrations/", import.meta.url));
const rust = readFileSync(fileURLToPath(new URL("../../../crates/generator/src/store.rs", import.meta.url)), "utf8");
const literal = /pub\(crate\) const DIGEST_POSTS: &str = "([\s\S]*?)";/.exec(rust);
// A Rust string's `\` + newline drops the newline and the next line's indent.
const DIGEST_POSTS = (literal?.[1] ?? "").replace(/\\\n\s*/g, "");

/** 400 trading days of published posts in both languages, some earnings with a filing row. */
function seed(): string {
  const rows: string[] = [];
  for (let i = 0; i < 400; i++) {
    const day = new Date(Date.UTC(2025, 0, 1) + i * 86_400_000).toISOString().slice(0, 10);
    for (const [n, type] of [
      [2 * i + 1, "earnings"],
      [2 * i + 2, "premarket"],
    ] as const) {
      const key = `${type}:${day}:${n}`;
      rows.push(`INSERT INTO runs VALUES ('${key}','${type}','${day}','published',1,NULL,'${day}T12:00:00Z','${day}T12:00:00Z');`);
      rows.push(
        `INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, published_at) VALUES (${n},'${key}','${type}','${type}:v1','${day}','${type}-${n}','${day}T21:00:00Z');`,
      );
      rows.push(`INSERT INTO post_translations (post_id, lang, status, title, summary) VALUES (${n},'en','published','T${n}','S${n}');`);
      rows.push(`INSERT INTO post_translations (post_id, lang, status, title, summary) VALUES (${n},'zh','published','中${n}','摘${n}');`);
    }
  }
  return rows.join("\n");
}

function database(analyze: boolean) {
  const sqlite = new Database(":memory:");
  for (const f of migrations) sqlite.exec(readFileSync(dir + f, "utf8"));
  sqlite.exec(seed());
  // The week of 2025-03-03: one post with only a Chinese page, one with an empty English title, one
  // unpublished, and one with a filing row (its ticker).
  sqlite.exec(`
    UPDATE post_translations SET status = 'pending' WHERE post_id = 129 AND lang = 'en';
    UPDATE post_translations SET title = '' WHERE post_id = 123;
    UPDATE posts SET published_at = NULL WHERE id = 125;
    INSERT INTO earnings_filings VALUES ('a1',2488,'AMD','8-K','2.02','2025-03-04','2025-01-31','x','d','earnings:2025-03-05:127','done','x');
  `);
  if (analyze) sqlite.exec("ANALYZE");
  return sqlite;
}

// `?1`..`?3`, as D1 binds them: better-sqlite3 takes numbered parameters as an object.
const ARGS = { 1: "earnings", 2: "2025-03-03", 3: "2025-03-09" };

describe.each([false, true])("DIGEST_POSTS (ANALYZE: %s)", (analyze) => {
  it("reads the week through the trading-date index", () => {
    expect(DIGEST_POSTS).toMatch(/^SELECT p\.slug, .* ORDER BY p\.trading_date, p\.published_at, p\.id$/);
    const sqlite = database(analyze);
    const plan = (sqlite.prepare(`EXPLAIN QUERY PLAN ${DIGEST_POSTS}`).all(ARGS) as { detail: string }[])
      .map((r) => r.detail)
      .join("\n");
    expect(plan).toMatch(/SEARCH p USING INDEX idx_posts_trading_date \(trading_date>\? AND trading_date<\?\)/);
    expect(plan).not.toMatch(/SCAN (p|en)\b|idx_post_translations_lang_status/);
  });

  it("returns only the week's earnings posts whose English page is published with a title", () => {
    const sqlite = database(analyze);
    const rows = sqlite.prepare(DIGEST_POSTS).all(ARGS) as {
      slug: string;
      trading_date: string;
      ticker: string | null;
      title: string;
      summary: string;
    }[];
    // Mar 3-9, 2025 are days 61-67: earnings posts 123..135 (odd ids), minus 123 (empty title),
    // 125 (unpublished) and 129 (Chinese page only).
    expect(rows.map((r) => r.slug)).toEqual(["earnings-127", "earnings-131", "earnings-133", "earnings-135"]);
    expect(rows[0]).toEqual({ slug: "earnings-127", trading_date: "2025-03-05", ticker: "AMD", title: "T127", summary: "S127" });
    expect(rows[1].ticker).toBeNull();
  });
});
