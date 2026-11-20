import { companyName } from "./earnings";
import { periodFromSlug } from "./earningsfacts";
import type { Lang } from "./posts";
import { formatDate } from "./time";

/** How long after a calendar quarter ends its results can still be "Q3 2026" rather than a fiscal quarter. */
const CALENDAR_WINDOW_DAYS = 120;

/**
 * True when "Q{quarter} {year}" read as a calendar quarter ended within the window before `tradingDate`, so
 * the plain wording is right (ConocoPhillips' Q3 2026, reported in November). A fiscal quarter whose calendar
 * namesake has not ended yet (Nvidia's Q3 fiscal 2027, Disney's Q4 fiscal 2026, both in November 2026) is not.
 */
export function calendarAligned(quarter: number, year: number, tradingDate: string): boolean {
  const end = Date.UTC(year, quarter * 3, 0); // day 0 of the next month: the quarter's last day
  const days = (Date.parse(`${tradingDate}T00:00:00Z`) - end) / 86_400_000;
  return days >= 0 && days <= CALENDAR_WINDOW_DAYS;
}

/**
 * An earnings write-up's search-result title (the HTML `<title>`; the page, cards and JSON-LD keep the
 * headline), shaped like what people search: "ConocoPhillips Q3 2026 earnings results", "Nvidia Q3 fiscal 2027
 * earnings results", or "Qualcomm earnings results, Nov 4, 2026" when the slug carries no period. Rick's call,
 * 2026-11-19 (docs/decisions.md).
 */
export function earningsSearchTitle(
  post: { slug: string; ticker: string; tradingDate: string },
  lang: Lang,
): string {
  const name = companyName(post.ticker) ?? post.ticker;
  const p = periodFromSlug(post.slug);
  if (!p) {
    const date = formatDate(post.tradingDate, lang);
    return lang === "zh" ? `${name} 财报，${date}` : `${name} earnings results, ${date}`;
  }
  const plain = p.quarter !== null && calendarAligned(p.quarter, p.fiscalYear, post.tradingDate);
  if (lang === "zh") {
    const q = p.quarter ? `第${"一二三四"[p.quarter - 1]}季度` : "全年";
    return `${name} ${p.fiscalYear}${plain ? "年" : "财年"}${q}财报`;
  }
  if (!p.quarter) return `${name} fiscal ${p.fiscalYear} earnings results`;
  return `${name} Q${p.quarter} ${plain ? "" : "fiscal "}${p.fiscalYear} earnings results`;
}
