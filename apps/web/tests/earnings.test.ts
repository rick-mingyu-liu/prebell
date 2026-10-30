import { describe, expect, it } from "vitest";
import {
  chartResponse,
  getPostCharts,
  isDate,
  listCalendarWeek,
  listLatestEarnings,
  listRecentBriefs,
  listTickerCalendar,
  listTickerPosts,
  normalizeTicker,
  stripDay,
  tickerKnown,
  todayEt,
  weekStart,
} from "../src/lib/earnings";
import { memoryD1 } from "./d1";

const ACC = "0000320193-26-000070";

/** One earnings post for a ticker; `zh` is the ZH translation status or null. */
function earnings(id: number, ticker: string, acc: string, date: string, zh: string | null): string {
  const key = `earnings:${ticker}:${acc}`;
  const slug = `${ticker.toLowerCase()}-${date}-earnings`;
  const ts = `${date}T21:00:00Z`;
  let sql = `INSERT INTO runs VALUES ('${key}','earnings','${date}','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (${id},'${key}','earnings','earnings:v1','${date}','${slug}','x','${ts}');
INSERT INTO earnings_filings VALUES ('${acc}',1,'${ticker}','8-K','2.02','${date}','${date}','${ts}','x.htm','${key}','done','${ts}');
INSERT INTO post_translations VALUES (${id},'en','published','EN ${ticker}','sum','# EN','${ts}');\n`;
  if (zh) sql += `INSERT INTO post_translations VALUES (${id},'zh','${zh}','中文 ${ticker}','摘要','# 中文','${ts}');\n`;
  return sql;
}

function cal(ticker: string, date: string, status: string, slot = "amc"): string {
  return `INSERT INTO earnings_calendar VALUES ('${ticker}','${date}','${slot}','${status}','yahoo',NULL,NULL,NULL,'t','t',NULL,'t');\n`;
}

const seed =
  earnings(1, "AAPL", ACC, "2026-10-30", "published") +
  earnings(2, "MSFT", "0000789019-26-000050", "2026-10-28", "pending") +
  cal("AAPL", "2026-10-30", "reported") +
  cal("MSFT", "2026-10-28", "reported") +
  cal("XOM", "2026-10-30", "unconfirmed", "bmo") +
  cal("CVX", "2026-10-30", "confirmed", "bmo") +
  cal("TSLA", "2026-10-29", "quarantined") +
  cal("AMZN", "2026-11-02", "unconfirmed") +
  `INSERT INTO post_charts VALUES (1,'revenue','charts/earnings/AAPL/${ACC}/revenue.svg','Apple revenue','苹果营收',0);
INSERT INTO post_charts VALUES (1,'eps','charts/earnings/AAPL/${ACC}/eps.svg','Apple EPS','苹果每股收益',1);
INSERT INTO post_charts VALUES (1,'bad','https://evil.test/x.svg','x','x',2);`;

describe("dates", () => {
  it("weekStart is the Monday", () => {
    expect(weekStart("2026-10-29")).toBe("2026-10-26");
    expect(weekStart("2026-10-26")).toBe("2026-10-26");
    expect(weekStart("2026-11-01")).toBe("2026-10-26"); // Sunday
    expect(weekStart("2027-01-01")).toBe("2026-12-28");
  });
  it("isDate", () => {
    expect(isDate("2026-10-29")).toBe(true);
    expect(isDate("2026-02-30")).toBe(false);
    expect(isDate("2026-10-29x")).toBe(false);
    expect(isDate(undefined)).toBe(false);
  });
  it("todayEt uses US Eastern", () => {
    expect(todayEt(new Date("2026-10-30T03:00:00Z"))).toBe("2026-10-29");
  });
  it("normalizeTicker", () => {
    expect(normalizeTicker("brk.b")).toBe("BRK.B");
    expect(normalizeTicker("aapl")).toBe("AAPL");
    expect(normalizeTicker("../x")).toBeNull();
    expect(normalizeTicker("<script>")).toBeNull();
    expect(normalizeTicker("%")).toBeNull();
    expect(normalizeTicker("%41")).toBeNull();
  });
});

describe("listCalendarWeek", () => {
  it("hides quarantined dates and links reported ones to their post", async () => {
    const days = await listCalendarWeek(memoryD1(seed), "2026-10-26", "en");
    expect(days.map((d) => d.date)).toEqual(["2026-10-26", "2026-10-27", "2026-10-28", "2026-10-29", "2026-10-30"]);
    expect(days[3].entries).toEqual([]); // TSLA is quarantined
    expect(days[4].entries.map((e) => [e.ticker, e.status, e.slot, e.postSlug])).toEqual([
      ["CVX", "confirmed", "bmo", null],
      ["XOM", "unconfirmed", "bmo", null],
      ["AAPL", "reported", "amc", "aapl-2026-10-30-earnings"],
    ]);
    expect(days[2].entries[0].postSlug).toBe("msft-2026-10-28-earnings");
  });

  it("in Chinese, links pending posts (the page shows the notice) but not failed ones", async () => {
    const db = memoryD1(seed + earnings(3, "NVDA", "0001045810-26-000010", "2026-10-29", "failed") + cal("NVDA", "2026-10-29", "reported"));
    const days = await listCalendarWeek(db, "2026-10-26", "zh");
    expect(days[2].entries[0].postSlug).toBe("msft-2026-10-28-earnings");
    expect(days[3].entries.map((e) => [e.ticker, e.postSlug])).toEqual([["NVDA", null]]);
  });
});

describe("ticker archive", () => {
  it("lists only that ticker's published posts in the language", async () => {
    const db = memoryD1(seed);
    expect((await listTickerPosts(db, "AAPL", "zh")).map((p) => p.slug)).toEqual(["aapl-2026-10-30-earnings"]);
    expect(await listTickerPosts(db, "MSFT", "zh")).toEqual([]);
    expect((await listTickerPosts(db, "MSFT", "en"))[0].title).toBe("EN MSFT");
    expect((await listTickerCalendar(db, "AAPL", "zh"))[0].status).toBe("reported");
  });
  it("knows which tickers exist", async () => {
    const db = memoryD1(seed);
    expect(await tickerKnown(db, "AAPL")).toBe(true);
    expect(await tickerKnown(db, "AMZN")).toBe(true);
    expect(await tickerKnown(db, "TSLA")).toBe(false);
    expect(await tickerKnown(db, "ZZZ")).toBe(false);
  });
  it("recent briefs leave earnings out, however many there are", async () => {
    let many = seed;
    for (let i = 0; i < 25; i++) many += earnings(100 + i, `T${i}`, `0000000001-26-${String(i).padStart(6, "0")}`, "2026-10-31", "published");
    many += `INSERT INTO runs VALUES ('premarket:2026-10-27','premarket','2026-10-27','published',1,NULL,'t','t');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (500,'premarket:2026-10-27','premarket','premarket:v1','2026-10-27','premarket-2026-10-27',NULL,'2026-10-27T12:34:00Z');
INSERT INTO post_translations VALUES (500,'en','published','Brief','s','# b','t');`;
    expect((await listRecentBriefs(memoryD1(many), "en", 10)).map((p) => p.slug)).toEqual(["premarket-2026-10-27"]);
  });

  it("latest earnings, newest first", async () => {
    const db = memoryD1(seed);
    expect((await listLatestEarnings(db, "en", 5)).map((p) => p.slug)).toEqual([
      "aapl-2026-10-30-earnings",
      "msft-2026-10-28-earnings",
    ]);
  });
});

describe("charts", () => {
  it("maps keys to same-site paths and drops anything else", async () => {
    const charts = await getPostCharts(memoryD1(seed), "aapl-2026-10-30-earnings", "zh");
    expect(charts).toEqual([
      { name: "revenue", src: `/charts/earnings/AAPL/${ACC}/revenue.svg`, alt: "苹果营收" },
      { name: "eps", src: `/charts/earnings/AAPL/${ACC}/eps.svg`, alt: "苹果每股收益" },
    ]);
  });

  it("serves only chart keys, as locked-down SVG", async () => {
    const seen: string[] = [];
    const bucket = {
      async get(key: string) {
        seen.push(key);
        return key.endsWith("revenue.svg") ? { body: "<svg/>" } : null;
      },
    };
    const ok = await chartResponse(bucket, ["earnings", "AAPL", ACC, "revenue.svg"]);
    expect(ok.status).toBe(200);
    expect(ok.headers.get("content-type")).toBe("image/svg+xml; charset=utf-8");
    expect(ok.headers.get("content-security-policy")).toContain("default-src 'none'");
    expect(await ok.text()).toBe("<svg/>");
    expect(seen).toEqual([`charts/earnings/AAPL/${ACC}/revenue.svg`]);
    for (const bad of [
      ["earnings", "AAPL", ACC, "eps.svg"], // not stored
      ["..", "earnings", "AAPL", ACC, "revenue.svg"],
      ["earnings", "AAPL", ACC, "x.html"],
      ["premarket", "2026-10-29.json"],
    ]) {
      expect((await chartResponse(bucket, bad)).status).toBe(404);
    }
    expect(seen).toHaveLength(2);
  });
});

describe("stripDay", () => {
  it("a weekend start date lands on Monday's entries", async () => {
    const day = await stripDay(memoryD1(seed), "2026-10-31", { exact: false, lang: "en" });
    expect(day?.date).toBe("2026-11-02");
    expect(day?.amc.map((e) => e.ticker)).toEqual(["AMZN"]);
  });
  it("an exact date without entries is null, even when a later date has some", async () => {
    const db = memoryD1(seed);
    expect(await stripDay(db, "2026-10-31", { exact: true, lang: "en" })).toBeNull();
    expect(await stripDay(db, "2026-10-29", { exact: true, lang: "en" })).toBeNull(); // only a quarantined row
  });
  it("looks at most 7 days ahead", async () => {
    const db = memoryD1(seed);
    expect((await stripDay(db, "2026-10-26", { exact: false, lang: "en" }))?.date).toBe("2026-10-28");
    expect(await stripDay(db, "2026-11-03", { exact: false, lang: "en" })).toBeNull();
    expect((await stripDay(memoryD1(cal("KO", "2026-11-09", "confirmed")), "2026-11-02", { exact: false, lang: "en" }))?.date).toBe("2026-11-09");
    expect(await stripDay(memoryD1(cal("KO", "2026-11-10", "confirmed")), "2026-11-02", { exact: false, lang: "en" })).toBeNull();
  });
  it("groups by slot in order and carries the post slug of reported entries", async () => {
    const day = await stripDay(memoryD1(seed), "2026-10-30", { exact: true, lang: "en" });
    expect(day?.bmo.map((e) => [e.ticker, e.postSlug])).toEqual([
      ["CVX", null],
      ["XOM", null],
    ]);
    expect(day?.amc.map((e) => [e.ticker, e.postSlug])).toEqual([["AAPL", "aapl-2026-10-30-earnings"]]);
    expect(day?.unknown).toEqual([]);
  });
  it("uses the entry's own date as the day", async () => {
    const day = await stripDay(memoryD1(seed), "2026-10-27", { exact: false, lang: "zh" });
    expect(day?.date).toBe("2026-10-28");
    expect(day?.amc[0].postSlug).toBe("msft-2026-10-28-earnings"); // pending ZH still links
  });
});
