export type Dir = "up" | "down" | "flat";

/** U+2212, so the sign lines up with "+" in tabular numerals. */
const MINUS = "−";
/** Shown in place of a number that is not finite. */
const DASH = "—";

const num = new Intl.NumberFormat("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

/**
 * Rounds to 2 decimals, half away from zero, so +x and -x always round to
 * mirrored values. Never returns -0. Every formatter here goes through it, so
 * the sign, the direction and the text cannot disagree.
 */
export function round2(n: number): number {
  const r = Math.sign(n) * (Math.round(Math.abs(n) * 100) / 100);
  return r === 0 ? 0 : r;
}

/** "7,665.50" ("−" for negatives, "—" when not finite). */
export function fmtPrice(n: number): string {
  if (!Number.isFinite(n)) return DASH;
  const r = round2(n);
  return `${r < 0 ? MINUS : ""}${num.format(Math.abs(r))}`;
}

function signed(n: number, suffix: string, dir?: Dir): string {
  if (!Number.isFinite(n)) return DASH;
  const r = round2(n);
  const d = dir ?? (r === 0 ? "flat" : r > 0 ? "up" : "down");
  const body = `${num.format(Math.abs(r))}${suffix}`;
  return d === "flat" ? body : `${d === "up" ? "+" : MINUS}${body}`;
}

/** "+1.14%", "−2.68%", "0.00%" (no sign when it rounds to zero). */
export function fmtPct(p: number): string {
  return signed(p, "%");
}

/**
 * "+24.75", "−88.75", "0.00". Pass the quote's `dir` so the sign follows the
 * percent change like the color does: a flat quote shows its point change
 * unsigned ("0.00%" never sits next to "+0.30"), and the magnitude is shown
 * under the sign that `dir` gives.
 */
export function fmtPoint(n: number, dir?: Dir): string {
  return signed(n, "", dir);
}

/** Direction follows the percent change; flat when it rounds to 0.00 or is not finite. */
export function dirOf(changePct: number): Dir {
  if (!Number.isFinite(changePct)) return "flat";
  const r = round2(changePct);
  return r === 0 ? "flat" : r > 0 ? "up" : "down";
}
