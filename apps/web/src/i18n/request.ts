import { hasLocale } from "next-intl";
import { getRequestConfig } from "next-intl/server";
import * as rootParams from "next/root-params";
import { routing } from "./routing";

export default getRequestConfig(async ({ locale }) => {
  const value = locale ?? (await rootParams.locale());
  // The [locale] layout rejects unknown locales with a 404; anything else falls back to the default.
  const resolved = hasLocale(routing.locales, value) ? value : routing.defaultLocale;
  return {
    locale: resolved,
    timeZone: "America/New_York",
    messages: (await import(`../../messages/${resolved}.json`)).default,
  };
});
