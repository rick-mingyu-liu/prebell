import { defineRouting } from "next-intl/routing";

export const routing = defineRouting({
  locales: ["en", "zh"],
  defaultLocale: "en",
  localePrefix: "always",
  localeDetection: false,
});

export type Locale = (typeof routing.locales)[number];

/** BCP 47 tag for `<html lang>` and hreflang. */
export const htmlLang = (locale: Locale) => (locale === "zh" ? "zh-Hans" : "en");
