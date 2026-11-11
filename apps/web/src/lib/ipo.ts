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
 * does not pair, a news or filing date with no named source (an unattributed date is not shown), or a reported
 * date that has passed (before yesterday, ET). A link that fails its source's check is dropped; the date stays.
 */
function dateLine(r: WatchRow, status: IpoStatus, today: string): IpoDate | null {
  const date = dateOf(r.first_trade_date);
  const dateStatus = r.date_status as DateStatus;
  const source = r.date_source as DateSource;
  if (status === "withdrawn" || !date || !DATE_STATUSES.includes(dateStatus) || !DATE_SOURCES.includes(source)) return null;
  if (!DATE_PAIRS[dateStatus].includes(source)) return null;
  if (dateStatus === "reported" && reportedDatePassed(date, today)) return null;
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

const SUMMARY_COLUMNS = `slug, name, name_zh, symbol, status, first_trade_date, date_status, date_source,
  date_source_name, date_source_url, date_quote, date_as_of`;

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

  const [facts, filings, items, sessions] = await Promise.all([
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
