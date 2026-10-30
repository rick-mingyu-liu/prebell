import type { Locale } from "../i18n/routing";
import { formatDate } from "./time";

const TYPE_KEYS = ["types.premarket", "types.recap", "types.earnings"] as const;
type Key = (typeof TYPE_KEYS)[number] | "post.pendingTitle";

/**
 * The subset of a root-namespace translator these helpers use. Keys are literal, so both
 * `getTranslations()` and the message-typed `translator(locale)` fit.
 */
export interface Translator {
  (key: Key, values?: Record<string, string>): string;
  has(key: Key): boolean;
}

/** "After-close recap" for a known content type, else the raw type. */
export function typeLabel(t: Translator, contentType: string): string {
  const key = TYPE_KEYS.find((k) => k === `types.${contentType}`);
  return key && t.has(key) ? t(key) : contentType;
}

/** "After-close recap, Oct 29, 2026": the heading of a post whose text is pending. */
export function pendingTitle(
  t: Translator,
  post: { contentType: string; tradingDate: string },
  locale: Locale,
): string {
  return t("post.pendingTitle", { type: typeLabel(t, post.contentType), date: formatDate(post.tradingDate, locale) });
}
