import type { Metadata } from "next";
import { headers } from "next/headers";

/** The site origin of the current request, so canonical and hreflang URLs are absolute on any domain. */
export async function siteOrigin(): Promise<URL> {
  const h = await headers();
  const host = h.get("x-forwarded-host") ?? h.get("host") ?? "localhost";
  const proto = h.get("x-forwarded-proto") ?? (host.startsWith("localhost") || host.startsWith("127.") ? "http" : "https");
  return new URL(`${proto}://${host}`);
}

/**
 * Canonical plus the hreflang pair for a page that exists in both languages.
 * `path` is the part after the locale, e.g. "" or "/p/premarket-2026-10-29".
 */
export function pairedAlternates(locale: string, path: string): Metadata["alternates"] {
  return {
    canonical: `/${locale}${path}`,
    languages: { en: `/en${path}`, "zh-Hans": `/zh${path}`, "x-default": `/en${path}` },
  };
}
