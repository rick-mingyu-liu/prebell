import type { Metadata } from "next";
import { headers } from "next/headers";
import { DEFAULT_CARD } from "./covers";

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

/** Open Graph locale tags. */
const OG_LOCALE = { en: "en_US", zh: "zh_CN" } as const;

export interface ShareInput {
  locale: "en" | "zh";
  /** Path after the origin, e.g. "/en/p/premarket-2026-10-29"; omitted only by the layout's fallback. */
  url?: string;
  title: string;
  description: string;
  siteName: string;
  /** Site path of the card image; the default card when absent. */
  image?: string;
  /** Set on posts: og:type article with its times (UTC ISO). */
  article?: { publishedTime: string | null; modifiedTime: string | null };
}

/** Open Graph and Twitter card tags in the page's language. Relative URLs resolve against `metadataBase`. */
export function shareMetadata(s: ShareInput): Pick<Metadata, "openGraph" | "twitter"> {
  const images = [{ url: s.image ?? DEFAULT_CARD, width: 1200, height: 630, type: "image/png", alt: s.title }];
  const base = {
    ...(s.url ? { url: s.url } : {}),
    title: s.title,
    description: s.description,
    siteName: s.siteName,
    locale: OG_LOCALE[s.locale],
    alternateLocale: OG_LOCALE[s.locale === "en" ? "zh" : "en"],
    images,
  };
  return {
    openGraph: s.article
      ? {
          ...base,
          type: "article",
          ...(s.article.publishedTime ? { publishedTime: s.article.publishedTime } : {}),
          ...(s.article.modifiedTime ? { modifiedTime: s.article.modifiedTime } : {}),
        }
      : { ...base, type: "website" },
    twitter: { card: "summary_large_image", title: s.title, description: s.description, images },
  };
}

/** The site owner, as the About page names them (a test keeps the name in step with `author.name`). */
export const AUTHOR = { name: "Rick Liu", url: "https://rickmingyuliu.com" };

/** The publisher block shared by the article and home markup. */
function publisher(origin: URL) {
  return {
    "@type": "Organization",
    name: "PreBell",
    url: new URL("/", origin).href,
    logo: { "@type": "ImageObject", url: new URL("/apple-icon.png", origin).href, width: 180, height: 180 },
  };
}

/** schema.org NewsArticle for a published post. */
export function newsArticleLd(
  origin: URL,
  p: {
    url: string;
    headline: string;
    description: string;
    image: string;
    lang: string;
    publishedAt: string | null;
    revisedAt: string | null;
  },
) {
  return {
    "@context": "https://schema.org",
    "@type": "NewsArticle",
    mainEntityOfPage: new URL(p.url, origin).href,
    headline: p.headline,
    description: p.description,
    image: [new URL(p.image, origin).href],
    inLanguage: p.lang,
    ...(p.publishedAt ? { datePublished: p.publishedAt, dateModified: p.revisedAt ?? p.publishedAt } : {}),
    author: { "@type": "Person", ...AUTHOR },
    publisher: publisher(origin),
  };
}

/** schema.org Organization and WebSite for the home page. */
export function homeLd(origin: URL, p: { url: string; name: string; description: string; lang: string }) {
  return [
    { "@context": "https://schema.org", ...publisher(origin), founder: { "@type": "Person", ...AUTHOR } },
    {
      "@context": "https://schema.org",
      "@type": "WebSite",
      name: p.name,
      ...(p.name === "PreBell" ? {} : { alternateName: "PreBell" }),
      url: new URL(p.url, origin).href,
      description: p.description,
      inLanguage: p.lang,
      publisher: { "@type": "Organization", name: "PreBell", url: new URL("/", origin).href },
    },
  ];
}

/** JSON for a `<script type="application/ld+json">`: `<` is escaped so text can never close the script. */
export function jsonLd(data: unknown): string {
  return JSON.stringify(data).replace(/</g, "\\u003c");
}
