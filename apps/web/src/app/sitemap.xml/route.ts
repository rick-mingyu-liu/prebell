import { getCloudflareContext } from "@opennextjs/cloudflare";
import { getDb } from "@/lib/db";
import { mcpEnabled } from "@/lib/mcpswitch";
import { siteOrigin } from "@/lib/seo";
import { cachedSitemap, type EdgeCache, sitemapEntries, sitemapXml } from "@/lib/sitemap";

// Read from D1 on a cache miss (one query), never at build time.
export const dynamic = "force-dynamic";

/** The sitemap. Past about 2,000 posts, split it (a sitemap index of one file per year) to keep each build inside the CPU limit. */
export async function GET(req: Request) {
  const { ctx } = getCloudflareContext();
  // Workers' `caches.default`; absent under `next dev`.
  const cache = (globalThis.caches as unknown as { default?: EdgeCache } | undefined)?.default;
  return cachedSitemap(
    req.url,
    cache,
    (p) => ctx.waitUntil(p),
    async () => sitemapXml(await sitemapEntries(getDb(), await siteOrigin(), mcpEnabled())),
  );
}
