import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Locale } from "../src/i18n/routing";
import {
  fmtValuation,
  formLabel,
  httpsUrl,
  iposByDate,
  listIpoSlugs,
  listIposBetween,
  listIpoWatch,
  loadIpo,
  reportedDatePassed,
  secUrl,
  yahooUrl,
} from "../src/lib/ipo";
import type { D1Like } from "../src/lib/posts";
import { sitemapEntries } from "../src/lib/sitemap";
import { translator } from "../src/lib/translator";
import { todayEt } from "../src/lib/earnings";
import { memoryD1 } from "./d1";

// The pages read D1 through getDb() and messages through next-intl/server; both are swapped for an in-memory D1
// (every migration plus db/seed-ipo-preview.sql) and the synchronous translator.
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

const { default: IpoPage, generateMetadata } = await import("../src/app/[locale]/ipo/[slug]/page");
const { default: EarningsCalendar } = await import("../src/app/[locale]/earnings/page");

/** The preview seed: Anthropic (range_set, real cover facts, made-up news), Northwind (listed), Harbor Bio (filed). */
const SEED = readFileSync(new URL("../../../db/seed-ipo-preview.sql", import.meta.url), "utf8");

/** Northwind as priced: the 424B4 is on file, the date is the rule's "expected", no trading rows yet. */
const PRICED = `
UPDATE ipo_watch SET status = 'priced', first_trade_date = '2026-11-05', date_status = 'expected', date_source = 'filing',
  date_source_name = '424B4', date_source_url = 'https://www.sec.gov/Archives/edgar/data/9999990001/999999000126000020/9999990001-26-000020-index.htm',
  date_quote = 'Prospectus dated November 4, 2026', date_as_of = '2026-11-05' WHERE slug = 'northwind-robotics';
DELETE FROM ipo_trading WHERE slug = 'northwind-robotics';`;

/** Harbor Bio withdrawn: an RW after its S-1. */
const WITHDRAWN = `
UPDATE ipo_watch SET status = 'withdrawn' WHERE slug = 'harbor-bio';
INSERT INTO ipo_filings (accession, slug, form, filing_date, index_url, doc_url, read_state, attempts, first_seen_at) VALUES
  ('9999990002-26-000009', 'harbor-bio', 'RW', '2026-11-10', 'https://www.sec.gov/Archives/edgar/data/9999990002/999999000226000009/9999990002-26-000009-index.htm', NULL, 'none', 0, '2026-11-10T14:00:00Z');`;

/**
 * Rows that break the migration's CHECKs (bad URL schemes, unpaired date fields) are written with the checks off,
 * as a buggy writer or an older table could have left them: the reader's own checks are what is tested.
 */
const UNCHECKED = (sql: string) => `PRAGMA ignore_check_constraints = ON;\n${sql}\nPRAGMA ignore_check_constraints = OFF;`;

/** A hostile row: markup in every text field, and links that must never become links. */
const EVIL = UNCHECKED(`
INSERT INTO ipo_watch (slug, cik, name, name_zh, status, exchange, symbol, facts_accession, first_trade_date,
  date_status, date_source, date_source_name, date_source_url, date_quote, date_as_of, checked_at, updated_at) VALUES
  ('evil-co', 1, '<script>alert("name")</script>', '<img src=x onerror=alert(1)>', 'range_set', '<b>Exchange</b>', '<i>EVL</i>', 'evil-1',
   '2026-11-18', 'reported', 'news', '<a href="javascript:alert(1)">Outlet</a>', 'javascript:alert(1)',
   '</blockquote><script>alert("date")</script>', '2026-11-10', 'x', 'x');
INSERT INTO ipo_filings (accession, slug, form, filing_date, index_url, doc_url, read_state, attempts, first_seen_at) VALUES
  ('evil-1', 'evil-co', '<svg onload=alert(1)>', '2026-11-09', 'javascript:alert(1)', NULL, 'read', 1, 'x'),
  ('evil-2', 'evil-co', 'S-1', '2026-11-01', 'http://www.sec.gov/Archives/x-index.htm', NULL, 'none', 0, 'x');
INSERT INTO ipo_facts (accession, kind, slug, value_num, value_text, quote) VALUES
  ('evil-1', 'exchange', 'evil-co', NULL, '<script>alert("fact")</script>', '<img src=x onerror=alert("quote")>'),
  ('evil-1', 'shares_offered', 'evil-co', 1000, NULL, '"><script>alert(2)</script>');
INSERT INTO ipo_items (slug, kind, url, outlet, published_on, quote, statement_en, statement_zh, speaker, value_usd, value_date, found_at) VALUES
  ('evil-co', 'valuation', 'javascript:alert(1)', '<script>alert("outlet")</script>', '2026-11-10', '</blockquote><script>alert(3)</script>', NULL, NULL, NULL, 5e9, NULL, 'x'),
  ('evil-co', 'company_move', 'http://insecure.example/a', 'Plain Outlet', '2026-11-09', 'Plain quote <b>bold</b>', '<script>alert("statement")</script>', NULL, NULL, NULL, NULL, 'x'),
  ('evil-co', 'view', 'https://ok.example/v', 'Outlet', '2026-11-08', 'A view', NULL, NULL, '<script>alert("speaker")</script>', NULL, NULL, 'x');`);

const props = (params: Record<string, string>, search: Record<string, string> = {}) =>
  ({ params: Promise.resolve(params), searchParams: Promise.resolve(search) }) as never;
const page = async (slug: string, locale: Locale = "en") => renderToStaticMarkup(await IpoPage(props({ locale, slug })));
const calendar = async (week: string, locale: Locale = "en") =>
  renderToStaticMarkup(await EarningsCalendar(props({ locale }, { week })));

/** Text of rendered HTML with tags removed and entities for quotes decoded, for wording checks. */
const textOf = (html: string) =>
  html
    .replace(/<[^>]+>/g, "")
    .replace(/&quot;/g, '"')
    .replace(/&#x27;/g, "'")
    .replace(/&amp;/g, "&");

const ANTHROPIC_FILING = "https://www.sec.gov/Archives/edgar/data/2130357/000162828026072484/0001628280-26-072484-index.htm";
const VIEW_QUOTE = "The office tools show how far they want to push into software that businesses already pay for";

beforeEach(() => {
  env.db = memoryD1(SEED);
  // The watch list keeps a listed IPO for 30 days after its first trade; fix "today" at the seed's day.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-11-11T15:00:00Z"));
});
afterEach(() => {
  vi.useRealTimers();
});

describe("IPO readers", () => {
  it("loads one IPO with its cover facts, filings, items and trading, newest first", async () => {
    const ipo = (await loadIpo(memoryD1(SEED), "anthropic", "en"))!;
    expect(ipo).toMatchObject({
      slug: "anthropic",
      name: "Anthropic",
      symbol: "ANT",
      exchange: "Nasdaq Global Select Market",
      status: "range_set",
      date: { date: "2026-11-16", status: "reported", source: "news", sourceName: "Example News", asOf: "2026-11-10" },
    });
    expect(ipo.facts!.filing).toEqual({
      accession: "0001628280-26-072484",
      form: "S-1/A",
      filingDate: "2026-11-10",
      indexUrl: ANTHROPIC_FILING,
    });
    const f = ipo.facts!.facts;
    expect(f.shares_offered).toEqual({ num: 825000000, text: null, quote: "Class A Common Stock 825,000,000 Shares" });
    expect(f.shares_company?.num).toBe(801351354);
    expect(f.shares_selling?.num).toBe(23648646);
    expect([f.price_low?.num, f.price_high?.num]).toEqual([70, 78]);
    expect(f.symbol?.text).toBe("ANT");
    expect(f.price_final).toBeUndefined();
    expect(ipo.filings.map((x) => x.filingDate)).toEqual([
      "2026-11-10",
      "2026-11-02",
      "2026-10-26",
      "2026-10-09",
      "2026-10-01",
      "2026-09-04",
      "2026-08-10",
      "2026-07-13",
      "2026-06-01",
    ]);
    expect(ipo.valuations.map((v) => [v.publishedOn, v.valueUsd, v.outlet])).toEqual([
      ["2026-11-10", 1.4e12, "Example News"],
      ["2026-10-12", 2e12, "Sample Times"],
    ]);
    expect(ipo.moves).toHaveLength(1);
    expect(ipo.views).toEqual([
      expect.objectContaining({ speaker: "Jane Doe, an analyst at Example Securities", statement: null, quote: VIEW_QUOTE }),
    ]);
    expect(ipo.trading).toBeNull();
  });

  it("reads Chinese names when set; v1 stores no statements, and a move's statement is still read if a row has one", async () => {
    const db = memoryD1(SEED);
    const zh = (await loadIpo(db, "anthropic", "zh"))!;
    expect(zh.name).toBe("Anthropic");
    // The seed, like v1, has no statement on any item; quotes stay as written.
    expect(zh.moves[0].statement).toBeNull();
    expect(zh.moves[0].quote).toMatch(/^Anthropic on Tuesday released/);
    expect((await loadIpo(db, "northwind-robotics", "zh"))!.name).toBe("北风机器人");
    // The columns remain: a move that carries statements shows the page's language, else English.
    const withBoth = memoryD1(`${SEED}\nUPDATE ipo_items SET statement_en = 'Released AI word-processing and presentation software for business customers.', statement_zh = '面向企业客户推出了 AI 文字处理和演示文稿软件。' WHERE slug = 'anthropic' AND kind = 'company_move';`);
    expect((await loadIpo(withBoth, "anthropic", "zh"))!.moves[0].statement).toBe("面向企业客户推出了 AI 文字处理和演示文稿软件。");
    expect((await loadIpo(withBoth, "anthropic", "en"))!.moves[0].statement).toBe("Released AI word-processing and presentation software for business customers.");
    const enOnly = memoryD1(`${SEED}\nUPDATE ipo_items SET statement_en = 'Released AI word-processing and presentation software for business customers.' WHERE slug = 'anthropic' AND kind = 'company_move';`);
    expect((await loadIpo(enOnly, "anthropic", "zh"))!.moves[0].statement).toBe("Released AI word-processing and presentation software for business customers.");
  });

  it("reads the first and the latest session, keeping the change only beside both inputs", async () => {
    const ipo = (await loadIpo(memoryD1(SEED), "northwind-robotics", "en"))!;
    expect(ipo.trading).toEqual({
      first: { sessionDate: "2026-11-05", open: null, close: 31.2, volume: 48210000, ipoPrice: 24, changePct: 30, sourceUrl: "https://finance.yahoo.com/quote/NWRB" },
      latest: { sessionDate: "2026-11-10", open: null, close: 28.45, volume: 9120000, ipoPrice: 24, changePct: 18.54, sourceUrl: "https://finance.yahoo.com/quote/NWRB" },
      // The date line is Yahoo's first session on Nov 5, the earliest stored: that session is day one.
      firstIsDayOne: true,
    });
    const noPrice = memoryD1(`${SEED}\nUPDATE ipo_trading SET ipo_price = NULL WHERE session_date = '2026-11-10';`);
    expect((await loadIpo(noPrice, "northwind-robotics", "en"))!.trading!.latest!.changePct).toBeNull();
  });

  it("is null for an unknown or malformed slug, without reading D1 for a malformed one", async () => {
    const db = memoryD1(SEED);
    expect(await loadIpo(db, "openai", "en")).toBeNull();
    const prepare = vi.spyOn(db, "prepare");
    for (const bad of ["Anthropic", "../etc", "a b", "-x", "x".repeat(65), ""]) expect(await loadIpo(db, bad, "en")).toBeNull();
    expect(prepare).not.toHaveBeenCalled();
  });

  it("hides malformed optional fields instead of failing", async () => {
    const db = memoryD1(`${SEED}
      UPDATE ipo_watch SET first_trade_date = '2026-13-45', exchange = '  ' WHERE slug = 'anthropic';
      UPDATE ipo_facts SET value_num = NULL WHERE kind = 'price_high' AND slug = 'anthropic';
      UPDATE ipo_facts SET value_num = -5 WHERE kind = 'shares_selling' AND slug = 'anthropic';
      UPDATE ipo_items SET published_on = 'yesterday' WHERE kind = 'company_move' AND slug = 'anthropic';
      UPDATE ipo_items SET value_usd = NULL WHERE url LIKE '%valuation' AND slug = 'anthropic';`);
    env.db = db;
    const ipo = (await loadIpo(db, "anthropic", "en"))!;
    expect(ipo.date).toBeNull();
    expect(ipo.exchange).toBeNull();
    // Half a range is no range.
    expect(ipo.facts!.facts.price_low).toBeUndefined();
    expect(ipo.facts!.facts.price_high).toBeUndefined();
    expect(ipo.facts!.facts.shares_selling).toBeUndefined();
    expect(ipo.facts!.facts.shares_offered?.num).toBe(825000000);
    expect(ipo.moves).toEqual([]);
    expect(ipo.valuations.map((v) => v.valueUsd)).toEqual([1.4e12, null]);
    await expect(page("anthropic")).resolves.toContain("First trading day: date not set.");
  });

  it("never shows another IPO's cover facts", async () => {
    const db = memoryD1(`${SEED}\nUPDATE ipo_watch SET facts_accession = '0001628280-26-072484' WHERE slug = 'harbor-bio';`);
    expect((await loadIpo(db, "harbor-bio", "en"))!.facts).toBeNull();
  });

  it("lists the calendar's IPOs by date, withdrawn ones left out", async () => {
    const db = memoryD1(`${SEED}\n${WITHDRAWN}\nUPDATE ipo_watch SET first_trade_date = '2026-11-17', date_status = 'reported', date_source = 'news' WHERE slug = 'harbor-bio';`);
    const week = await listIposBetween(db, "2026-11-16", "2026-11-20", "en");
    expect(week.map((i) => [i.slug, i.date?.date, i.date?.status])).toEqual([["anthropic", "2026-11-16", "reported"]]);
    expect(Object.keys(iposByDate(week))).toEqual(["2026-11-16"]);
    const earlier = await listIposBetween(db, "2026-11-02", "2026-11-06", "zh");
    expect(earlier.map((i) => [i.name, i.date?.status])).toEqual([["北风机器人", "confirmed"]]);
  });

  it("lists the watch: not yet listed, or listed in the last 30 days; dated first, then date not set", async () => {
    const db = memoryD1(SEED);
    const watch = await listIpoWatch(db, "2026-11-11", "en");
    expect(watch.map((i) => [i.slug, i.status, i.date?.date ?? null])).toEqual([
      ["northwind-robotics", "listed", "2026-11-05"],
      ["anthropic", "range_set", "2026-11-16"],
      ["harbor-bio", "filed", null],
    ]);
    // Over 30 days after its first trade a listed IPO leaves the list.
    expect((await listIpoWatch(db, "2026-12-06", "en")).map((i) => i.slug)).toEqual(["anthropic", "harbor-bio"]);
    expect((await listIpoWatch(memoryD1(`${SEED}\n${WITHDRAWN}`), "2026-11-11", "en")).map((i) => i.slug)).not.toContain("harbor-bio");
  });

  it("reads nothing, and does not fail, before migration 0009", async () => {
    const db: D1Like = {
      prepare() {
        throw new Error("no such table: ipo_watch");
      },
    };
    expect(await listIposBetween(db, "2026-11-16", "2026-11-20", "en")).toEqual([]);
    expect(await listIpoWatch(db, "2026-11-11", "en")).toEqual([]);
    expect(await listIpoSlugs(db)).toEqual([]);
  });

  it("keeps only https links", () => {
    expect(httpsUrl("https://news.example/a?b=1")).toBe("https://news.example/a?b=1");
    for (const bad of ["javascript:alert(1)", "http://news.example/a", "JAVASCRIPT:alert(1)", " javascript:x", "data:text/html,x", "//news.example/a", "https://", "https://a b.example", 42, null])
      expect(httpsUrl(bad), String(bad)).toBeNull();
  });

  it("names forms from the contract's glossary and writes valuations in words", () => {
    expect(formLabel("S-1/A", "en")).toBe("Amended registration statement (S-1/A)");
    expect(formLabel("424B4", "zh")).toBe("最终招股说明书");
    expect(formLabel("10-K", "en")).toBeNull();
    expect(formLabel("constructor", "en")).toBeNull();
    expect(fmtValuation(1.4e12, "en")).toBe("$1.4 trillion");
    expect(fmtValuation(2e12, "en")).toBe("$2 trillion");
    expect(fmtValuation(965e9, "en")).toBe("$965 billion");
    expect(fmtValuation(1.4e12, "zh")).toBe("1.4万亿美元");
    expect(fmtValuation(965e9, "zh")).toBe("9650亿美元");
  });
});

describe("calendar with IPOs", () => {
  it("shows an unconfirmed IPO dashed on its day, linked to its page, and the watch list above the board", async () => {
    const out = await calendar("2026-11-16");
    const day = out.slice(out.indexOf('id="d-2026-11-16"'), out.indexOf('id="d-2026-11-17"'));
    expect(day).toContain('<div class="cal-ipo"><h3 class="cal-slot">IPO</h3>');
    expect(day).toContain(
      '<li class="cal-card cal-card-text cal-unconfirmed"><a class="cal-card-link" title="Anthropic" href="/en/ipo/anthropic"><span class="cal-card-body"><span class="cal-card-sym" aria-hidden="true">ANT</span><span class="cal-card-name">Anthropic</span></span><span class="sr-only">Unconfirmed</span><span class="sr-only">IPO page</span></a></li>',
    );
    // Only on its own day.
    expect(out.match(/class="cal-ipo"/g)).toHaveLength(1);
    // The watch list sits between the head and the board.
    expect(out.indexOf('<section class="cal-ipo-watch"')).toBeGreaterThan(out.indexOf("</header>"));
    expect(out.indexOf('<section class="cal-ipo-watch"')).toBeLessThan(out.indexOf('<div class="cal-week"'));
    const list = out.slice(out.indexOf('<section class="cal-ipo-watch"'), out.indexOf("</section>", out.indexOf('<section class="cal-ipo-watch"')));
    expect(textOf(list)).toBe(
      "IPO watch" +
        "Northwind RoboticsNWRBListedNov 5, 2026" +
        "AnthropicANTPrice range setNov 16, 2026Unconfirmed" +
        "Harbor BioHRBOFiledDate not set",
    );
    expect(list).toContain('<a class="cal-ipo-watch-name" href="/en/ipo/anthropic">Anthropic</a>');
    expect(out).toContain("each links to its IPO page.");
  });

  it("shows a confirmed IPO solid", async () => {
    const out = await calendar("2026-11-02");
    expect(out).toContain('<li class="cal-card cal-card-text cal-confirmed"><a class="cal-card-link" title="Northwind Robotics" href="/en/ipo/northwind-robotics">');
    expect(out).toContain('<span class="sr-only">Confirmed</span><span class="sr-only">IPO page</span>');
    expect(out).not.toContain("cal-unconfirmed");
  });

  it("in Chinese: the Chinese name, labels and date not set", async () => {
    const out = await calendar("2026-11-02", "zh");
    expect(out).toContain('title="北风机器人" href="/zh/ipo/northwind-robotics"');
    expect(out).toContain('<h2 class="cal-slot" id="cal-ipo-watch">IPO 追踪</h2>');
    expect(out).toContain('<span class="cal-ipo-watch-status">已定发行价区间</span>');
    expect(out).toContain('<span class="cal-ipo-watch-date">日期未定</span>');
    expect(out).toContain("<time dateTime=\"2026-11-16\">2026年11月16日</time><span class=\"ipo-flag\">未确认</span>");
  });

  it("shows nothing about IPOs when none is tracked", async () => {
    env.db = memoryD1("");
    const out = await calendar("2026-11-16");
    expect(out).not.toMatch(/ipo/i);
    expect(out).toContain('<div class="cal-board">');
  });

  it("hides a withdrawn IPO from the board and the list", async () => {
    env.db = memoryD1(`${SEED}\nUPDATE ipo_watch SET status = 'withdrawn' WHERE slug = 'anthropic';`);
    const out = await calendar("2026-11-16");
    expect(out).not.toContain("/en/ipo/anthropic");
    expect(out).not.toContain('class="cal-ipo"');
    expect(out).toContain("/en/ipo/harbor-bio");
  });
});

describe("IPO page", () => {
  it("range_set (Anthropic): header, dated source line, the offering with its filing, timeline, reported items and the disclaimer", async () => {
    const out = await page("anthropic");
    const text = textOf(out);
    expect(out.startsWith('<article class="prose prose-ipo"><p class="kicker">IPO watch</p><h1>Anthropic IPO</h1>')).toBe(true);
    expect(out).toContain('<p class="ipo-meta"><span>ANT · Nasdaq Global Select Market</span><span class="ipo-status">Price range set</span></p>');
    expect(text).toContain("Unconfirmed Expected to start trading Mon, Nov 16, 2026, as reported by Example News on Nov 10, 2026. Article ↗");
    expect(out).toContain('<a class="more" href="https://news.example/2026/11/10/anthropic-ipo-pricing" rel="nofollow noopener noreferrer">Article ↗</a>');
    expect(text).toContain("The source's words“Anthropic is expected to begin trading on the Nasdaq on Nov. 16");
    // The offering: from the S-1/A, with its link; every row with the cover's sentence.
    expect(text).toContain("From the S-1/A filed Nov 10, 2026 · SEC filing ↗");
    expect(out).toContain(`<a href="${ANTHROPIC_FILING}" rel="noopener noreferrer">SEC filing ↗</a>`);
    for (const [label, value] of [
      ["Exchange", "Nasdaq Global Select Market"],
      ["Symbol", "ANT"],
      ["Shares offered", "825,000,000 shares"],
      ["Sold by the company", "801,351,354 shares"],
      ["Sold by selling stockholders", "23,648,646 shares"],
      ["Underwriters’ option", "up to 123,750,000 more shares"],
      ["Price range", "$70.00 to $78.00 per share"],
    ])
      expect(out, label).toContain(`<dt>${label}</dt><dd><span class="ipo-value">${value}</span><details class="ipo-src-quote"><summary>Filing text</summary><blockquote class="ipo-quote">“`);
    expect(text).toContain("Filing text“It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00.”");
    expect(out.match(/<details class="ipo-src-quote">/g)).toHaveLength(8);
    expect(out).not.toContain("IPO price");
    // Timeline: glossary names, newest first, each linked.
    expect(text).toContain("Nov 10, 2026 Amended registration statement (S-1/A)Filing ↗");
    expect(text).toContain("Jun 1, 2026 Draft registration statement (confidential)");
    expect(out.match(/class="date-row"/g)).toHaveLength(9);
    // Valuation history: each figure on its own dated line, newest first.
    const val = text.slice(text.indexOf("Valuation as reported"), text.indexOf("Recent moves"));
    expect(val.indexOf("$1.4 trillionReported by Example News on Nov 10, 2026")).toBeGreaterThan(0);
    expect(val.indexOf("$2 trillionReported by Sample Times on Oct 12, 2026")).toBeGreaterThan(val.indexOf("$1.4 trillion"));
    expect(val).toContain("“At the top of the range the offering would value Anthropic at roughly $1.4 trillion, the people said.”");
    expect(text).toContain(
      "Recent movesSample Times reported on Nov 4, 2026:“Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers.”Article ↗",
    );
    expect(out).not.toContain("After listing");
    expect(text).toContain(
      "This page is generated automatically from SEC filings and public news reports for information only. It is not investment advice. Valuations, dates and company news before listing are as reported by the named sources and may be wrong.",
    );
    expect(out).toContain('<a class="more" href="/en/earnings">Market calendar →</a>');
  });

  it("prints each valuation and never a change between them", async () => {
    const text = textOf(await page("anthropic"));
    expect(text).toContain("$1.4 trillion");
    expect(text).toContain("$2 trillion");
    expect(text).not.toMatch(/%|\bpercent|\bfell\b|\brose\b|\bdown from\b|\bup from\b|\blower than\b|\bhigher than\b|\bcut\b/i);
    expect(textOf(await page("anthropic", "zh"))).not.toMatch(/%|下降|上升|下跌|上涨|减少|增加|下调|上调/);
  });

  it("shows a view only as a quotation, after its speaker and outlet", async () => {
    const out = await page("anthropic");
    const text = textOf(out);
    expect(text.split(VIEW_QUOTE)).toHaveLength(2);
    expect(text).toContain(`What others sayJane Doe, an analyst at Example Securities, quoted by Sample Times on Nov 9, 2026:“${VIEW_QUOTE}”`);
    expect(out).toContain(`<blockquote class="ipo-quote">“${VIEW_QUOTE}”</blockquote>`);
    const zh = textOf(await page("anthropic", "zh"));
    expect(zh).toContain(`各方观点Sample Times 2026年11月9日引述 Jane Doe, an analyst at Example Securities：“${VIEW_QUOTE}”`);
    // Without views the section is left out.
    env.db = memoryD1(`${SEED}\nDELETE FROM ipo_items WHERE kind = 'view';`);
    expect(await page("anthropic")).not.toContain("What others say");
  });

  it("shows at most three views, the newest", async () => {
    const more = [1, 2, 3, 4]
      .map((n) => `('anthropic','view','https://v.example/${n}','Outlet ${n}','2026-11-0${n}','Quote ${n}',NULL,NULL,'Speaker ${n}',NULL,NULL,'x')`)
      .join(",");
    env.db = memoryD1(`${SEED}\nDELETE FROM ipo_items WHERE kind = 'view';\nINSERT INTO ipo_items VALUES ${more};`);
    const text = textOf(await page("anthropic"));
    expect(text).toContain("“Quote 4”");
    expect(text).toContain("“Quote 2”");
    expect(text).not.toContain("“Quote 1”");
  });

  it("range_set in Chinese: Chinese labels and statements, the glossary's Chinese, quotes as written", async () => {
    const out = await page("anthropic", "zh");
    const text = textOf(out);
    expect(out).toContain('<p class="kicker">IPO 追踪</p><h1>Anthropic IPO</h1>');
    expect(out).toContain('<span class="ipo-status">已定发行价区间</span>');
    expect(text).toContain("未确认 预计2026年11月16日开始交易，据 Example News 2026年11月10日报道。 原文 ↗");
    expect(text).toContain("出自2026年11月10日提交的S-1/A · SEC 文件 ↗");
    expect(out).toContain('<dt>发行股数</dt><dd><span class="ipo-value">825,000,000股</span>');
    expect(out).toContain('<dt>发行价区间</dt><dd><span class="ipo-value">每股70.00至78.00美元</span>');
    expect(text).toContain("2026年11月10日 注册声明修订（S-1/A）文件 ↗");
    expect(text).toContain("1.4万亿美元Example News 2026年11月10日报道");
    expect(text).toContain(
      "公司近期动态Sample Times 2026年11月4日报道：“Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers.”原文 ↗",
    );
    expect(text).toContain("“Anthropic on Tuesday released AI versions of word-processing and presentation software for business customers.”");
    expect(text).toContain("本页根据美国证监会文件和公开新闻报道自动生成，仅供参考，不构成投资建议。");
  });

  it("filed (Harbor Bio): no date, the facts it has and a plain line that shares and price are not available yet", async () => {
    for (const [locale, notSet, missing] of [
      ["en", "First trading day: date not set.", "Shares and price are not available yet."],
      ["zh", "首个交易日：日期未定。", "发行股数和价格暂未公布。"],
    ] as const) {
      const out = await page("harbor-bio", locale);
      expect(out).toContain(`<p class="ipo-date">${notSet}</p>`);
      expect(out).toContain(missing);
      expect(out).toContain(locale === "en" ? "<dd><span class=\"ipo-value\">Nasdaq Global Market</span>" : "<dt>交易所</dt>");
      expect(out).toContain(locale === "en" ? '<span class="ipo-status">Filed</span>' : '<span class="ipo-status">已提交注册</span>');
      expect(out).not.toMatch(/Valuation as reported|媒体报道的估值|After listing|上市后表现/);
    }
  });

  it("says plainly when no offering facts are available", async () => {
    env.db = memoryD1(`${SEED}\nUPDATE ipo_watch SET facts_accession = NULL WHERE slug = 'harbor-bio';`);
    expect(await page("harbor-bio")).toContain("<h2 id=\"offering\">The offering</h2><p>The offering&#x27;s terms are not available yet.</p>");
    expect(await page("harbor-bio", "zh")).toContain("<p>发行条款暂未公布。</p>");
  });

  it("priced: the final price from the 424B4 and the expected date by rule, still unconfirmed", async () => {
    env.db = memoryD1(`${SEED}\n${PRICED}`);
    for (const [locale, status, line, price] of [
      ["en", "Priced", "Unconfirmed Expected to start trading Thu, Nov 5, 2026: the trading day after pricing (424B4 filed Nov 5, 2026). SEC filing ↗", "$24.00 per share"],
      ["zh", "已定价", "未确认 预计2026年11月5日开始交易：定价后的下一个交易日（2026年11月5日提交的 424B4）。 SEC 文件 ↗", "每股24.00美元"],
    ] as const) {
      const out = await page("northwind-robotics", locale);
      const text = textOf(out);
      expect(out).toContain(`<span class="ipo-status">${status}</span>`);
      expect(text).toContain(line);
      expect(out).toContain(`<span class="ipo-value">${price}</span>`);
      expect(text).toContain(locale === "en" ? "From the 424B4 filed Nov 5, 2026" : "出自2026年11月5日提交的424B4");
      expect(text).toContain(locale === "en" ? "PricedNov 4, 2026" : "定价日期2026年11月4日");
      expect(out).not.toMatch(/After listing|上市后表现/);
    }
  });

  it("listed (Northwind): first day and latest close, volume and the change beside both inputs, Yahoo named", async () => {
    const out = await page("northwind-robotics");
    const text = textOf(out);
    expect(out).toContain('<span class="ipo-status">Listed</span>');
    expect(text).toContain("Confirmed First trading day Thu, Nov 5, 2026: its first regular session, per Yahoo Finance. Source: Yahoo Finance ↗");
    const after = text.slice(text.indexOf("After listing"));
    expect(after).toContain("First trading day · Thu, Nov 5Close$31.20Volume48,210,000 sharesChange from IPO price+30.00% (close $31.20 vs. IPO price $24.00)");
    expect(after).toContain("Latest · Tue, Nov 10Close$28.45Volume9,120,000 sharesChange from IPO price+18.54% (close $28.45 vs. IPO price $24.00)");
    expect(out).toContain('<a href="https://finance.yahoo.com/quote/NWRB" rel="noopener noreferrer">Source: Yahoo Finance ↗</a>');
    expect(out).toContain('<span class="num up">+30.00%</span>');
    // Open is never stored in v1, so it is not shown.
    expect(after).not.toContain("Open");
    const zh = textOf(await page("northwind-robotics", "zh"));
    expect(zh).toContain("北风机器人 IPO");
    expect(zh).toContain("首个交易日 · 11月5日 周四收盘价31.20美元成交量48,210,000股较发行价涨跌+30.00%（收盘价31.20美元，发行价24.00美元）");
    expect(zh).toContain("来源：Yahoo Finance ↗");
  });

  it("listed with one session: the first day alone; without the IPO price, no change", async () => {
    env.db = memoryD1(`${SEED}\nDELETE FROM ipo_trading WHERE session_date = '2026-11-10';\nUPDATE ipo_trading SET ipo_price = NULL, open = 25.5;`);
    const text = textOf(await page("northwind-robotics"));
    expect(text).toContain("First trading day · Thu, Nov 5Open$25.50Close$31.20Volume48,210,000 shares");
    expect(text).not.toContain("Latest ·");
    expect(text).not.toContain("Change from IPO price");
  });

  it("withdrawn: says so at the top and still renders what it has", async () => {
    env.db = memoryD1(`${SEED}\n${WITHDRAWN}`);
    const en = await page("harbor-bio");
    expect(en).toContain(
      '<p class="notice ipo-withdrawn">This IPO has been withdrawn: the company filed to withdraw its registration statement on Nov 10, 2026.</p>',
    );
    expect(en.indexOf("ipo-withdrawn")).toBeLessThan(en.indexOf("The offering"));
    expect(textOf(en)).toContain("Nov 10, 2026 Registration withdrawnFiling ↗");
    const zh = await page("harbor-bio", "zh");
    expect(zh).toContain('<span class="ipo-status">已撤回</span>');
    expect(zh).toContain("此次 IPO 已撤回：公司已于2026年11月10日提交撤回注册申请。");
    expect(textOf(zh)).toContain("撤回注册");
  });

  it("renders a hostile title, outlet, quote and links as inert text", async () => {
    env.db = memoryD1(`${SEED}\n${EVIL}`);
    for (const locale of ["en", "zh"] as const) {
      const out = await page("evil-co", locale);
      expect(out).not.toMatch(/<script|<img|<svg|<b>|<i>|<a href="javascript/i);
      expect(out).not.toMatch(/href="(?!https:\/\/|\/)/);
      expect(out).not.toContain('href="http://');
      expect(out).not.toMatch(/(href|src)=["']?\s*javascript/i);
      // The text is still shown, escaped.
      expect(out).toContain("&lt;script&gt;alert(&quot;outlet&quot;)&lt;/script&gt;");
      expect(out).toContain("“&lt;/blockquote&gt;&lt;script&gt;alert(3)&lt;/script&gt;”");
      expect(out).toContain("&lt;svg onload=alert(1)&gt;");
      // The insecure article keeps its attribution but loses its link.
      expect(textOf(out)).toContain("Plain Outlet");
      expect(out).not.toContain("insecure.example");
    }
    expect(await page("evil-co")).toContain("&lt;script&gt;alert(&quot;name&quot;)&lt;/script&gt; IPO</h1>");
    expect(await page("evil-co", "zh")).toContain("&lt;img src=x onerror=alert(1)&gt; IPO</h1>");
    // The calendar too.
    const cal = await calendar("2026-11-16");
    expect(cal).not.toMatch(/<script|<img src=x|<i>EVL/);
    expect(cal).toContain("&lt;i&gt;EVL&lt;/i&gt;");
  });

  it("is a 404 for an unknown slug, with no metadata", async () => {
    await expect(IpoPage(props({ locale: "en", slug: "openai" }))).rejects.toMatchObject({ digest: "NEXT_HTTP_ERROR_FALLBACK;404" });
    await expect(IpoPage(props({ locale: "zh", slug: "../x" }))).rejects.toMatchObject({ digest: "NEXT_HTTP_ERROR_FALLBACK;404" });
    expect(await generateMetadata(props({ locale: "en", slug: "openai" }))).toEqual({});
  });

  it("pairs the two languages and sets share metadata", async () => {
    const en = await generateMetadata(props({ locale: "en", slug: "anthropic" }));
    expect(en).toMatchObject({
      title: "Anthropic IPO",
      alternates: {
        canonical: "/en/ipo/anthropic",
        languages: { en: "/en/ipo/anthropic", "zh-Hans": "/zh/ipo/anthropic", "x-default": "/en/ipo/anthropic" },
      },
      openGraph: { url: "/en/ipo/anthropic", title: "Anthropic IPO", locale: "en_US", type: "website", siteName: "PreBell" },
      twitter: { card: "summary_large_image", title: "Anthropic IPO" },
    });
    expect(en.description).toMatch(/^Anthropic's IPO: the offering from its SEC filings/);
    const zh = await generateMetadata(props({ locale: "zh", slug: "northwind-robotics" }));
    expect(zh).toMatchObject({ title: "北风机器人 IPO", alternates: { canonical: "/zh/ipo/northwind-robotics" }, openGraph: { locale: "zh_CN" } });
  });

  it("reads D1 five times and R2 never", async () => {
    const db = memoryD1(SEED);
    const prepare = vi.spyOn(db, "prepare");
    await loadIpo(db, "anthropic", "en");
    expect(prepare).toHaveBeenCalledTimes(5);
  });
});

describe("IPO sitemap entries", () => {
  it("lists each tracked IPO in both languages with its update time, after the fixed pages", async () => {
    const entries = await sitemapEntries(memoryD1(SEED), new URL("https://prebell.site"));
    const urls = entries.map((e) => e.url.replace("https://prebell.site", ""));
    expect(urls.slice(-6)).toEqual([
      "/en/ipo/anthropic",
      "/zh/ipo/anthropic",
      "/en/ipo/harbor-bio",
      "/zh/ipo/harbor-bio",
      "/en/ipo/northwind-robotics",
      "/zh/ipo/northwind-robotics",
    ]);
    const anthropic = entries.find((e) => e.url === "https://prebell.site/zh/ipo/anthropic")!;
    expect(anthropic.lastModified).toBe("2026-11-11T15:00:00Z");
    expect(anthropic.alternates).toEqual({
      languages: {
        en: "https://prebell.site/en/ipo/anthropic",
        "zh-Hans": "https://prebell.site/zh/ipo/anthropic",
        "x-default": "https://prebell.site/en/ipo/anthropic",
      },
    });
  });

  it("has no IPO entries when none is tracked", async () => {
    const entries = await sitemapEntries(memoryD1(""), new URL("https://prebell.site"));
    expect(entries.some((e) => e.url.includes("/ipo/"))).toBe(false);
  });
});

describe("review fixes: what the reader refuses to print", () => {
  const watch = (set: string, slug = "anthropic") =>
    memoryD1(`${SEED}\n${UNCHECKED(`UPDATE ipo_watch SET ${set} WHERE slug = '${slug}';`)}`);

  it("links filings only on www.sec.gov and the trading source only on finance.yahoo.com, against host tricks", () => {
    expect(secUrl(ANTHROPIC_FILING)).toBe(ANTHROPIC_FILING);
    expect(yahooUrl("https://finance.yahoo.com/quote/NWRB")).toBe("https://finance.yahoo.com/quote/NWRB");
    for (const [check, host] of [
      [secUrl, "www.sec.gov"],
      [yahooUrl, "finance.yahoo.com"],
    ] as const) {
      for (const bad of [
        `https://${host}.evil.example/x`,
        `https://evil.example/?u=https://${host}/x`,
        `https://evil.example/https://${host}/x`,
        `https://${host}@evil.example/x`,
        `https://${host}:443@evil.example/x`,
        `https://user:pass@${host}/x`,
        `https://${host}:8443/x`,
        `http://${host}/x`,
        `https://${host}`,
        `https://${host.toUpperCase()}/x`,
        `https://x${host}/x`,
        "javascript:alert(1)",
      ])
        expect(check(bad), bad).toBeNull();
    }
  });

  it("checks the date line's link by its source, and drops only the link when it fails", async () => {
    const cases: [string, string, string | null][] = [
      // news: any https article
      ["date_source_url = 'https://www.sec.gov/x'", "anthropic", "https://www.sec.gov/x"],
      ["date_source_url = 'javascript:alert(1)'", "anthropic", null],
      ["date_source_url = 'http://news.example/a'", "anthropic", null],
      // filing (the priced fixture): www.sec.gov only
      [`status = 'priced', date_status = 'expected', date_source = 'filing', date_source_name = '424B4', date_source_url = 'https://evil.example/x-index.htm'`, "northwind-robotics", null],
      [`status = 'priced', date_status = 'expected', date_source = 'filing', date_source_name = '424B4', date_source_url = '${ANTHROPIC_FILING}'`, "northwind-robotics", ANTHROPIC_FILING],
      // trading: finance.yahoo.com only
      ["date_source_url = 'https://evil.example/quote/NWRB'", "northwind-robotics", null],
      ["date_source_url = 'https://finance.yahoo.com.evil.example/quote/NWRB'", "northwind-robotics", null],
      ["date_source_url = 'https://finance.yahoo.com/quote/NWRB'", "northwind-robotics", "https://finance.yahoo.com/quote/NWRB"],
    ];
    for (const [set, slug, url] of cases) {
      const ipo = (await loadIpo(watch(set, slug), slug, "en"))!;
      expect(ipo.date, set).not.toBeNull();
      expect(ipo.date!.sourceUrl, set).toBe(url);
    }
    env.db = watch("date_source_url = 'https://evil.example/quote/NWRB'", "northwind-robotics");
    const out = await page("northwind-robotics");
    expect(out).not.toContain("evil.example");
    expect(textOf(out)).toContain("Confirmed First trading day Thu, Nov 5, 2026: its first regular session, per Yahoo Finance.");
  });

  it("shows a date only for the contract's status and source pairs, and only with a named source", async () => {
    for (const [status, source, ok] of [
      ["reported", "news", true],
      ["expected", "filing", true],
      ["confirmed", "filing", true],
      ["confirmed", "trading", true],
      ["confirmed", "news", false],
      ["expected", "news", false],
      ["reported", "filing", false],
      ["reported", "trading", false],
      ["expected", "trading", false],
    ] as const) {
      // A trading date also needs Yahoo's first session on that day (the first-day check).
      const session =
        source === "trading"
          ? `INSERT INTO ipo_trading (slug, session_date, symbol, open, close, volume, ipo_price, change_pct, source_url, fetched_at) VALUES ('anthropic', '2026-11-16', 'ANT', NULL, 80, 1, 75, NULL, 'https://finance.yahoo.com/quote/ANT', 'x');`
          : "";
      const db = memoryD1(`${SEED}\n${session}\n${UNCHECKED(`UPDATE ipo_watch SET date_status = '${status}', date_source = '${source}' WHERE slug = 'anthropic';`)}`);
      const ipo = (await loadIpo(db, "anthropic", "en"))!;
      expect(ipo.date !== null, `${status}+${source}`).toBe(ok);
      const week = await listIposBetween(db, "2026-11-16", "2026-11-20", "en");
      expect(week.length, `${status}+${source} on the calendar`).toBe(ok ? 1 : 0);
    }
    // A news date that names no outlet is not shown.
    expect((await loadIpo(watch("date_source_name = NULL"), "anthropic", "en"))!.date).toBeNull();
    // On the page and the calendar, a refused pair reads "date not set" and draws no card.
    env.db = watch("date_status = 'confirmed', date_source = 'news'");
    expect(await page("anthropic")).toContain('<p class="ipo-date">First trading day: date not set.</p>');
    const cal = await calendar("2026-11-16");
    expect(cal).not.toContain('class="cal-ipo"');
    expect(textOf(cal)).toContain("AnthropicANTPrice range setDate not set");
  });

  it("a withdrawn IPO shows no date even when one is stored", async () => {
    env.db = memoryD1(`${SEED}\n${WITHDRAWN}\nUPDATE ipo_watch SET first_trade_date = '2026-11-20', date_status = 'reported', date_source = 'news',
      date_source_name = 'Example News', date_as_of = '2026-11-09' WHERE slug = 'harbor-bio';`);
    expect((await loadIpo(env.db as D1Like, "harbor-bio", "en"))!.date).toBeNull();
    const out = await page("harbor-bio");
    expect(out).not.toContain("ipo-date");
    expect(out).not.toContain("Expected to start trading");
  });

  it("never shows a later session as the first day: a malformed earliest row hides the first day", async () => {
    env.db = memoryD1(`${SEED}\nUPDATE ipo_trading SET close = 0 WHERE session_date = '2026-11-05';`);
    const ipo = (await loadIpo(env.db as D1Like, "northwind-robotics", "en"))!;
    expect(ipo.trading!.first).toBeNull();
    expect(ipo.trading!.latest!.sessionDate).toBe("2026-11-10");
    const text = textOf(await page("northwind-robotics"));
    const after = text.slice(text.indexOf("After listing"));
    expect(after).not.toMatch(/First|Latest/);
    // With the first session unknown, the one shown is labelled by its date only.
    expect(after).toContain("After listingNov 10, 2026Close$28.45");
    // A malformed latest row hides only the latest.
    const bad = memoryD1(`${SEED}\nUPDATE ipo_trading SET close = -3 WHERE session_date = '2026-11-10';`);
    const t2 = (await loadIpo(bad, "northwind-robotics", "en"))!.trading!;
    expect([t2.first?.sessionDate, t2.latest]).toEqual(["2026-11-05", null]);
    // Both malformed: no section.
    const none = memoryD1(`${SEED}\nUPDATE ipo_trading SET close = 0;`);
    expect((await loadIpo(none, "northwind-robotics", "en"))!.trading).toBeNull();
  });

  it("shows no cover facts without the filing they came from", async () => {
    env.db = memoryD1(`${SEED}\nDELETE FROM ipo_filings WHERE accession = '0001628280-26-072484';`);
    expect((await loadIpo(env.db as D1Like, "anthropic", "en"))!.facts).toBeNull();
    const out = await page("anthropic");
    expect(out).toContain("The offering&#x27;s terms are not available yet.");
    expect(out).not.toContain("825,000,000");
    // A filing row with a malformed date is no source either.
    const bad = memoryD1(`${SEED}\nUPDATE ipo_filings SET filing_date = 'soon' WHERE accession = '0001628280-26-072484';`);
    expect((await loadIpo(bad, "anthropic", "en"))!.facts).toBeNull();
  });

  it("computes the change from the close and IPO price shown beside it, never the stored one", async () => {
    const at = async (close: number, ipo: number | null, stored: number | null = 99) => {
      const db = memoryD1(`${SEED}\nUPDATE ipo_trading SET close = ${close}, ipo_price = ${ipo ?? "NULL"}, change_pct = ${stored ?? "NULL"} WHERE session_date = '2026-11-05';`);
      return (await loadIpo(db, "northwind-robotics", "en"))!.trading!.first!.changePct;
    };
    expect(await at(31.2, 24)).toBe(30);
    expect(await at(28.45, 24)).toBe(18.54);
    expect(await at(20, 24)).toBe(-16.67);
    expect(await at(24, 24)).toBe(0);
    expect(await at(30.0003, 24)).toBe(25);
    // From the close as printed: $24.004 prints as $24.00, so the change is 0.00%, not +0.02%.
    expect(await at(24.004, 24)).toBe(0);
    expect(await at(24.006, 24)).toBe(0.04);
    expect(await at(31.2, 24, null)).toBe(30);
    expect(await at(31.2, null)).toBeNull();
    expect(await at(31.2, 0)).toBeNull();
    expect(await at(31.2, -24)).toBeNull();
    env.db = memoryD1(`${SEED}\nUPDATE ipo_trading SET close = 20, change_pct = 30 WHERE session_date = '2026-11-05';`);
    const out = await page("northwind-robotics");
    expect(out).toContain('<span class="num down">−16.67%</span>');
    expect(textOf(out)).toContain("(close $20.00 vs. IPO price $24.00)");
    expect(out).not.toContain("+30.00%");
  });

  it("drops a view with no speaker, and any item with no outlet", async () => {
    env.db = memoryD1(`${SEED}\nUPDATE ipo_items SET speaker = '  ' WHERE kind = 'view';\nUPDATE ipo_items SET outlet = ' ' WHERE kind = 'company_move';`);
    const ipo = (await loadIpo(env.db as D1Like, "anthropic", "en"))!;
    expect(ipo.views).toEqual([]);
    expect(ipo.moves).toEqual([]);
    const out = await page("anthropic");
    expect(out).not.toContain("What others say");
    expect(out).not.toContain("Recent moves");
  });

  it("writes valuations as stored, the unit from the raw value, promoted only when the printed number reaches it", () => {
    const table: [number, string | null, string | null][] = [
      [1.4e12, "$1.4 trillion", "1.4万亿美元"],
      [1.45e12, "$1.45 trillion", "1.45万亿美元"],
      [1.375e12, "$1.375 trillion", "1.375万亿美元"],
      [2e12, "$2 trillion", "2万亿美元"],
      [965e9, "$965 billion", "9650亿美元"],
      [999.6e9, "$999.6 billion", "9996亿美元"],
      [29.25e9, "$29.25 billion", "292.5亿美元"],
      [1.2345e12, "$1.2345 trillion", "1.2345万亿美元"],
      [61.5e9, "$61.5 billion", "615亿美元"],
      [1.5e9, "$1.5 billion", "15亿美元"],
      [250e6, "$250 million", "2.5亿美元"],
      [5e6, "$5 million", "500万美元"],
      [1e6, "$1 million", "100万美元"],
      [1e14, "$100 trillion", "100万亿美元"],
      // The promotion edge: float noise that prints as 1000 (10000) moves up a unit, never "$1,000 billion".
      [999_999_999_999, "$1 trillion", "1万亿美元"],
      [999.9996e9, "$1 trillion", "1万亿美元"],
      [999_999_999.9, "$1 billion", "10亿美元"],
      [99_999_999.99, "$100 million", "1亿美元"],
      [999.999e9, "$999.999 billion", "9999.99亿美元"],
      [999_999, null, null],
      [0, null, null],
      [-1.4e12, null, null],
      [1.01e14, null, null],
      [1e21, null, null],
      [Number.NaN, null, null],
      [Number.POSITIVE_INFINITY, null, null],
    ];
    for (const [usd, en, zh] of table) {
      expect(fmtValuation(usd, "en"), String(usd)).toBe(en);
      expect(fmtValuation(usd, "zh"), String(usd)).toBe(zh);
      // Never a number that has reached the next unit: four integer digits in English, five in Chinese.
      expect(String(fmtValuation(usd, "en"))).not.toMatch(/^\$(\d{4}|\d,\d{3})/);
      expect(String(fmtValuation(usd, "zh"))).not.toMatch(/^\d{5}/);
    }
  });

  it("an implausible valuation shows no figure, only its attributed quote", async () => {
    env.db = memoryD1(`${SEED}\nUPDATE ipo_items SET value_usd = 1e21 WHERE kind = 'valuation' AND url LIKE '%sampletimes%';`);
    const text = textOf(await page("anthropic"));
    expect(text).not.toContain("trillion trillion");
    expect(text).not.toContain("$1,000,000,000");
    expect(text).toContain("Sample Times reported on Oct 12, 2026:“Bankers have pitched a valuation of about $2 trillion");
    expect(text).toContain("$1.4 trillionReported by Example News");
  });

  it("says shares and price are not available only when no share or price row is shown", async () => {
    env.db = memoryD1(`${SEED}\nDELETE FROM ipo_facts WHERE slug = 'anthropic' AND kind NOT IN ('shares_company', 'exchange');`);
    const out = await page("anthropic");
    expect(out).toContain("<dt>Sold by the company</dt>");
    expect(out).not.toContain("Shares and price are not available yet.");
    // Harbor Bio has only the exchange and symbol rows: the line stays.
    expect(await page("harbor-bio")).toContain("Shares and price are not available yet.");
  });

  it("uses the site's Chinese money style (X美元) for every amount on a Chinese page", async () => {
    // Quotes stay as the filing or article wrote them; PreBell's own figures are what is checked here.
    const html = await page("northwind-robotics", "zh");
    const out = textOf(html);
    expect(out).toContain("收盘价31.20美元");
    expect(out).toContain("每股24.00美元");
    expect(out).toContain("（收盘价31.20美元，发行价24.00美元）");
    const own = html.replace(/<blockquote[^>]*>[\s\S]*?<\/blockquote>/g, "");
    expect(textOf(own)).not.toMatch(/\$\d/);
    const anthropic = (await page("anthropic", "zh")).replace(/<blockquote[^>]*>[\s\S]*?<\/blockquote>/g, "");
    expect(textOf(anthropic)).not.toMatch(/\$\d/);
  });

  it("gives the sitemap only a well-formed UTC lastmod", async () => {
    const db = memoryD1(`${SEED}\nUPDATE ipo_watch SET updated_at = 'yesterday' WHERE slug = 'harbor-bio';
      UPDATE ipo_watch SET updated_at = '2026-13-01T00:00:00Z' WHERE slug = 'northwind-robotics';`);
    const slugs = await listIpoSlugs(db);
    expect(slugs).toEqual([
      { slug: "anthropic", updatedAt: "2026-11-11T15:00:00Z" },
      { slug: "harbor-bio", updatedAt: null },
      { slug: "northwind-robotics", updatedAt: null },
    ]);
    const entries = await sitemapEntries(db, new URL("https://prebell.site"));
    expect(entries.find((e) => e.url === "https://prebell.site/en/ipo/harbor-bio")!.lastModified).toBeUndefined();
  });
});

describe("contract revision: statements, first trading day, passed reported dates", () => {
  const db = (sql = "") => memoryD1(`${SEED}\n${sql}`);
  /** Northwind as if the recap missed Nov 5: the filing's expected date stays, sessions start Nov 6. */
  const MISSED_DAY_ONE = `
    UPDATE ipo_watch SET date_status = 'expected', date_source = 'filing', date_source_name = '424B4',
      date_source_url = 'https://www.sec.gov/Archives/edgar/data/9999990001/999999000126000020/9999990001-26-000020-index.htm',
      date_quote = 'Prospectus dated November 4, 2026' WHERE slug = 'northwind-robotics';
    DELETE FROM ipo_trading WHERE session_date = '2026-11-05';
    INSERT INTO ipo_trading (slug, session_date, symbol, open, close, volume, ipo_price, change_pct, source_url, fetched_at) VALUES
      ('northwind-robotics', '2026-11-06', 'NWRB', NULL, 30.1, 20100000, 24, NULL, 'https://finance.yahoo.com/quote/NWRB', '2026-11-06T21:00:00Z');`;

  it("labels no session the first trading day when day one may have been missed", async () => {
    env.db = db(MISSED_DAY_ONE);
    const ipo = (await loadIpo(env.db as D1Like, "northwind-robotics", "en"))!;
    expect(ipo.trading).toMatchObject({ first: { sessionDate: "2026-11-06" }, latest: { sessionDate: "2026-11-10" }, firstIsDayOne: false });
    for (const [locale, first, latest, banned] of [
      ["en", "Nov 6, 2026Close$30.10", "Nov 10, 2026Close$28.45", /First|Latest|day one|debut/],
      ["zh", "2026年11月6日收盘价30.10美元", "2026年11月10日收盘价28.45美元", /首个交易日|首日|最新/],
    ] as const) {
      const text = textOf(await page("northwind-robotics", locale));
      const after = text.slice(text.indexOf(locale === "en" ? "After listing" : "上市后表现"));
      expect(after).toContain(first);
      expect(after).toContain(latest);
      expect(after).not.toMatch(banned);
    }
  });

  it("calls the earliest session the first trading day only for a trading date on that same day", async () => {
    // A trading date, but not the earliest stored session's day: no first-day wording.
    const other = db(`${UNCHECKED("UPDATE ipo_watch SET first_trade_date = '2026-11-04' WHERE slug = 'northwind-robotics';")}`);
    expect((await loadIpo(other, "northwind-robotics", "en"))!.trading!.firstIsDayOne).toBe(false);
    // A confirmed filing date on the same day is a filing's word, not the trading record: still no label.
    const filing = db(`UPDATE ipo_watch SET date_source = 'filing', date_source_name = '424B4' WHERE slug = 'northwind-robotics';`);
    expect((await loadIpo(filing, "northwind-robotics", "en"))!.trading!.firstIsDayOne).toBe(false);
    // The seed: confirmed by trading on Nov 5, the earliest session.
    expect((await loadIpo(db(), "northwind-robotics", "en"))!.trading!.firstIsDayOne).toBe(true);
  });

  it("takes the clock as an injected ET date: the readers never read it themselves when given one", async () => {
    // todayEt of an instant is the New York calendar date; the readers take that date as an argument.
    expect(todayEt(new Date("2026-11-18T04:59:59Z"))).toBe("2026-11-17");
    expect(todayEt(new Date("2026-11-18T05:00:00Z"))).toBe("2026-11-18");
    expect(reportedDatePassed("2026-11-16", "2026-11-17")).toBe(false);
    expect(reportedDatePassed("2026-11-16", "2026-11-18")).toBe(true);
    // A Friday report shows through Saturday and is gone on Sunday (calendar days, not trading days).
    expect(reportedDatePassed("2026-11-20", "2026-11-21")).toBe(false);
    expect(reportedDatePassed("2026-11-20", "2026-11-22")).toBe(true);
    // The fake clock says Nov 11, but an injected Nov 18 wins.
    expect((await loadIpo(db(), "anthropic", "en", "2026-11-18"))!.date).toBeNull();
  });

  it("shows a trading-sourced date only when it is the earliest stored session's day, on the page, the calendar and the strip", async () => {
    // The seed passes: Yahoo's first session Nov 5 is the earliest stored, so the date line and a solid card show.
    env.db = db();
    expect(textOf(await page("northwind-robotics"))).toContain("Confirmed First trading day Thu, Nov 5, 2026: its first regular session, per Yahoo Finance.");
    expect(await calendar("2026-11-02")).toContain('class="cal-card cal-card-text cal-confirmed"');
    // The row says trading on Nov 5, but the earliest stored session is Nov 6 (or none is stored): date not set.
    for (const sql of [
      `DELETE FROM ipo_trading WHERE session_date = '2026-11-05';
       INSERT INTO ipo_trading (slug, session_date, symbol, open, close, volume, ipo_price, change_pct, source_url, fetched_at) VALUES
         ('northwind-robotics', '2026-11-06', 'NWRB', NULL, 30.1, 20100000, 24, NULL, 'https://finance.yahoo.com/quote/NWRB', 'x');`,
      `DELETE FROM ipo_trading WHERE slug = 'northwind-robotics';`,
      // An earlier session than the date (the date is later than the first stored session).
      `INSERT INTO ipo_trading (slug, session_date, symbol, open, close, volume, ipo_price, change_pct, source_url, fetched_at) VALUES
         ('northwind-robotics', '2026-11-04', 'NWRB', NULL, 25, 100, 24, NULL, 'https://finance.yahoo.com/quote/NWRB', 'x');`,
    ]) {
      env.db = db(sql);
      expect((await loadIpo(env.db as D1Like, "northwind-robotics", "en"))!.date, sql).toBeNull();
      for (const [locale, notSet] of [
        ["en", "First trading day: date not set."],
        ["zh", "首个交易日：日期未定。"],
      ] as const) {
        const text = textOf(await page("northwind-robotics", locale));
        expect(text, sql).toContain(notSet);
        expect(text, sql).not.toMatch(/per Yahoo Finance\.|当日为其首个常规交易日/);
      }
      const cal = await calendar("2026-11-02");
      expect(cal, sql).not.toContain('class="cal-ipo"');
      expect(cal, sql).not.toContain("cal-confirmed");
      const at = cal.indexOf('<section class="cal-ipo-watch"');
      expect(textOf(cal.slice(at, cal.indexOf("</section>", at))), sql).toContain("Northwind RoboticsNWRBListedDate not set");
    }
  });

  it("treats a reported date before yesterday (ET) as date not set, at the boundary", async () => {
    // Anthropic's reported date is Mon Nov 16.
    for (const [today, shown] of [
      ["2026-11-16", true],
      ["2026-11-17", true], // the date is yesterday: still shown
      ["2026-11-18", false], // the date is before yesterday: passed
      ["2026-12-01", false],
    ] as const) {
      expect((await loadIpo(db(), "anthropic", "en", today))!.date !== null, today).toBe(shown);
      expect((await listIposBetween(db(), "2026-11-16", "2026-11-20", "en", today)).length, today).toBe(shown ? 1 : 0);
      const watch = await listIpoWatch(db(), today, "en");
      expect(watch.find((i) => i.slug === "anthropic")!.date !== null, today).toBe(shown);
    }
    // Only a reported date passes: an expected or confirmed filing date stays.
    const expected = db(`UPDATE ipo_watch SET date_status = 'expected', date_source = 'filing', date_source_name = '424B4' WHERE slug = 'anthropic';`);
    expect((await loadIpo(expected, "anthropic", "en", "2026-12-01"))!.date).not.toBeNull();
  });

  it("shows a passed reported date as date not set on the page, the calendar and the strip", async () => {
    vi.setSystemTime(new Date("2026-11-18T15:00:00Z"));
    expect(await page("anthropic")).toContain('<p class="ipo-date">First trading day: date not set.</p>');
    expect(await page("anthropic", "zh")).toContain('<p class="ipo-date">首个交易日：日期未定。</p>');
    const cal = await calendar("2026-11-16");
    expect(cal).not.toContain('class="cal-ipo"');
    // The strip: dated IPOs first, then the not-set ones by name (Anthropic before Harbor Bio).
    const strip = textOf(cal.slice(cal.indexOf('<section class="cal-ipo-watch"'), cal.indexOf("</section>", cal.indexOf('<section class="cal-ipo-watch"'))));
    expect(strip).toBe("IPO watchNorthwind RoboticsNWRBListedNov 5, 2026AnthropicANTPrice range setDate not setHarbor BioHRBOFiledDate not set");
    // The day before: still shown, dashed.
    vi.setSystemTime(new Date("2026-11-17T15:00:00Z"));
    expect(await calendar("2026-11-16")).toContain('class="cal-card cal-card-text cal-unconfirmed"');
    // The edge is midnight in New York, not UTC (the core test's boundary): 23:59 ET on Tue Nov 17 still shows the
    // Nov 16 report on the page, the calendar and the strip; 00:00 ET Wed Nov 18 shows "date not set" on all three.
    const stripOf = (out: string) => {
      const at = out.indexOf('<section class="cal-ipo-watch"');
      return textOf(out.slice(at, out.indexOf("</section>", at)));
    };
    vi.setSystemTime(new Date("2026-11-18T04:59:59Z"));
    expect(textOf(await page("anthropic"))).toContain("Expected to start trading Mon, Nov 16, 2026, as reported by Example News");
    const before = await calendar("2026-11-16");
    expect(before).toContain('class="cal-ipo"');
    expect(stripOf(before)).toContain("AnthropicANTPrice range setNov 16, 2026Unconfirmed");
    vi.setSystemTime(new Date("2026-11-18T05:00:00Z"));
    expect(await page("anthropic")).toContain('<p class="ipo-date">First trading day: date not set.</p>');
    const after = await calendar("2026-11-16");
    expect(after).not.toContain('class="cal-ipo"');
    expect(stripOf(after)).toContain("AnthropicANTPrice range setDate not set");
  });

  it("renders every item from its own fields and quote with NULL statements, as the v1 seed has them, in both languages", async () => {
    for (const locale of ["en", "zh"] as const) {
      const ipo = (await loadIpo(db(), "anthropic", locale))!;
      expect([...ipo.valuations, ...ipo.moves, ...ipo.views].map((i) => i.statement)).toEqual([null, null, null, null]);
      const text = textOf(await page("anthropic", locale));
      expect(text).toContain(locale === "en" ? "$1.4 trillionReported by Example News on Nov 10, 2026“At the top" : "1.4万亿美元Example News 2026年11月10日报道“At the top");
    }
    // The seed's moves: the attribution introduces the quote, then the link; no empty lead line.
    for (const [locale, line] of [
      ["en", "Recent movesSample Times reported on Nov 4, 2026:“Anthropic on Tuesday released"],
      ["zh", "公司近期动态Sample Times 2026年11月4日报道：“Anthropic on Tuesday released"],
    ] as const) {
      const out = await page("anthropic", locale);
      expect(textOf(out)).toContain(line);
      expect(out).not.toContain('<p class="ipo-lead"></p>');
      // Heading, the attribution introducing the quote, the quote in quotation marks, the article link.
      const moves = out.slice(out.indexOf('<h2 id="moves">'), out.indexOf('<h2 id="views">'));
      expect(moves).toMatch(
        /^<h2 id="moves">[^<]+<\/h2><ul class="ipo-items"><li class="ipo-item"><p class="ipo-by ipo-by-lead">[^<]+<\/p><blockquote class="ipo-quote">“[^<]+”<\/blockquote><p class="ipo-link"><a class="more" href="https:\/\/sampletimes\.example\/2026\/11\/04\/anthropic-office-apps" rel="nofollow noopener noreferrer">[^<]+ ↗<\/a><\/p><\/li><\/ul>$/,
      );
    }
    const northwind = textOf(await page("northwind-robotics", "zh"));
    expect(northwind).toContain("公司近期动态Example News 2026年10月30日报道：“Northwind Robotics signed a deal");
    // A statement stored on a valuation or a view anyway is not printed.
    env.db = db(`UPDATE ipo_items SET statement_en = 'Stray sentence.', statement_zh = '多余的句子。' WHERE kind IN ('valuation', 'view');`);
    for (const locale of ["en", "zh"] as const) expect(await page("anthropic", locale)).not.toMatch(/Stray sentence|多余的句子/);
    // A move that does carry a statement (the columns remain) leads with it, the attribution under it.
    env.db = db(`UPDATE ipo_items SET statement_en = 'Released AI word-processing and presentation software for business customers.' WHERE kind = 'company_move';`);
    expect(textOf(await page("anthropic", "zh"))).toContain(
      "公司近期动态Released AI word-processing and presentation software for business customers.Sample Times 2026年11月4日报道“Anthropic",
    );
    expect(textOf(await page("anthropic"))).toContain("Recent movesReleased AI word-processing and presentation software for business customers.Reported by Sample Times on Nov 4, 2026“Anthropic");
  });

  it("prints a speaker given by name alone", async () => {
    env.db = db(`UPDATE ipo_items SET speaker = 'Jane Doe' WHERE kind = 'view';`);
    expect(textOf(await page("anthropic"))).toContain(`What others sayJane Doe, quoted by Sample Times on Nov 9, 2026:“${VIEW_QUOTE}”`);
    expect(textOf(await page("anthropic", "zh"))).toContain(`各方观点Sample Times 2026年11月9日引述 Jane Doe：“${VIEW_QUOTE}”`);
  });
});
