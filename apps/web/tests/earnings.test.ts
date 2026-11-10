import { describe, expect, it } from "vitest";
import {
  addDays,
  chartResponse,
  getPostCharts,
  isDate,
  listCalendarRange,
  listCalendarWeek,
  listLatestEarnings,
  listRecentBriefs,
  listTickerCalendar,
  listTickerPosts,
  normalizeTicker,
  stripDay,
  tickerHasCheck,
  tickerKnown,
  todayEt,
  weekStart,
} from "../src/lib/earnings";
import type { D1Like } from "../src/lib/posts";
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
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'en','published','EN ${ticker}','sum','# EN','${ts}');\n`;
  if (zh) sql += `INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (${id},'zh','${zh}','中文 ${ticker}','摘要','# 中文','${ts}');\n`;
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

describe("listCalendarRange", () => {
  it("reads any date range, hides quarantined dates and keeps the week's order", async () => {
    const rows = await listCalendarRange(memoryD1(seed), "2026-10-28", "2026-11-02", "en");
    expect(rows.map((e) => [e.date, e.ticker, e.postSlug])).toEqual([
      ["2026-10-28", "MSFT", "msft-2026-10-28-earnings"],
      ["2026-10-30", "CVX", null],
      ["2026-10-30", "XOM", null],
      ["2026-10-30", "AAPL", "aapl-2026-10-30-earnings"],
      ["2026-11-02", "AMZN", null],
    ]);
  });

  it("filters by ticker", async () => {
    const rows = await listCalendarRange(memoryD1(seed), "2026-10-01", "2026-11-30", "zh", ["AAPL", "AMZN", "TSLA"]);
    expect(rows.map((e) => e.ticker)).toEqual(["AAPL", "AMZN"]);
    await expect(listCalendarRange(memoryD1(seed), "2026-10-01", "2026-10-02", "en", Array(51).fill("A"))).rejects.toThrow();
  });
});

describe("calendar rows at S&P 500 size", () => {
  const SEC = "https://www.sec.gov/Archives/edgar/data/1090872/000109087226000041/0001090872-26-000041-index.htm";
  const row = (ticker: string, status: string, source: string, proof: string | null) =>
    `INSERT INTO earnings_calendar VALUES ('${ticker}','2026-11-02','bmo','${status}','${source}',${proof ? `'${proof}'` : "NULL"},NULL,NULL,'t','t',NULL,'t');\n`;
  const db = () =>
    memoryD1(
      row("A", "reported", "edgar", SEC) +
        row("ACGL", "reported", "edgar", "https://evil.test/0001090872-26-000041-index.htm") +
        row("MCK", "unconfirmed", "yahoo", null) +
        row("ZZZQ", "confirmed", "ir", SEC),
    );

  it("names each company, puts write-up companies first and links calendar-only reports to the 8-K", async () => {
    const days = await listCalendarWeek(db(), "2026-11-02", "en");
    expect(days[0].entries.map((e) => [e.ticker, e.name, e.writeUps, e.filingUrl])).toEqual([
      ["MCK", "McKesson", true, null],
      ["A", "Agilent Technologies", false, SEC],
      // Only EDGAR filing indexes are linked.
      ["ACGL", "Arch Capital Group", false, null],
      // A ticker off the watchlist keeps its row, without a name; only reported rows link a filing.
      ["ZZZQ", null, false, null],
    ]);
    const strip = await stripDay(db(), "2026-11-02", { exact: true, lang: "zh" });
    expect(strip?.bmo.map((e) => e.ticker)).toEqual(["MCK", "A", "ACGL", "ZZZQ"]);
    expect(strip?.bmo[1].name).toBe("Agilent Technologies"); // English names on ZH pages (v1)
  });

  it("the week reads only that week's posts", async () => {
    const base = memoryD1(seed);
    const seen: { sql: string; args: unknown[] }[] = [];
    const spy: D1Like = {
      prepare(sql) {
        const st = base.prepare(sql);
        const bind = st.bind.bind(st);
        st.bind = (...args) => {
          seen.push({ sql, args });
          return bind(...args);
        };
        return st;
      },
    };
    await listCalendarWeek(spy, "2026-10-26", "en");
    const q = seen[0];
    expect(q.sql).toContain("WHERE p.trading_date BETWEEN ? AND ?");
    expect(q.args).toEqual(["en", "2026-10-26", "2026-10-30", "2026-10-26", "2026-10-30"]);
    const { results } = await base.prepare(`EXPLAIN QUERY PLAN ${q.sql}`).bind(...q.args).all<{ detail: string }>();
    expect(results.map((r) => r.detail).join("\n")).toContain("idx_posts_trading_date");
    // A ticker's dates read that ticker's filings only.
    seen.length = 0;
    await listTickerCalendar(spy, "AAPL", "en");
    const plan = await base.prepare(`EXPLAIN QUERY PLAN ${seen[0].sql}`).bind(...seen[0].args).all<{ detail: string }>();
    expect(plan.results.map((r) => r.detail).join("\n")).toContain("idx_earnings_filings_ticker");
  });

  it("knows which tickers have a check", async () => {
    const d = memoryD1(seed + earnings(3, "NVDA", "0001045810-26-000010", "2026-10-29", "failed"));
    expect(await tickerHasCheck(d, "AAPL")).toBe(true);
    expect(await tickerHasCheck(d, "NVDA")).toBe(true);
    expect(await tickerHasCheck(d, "XOM")).toBe(false);
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
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (500,'en','published','Brief','s','# b','t');`;
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

describe("cover fields in lists", () => {
  it("latest earnings carry ticker and cover fields", async () => {
    const ts = "2026-10-30T21:00:00Z";
    const db = memoryD1(
      seed +
        `UPDATE posts SET cover_key = 'covers/aapl-2026-10-30-earnings.png' WHERE id = 1;
UPDATE post_translations SET cover_line1 = 'Apple beats', cover_line2 = 'on services' WHERE post_id = 1 AND lang = 'en';
INSERT INTO runs VALUES ('premarket:2026-10-31','premarket','2026-10-31','published',1,NULL,'${ts}','${ts}');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
VALUES (9,'premarket:2026-10-31','premarket','premarket:v1','2026-10-31','premarket-2026-10-31',NULL,'${ts}');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (9,'en','published','EN pm','s','# x','${ts}');`,
    );
    const [aapl] = await listLatestEarnings(db, "en", 5);
    expect(aapl).toMatchObject({
      slug: "aapl-2026-10-30-earnings",
      ticker: "AAPL",
      coverKey: "covers/aapl-2026-10-30-earnings.png",
      coverLines: ["Apple beats", "on services"],
    });
    const [msft] = (await listLatestEarnings(db, "en", 5)).filter((p) => p.ticker === "MSFT");
    expect(msft).toMatchObject({ coverKey: null, coverLines: null });
    expect((await listTickerPosts(db, "AAPL", "en"))[0]).toMatchObject({ ticker: "AAPL" });
    const [brief] = await listRecentBriefs(db, "en", 1);
    expect(brief).toMatchObject({ slug: "premarket-2026-10-31", ticker: null, coverKey: null, coverLines: null });
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
  it("other dates and other days' posts do not affect the result", async () => {
    const extras =
      cal("OLD", "2026-10-26", "confirmed") +
      cal("FAR", "2026-11-20", "confirmed") +
      cal("TSLA", "2026-11-02", "quarantined") +
      earnings(3, "NVDA", "0001045810-26-000010", "2026-11-03", "published") +
      cal("NVDA", "2026-11-03", "reported");
    const opts = { exact: false, lang: "en" } as const;
    const plain = await stripDay(memoryD1(seed), "2026-10-31", opts);
    expect(await stripDay(memoryD1(seed + extras), "2026-10-31", opts)).toEqual(plain);
  });
  it("in Chinese, a reported entry with no ZH row links no post", async () => {
    const db = memoryD1(earnings(3, "NVDA", "0001045810-26-000010", "2026-11-03", null) + cal("NVDA", "2026-11-03", "reported"));
    const day = await stripDay(db, "2026-11-03", { exact: true, lang: "zh" });
    expect(day?.amc.map((e) => [e.ticker, e.postSlug])).toEqual([["NVDA", null]]);
    expect((await stripDay(db, "2026-11-03", { exact: true, lang: "en" }))?.amc[0].postSlug).toBe("nvda-2026-11-03-earnings");
  });
  it("reads one day's posts through the trading_date index", async () => {
    const db = memoryD1(seed);
    const seen: { sql: string; args: unknown[] }[] = [];
    const spy: D1Like = {
      prepare(sql) {
        const st = db.prepare(sql);
        const bind = st.bind.bind(st);
        st.bind = (...args) => {
          seen.push({ sql, args });
          return bind(...args);
        };
        return st;
      },
    };
    await stripDay(spy, "2026-10-30", { exact: true, lang: "en" });
    const entries = seen.find((q) => q.sql.includes("post_translations"));
    expect(entries).toBeDefined();
    const { results } = await db
      .prepare(`EXPLAIN QUERY PLAN ${entries!.sql}`)
      .bind(...entries!.args)
      .all<{ detail: string }>();
    expect(results.map((r) => r.detail).join("\n")).toContain("idx_posts_trading_date");
  });
});

describe("date helpers stay in four-digit years", () => {
  it("isDate refuses years a caller could step out of", () => {
    expect(isDate("9998-12-31")).toBe(true);
    expect(isDate("9999-12-31")).toBe(false);
    expect(isDate("1000-01-01")).toBe(true);
    expect(isDate("0999-12-31")).toBe(false);
  });

  it("addDays throws rather than return a date that sorts wrongly", () => {
    expect(addDays("9998-12-31", 1)).toBe("9999-01-01");
    expect(() => addDays("9999-12-31", 1)).toThrow(RangeError);
    expect(() => addDays("0000-01-01", -1)).toThrow(RangeError);
  });
});
