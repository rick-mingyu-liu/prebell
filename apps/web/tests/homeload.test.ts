import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { loadHome } from "../src/lib/home";
import type { R2Like } from "../src/lib/facts";
import { memoryD1 } from "./d1";

const fixtureText = (name: string) => readFileSync(new URL(`./fixtures/${name}`, import.meta.url), "utf8");

/** A published EN+ZH post; `factsKey` null stores none. */
function post(id: number, type: string, date: string, publishedAt: string, factsKey: string | null): string {
  const key = `${type}:${date}`;
  return `INSERT INTO runs VALUES ('${key}','${type}','${date}','published',1,NULL,'${publishedAt}','${publishedAt}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (${id},'${key}','${type}','${type}:v1','${date}','${type}-${date}',${factsKey ? `'${factsKey}'` : "NULL"},'${publishedAt}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'en','published','EN ${type} ${date}','Summary','# Body','${publishedAt}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'zh','published','中文 ${type} ${date}','摘要','# 正文','${publishedAt}');\n`;
}

/** A published EN earnings check for `ticker`, published at 21:00Z on `date`; slug `<ticker>-<date>-earnings`. */
function check(id: number, ticker: string, date: string): string {
  const key = `earnings:${ticker}:${id}`;
  const ts = `${date}T21:00:00Z`;
  return `INSERT INTO runs VALUES ('${key}','earnings','${date}','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (${id},'${key}','earnings','earnings:v1','${date}','${ticker.toLowerCase()}-${date}-earnings','x','${ts}');
INSERT INTO earnings_filings VALUES ('acc-${id}',1,'${ticker}','8-K','2.02','${date}','${date}','${ts}','x.htm','${key}','done','${ts}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'en','published','EN ${ticker}','sum','# EN','${ts}');\n`;
}

function cal(ticker: string, date: string, slot: string): string {
  return `INSERT INTO earnings_calendar VALUES ('${ticker}','${date}','${slot}','confirmed','yahoo',NULL,NULL,NULL,'t','t',NULL,'t');\n`;
}

const seed =
  post(1, "premarket", "2026-10-30", "2026-10-30T13:20:43Z", "premarket/2026-10-30.json") +
  post(2, "recap", "2026-10-29", "2026-10-29T20:45:00Z", "recap/2026-10-29.json") +
  post(3, "premarket", "2026-10-29", "2026-10-29T12:34:00Z", "premarket/2026-10-29.json");

/** The recap of 2026-10-30, published after that day's premarket. */
const laterRecap = post(4, "recap", "2026-10-30", "2026-10-30T20:45:00Z", "recap/2026-10-29.json");

/** An R2 bucket holding the fixtures that records every key it is asked for. */
function fakeR2() {
  const objects: Record<string, string> = {
    "premarket/2026-10-30.json": fixtureText("premarket-2026-10-30.json"),
    "recap/2026-10-29.json": fixtureText("recap-sample.json"),
  };
  const gets: string[] = [];
  const bucket: R2Like = {
    async get(key) {
      gets.push(key);
      const text = objects[key];
      return text === undefined ? null : { text: async () => text };
    },
  };
  return { bucket, gets };
}

const TODAY = "2026-10-30"; // a Friday

describe("loadHome", () => {
  it("leads with the later-published brief and reads only its facts", async () => {
    const pm = fakeR2();
    const home = await loadHome(memoryD1(seed), pm.bucket, "en", TODAY);
    expect(home.lead?.slug).toBe("premarket-2026-10-30");
    expect(pm.gets).toEqual(["premarket/2026-10-30.json"]);
    expect(home.leadFacts?.kind).toBe("premarket");
    expect(home.leadFacts?.tradingDate).toBe("2026-10-30");

    const rc = fakeR2();
    const later = await loadHome(memoryD1(seed + laterRecap), rc.bucket, "zh", TODAY);
    expect(later.lead?.slug).toBe("recap-2026-10-30");
    expect(rc.gets).toEqual(["recap/2026-10-29.json"]);
    expect(later.leadFacts?.kind).toBe("recap");
    expect(later.leadFacts?.lead.symbol).toBe("^GSPC");

    // Published at the same moment: the premarket wins.
    const tie = post(4, "recap", "2026-10-30", "2026-10-30T13:20:43Z", "recap/2026-10-29.json");
    const tied = await loadHome(memoryD1(seed + tie), fakeR2().bucket, "en", TODAY);
    expect(tied.lead?.slug).toBe("premarket-2026-10-30");
  });

  it("makes no R2 read without a post or without a facts key", async () => {
    const none = fakeR2();
    const empty = await loadHome(memoryD1(""), none.bucket, "en", TODAY);
    expect(empty).toEqual({ lead: null, leadFacts: null, earnings: [], recent: [], reporting: null });
    expect(none.gets).toEqual([]);

    const keyless = fakeR2();
    const home = await loadHome(
      memoryD1(post(1, "premarket", "2026-10-30", "2026-10-30T13:20:43Z", null)),
      keyless.bucket,
      "en",
      TODAY,
    );
    expect(home.lead?.slug).toBe("premarket-2026-10-30");
    expect(home.leadFacts).toBeNull();
    expect(keyless.gets).toEqual([]);
  });

  it("reports a facts failure with the lead's key", async () => {
    const errors: (string | null)[] = [];
    const bucket: R2Like = {
      async get() {
        return { text: async () => "{not json" };
      },
    };
    const home = await loadHome(memoryD1(seed), bucket, "en", TODAY, (_e, key) => errors.push(key));
    expect(home.leadFacts).toBeNull();
    expect(errors).toEqual(["premarket/2026-10-30.json"]);
  });

  it("reporting comes from tomorrow for a recap lead and today for a premarket lead", async () => {
    const calendar = cal("KO", "2026-10-30", "bmo") + cal("AAPL", "2026-11-02", "amc") + cal("MCD", "2026-11-02", "bmo");

    const premarket = await loadHome(memoryD1(seed + calendar), fakeR2().bucket, "en", TODAY);
    expect(premarket.reporting?.date).toBe("2026-10-30");
    expect(premarket.reporting?.bmo.map((e) => e.ticker)).toEqual(["KO"]);

    // From Saturday, non-exact: the next day with reports is Monday.
    const recap = await loadHome(memoryD1(seed + laterRecap + calendar), fakeR2().bucket, "en", TODAY);
    expect(recap.reporting?.date).toBe("2026-11-02");
    expect(recap.reporting?.bmo.map((e) => e.ticker)).toEqual(["MCD"]);
    expect(recap.reporting?.amc.map((e) => e.ticker)).toEqual(["AAPL"]);

    const quiet = await loadHome(memoryD1(seed), fakeR2().bucket, "en", TODAY);
    expect(quiet.reporting).toBeNull();
  });

  it("recent excludes the lead and is capped at eight", async () => {
    // A week of briefs before the lead, and five earnings checks.
    let sql = post(1, "premarket", "2026-10-30", "2026-10-30T13:20:43Z", "premarket/2026-10-30.json");
    let id = 10;
    for (let day = 23; day <= 29; day++) {
      sql += post(id++, "premarket", `2026-10-${day}`, `2026-10-${day}T12:30:00Z`, null);
      sql += post(id++, "recap", `2026-10-${day}`, `2026-10-${day}T20:45:00Z`, null);
    }
    sql += check(50, "AAPL", "2026-10-29") + check(51, "AMZN", "2026-10-28") + check(52, "MSFT", "2026-10-27");
    sql += check(53, "META", "2026-10-26") + check(54, "KO", "2026-10-25");

    const home = await loadHome(memoryD1(sql), fakeR2().bucket, "en", TODAY);
    expect(home.lead?.slug).toBe("premarket-2026-10-30");
    expect(home.earnings.map((p) => p.slug)).toEqual([
      "aapl-2026-10-29-earnings",
      "amzn-2026-10-28-earnings",
      "msft-2026-10-27-earnings",
    ]);
    // Newest first by publish time; checks beyond the three shown join the briefs.
    expect(home.recent.map((p) => p.slug)).toEqual([
      "recap-2026-10-29",
      "premarket-2026-10-29",
      "recap-2026-10-28",
      "premarket-2026-10-28",
      "recap-2026-10-27",
      "premarket-2026-10-27",
      "meta-2026-10-26-earnings",
      "recap-2026-10-26",
    ]);
  });
});
