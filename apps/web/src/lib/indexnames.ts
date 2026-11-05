import type { translator } from "./translator";

/** `=` and `^` are awkward in message keys, so symbols map to `index.*` keys here. */
const NAME_KEY = {
  "ES=F": "index.es",
  "NQ=F": "index.nq",
  "YM=F": "index.ym",
  "RTY=F": "index.rty",
  "^GSPC": "index.gspc",
  "^NDX": "index.ndx",
  "^IXIC": "index.ixic",
  "^DJI": "index.dji",
  "^RUT": "index.rut",
} as const;
type NameKey = (typeof NAME_KEY)[keyof typeof NAME_KEY];

/** The localized index or futures name for a quote symbol; an unknown symbol stands for itself. */
export function indexName(t: ReturnType<typeof translator>, symbol: string): string {
  const key = (NAME_KEY as Record<string, NameKey | undefined>)[symbol];
  return key ? t(key) : symbol;
}
