/**
 * An earnings check's stored facts (`crates/core/src/earnings/facts.rs`, `EarningsFacts`), read for the MCP
 * server. The generator writes one JSON object per check to R2 at the post's `facts_r2_key`
 * (`earnings/{TICKER}/{accession}.json`). Parsing is lenient: each section is read on its own, a malformed one
 * reads as absent, and every field added after launch (`share_move`, a note's `cause`, `scale`, `current`, a
 * figure's `table_scale`) is optional.
 */

import type { Lang } from "./posts";
import type { R2Like } from "./facts";

/** Metric keys: the `Metrics` struct's field names, in `Metric::ALL` order. */
export const METRIC_KEYS = [
  "revenue",
  "net_income",
  "eps_gaap",
  "eps_non_gaap",
  "gross_margin",
  "operating_margin",
  "operating_cash_flow",
  "free_cash_flow",
] as const;
export type MetricKey = (typeof METRIC_KEYS)[number];

/** `Metric::label_en` / `label_zh`; a test keeps this table equal to the Rust one. */
export const METRIC_LABELS: Record<MetricKey, { en: string; zh: string }> = {
  revenue: { en: "Revenue", zh: "营收" },
  net_income: { en: "Net income (GAAP)", zh: "净利润（GAAP）" },
  eps_gaap: { en: "Diluted EPS (GAAP)", zh: "摊薄每股收益（GAAP）" },
  eps_non_gaap: { en: "Diluted EPS (non-GAAP)", zh: "摊薄每股收益（非GAAP）" },
  gross_margin: { en: "Gross margin", zh: "毛利率" },
  operating_margin: { en: "Operating margin (GAAP)", zh: "营业利润率（GAAP）" },
  operating_cash_flow: { en: "Operating cash flow", zh: "经营现金流" },
  free_cash_flow: { en: "Free cash flow (non-GAAP)", zh: "自由现金流（非GAAP）" },
};

const UNITS = { Usd: "usd", UsdPerShare: "usd_per_share", Percent: "percent" } as const;
export type FigureUnit = (typeof UNITS)[keyof typeof UNITS];

export interface FigureView {
  key: MetricKey;
  /** In base units: dollars, dollars per share, or percent (47.2 is 47.2%). Signed. */
  value: number;
  unit: FigureUnit;
  /** The amount exactly as the press release wrote it. */
  amount: string;
  /** The press release's sentence or table row the amount was read from, verbatim (English). */
  quote: string;
}

export interface NoteView {
  /** A short English summary whose numbers all come from the quote. */
  summaryEn: string;
  quote: string;
}

export interface ShareMoveView {
  session: "premarket" | "regular" | "after_hours";
  /** Percent, one decimal, as printed: -4.2 is -4.2%. */
  changePct: number;
  last: number;
  /** The quote's own time (UTC ISO). */
  asOf: string;
}

export interface EarningsFactsView {
  ticker: string;
  company: string | null;
  filingDate: string | null;
  acceptedAt: string | null;
  /** The EDGAR filing index page. */
  filingUrl: string | null;
  /** The Exhibit 99.1 press release. */
  pressReleaseUrl: string | null;
  fiscalPeriod: { quarter: number | null; fiscalYear: number; quote: string | null } | null;
  figures: FigureView[];
  guidance: NoteView[];
  highlights: NoteView[];
  shareMove: ShareMoveView | null;
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): string | null => (typeof v === "string" && v.trim() !== "" ? v : null);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);
/** Only SEC links are passed on: the facts hold no other kind. */
const secUrl = (v: unknown): string | null =>
  typeof v === "string" && /^https:\/\/www\.sec\.gov\/[\w./?=&%-]*$/.test(v) ? v : null;

/** serde's `Field<T>`: the value of `{"Value": v}`, else null (`{"Missing": ...}` or anything else). */
function value(v: unknown): unknown {
  return isObj(v) && "Value" in v ? v.Value : null;
}

function figure(key: MetricKey, v: unknown): FigureView | null {
  const f = value(v);
  if (!isObj(f)) return null;
  const val = num(f.value);
  const unit = typeof f.unit === "string" ? (UNITS as Record<string, FigureUnit | undefined>)[f.unit] : undefined;
  const amount = str(f.amount);
  const quote = str(f.quote);
  if (val === null || !unit || !amount || !quote) return null;
  return { key, value: val, unit, amount, quote };
}

function notes(v: unknown): NoteView[] {
  const list = value(v);
  if (!Array.isArray(list)) return [];
  const out: NoteView[] = [];
  for (const n of list) {
    if (!isObj(n)) continue;
    const quote = str(n.quote);
    const summaryEn = str(n.summary_en);
    if (quote && summaryEn) out.push({ summaryEn, quote });
  }
  return out;
}

function period(v: unknown): EarningsFactsView["fiscalPeriod"] {
  const p = value(v);
  if (!isObj(p)) return null;
  const year = num(p.fiscal_year);
  if (year === null || !Number.isInteger(year)) return null;
  const q = num(p.quarter);
  return { quarter: q !== null && Number.isInteger(q) && q >= 1 && q <= 4 ? q : null, fiscalYear: year, quote: str(p.quote) };
}

const SESSIONS = ["premarket", "regular", "after_hours"] as const;

function shareMove(v: unknown): ShareMoveView | null {
  const m = value(v);
  if (!isObj(m)) return null;
  const session = SESSIONS.find((s) => s === m.session);
  const changePct = num(m.change_pct);
  const last = num(m.last);
  const asOf = str(m.as_of);
  if (!session || changePct === null || last === null || !asOf || Number.isNaN(Date.parse(asOf))) return null;
  return { session, changePct, last, asOf };
}

/**
 * The facts JSON to what the MCP server returns, or null when it is not an earnings facts object for `ticker`
 * (no object, or another company's). Sections that do not parse are left out, never guessed.
 */
export function parseEarningsFacts(json: unknown, ticker: string): EarningsFactsView | null {
  if (!isObj(json) || json.ticker !== ticker) return null;
  const metrics = isObj(json.metrics) ? json.metrics : {};
  return {
    ticker,
    company: str(json.company),
    filingDate: str(json.filing_date),
    acceptedAt: str(json.accepted_at),
    filingUrl: secUrl(json.filing_url),
    pressReleaseUrl: secUrl(value(json.exhibit_url)),
    fiscalPeriod: period(json.fiscal_period),
    figures: METRIC_KEYS.map((k) => figure(k, metrics[k])).filter((f): f is FigureView => f !== null),
    guidance: notes(json.guidance),
    highlights: notes(json.highlights),
    shareMove: shareMove(json.share_move),
  };
}

/** R2 keys the earnings build writes: `earnings/{TICKER with . as -}/{accession}.json`. */
const EARNINGS_KEY = /^earnings\/[A-Z0-9-]{1,10}\/\d{10}-\d{2}-\d{6}\.json$/;

/**
 * Reads and parses one earnings facts object (one R2 read). Never throws: a missing key, object, bad JSON or
 * wrong shape is null; `onError` hears about R2 errors, bad JSON and shape errors, not a missing object.
 */
export async function loadEarningsFacts(
  bucket: R2Like | undefined,
  key: string | null,
  ticker: string,
  onError?: (error: unknown) => void,
): Promise<EarningsFactsView | null> {
  if (!bucket || !key || !EARNINGS_KEY.test(key)) return null;
  const report = (e: unknown): null => {
    try {
      onError?.(e);
    } catch {}
    return null;
  };
  try {
    const object = await bucket.get(key);
    if (!object) return null;
    return parseEarningsFacts(JSON.parse(await object.text()), ticker) ?? report(new Error(`facts ${key}: unexpected shape`));
  } catch (e) {
    return report(e);
  }
}

/** "Q4 FY2026" / "2026财年第四季度"; a full-year release is "FY2026" / "2026财年". */
export function periodLabel(p: { quarter: number | null; fiscalYear: number }, lang: Lang): string {
  if (lang === "zh") return p.quarter ? `${p.fiscalYear}财年第${"一二三四"[p.quarter - 1]}季度` : `${p.fiscalYear}财年`;
  return p.quarter ? `Q${p.quarter} FY${p.fiscalYear}` : `FY${p.fiscalYear}`;
}

/**
 * The fiscal period an earnings post's slug was named from (`{ticker}-q4-fy2026-earnings`, `-fy2026-earnings`,
 * either with a `-{6 digits}` accession suffix), for a check whose facts cannot be read. Null for a slug named
 * by filing date (facts without a period).
 */
export function periodFromSlug(slug: string): { quarter: number | null; fiscalYear: number } | null {
  const m = /-(?:q([1-4])-)?fy(\d{4})-earnings(?:-\d{6})?$/.exec(slug);
  return m ? { quarter: m[1] ? Number(m[1]) : null, fiscalYear: Number(m[2]) } : null;
}
