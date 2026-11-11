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
} from "../src/lib/ipo";
import type { D1Like } from "../src/lib/posts";
import { sitemapEntries } from "../src/lib/sitemap";
import { translator } from "../src/lib/translator";
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

/** A hostile row: markup in every text field, and links that must never become links. */
const EVIL = `
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
  ('evil-co', 'view', 'https://ok.example/v', 'Outlet', '2026-11-08', 'A view', NULL, NULL, '<script>alert("speaker")</script>', NULL, NULL, 'x');`;

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

  it("reads Chinese names and statements when set, English otherwise; quotes stay as written", async () => {
    const db = memoryD1(SEED);
    const zh = (await loadIpo(db, "anthropic", "zh"))!;
    expect(zh.name).toBe("Anthropic");
    expect(zh.moves[0].statement).toBe("面向企业客户推出了 AI 文字处理和演示文稿软件。");
    expect(zh.moves[0].quote).toMatch(/^Anthropic on Tuesday released/);
    expect((await loadIpo(db, "northwind-robotics", "zh"))!.name).toBe("北风机器人");
    const noZh = memoryD1(`${SEED}\nUPDATE ipo_items SET statement_zh = NULL WHERE slug = 'anthropic';`);
    expect((await loadIpo(noZh, "anthropic", "zh"))!.moves[0].statement).toBe(
      "Released AI word-processing and presentation software for business customers.",
    );
  });

  it("reads the first and the latest session, keeping the change only beside both inputs", async () => {
    const ipo = (await loadIpo(memoryD1(SEED), "northwind-robotics", "en"))!;
    expect(ipo.trading).toEqual({
      first: { sessionDate: "2026-11-05", open: null, close: 31.2, volume: 48210000, ipoPrice: 24, changePct: 30, sourceUrl: "https://finance.yahoo.com/quote/NWRB" },
      latest: { sessionDate: "2026-11-10", open: null, close: 28.45, volume: 9120000, ipoPrice: 24, changePct: 18.54, sourceUrl: "https://finance.yahoo.com/quote/NWRB" },
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
    expect(out.match(/cal-ipo/g)).toHaveLength(1);
    // The watch list sits between the head and the board.
    expect(out.indexOf('<section class="ipo-watch"')).toBeGreaterThan(out.indexOf("</header>"));
    expect(out.indexOf('<section class="ipo-watch"')).toBeLessThan(out.indexOf('<div class="cal-week"'));
    const list = out.slice(out.indexOf('<section class="ipo-watch"'), out.indexOf("</section>", out.indexOf('<section class="ipo-watch"')));
    expect(textOf(list)).toBe(
      "IPO watch" +
        "Northwind RoboticsNWRBListedNov 5, 2026" +
        "AnthropicANTPrice range setNov 16, 2026Unconfirmed" +
        "Harbor BioHRBOFiledDate not set",
    );
    expect(list).toContain('<a class="ipo-watch-name" href="/en/ipo/anthropic">Anthropic</a>');
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
    expect(out).toContain('<h2 class="cal-slot" id="ipo-watch">IPO 追踪</h2>');
    expect(out).toContain('<span class="ipo-watch-status">已定发行价区间</span>');
    expect(out).toContain('<span class="ipo-watch-date">日期未定</span>');
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
    expect(out).not.toContain("cal-ipo");
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
    expect(text).toContain("Filing text“It is currently estimated that the initial public offering price per share will be between $70.00 and $78.00”");
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
    expect(text).toContain("Recent movesReleased AI word-processing and presentation software for business customers.Reported by Sample Times on Nov 4, 2026");
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
    expect(zh).toContain(`各方观点Sample Times于2026年11月9日引述Jane Doe, an analyst at Example Securities：“${VIEW_QUOTE}”`);
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
    expect(text).toContain("未确认 预计2026年11月16日开始交易，据Example News2026年11月10日报道。 原文 ↗");
    expect(text).toContain("出自2026年11月10日提交的S-1/A · SEC 文件 ↗");
    expect(out).toContain('<dt>发行股数</dt><dd><span class="ipo-value">825,000,000股</span>');
    expect(out).toContain('<dt>发行价区间</dt><dd><span class="ipo-value">每股70.00至78.00美元</span>');
    expect(text).toContain("2026年11月10日 注册声明修订（S-1/A）文件 ↗");
    expect(text).toContain("1.4万亿美元Example News于2026年11月10日报道");
    expect(text).toContain("面向企业客户推出了 AI 文字处理和演示文稿软件。");
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
      ["zh", "已定价", "未确认 预计2026年11月5日开始交易：定价后的下一个交易日（2026年11月5日提交的424B4）。 SEC 文件 ↗", "每股24.00美元"],
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
    expect(after).toContain("First day · Thu, Nov 5Close$31.20Volume48,210,000 sharesChange from IPO price+30.00% (close $31.20 vs. IPO price $24.00)");
    expect(after).toContain("Latest · Tue, Nov 10Close$28.45Volume9,120,000 sharesChange from IPO price+18.54% (close $28.45 vs. IPO price $24.00)");
    expect(out).toContain('<a href="https://finance.yahoo.com/quote/NWRB" rel="noopener noreferrer">Source: Yahoo Finance ↗</a>');
    expect(out).toContain('<span class="num up">+30.00%</span>');
    // Open is never stored in v1, so it is not shown.
    expect(after).not.toContain("Open");
    const zh = textOf(await page("northwind-robotics", "zh"));
    expect(zh).toContain("北风机器人 IPO");
    expect(zh).toContain("首日 · 11月5日 周四收盘价$31.20成交量48,210,000股较发行价涨跌+30.00%（收盘价31.20美元，发行价24.00美元）");
    expect(zh).toContain("来源：Yahoo Finance ↗");
  });

  it("listed with one session: the first day alone; without the IPO price, no change", async () => {
    env.db = memoryD1(`${SEED}\nDELETE FROM ipo_trading WHERE session_date = '2026-11-10';\nUPDATE ipo_trading SET ipo_price = NULL, open = 25.5;`);
    const text = textOf(await page("northwind-robotics"));
    expect(text).toContain("First day · Thu, Nov 5Open$25.50Close$31.20Volume48,210,000 shares");
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
