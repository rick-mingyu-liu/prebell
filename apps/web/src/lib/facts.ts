import { dirOf, type Dir } from "./format";

export type { Dir };

export interface QuoteView {
  symbol: string;
  last: number;
  changePct: number;
  /** `last - prev_close`, 2 decimals. The only derived number the UI shows. */
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

export interface FactsView {
  kind: "premarket" | "recap";
  tradingDate: string;
  asOfUtc: string;
  lead: IndexView;
  others: IndexView[];
  gainers: MoverView[];
  losers: MoverView[];
}

/** The slice of the R2 binding `loadFacts` uses. */
export interface R2Like {
  get(key: string): Promise<{ text(): Promise<string> } | null>;
}

type Kind = FactsView["kind"];

/** Symbols per slot, lead first. The recap generator quotes ^NDX for Nasdaq. */
const SLOTS: Record<Kind, string[][]> = {
  premarket: [["ES=F"], ["NQ=F"], ["YM=F"], ["RTY=F"]],
  recap: [["^GSPC"], ["^IXIC", "^NDX"], ["^DJI"], ["^RUT"]],
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

const round2 = (n: number): number => Math.round(n * 100) / 100;

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
    return { symbol: symbols[0], quote };
  });
}

function moverView(v: unknown): MoverView {
  const m = obj(v, "mover");
  const quote = quoteView(m.quote);
  const first = arr(m.reasons, "reasons")[0];
  const r = first === undefined ? null : obj(first, "reason");
  const summary = r ? str(r.summary_en, "summary_en").trim() : "";
  const url = r ? str(r.url, "url") : "";
  return {
    symbol: quote.symbol,
    name: str(m.name, "name"),
    quote,
    reason: summary || null,
    reasonUrl: summary && url ? url : null,
  };
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
    };
  } catch (e) {
    if (e instanceof ShapeError) return null;
    throw e;
  }
}

/** Reads and parses one facts object. Never throws: any failure is null (callers fall back to markdown). */
export async function loadFacts(bucket: R2Like | undefined, key: string | null, kind: Kind): Promise<FactsView | null> {
  if (!bucket || !key) return null;
  try {
    const object = await bucket.get(key);
    if (!object) return null;
    return parseFacts(kind, JSON.parse(await object.text()));
  } catch {
    return null;
  }
}
