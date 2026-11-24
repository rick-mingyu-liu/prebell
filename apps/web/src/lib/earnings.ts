/** Read-only queries over the earnings tables (migrations/0003_earnings.sql). */

import { COMPANIES } from "./companies.generated";
import { COVER_COLUMNS, JOIN_FILING, toCover, type CoverRow, type D1Like, type Lang, type PostListItem } from "./posts";

export type Slot = "bmo" | "amc" | "unknown";
export type CalendarStatus = "unconfirmed" | "confirmed" | "reported";

export interface CalendarEntry {
  ticker: string;
  /** The watchlist's display name (English in both languages), or null for a ticker no longer on it. */
  name: string | null;
  /** The company gets earnings checks (the write-up tier); the rest are calendar only. */
  writeUps: boolean;
  date: string;
  slot: Slot;
  status: CalendarStatus;
  /** The earnings post for a reported entry, when its English text is published. */
  postSlug: string | null;
  /** The 8-K that reported it (EDGAR's filing index), for a reported entry. */
  filingUrl: string | null;
}

export interface CalendarDay {
  date: string;
  entries: CalendarEntry[];
}

export interface PostChart {
  name: string;
  src: string;
  alt: string;
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const TICKER = /^[A-Z][A-Z0-9.-]{0,9}$/;
/** The only proof links shown: EDGAR filing indexes the generator writes for reported rows. */
const FILING_URL = /^https:\/\/www\.sec\.gov\/Archives\/edgar\/data\/\d+\/\d{18}\/\d{10}-\d{2}-\d{6}-index\.htm$/;
/** R2 keys the generator writes: charts/earnings/{TICKER}/{accession}/{name}.svg */
const CHART_KEY = /^earnings\/[A-Z0-9-]{1,10}\/\d{10}-\d{2}-\d{6}\/(revenue|eps)\.svg$/;

/**
 * A real calendar date written as YYYY-MM-DD, in the years 1000-9998, so that a caller's few days either side
 * (the week links, a range) stay four-digit dates that sort as strings.
 */
export function isDate(s: string | undefined | null): s is string {
  if (!s || !DATE.test(s) || s < "1000" || s >= "9999") return false;
  const d = new Date(`${s}T00:00:00Z`);
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s;
}

/** A URL ticker in display form (`brk.b` -> `BRK.B`), or null if it cannot be one. */
export function normalizeTicker(raw: string): string | null {
  // Next has already decoded the route param.
  const t = raw.toUpperCase();
  return TICKER.test(t) ? t : null;
}

/**
 * `date` plus `n` days. Throws a RangeError rather than return something that is not a four-digit-year date
 * (`toISOString` writes year 10000 as "+010000-01-01", which sorts before "9999-12-31").
 */
export function addDays(date: string, n: number): string {
  const d = new Date(`${date}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + n);
  const out = Number.isNaN(d.getTime()) ? "" : d.toISOString().slice(0, 10);
  if (!DATE.test(out)) throw new RangeError(`addDays: ${date} + ${n} is out of range`);
  return out;
}

/** The Monday of the week containing `date` (YYYY-MM-DD, calendar arithmetic only). */
export function weekStart(date: string): string {
  const day = new Date(`${date}T00:00:00Z`).getUTCDay(); // 0 = Sunday
  return addDays(date, day === 0 ? -6 : 1 - day);
}

/** True on Saturday and Sunday (`date` is an ET calendar date). */
export function isWeekend(date: string): boolean {
  const day = new Date(`${date}T00:00:00Z`).getUTCDay();
  return day === 0 || day === 6;
}

/**
 * The Monday of the week the undated calendar shows on `today` (an ET date): this week from Monday to Friday,
 * and the coming week on Saturday and Sunday, once the week's reports are all in (Rick, 2026-11-24: "the
 * calendar stays the live view, and switches to the coming week on Saturday").
 */
export function readingWeek(today: string): string {
  const monday = weekStart(today);
  return isWeekend(today) ? addDays(monday, 7) : monday;
}

/** Today's date in US Eastern time. */
export function todayEt(now: Date = new Date()): string {
  return new Intl.DateTimeFormat("en-CA", { timeZone: "America/New_York" }).format(now);
}

interface CalRow {
  ticker: string;
  report_date: string;
  time_slot: string;
  status: string;
  proof_url: string | null;
  post_slug: string | null;
}

const CAL_COLUMNS = "c.ticker, c.report_date, c.time_slot, c.status, c.proof_url, x.slug AS post_slug";

// Earnings posts the post page shows in `lang` (a pending ZH page shows the
// "coming soon" notice), by ticker and ET date, for a range of ET dates (binds
// `lang`, then the first and last date) or one ticker (binds `lang`, then the
// ticker). CROSS JOIN pins the join order so SQLite starts from the date or
// ticker index instead of reading every published translation.
const POSTS_BETWEEN = `SELECT f.ticker, p.trading_date, MIN(p.slug) AS slug
  FROM posts p
  CROSS JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' AND en.status = 'published'
  CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
    AND (t.status = 'published' OR (t.lang = 'zh' AND t.status = 'pending'))
  CROSS JOIN earnings_filings f ON f.run_key = p.run_key
  WHERE p.trading_date BETWEEN ? AND ?
  GROUP BY f.ticker, p.trading_date`;
const POSTS_FOR_TICKER = `SELECT f.ticker, p.trading_date, MIN(p.slug) AS slug
  FROM earnings_filings f
  CROSS JOIN posts p ON p.run_key = f.run_key
  CROSS JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' AND en.status = 'published'
  CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
    AND (t.status = 'published' OR (t.lang = 'zh' AND t.status = 'pending'))
  WHERE f.ticker = ?
  GROUP BY f.ticker, p.trading_date`;

// The same rows for one ET date (binds `lang`, then the date). CROSS JOIN pins the join order so
// SQLite starts from idx_posts_trading_date instead of scanning every published translation.
const POSTS_ON_DATE = `SELECT f.ticker, p.trading_date, MIN(p.slug) AS slug
  FROM posts p
  CROSS JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' AND en.status = 'published'
  CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
    AND (t.status = 'published' OR (t.lang = 'zh' AND t.status = 'pending'))
  CROSS JOIN earnings_filings f ON f.run_key = p.run_key
  WHERE p.trading_date = ?
  GROUP BY f.ticker, p.trading_date`;

const toEntry = (r: CalRow): CalendarEntry => {
  const company = COMPANIES[r.ticker];
  return {
    ticker: r.ticker,
    name: company?.[0] ?? null,
    writeUps: company?.[1] ?? false,
    date: r.report_date,
    slot: r.time_slot === "bmo" || r.time_slot === "amc" ? r.time_slot : "unknown",
    status: r.status as CalendarStatus,
    postSlug: r.post_slug,
    filingUrl: r.status === "reported" && r.proof_url && FILING_URL.test(r.proof_url) ? r.proof_url : null,
  };
};

/** Write-up companies first, the rest after; the query's order (slot, ticker) otherwise. */
const writeUpsFirst = (entries: CalendarEntry[]): CalendarEntry[] =>
  [...entries].sort((a, b) => Number(b.writeUps) - Number(a.writeUps));

/** Monday-Friday of the week starting `monday`; quarantined dates are not shown. */
export async function listCalendarWeek(db: D1Like, monday: string, lang: Lang): Promise<CalendarDay[]> {
  const friday = addDays(monday, 4);
  const { results } = await db
    .prepare(
      `SELECT ${CAL_COLUMNS}
       FROM earnings_calendar c
       LEFT JOIN (${POSTS_BETWEEN}) x ON x.ticker = c.ticker AND x.trading_date = c.report_date
       WHERE c.report_date BETWEEN ? AND ? AND c.status IN ('unconfirmed','confirmed','reported')
       ORDER BY c.report_date, CASE c.time_slot WHEN 'bmo' THEN 0 WHEN 'amc' THEN 1 ELSE 2 END, c.ticker`,
    )
    .bind(lang, monday, friday, monday, friday)
    .all<CalRow>();
  return [0, 1, 2, 3, 4].map((i) => {
    const date = addDays(monday, i);
    return { date, entries: writeUpsFirst(results.filter((r) => r.report_date === date).map(toEntry)) };
  });
}

/**
 * Calendar entries from `from` to `to` (ET dates), optionally only `tickers` (at most 50: D1 binds at most 100
 * values), in one D1 read; by date, write-up companies first within a day, then slot and ticker. Quarantined
 * dates are not shown.
 */
export async function listCalendarRange(
  db: D1Like,
  from: string,
  to: string,
  lang: Lang,
  tickers: string[] = [],
): Promise<CalendarEntry[]> {
  if (tickers.length > 50) throw new Error("listCalendarRange: at most 50 tickers");
  const only = tickers.length ? `AND c.ticker IN (${tickers.map(() => "?").join(",")})` : "";
  const { results } = await db
    .prepare(
      `SELECT ${CAL_COLUMNS}
       FROM earnings_calendar c
       LEFT JOIN (${POSTS_BETWEEN}) x ON x.ticker = c.ticker AND x.trading_date = c.report_date
       WHERE c.report_date BETWEEN ? AND ? AND c.status IN ('unconfirmed','confirmed','reported') ${only}
       ORDER BY c.report_date, CASE c.time_slot WHEN 'bmo' THEN 0 WHEN 'amc' THEN 1 ELSE 2 END, c.ticker`,
    )
    .bind(lang, from, to, from, to, ...tickers)
    .all<CalRow>();
  const entries = results.map(toEntry);
  return [...new Set(entries.map((e) => e.date))].flatMap((d) => writeUpsFirst(entries.filter((e) => e.date === d)));
}

/** The earnings strip's day: its entries grouped by report slot. */
export interface StripDay {
  date: string;
  bmo: CalendarEntry[];
  amc: CalendarEntry[];
  unknown: CalendarEntry[];
}

/** How far ahead the homepage strip looks for a date with entries. */
const STRIP_LOOKAHEAD_DAYS = 7;

/**
 * The strip's report day. With `exact`, only `fromDate`; otherwise the first date from `fromDate`
 * through 7 days ahead that has entries. Null when there are none. Quarantined rows are not shown.
 */
export async function stripDay(
  db: D1Like,
  fromDate: string,
  opts: { exact: boolean; lang: Lang },
): Promise<StripDay | null> {
  const last = opts.exact ? fromDate : addDays(fromDate, STRIP_LOOKAHEAD_DAYS);
  // Two cheap indexed reads: find the day, then read only that day (and only its posts).
  const first = await db
    .prepare(
      `SELECT MIN(report_date) AS date FROM earnings_calendar
       WHERE report_date BETWEEN ? AND ? AND status IN ('unconfirmed','confirmed','reported')`,
    )
    .bind(fromDate, last)
    .first<{ date: string | null }>();
  const date = first?.date;
  if (!date) return null;
  const { results } = await db
    .prepare(
      `SELECT ${CAL_COLUMNS}
       FROM earnings_calendar c
       LEFT JOIN (${POSTS_ON_DATE}) x ON x.ticker = c.ticker AND x.trading_date = c.report_date
       WHERE c.report_date = ? AND c.status IN ('unconfirmed','confirmed','reported')
       ORDER BY CASE c.time_slot WHEN 'bmo' THEN 0 WHEN 'amc' THEN 1 ELSE 2 END, c.ticker`,
    )
    .bind(opts.lang, date, date)
    .all<CalRow>();
  const entries = writeUpsFirst(results.map(toEntry));
  return {
    date,
    bmo: entries.filter((e) => e.slot === "bmo"),
    amc: entries.filter((e) => e.slot === "amc"),
    unknown: entries.filter((e) => e.slot === "unknown"),
  };
}

/** A ticker's calendar rows (newest first), quarantined ones hidden. */
export async function listTickerCalendar(db: D1Like, ticker: string, lang: Lang): Promise<CalendarEntry[]> {
  const { results } = await db
    .prepare(
      `SELECT ${CAL_COLUMNS}
       FROM earnings_calendar c
       LEFT JOIN (${POSTS_FOR_TICKER}) x ON x.ticker = c.ticker AND x.trading_date = c.report_date
       WHERE c.ticker = ? AND c.status IN ('unconfirmed','confirmed','reported')
       ORDER BY c.report_date DESC LIMIT 12`,
    )
    .bind(lang, ticker, ticker)
    .all<CalRow>();
  return results.map(toEntry);
}

interface ListRow extends CoverRow {
  slug: string;
  content_type: string;
  trading_date: string;
  published_at: string | null;
  title: string | null;
  summary: string | null;
}

const toItem = (r: ListRow): PostListItem => ({
  slug: r.slug,
  contentType: r.content_type,
  tradingDate: r.trading_date,
  publishedAt: r.published_at,
  title: r.title ?? "",
  summary: r.summary ?? "",
  ...toCover(r),
});

const EARNINGS_LIST = `SELECT p.slug, p.content_type, p.trading_date, p.published_at, t.title, t.summary, ${COVER_COLUMNS}
  FROM posts p
  JOIN earnings_filings f ON f.run_key = p.run_key
  JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
  WHERE t.status = 'published'`;

/** Published earnings posts for one ticker in `lang`, newest first. */
export async function listTickerPosts(db: D1Like, ticker: string, lang: Lang): Promise<PostListItem[]> {
  const { results } = await db
    .prepare(`${EARNINGS_LIST} AND f.ticker = ? ORDER BY p.trading_date DESC, p.published_at DESC LIMIT 40`)
    .bind(lang, ticker)
    .all<ListRow>();
  return results.map(toItem);
}

/** The newest published earnings posts in `lang`. */
export async function listLatestEarnings(db: D1Like, lang: Lang, n: number): Promise<PostListItem[]> {
  const { results } = await db
    .prepare(`${EARNINGS_LIST} ORDER BY p.trading_date DESC, p.published_at DESC LIMIT ?`)
    .bind(lang, n)
    .all<ListRow>();
  return results.map(toItem);
}

/** The newest published non-earnings posts (briefs, recaps) in `lang`. */
export async function listRecentBriefs(db: D1Like, lang: Lang, n: number): Promise<PostListItem[]> {
  const { results } = await db
    .prepare(
      `SELECT p.slug, p.content_type, p.trading_date, p.published_at, t.title, t.summary, ${COVER_COLUMNS}
       FROM posts p JOIN post_translations t ON t.post_id = p.id AND t.lang = ?
       ${JOIN_FILING}
       WHERE t.status = 'published' AND p.content_type <> 'earnings'
       ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC LIMIT ?`,
    )
    .bind(lang, n)
    .all<ListRow>();
  return results.map(toItem);
}

/** Weeks this far from today are not indexed. */
export const INDEXED_WEEKS = 8;

/**
 * The calendar's first week (the site's first): earlier weeks answer 404 and have no "previous" link.
 * Unbounded week links led crawlers back to 2004 and forward to 2050, each week a full render.
 */
export const FIRST_WEEK = "2026-10-26";

/** How many weeks past this one the calendar goes. */
export const WEEKS_AHEAD = 13;

/** The Mondays of the first and last weeks the calendar shows. */
export function calendarWeeks(today: string): { first: string; last: string } {
  return { first: FIRST_WEEK, last: addDays(weekStart(today), 7 * WEEKS_AHEAD) };
}

/** True when the ticker has an earnings filing or a calendar row. */
export async function tickerKnown(db: D1Like, ticker: string): Promise<boolean> {
  const row = await db
    .prepare(
      `SELECT 1 AS hit WHERE EXISTS (SELECT 1 FROM earnings_filings WHERE ticker = ?)
         OR EXISTS (SELECT 1 FROM earnings_calendar WHERE ticker = ? AND status <> 'quarantined')`,
    )
    .bind(ticker, ticker)
    .first<{ hit: number }>();
  return row !== null;
}

/** At most this many tickers are looked up at once (a brief has about 20 movers; D1 binds at most 100). */
const PAGES_LOOKUP_MAX = 40;

/**
 * Which of these tickers have a page (`/{locale}/t/{ticker}` renders rather than 404): `tickerKnown`'s rule for
 * a list, in one query. Tickers are compared as written (display form, e.g. `BRK.B`).
 */
export async function tickersWithPages(db: D1Like, tickers: readonly string[]): Promise<Set<string>> {
  const list = [...new Set(tickers.filter((t) => TICKER.test(t)))].slice(0, PAGES_LOOKUP_MAX);
  if (list.length === 0) return new Set();
  const qs = list.map(() => "?").join(",");
  const { results } = await db
    .prepare(
      `SELECT ticker FROM earnings_filings WHERE ticker IN (${qs})
       UNION SELECT ticker FROM earnings_calendar WHERE status <> 'quarantined' AND ticker IN (${qs})`,
    )
    .bind(...list, ...list)
    .all<{ ticker: string }>();
  return new Set(results.map((r) => r.ticker));
}

/** True when the ticker has an earnings check whose English text is published (else its page is noindex). */
export async function tickerHasCheck(db: D1Like, ticker: string): Promise<boolean> {
  const row = await db
    .prepare(
      `SELECT 1 AS hit WHERE EXISTS (
         SELECT 1 FROM earnings_filings f
         JOIN posts p ON p.run_key = f.run_key
         JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en' AND en.status = 'published'
         WHERE f.ticker = ?)`,
    )
    .bind(ticker)
    .first<{ hit: number }>();
  return row !== null;
}

/** The watchlist's display name for a ticker, or null. */
export function companyName(ticker: string): string | null {
  return COMPANIES[ticker]?.[0] ?? null;
}

/** A post's charts, with the alt text in `lang`. */
export async function getPostCharts(db: D1Like, slug: string, lang: Lang): Promise<PostChart[]> {
  const { results } = await db
    .prepare(
      `SELECT c.name, c.r2_key, c.alt_en, c.alt_zh FROM post_charts c
       JOIN posts p ON p.id = c.post_id WHERE p.slug = ? ORDER BY c.sort, c.name`,
    )
    .bind(slug)
    .all<{ name: string; r2_key: string; alt_en: string; alt_zh: string }>();
  return results
    .filter((c) => c.r2_key.startsWith("charts/") && CHART_KEY.test(c.r2_key.slice("charts/".length)))
    .map((c) => ({
      name: c.name,
      src: `/${c.r2_key}`,
      alt: lang === "zh" ? c.alt_zh : c.alt_en,
    }));
}

/** The slice of the R2 binding the chart route uses. */
export interface R2Like {
  get(key: string): Promise<{ body: ReadableStream | string } | null>;
}

/**
 * The response for `GET /charts/<segments>`: only generator-written chart keys,
 * served as SVG that cannot run script or load anything.
 */
export async function chartResponse(bucket: R2Like, segments: string[]): Promise<Response> {
  const key = segments.join("/");
  if (!CHART_KEY.test(key)) return new Response("not found", { status: 404 });
  const obj = await bucket.get(`charts/${key}`);
  if (!obj) return new Response("not found", { status: 404 });
  return new Response(obj.body, {
    headers: {
      "Content-Type": "image/svg+xml; charset=utf-8",
      "Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'",
      "X-Content-Type-Options": "nosniff",
      // A chart key names one filing and never changes.
      "Cache-Control": "public, max-age=86400, s-maxage=86400",
    },
  });
}
