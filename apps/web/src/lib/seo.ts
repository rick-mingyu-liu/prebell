import type { Metadata } from "next";
import { headers } from "next/headers";

/**
 * The site origin for canonical and hreflang URLs.
 *
 * `SITE_URL` (wrangler.jsonc var) wins. Without it (local dev only) the request `Host` is used.
 * `x-forwarded-*` headers are never read: on Workers nothing strips them, so a client could
 * otherwise point canonical/hreflang at any domain and poison the shared cache (s-maxage=60).
 */
export function resolveOrigin(siteUrl: string | undefined, requestHeaders: Pick<Headers, "get">): URL {
  if (siteUrl) return new URL(new URL(siteUrl).origin);
  const host = requestHeaders.get("host") ?? "localhost";
  const local = /^(localhost|127\.0\.0\.1)(:\d+)?$/.test(host);
  return new URL(`${local ? "http" : "https"}://${host}`);
}

export async function siteOrigin(): Promise<URL> {
  return resolveOrigin(process.env.SITE_URL, await headers());
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
