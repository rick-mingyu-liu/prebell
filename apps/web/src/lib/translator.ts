import { createTranslator } from "next-intl";
import en from "../../messages/en.json";
import zh from "../../messages/zh.json";
import type { Locale } from "../i18n/routing";

/** Synchronous translator for presentational server components that are handed a `locale`. */
export function translator(locale: Locale) {
  return createTranslator({ locale, messages: locale === "zh" ? zh : en });
}
