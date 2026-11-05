/** The generator's index snapshot (`kv` key `index_line`): futures before the open, index closes after. */

import { dirOf, type Dir } from "./format";
import type { D1Like } from "./posts";

export interface IndexEntryView {
  symbol: string;
  last: number;
  changePct: number;
  dir: Dir;
}

export interface IndexLineView {
  kind: "premarket" | "recap";
  asOfUtc: string;
  entries: IndexEntryView[];
}

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

function parseEntry(raw: unknown): IndexEntryView | null {
  if (!isObject(raw)) return null;
  const { symbol, last, change_pct: changePct } = raw;
  if (typeof symbol !== "string" || symbol === "") return null;
  if (typeof last !== "number" || !Number.isFinite(last)) return null;
  if (typeof changePct !== "number" || !Number.isFinite(changePct)) return null;
  return { symbol, last, changePct, dir: dirOf(changePct) };
}

/** The snapshot's validated view, or null when anything about its shape is off. */
export function parseIndexLine(json: unknown): IndexLineView | null {
  if (!isObject(json)) return null;
  const { kind, as_of_utc: asOfUtc, entries } = json;
  if (kind !== "premarket" && kind !== "recap") return null;
  if (typeof asOfUtc !== "string" || asOfUtc === "") return null;
  if (!Array.isArray(entries) || entries.length === 0) return null;
  const parsed = entries.map(parseEntry);
  if (parsed.some((e) => e === null)) return null;
  return { kind, asOfUtc, entries: parsed as IndexEntryView[] };
}

/** The stored snapshot, or null when it is missing or unreadable. */
export async function loadIndexLine(db: D1Like): Promise<IndexLineView | null> {
  const row = await db
    .prepare("SELECT value FROM kv WHERE key = 'index_line'")
    .first<{ value: string }>();
  if (!row) return null;
  try {
    return parseIndexLine(JSON.parse(row.value));
  } catch {
    return null;
  }
}
