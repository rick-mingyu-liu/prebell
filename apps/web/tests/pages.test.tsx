import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import { translator } from "../src/lib/translator";
import { addDays, todayEt, weekStart, WEEKS_AHEAD } from "../src/lib/earnings";
import { memoryD1 } from "./d1";

// The pages read D1 through getDb() and messages through next-intl/server; both are swapped for
// an in-memory D1 and the synchronous translator, so the real page components render here.
const env = vi.hoisted(() => ({ db: null as unknown, locale: "en" as string }));
vi.mock("@/lib/db", () => ({ getDb: () => env.db }));
vi.mock("next-intl/server", () => ({
  setRequestLocale: (l: string) => {
    env.locale = l;
  },
  getTranslations: async (arg?: string | { locale: string; namespace?: string }) => {
    const ns = typeof arg === "string" ? arg : arg?.namespace;
    const t = translator((typeof arg === "object" ? arg.locale : env.locale) as Locale);
    return ns ? (k: string, v?: Record<string, unknown>) => t(`${ns}.${k}` as never, v as never) : t;
  },
}));

const { default: Archive } = await import("../src/app/[locale]/archive/page");
const { default: EarningsCalendar, generateMetadata: calendarMetadata } = await import("../src/app/[locale]/earnings/page");
const { default: TickerArchive, generateMetadata: tickerMetadata } = await import("../src/app/[locale]/t/[ticker]/page");

const seed = `
INSERT INTO runs VALUES ('earnings:intc','earnings','2026-10-29','published',1,NULL,'2026-10-29T21:00:00Z','2026-10-29T21:00:00Z');
INSERT INTO posts (id, run_key, content_type, template_id, trading_date, slug, facts_r2_key, published_at)
  VALUES (1,'earnings:intc','earnings','earnings:v1','2026-10-29','earnings-intc-2026-10-29',NULL,'2026-10-29T21:10:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (1,'en','published','Intel Q3','Sum','# Intel Q3','2026-10-29T21:10:00Z');
INSERT INTO post_translations (post_id, lang, status, title, summary, body_md, updated_at) VALUES (1,'zh','published','英特尔 Q3','摘要','# 英特尔 Q3','2026-10-29T21:10:00Z');
INSERT INTO earnings_filings VALUES ('0000050863-26-000041',50863,'INTC','8-K','2.02','2026-10-29','2026-10-29','2026-10-29T20:05:00Z','x.htm','earnings:intc','done','2026-10-29T20:06:00Z');
INSERT INTO earnings_calendar (ticker, report_date, time_slot, status, source, first_seen_at, checked_at, updated_at)
  VALUES ('INTC','2026-10-29','amc','reported','edgar','2026-10-01','2026-10-29','2026-10-29');
INSERT INTO earnings_calendar (ticker, report_date, time_slot, status, source, proof_url, first_seen_at, checked_at, updated_at)
  VALUES ('A','2026-10-28','amc','reported','edgar','https://www.sec.gov/Archives/edgar/data/1090872/000109087226000041/0001090872-26-000041-index.htm','2026-10-01','2026-10-28','2026-10-28');
`;

const props = <P extends Record<string, string>>(params: P, search: Record<string, string> = {}) =>
  ({ params: Promise.resolve(params), searchParams: Promise.resolve(search) }) as never;

const pages: [string, () => Promise<React.ReactElement>][] = [
  ["archive", () => Archive(props({ locale: "en" }, { type: "earnings" }))],
  ["earnings calendar", () => EarningsCalendar(props({ locale: "en" }, { week: "2026-10-26" }))],
  ["ticker", () => TickerArchive(props({ locale: "en", ticker: "INTC" }))],
];

describe("list pages are one paper column", () => {
  beforeEach(() => {
    env.db = memoryD1(seed);
  });

  for (const [name, page] of pages) {
    it(`${name}: one column with no sidebar and no in-page language pill`, async () => {
      const out = renderToStaticMarkup(await page());
      // React hoists a preload link for each eager image (the calendar's first logos); it comes ahead of the page.
      // The calendar is the one wide page: its board of day columns is wider than the paper column.
      const root = name === "earnings calendar" ? '<div class="cal-page">' : '<div class="list-page">';
      expect(out.replace(/^(<link [^>]*\/>)+/, "").startsWith(root)).toBe(true);
      expect(out).not.toContain("sidebar");
      expect(out).not.toContain("grid-article");
      expect(out).not.toContain("lang-pill");
      expect(out).not.toMatch(/badge|class="tag"/);
    });
  }

  it("the archive shows its rows with a cover, filters and pager as plain links", async () => {
    const out = renderToStaticMarkup(await pages[0][1]());
    expect(out).toContain('class="post-row"');
    expect(out).toContain('class="cover cover-small cover-flat"');
    expect(out).toContain('<nav class="filters"');
    // The outline is h1 then h2 row titles, with no skipped level.
    expect(out).toContain('<h2 class="story-title">');
    expect(out).not.toContain("<h3");
  });

  it("the calendar page renders the week from D1 as a board", async () => {
    const out = renderToStaticMarkup(await pages[1][1]());
    expect(out).toContain('<div class="cal-board">');
    expect(out).toMatch(/<a class="cal-card-link" title="Intel" href="\/en\/p\/earnings-intc-2026-10-29">/);
    expect(out).toContain('alt="Intel"');
    expect(out).toContain('<span class="cal-card-tick" aria-hidden="true">✓</span>');
    expect(out).toContain('alt="Agilent Technologies"');
    expect(out).toContain("Confirmed: the company announced the date.");
    expect(out).toContain("Times US Eastern. Economy &amp; Fed:");
    expect(out).toContain('<nav class="pager">');
  });

  it("heads the board with the week, its count, the week arrows, the way back and a share button", async () => {
    const out = renderToStaticMarkup(await pages[1][1]());
    expect(out).toContain('<h1 class="cal-title"><span class="kicker">Market calendar</span>Week of Oct 26, 2026</h1>');
    // Two companies, no events that week: the events count is left out.
    expect(out).toContain('<p class="cal-count">2 companies</p>');
    expect(out).toContain('<nav class="cal-weeks" aria-label="Weeks">');
    // The calendar's first week: no way further back.
    expect(out).not.toContain('rel="prev"');
    expect(out).not.toContain("Previous week");
    expect(out).toContain('<a class="cal-btn cal-arrow" rel="next" href="/en/earnings?week=2026-11-02">');
    expect(out).toContain('<nav class="pager"><a class="next" href="/en/earnings?week=2026-11-02">Next week →</a></nav>');
    // A week that is not this one links back to it.
    expect(out).toContain('<a class="cal-btn cal-this-week" href="/en/earnings">Back to this week</a>');
    // The share button waits for script (hidden in the server markup), with a live region for its result.
    expect(out).toMatch(/<button type="button" class="cal-btn cal-share-btn" hidden="">.*?Share<\/button><span class="cal-share-msg" role="status"><\/span>/);
    expect(out).not.toMatch(/download/i);
    // The board itself: five columns, none of them today in a past week.
    expect(out.match(/<section id="d-2026-10-/g)).toHaveLength(5);
    expect(out).not.toContain("is-today");
  });

  it("a later week links back a week", async () => {
    const out = renderToStaticMarkup(await EarningsCalendar(props({ locale: "en" }, { week: "2026-11-02" })));
    expect(out).toContain(
      '<a class="cal-btn cal-arrow" rel="prev" href="/en/earnings?week=2026-10-26"><span aria-hidden="true">←</span><span class="sr-only">Previous week</span></a>',
    );
    expect(out).toContain('<a href="/en/earnings?week=2026-10-26">← Previous week</a>');
  });

  it("weeks before the first and past the last answer 404 before reading anything, and the last has no next", async () => {
    const last = addDays(weekStart(todayEt()), 7 * WEEKS_AHEAD);
    for (const week of ["2026-10-19", "2004-12-27", addDays(last, 7), "2048-05-11"]) {
      await expect(EarningsCalendar(props({ locale: "en" }, { week })), week).rejects.toThrow("NEXT_HTTP_ERROR_FALLBACK;404");
      await expect(calendarMetadata(props({ locale: "en" }, { week })), week).rejects.toThrow("NEXT_HTTP_ERROR_FALLBACK;404");
    }
    const out = renderToStaticMarkup(await EarningsCalendar(props({ locale: "en" }, { week: last })));
    expect(out).toContain('rel="prev"');
    expect(out).not.toContain('rel="next"');
    expect(out).not.toContain("Next week");
  });

  it("a dated week's title names the week and its description is the calendar's own", async () => {
    const m = await calendarMetadata(props({ locale: "en" }, { week: "2026-11-04" }));
    expect(m.title).toBe("Market calendar · Week of Nov 2, 2026");
    expect(m.description).toBe(
      "US earnings dates for the S&P 500 and Nasdaq-100, with confirmed dates, reported results and links to the SEC filings.",
    );
    expect((await calendarMetadata(props({ locale: "en" }))).title).toBe("Market calendar");
  });

  it("the current week has no way back, counts companies once and events, and marks today", async () => {
    const today = todayEt();
    const monday = weekStart(today);
    env.db = memoryD1(
      `${seed}
      INSERT INTO earnings_calendar (ticker, report_date, time_slot, status, source, first_seen_at, checked_at, updated_at)
        VALUES ('AMD','${monday}','amc','confirmed','yahoo','2026-10-01','2026-10-29','2026-10-29');
      INSERT INTO earnings_calendar (ticker, report_date, time_slot, status, source, first_seen_at, checked_at, updated_at)
        VALUES ('AMD','${addDays(monday, 1)}','bmo','unconfirmed','yahoo','2026-10-01','2026-10-29','2026-10-29');
      INSERT INTO macro_events (id, series, ref_period, variant, starts_at, et_date, tier, status, proof_url, release_url, moved_from, missing_count, first_seen_at, checked_at, updated_at)
        VALUES ('bls:cpi:x','cpi','2026-10',NULL,'${monday}T13:30:00Z','${monday}',1,'confirmed','https://www.bls.gov/schedule/news_release/cpi.htm',NULL,NULL,0,'x','x','x');`,
    );
    const out = renderToStaticMarkup(await EarningsCalendar(props({ locale: "en" })));
    expect(out).toContain('<p class="cal-count">1 company · 1 economy event</p>');
    expect(out).not.toContain("Back to this week");
    expect(out).toContain('<div class="cal-econ"><h3 class="cal-slot">Economy &amp; Fed</h3>');
    const weekday = new Date(`${today}T12:00:00Z`).getUTCDay();
    if (weekday >= 1 && weekday <= 5) expect(out).toContain(`<section id="d-${today}" class="cal-col is-today"`);
  });

  it("the Chinese calendar heads the board in Chinese", async () => {
    const out = renderToStaticMarkup(await EarningsCalendar(props({ locale: "zh" }, { week: "2026-10-26" })));
    expect(out).toContain('<h1 class="cal-title"><span class="kicker">市场日历</span>2026年10月26日当周</h1>');
    expect(out).toContain('<p class="cal-count">本周 2 家公司</p>');
    expect(out).toContain('<span class="sr-only">下一周</span>');
    expect(out).toContain(">回到本周</a>");
    expect(out).toMatch(/hidden="">.*?分享<\/button>/);
    expect(out).toContain("<time dateTime=\"2026-10-26\">周一 · 10/26</time>");
    expect(out).toContain("时间均为美东时间。经济与美联储");
  });

  it("the ticker page lists its checks and its report dates without badges", async () => {
    const out = renderToStaticMarkup(await pages[2][1]());
    expect(out).toContain('<li class="date-row">');
    expect(out).toContain('<span class="num up" aria-hidden="true">✓</span>');
    expect(out).toContain("Reported");
  });

  it("a calendar-only company's page names it, links its SEC filing, and is not indexed until it has a check", async () => {
    const out = renderToStaticMarkup(await TickerArchive(props({ locale: "en", ticker: "A" })));
    expect(out).toContain('<p class="meta">Agilent Technologies</p>');
    expect(out).toContain("No earnings checks yet.");
    expect(out).toContain('aria-label="SEC filing: Oct 28, 2026">SEC filing ↗</a>');
    expect((await tickerMetadata(props({ locale: "en", ticker: "A" }))).robots).toEqual({ index: false });
    expect((await tickerMetadata(props({ locale: "zh", ticker: "INTC" }))).robots).toBeUndefined();
  });
});
