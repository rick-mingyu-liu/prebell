import Database from "better-sqlite3";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { migrations } from "./d1";

/**
 * The earnings week post's queries live in the generator (`crates/generator/src/week_store.rs`), which has no
 * SQLite in its tests, so they are run here against the real schema: their rows, and their plans with and
 * without ANALYZE (D1 does not run it for you). The per-poll read (`WEEK_RUNS`) must stay a range of the runs
 * primary key; the others read posts by the date index or by slug, never a whole table.
 */
const dir = fileURLToPath(new URL("../../../migrations/", import.meta.url));
const rust = readFileSync(fileURLToPath(new URL("../../../crates/generator/src/week_store.rs", import.meta.url)), "utf8");
const literal = (name: string) => {
  const m = new RegExp(`pub\\(crate\\) const ${name}: &str =\\s*"([\\s\\S]*?)";`).exec(rust);
  // A Rust string's `\` + newline drops the newline and the next line's indent.
  return (m?.[1] ?? "").replace(/\\\n\s*/g, "");
};
const SQL = {
  runs: literal("WEEK_RUNS"),
  posts: literal("WEEK_POSTS"),
  checks: literal("WEEK_CHECKS"),
  bySlug: literal("CHECKS_BY_SLUG"),
  written: literal("WRITTEN"),
};
const marked = (sql: string, n: number) => sql.replace("{slugs}", Array.from({ length: n }, (_, i) => `?${i + 1}`).join(", "));

/** A year of earnings checks and briefs, and the week posts and runs of four Fridays. */
function seed(): string {
  const rows: string[] = [];
  let id = 0;
  const post = (key: string, type: string, date: string, slug: string, published: string | null, en: string, zh: string | null, revised: string | null = null) => {
    id++;
    rows.push(`INSERT INTO runs VALUES ('${key}','${type}','${date}','published',1,NULL,'${date}T12:00:00Z','${date}T12:00:00Z');`);
    rows.push(
      `INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at, revised_at) VALUES (${id},'${key}','${type}','${type}:v1','${date}','${slug}','${type}/${slug}.json',${published ? `'${published}'` : "NULL"},${revised ? `'${revised}'` : "NULL"});`,
    );
    rows.push(`INSERT INTO post_translations (post_id, lang, status, title, cover_line1, cover_line2) VALUES (${id},'en','${en}','T','Revenue $1.0B','GAAP EPS $0.10');`);
    if (zh) rows.push(`INSERT INTO post_translations (post_id, lang, status, title, cover_line1, cover_line2) VALUES (${id},'zh','${zh}','中','营收 10亿美元','GAAP每股收益 0.10美元');`);
  };
  for (let i = 0; i < 365; i++) {
    const day = new Date(Date.UTC(2026, 0, 1) + i * 86_400_000).toISOString().slice(0, 10);
    post(`earnings:T${i}:acc-${i}`, "earnings", day, `t${i}-q3-fy2026-earnings`, `${day}T21:00:00Z`, "published", i % 7 === 0 ? "failed" : "published");
    post(`premarket:${day}`, "premarket", day, `premarket-${day}`, `${day}T12:00:00Z`, "published", "published");
  }
  // Week posts: Nov 13 and Nov 20 live, Nov 27 taken down (English failed), Dec 4 being built.
  post("week:2026-11-13", "week", "2026-11-13", "earnings-week-2026-11-13", "2026-11-13T23:05:00Z", "published", "published");
  post("week:2026-11-20", "week", "2026-11-20", "earnings-week-2026-11-20", "2026-11-20T23:05:00Z", "published", "published");
  post("week:2026-11-27", "week", "2026-11-27", "earnings-week-2026-11-27", "2026-11-27T23:05:00Z", "failed", "failed");
  post("week:2026-12-04", "week", "2026-12-04", "earnings-week-2026-12-04", "2026-12-04T23:05:00Z", "pending", null);
  rows.push(`UPDATE runs SET status = 'pending' WHERE run_key IN ('week:2026-11-20', 'week:2026-12-04');`);
  rows.push(`UPDATE runs SET status = 'building' WHERE run_key = 'week:2026-11-13';`);
  rows.push(`INSERT INTO runs VALUES ('week:2026-12-11','week','2026-12-11','failed',5,'x','t','t');`);
  rows.push(`INSERT INTO runs VALUES ('week:2026-12-18','week','2026-12-18','skipped',1,'x','t','t');`);
  // A check without a Chinese row, and one rebuilt after the cutoff.
  rows.push(`DELETE FROM post_translations WHERE post_id = 1 AND lang = 'zh';`);
  rows.push(`UPDATE posts SET published_at = '2026-12-10T15:00:00Z' WHERE slug = 't330-q3-fy2026-earnings';`);
  rows.push(`UPDATE posts SET revised_at = '2026-12-09T15:00:00Z' WHERE slug = 't331-q3-fy2026-earnings';`);
  return rows.join("\n");
}

function database(analyze: boolean) {
  const sqlite = new Database(":memory:");
  for (const f of migrations) sqlite.exec(readFileSync(dir + f, "utf8"));
  sqlite.exec(seed());
  if (analyze) sqlite.exec("ANALYZE");
  return sqlite;
}

/** `?1`.. as D1 binds them: better-sqlite3 takes numbered parameters as an object. */
const args = (...v: unknown[]) => Object.fromEntries(v.map((x, i) => [i + 1, x]));
const plan = (db: Database.Database, sql: string, a: Record<number, unknown>) =>
  (db.prepare(`EXPLAIN QUERY PLAN ${sql}`).all(a) as { detail: string }[]).map((r) => r.detail).join("\n");

describe.each([false, true])("week post queries (ANALYZE: %s)", (analyze) => {
  it("WEEK_RUNS: the pending and building week runs and this Friday's, with whether each is live, by the key range", () => {
    expect(SQL.runs).toContain("r.run_key >= 'week:' AND r.run_key < 'week;'");
    const db = database(analyze);
    const rows = db.prepare(SQL.runs).all(args("week:2026-12-11")) as { run_key: string; status: string; live: number }[];
    expect(rows.map((r) => [r.run_key, r.status, r.live])).toEqual([
      ["week:2026-11-13", "building", 1],
      ["week:2026-11-20", "pending", 1],
      ["week:2026-12-04", "pending", 0],
      ["week:2026-12-11", "failed", 0],
    ]);
    const p = plan(db, SQL.runs, args("week:2026-12-11"));
    expect(p).toMatch(/SEARCH r USING INDEX sqlite_autoindex_runs_1 \(run_key>\? AND run_key<\?\)/);
    expect(p).not.toMatch(/SCAN (r|p|t)\b/);
  });

  it("WEEK_POSTS: the live week posts before a Friday, newest first, by the date index", () => {
    const db = database(analyze);
    const rows = db.prepare(SQL.posts).all(args("2026-12-04", 4)) as { slug: string }[];
    expect(rows.map((r) => r.slug)).toEqual(["earnings-week-2026-11-20", "earnings-week-2026-11-13"]);
    const p = plan(db, SQL.posts, args("2026-12-04", 4));
    expect(p).toMatch(/idx_posts_trading_date/);
    expect(p).not.toMatch(/SCAN t\b|idx_post_translations_lang_status/);
  });

  it("WEEK_CHECKS: published checks from a report date, published by the cutoff, with both languages' lines", () => {
    const db = database(analyze);
    const a = args("2026-11-30", "2026-12-04T23:05:00Z");
    const rows = db.prepare(SQL.checks).all(a) as Record<string, string | null>[];
    // Nov 30 to Dec 4 (days 333-337 of 2026); the rebuilt day 330 and revised 331 are before it anyway.
    expect(rows.map((r) => r.slug)).toEqual([333, 334, 335, 336, 337].map((i) => `t${i}-q3-fy2026-earnings`));
    expect(rows[0]).toMatchObject({ en1: "Revenue $1.0B", en2: "GAAP EPS $0.10", zh1: "营收 10亿美元", zh_status: "published" });
    // Day 336's Chinese translation failed: its lines and status still come back.
    expect(rows[3]).toMatchObject({ slug: "t336-q3-fy2026-earnings", zh1: "营收 10亿美元", zh_status: "failed" });
    // One published after the cutoff is not listed; the first check has no Chinese row at all.
    const late = db.prepare(SQL.checks).all(args("2026-11-26", "2026-12-04T23:05:00Z")) as { slug: string }[];
    expect(late.map((r) => r.slug)).not.toContain("t330-q3-fy2026-earnings");
    const first = db.prepare(SQL.checks).all(args("2026-01-01", "2026-01-01T23:00:00Z")) as Record<string, string | null>[];
    expect(first).toMatchObject([{ slug: "t0-q3-fy2026-earnings", zh_status: null, zh1: null }]);
    const p = plan(db, SQL.checks, a);
    expect(p).toMatch(/SEARCH p USING INDEX idx_posts_trading_date \(trading_date>\?\)/);
    expect(p).not.toMatch(/SCAN (p|en)\b|idx_post_translations_lang_status/);
  });

  it("CHECKS_BY_SLUG and WRITTEN: by slug only", () => {
    const db = database(analyze);
    const slugs = ["t330-q3-fy2026-earnings", "t331-q3-fy2026-earnings", "earnings-week-2026-11-20"];
    const by = db.prepare(marked(SQL.bySlug, 3)).all(args(...slugs)) as { slug: string; published_at: string; revised_at: string | null }[];
    expect(by.map((r) => [r.slug, r.published_at, r.revised_at])).toEqual([
      ["t330-q3-fy2026-earnings", "2026-12-10T15:00:00Z", null],
      ["t331-q3-fy2026-earnings", "2026-11-28T21:00:00Z", "2026-12-09T15:00:00Z"],
    ]);
    const written = db.prepare(marked(SQL.written, 3)).all(args(...slugs)) as { slug: string }[];
    expect(written.map((r) => r.slug).sort()).toEqual(["t330-q3-fy2026-earnings", "t331-q3-fy2026-earnings"]);
    for (const sql of [marked(SQL.bySlug, 3), marked(SQL.written, 3)]) {
      const p = plan(db, sql, args(...slugs));
      expect(p).toMatch(/SEARCH (p|posts) USING INDEX sqlite_autoindex_posts_2 \(slug=\?\)/);
      expect(p).not.toMatch(/SCAN (p|posts|en)\b/);
    }
  });
});
