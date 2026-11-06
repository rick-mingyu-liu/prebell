import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import { translator } from "../src/lib/translator";
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
const { default: EarningsCalendar } = await import("../src/app/[locale]/earnings/page");
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
      expect(out.replace(/^(<link [^>]*\/>)+/, "").startsWith('<div class="list-page">')).toBe(true);
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

  it("the calendar page renders the week from D1", async () => {
    const out = renderToStaticMarkup(await pages[1][1]());
    expect(out).toMatch(/<a class="cal-tile-link" title="Intel" href="\/en\/p\/earnings-intc-2026-10-29">/);
    expect(out).toContain('alt="Intel"');
    expect(out).toContain('<span class="num up" aria-hidden="true">✓</span>');
    expect(out).toContain("Confirmed: the company announced the date.");
    expect(out).toContain('<nav class="pager">');
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
