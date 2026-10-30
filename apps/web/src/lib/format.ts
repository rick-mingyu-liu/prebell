export type Dir = "up" | "down" | "flat";

/** U+2212, so the sign lines up with "+" in tabular numerals. */
const MINUS = "−";

const round2 = (n: number): number => Math.round(n * 100) / 100;

const num = new Intl.NumberFormat("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/** "7,665.50" */
export function fmtPrice(n: number): string {
  return num.format(n);
}

function signed(n: number, suffix: string): string {
  const r = round2(n);
  if (r === 0) return `${num.format(0)}${suffix}`;
  return `${r > 0 ? "+" : MINUS}${num.format(Math.abs(r))}${suffix}`;
}

/** "+1.14%", "−2.68%", "0.00%" (no sign when it rounds to zero). */
export function fmtPct(p: number): string {
  return signed(p, "%");
}

/** "+24.75", "−88.75", "0.00" */
export function fmtPoint(n: number): string {
  return signed(n, "");
}

/** Direction follows the percent change; flat when it rounds to 0.00. */
export function dirOf(changePct: number): Dir {
  const r = round2(changePct);
  return r === 0 ? "flat" : r > 0 ? "up" : "down";
}
