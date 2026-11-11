import Database from "better-sqlite3";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { prepareCall, type ToolContext } from "../src/lib/mcptools";
import { getLatestPost, type D1Like } from "../src/lib/posts";
import { migrations } from "./d1";
import { bucket, FRIDAY, objects, seed } from "./mcpseed";

/**
 * Guards against queries whose cost grows with the archive: the newest brief must be found by walking
 * idx_posts_trading_date backwards, never by reading every translation in a language.
 */
const dir = fileURLToPath(new URL("../../../migrations/", import.meta.url));

function database(analyze: boolean) {
  const sqlite = new Database(":memory:");
  for (const f of migrations) sqlite.exec(readFileSync(dir + f, "utf8"));
  sqlite.exec(seed());
  if (analyze) sqlite.exec("ANALYZE");
  return sqlite;
}

/** A D1 stand-in that records each statement's query plan alongside running it. */
function planning(sqlite: Database.Database) {
  const plans: { sql: string; plan: string }[] = [];
  const db: D1Like = {
    prepare(sql: string) {
      let args: unknown[] = [];
      const st = sqlite.prepare(sql);
      const record = () => {
        const rows = sqlite.prepare(`EXPLAIN QUERY PLAN ${sql}`).all(...args) as { detail: string }[];
        plans.push({ sql, plan: rows.map((r) => r.detail).join("\n") });
      };
      const stmt = {
        bind(...values: unknown[]) {
          args = values;
          return stmt;
        },
        async first<T>() {
          record();
          return (st.get(...args) as T | undefined) ?? null;
        },
        async all<T>() {
          record();
          return { results: st.all(...args) as T[] };
        },
      };
      return stmt;
    },
  };
  return { db, plans };
}

const POSTS_SCAN = /SCAN (p|posts)\b|idx_post_translations_lang_status/;

describe.each([false, true])("query plans (ANALYZE: %s)", (analyze) => {
  it("latest_brief walks the trading-date index", async () => {
    for (const args of [{}, { kind: "recap" }, { lang: "zh", full_text: true }]) {
      const { db, plans } = planning(database(analyze));
      const ctx: ToolContext = { db, bucket: bucket(objects()), origin: new URL("https://prebell.site"), now: FRIDAY };
      await prepareCall("latest_brief", args, ctx).run();
      const brief = plans.find((p) => p.sql.includes("FROM posts p"))!;
      expect(brief.plan).toMatch(/SEARCH p USING INDEX idx_posts_trading_date/);
      expect(brief.plan).not.toMatch(POSTS_SCAN);
    }
  });

  it("getLatestPost (the home page) walks the trading-date index", async () => {
    for (const lang of ["en", "zh"] as const) {
      for (const type of ["premarket", "recap", "earnings"] as const) {
        const { db, plans } = planning(database(analyze));
        await getLatestPost(db, lang, type);
        expect(plans[0].plan).toMatch(/SEARCH p USING INDEX idx_posts_trading_date/);
        expect(plans[0].plan).not.toMatch(POSTS_SCAN);
      }
    }
  });
});
