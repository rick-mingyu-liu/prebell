import { describe, expect, it } from "vitest";
import { ArgumentError, NOTICE, parseWatchList, prepareCall, TOOLS, type ToolContext, type ToolName } from "../src/lib/mcptools";
import { memoryD1 } from "./d1";
import { schemaErrors } from "./jsonschema";
import {
  AAPL_KEY,
  bucket,
  cal,
  check,
  counted,
  fixture,
  FRIDAY,
  macro,
  memoryCache,
  objects,
  post,
  premarketFacts,
  seed,
} from "./mcpseed";

type Result = Record<string, unknown> & { [k: string]: unknown };

function ctx(over: Partial<ToolContext> & { sql?: string; objs?: Record<string, string | object> } = {}) {
  const c = counted(memoryD1(over.sql ?? seed()));
  const b = bucket(over.objs ?? objects());
  const errors: unknown[] = [];
  const context: ToolContext = {
    db: c.db,
    bucket: b,
    origin: new URL("https://prebell.site"),
    now: FRIDAY,
    onError: (e) => errors.push(e),
    ...over,
  };
  return { context, reads: c.sql, gets: b.gets, errors };
}

/** Runs a tool and checks its result against the tool's own output schema (which types it), then hands it back loosely typed for the assertions. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function run(name: ToolName, args: Record<string, unknown>, c = ctx()): Promise<any> {
  const out = (await prepareCall(name, args, c.context).run()) as Result;
  const schema = TOOLS.find((t) => t.name === name)!.outputSchema as unknown as Record<string, unknown>;
  expect(schemaErrors(schema, out)).toEqual([]);
  expect(out.notice).toBe(NOTICE[(args.lang as "en" | "zh") ?? "en"]);
  expect(String(out.source)).toMatch(/^https:\/\/prebell\.site\//);
  expect(out.as_of !== undefined || out.published_at !== undefined).toBe(true);
  return out;
}

const DROP_FRIDAY_BRIEF = `DELETE FROM post_translations WHERE post_id = (SELECT id FROM posts WHERE slug = 'premarket-2026-10-30');
DELETE FROM posts WHERE slug = 'premarket-2026-10-30';`;

const bad = (name: ToolName, args: Record<string, unknown>) => () => prepareCall(name, args, ctx().context);

describe("tool definitions", () => {
  it("every schema is an object schema the check helper accepts", () => {
    for (const t of TOOLS) {
      expect(t.inputSchema.type).toBe("object");
      expect(t.outputSchema.type).toBe("object");
      expect(t.description.length).toBeGreaterThan(100);
    }
  });
});

describe("earnings_calendar", () => {
  it("defaults to the current week, with companies, statuses, links and events", async () => {
    const c = ctx();
    const out = await run("earnings_calendar", {}, c);
    expect([out.from, out.to]).toEqual(["2026-10-26", "2026-10-30"]);
    expect(out.source).toBe("https://prebell.site/en/earnings?week=2026-10-26");
    expect(out.days.map((d: { date: string }) => d.date)).toEqual(["2026-10-26", "2026-10-27", "2026-10-28", "2026-10-29", "2026-10-30"]);
    const fri = out.days[4];
    expect(fri.weekday).toBe("Fri");
    expect(fri.companies).toEqual([
      {
        ticker: "XOM",
        name: "ExxonMobil",
        date: "2026-10-30",
        slot: "before_open",
        status: "unconfirmed",
        writes_check: true,
        check_url: null,
        filing_url: null,
      },
      {
        ticker: "AAPL",
        name: "Apple",
        date: "2026-10-30",
        slot: "after_close",
        status: "reported",
        writes_check: true,
        check_url: "https://prebell.site/en/p/aapl-q4-fy2026-earnings",
        filing_url: "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm",
      },
    ]);
    // Quarantined TSLA is hidden; a calendar-only company is marked so.
    expect(out.days[3].companies).toEqual([]);
    expect(out.days[1].companies[0]).toMatchObject({ ticker: "A", writes_check: false, status: "reported" });
    expect(out.days[2].events).toEqual([
      {
        title: "FOMC rate decision",
        note: "Press conference 2:30 PM ET",
        date: "2026-10-28",
        starts_at: "2026-10-28T18:00:00Z",
        time_et: "2:00 PM ET",
        status: "scheduled",
        importance: "major",
        official_url: "https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm",
        release_url: null,
        moved_from: null,
      },
    ]);
    expect(out.tickers_without_dates).toBeUndefined();
    // Two D1 reads (calendar, events), no R2.
    expect(c.reads).toHaveLength(2);
    expect(c.gets).toHaveLength(0);
  });

  it("in Chinese: event titles, weekdays and check links in Chinese; company names stay English", async () => {
    const out = await run("earnings_calendar", { lang: "zh" });
    expect(out.days[4].weekday).toBe("周五");
    expect(out.days[2].events[0].title).toBe("联邦公开市场委员会（FOMC）议息会议");
    expect(out.days[2].events[0].time_et).toBe("下午2:00（美东）");
    expect(out.days[4].companies[1].check_url).toBe("https://prebell.site/zh/p/aapl-q4-fy2026-earnings");
    expect(out.days[4].companies[1].name).toBe("Apple");
    // MSFT's ZH text is pending: the page shows the notice, so the link stands.
    expect(out.days[2].companies[0].check_url).toBe("https://prebell.site/zh/p/msft-q1-fy2027-earnings");
    expect(out.source).toBe("https://prebell.site/zh/earnings?week=2026-10-26");
  });

  it("takes a range and a ticker list, and names tickers with no date in it", async () => {
    const out = await run("earnings_calendar", { from: "2026-10-30", to: "2026-11-19", tickers: ["ko", "NVDA", "brk.b", "KO"] });
    const rows = out.days.flatMap((d: { companies: { ticker: string; date: string }[] }) => d.companies.map((x) => [x.ticker, x.date]));
    expect(rows).toEqual([
      ["KO", "2026-11-03"],
      ["NVDA", "2026-11-18"],
    ]);
    expect(out.tickers_without_dates).toEqual(["BRK.B"]);
    // Weekends are left out unless something is on them.
    expect(out.days.some((d: { date: string }) => d.date === "2026-10-31")).toBe(false);
    expect(out.days).toHaveLength(15);
  });

  it("from alone means a week from it; to alone a week up to it", async () => {
    const a = await run("earnings_calendar", { from: "2026-11-02" });
    expect([a.from, a.to]).toEqual(["2026-11-02", "2026-11-08"]);
    const b = await run("earnings_calendar", { to: "2026-11-08" });
    expect([b.from, b.to]).toEqual(["2026-11-02", "2026-11-08"]);
  });

  it("refuses bad dates, long ranges, too many tickers, bad tickers and unknown arguments", () => {
    expect(bad("earnings_calendar", { from: "2026-02-30" })).toThrow(ArgumentError);
    expect(bad("earnings_calendar", { from: "next week" })).toThrow(/YYYY-MM-DD/);
    expect(bad("earnings_calendar", { from: "2026-11-10", to: "2026-11-01" })).toThrow(/before/);
    expect(bad("earnings_calendar", { from: "2026-11-01", to: "2026-11-22" })).toThrow(/at most 21 days/);
    expect(bad("earnings_calendar", { from: "2026-11-01", to: "2026-11-21" })).not.toThrow();
    expect(bad("earnings_calendar", { tickers: Array.from({ length: 51 }, (_, i) => `T${i}`) })).toThrow(/At most 50/);
    expect(bad("earnings_calendar", { tickers: Array.from({ length: 50 }, (_, i) => `T${i}`) })).not.toThrow();
    expect(bad("earnings_calendar", { tickers: "AAPL" })).toThrow(/list/);
    expect(bad("earnings_calendar", { tickers: ["AAPL; DROP TABLE posts"] })).toThrow(/Not a ticker/);
    expect(bad("earnings_calendar", { lang: "fr" })).toThrow(/lang/);
    expect(bad("earnings_calendar", { week: "2026-11-02" })).toThrow(/Unknown argument: week/);
  });
});

describe("earnings_report", () => {
  it("a published check: figures with quotes, period, links, guidance; no share move in an old bundle", async () => {
    const c = ctx();
    const out = await run("earnings_report", { ticker: "aapl" }, c);
    expect(out).toMatchObject({
      status: "check_published",
      ticker: "AAPL",
      company: "Apple",
      writes_check: true,
      title: "EN title aapl-q4-fy2026-earnings",
      summary: "EN summary aapl-q4-fy2026-earnings",
      text_lang: "en",
      url: "https://prebell.site/en/p/aapl-q4-fy2026-earnings",
      source: "https://prebell.site/en/p/aapl-q4-fy2026-earnings",
      published_at: "2026-10-30T21:00:00Z",
      updated_at: "2026-10-30T21:00:00Z",
      filing_date: "2026-10-30",
      filing_url: "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm",
      press_release_url: "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/a8-kex991q4202609262026.htm",
      figures_available: true,
      share_move: null,
      fiscal_period: { quarter: 4, fiscal_year: 2026, label: "Q4 FY2026" },
    });
    expect(out.figures[0]).toEqual({
      key: "revenue",
      label: "Revenue",
      value: 102.5e9,
      unit: "usd",
      amount: "$102.5 billion",
      quote: "The Company posted quarterly revenue of $102.5 billion, up 8 percent year over year",
    });
    expect(out.guidance[0].summary_en).toBe("Apple expects revenue growth of 10 to 12 percent.");
    expect(out.article).toBeUndefined();
    expect(c.reads).toHaveLength(1);
    expect(c.gets).toEqual([AAPL_KEY]);
  });

  it("in Chinese: Chinese title, summary and labels; quotes stay English", async () => {
    const out = await run("earnings_report", { ticker: "AAPL", lang: "zh", full_text: true });
    expect(out.title).toBe("中文标题 aapl-q4-fy2026-earnings");
    expect(out.text_lang).toBe("zh");
    expect(out.url).toBe("https://prebell.site/zh/p/aapl-q4-fy2026-earnings");
    expect(out.fiscal_period.label).toBe("2026财年第四季度");
    expect(out.figures.map((f: { label: string }) => f.label)).toEqual(["营收", "摊薄每股收益（GAAP）", "毛利率"]);
    expect(out.figures[0].quote).toMatch(/^The Company posted/);
    expect(out.article).toBe("# 中文正文 aapl-q4-fy2026-earnings");
  });

  it("returns the share move when the facts hold one (session, percent, last, time; not the reference close)", async () => {
    const objs = { ...objects(), [AAPL_KEY]: fixture("earnings-AAPL-share-move.json") };
    const out = await run("earnings_report", { ticker: "AAPL" }, ctx({ objs }));
    expect(out.share_move).toEqual({ session: "after_hours", change_pct: -4.2, last: 251.3, as_of: "2026-10-30T21:05:00Z" });
    expect(out.highlights).toEqual([{ summary_en: "Services revenue was $28,750 million.", quote: "Services revenue $28,750 $24,972" }]);
    expect(JSON.stringify(out)).not.toContain("262.32");
  });

  it("a missing or unreadable bundle still returns title, summary, period and links", async () => {
    for (const objs of [{}, { [AAPL_KEY]: "{oops" }] as Record<string, string>[]) {
      const c = ctx({ objs });
      const out = await run("earnings_report", { ticker: "AAPL" }, c);
      expect(out).toMatchObject({
        status: "check_published",
        title: "EN title aapl-q4-fy2026-earnings",
        figures_available: false,
        figures: [],
        fiscal_period: { quarter: 4, fiscal_year: 2026, label: "Q4 FY2026", quote: null },
        filing_url: "https://www.sec.gov/Archives/edgar/data/320193/000032019326000070/0000320193-26-000070-index.htm",
        press_release_url: null,
        share_move: null,
      });
      expect(out.message).toMatch(/not available/);
    }
  });

  it("a check whose Chinese text is pending answers in English and says so", async () => {
    const out = await run("earnings_report", { ticker: "MSFT", lang: "zh" });
    expect(out.status).toBe("check_published");
    expect(out.text_lang).toBe("en");
    expect(out.title).toBe("EN title msft-q1-fy2027-earnings");
    expect(out.url).toBe("https://prebell.site/zh/p/msft-q1-fy2027-earnings");
  });

  it("a calendar-only company: no check by design, with its reported filing", async () => {
    const out = await run("earnings_report", { ticker: "A" });
    expect(out).toMatchObject({
      status: "no_check",
      company: "Agilent Technologies",
      writes_check: false,
      reported: true,
      last_report: {
        date: "2026-10-27",
        filing_url: "https://www.sec.gov/Archives/edgar/data/1090872/000109087226000041/0001090872-26-000041-index.htm",
      },
      next_report: { date: "2027-02-20", slot: "after_close", status: "unconfirmed" },
      source: "https://prebell.site/en/t/A",
    });
    expect(out.message).toMatch(/calendar only/);
  });

  it("a write-up company without a check yet gives its next date", async () => {
    const out = await run("earnings_report", { ticker: "NVDA", lang: "zh" });
    expect(out).toMatchObject({
      status: "no_check",
      writes_check: true,
      reported: false,
      last_report: null,
      next_report: { date: "2026-11-18", slot: "after_close", status: "unconfirmed" },
    });
    expect(out.message).toBe("盘前铃尚未发布这家公司的财报核查。");
  });

  it("an unknown ticker is a normal result saying so", async () => {
    const c = ctx();
    const out = await run("earnings_report", { ticker: "ZZZZ" }, c);
    expect(out.status).toBe("unknown_ticker");
    expect(out.message).toMatch(/no calendar date or earnings check for ZZZZ/);
    expect(c.reads).toHaveLength(2);
    expect(c.gets).toHaveLength(0);
  });

  it("refuses a missing or malformed ticker", () => {
    expect(bad("earnings_report", {})).toThrow(/ticker is required/);
    expect(bad("earnings_report", { ticker: "../etc" })).toThrow(/Not a ticker/);
    expect(bad("earnings_report", { ticker: "AAPL", full_text: "yes" })).toThrow(/full_text/);
  });
});

describe("latest_brief", () => {
  it("today's premarket brief: snapshot time, indexes, movers as printed, themes, watch list", async () => {
    const c = ctx();
    const out = await run("latest_brief", {}, c);
    expect(out).toMatchObject({
      found: true,
      kind: "premarket",
      trading_date: "2026-10-30",
      title: "EN title premarket-2026-10-30",
      text_lang: "en",
      url: "https://prebell.site/en/p/premarket-2026-10-30",
      published_at: "2026-10-30T12:05:00Z",
      updated_at: "2026-10-30T12:40:00Z",
      as_of: "2026-10-30T13:20:43.115Z",
      is_today: true,
      today_et: "2026-10-30",
      market_data_available: true,
    });
    expect(out.snapshot_note).toBe("Prices are a snapshot taken at Oct 30, 2026, 9:20 AM ET, not live quotes.");
    expect(out.not_today_note).toBeUndefined();
    expect(out.next_brief_due).toBeUndefined();
    expect(out.indexes[0]).toEqual({ symbol: "ES=F", name: "S&P 500 futures", last: 7665.5, change: -88.75, change_pct: -1.1445336428410227 });
    expect(out.gainers[0]).toEqual({
      ticker: "MDLZ",
      name: "Mondelez International",
      last: 66.96,
      change_pct: 7.0846,
      reason_en: "Shares surged in premarket trading after the global snacks manufacturer reported results.",
      reason_url: "https://finance.yahoo.com/markets/stocks/articles/why-mondelez-international-stock-surging-105020533.html",
      reason_check_url: null,
    });
    expect(out.losers[1]).toMatchObject({ ticker: "HPQ", reason_en: "Reported results after Thursday's close", reason_url: null, reason_check_url: "https://prebell.site/en/p/hpq-q4-fy2026-earnings" });
    // Never more market data than the page: no previous close, no volume.
    const text = JSON.stringify(out);
    expect(text).not.toMatch(/prev_close|volume|224089|62\.53/);
    expect(out.themes).toEqual([
      { group: "Semiconductors", direction: "down", median_pct: -4.1, members: [{ ticker: "INTC", change_pct: -26 }, { ticker: "ARM", change_pct: -10.08 }], more: 2 },
    ]);
    expect(out.watch[0]).toEqual({
      kind: "event",
      title: "October CPI",
      time: "8:30 AM ET",
      note: null,
      status: "released",
      link: "https://www.bls.gov/news.release/archives/cpi_11102026.htm",
      moved_from: null,
      figures: [{ key: "cpi_year", value: 3.1, quote: "The all items index rose 3.1 percent for the 12 months ending October." }],
    });
    expect(out.watch[1]).toMatchObject({ kind: "earnings", title: "Apple earnings", time: null, link: null });
    expect(c.reads).toHaveLength(1);
    expect(c.gets).toEqual(["premarket/2026-10-30.json"]);
  });

  it("in Chinese, with the full article", async () => {
    const out = await run("latest_brief", { lang: "zh", full_text: true });
    expect(out.title).toBe("中文标题 premarket-2026-10-30");
    expect(out.text_lang).toBe("zh");
    expect(out.indexes[0].name).toBe("标普500期货");
    expect(out.themes[0].group).toBe("半导体");
    expect(out.watch[0]).toMatchObject({ title: "10月消费者物价指数（CPI）", time: "上午8:30（美东）" });
    expect(out.gainers[0].reason_en).toMatch(/^Shares surged/);
    expect(out.snapshot_note).toMatch(/^价格为2026年10月30日/);
    expect(out.article).toBe("# 中文正文 premarket-2026-10-30");
  });

  it("the recap on request", async () => {
    const out = await run("latest_brief", { kind: "recap" });
    expect(out).toMatchObject({ kind: "recap", trading_date: "2026-10-29", is_today: false, market_data_available: true });
    expect(out.indexes.map((i: { symbol: string }) => i.symbol)).toEqual(["^GSPC", "^NDX", "^DJI", "^RUT"]);
    expect(out.watch).toBeUndefined();
    // Recap due after today's close.
    expect(out.next_brief_due).toEqual({ date: "2026-10-30", time_et: "16:45", text: "The next after-close recap is due at 4:45 PM ET on 2026-10-30." });
  });

  it("before 8:00 ET the newest brief is yesterday's recap, with is_today false and today's due time", async () => {
    const c = ctx({ now: new Date("2026-10-30T11:00:00Z"), sql: seed() + DROP_FRIDAY_BRIEF });
    const out = await run("latest_brief", {}, c);
    expect(out).toMatchObject({ kind: "recap", trading_date: "2026-10-29", is_today: false });
    expect(out.not_today_note).toBe("This is the brief for 2026-10-29, not today (2026-10-30).");
    expect(out.next_brief_due).toEqual({ date: "2026-10-30", time_et: "08:00", text: "The next premarket brief is due at 8:00 AM ET on 2026-10-30." });
    // Brief, its facts, the closure lookup.
    expect(c.reads).toHaveLength(2);
    expect(c.gets).toHaveLength(1);
  });

  it("on a weekend, the next brief skips to the next trading day, and skips closures", async () => {
    const sat = new Date("2026-10-31T15:00:00Z");
    const out = await run("latest_brief", { lang: "zh" }, ctx({ now: sat }));
    expect(out.is_today).toBe(false);
    expect(out.next_brief_due).toEqual({ date: "2026-11-02", time_et: "08:00", text: "下一期盘前简报将于2026-11-02美东时间上午8:00发布。" });
    const closed = await run("latest_brief", {}, ctx({ now: sat, sql: seed() + macro("nyse:closed:2026-11-02", "closed", "2026-11-02") }));
    expect(closed.next_brief_due.date).toBe("2026-11-03");
  });

  it("without closure rows no date is promised", async () => {
    const sql = seed() + "DELETE FROM macro_events WHERE series = 'closed';";
    const out = await run("latest_brief", {}, ctx({ now: new Date("2026-10-31T15:00:00Z"), sql }));
    expect(out.next_brief_due).toEqual({ date: null, time_et: "08:00", text: "The next premarket brief is due at 8:00 AM ET on the next trading day." });
  });

  it("past 8:00 on a trading day with no brief yet says today's is late", async () => {
    const out = await run("latest_brief", { kind: "premarket" }, ctx({ now: new Date("2026-11-02T14:30:00Z") }));
    expect(out.next_brief_due).toEqual({
      date: "2026-11-02",
      time_et: "08:00",
      text: "Today's premarket brief is normally out by 8:00 AM ET but has not been published yet.",
    });
  });

  it("missing facts: the post without market data", async () => {
    const out = await run("latest_brief", {}, ctx({ objs: {} }));
    expect(out).toMatchObject({ found: true, market_data_available: false, as_of: null });
    expect(out.gainers).toBeUndefined();
    expect(out.snapshot_note).toMatch(/not available/);
  });

  it("no brief at all is an empty result, not an error", async () => {
    const out = await run("latest_brief", {}, ctx({ sql: "" }));
    expect(out).toMatchObject({ found: false, as_of: null, source: "https://prebell.site/en" });
    expect(out.message).toMatch(/not published a brief/);
    expect(out.next_brief_due.time_et).toBe("08:00");
  });

  it("refuses a bad kind", () => {
    expect(bad("latest_brief", { kind: "weekly" })).toThrow(/kind/);
  });

  it("parseWatchList is lenient and omits a Missing list", () => {
    expect(parseWatchList(fixture("premarket-2026-10-30.json"), "en")).toBeNull();
    expect(parseWatchList({ watch_list: { Value: [{ kind: "event" }, 5, { kind: "event", title: "X", title_zh: "", status: "scheduled", link: "javascript:x" }] } }, "zh")).toEqual([
      { kind: "event", title: "X", time: null, note: null, status: "scheduled", link: null, moved_from: null },
    ]);
  });
});

describe("seed sanity", () => {
  it("helpers produce valid SQL", () => {
    expect(() => memoryD1(seed() + post({ type: "recap", date: "2026-10-30", slug: "x", factsKey: null, publishedAt: "t", zh: null }) + check("KO", 21344, "0000021344-26-000001", "2026-10-21", "ko-q3-fy2026-earnings") + cal("KO", "2026-10-21", "reported"))).not.toThrow();
    expect(premarketFacts().themes).toBeDefined();
  });
});

describe("review fixes", () => {
  it("B1: dates far outside the calendar's window are refused at once (no runaway loop)", () => {
    const started = Date.now();
    // The reviewer's request: not a date the helpers accept (years 1000-9998), so refused before any arithmetic.
    expect(bad("earnings_calendar", { from: "9999-12-31", to: "9999-12-31" })).toThrow(/YYYY-MM-DD/);
    expect(bad("earnings_calendar", { from: "9998-12-31", to: "9998-12-31" })).toThrow(/between 2025-09-25 and 2029-01-07/);
    expect(bad("earnings_calendar", { from: "0001-01-01" })).toThrow(ArgumentError);
    expect(Date.now() - started).toBeLessThan(1000);
  });

  it("B1: the window is 400 days back to 800 days ahead of today (ET), inclusive", async () => {
    // FRIDAY is 2026-10-30: the window is 2025-09-25 .. 2029-01-07.
    expect(bad("earnings_calendar", { from: "2025-09-25", to: "2025-09-25" })).not.toThrow();
    expect(bad("earnings_calendar", { from: "2025-09-24", to: "2025-09-25" })).toThrow(/between/);
    expect(bad("earnings_calendar", { from: "2029-01-07", to: "2029-01-07" })).not.toThrow();
    expect(bad("earnings_calendar", { from: "2029-01-02", to: "2029-01-08" })).toThrow(/between/);
    // `from` alone makes a week from it; ending on the window's last day is still allowed.
    expect(bad("earnings_calendar", { from: "2029-01-01" })).not.toThrow();
    expect(bad("earnings_calendar", { from: "2029-01-02" })).toThrow(/between/);
    const out = await run("earnings_calendar", { from: "2029-01-01" });
    expect(out.days.map((d: { date: string }) => d.date)).toEqual(["2029-01-01", "2029-01-02", "2029-01-03", "2029-01-04", "2029-01-05"]);
  });

  it("S2: calendar weeks are read once and shared by calls with any ticker subset or range inside them", async () => {
    const cache = memoryCache();
    const c = ctx({ cache });
    await run("earnings_calendar", { from: "2026-11-02", to: "2026-11-06", tickers: ["KO"] }, c);
    const reads = c.reads.length;
    expect(reads).toBe(2); // that week's calendar rows and events
    const out = await run("earnings_calendar", { from: "2026-11-03", to: "2026-11-04", tickers: ["NVDA", "KO"] }, c);
    expect(c.reads.length).toBe(reads);
    expect(out.days.map((d: { date: string }) => d.date)).toEqual(["2026-11-03", "2026-11-04"]);
    expect(out.days[0].companies.map((x: { ticker: string }) => x.ticker)).toEqual(["KO"]);
    expect(out.tickers_without_dates).toEqual(["NVDA"]);
    expect(cache.keys.every((k) => /\/__mcp_cache\/v1\/calendar_week\?lang=(en|zh)&monday=\d{4}-\d{2}-\d{2}$/.test(k))).toBe(true);
  });

  it("S2: a range across weeks reads each week once", async () => {
    const c = ctx({ cache: memoryCache() });
    // Wed 2026-10-28 .. Mon 2026-11-16 touches four weeks.
    const first = await run("earnings_calendar", { from: "2026-10-28", to: "2026-11-16" }, c);
    expect(c.reads.length).toBe(8);
    const rows = first.days.flatMap((d: { companies: { ticker: string }[] }) => d.companies.map((x) => x.ticker));
    expect(rows).toEqual(["MSFT", "XOM", "AAPL", "KO"]);
    await run("earnings_calendar", { from: "2026-10-26", to: "2026-11-15", lang: "en" }, c);
    expect(c.reads.length).toBe(8);
    await run("earnings_calendar", { from: "2026-10-26", to: "2026-11-01", lang: "zh" }, c);
    expect(c.reads.length).toBe(10);
  });

  it("S6: a Chinese check that failed links the English page", async () => {
    const sql = seed() + check("NFLX", 1065280, "0001065280-26-000040", "2026-10-21", "nflx-q3-fy2026-earnings", null) +
      "INSERT INTO post_translations (post_id, lang, status, updated_at) SELECT id,'zh','failed','t' FROM posts WHERE slug='nflx-q3-fy2026-earnings';";
    const out = await run("earnings_report", { ticker: "NFLX", lang: "zh" }, ctx({ sql }));
    expect(out.text_lang).toBe("en");
    expect(out.url).toBe("https://prebell.site/en/p/nflx-q3-fy2026-earnings");
    expect(out.source).toBe(out.url);
    // Pending still links the Chinese page, which shows the coming-soon notice.
    const msft = await run("earnings_report", { ticker: "MSFT", lang: "zh" });
    expect(msft.url).toBe("https://prebell.site/zh/p/msft-q1-fy2027-earnings");
  });

  it("N3: before 8:00 ET on a weekday with no closure rows, today's brief is due today", async () => {
    const sql = seed() + DROP_FRIDAY_BRIEF + "DELETE FROM macro_events WHERE series = 'closed';";
    const out = await run("latest_brief", {}, ctx({ now: new Date("2026-10-30T11:00:00Z"), sql }));
    expect(out.next_brief_due).toEqual({ date: "2026-10-30", time_et: "08:00", text: "The next premarket brief is due at 8:00 AM ET on 2026-10-30." });
  });

  it("N1: a degraded answer reports itself through onError", async () => {
    const failing = { async get() { throw new Error("r2 down"); } };
    const c = ctx({ bucket: failing });
    const out = await run("latest_brief", {}, c);
    expect(out.market_data_available).toBe(false);
    expect(c.errors).toHaveLength(1);
    const broken = counted(memoryD1(seed()));
    const d1: typeof broken.db = { prepare: (s) => { if (s.includes("macro_events")) throw new Error("D1 down"); return broken.db.prepare(s); } };
    const c2 = ctx({ db: d1, now: new Date("2026-10-31T15:00:00Z") });
    const stale = await run("latest_brief", {}, c2);
    expect(stale.next_brief_due.date).toBeNull();
    expect(c2.errors).toHaveLength(1);
  });
});
