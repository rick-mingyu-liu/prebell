import { dirOf, fmtPct } from "../lib/format";

/** A percent change as a colored pill; the sign is always in the text. */
export default function Pill({ pct }: { pct: number }) {
  return <span className={`pill ${dirOf(pct)}`}>{fmtPct(pct)}</span>;
}
