/** Read-only queries over the earnings tables (migrations/0003_earnings.sql). */

import type { D1Like, Lang, PostListItem } from "./posts";

export type Slot = "bmo" | "amc" | "unknown";
export type CalendarStatus = "unconfirmed" | "confirmed" | "reported";

export interface CalendarEntry {
  ticker: string;
  date: string;
  slot: Slot;
  status: CalendarStatus;
  /** The earnings post for a reported entry, when its English text is published. */
  postSlug: string | null;
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
/** R2 keys the generator writes: charts/earnings/{TICKER}/{accession}/{name}.svg */
const CHART_KEY = /^earnings\/[A-Z0-9-]{1,10}\/\d{10}-\d{2}-\d{6}\/(revenue|eps)\.svg$/;

/** A real calendar date written as YYYY-MM-DD. */
export function isDate(s: string | undefined | null): s is string {
  if (!s || !DATE.test(s)) return false;
  const d = new Date(`${s}T00:00:00Z`);
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s;
}

/** A URL ticker in display form (`brk.b` -> `BRK.B`), or null if it cannot be one. */
export function normalizeTicker(raw: string): string | null {
  const t = decodeURIComponent(raw).toUpperCase();
  return TICKER.test(t) ? t : null;
}

export function addDays(date: string, n: number): string {
  const d = new Date(`${date}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + n);
  return d.toISOString().slice(0, 10);
}

/** The Monday of the week containing `date` (YYYY-MM-DD, calendar arithmetic only). */
export function weekStart(date: string): string {
  const day = new Date(`${date}T00:00:00Z`).getUTCDay(); // 0 = Sunday
  return addDays(date, day === 0 ? -6 : 1 - day);
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
  post_slug: string | null;
}

// Earnings posts whose English text is published, by ticker and ET date.
const POSTS_BY_TICKER_DATE = `SELECT f.ticker, p.trading_date, MIN(p.slug) AS slug
  FROM earnings_filings f
  JOIN posts p ON p.run_key = f.run_key
  JOIN post_translations t ON t.post_id = p.id AND t.lang = 'en' AND t.status = 'published'
  GROUP BY f.ticker, p.trading_date`;

const toEntry = (r: CalRow): CalendarEntry => ({
  ticker: r.ticker,
  date: r.report_date,
  slot: r.time_slot === "bmo" || r.time_slot === "amc" ? r.time_slot : "unknown",
  status: r.status as CalendarStatus,
  postSlug: r.post_slug,
});

/** Monday-Friday of the week starting `monday`; quarantined dates are not shown. */
export async function listCalendarWeek(db: D1Like, monday: string): Promise<CalendarDay[]> {
  const friday = addDays(monday, 4);
  const { results } = await db
    .prepare(
      `SELECT c.ticker, c.report_date, c.time_slot, c.status, x.slug AS post_slug
       FROM earnings_calendar c
       LEFT JOIN (${POSTS_BY_TICKER_DATE}) x ON x.ticker = c.ticker AND x.trading_date = c.report_date
       WHERE c.report_date BETWEEN ? AND ? AND c.status IN ('unconfirmed','confirmed','reported')
       ORDER BY c.report_date, CASE c.time_slot WHEN 'bmo' THEN 0 WHEN 'amc' THEN 1 ELSE 2 END, c.ticker`,
    )
    .bind(monday, friday)
    .all<CalRow>();
  return [0, 1, 2, 3, 4].map((i) => {
    const date = addDays(monday, i);
    return { date, entries: results.filter((r) => r.report_date === date).map(toEntry) };
  });
}

/** A ticker's calendar rows (newest first), quarantined ones hidden. */
export async function listTickerCalendar(db: D1Like, ticker: string): Promise<CalendarEntry[]> {
  const { results } = await db
    .prepare(
      `SELECT c.ticker, c.report_date, c.time_slot, c.status, x.slug AS post_slug
       FROM earnings_calendar c
       LEFT JOIN (${POSTS_BY_TICKER_DATE}) x ON x.ticker = c.ticker AND x.trading_date = c.report_date
       WHERE c.ticker = ? AND c.status IN ('unconfirmed','confirmed','reported')
       ORDER BY c.report_date DESC LIMIT 12`,
    )
    .bind(ticker)
    .all<CalRow>();
  return results.map(toEntry);
}

interface ListRow {
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
});

const EARNINGS_LIST = `SELECT p.slug, p.content_type, p.trading_date, p.published_at, t.title, t.summary
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
