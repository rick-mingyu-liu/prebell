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
 * its update time, else its first publish.
 */
export async function sitemapEntries(db: D1Like, origin: URL): Promise<MetadataRoute.Sitemap> {
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
    ...paired(origin, "/legal"),
    ...posts,
    ...[...tickers].flatMap(([ticker, modified]) => paired(origin, `/t/${encodeURIComponent(ticker)}`, modified)),
  ];
}
