import { dirOf, round2, type Dir } from "./format";

export type { Dir };

export interface QuoteView {
  symbol: string;
  last: number;
  changePct: number;
  /** `last - prev_close`, 2 decimals. The only derived number the UI shows. Display it with `fmtPoint(pointChange, dir)`. */
  pointChange: number;
  dir: Dir;
}

/** `quote: null` means the generator marked the value Missing. */
export interface IndexView {
  symbol: string;
  quote: QuoteView | null;
}

export interface MoverView {
  symbol: string;
  name: string;
  quote: QuoteView;
  /** English only (`summary_en`) of the first reason. */
  reason: string | null;
  reasonUrl: string | null;
}

export interface ThemeMemberView {
  symbol: string;
  changePct: number;
}

/** A static industry group that moved together; `more` counts members beyond the ones listed. */
export interface ThemeView {
  group: string;
  /** Chinese label; falls back to `group` when the generator wrote none. */
  groupZh: string;
  direction: "up" | "down";
  medianPct: number;
  members: ThemeMemberView[];
  more: number;
}

export interface FactsView {
  kind: "premarket" | "recap";
  tradingDate: string;
  asOfUtc: string;
  lead: IndexView;
  others: IndexView[];
  gainers: MoverView[];
  losers: MoverView[];
  /** Empty on posts written before themes existed, or when `themes` is malformed. */
  themes: ThemeView[];
}

/** The slice of the R2 binding `loadFacts` uses. */
export interface R2Like {
  get(key: string): Promise<{ text(): Promise<string> } | null>;
}

type Kind = FactsView["kind"];

/**
 * Accepted symbols per slot, lead first; the first is the generator's own
 * (the recap quotes ^NDX for Nasdaq) and names a Missing entry, which carries
 * no symbol. A present quote always reports the symbol it actually has.
 */
const SLOTS: Record<Kind, string[][]> = {
  premarket: [["ES=F"], ["NQ=F"], ["YM=F"], ["RTY=F"]],
  recap: [["^GSPC"], ["^NDX", "^IXIC"], ["^DJI"], ["^RUT"]],
};
const LIST_KEY: Record<Kind, string> = { premarket: "futures", recap: "indexes" };

class ShapeError extends Error {}

function fail(what: string): never {
  throw new ShapeError(what);
}

function obj(v: unknown, what: string): Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : fail(what);
}
function arr(v: unknown, what: string): unknown[] {
  return Array.isArray(v) ? v : fail(what);
}
function str(v: unknown, what: string): string {
  return typeof v === "string" ? v : fail(what);
}
function finite(v: unknown, what: string): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fail(what);
}

function quoteView(v: unknown): QuoteView {
  const q = obj(v, "quote");
  const last = finite(q.last, "last");
  const prev = finite(q.prev_close, "prev_close");
  const changePct = finite(q.change_pct, "change_pct");
  return { symbol: str(q.symbol, "symbol"), last, changePct, pointChange: round2(last - prev), dir: dirOf(changePct) };
}

/** serde's `Field<Quote>`: `{"Value": q}` or `{"Missing": {...}}`. */
function fieldQuote(v: unknown): QuoteView | null {
  const f = obj(v, "field");
  if ("Value" in f) return quoteView(f.Value);
  if ("Missing" in f) return null;
  return fail("field");
}

/** `[label, Field<Quote>]` tuples resolved to the four slots, by symbol then by position. */
function indexViews(kind: Kind, list: unknown[]): IndexView[] {
  const entries = list.map((e) => {
    const t = arr(e, "tuple");
    return fieldQuote(t[1]);
  });
  return SLOTS[kind].map((symbols, i) => {
    const quote = entries.find((q) => q !== null && symbols.includes(q.symbol)) ?? entries[i] ?? null;
    return { symbol: quote && symbols.includes(quote.symbol) ? quote.symbol : symbols[0], quote };
  });
}

function moverView(v: unknown): MoverView {
  const m = obj(v, "mover");
  const quote = quoteView(m.quote);
  const first = arr(m.reasons, "reasons")[0];
  const r = first === undefined ? null : obj(first, "reason");
  const summary = r ? str(r.summary_en, "summary_en").trim() : "";
  const url = r ? str(r.url, "url") : "";
  const safeUrl = /^https?:\/\//i.test(url) ? url : null;
  return {
    symbol: quote.symbol,
    name: str(m.name, "name"),
    quote,
    reason: summary || null,
    reasonUrl: summary ? safeUrl : null,
  };
}

function themeView(v: unknown): ThemeView {
  const t = obj(v, "theme");
  const group = str(t.group, "group").trim();
  if (!group) fail("group");
  const direction = t.direction === "Up" ? "up" : t.direction === "Down" ? "down" : fail("direction");
  const members = arr(t.members, "members").map((m) => {
    const o = obj(m, "member");
    return { symbol: str(o.symbol ?? o.ticker, "ticker"), changePct: finite(o.change_pct, "change_pct") };
  });
  const more = typeof t.more === "number" && Number.isInteger(t.more) && t.more > 0 ? t.more : 0;
  const zh = typeof t.group_zh === "string" ? t.group_zh.trim() : "";
  return { group, groupZh: zh || group, direction, medianPct: finite(t.median_pct, "median_pct"), members, more };
}

/** Optional and defensive: a missing or non-array `themes` is empty, and a malformed entry is skipped. */
function themeViews(v: unknown): ThemeView[] {
  if (!Array.isArray(v)) return [];
  const out: ThemeView[] = [];
  for (const e of v) {
    try {
      out.push(themeView(e));
    } catch (err) {
      if (!(err instanceof ShapeError)) throw err;
    }
  }
  return out;
}

/** Facts JSON (generator output) to what the pages render; null on any shape error. */
export function parseFacts(kind: Kind, json: unknown): FactsView | null {
  try {
    const f = obj(json, "facts");
    const [lead, ...others] = indexViews(kind, arr(f[LIST_KEY[kind]], LIST_KEY[kind]));
    return {
      kind,
      tradingDate: str(f.trading_date, "trading_date"),
      asOfUtc: str(f.as_of_utc, "as_of_utc"),
      lead,
      others,
      gainers: arr(f.gainers, "gainers").map(moverView),
      losers: arr(f.losers, "losers").map(moverView),
      themes: themeViews(f.themes),
    };
  } catch (e) {
    if (e instanceof ShapeError) return null;
    throw e;
  }
}

/**
 * Reads and parses one facts object. Never throws: any failure is null
 * (callers fall back to markdown). `onError` hears about R2 errors, bad JSON
 * and shape errors, so an outage or schema drift leaves a trace; a missing
 * object or key is normal and silent. A throwing `onError` is ignored.
 */
export async function loadFacts(
  bucket: R2Like | undefined,
  key: string | null,
  kind: Kind,
  onError?: (error: unknown) => void,
): Promise<FactsView | null> {
  if (!bucket || !key) return null;
  const report = (e: unknown): null => {
    try {
      onError?.(e);
    } catch {}
    return null;
  };
  try {
    const object = await bucket.get(key);
    if (!object) return null;
    const view = parseFacts(kind, JSON.parse(await object.text()));
    return view ?? report(new Error(`facts ${key}: unexpected shape for ${kind}`));
  } catch (e) {
    return report(e);
  }
}
