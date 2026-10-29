import type { Locale } from "../i18n/routing";
import { formatDate } from "./time";

/** The subset of a root-namespace next-intl translator these helpers use. */
export interface Translator {
  (key: string, values?: Record<string, string>): string;
  has(key: string): boolean;
}

/** "After-close recap" for a known content type, else the raw type. */
export function typeLabel(t: Translator, contentType: string): string {
  const key = `types.${contentType}`;
  return t.has(key) ? t(key) : contentType;
}

/** "After-close recap, Oct 29, 2026": the heading of a post whose text is pending. */
export function pendingTitle(
  t: Translator,
  post: { contentType: string; tradingDate: string },
  locale: Locale,
): string {
  return t("post.pendingTitle", { type: typeLabel(t, post.contentType), date: formatDate(post.tradingDate, locale) });
}
