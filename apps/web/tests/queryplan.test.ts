import Database from "better-sqlite3";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { listHomeIpos, listIpoSlugs, listIposBetween, listIpoWatch, loadIpo } from "../src/lib/ipo";
import { prepareCall, type ToolContext } from "../src/lib/mcptools";
import { getLatestPost, type D1Like } from "../src/lib/posts";
import { migrations } from "./d1";
import { bucket, FRIDAY, objects, seed } from "./mcpseed";

/**
 * Guards against queries whose cost grows with the archive: the newest brief must be found by walking
 * idx_posts_trading_date backwards, never by reading every translation in a language.
 */
const dir = fileURLToPath(new URL("../../../migrations/", import.meta.url));

function database(analyze: boolean, seedSql = seed()) {
  const sqlite = new Database(":memory:");
  for (const f of migrations) sqlite.exec(readFileSync(dir + f, "utf8"));
  sqlite.exec(seedSql);
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

/**
 * The IPO reads stay on the keys and indexes of migration 0009. The watch list alone reads ipo_watch whole: the
 * table is the hand-kept list of tracked IPOs (a handful a year), and the read is capped at 20 rows.
 */
const IPO_SEED = readFileSync(new URL("../../../db/seed-ipo-preview.sql", import.meta.url), "utf8");

describe.each([false, true])("IPO query plans (ANALYZE: %s)", (analyze) => {
  it("the calendar's IPOs walk the first-trade-date index", async () => {
    const { db, plans } = planning(database(analyze, IPO_SEED));
    await listIposBetween(db, "2026-11-16", "2026-11-20", "en");
    expect(plans).toHaveLength(1);
    expect(plans[0].plan).toMatch(/SEARCH ipo_watch USING INDEX idx_ipo_watch_date \(first_trade_date>\? AND first_trade_date<\?\)/);
  });

  it("the watch list is one capped read of the hand-kept list", async () => {
    const { db, plans } = planning(database(analyze, IPO_SEED));
    await listIpoWatch(db, "2026-11-11", "en");
    expect(plans).toHaveLength(1);
    expect(plans[0].sql).toMatch(/LIMIT 20$/);
  });

  it("an IPO page reads each table by its key or index, never a whole table", async () => {
    const { db, plans } = planning(database(analyze, IPO_SEED));
    await loadIpo(db, "anthropic", "en");
    expect(plans).toHaveLength(6);
    const plan = (table: string) => plans.find((p) => new RegExp(`FROM ${table}\\b`).test(p.sql))!.plan;
    expect(plans[0].plan).toMatch(/SEARCH ipo_watch USING INDEX sqlite_autoindex_ipo_watch_1 \(slug=\?\)/);
    expect(plan("ipo_facts")).toMatch(/SEARCH f USING INDEX sqlite_autoindex_ipo_facts_1 \(accession=\?\)/);
    expect(plan("ipo_facts")).toMatch(/SEARCH g USING INDEX sqlite_autoindex_ipo_filings_1 \(accession=\?\)/);
    expect(plan("ipo_filings")).toMatch(/SEARCH ipo_filings USING INDEX idx_ipo_filings_slug \(slug=\?\)/);
    expect(plan("ipo_items")).toMatch(/SEARCH ipo_items USING INDEX idx_ipo_items_slug \(slug=\? AND kind=\?\)/);
    expect(plan("ipo_trading")).toMatch(/SEARCH ipo_trading USING (COVERING )?INDEX sqlite_autoindex_ipo_trading_1 \(slug=\?/);
    // The write-ups: a range on posts' unique run_key, then each one's translation by its primary key.
    expect(plan("posts")).toMatch(/SEARCH p USING INDEX sqlite_autoindex_posts_\d \(run_key>\? AND run_key<\?\)/);
    for (const p of plans) expect(p.plan, p.sql).not.toMatch(/SCAN (ipo_\w+|f|g|p|t|posts|post_translations)\b/);
  });

  it("the home page's IPO section is one read: the hand-kept watch list, every detail by key or index", async () => {
    const { db, plans } = planning(database(analyze, IPO_SEED));
    await listHomeIpos(db, "en", "2026-11-11");
    expect(plans).toHaveLength(1);
    const plan = plans[0].plan;
    expect(plan).toMatch(/SEARCH ipo_trading USING COVERING INDEX sqlite_autoindex_ipo_trading_1 \(slug=\?\)/);
    expect(plan).toMatch(/SEARCH g USING INDEX sqlite_autoindex_ipo_filings_1 \(accession=\?\)/);
    expect(plan).toMatch(/SEARCH ipo_facts USING INDEX sqlite_autoindex_ipo_facts_1 \(accession=\? AND kind=\?\)/);
    expect(plan).toMatch(/SEARCH ipo_items USING INDEX idx_ipo_items_slug \(slug=\? AND kind=\?\)/);
    expect(plan).toMatch(/SEARCH p USING INDEX sqlite_autoindex_posts_\d \(run_key>\? AND run_key<\?\)/);
    // Only the watch list itself is read whole: the hand-kept list of tracked IPOs, capped at 20 rows.
    expect(plan.match(/SCAN \w+/g) ?? []).toEqual(["SCAN ipo_watch"]);
  });

  it("the sitemap's IPO read is capped", async () => {
    const { db, plans } = planning(database(analyze, IPO_SEED));
    await listIpoSlugs(db);
    expect(plans).toHaveLength(1);
    expect(plans[0].sql).toMatch(/LIMIT 200$/);
  });
});
