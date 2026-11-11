/** The sitemap's entries, from one D1 read. */

import type { MetadataRoute } from "next";
import { JOIN_FILING, type D1Like } from "./posts";

type Entry = MetadataRoute.Sitemap[number];

interface Row {
  slug: string;
  published_at: string;
  revised_at: string | null;
  en: string | null;
  zh: string | null;
  ticker: string | null;
}

/** Both languages of a page, each listing the pair (the same hreflang set the pages emit). */
function paired(origin: URL, path: string, lastModified?: string): Entry[] {
  const url = (l: string) => new URL(`/${l}${path}`, origin).href;
  const languages = { en: url("en"), "zh-Hans": url("zh"), "x-default": url("en") };
  return ["en", "zh"].map((l) => ({
    url: url(l),
    ...(lastModified ? { lastModified } : {}),
    alternates: { languages },
  }));
}

const newest = (a: string | undefined, b: string) => (a && a > b ? a : b);

/**
 * Every indexable page: the fixed pages, each published translation of a post (paired only when both
 * are published; a pending Chinese page is noindex and left out), and the ticker pages that have an
 * earnings check with published English text (calendar-only tickers are noindex). A post's lastmod is
 * its update time, else its first publish. The For AI agents page is listed only while the MCP server is on
 * (`mcp`), as the page answers 404 otherwise.
 */
export async function sitemapEntries(db: D1Like, origin: URL, mcp = false): Promise<MetadataRoute.Sitemap> {
  const { results } = await db
    .prepare(
      `SELECT p.slug, p.published_at, p.revised_at, en.status AS en, zh.status AS zh, f.ticker
       FROM posts p
       LEFT JOIN post_translations en ON en.post_id = p.id AND en.lang = 'en'
       LEFT JOIN post_translations zh ON zh.post_id = p.id AND zh.lang = 'zh'
       ${JOIN_FILING}
       WHERE p.published_at IS NOT NULL AND (en.status = 'published' OR zh.status = 'published')
       ORDER BY p.trading_date DESC, p.published_at DESC, p.id DESC`,
    )
    .all<Row>();

  let latest: string | undefined;
  const tickers = new Map<string, string>();
  const posts: Entry[] = [];
  for (const r of results) {
    const modified = r.revised_at ?? r.published_at;
    latest = newest(latest, modified);
    if (r.en === "published" && r.ticker) tickers.set(r.ticker, newest(tickers.get(r.ticker), modified));
    if (r.en === "published" && r.zh === "published") {
      posts.push(...paired(origin, `/p/${r.slug}`, modified));
    } else {
      const lang = r.en === "published" ? "en" : "zh";
      posts.push({ url: new URL(`/${lang}/p/${r.slug}`, origin).href, lastModified: modified });
    }
  }

  return [
    ...paired(origin, "", latest),
    ...paired(origin, "/archive", latest),
    ...paired(origin, "/earnings"),
    ...paired(origin, "/about"),
    ...(mcp ? paired(origin, "/mcp") : []),
    ...paired(origin, "/legal"),
    ...posts,
    ...[...tickers].flatMap(([ticker, modified]) => paired(origin, `/t/${encodeURIComponent(ticker)}`, modified)),
  ];
}

const escapeXml = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&apos;" })[c]!);

/** The sitemap XML, with each entry's hreflang alternates as `xhtml:link`. */
export function sitemapXml(entries: MetadataRoute.Sitemap): string {
  const urls = entries.map((e) => {
    const links = Object.entries(e.alternates?.languages ?? {}).map(
      ([lang, href]) => `<xhtml:link rel="alternate" hreflang="${lang}" href="${escapeXml(String(href))}" />`,
    );
    const lastmod = e.lastModified ? [`<lastmod>${escapeXml(String(e.lastModified))}</lastmod>`] : [];
    return ["<url>", `<loc>${escapeXml(e.url)}</loc>`, ...links, ...lastmod, "</url>"].join("\n");
  });
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">',
    ...urls,
    "</urlset>",
    "",
  ].join("\n");
}

/** The slice of the Workers Cache API (`caches.default`) the sitemap uses. */
export interface EdgeCache {
  match(request: Request): Promise<Response | undefined>;
  put(request: Request, response: Response): Promise<void>;
}

/** How long the edge keeps the sitemap. */
export const SITEMAP_MAX_AGE = 3600;

/**
 * `GET /sitemap.xml`, kept in this colo's edge cache for an hour: a Worker's own responses are not
 * cached by `Cache-Control` alone, so without this every crawl would read D1 and build the XML again.
 * The key ignores the query string. `build` runs only on a miss; the write finishes after the response.
 */
export async function cachedSitemap(
  requestUrl: string,
  cache: EdgeCache | undefined,
  waitUntil: (p: Promise<unknown>) => void,
  build: () => Promise<string>,
): Promise<Response> {
  const key = new Request(new URL("/sitemap.xml", requestUrl));
  const hit = await cache?.match(key);
  if (hit) return hit;
  const res = new Response(await build(), {
    headers: {
      "Content-Type": "application/xml; charset=utf-8",
      "Cache-Control": `public, max-age=${SITEMAP_MAX_AGE}`,
    },
  });
  if (cache) waitUntil(cache.put(key, res.clone()));
  return res;
}
