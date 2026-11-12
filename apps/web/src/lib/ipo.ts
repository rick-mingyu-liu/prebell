/**
 * Read-only queries over the IPO watch tables (migrations/0009_ipo_watch.sql; contract
 * docs/superpowers/specs/2026-11-11-ipo-watch-contract.md). The generator is the only writer.
 *
 * Every text column here came from outside (filings, articles, Yahoo) and is rendered as plain text. Parsing is
 * lenient: a missing or malformed optional field reads as absent and hides that part of the page; only a row
 * without a usable slug or name is dropped. A link is kept only when it is an `https://` URL (filing links only
 * on www.sec.gov, the trading source only on finance.yahoo.com).
 */

import { addDays, isDate, todayEt } from "./earnings";
import { round2 } from "./format";
import type { D1Like, Lang } from "./posts";

export const IPO_STATUSES = ["filed", "range_set", "priced", "listed", "withdrawn"] as const;
export type IpoStatus = (typeof IPO_STATUSES)[number];
export type DateStatus = "reported" | "expected" | "confirmed";
export type DateSource = "news" | "filing" | "trading";

/** The first trading day shown, with who says so. Null on the IPO when no date can be sourced ("date not set"). */
export interface IpoDate {
  date: string;
  status: DateStatus;
  source: DateSource;
  /** An outlet or a filing label (`424B4`); for a trading source, `Yahoo Finance` when unreadable. */
  sourceName: string;
  /** Checked by source: any https article for news, www.sec.gov for a filing, finance.yahoo.com for trading. */
  sourceUrl: string | null;
  /** The article's or filing's own sentence; null for a trading source. */
  quote: string | null;
  /** The source's day: publication, filing or session. */
  asOf: string | null;
}

/** One tracked IPO as the calendar and the watch list show it. */
export interface IpoSummary {
  slug: string;
  /** `name_zh` on Chinese pages when set, else `name`. */
  name: string;
  symbol: string | null;
  status: IpoStatus;
  date: IpoDate | null;
}

export const FORM_LABELS: Record<string, { en: string; zh: string }> = {
  DRS: { en: "Draft registration statement (confidential)", zh: "保密提交的注册草案" },
  "DRS/A": { en: "Amended draft registration statement", zh: "注册草案修订" },
  "S-1": { en: "Registration statement (S-1)", zh: "注册声明（S-1）" },
  "S-1/A": { en: "Amended registration statement (S-1/A)", zh: "注册声明修订（S-1/A）" },
  "F-1": { en: "Registration statement (F-1)", zh: "注册声明（F-1）" },
  "F-1/A": { en: "Amended registration statement (F-1/A)", zh: "注册声明修订（F-1/A）" },
  "424B1": { en: "Final prospectus", zh: "最终招股说明书" },
  "424B4": { en: "Final prospectus", zh: "最终招股说明书" },
  "8-A12B": { en: "Exchange registration (8-A)", zh: "交易所注册（8-A）" },
  EFFECT: { en: "Declared effective by the SEC", zh: "SEC 宣布注册生效" },
  CERT: { en: "Exchange certified the listing", zh: "交易所批准上市" },
  RW: { en: "Registration withdrawn", zh: "撤回注册" },
};

/** The contract's glossary name for an EDGAR form code, or null for a code it does not list. */
export function formLabel(form: string, lang: Lang): string | null {
  return Object.hasOwn(FORM_LABELS, form) ? FORM_LABELS[form][lang] : null;
}

export interface IpoFiling {
  accession: string;
  form: string;
  filingDate: string;
  /** The filing's index page on www.sec.gov, or null. */
  indexUrl: string | null;
}

/** Cover facts the page shows. `cover_date` and unknown kinds are not read. */
export const FACT_KINDS = [
  "exchange",
  "symbol",
  "shares_offered",
  "shares_company",
  "shares_selling",
  "option_shares",
  "price_low",
  "price_high",
  "price_final",
  "prospectus_date",
  "trading_date",
] as const;
export type FactKind = (typeof FACT_KINDS)[number];
const TEXT_KINDS: FactKind[] = ["exchange", "symbol", "prospectus_date", "trading_date"];
const DATE_KINDS: FactKind[] = ["prospectus_date", "trading_date"];

export interface IpoFact {
  /** Numeric kinds: a finite, positive number. */
  num: number | null;
  /** Text and date kinds. */
  text: string | null;
  /** The cover's own words. */
  quote: string;
}

export interface IpoFacts {
  /** The filing the facts were read from. */
  filing: IpoFiling;
  facts: Partial<Record<FactKind, IpoFact>>;
}

export type ItemKind = "valuation" | "expected_date" | "company_move" | "view";

export interface IpoItem {
  kind: ItemKind;
  /** The article, https only; null hides the link (the item still shows, attributed). */
  url: string | null;
  outlet: string;
  publishedOn: string;
  /** Word for word from the article, in its own language. */
  quote: string;
  /** A company move's statement in the page's language, else English; null for every other kind or when none. */
  statement: string | null;
  /** A view's speaker as the article names them (a view without one is not shown). */
  speaker: string | null;
  /** A valuation in dollars. */
  valueUsd: number | null;
}

export interface IpoSession {
  sessionDate: string;
  open: number | null;
  close: number;
  volume: number | null;
  ipoPrice: number | null;
  /** (close − IPO price) / IPO price × 100, two decimals, computed here from the two numbers shown beside it. */
  changePct: number | null;
  /** The Yahoo Finance quote page, or null. */
  sourceUrl: string | null;
}

export interface IpoPage extends IpoSummary {
  exchange: string | null;
  facts: IpoFacts | null;
  /** Newest first. */
  filings: IpoFiling[];
  /** Newest first. */
  valuations: IpoItem[];
  /** Newest first. */
  moves: IpoItem[];
  /** The newest three, newest first. */
  views: IpoItem[];
  /**
   * The earliest session and the latest by date, each null when its row is unusable (never replaced by the other);
   * `latest` is also null when there is one session. `firstIsDayOne` is true only when the date line is a trading
   * date (`date_source = 'trading'`) equal to the earliest session's date: only then is that session the first
   * trading day (the recap may have missed day one). Null when neither session can be shown.
   */
  trading: { first: IpoSession | null; latest: IpoSession | null; firstIsDayOne: boolean } | null;
  updatedAt: string | null;
  /** PreBell's write-ups about this IPO published in the page's language, newest first. */
  writeups: IpoWriteup[];
}

/** The URL segment the generator writes. */
const SLUG = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
export const isIpoSlug = (s: string): boolean => SLUG.test(s);

const DATE_STATUSES: DateStatus[] = ["reported", "expected", "confirmed"];
const DATE_SOURCES: DateSource[] = ["news", "filing", "trading"];
const ITEM_KINDS: ItemKind[] = ["valuation", "expected_date", "company_move", "view"];
/** At most this many views show (contract: the three newest). */
export const VIEWS_SHOWN = 3;

/** Trimmed non-empty text, else null. */
function text(v: unknown): string | null {
  if (typeof v !== "string") return null;
  const s = v.trim();
  return s ? s : null;
}

/** A finite number above zero, else null. */
function positive(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) && v > 0 ? v : null;
}

/** An `https://` URL with a host, else null. `javascript:`, `http:` and the rest never become links. */
export function httpsUrl(v: unknown): string | null {
  const s = text(v);
  if (!s || !/^https:\/\//i.test(s) || /[\s"<>\\]/.test(s)) return null;
  try {
    const u = new URL(s);
    return u.protocol === "https:" && u.hostname ? s : null;
  } catch {
    return null;
  }
}

/** An https URL on exactly `host` (no user info, no port), as written, else null. */
function onHost(v: unknown, host: string): string | null {
  const u = httpsUrl(v);
  if (!u || !u.startsWith(`https://${host}/`)) return null;
  const parsed = new URL(u);
  return parsed.hostname === host && !parsed.username && !parsed.password && !parsed.port ? u : null;
}

/** Filing links: EDGAR pages on www.sec.gov only. */
export const secUrl = (v: unknown): string | null => onHost(v, "www.sec.gov");

/** The trading source: Yahoo Finance quote pages only. */
export const yahooUrl = (v: unknown): string | null => onHost(v, "finance.yahoo.com");

const dateOf = (v: unknown): string | null => (typeof v === "string" && isDate(v) ? v : null);

interface WatchRow {
  slug: unknown;
  name: unknown;
  name_zh: unknown;
  status: unknown;
  exchange?: unknown;
  symbol: unknown;
  first_trade_date: unknown;
  date_status: unknown;
  date_source: unknown;
  date_source_name: unknown;
  date_source_url: unknown;
  date_quote: unknown;
  date_as_of: unknown;
  /** The earliest stored trading session's date for this IPO, or null when none is stored. */
  first_session: unknown;
  updated_at?: unknown;
}

/** The contract's status and source pairs: a news date is only ever reported, never expected or confirmed. */
const DATE_PAIRS: Record<DateStatus, DateSource[]> = {
  reported: ["news"],
  expected: ["filing"],
  confirmed: ["filing", "trading"],
};

/**
 * True when a reported (news) date has passed: it is before yesterday, ET (contract: the generator drops such a date,
 * but only while IPO_WATCH_ENABLED is on, so the site applies the same rule). `today` is the ET date.
 */
export function reportedDatePassed(date: string, today: string): boolean {
  return date < addDays(today, -1);
}

/**
 * The date line, or null ("date not set") for a withdrawn IPO, a malformed date, a status and source the contract
 * does not pair, a news or filing date with no named source (an unattributed date is not shown), a reported
 * date that has passed (before yesterday, ET), or a trading date that is not the earliest stored session's day (the
 * same first-day check as the session headings: Yahoo's record shows that day as the first only when it is the
 * earliest one stored). A link that fails its source's check is dropped; the date stays.
 */
function dateLine(r: WatchRow, status: IpoStatus, today: string): IpoDate | null {
  const date = dateOf(r.first_trade_date);
  const dateStatus = r.date_status as DateStatus;
  const source = r.date_source as DateSource;
  if (status === "withdrawn" || !date || !DATE_STATUSES.includes(dateStatus) || !DATE_SOURCES.includes(source)) return null;
  if (!DATE_PAIRS[dateStatus].includes(source)) return null;
  if (dateStatus === "reported" && reportedDatePassed(date, today)) return null;
  if (source === "trading" && r.first_session !== date) return null;
  const sourceName = source === "trading" ? "Yahoo Finance" : text(r.date_source_name);
  if (!sourceName) return null;
  const url = r.date_source_url;
  return {
    date,
    status: dateStatus,
    source,
    sourceName,
    sourceUrl: source === "news" ? httpsUrl(url) : source === "filing" ? secUrl(url) : yahooUrl(url),
    quote: source === "trading" ? null : text(r.date_quote),
    asOf: dateOf(r.date_as_of),
  };
}

function summary(r: WatchRow, lang: Lang, today: string): IpoSummary | null {
  const slug = typeof r.slug === "string" && isIpoSlug(r.slug) ? r.slug : null;
  const name = (lang === "zh" ? text(r.name_zh) : null) ?? text(r.name);
  const status = r.status as IpoStatus;
  if (!slug || !name || !IPO_STATUSES.includes(status)) return null;
  return { slug, name, symbol: text(r.symbol), status, date: dateLine(r, status, today) };
}

// The earliest session comes from the trading table's primary key (slug, session_date): one index probe per row.
const SUMMARY_COLUMNS = `slug, name, name_zh, symbol, status, first_trade_date, date_status, date_source,
  date_source_name, date_source_url, date_quote, date_as_of,
  (SELECT MIN(session_date) FROM ipo_trading WHERE ipo_trading.slug = ipo_watch.slug) AS first_session`;

/**
 * Tracked IPOs whose first trading day falls from `from` to `to` (ET dates), withdrawn ones and passed reported
 * dates left out (`today` is the ET date), by date then name: one read on idx_ipo_watch_date. Before migration 0009
 * is applied the table is missing; the calendar then shows no IPOs.
 */
export async function listIposBetween(
  db: D1Like,
  from: string,
  to: string,
  lang: Lang,
  today: string = todayEt(),
): Promise<IpoSummary[]> {
  try {
    const { results } = await db
      .prepare(
        `SELECT ${SUMMARY_COLUMNS} FROM ipo_watch
         WHERE first_trade_date BETWEEN ? AND ? AND status <> 'withdrawn'
         ORDER BY first_trade_date, name LIMIT 50`,
      )
      .bind(from, to)
      .all<WatchRow>();
    return results.flatMap((r) => summary(r, lang, today) ?? []).filter((s) => s.date !== null);
  } catch {
    return [];
  }
}

/** The calendar's IPOs grouped by date, for `CalendarWeek`. */
export function iposByDate(ipos: IpoSummary[]): Record<string, IpoSummary[]> {
  const out: Record<string, IpoSummary[]> = {};
  for (const i of ipos) if (i.date) (out[i.date.date] ??= []).push(i);
  return out;
}

/** How long a listed IPO stays on the watch list after its first trading day. */
export const LISTED_DAYS = 30;

/**
 * The calendar page's "IPO watch" list: every IPO not yet listed (filed, range set, priced) and those listed in
 * the last 30 days (`today` is the ET date), dated ones first by date, then "date not set" (which includes a
 * reported date that has passed), each group by name.
 * The table is the hand-kept list (a handful a year), so the read is bounded by it and capped at 20 rows.
 * Empty before migration 0009.
 */
export async function listIpoWatch(db: D1Like, today: string, lang: Lang): Promise<IpoSummary[]> {
  const since = new Date(`${today}T00:00:00Z`);
  since.setUTCDate(since.getUTCDate() - LISTED_DAYS);
  try {
    const { results } = await db
      .prepare(
        `SELECT ${SUMMARY_COLUMNS} FROM ipo_watch
         WHERE status IN ('filed', 'range_set', 'priced') OR (status = 'listed' AND first_trade_date >= ?)
         ORDER BY first_trade_date IS NULL, first_trade_date, name LIMIT 20`,
      )
      .bind(since.toISOString().slice(0, 10))
      .all<WatchRow>();
    // Ordered again here: a passed reported date reads "date not set" though its row still has the date.
    const out = results.flatMap((r) => summary(r, lang, today) ?? []);
    const key = (i: IpoSummary) => (i.date ? `0${i.date.date}` : "1");
    return out.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  } catch {
    return [];
  }
}

interface FactRow {
  kind: unknown;
  slug?: unknown;
  value_num: unknown;
  value_text: unknown;
  quote: unknown;
  form: unknown;
  filing_date: unknown;
  index_url: unknown;
  accession: unknown;
}

function filingOf(r: { accession: unknown; form: unknown; filing_date: unknown; index_url: unknown }): IpoFiling | null {
  const accession = text(r.accession);
  const form = text(r.form);
  const filingDate = dateOf(r.filing_date);
  if (!accession || !form || !filingDate) return null;
  return { accession, form, filingDate, indexUrl: secUrl(r.index_url) };
}

function factsOf(rows: FactRow[]): IpoFacts | null {
  const facts: Partial<Record<FactKind, IpoFact>> = {};
  for (const r of rows) {
    const kind = r.kind as FactKind;
    const quote = text(r.quote);
    if (!FACT_KINDS.includes(kind) || !quote) continue;
    if (TEXT_KINDS.includes(kind)) {
      const v = DATE_KINDS.includes(kind) ? dateOf(r.value_text) : text(r.value_text);
      if (v) facts[kind] = { num: null, text: v, quote };
    } else {
      const v = positive(r.value_num);
      if (v !== null) facts[kind] = { num: v, text: null, quote };
    }
  }
  // A range is shown only whole: both ends, low not above high.
  const low = facts.price_low?.num;
  const high = facts.price_high?.num;
  if (low == null || high == null || low > high) {
    delete facts.price_low;
    delete facts.price_high;
  }
  // Every fact is shown with the filing it came from; without that filing row, none is shown.
  const filing = rows.length > 0 ? filingOf(rows[0]) : null;
  if (!filing || Object.keys(facts).length === 0) return null;
  return { filing, facts };
}

interface ItemRow {
  kind: unknown;
  url: unknown;
  outlet: unknown;
  published_on: unknown;
  quote: unknown;
  statement_en: unknown;
  statement_zh: unknown;
  speaker: unknown;
  value_usd: unknown;
}

function itemOf(r: ItemRow, lang: Lang): IpoItem | null {
  const kind = r.kind as ItemKind;
  const outlet = text(r.outlet);
  const publishedOn = dateOf(r.published_on);
  const quote = text(r.quote);
  if (!ITEM_KINDS.includes(kind) || !outlet || !publishedOn || !quote) return null;
  const view = kind === "view";
  // The contract's form is "{speaker}, quoted by {outlet}": a view with no speaker would read as the outlet's own.
  if (view && !text(r.speaker)) return null;
  return {
    kind,
    url: httpsUrl(r.url),
    outlet,
    publishedOn,
    quote,
    // Only a company move carries a statement (contract); a valuation renders from value_usd, a view from its
    // speaker and quote. A move with none shows its attributed quote alone.
    statement: kind === "company_move" ? ((lang === "zh" ? text(r.statement_zh) : null) ?? text(r.statement_en)) : null,
    speaker: view ? text(r.speaker) : null,
    valueUsd: kind === "valuation" ? positive(r.value_usd) : null,
  };
}

interface TradingRow {
  session_date: unknown;
  open: unknown;
  close: unknown;
  volume: unknown;
  ipo_price: unknown;
  source_url: unknown;
}

function sessionOf(r: TradingRow): IpoSession | null {
  const sessionDate = dateOf(r.session_date);
  const close = positive(r.close);
  if (!sessionDate || close === null) return null;
  const ipoPrice = positive(r.ipo_price);
  const volume = typeof r.volume === "number" && Number.isInteger(r.volume) && r.volume >= 0 ? r.volume : null;
  return {
    sessionDate,
    open: positive(r.open),
    close,
    volume,
    ipoPrice,
    // Computed from the two numbers printed beside it (each rounded to cents as printed), never read from the
    // stored change_pct, so the three numbers on the line always agree.
    changePct: ipoPrice === null ? null : round2(((round2(close) - round2(ipoPrice)) / round2(ipoPrice)) * 100),
    sourceUrl: yahooUrl(r.source_url),
  };
}

/**
 * One IPO's page, or null for a slug that is not tracked (the page is then a 404). One primary-key read for the
 * IPO, then four reads at once, each on a primary key or an index of migration 0009: the current cover facts
 * with their filing, the filing timeline (at most 50), the news items (at most 40; the generator keeps 29) and the
 * first and latest trading sessions.
 */
export async function loadIpo(db: D1Like, slug: string, lang: Lang, today: string = todayEt()): Promise<IpoPage | null> {
  if (!isIpoSlug(slug)) return null;
  const row = await db
    .prepare(`SELECT ${SUMMARY_COLUMNS}, exchange, updated_at FROM ipo_watch WHERE slug = ?`)
    .bind(slug)
    .first<WatchRow>();
  const base = row ? summary(row, lang, today) : null;
  if (!row || !base) return null;

  const [facts, filings, items, sessions, writeups] = await Promise.all([
    db
      .prepare(
        `SELECT f.kind, f.slug, f.value_num, f.value_text, f.quote, f.accession, g.form, g.filing_date, g.index_url
         FROM ipo_facts f LEFT JOIN ipo_filings g ON g.accession = f.accession
         WHERE f.accession = (SELECT facts_accession FROM ipo_watch WHERE slug = ?)`,
      )
      .bind(slug)
      .all<FactRow>(),
    db
      .prepare(
        `SELECT accession, form, filing_date, index_url FROM ipo_filings
         WHERE slug = ? ORDER BY filing_date DESC, accession DESC LIMIT 50`,
      )
      .bind(slug)
      .all<FactRow>(),
    db
      .prepare(
        `SELECT kind, url, outlet, published_on, quote, statement_en, statement_zh, speaker, value_usd
         FROM ipo_items WHERE slug = ? AND kind IN ('view', 'valuation', 'company_move')
         ORDER BY kind DESC, published_on DESC, found_at DESC LIMIT 40`,
      )
      .bind(slug)
      .all<ItemRow>(),
    db
      .prepare(
        `SELECT session_date, open, close, volume, ipo_price, source_url FROM ipo_trading
         WHERE slug = ? AND session_date IN (
           (SELECT MIN(session_date) FROM ipo_trading WHERE slug = ?),
           (SELECT MAX(session_date) FROM ipo_trading WHERE slug = ?))
         ORDER BY session_date`,
      )
      .bind(slug, slug, slug)
      .all<TradingRow>(),
    listIpoWriteups(db, slug, lang),
  ]);

  // Facts of another IPO's filing (a wrong facts_accession) are not this IPO's.
  const ownFacts = facts.results.filter((r) => r.slug === slug);
  const list = items.results.flatMap((r) => itemOf(r, lang) ?? []);
  // Newest first within a kind, as the query orders them.
  const of = (kind: ItemKind) => list.filter((i) => i.kind === kind);
  // The query returns the earliest and the latest rows by date, in that order. Each is read in its own place, so
  // an unusable earliest row hides the first day and never moves the latest session into it.
  const rows = sessions.results;
  const first = rows.length > 0 ? sessionOf(rows[0]) : null;
  const latest = rows.length > 1 ? sessionOf(rows[rows.length - 1]) : null;

  return {
    ...base,
    exchange: text(row.exchange),
    facts: factsOf(ownFacts),
    filings: filings.results.flatMap((r) => filingOf(r) ?? []),
    valuations: of("valuation"),
    moves: of("company_move"),
    views: of("view").slice(0, VIEWS_SHOWN),
    trading:
      first || latest
        ? { first, latest, firstIsDayOne: first !== null && base.date?.source === "trading" && base.date.date === first.sessionDate }
        : null,
    updatedAt: text(row.updated_at),
    writeups,
  };
}

/** A UTC RFC 3339 time as the generator writes it (`2026-11-11T15:00:00Z`), else null. */
function utcTime(v: unknown): string | null {
  if (typeof v !== "string" || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,9})?Z$/.test(v)) return null;
  return isDate(v.slice(0, 10)) && !Number.isNaN(Date.parse(v)) ? v : null;
}

/** Every tracked IPO's slug and last update, for the sitemap: at most 200 rows. Empty before migration 0009. */
export async function listIpoSlugs(db: D1Like): Promise<{ slug: string; updatedAt: string | null }[]> {
  try {
    const { results } = await db
      .prepare("SELECT slug, updated_at FROM ipo_watch ORDER BY slug LIMIT 200")
      .all<{ slug: unknown; updated_at: unknown }>();
    return results.flatMap((r) =>
      typeof r.slug === "string" && isIpoSlug(r.slug) ? [{ slug: r.slug, updatedAt: utcTime(r.updated_at) }] : [],
    );
  } catch {
    return [];
  }
}

const PRICE = new Intl.NumberFormat("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const COUNT = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });
/** Six significant digits: enough to drop float noise (1.4e12 / 1e12), never enough to round a reported figure. */
const SCALED = new Intl.NumberFormat("en-US", { maximumSignificantDigits: 6, useGrouping: false });

/** A share count with separators: "825,000,000". */
export const fmtShares = (n: number): string => COUNT.format(n);
/** A per-share price: "70.00". */
export const fmtSharePrice = (n: number): string => PRICE.format(n);

/** Valuations outside this range are not printed as a figure (the attributed quote still shows). */
export const VALUATION_MIN = 1e6;
export const VALUATION_MAX = 1e14;

/** Units largest first: [divisor, English word or Chinese suffix]. */
const EN_UNITS: [number, string][] = [
  [1e12, "trillion"],
  [1e9, "billion"],
  [1e6, "million"],
];
const ZH_UNITS: [number, string][] = [
  [1e12, "万亿美元"],
  [1e8, "亿美元"],
  [1e4, "万美元"],
];

/**
 * A stored valuation in words, or null when it is not a plausible one (non-finite, under $1 million or over $100
 * trillion). The unit comes from the raw value and the number is printed as stored (only float noise is rounded
 * away), so the figure is the one the quote beside it gives: "$1.375 trillion", "$29.25 billion", "$999.6 billion";
 * "1.375万亿美元", "292.5亿美元", "9996亿美元". Only when the printed number itself reaches the next unit (1000, or
 * 10000 for 亿 and 万) is it promoted, so nothing reads "$1,000 billion" or "10000亿美元".
 */
export function fmtValuation(usd: number, lang: Lang): string | null {
  if (!Number.isFinite(usd) || usd < VALUATION_MIN || usd > VALUATION_MAX) return null;
  const units = lang === "zh" ? ZH_UNITS : EN_UNITS;
  const next = lang === "zh" ? 10000 : 1000;
  let i = units.findIndex(([d]) => usd >= d);
  let n = SCALED.format(usd / units[i][0]);
  if (Number(n) >= next && i > 0) {
    i -= 1;
    n = SCALED.format(usd / units[i][0]);
  }
  return lang === "zh" ? `${n}${units[i][1]}` : `$${n} ${units[i][1]}`;
}

/** A write-up post about an IPO (`content_type = 'ipo'`, run key `ipo:<ipo_slug>:<milestone>:<key>`). */
export interface IpoWriteup {
  slug: string;
  title: string;
  /** The filing (or session) date the write-up is about. */
  tradingDate: string;
}

/** One active IPO as the home page's section shows it. */
export interface HomeIpo extends IpoSummary {
  exchange: string | null;
  /** The offering's price, from the current cover facts, with the filing they came from; null without a filing row. */
  price: { low: number; high: number } | { final: number } | null;
  /** The filing the price came from. */
  priceFiling: IpoFiling | null;
  /** The newest stored valuation with a printable figure, attributed, with the article's sentence it came from. */
  valuation: { usd: number; outlet: string; publishedOn: string; url: string | null; quote: string } | null;
  /** The latest stored session after listing, with the change computed as the IPO page computes it. */
  latest: IpoSession | null;
  /** The newest write-up published in the page's language. */
  writeup: IpoWriteup | null;
}

/**
 * The home section's active rule is the news sweep's, `news::active` in crates/core/src/ipo/news.rs (compare the two
 * there): not withdrawn; after listing, no more than `ACTIVE_DAYS_AFTER_LISTING` (30) days past the first stored
 * session; before listing, a filing within `ACTIVE_DAYS_WITHOUT_FILING` (60) days, so a postponed deal leaves the
 * front page. The IPO page and the calendar do not use it.
 */
export const HOME_DAYS_AFTER_LISTING = 30;
export const HOME_DAYS_WITHOUT_FILING = 60;

/** At most this many IPOs show in the home section, soonest first; the rest are a link to the calendar. */
export const HOME_IPOS_SHOWN = 3;

const POST_SLUG = /^[a-z0-9][a-z0-9-]*$/;

/** A JSON object produced by SQLite's json_object, or null. */
function jsonObj(v: unknown): Record<string, unknown> | null {
  if (typeof v !== "string") return null;
  try {
    const o = JSON.parse(v) as unknown;
    return o && typeof o === "object" && !Array.isArray(o) ? (o as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/**
 * The home page's IPOs: every tracked IPO that is active by the news sweep's rule (above), in one D1 read. Each row carries,
 * through scalar subqueries on the primary keys and indexes of migration 0009 and posts' run_key: the price from
 * its current cover facts and that filing, the newest valuation, the latest session, and the newest write-up in
 * `lang` (found by run-key range). Empty, never an error, before migration 0009 or with no IPO tracked.
 */
export async function listHomeIpos(db: D1Like, lang: Lang, today: string): Promise<HomeIpo[]> {
  let rows: (WatchRow & Record<string, unknown>)[];
  try {
    ({ results: rows } = await db
      .prepare(
        `SELECT ${SUMMARY_COLUMNS}, exchange,
           (SELECT MAX(filing_date) FROM ipo_filings WHERE ipo_filings.slug = ipo_watch.slug) AS last_filing,
           (SELECT json_object('form', g.form, 'filing_date', g.filing_date, 'index_url', g.index_url, 'accession', g.accession)
              FROM ipo_filings g WHERE g.accession = ipo_watch.facts_accession AND g.slug = ipo_watch.slug) AS price_filing,
           (SELECT value_num FROM ipo_facts WHERE accession = ipo_watch.facts_accession AND kind = 'price_low' AND slug = ipo_watch.slug) AS price_low,
           (SELECT value_num FROM ipo_facts WHERE accession = ipo_watch.facts_accession AND kind = 'price_high' AND slug = ipo_watch.slug) AS price_high,
           (SELECT value_num FROM ipo_facts WHERE accession = ipo_watch.facts_accession AND kind = 'price_final' AND slug = ipo_watch.slug) AS price_final,
           (SELECT json_object('usd', value_usd, 'outlet', outlet, 'published_on', published_on, 'url', url, 'quote', quote)
              FROM ipo_items WHERE ipo_items.slug = ipo_watch.slug AND kind = 'valuation'
                AND value_usd >= ${VALUATION_MIN} AND value_usd <= ${VALUATION_MAX} AND trim(outlet) <> '' AND trim(quote) <> ''
              ORDER BY published_on DESC, found_at DESC LIMIT 1) AS valuation,
           (SELECT json_object('session_date', session_date, 'close', close, 'ipo_price', ipo_price, 'source_url', source_url)
              FROM ipo_trading WHERE ipo_trading.slug = ipo_watch.slug ORDER BY session_date DESC LIMIT 1) AS latest,
           (SELECT json_object('slug', p.slug, 'title', t.title, 'trading_date', p.trading_date)
              FROM posts p CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ? AND t.status = 'published'
              WHERE p.run_key >= 'ipo:' || ipo_watch.slug || ':' AND p.run_key < 'ipo:' || ipo_watch.slug || ';'
                AND p.content_type = 'ipo'
              ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC LIMIT 1) AS writeup
         FROM ipo_watch
         WHERE status <> 'withdrawn'
           -- Active (news::active): by the first stored session once there is one, else by the newest filing. So the
           -- LIMIT counts active IPOs only, and an IPO listed long ago costs no subqueries.
           AND COALESCE(
             (SELECT MIN(session_date) FROM ipo_trading WHERE ipo_trading.slug = ipo_watch.slug) >= ?,
             (SELECT MAX(filing_date) FROM ipo_filings WHERE ipo_filings.slug = ipo_watch.slug) >= ?,
             0)
         ORDER BY first_trade_date IS NULL, first_trade_date, name LIMIT 20`,
      )
      .bind(lang, addDays(today, -HOME_DAYS_AFTER_LISTING), addDays(today, -HOME_DAYS_WITHOUT_FILING))
      .all<WatchRow & Record<string, unknown>>());
  } catch {
    return [];
  }
  const listedSince = addDays(today, -HOME_DAYS_AFTER_LISTING);
  const filedSince = addDays(today, -HOME_DAYS_WITHOUT_FILING);
  const out = rows.flatMap((r): HomeIpo[] => {
    const base = summary(r, lang, today);
    if (!base || base.status === "withdrawn") return [];
    // Active, as `news::active` (the query applies it too; this is the second gate).
    const first = dateOf(r.first_session);
    if (first ? first < listedSince : !(dateOf(r.last_filing) && dateOf(r.last_filing)! >= filedSince)) return [];

    const filingRow = jsonObj(r.price_filing);
    const priceFiling = filingRow
      ? filingOf({ accession: filingRow.accession, form: filingRow.form, filing_date: filingRow.filing_date, index_url: filingRow.index_url })
      : null;
    const low = positive(r.price_low);
    const high = positive(r.price_high);
    const final = positive(r.price_final);
    // A price is shown only with the filing it came from, and a range only whole.
    const price = !priceFiling ? null : final !== null ? { final } : low !== null && high !== null && low <= high ? { low, high } : null;

    const v = jsonObj(r.valuation);
    const usd = typeof v?.usd === "number" ? v.usd : null;
    const outlet = text(v?.outlet);
    const publishedOn = dateOf(v?.published_on);
    const quote = text(v?.quote);
    // No figure from news without its sentence: a valuation without its quote is not shown.
    const valuation =
      usd !== null && fmtValuation(usd, lang) !== null && outlet && publishedOn && quote
        ? { usd, outlet, publishedOn, url: httpsUrl(v?.url), quote }
        : null;

    const s = jsonObj(r.latest);
    const latest = s ? sessionOf({ session_date: s.session_date, open: null, close: s.close, volume: null, ipo_price: s.ipo_price, source_url: s.source_url }) : null;

    const w = jsonObj(r.writeup);
    const wSlug = typeof w?.slug === "string" && POST_SLUG.test(w.slug) ? w.slug : null;
    const wTitle = text(w?.title);
    const wDate = dateOf(w?.trading_date);
    const writeup = wSlug && wTitle && wDate ? { slug: wSlug, title: wTitle, tradingDate: wDate } : null;

    return [{ ...base, exchange: text(r.exchange), price, priceFiling: price ? priceFiling : null, valuation, latest, writeup }];
  });
  // Soonest first: dated IPOs by date (a passed reported date reads "date not set" and moves down), then by name.
  const key = (i: HomeIpo) => (i.date ? `0${i.date.date}` : "1");
  return out.sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

/**
 * An IPO's write-ups published in `lang`, newest first (at most 12), by run-key range on posts' unique run_key
 * index: `ipo:<slug>:` up to `ipo:<slug>;`.
 */
export async function listIpoWriteups(db: D1Like, slug: string, lang: Lang): Promise<IpoWriteup[]> {
  if (!isIpoSlug(slug)) return [];
  try {
    const { results } = await db
      .prepare(
        `SELECT p.slug, t.title, p.trading_date FROM posts p
         CROSS JOIN post_translations t ON t.post_id = p.id AND t.lang = ? AND t.status = 'published'
         WHERE p.run_key >= ? AND p.run_key < ? AND p.content_type = 'ipo'
         ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC LIMIT 12`,
      )
      .bind(lang, `ipo:${slug}:`, `ipo:${slug};`)
      .all<{ slug: unknown; title: unknown; trading_date: unknown }>();
    return results.flatMap((r) => {
      const s = typeof r.slug === "string" && POST_SLUG.test(r.slug) ? r.slug : null;
      const title = text(r.title);
      const tradingDate = dateOf(r.trading_date);
      return s && title && tradingDate ? [{ slug: s, title, tradingDate }] : [];
    });
  } catch {
    return [];
  }
}

/** The IPO a write-up is about, from its facts key `ipo/<ipo_slug>/<milestone>-<key>.json`, or null. */
export function writeupIpoSlug(factsKey: string | null): string | null {
  const m = factsKey ? /^ipo\/([a-z0-9-]+)\/[a-z_]+-[A-Za-z0-9-]+\.json$/.exec(factsKey) : null;
  return m && isIpoSlug(m[1]) ? m[1] : null;
}
